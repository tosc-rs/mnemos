use super::*;

use mnemos_alloc::heap::MnemosAlloc;
use std::{
    future::Future,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

#[global_allocator]
static ALLOC: MnemosAlloc<std::alloc::System> = MnemosAlloc::new();

pub(crate) struct TestKernel {
    kernel: NonNull<Kernel>,
}

impl TestKernel {
    fn new() -> Self {
        trace_init();

        // TODO(eliza): this clock implementation is also used in Melpomene, so
        // it would be nice if we could share it with melpo...
        let clock = {
            maitake::time::Clock::new(Duration::from_micros(1), || {
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_micros() as u64
            })
            .named("CLOCK_SYSTEMTIME_NOW")
        };

        // XXX(eliza): the test kernel is gonna be leaked forever...maybe we
        // should do something about that, if we wanna have a lot of tests. but,
        // at least it means we never create a dangling pointer to it.
        let kernel = unsafe {
            NonNull::new(mnemos_alloc::containers::Box::into_raw(
                Kernel::new(KernelSettings { max_drivers: 16 }, clock).unwrap(),
            ))
            .expect("newly-allocated kernel mustn't be null!")
        };

        Self { kernel }
    }

    pub fn run<F: Future + 'static>(future: impl FnOnce(&'static Kernel) -> F) {
        let running = Arc::new(AtomicBool::new(true));
        let test = Self::new();
        let k = unsafe { test.kernel.as_ref() };
        k.initialize({
            let running = running.clone();
            let f = future(k);
            async move {
                f.await;
                running.store(false, Ordering::SeqCst);
            }
        })
        .unwrap();
        let mut ticks = 0;
        while running.load(Ordering::SeqCst) {
            tracing::trace!("\n");
            tracing::trace!("---- TICK {ticks} ---");
            tracing::trace!("\n");
            let tick = k.tick();

            tracing::trace!("\n");
            tracing::trace!(?tick);
            ticks += 1;
            if running.load(Ordering::SeqCst) {
                assert!(
                    tick.has_remaining,
                    "no tasks were woken, but the test future hasn't finished \
                     yet.this would hang forever --- seems bad!"
                );
            }
        }
    }
}

fn trace_init() {
    use tracing_subscriber::{
        filter::{EnvFilter, LevelFilter},
        prelude::*,
    };
    let env = std::env::var("RUST_LOG").unwrap_or_default();
    let builder = EnvFilter::builder().with_default_directive(LevelFilter::INFO.into());
    let filter = if env.is_empty() {
        builder.parse("kernel=debug").unwrap()
    } else {
        builder.parse_lossy(env)
    };

    let _res = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .with_thread_names(true)
        .without_time()
        .finish()
        .try_init();
}
