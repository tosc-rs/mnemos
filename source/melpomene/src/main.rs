use std::{alloc::System, sync::Arc};

use clap::Parser;
use embedded_graphics::{primitives::{Rectangle, StyledDrawable, Primitive, PrimitiveStyleBuilder}, prelude::{Point, Size}, pixelcolor::BinaryColor, Drawable};
use futures::FutureExt;
use melpomene::{
    cli::{self, MelpomeneOptions},
    sim_drivers::{emb_display::SimDisplay, tcp_serial::TcpSerial, emb_display2::SimDisplay2, embd2_svc::{EmbDisplay2Client, FrameChunk, FrameLocSize, MonoChunk}},
};
use mnemos_alloc::heap::MnemosAlloc;
use mnemos_kernel::{
    daemons::{
        sermux::{hello, loopback, HelloSettings, LoopbackSettings},
        shells::{graphical_shell_mono, GraphicalShellSettings},
    },
    services::{forth_spawnulator::SpawnulatorServer, serial_mux::SerialMuxServer},
    Kernel, KernelSettings,
};
use tokio::{
    task,
    time::{self, Duration},
};

use tracing::Instrument;
const DISPLAY_WIDTH_PX: u32 = 400;
const DISPLAY_HEIGHT_PX: u32 = 240;

fn main() {
    let args = cli::Args::parse();
    args.tracing.setup_tracing();
    let _span = tracing::info_span!("Melpo").entered();
    run_melpomene(args.melpomene);
}

#[global_allocator]
static AHEAP: MnemosAlloc<System> = MnemosAlloc::new();

