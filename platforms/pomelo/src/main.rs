use std::{alloc::System, time::Duration};

use async_std::stream::IntoStream;
use futures::{
    channel::mpsc::{self},
    FutureExt,
};
use futures_util::{select, StreamExt};
use gloo::timers::future::TimeoutFuture;
use gloo_utils::format::JsValueSerdeExt;
use mnemos_alloc::heap::MnemosAlloc;
use mnemos_kernel::{
    daemons::sermux::{loopback, LoopbackSettings},
    forth::{self, Forth},
    services::{
        forth_spawnulator::SpawnulatorServer,
        keyboard::mux::KeyboardMuxServer,
        serial_mux::{PortHandle, SerialMuxServer, WellKnown},
    },
    Kernel, KernelSettings,
};
use pomelo::{
    sim_drivers::serial::Serial,
    term_iface::{init_term, to_term, Command, SERMUX_TX},
};
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, Instrument, Level};
use tracing_subscriber::{
    filter::{self},
    prelude::*,
    Registry,
};
use tracing_wasm::{WASMLayer, WASMLayerConfig};
use wasm_bindgen::{closure::Closure, prelude::*};
use wasm_bindgen_futures::spawn_local;

#[global_allocator]
static AHEAP: MnemosAlloc<System> = MnemosAlloc::new();

fn setup_tracing() {
    let wasm_layer = WASMLayer::new(WASMLayerConfig::default());
    let filter = filter::Targets::new()
        .with_target("pomelo", Level::DEBUG)
        .with_target("maitake", Level::INFO)
        .with_default(Level::DEBUG);

    let subscriber = Registry::default().with(wasm_layer).with(filter);
    tracing::subscriber::set_global_default(subscriber).unwrap();
}
fn main() {
    setup_tracing();

    let _span: tracing::span::EnteredSpan = tracing::info_span!("Pomelo").entered();

    spawn_local(run_pomelo());
    // TODO do we need to wait? (how?)
}

async fn run_pomelo() {
    info!("Kernel started.");
    let res = kernel_entry().await;
    info!("Kernel ended: {:?}", res);
}

#[tracing::instrument(name = "Kernel", level = "info")]
async fn kernel_entry() {
    let settings = KernelSettings {
        max_drivers: 16,
        // TODO(eliza): chosen totally arbitrarily
        timer_granularity: maitake::time::Duration::from_micros(1),
    };

    let kernel = unsafe {
        mnemos_alloc::containers::Box::into_raw(Kernel::new(settings).unwrap())
            .as_ref()
            .unwrap()
    };

    // Simulates the kernel main loop being woken by an IRQ.
    let (irq_tx, irq_rx) = mpsc::channel::<()>(4);

    // Initialize the virtual serial port mux
    const SERIAL_FRAME_SIZE: usize = 512;
    let (tx, rx) = mpsc::channel::<u8>(64);
    SERMUX_TX.set(tx.clone()).unwrap();

    // Initialize a loopback UART
    kernel
        .initialize({
            async move {
                debug!("initializing loopback UART");
                Serial::register(
                    kernel,
                    256,
                    SERIAL_FRAME_SIZE * 2, // *1 is not quite enough, required overhead to be +10 bytes for cobs + sermux
                    WellKnown::Loopback.into(),
                    irq_tx.clone(),
                    rx.into_stream(),
                    to_term,
                )
                .await
                .unwrap();
                info!("loopback UART initialized!");
            }
        })
        .unwrap();

    kernel.initialize_default_services(Default::default());

    // go forth and replduce
    spawn_local(async move {
        let port = PortHandle::open(kernel, WellKnown::ForthShell0.into(), 256)
            .await
            .unwrap();
        let (task, tid_io) = Forth::new(kernel, forth::Params::default())
            .await
            .expect("Forth spawning must succeed");
        kernel.spawn(task.run()).await;
        kernel
            .spawn(async move {
                loop {
                    futures::select_biased! {
                        rgr = port.consumer().read_grant().fuse() => {
                            let needed = rgr.len();
                            trace!(needed, "Forth: received input");
                            let mut tid_io_wgr = tid_io.producer().send_grant_exact(needed).await;
                            tid_io_wgr.copy_from_slice(&rgr);
                            tid_io_wgr.commit(needed);
                            rgr.release(needed);
                        },
                        output = tid_io.consumer().read_grant().fuse() => {
                            let needed = output.len();
                            trace!(needed, "Forth: Received output from tid_io");
                            port.send(&output).await;
                            output.release(needed);
                        }
                    }
                }
            })
            .await;
    });

    // link to browser terminal: receive commands, dispatch bacon
    let eternal_cb: Closure<dyn Fn(JsValue)> = Closure::new(|val: JsValue| {
        if let Ok(cmd) = val.into_serde::<Command>() {
            cmd.dispatch(kernel);
        }
    });
    init_term(&eternal_cb);
    eternal_cb.forget();

    let mut irq_rx = irq_rx.into_stream().fuse();
    let timer = kernel.timer();
    loop {
        let mut then = chrono::Local::now();
        let tick = kernel.tick();
        let dt = chrono::Local::now()
            .signed_duration_since(then)
            .to_std()
            .unwrap();
        trace!("timer - before sleep: advance {dt:?}");
        let next_turn = timer
            .force_advance(dt)
            .time_to_next_deadline()
            .unwrap_or(Duration::from_secs(1));
        trace!("timer: before sleep: next turn in {next_turn:?}");
        let mut next_fut = TimeoutFuture::new(
            next_turn
                .as_millis()
                .try_into()
                .expect("next turn is too far in the future"),
        )
        .fuse();

        then = chrono::Local::now();
        let now = select! {
            _ = irq_rx.next() => {
                trace!("timer: WAKE: \"irq\" {tick:?}");
                chrono::Local::now()
            },
            _ = next_fut => {
                let tick = kernel.tick();
                trace!("timer: WAKE: timer {tick:?}");
                chrono::Local::now()
            }
        };
        let dt = now.signed_duration_since(then).to_std().unwrap();
        trace!("timer: slept for {dt:?}");
        kernel.timer().force_advance(dt);
    }
}
