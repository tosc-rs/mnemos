use std::{alloc::System, sync::Arc, time::Duration};

use async_std::{
    stream::{IntoStream, StreamExt},
    sync::{Condvar, Mutex},
};
use futures::{channel::mpsc, FutureExt};
use gloo::timers::future::IntervalStream;
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
use pomelo::sim_drivers::serial::Serial;
use sermux_proto::PortChunk;
use tracing::{trace, Instrument, Level};
use tracing_wasm::WASMLayerConfigBuilder;
use wasm_bindgen_futures::spawn_local;
const DISPLAY_WIDTH_PX: u32 = 400;
const DISPLAY_HEIGHT_PX: u32 = 240;

#[global_allocator]
static AHEAP: MnemosAlloc<System> = MnemosAlloc::new();

fn main() {
    let tracing_config = WASMLayerConfigBuilder::new()
        .set_max_level(Level::TRACE)
        .build();
    tracing_wasm::set_as_global_default_with_config(tracing_config);
    let _span: tracing::span::EnteredSpan = tracing::info_span!("Pomelo").entered();
    spawn_local(run_pomelo());
    // TODO wait, somehow?
}

async fn run_pomelo() {
    tracing::info!("Kernel started.");
    let res = kernel_entry().await;
    tracing::info!("Kernel ended: {:?}", res);
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
                tracing::debug!("initializing SerialMuxServer...");
                SerialMuxServer::register(kernel, PORTS, FRAME_SIZE)
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

    // Initialize a loopback UART
    let (mut tx, rx) = mpsc::channel::<u8>(16);
    kernel
        .initialize({
            let irq = irq.clone();
            async move {
                tracing::debug!("initializing loopback UART");
                Serial::register(
                    kernel,
                    8,
                    8,
                    WellKnown::Loopback.into(),
                    irq,
                    rx.into_stream(),
                )
                .await
                .unwrap();
                tracing::info!("loopback UART initialized!");
            }
        })
        .unwrap();

    // Spawn a loopback port
    let loopback_settings = LoopbackSettings::default();
    kernel
        .initialize(loopback(kernel, loopback_settings))
        .unwrap();

    // Spawn a hello port
    let hello_settings = HelloSettings::default();
    kernel.initialize(hello(kernel, hello_settings)).unwrap();

    // Spawn the spawnulator
    kernel
        .initialize(SpawnulatorServer::register(kernel, 16))
        .unwrap();

    // test loopback service by throwing bytes at it
    spawn_local(async move {
        IntervalStream::new(500)
            .for_each(move |_| {
                let chunk = PortChunk::new(WellKnown::Loopback, b"!!");
                let mut buf = [0u8; 8];
                if let Ok(ser) = chunk.encode_to(&mut buf) {
                    for byte in ser {
                        if let Err(e) = tx.try_send(*byte) {
                            tracing::error!("could not send: {e:?}");
                        } else {
                            tracing::info!("sent a byte!");
                        }
                    }
                }
            })
            .await;
    });

    // run the kernel on its own
    spawn_local(async move {
        let tick_millis = 500;
        let tick_duration = Duration::from_millis(tick_millis);
        IntervalStream::new(tick_millis as u32)
            .for_each(move |_| {
                let tick = kernel.tick();
                // tracing::debug!("Tick {tick:?}");
                // TODO add sleep logic
                kernel.timer().force_advance(tick_duration);
            })
            .await;
    });

    // run the kernel on "interrupt"
    let dummy_mutex = Mutex::new(false);
    loop {
        let dummy_guard = dummy_mutex.lock().await;
        irq.wait(dummy_guard).await;
        trace!("...woken by I/O interrupt");
    }
}
