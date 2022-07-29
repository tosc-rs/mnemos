use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread::{sleep, spawn},
};

use abi::bbqueue_ipc::BBBuffer;
use clap::Parser;
use melpomene::{
    cli::{self, MelpomeneOptions},
    sim_drivers::{delay::Delay, tcp_serial::TcpSerial},
};
use mnemos_kernel::{
    drivers::{serial_mux::{SerialMux, SerialMuxHandle}, emb_display::{EmbDisplay, EmbDisplayHandle}},
    Kernel, KernelSettings,
};
use tokio::{
    task,
    time::{self, Duration},
};

use tracing::Instrument;

use chrono::{Local, Timelike, Datelike};

use embedded_graphics::{
    pixelcolor::{Gray8},
    prelude::*,
    primitives::{Line, PrimitiveStyle},
    mono_font::{ascii::FONT_6X9, ascii::FONT_5X7, MonoTextStyle},
    text::Text,
    image::{Image, ImageRaw},
};

use embedded_graphics_simulator::{BinaryColorTheme, SimulatorDisplay, Window, OutputSettingsBuilder, SimulatorEvent};

const HEAP_SIZE: usize = 192 * 1024;
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
    };

    let k = unsafe { Kernel::new(settings).unwrap().leak().as_ref() };

    // First let's make a dummy driver just to make sure some stuff happens
    let initialization_future = async move {
        // Delay for one second, just for funsies
        Delay::new(Duration::from_secs(1)).await;

        // Set up the bidirectional, async bbqueue channel between the TCP port
        // (acting as a serial port) and the virtual serial port mux.
        //
        // Create the buffer, and spawn the worker task, giving it one of the
        // queue handles
        TcpSerial::register(k, opts.serial_addr, 4096, 4096)
            .await
            .unwrap();

        // Now, right now this is a little awkward, but what I'm doing here is spawning
        // a new virtual mux, and configuring it with:
        // * Up to 4 virtual ports max
        // * Framed messages up to 512 bytes max each
        SerialMux::register(k, 4, 512).await.unwrap();

        let mut mux_hdl = SerialMuxHandle::from_registry(k).await.unwrap();
        let p0 = mux_hdl.open_port(0, 1024).await.unwrap();
        let p1 = mux_hdl.open_port(1, 1024).await.unwrap();
        drop(mux_hdl);

        k.spawn(
            async move {
                loop {
                    let rgr = p0.consumer().read_grant().await;
                    let len = rgr.len();
                    p0.send(&rgr).await;
                    rgr.release(len);
                }
            }
            .instrument(tracing::info_span!("Loopback")),
        )
        .await;

        // Now we just send out data every second
        k.spawn(
            async move {
                loop {
                    Delay::new(Duration::from_secs(1)).await;
                    p1.send(b"hello\r\n").await;
                }
            }
            .instrument(tracing::info_span!("Hello Loop")),
        )
        .await;
    }
    .instrument(tracing::info_span!("Initialize"));

    k.initialize(initialization_future).unwrap();

    // Creating a dummy task to use the embedded display driver
    let graphics_init_future = async move {
        // Delay for one second, just for funsies
        Delay::new(Duration::from_secs(1)).await;

        EmbDisplay::register(k, 4, 160, 120).await.unwrap();

        let mut disp_hdl = EmbDisplayHandle::from_registry(k).await.unwrap();
        let mut fc_0 = disp_hdl.get_framechunk(0, 80, 45, 160, 120).await.unwrap();
        drop(disp_hdl);

        let line_style = PrimitiveStyle::with_stroke(Gray8::WHITE, 1);
        let tline = Line::new(Point::new(148, 2), Point::new(8, 2))
            .into_styled(line_style);
        let bline = Line::new(Point::new(148, 24), Point::new(8, 24))
            .into_styled(line_style);
        let lline = Line::new(Point::new(7, 24), Point::new(7, 2))
            .into_styled(line_style);
        let rline = Line::new(Point::new(149, 24), Point::new(149, 2))
            .into_styled(line_style);
        let text_style = MonoTextStyle::new(&FONT_6X9, Gray8::WHITE);
        let datetime_style = MonoTextStyle::new(&FONT_5X7, Gray8::WHITE);
        let text1 = Text::new("Welcome to mnemOS!", Point::new(10, 10), text_style);
        let text2 = Text::new("A tiny operating system", Point::new(10, 20), text_style);
        let output_settings = OutputSettingsBuilder::new()
            .theme(BinaryColorTheme::OledBlue)
            .build();
                
        let mut sdisp = SimulatorDisplay::<Gray8>::new(Size::new(320, 240));
        let mut window = Window::new("mnemOS", &output_settings);

        k.spawn(
            async move {
                'running: loop {

                    // disp.clear(Gray8::BLACK).unwrap();

                    tline.draw(&mut fc_0).unwrap();
                    bline.draw(&mut fc_0).unwrap();
                    rline.draw(&mut fc_0).unwrap();
                    lline.draw(&mut fc_0).unwrap();

                    text1.draw(&mut fc_0).unwrap();
                    text2.draw(&mut fc_0).unwrap();

                    let time = Local::now();

                    let time_str = format!(
                        "{:02}:{:02}:{02}",
                        time.hour(),
                        time.minute(),
                        time.second()
                    );

                    let date_str = format!(
                        "{:02}/{:02}/{:02}",
                        time.month(),
                        time.day(),
                        time.year()
                    );

                    let date_text = Text::new(&date_str, Point::new(28, 35), datetime_style);
                    let time_text = Text::new(&time_str, Point::new(88, 35), datetime_style);

                    date_text.draw(&mut fc_0).unwrap();
                    time_text.draw(&mut fc_0).unwrap();
                    {
                        let raw_img = fc_0.frame_display().unwrap();
                        let image = Image::new(&raw_img, Point::new(80, 45));
                        image.draw(&mut sdisp).unwrap();
                        
                        window.update(&sdisp);
                        if window.events().any(|e| e== SimulatorEvent::Quit) {
                            break 'running
                        }

                        Delay::new(Duration::from_secs(1)).await;
                        sdisp.clear(Gray8::BLACK).unwrap();
                    }
                    fc_0.frame_clear();
                }
            }
            .instrument(tracing::info_span!("Update clock")),
        )
        .await;
    }
    .instrument(tracing::info_span!("Initialize graphics driver"));

    k.initialize(graphics_init_future).unwrap();


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
