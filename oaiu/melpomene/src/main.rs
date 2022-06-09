use std::{
    sync::atomic::{Ordering, compiler_fence, AtomicBool},
    thread::{spawn, yield_now, sleep},
    time::{Duration, Instant},
    future::Future,
    task::Poll,
    collections::VecDeque,
};

use abi::{
    bbqueue_ipc::BBBuffer,
    syscall::{
        UserRequest, KernelResponse, KernelResponseBody,
        KernelResponseHeader, UserRequestBody,
        serial::SerialRequest, KernelMsg,
    },
};
use mstd::executor::mailbox::{Rings, MAILBOX};

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

    let uj = userspace.join();
    println!("========================================");
    sleep(Duration::from_millis(50));
    println!("[Melpo]: Userspace ended: {:?}", uj);

    let kj = kernel.join();
    sleep(Duration::from_millis(50));
    println!("[Melpo]: Kernel ended:    {:?}", kj);


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
        msg: UserRequest,
    }

    let mut msgs: VecDeque<DelayMsg> = VecDeque::new();

    loop {
        while !KERNEL_LOCK.load(Ordering::Acquire) {
            sleep(Duration::from_millis(50));
        }

        // Here I would do kernel things, IF I HAD ANY
        while let Some(msg) = u2k.read() {
            let req = postcard::from_bytes(&msg).unwrap();
            msgs.push_back(DelayMsg {
                rx: Instant::now(),
                msg: req,
            });
            msg.release();
        }

        // Loop back userspace messages, delayed one second
        while let Some(msg) = msgs.pop_front() {
            if msg.rx.elapsed() > Duration::from_secs(1) {
                if let Ok(mut wgr) = k2u.grant(128) {
                    let used = postcard::to_slice(
                        &KernelMsg::Response(
                            KernelResponse {
                                header: KernelResponseHeader { nonce: msg.msg.header.nonce },
                                body: KernelResponseBody::TodoLoopback,
                            },
                        ),
                        &mut wgr
                    ).unwrap().len();
                    wgr.commit(used);
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
    let u2k = unsafe { BBBuffer::take_framed_producer(abi::U2K_RING.load(Ordering::Acquire)) };
    let k2u = unsafe { BBBuffer::take_framed_consumer(abi::K2U_RING.load(Ordering::Acquire)) };

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
    let hg2 = HEAP.lock().unwrap();
    drop(hg2);
    let _mhdl = mstd::executor::spawn_allocated(hbmtask);

    let rings = Rings {
        u2k,
        k2u,
    };
    mstd::executor::mailbox::MAILBOX.set_rings(rings);



    let start = Instant::now();
    loop {
        *mstd::executor::time::CURRENT_TIME.borrow_mut().unwrap() = start.elapsed().as_micros() as u64;
        terpsichore.run();
        KERNEL_LOCK.store(true, Ordering::Release);
        while KERNEL_LOCK.load(Ordering::Acquire) {
            sleep(Duration::from_millis(50));
        }
    }

}

async fn aman() -> Result<(), ()> {
    mstd::executor::spawn(async {
        for _ in 0..3 {
            println!("[ST1] Hi, I'm aman's subtask!");
            Sleepy::new(Duration::from_secs(3)).await;
        }
        println!("[ST1] subtask done!");
    }).await;

    mstd::executor::spawn(async {
        for _ in 0..3 {
            let msg = UserRequestBody::Serial(SerialRequest::Flush);
            println!("[ST2] Sending to kernel: {:?}", msg);
            let resp = MAILBOX.request(msg).await;
            println!("[ST2] Kernel said: {:?}", resp);
            Sleepy::new(Duration::from_secs(2)).await;
        }
        println!("[ST2] subtask done!");
    }).await;

    loop {
        println!("[MT] Hi, I'm aman!");
        Sleepy::new(Duration::from_secs(1)).await;
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
