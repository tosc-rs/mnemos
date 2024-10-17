use std::{alloc::System, time::Duration};

use async_std::stream::IntoStream;
use futures::{
    channel::mpsc::{self},
    FutureExt,
};
use futures_util::select;
use gloo::timers::future::TimeoutFuture;
use gloo_utils::format::JsValueSerdeExt;
use mnemos_kernel::{
    comms::kchannel::KChannel,
    daemons::shells::{graphical_shell_mono, GraphicalShellSettings},
    forth::{self, Forth},
    mnemos_alloc::heap::MnemosAlloc,
    services::serial_mux::{PortHandle, WellKnown},
    Kernel, KernelServiceSettings, KernelSettings,
};
use pomelo::{
    sim_drivers::{
        emb_display::SimDisplay,
        io::{irq_async, IRQ_TX},
        serial::Serial,
    },
    term_iface::{init_term, to_term, Command, SERMUX_TX},
};
use tracing::{debug, info, trace, Level};
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
    console_error_panic_hook::set_once();
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
    let settings = KernelSettings { max_drivers: 16 };

    let clock = {
        maitake::time::Clock::new(
            // TODO(eliza): timer granularity chosen totally arbitrarily
            Duration::from_micros(1),
            || {
                // TODO(anatol) please fix this for me thanks :)
                unimplemented!("FIGURE OUT HOW TO GET A NORMAL TIMESTAMP OUT OF WASM LOL")
            },
        )
        .named("CLOCK_THIS_ONE_JUST_FUCKING_PANICS_LOL")
    };
    let kernel = unsafe {
        mnemos_alloc::containers::Box::into_raw(Kernel::new(settings, clock).unwrap())
            .as_ref()
            .unwrap()
    };

    // Simulates the kernel main loop being woken by an IRQ.
    let (irq_tx, irq_rx) = KChannel::new_async(4).await.split();
    IRQ_TX.set(irq_tx.clone()).ok();

    // TODO: something in the init sequence is not waking up the kernel when it should.
    // work around for now by forcing one irq
    irq_async().await;

    // Initialize the virtual serial port mux
    const SERIAL_FRAME_SIZE: usize = 512;
    let (tx, rx) = mpsc::channel::<u8>(64);
    SERMUX_TX.set(tx.clone()).unwrap();

    // Initialize a loopback UART
    // TODO this vs. default services
    kernel
        .initialize({
            async move {
                debug!("initializing loopback UART");
                Serial::register(
                    kernel,
                    256,
                    SERIAL_FRAME_SIZE * 2, // *1 is not quite enough, required overhead to be +10 bytes for cobs + sermux
                    WellKnown::Loopback.into(),
                    rx.into_stream(),
                    to_term,
                )
                .await
                .unwrap();
                info!("loopback UART initialized!");
            }
        })
        .unwrap();

    let mut service_settings: KernelServiceSettings = Default::default();
    service_settings.sermux_hello.enabled = false;
    kernel.initialize_default_services(service_settings);
    let width = 240;
    let height = 240;
    kernel
        .initialize({
            async move {
                SimDisplay::register(kernel, height, height).await.unwrap();
            }
        })
        .unwrap();

    let mut guish = GraphicalShellSettings::with_display_size(width, height);

    guish.capacity = Default::default();
    guish.forth_settings = Default::default();
    kernel
        .initialize(graphical_shell_mono(kernel, guish))
        .unwrap();

    // go forth and replduce
    kernel
        .spawn(async move {
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
        })
        .await;

    // link to browser terminal: receive commands, dispatch bacon
    let eternal_cb: Closure<dyn Fn(JsValue)> = Closure::new(|val: JsValue| {
        if let Ok(cmd) = val.into_serde::<Command>() {
            cmd.dispatch(kernel);
        }
    });

    init_term(&eternal_cb);

    eternal_cb.forget();

    let timer = kernel.timer();
    loop {
        let mut then = chrono::Local::now();
        let tick = kernel.tick();
        let dt = chrono::Local::now()
            .signed_duration_since(then)
            .to_std()
            .unwrap();
        trace!("timer - before sleep: advance {dt:?}");
        let next_turn = timer.turn();

        trace!("timer: before sleep: next turn in {next_turn:?}");

        if next_turn.expired == 0 || !tick.has_remaining {
            trace!("timer: sleeping");
            let next_turn = next_turn
                .time_to_next_deadline()
                .unwrap_or(Duration::from_millis(1000));
            let mut next_fut = TimeoutFuture::new(
                next_turn
                    .as_millis()
                    .try_into()
                    .expect("next turn is too far in the future"),
            )
            .fuse();

            then = chrono::Local::now();
            select! {
                _ = irq_rx.dequeue_async().fuse() => {
                    trace!("timer: WAKE: \"irq\" {tick:?}");
                },
                _ = next_fut => {
                    trace!("timer: WAKE: timer {tick:?}");
                }
            }
            let now = chrono::Local::now();
            let dt = now.signed_duration_since(then).to_std().unwrap();
            trace!("timer: slept for {dt:?}");
            kernel.timer().turn();
        }
    }
}