#[tokio::main(flavor = "current_thread")]
async fn run_melpomene(opts: cli::MelpomeneOptions) {
    let local = tokio::task::LocalSet::new();
    println!("========================================");
    local
        .run_until(async move {
            let kernel = task::spawn_local(kernel_entry(opts));
            tracing::info!("Kernel started.");

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
    let settings = KernelSettings {
        max_drivers: 16,
        // TODO(eliza): chosen totally arbitrarily
        timer_granularity: maitake::time::Duration::from_micros(1),
    };

    let k = unsafe {
        mnemos_alloc::containers::Box::into_raw(Kernel::new(settings).unwrap())
            .as_ref()
            .unwrap()
    };

    // Simulates the kernel main loop being woken by an IRQ.
    let irq = Arc::new(tokio::sync::Notify::new());

    // Initialize the UART
    k.initialize({
        let irq = irq.clone();
        async move {
            // Set up the bidirectional, async bbqueue channel between the TCP port
            // (acting as a serial port) and the virtual serial port mux.
            //
            // Create the buffer, and spawn the worker task, giving it one of the
            // queue handles
            tracing::debug!("initializing simulated UART ({})", opts.serial_addr);
            TcpSerial::register(k, opts.serial_addr, 4096, 4096, irq)
                .await
                .unwrap();
            tracing::info!("simulated UART ({}) initialized!", opts.serial_addr);
        }
    })
    .unwrap();

    // Initialize the SerialMuxServer
    k.initialize({
        const PORTS: usize = 16;
        const FRAME_SIZE: usize = 512;
        async {
            // * Up to 16 virtual ports max
            // * Framed messages up to 512 bytes max each
            tracing::debug!("initializing SerialMuxServer...");
            SerialMuxServer::register(k, PORTS, FRAME_SIZE)
                .await
                .unwrap();
            tracing::info!("SerialMuxServer initialized!");
        }
        .instrument(tracing::info_span!(
            "SerialMuxServer",
            ports = PORTS,
            frame_size = FRAME_SIZE
        ))
    })
    .unwrap();

    // Spawn the graphics driver
    // k.initialize(async move {
    //     SimDisplay::register(k, 4, DISPLAY_WIDTH_PX, DISPLAY_HEIGHT_PX)
    //         .await
    //         .unwrap();
    // })
    // .unwrap();

    // Spawn the graphics driver
    k.initialize(async move {
        SimDisplay2::register(k, 4, DISPLAY_WIDTH_PX, DISPLAY_HEIGHT_PX)
            .await
            .unwrap();
    })
    .unwrap();

    k.initialize(disp_demo(k)).unwrap();

    // Spawn a loopback port
    let loopback_settings = LoopbackSettings::default();
    k.initialize(loopback(k, loopback_settings)).unwrap();

    // Spawn a hello port
    let hello_settings = HelloSettings::default();
    k.initialize(hello(k, hello_settings)).unwrap();

    // Spawn a graphical shell
    // let mut guish = GraphicalShellSettings::with_display_size(DISPLAY_WIDTH_PX, DISPLAY_HEIGHT_PX);
    // guish.capacity = 1024;
    // k.initialize(graphical_shell_mono(k, guish)).unwrap();

    // Spawn the spawnulator
    // k.initialize(SpawnulatorServer::register(k, 16)).unwrap();

    loop {
        // Tick the scheduler
        let t0 = tokio::time::Instant::now();
        let tick = k.tick();

        // advance the timer (don't take more than 500k years)
        let ticks = t0.elapsed().as_micros() as u64;
        let turn = k.timer().force_advance_ticks(ticks);
        tracing::trace!("advanced timer by {ticks:?}");

        // If there is nothing else scheduled, and we didn't just wake something up,
        // sleep for some amount of time
        if turn.expired == 0 && !tick.has_remaining {
            let wfi_start = tokio::time::Instant::now();
            // if no timers have expired on this tick, we should sleep until the
            // next timer expires *or* something is woken by I/O, to simulate a
            // hardware platform waiting for an interrupt.
            tracing::trace!("waiting for an interrupt...");

            // Cap out at 100ms, just in case sim services aren't using the IRQ

            // 1 ticks per us, 1000 us per ms, 100ms sleep
            const CAP: u64 = 100 * 1000;
            let amount = turn.ticks_to_next_deadline().unwrap_or(CAP);
            tracing::trace!("next timer expires in {amount:?}us");
            // wait for an "interrupt"
            futures::select! {
                _ = irq.notified().fuse() => {
                    tracing::trace!("...woken by I/O interrupt");
               },
               _ = tokio::time::sleep(Duration::from_micros(amount.into())).fuse() => {
                    tracing::trace!("woken by timer");
               }
            }

            // Account for time slept
            let elapsed = wfi_start.elapsed().as_micros() as u64;
            let _turn = k.timer().force_advance_ticks(elapsed.into());
        } else {
            // let other tokio tasks (simulated hardware devices) run.
            tokio::task::yield_now().await;
        }
    }
}

async fn disp_demo(k: &'static Kernel) {
    k.sleep(Duration::from_millis(1000)).await;
    tracing::warn!("DRAWING");
    let mut client = EmbDisplay2Client::from_registry(k).await;
    let mut chunk = MonoChunk::allocate_mono(FrameLocSize {
        offset_x: 0,
        offset_y: 100,
        width: 100,
        height: 100,
    }).await;
    loop {
        for x in 0..6 {
            for i in 0..5 {
                chunk.meta.start_x = x * 50;
                chunk.clear();
                let style = PrimitiveStyleBuilder::new()
                    .stroke_color(BinaryColor::On)
                    .stroke_width(3)
                    .build();
                Rectangle::new(Point::new(10, 10), Size::new(i * 15, i * 15))
                    .into_styled(style)
                    .draw(&mut chunk).unwrap();
                chunk = client.draw_mono(chunk).await.unwrap();

                tracing::warn!("DREW");
                k.sleep(Duration::from_millis(1000) / 15).await;

                chunk.invert_masked();
                chunk = client.draw_mono(chunk).await.unwrap();

                tracing::warn!("DREW");

            }
        }
    }
}
