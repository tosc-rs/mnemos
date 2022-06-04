use std::{sync::atomic::{Ordering, compiler_fence, AtomicBool}, thread::{spawn, yield_now, sleep}, time::{Duration, Instant}, ops::Deref, future::Future, task::Poll, collections::VecDeque};

use abi::{bbqueue_ipc::BBBuffer, HEAP_PTR};
use mstd::executor::Task;

const RING_SIZE: usize = 4096;
const HEAP_SIZE: usize = 192 * 1024;
static KERNEL_LOCK: AtomicBool = AtomicBool::new(true);

fn main() {
    println!("========================================");
    let kernel = spawn(move || {
        kernel_entry();
    });
    println!("[Melpo]: Kernel started.");

    // Wait for the kernel to complete initialization...
    while KERNEL_LOCK.load(Ordering::Acquire) {
        yield_now();
    }

    let userspace = spawn(move || {
        userspace_entry();
    });
    println!("[Melpo]: Userspace started.");
    println!("========================================");

    let kj = kernel.join();
    sleep(Duration::from_millis(50));
    let uj = userspace.join();
    sleep(Duration::from_millis(50));

    println!("========================================");
    println!("[Melpo]: Kernel ended:    {:?}", kj);
    println!("[Melpo]: Userspace ended: {:?}", uj);
    println!("========================================");

    println!("[Melpo]: You've met with a terrible fate, haven't you?");
}

fn kernel_entry() {
    // First, we'll do some stuff that later the linker script will do...
    let u2k = Box::into_raw(Box::new(BBBuffer::new()));
    let u2k_buf = Box::into_raw(Box::new([0u8; RING_SIZE]));
    let k2u = Box::into_raw(Box::new(BBBuffer::new()));
    let k2u_buf = Box::into_raw(Box::new([0u8; RING_SIZE]));
    let user_heap = Box::into_raw(Box::new([0u8; HEAP_SIZE]));

    // Still linker script things...
    abi::U2K_RING.store(u2k, Ordering::Relaxed);
    abi::K2U_RING.store(k2u, Ordering::Relaxed);
    abi::HEAP_PTR.store(user_heap.cast(), Ordering::Relaxed);
    abi::HEAP_LEN.store(HEAP_SIZE, Ordering::Relaxed);

    // TODO: The kernel itself is supposed to do this...
    unsafe {
        (*u2k).initialize(u2k_buf.cast(), RING_SIZE);
        (*k2u).initialize(k2u_buf.cast(), RING_SIZE);
    }

    let u2k = unsafe { BBBuffer::take_framed_consumer(u2k) };
    let k2u = unsafe { BBBuffer::take_framed_producer(k2u) };

    compiler_fence(Ordering::SeqCst);

    struct DelayMsg {
        rx: Instant,
        msg: Vec<u8>,
    }

    let mut msgs: VecDeque<DelayMsg> = VecDeque::new();

    loop {
        while !KERNEL_LOCK.load(Ordering::Acquire) {
            yield_now();
        }

        // Here I would do kernel things, IF I HAD ANY
        while let Some(msg) = u2k.read() {
            msgs.push_back(DelayMsg {
                rx: Instant::now(),
                msg: msg.to_vec(),
            });
            msg.release();
        }

        // Loop back userspace messages, delayed one second
        while let Some(msg) = msgs.pop_front() {
            if msg.rx.elapsed() > Duration::from_secs(1) {
                if let Ok(mut wgr) = k2u.grant(msg.msg.len()) {
                    wgr.copy_from_slice(&msg.msg);
                    wgr.commit(msg.msg.len());
                } else {
                    // No receive space, put it back
                    msgs.push_front(msg);
                    break;
                }
            } else {
                // Not ready, put it back
                msgs.push_front(msg);
                break;
            }
        }

        KERNEL_LOCK.store(false, Ordering::Release);
    }
}

fn userspace_entry() {
    use mstd::alloc::HEAP;

    // Set up kernel rings
    let mut u2k = unsafe { BBBuffer::take_framed_producer(abi::U2K_RING.load(Ordering::Acquire)) };
    let mut k2u = unsafe { BBBuffer::take_framed_consumer(abi::K2U_RING.load(Ordering::Acquire)) };

    // Set up allocator
    let mut hg = HEAP.init_exclusive(
        abi::HEAP_PTR.load(Ordering::Acquire) as usize,
        abi::HEAP_LEN.load(Ordering::Acquire),
    ).unwrap();

    // Set up executor
    let terpsichore = &mstd::executor::EXECUTOR;

    // Spawn the `main` task
    let mtask = mstd::executor::Task::new(async move {
        aman().await
    });
    let hbmtask = hg.alloc_box(mtask).map_err(drop).unwrap();
    drop(hg);
    let _mhdl = mstd::executor::spawn_allocated(hbmtask);

    let start = Instant::now();
    loop {
        *mstd::executor::time::CURRENT_TIME.borrow_mut().unwrap() = start.elapsed().as_micros() as u64;
        terpsichore.run(&mut u2k, &mut k2u);
        sleep(Duration::from_millis(50));
    }

}

async fn aman() -> Result<(), ()> {
    mstd::executor::spawn(async {
        for _ in 0..3 {
            println!("Hi, I'm aman's subtask!");
            Sleepy::new(Duration::from_secs(3)).await;
        }
        println!("subtask done!");
    }).await;

    loop {
        println!("Hi, I'm aman!");
        Sleepy::new(Duration::from_secs(1)).await;
    }
}

fn ayield_now() -> Yield {
    Yield { once: false }
}

struct Yield {
    once: bool,
}

impl Future for Yield {
    type Output = ();

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        if !self.once {
            self.once = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

struct Sleepy {
    start: Instant,
    dur: Duration,
}

impl Sleepy {
    fn new(dur: Duration) -> Self {
        Self {
            start: Instant::now(),
            dur,
        }
    }
}

impl Future for Sleepy {
    type Output = ();

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        cx.waker().wake_by_ref();
        if self.start.elapsed() < self.dur {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}
