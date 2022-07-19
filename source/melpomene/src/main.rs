use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread::{sleep, spawn},
};

use abi::bbqueue_ipc::BBBuffer;
use clap::{Parser, Command};
use melpomene::{
    cli::{self, MelpomeneOptions},
    sim_drivers::{delay::Delay, tcp_serial::spawn_tcp_serial},
};
use mnemos_kernel::{
    comms::{bbq::new_bidi_channel, kchannel::KChannel},
    drivers::serial_mux::{Message, Request, Response, SerialMux},
    drivers::graphics_driver::{Msg, Req, Resp, FrameChunk},
    Kernel, KernelSettings,
};
use tokio::{
    task,
    time::{self, Duration},
};

use embedded_graphics::{
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Circle, Line, Rectangle, PrimitiveStyle},
    mono_font::{ascii::FONT_6X9, MonoTextStyle},
    text::Text,
};

use embedded_graphics_simulator::{BinaryColorTheme, SimulatorDisplay, Window, OutputSettingsBuilder};

use tracing::Instrument;

const HEAP_SIZE: usize = 384 * 1024;
static KERNEL_LOCK: AtomicBool = AtomicBool::new(true);

fn main() {
    let args = cli::Args::parse();
    args.tracing.setup_tracing();
    let _span = tracing::info_span!("Melpo").entered();
    run_melpomene(args.melpomene);
}

#[tokio::main(flavor = "current_thread")]
async fn run_melpomene(opts: cli::MelpomeneOptions) {
    println!("========================================");
    let kernel = task::spawn_blocking(move || {
        kernel_entry(opts);
    });
    tracing::info!("Kernel started.");

    // Wait for the kernel to complete initialization...
    while KERNEL_LOCK.load(Ordering::Acquire) {
        task::yield_now().await;
    }

    tracing::debug!("Kernel initialized.");

    // let userspace = spawn(move || {
    //     userspace_entry();
    // });
    // println!("[Melpo]: Userspace started.");
    // println!("========================================");

    // let uj = userspace.join();
    println!("========================================");
    time::sleep(Duration::from_millis(50)).await;
    // println!("[Melpo]: Userspace ended: {:?}", uj);

    let kj = kernel.await;
    time::sleep(Duration::from_millis(50)).await;
    tracing::info!("Kernel ended:    {:?}", kj);

    println!("========================================");

    tracing::error!("You've met with a terrible fate, haven't you?");
}

#[tracing::instrument(name = "Kernel", level = "info", skip(opts))]
fn kernel_entry(opts: MelpomeneOptions) {
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
                // Delay for one second, just for funsies
                Delay::new(Duration::from_secs(1)).await;
                
                // Create a virtual display that the graphics driver will talk to. For now I've left it at 320 x 240. Later number of rows and columns 
                // can be made configurable. 
                //
                // The virtual display is passed to FrameChunk, which after spawning returns a KProducer which we use to send commands
                // For now, we use it just open a window, and display some text.
                let fprod  = FrameChunk::new(k, 320, 240).await;

                // Kproducer to send requests to the graphics driver
                let (mprod, mcons) = KChannel::<Result<Resp, ()>>::new_async(k, 1)
                    .await
                    .split();

                // Request a window. This serves as the display screen
                let win_req = Req::OpenWindow;

                // Send the request with the KProducer handle received above
                fprod.enqueue_async( Msg{
                    req: win_req,
                    resp: mprod.clone(),
                }).await
                .map_err(drop)
                .unwrap();

                // Response from driver confirms whether the window as opened or not.
                // At this point a window with some text should have opened
                let win_handle = mcons.dequeue_async().await.unwrap().unwrap();

                let _wh = match win_handle {
                    Resp::WindowOpened(w) => w,
                };

                // TODO: implement methods to send update commands over the KChannel with above 
                // window handle



                // Set up the bidirectional, async bbqueue channel between the TCP port
                // (acting as a serial port) and the virtual serial port mux.
                //
                // Create the buffer, and spawn the worker task, giving it one of the
                // queue handles
                let (mux_ring, tcp_ring) = new_bidi_channel(k.heap(), 4096, 4096).await;
                spawn_tcp_serial(opts.serial_addr, tcp_ring).await;

                // Now, right now this is a little awkward, but what I'm doing here is spawning
                // a new virtual mux, and configuring it with:
                // * Up to 4 virtual ports max
                // * Framed messages up to 512 bytes max each
                // * The other side of the async connection to the TCP "serial" port
                //
                // After spawning, it gives us back IT'S message passing handle, where we can
                // send it configuration commands. Right now - that basically just is used for
                // mapping new virtual ports.
                let mux_hdl = SerialMux::new(k, 4, 512, mux_ring).await;

                // To send a message to the Mux, we need a "return address". The message
                // we send includes a Request, and a producer for the KChannel of the kind below.
                //
                // I still need to do some more work on the design of the message bus/types that
                // will go on with this, or decide whether a mutex'd handle is better for non-userspace
                // communications.
                let (kprod, kcons) = KChannel::<Result<Response, ()>>::new_async(k, 4)
                    .await
                    .split();

                // Map virtual port 0, with a maximum (incoming) buffer capacity of 1024 bytes.
                let request_0 = Request::RegisterPort {
                    port_id: 0,
                    capacity: 1024,
                };
                let request_1 = Request::RegisterPort {
                    port_id: 1,
                    capacity: 1024,
                };

                // Send the Message with our request, and our KProducer handle. If we did this
                // a bunch, we could clone the producer.
                mux_hdl
                    .enqueue_async(Message {
                        req: request_0,
                        resp: kprod.clone(),
                    })
                    .await
                    .map_err(drop)
                    .unwrap();
                mux_hdl
                    .enqueue_async(Message {
                        req: request_1,
                        resp: kprod.clone(),
                    })
                    .await
                    .map_err(drop)
                    .unwrap();

                // Now we get back a message in our "return address", which SHOULD contain
                // a confirmation that the port was registered.
                let resp_0 = kcons.dequeue_async().await.unwrap().unwrap();
                let resp_1 = kcons.dequeue_async().await.unwrap().unwrap();

                let p0 = match resp_0 {
                    Response::PortRegistered(p) => p,
                };
                let p1 = match resp_1 {
                    Response::PortRegistered(p) => p,
                };

                k.spawn(async move {
                    loop {
                        let rgr = p0.consumer().read_grant().await;
                        let len = rgr.len();
                        p0.send(&rgr).await;
                        rgr.release(len);
                    }
                })
                .await;

                loop {
                    Delay::new(Duration::from_secs(1)).await;
                    p1.send(b"hello\r\n").await;
                }
                

            }
            .instrument(tracing::info_span!("Loopback"));

            let dummy_task = k.new_task(dummy_fut);
            let boxed_dummy = guard.alloc_box(dummy_task).map_err(drop).unwrap();
            k.spawn_allocated(boxed_dummy);
        }

        // Make sure we don't accidentally give away the mutex guard
        drop(guard);
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
