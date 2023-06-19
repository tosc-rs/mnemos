use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{sleep, spawn},
};

use abi::bbqueue_ipc::BBBuffer;
use clap::Parser;
use futures::FutureExt;
use input_mgr::RingLine;
use melpomene::{
    cli::{self, MelpomeneOptions},
    sim_drivers::{emb_display::SimDisplay, tcp_serial::TcpSerial},
};
use mnemos_kernel::{
    drivers::{
        emb_display::EmbDisplayClient,
        serial_mux::{SerialMuxClient, SerialMuxService},
    },
    Kernel, KernelSettings,
};
use tokio::{
    task,
    time::{self, Duration},
}; // fuse()

use tracing::Instrument;

use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Gray8,
    prelude::{Drawable, GrayColor, Point, Primitive},
    primitives::{Line, PrimitiveStyle},
    text::Text,
};
use profont::PROFONT_12_POINT;

const DISPLAY_WIDTH_PX: u32 = 400;
const DISPLAY_HEIGHT_PX: u32 = 240;
/// The Allwinner D1 has 1GB of memory, so we can definitely get away with two
/// 1MB heaps.
const HEAP_SIZE: usize = 1024 * 1024;

static KERNEL_LOCK: AtomicBool = AtomicBool::new(true);

fn main() {
    let args = cli::Args::parse();
    args.tracing.setup_tracing();
    let _span = tracing::info_span!("Melpo").entered();
    run_melpomene(args.melpomene);
}

#[tokio::main(flavor = "current_thread")]
async fn run_melpomene(opts: cli::MelpomeneOptions) {
    let local = tokio::task::LocalSet::new();
    println!("========================================");
    local
        .run_until(async move {
            let kernel = task::spawn_local(kernel_entry(opts));
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
        })
        .await;

    println!("========================================");

    tracing::error!("You've met with a terrible fate, haven't you?");
}

