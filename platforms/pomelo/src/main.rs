use std::{alloc::System, sync::Arc, time::Duration};

use async_std::{
    stream::{IntoStream, StreamExt},
    sync::{Condvar, Mutex},
};
use futures::{
    channel::mpsc::{self, Sender},
    FutureExt,
};
use gloo::timers::future::IntervalStream;
use gloo_utils::format::JsValueSerdeExt;
use mnemos_alloc::heap::MnemosAlloc;
use mnemos_kernel::{
    daemons::{
        sermux::{hello, loopback, HelloSettings, LoopbackSettings},
        shells::{graphical_shell_mono, GraphicalShellSettings},
    },
    services::{
        forth_spawnulator::SpawnulatorServer,
        serial_mux::{SerialMuxServer, WellKnown},
    },
    Kernel, KernelSettings,
};
use pomelo::{
    sim_drivers::serial::Serial,
    term_iface::{init_term, to_term, Command, ECHO_TX},
};
use serde::{Deserialize, Serialize};
use sermux_proto::PortChunk;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, Instrument, Level};
use tracing_wasm::WASMLayerConfigBuilder;
use wasm_bindgen::{closure::Closure, prelude::*};
use wasm_bindgen_futures::spawn_local;

const DISPLAY_WIDTH_PX: u32 = 400;
const DISPLAY_HEIGHT_PX: u32 = 240;

#[global_allocator]
static AHEAP: MnemosAlloc<System> = MnemosAlloc::new();

fn main() {
    let tracing_config = WASMLayerConfigBuilder::new()
        .set_max_level(Level::DEBUG)
        .build();
    tracing_wasm::set_as_global_default_with_config(tracing_config);

    let _span: tracing::span::EnteredSpan = tracing::info_span!("Pomelo").entered();

    spawn_local(run_pomelo());
    // TODO do we need to wait?
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
    // TODO is `Condvar` the right thing to use?
    let irq = Arc::new(Condvar::new());

    // Initialize the SerialMuxServer
    kernel
        .initialize({
            const PORTS: usize = 16;
            const FRAME_SIZE: usize = 512;
            async {
                // * Up to 16 virtual ports max
                // * Framed messages up to 512 bytes max each
                debug!("initializing SerialMuxServer...");
                SerialMuxServer::register(kernel, PORTS, FRAME_SIZE)
                    .await
                    .unwrap();
                info!("SerialMuxServer initialized!");
            }
            .instrument(tracing::info_span!(
                "SerialMuxServer",
                ports = PORTS,
                frame_size = FRAME_SIZE
            ))
        })
        .unwrap();

    // Initialize a loopback UART
    let (tx, rx) = mpsc::channel::<u8>(64);
    ECHO_TX.set(tx.clone()).unwrap();

    kernel
        .initialize({
            let irq = irq.clone();
            async move {
                debug!("initializing loopback UART");
                Serial::register(
                    kernel,
                    256,
                    256,
                    WellKnown::Loopback.into(),
                    irq,
                    rx.into_stream(),
                    to_term,
                )
                .await
                .unwrap();
                info!("loopback UART initialized!");
            }
        })
        .unwrap();

    // Spawn a loopback port
    let loopback_settings = LoopbackSettings::default();
    kernel
        .initialize(loopback(kernel, loopback_settings))
        .unwrap();

    // Spawn the spawnulator
    kernel
        .initialize(SpawnulatorServer::register(kernel, 16))
        .unwrap();

    // test loopback service by throwing bytes at it
    spawn_local(async move {
        IntervalStream::new(500)
            .for_each(move |_| {
                //echo("mooo".to_string());
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

    // run the kernel on its own
    // TODO: remodel to a select, so we can actually advance the clock correctly
    spawn_local(async move {
        let tick_millis = 250;
        let tick_duration = Duration::from_millis(tick_millis);
        IntervalStream::new(tick_millis as u32)
            .for_each(move |_| {
                let tick = kernel.tick();
                trace!("Periodic kernel tick: {tick:?}");
                kernel.timer().force_advance(tick_duration);
            })
            .await;
    });

    // run the kernel on "interrupt"
    // TODO: remodel to a select, so we can actually advance the clock correctly
    let dummy_mutex = Mutex::new(false);
    loop {
        let then = chrono::Local::now();
        let dummy_guard = dummy_mutex.lock().await;
        irq.wait(dummy_guard).await;
        let now = chrono::Local::now();
        let diff = now.signed_duration_since(then).to_std().unwrap();
        // TODO
        // kernel.timer().force_advance(diff);
        let human_diff = humantime::Duration::from(diff);
        trace!("woken by I/O interrupt after {human_diff}");
    }
}
