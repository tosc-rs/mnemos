use std::{alloc::System, sync::Arc};

use clap::Parser;
use futures::FutureExt;
use melpomene::{
    cli::{self, MelpomeneOptions},
    sim_drivers::{emb_display::SimDisplay, tcp_serial::TcpSerial},
};
use mnemos_alloc::heap::MnemosAlloc;
use mnemos_kernel::{
    drivers::serial_mux::{SerialMuxClient, SerialMuxServer},
    forth::shells::graphical_shell_mono,
    Kernel, KernelSettings,
};
use tokio::{
    task,
    time::{self, Duration},
}; // fuse()

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
    k.initialize(
        async {
            tracing::debug!(
                "initializing SimDisplay server ({DISPLAY_WIDTH_PX}x{DISPLAY_HEIGHT_PX})..."
            );
            SimDisplay::register(k, 4, DISPLAY_WIDTH_PX, DISPLAY_HEIGHT_PX)
                .await
                .unwrap();
            tracing::info!("SimDisplayServer initialized!");
        }
        .instrument(tracing::info_span!("SimDisplayServer")),
    )
    .unwrap();

    // Spawn a loopback port
    k.initialize({
        const PORT: u16 = 0;
        async {
            tracing::debug!("initializing SerMux loopback...");
            let mut mux_hdl = SerialMuxClient::from_registry(k).await;
            let p0 = mux_hdl.open_port(PORT, 1024).await.unwrap();
            drop(mux_hdl);
            tracing::info!("SerMux Loopback running!");

            loop {
                let rgr = p0.consumer().read_grant().await;
                let len = rgr.len();
                tracing::trace!("Loopback read {len}B");
                p0.send(&rgr).await;
                rgr.release(len);
            }
        }
        .instrument(tracing::info_span!("Loopback", port = PORT))
    })
    .unwrap();

    // Spawn a hello port
    k.initialize({
        const PORT: u16 = 1;
        async {
            tracing::debug!("Starting SerMux 'hello world'...");
            let mut mux_hdl = SerialMuxClient::from_registry(k).await;
            let p1 = mux_hdl.open_port(PORT, 1024).await.unwrap();
            drop(mux_hdl);
            tracing::info!("SerMux 'hello world' running!");

            loop {
                k.sleep(Duration::from_secs(1)).await;
                p1.send(b"hello\r\n").await;
            }
        }
        .instrument(tracing::info_span!("Hello Loop", port = PORT))
    })
    .unwrap();

    // Spawn a graphical shell
    k.initialize(
        async move {
            tracing::debug!("Starting graphics console...");
            graphical_shell_mono(
                k,   // disp_width_px
                400, // disp_height_px
                240, // port
                2,   // capacity
                1024,
            )
            .await;
        }
        .instrument(tracing::info_span!("Graphics Console")),
    )
    .unwrap();

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
            let amount = turn.ticks_to_next_deadline().unwrap_or(100 * 1000); // 1 ticks per us, 1000 us per ms, 100ms sleep
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