#[tracing::instrument(name = "Kernel", level = "info", skip(opts))]
async fn kernel_entry(opts: MelpomeneOptions) {
    // First, we'll do some stuff that later the linker script will do...
    fn alloc_heap() -> (*mut u8, usize) {
        use std::mem::ManuallyDrop;
        // use `Vec::with_capacity` to allocate the memory without having to
        // create a stack array, or initialize the memory.
        // the vector is intentionally leaked.
        let mut mem = ManuallyDrop::new(Vec::<u8>::with_capacity(HEAP_SIZE));
        let slice = mem.spare_capacity_mut();
        // we use the *actual* size of the allocation, since liballoc may have
        // given us more than we asked for.
        let sz = slice.len();
        (slice.as_mut_ptr().cast(), sz)
    }

    let (heap_start, heap_size) = alloc_heap();
    let (user_heap, user_heap_size) = alloc_heap();

    let settings = KernelSettings {
        heap_start,
        heap_size,
        max_drivers: 16,
        k2u_size: 4096,
        u2k_size: 4096,
        // TODO(eliza): chosen totally arbitrarily
        timer_granularity: maitake::time::Duration::from_millis(1),
    };

    let k = unsafe { Kernel::new(settings).unwrap().leak().as_ref() };

    // Simulates the kernel main loop being woken by an IRQ.
    let irq = Arc::new(tokio::sync::Notify::new());

    // First let's make a dummy driver just to make sure some stuff happens
    let initialization_future = {
        let irq = irq.clone();
        async move {
            // Delay for one second, just for funsies
            k.sleep(Duration::from_secs(1)).await;

            // Set up the bidirectional, async bbqueue channel between the TCP port
            // (acting as a serial port) and the virtual serial port mux.
            //
            // Create the buffer, and spawn the worker task, giving it one of the
            // queue handles
            TcpSerial::register(k, opts.serial_addr, 4096, 4096, irq.clone())
                .await
                .unwrap();

            // Now, right now this is a little awkward, but what I'm doing here is spawning
            // a new virtual mux, and configuring it with:
            // * Up to 4 virtual ports max
            // * Framed messages up to 512 bytes max each
            SerialMuxService::register(k, 4, 512).await.unwrap();

            let mut mux_hdl = SerialMuxClient::from_registry(k).await.unwrap();
            let p0 = mux_hdl.open_port(0, 1024).await.unwrap();
            let p1 = mux_hdl.open_port(1, 1024).await.unwrap();
            drop(mux_hdl);

            // Spawn the graphics driver
            SimDisplay::register(k, 4, DISPLAY_WIDTH_PX, DISPLAY_HEIGHT_PX)
                .await
                .unwrap();

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
                        k.sleep(Duration::from_secs(1)).await;
                        p1.send(b"hello\r\n").await;
                    }
                }
                .instrument(tracing::info_span!("Hello Loop")),
            )
            .await;
        }
        .instrument(tracing::info_span!("Initialize"))
    };

    k.initialize(initialization_future).unwrap();
    let tid0_future = k.initialize_forth_tid0(Default::default());

    // Create the interactive console task
    let graphics_console = async move {
        // Delay for 1.5 seconds, just for funsies
        k.sleep(Duration::from_millis(1500)).await;

        // Take Port 2 from the serial mux. This corresponds to TCP port 10002 when
        // you are running crowtty
        let mut mux_hdl = SerialMuxClient::from_registry(k).await.unwrap();
        let p2 = mux_hdl.open_port(2, 1024).await.unwrap();
        drop(mux_hdl);

        let mut disp_hdl = EmbDisplayClient::from_registry(k).await.unwrap();
        let char_y = PROFONT_12_POINT.character_size.height;
        let char_x = PROFONT_12_POINT.character_size.width + PROFONT_12_POINT.character_spacing;

        // Draw titlebar
        {
            let mut fc_0 = disp_hdl
                .get_framechunk(0, 0, DISPLAY_WIDTH_PX, char_y)
                .await
                .unwrap();
            let text_style = MonoTextStyle::new(&PROFONT_12_POINT, Gray8::WHITE);
            let text1 = Text::new(
                "mnemOS",
                Point::new(0, PROFONT_12_POINT.baseline as i32),
                text_style,
            );
            text1.draw(&mut fc_0).unwrap();

            let title = "forth shell";
            let text2 = Text::new(
                title,
                Point::new(
                    (DISPLAY_WIDTH_PX as i32) - ((title.len() as u32) * char_x) as i32,
                    PROFONT_12_POINT.baseline as i32,
                ),
                text_style,
            );
            text2.draw(&mut fc_0).unwrap();

            let line_style = PrimitiveStyle::with_stroke(Gray8::WHITE, 1);
            Line::new(
                Point {
                    x: 0,
                    y: PROFONT_12_POINT.underline.offset as i32,
                },
                Point {
                    x: DISPLAY_WIDTH_PX as i32,
                    y: PROFONT_12_POINT.underline.offset as i32,
                },
            )
            .into_styled(line_style)
            .draw(&mut fc_0)
            .unwrap();
            disp_hdl.draw_framechunk(fc_0).await.unwrap();
        }

        k.spawn(
            async move {
                // TODO(eliza): don't spawn the forth task from within the
                // graphics driver lol...

                let style = ring_drawer::BwStyle {
                    background: Gray8::BLACK,
                    font: MonoTextStyle::new(&PROFONT_12_POINT, Gray8::WHITE),
                };

                // At 12-pt font, there is enough room for 16 lines, with 50 chars/line.
                //
                // Leave out 4 for the implicit margin of two characters on each gutter.
                let mut rline = RingLine::<16, 46>::new();

                let tid0 = tid0_future.await.expect("TID 0 initialization task must succeed");

                loop {
                    // Wait until there is a frame buffer ready. There wouldn't be if we've spammed frames
                    // before they've been consumed.
                    let mut fc_0 = loop {
                        let fc = disp_hdl
                            .get_framechunk(
                                0,
                                char_y as i32,
                                DISPLAY_WIDTH_PX,
                                DISPLAY_HEIGHT_PX - char_y,
                            )
                            .await;
                        if let Some(fc) = fc {
                            break fc;
                        } else {
                            k.sleep(Duration::from_millis(10)).await;
                        }
                    };
                    ring_drawer::drawer_bw(&mut fc_0, &rline, style.clone()).unwrap();
                    disp_hdl.draw_framechunk(fc_0).await.unwrap();

                    futures::select! {
                        rgr = p2.consumer().read_grant().fuse() => {
                            let mut used = 0;
                            'input: for &b in rgr.iter() {
                                used += 1;
                                match rline.append_local_char(b) {
                                    Ok(_) => {}
                                    // backspace
                                    Err(_) if b == 0x7F => {
                                        rline.pop_local_char();
                                    }
                                    Err(_) if b == b'\n' => {
                                        let needed = rline.local_editing_len();
                                        if needed != 0 {
                                            let mut tid0_wgr = tid0.producer().send_grant_exact(needed).await;
                                            rline.copy_local_editing_to(&mut tid0_wgr).unwrap();
                                            tid0_wgr.commit(needed);
                                            rline.submit_local_editing();
                                            break 'input;
                                        }
                                    }
                                    Err(error) => {
                                        tracing::warn!(?error, "Error appending char: {:02X}", b);
                                    }
                                }
                            }

                            rgr.release(used);
                        },
                        output = tid0.consumer().read_grant().fuse() => {
                            let len = output.len();
                            tracing::trace!(len, "Received output from TID0");
                            for &b in output.iter() {
                                // TODO(eliza): what if this errors lol
                                if b == b'\n' {
                                    rline.submit_remote_editing();
                                } else {
                                    let _ = rline.append_remote_char(b);
                                }
                            }
                            output.release(len);
                        }
                    }
                }
            }
            .instrument(tracing::info_span!("Update clock")),
        )
        .await;
    }
    .instrument(tracing::info_span!("Initialize graphics driver"));

    k.initialize(graphics_console).unwrap();

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
        mstd::executor::EXECUTOR.initialize(user_heap, user_heap_size);
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

    let mut t0 = tokio::time::Instant::now();
    loop {
        while !KERNEL_LOCK.load(Ordering::Acquire) {
            sleep(Duration::from_millis(10));
        }

        let tick = k.tick();

        KERNEL_LOCK.store(false, Ordering::Release);

        // advance the timer
        let ticks = t0.elapsed().as_millis() as u64;
        let turn = k.timer().force_advance_ticks(ticks);
        tracing::trace!("advanced timer by {ticks:?}");
        t0 = tokio::time::Instant::now();

        if turn.expired == 0 && !tick.has_remaining {
            // if no timers have expired on this tick, we should sleep until the
            // next timer expires *or* something is woken by I/O, to simulate a
            // hardware platform waiting for an interrupt.
            tracing::debug!("waiting for an interrupt...");
            if let Some(next_timer) = turn.time_to_next_deadline() {
                tracing::debug!("next timer expires in {next_timer:?}");
                // wait for an "interrupt"
                futures::select! {
                    _ = irq.notified().fuse() => {
                        tracing::debug!("...woken by I/O interrupt");
                    },
                   _ = tokio::time::sleep(next_timer).fuse() => {
                        tracing::debug!("woken by timer");
                   }
                }
            } else {
                irq.notified().await;
                tracing::debug!("...woken by I/O interrupt");
            }
        }

        // let other tokio tasks (simulated hardware devices) run.
        tokio::task::yield_now().await;
    }
}
