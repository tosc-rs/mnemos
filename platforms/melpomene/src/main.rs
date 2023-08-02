use std::{alloc::System, sync::Arc};

use clap::Parser;
use futures::FutureExt;
use melpo_config::PlatformConfig;
use melpomene::{
    cli,
    sim_drivers::{emb_display::SimDisplay, tcp_serial::TcpSerial},
};
use mnemos_alloc::heap::MnemosAlloc;
use mnemos_kernel::{
    daemons::shells::{graphical_shell_mono, GraphicalShellSettings},
    Kernel,
};
use tokio::{
    task,
    time::{self, Duration},
};

const DISPLAY_WIDTH_PX: u32 = 400;
const DISPLAY_HEIGHT_PX: u32 = 240;

fn main() {
    let args = cli::Args::parse();
    args.tracing.setup_tracing();
    let _span = tracing::info_span!("Melpo").entered();
    run_melpomene();
}

#[global_allocator]
static AHEAP: MnemosAlloc<System> = MnemosAlloc::new();

#[tokio::main(flavor = "current_thread")]
async fn run_melpomene() {
    let local = tokio::task::LocalSet::new();
    println!("========================================");
    local
        .run_until(async move {
            let kernel = task::spawn_local(kernel_entry());
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

#[tracing::instrument(name = "Kernel", level = "info")]
async fn kernel_entry() {
    let config = config::load_configuration!(PlatformConfig).unwrap();

    tracing::info!(
        settings = ?config,
        "Loaded settings",
    );

    let k = unsafe {
        mnemos_alloc::containers::Box::into_raw(Kernel::new(config.kernel_cfg).unwrap())
            .as_ref()
            .unwrap()
    };

    // Simulates the kernel main loop being woken by an IRQ.
    let irq = Arc::new(tokio::sync::Notify::new());

    // Initialize the UART
    if let Some(tcp_uart) = config.platform_cfg.tcp_uart {
        k.initialize({
            let irq = irq.clone();
            let socket_addr = tcp_uart.socket_addr.clone();
            async move {
                // Set up the bidirectional, async bbqueue channel between the TCP port
                // (acting as a serial port) and the virtual serial port mux.
                //
                // Create the buffer, and spawn the worker task, giving it one of the
                // queue handles
                tracing::debug!("initializing simulated UART ({})", socket_addr);
                TcpSerial::register(k, tcp_uart, irq).await.unwrap();
                tracing::info!("simulated UART ({}) initialized!", socket_addr);
            }
        })
        .unwrap();
    } else {
        tracing::warn!("Not spawning TCP UART server!");
    }

    // Spawn the graphics driver
    if let Some(display) = config.platform_cfg.display {
        k.initialize(async move {
            SimDisplay::register(k, display, DISPLAY_WIDTH_PX, DISPLAY_HEIGHT_PX)
                .await
                .unwrap();
        })
        .unwrap();
    } else {
        tracing::warn!("Not spawning graphics driver!");
    }

    k.initialize_default_services(config.kernel_svc_cfg);

    // Spawn a graphical shell
    if let Some(forth_shell) = config.platform_cfg.forth_shell {
        let mut guish =
            GraphicalShellSettings::with_display_size(DISPLAY_WIDTH_PX, DISPLAY_HEIGHT_PX);
        guish.capacity = forth_shell.capacity;
        guish.forth_settings = forth_shell.params;
        k.initialize(graphical_shell_mono(k, guish)).unwrap();
    } else {
        tracing::warn!("Not spawning forth GUI shell!");
    }

    let sleep_cap = config.platform_cfg.sleep_cap.as_micros() as u64;
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

            let amount = turn.ticks_to_next_deadline().unwrap_or(sleep_cap);
            tracing::trace!("next timer expires in {amount:?}us");
            // wait for an "interrupt"
            futures::select! {
                _ = irq.notified().fuse() => {
                    tracing::trace!("...woken by I/O interrupt");
               },
               _ = tokio::time::sleep(Duration::from_micros(amount)).fuse() => {
                    tracing::trace!("woken by timer");
               }
            }

            // Account for time slept
            let elapsed = wfi_start.elapsed().as_micros() as u64;
            let _turn = k.timer().force_advance_ticks(elapsed);
        } else {
            // let other tokio tasks (simulated hardware devices) run.
            tokio::task::yield_now().await;
        }
    }
}
