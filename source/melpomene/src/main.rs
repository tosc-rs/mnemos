use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread::{sleep, spawn, yield_now},
    time::Duration,
};

use abi::bbqueue_ipc::BBBuffer;
use melpomene::{
    sim_drivers::{delay::Delay, tcp_serial::spawn_tcp_serial},
    sim_tracing::setup_tracing,
};
use mnemos_kernel::{bbq::new_bidi_channel, Kernel, KernelSettings};

use tracing::Instrument;

const HEAP_SIZE: usize = 192 * 1024;
static KERNEL_LOCK: AtomicBool = AtomicBool::new(true);

fn main() {
    setup_tracing();
    let _span = tracing::info_span!("Melpo").entered();

    println!("========================================");
    let kernel = spawn(move || {
        kernel_entry();
    });
    tracing::info!("Kernel started.");

    // Wait for the kernel to complete initialization...
    while KERNEL_LOCK.load(Ordering::Acquire) {
        yield_now();
    }

    tracing::debug!("Kernel initialized.");

    // let userspace = spawn(move || {
    //     userspace_entry();
    // });
    // println!("[Melpo]: Userspace started.");
    // println!("========================================");

    // let uj = userspace.join();
    println!("========================================");
    sleep(Duration::from_millis(50));
    // println!("[Melpo]: Userspace ended: {:?}", uj);

    let kj = kernel.join();
    sleep(Duration::from_millis(50));
    tracing::info!("Kernel ended:    {:?}", kj);

    println!("========================================");

    tracing::error!("You've met with a terrible fate, haven't you?");
}

#[tracing::instrument(name = "Kernel", level = "info")]
fn kernel_entry() {
    // First, we'll do some stuff that later the linker script will do...
    let kernel_heap = Box::into_raw(Box::new([0u8; HEAP_SIZE]));
    let user_heap = Box::into_raw(Box::new([0u8; HEAP_SIZE]));

    let settings = KernelSettings {
        heap_start: kernel_heap.cast(),
        heap_size: HEAP_SIZE,
        max_drivers: 16,
        k2u_size: 4096,
        u2k_size: 4096,
        user_reply_max_ct: 32,
    };

    let k = unsafe { Kernel::new(settings).unwrap().leak().as_ref() };
    {
        let mut guard = k.heap().lock().unwrap();

        {
            // First let's make a dummy driver just to make sure some stuff happens
            let dummy_fut = async move {
                Delay::new(Duration::from_secs(1)).await;
                let (a_ring, b_ring) = new_bidi_channel(k.heap(), 4096, 4096).await;
                spawn_tcp_serial(b_ring);

                loop {
                    let in_grant = a_ring.read_grant().await;
                    let mut in_gr_slice: &[u8] = &in_grant;

                    while !in_gr_slice.is_empty() {
                        let in_len = in_gr_slice.len();
                        let mut out_grant = a_ring.send_grant_max(in_len).await;
                        let out_len = out_grant.len();
                        let len = in_len.min(out_len);
                        let (now, later) = in_gr_slice.split_at(len);
                        out_grant.copy_from_slice(now);
                        in_gr_slice = later;
                        out_grant.commit(len);
                    }

                    let len = in_grant.len();
                    in_grant.release(len);
                }
            }
            .instrument(tracing::info_span!("Loopback"));

            let dummy_task = k.new_task(dummy_fut);
            let boxed_dummy = guard.alloc_box(dummy_task).map_err(drop).unwrap();
            k.spawn_allocated(boxed_dummy);
        }
    }

    //////////////////////////////////////////////////////////////////////////////
    // TODO: Userspace doesn't really do anything yet! Simulate initialization of
    // the userspace structures, and just periodically wake the kernel for now.
    //////////////////////////////////////////////////////////////////////////////

    let rings = k.rings();
    unsafe {
        let urings = mstd::executor::mailbox::Rings {
            u2k: BBBuffer::take_framed_producer(rings.u2k.as_ptr()),
            k2u: BBBuffer::take_framed_consumer(rings.k2u.as_ptr()),
        };
        mstd::executor::mailbox::MAILBOX.set_rings(urings);
        mstd::executor::EXECUTOR.initialize(user_heap.cast(), HEAP_SIZE);
    }

    let _userspace = spawn(|| {
        let _span = tracing::info_span!("userspace").entered();
        loop {
            while KERNEL_LOCK.load(Ordering::Acquire) {
                sleep(Duration::from_millis(10));
            }

            mstd::executor::EXECUTOR.run();
            KERNEL_LOCK.store(true, Ordering::Release);
        }
    });

    loop {
        while !KERNEL_LOCK.load(Ordering::Acquire) {
            sleep(Duration::from_millis(10));
        }

        k.tick();

        KERNEL_LOCK.store(false, Ordering::Release);
    }
}

// fn userspace_entry() {
//     use mstd::alloc::HEAP;

//     // Set up kernel rings
//     let u2k = unsafe { BBBuffer::take_framed_producer(abi::U2K_RING.load(Ordering::Acquire)) };
//     let k2u = unsafe { BBBuffer::take_framed_consumer(abi::K2U_RING.load(Ordering::Acquire)) };

//     // Set up allocator
//     let mut hg = HEAP.init_exclusive(
//         abi::HEAP_PTR.load(Ordering::Acquire) as usize,
//         abi::HEAP_LEN.load(Ordering::Acquire),
//     ).unwrap();

//     // Set up executor
//     let terpsichore = &mstd::executor::EXECUTOR;

//     // Spawn the `main` task
//     let mtask = mstd::executor::Task::new(async move {
//         aman().await
//     });
//     let hbmtask = hg.alloc_box(mtask).map_err(drop).unwrap();
//     drop(hg);
//     let hg2 = HEAP.lock().unwrap();
//     drop(hg2);
//     let _mhdl = mstd::executor::spawn_allocated(hbmtask);

//     let rings = Rings {
//         u2k,
//         k2u,
//     };
//     mstd::executor::mailbox::MAILBOX.set_rings(rings);

//     let start = Instant::now();
//     loop {
//         *mstd::executor::time::CURRENT_TIME.borrow_mut().unwrap() = start.elapsed().as_micros() as u64;
//         terpsichore.run();
//         KERNEL_LOCK.store(true, Ordering::Release);
//         while KERNEL_LOCK.load(Ordering::Acquire) {
//             sleep(Duration::from_millis(50));
//         }
//     }

// }

// async fn aman() -> Result<(), ()> {
//     mstd::executor::spawn(async {
//         for _ in 0..3 {
//             println!("[ST1] Hi, I'm aman's subtask!");
//             Sleepy::new(Duration::from_secs(3)).await;
//         }
//         println!("[ST1] subtask done!");
//     }).await;

//     mstd::executor::spawn(async {
//         for _ in 0..3 {
//             let msg = UserRequestBody::Serial(SerialRequest::Flush);
//             println!("[ST2] Sending to kernel: {:?}", msg);
//             let resp = MAILBOX.request(msg).await;
//             println!("[ST2] Kernel said: {:?}", resp);
//             Sleepy::new(Duration::from_secs(2)).await;
//         }
//         println!("[ST2] subtask done!");
//     }).await;

//     loop {
//         println!("[MT] Hi, I'm aman!");
//         Sleepy::new(Duration::from_secs(1)).await;
//     }
// }
