//! # The mnemos kernel
//!
//! The mnemos kernel is implemented as a library, with platform-specific code depending on the
//! kernel library and producing the final binary.
//!
//! ## The "kernelspace" entry point
//!
//! At the moment, the kernel requires some "entry point" code, more or less the `main()` function
//! that runs when the system boots, to perform any system-specific initialization.
//!
//! This entry point code is responsible for setting up any hardware or subsystems that exist
//! outside the kernel (or are required by any kernel services), as well as starting and running
//! the kernel itself.
//!
//! ## Setting up the Allocator
//!
//! Before creating the kernel, a [MnemosAlloc][mnemos_alloc::heap::MnemosAlloc] instance must be
//! registered as a [GlobalAlloc][core::alloc::GlobalAlloc], and if the
//! [UnderlyingAllocator][mnemos_alloc::heap::UnderlyingAllocator] requires initialization,
//! it must occur before creating the kernel.
//!
//! The kernel will be allocated using the provided allocator.
//!
//! ## Creating the kernel
//!
//! The allocator and any kernel settings are provided to the kernel constructor. The kernel
//! will allocate itself, and return an owned kernel entity.
//!
//! After creation, the executor is *not* running yet.
//!
//! ## Initialization phase
//!
//! After creating the kernel, we need to register kernel services, which are expected to act as
//! drivers for various system components.
//!
//! Since we are not running the executor yet, the kernel provides an interface,
//! [`Kernel::initialize()`], which takes a future and spawns it on the executor. Futures added with
//! `initialize` still do not run until the later "running" phase.
//!
//! Right now, it is generally suggested you use one or more `initialize`
//! calls to register all (initial) kernel services.
//!
//! ## Running mode
//!
//! Once everything is prepared and initialized, the startup code is expected to call
//! [`Kernel::tick()`] repeatedly. On each call to tick:
//!
//! * The allocator frees any synchronously dropped allocations, making them available for
//!   asynchronous allocation
//! * The async executor is polled
//!
//! AT THE MOMENT, there is no indication of whether all tasks are blocked, which could be use to
//! inform whether we should put the CPU into some kind of sleep mode until a hardware event (like
//! a timer or DMA transaction) is triggered, and an async task has potentially been awoken.
//!
//! ## Not covered: "userspace"
//!
//! At the moment, there is SOME concept of a userspace, which interacts with the kernel via a
//! bidirectional IPC ringbuffer. Space for this ringbuffer is allocated when calling
//! [`Kernel::new()`]. Additionally this ringbuffer is polled on each call to `tick`, after freeing
//! allocations and before calling `tick` on the scheduler.
//!
//! This is an artifact of how mnemos 0.1 worked, where there was a single userspace executor that
//! existed and interacted with the kernel executor.
//!
//! As of 2023-05-30, I don't think this is the right abstraction for multiple userspace processes.
//! The pieces that exist currently are likely to be removed or reworked heavily before they are
//! usable, and should be considered nonfunctional at the moment.

#![no_std]
#![allow(clippy::missing_safety_doc)]
#![feature(impl_trait_in_assoc_type)]
#![feature(async_fn_in_trait)] // needed for `embedded-hal-async`

extern crate alloc;

pub mod comms;
pub mod daemons;
pub(crate) mod fmt;
pub mod forth;
pub mod isr;
pub mod registry;
pub mod retry;
#[cfg(feature = "serial-trace")]
pub mod serial_trace;
pub mod services;

use abi::{
    bbqueue_ipc::BBBuffer,
    syscall::{KernelResponse, UserRequest},
};
use comms::kchannel::KChannel;
use core::{future::Future, ptr::NonNull, convert::identity};
pub use embedded_hal_async;
pub use maitake;
use maitake::{
    scheduler::LocalScheduler,
    sync::Mutex,
    task::{BoxStorage, JoinHandle, Storage},
    time::{Duration, Sleep, Timeout, Timer},
};
pub use mnemos_alloc;
use mnemos_alloc::containers::Box;
use registry::Registry;
use serde::{Deserialize, Serialize};
use services::{
    forth_spawnulator::{SpawnulatorServer, SpawnulatorSettings},
    keyboard::mux::{KeyboardMuxServer, KeyboardMuxSettings},
    serial_mux::{SerialMuxServer, SerialMuxSettings},
};
pub use tracing;

pub struct Rings {
    pub u2k: NonNull<BBBuffer>,
    pub k2u: NonNull<BBBuffer>,
}

pub struct KernelSettings {
    pub max_drivers: usize,
    pub timer_granularity: Duration,
}

pub struct Message {
    pub request: UserRequest,
    pub response: KChannel<KernelResponse>,
}

pub struct Kernel {
    /// Items that do not require a lock to access, and must only
    /// be accessed with shared refs
    inner: KernelInner,
    /// The run-time driver registry, accessed via an async Mutex
    registry: Mutex<Registry>,
}

unsafe impl Sync for Kernel {}

pub struct KernelInner {
    /// MnemOS currently only targets single-threaded platforms, so we can use a
    /// `maitake` scheduler capable of running `!Send` futures.
    scheduler: LocalScheduler,

    /// Maitake timer wheel.
    timer: Timer,
}

// TODO: This is a workaround because the SETTINGS should always exist, even
// if the feature doesn't. I think.
pub mod serial_trace_settings {
    use serde::{Serialize, Deserialize};
    use tracing::metadata::LevelFilter;

    use crate::services;


    #[derive(Debug, Serialize, Deserialize)]
    #[non_exhaustive]
    pub struct SerialTraceSettings {
        /// SerialMux port for sermux tracing.
        pub port: u16,

        /// Capacity for the serial port's send buffer.
        pub sendbuf_capacity: usize,

        /// Capacity for the trace ring buffer.
        ///
        /// Note that *two* buffers of this size will be allocated. One buffer is
        /// used for the normal trace ring buffer, and another is used for the
        /// interrupt service routine trace ring buffer.
        pub tracebuf_capacity: usize,

        /// Initial level filter used if the debug host does not select a max level.
        #[serde(with = "level_filter")]
        pub initial_level: tracing::metadata::LevelFilter,
    }

    pub const fn level_to_u8(level: tracing::metadata::LevelFilter) -> u8 {
        match level {
            tracing::metadata::LevelFilter::TRACE => 0,
            tracing::metadata::LevelFilter::DEBUG => 1,
            tracing::metadata::LevelFilter::INFO => 2,
            tracing::metadata::LevelFilter::WARN => 3,
            tracing::metadata::LevelFilter::ERROR => 4,
            tracing::metadata::LevelFilter::OFF => 5,
        }
    }

    pub const fn u8_to_level(level: u8) -> tracing::metadata::LevelFilter {
        match level {
            0 => tracing::metadata::LevelFilter::TRACE,
            1 => tracing::metadata::LevelFilter::DEBUG,
            2 => tracing::metadata::LevelFilter::INFO,
            3 => tracing::metadata::LevelFilter::WARN,
            4 => tracing::metadata::LevelFilter::ERROR,
            _ => tracing::metadata::LevelFilter::OFF,
        }
    }

    mod level_filter {
        use serde::{de::Visitor, Deserializer, Serializer};

        use super::{level_to_u8, u8_to_level};

        pub fn serialize<S>(lf: &tracing::metadata::LevelFilter, s: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let lf_u8: u8 = level_to_u8(*lf);
            s.serialize_u8(lf_u8)
        }

        pub fn deserialize<'de, D>(d: D) -> Result<tracing::metadata::LevelFilter, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct LFVisitor;
            impl<'de> Visitor<'de> for LFVisitor {
                type Value = tracing::metadata::LevelFilter;

                fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
                    formatter.write_str("a level filter as a u8 value")
                }

                fn visit_u8<E>(self, v: u8) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    Ok(u8_to_level(v))
                }
            }

            d.deserialize_u8(LFVisitor)
        }
    }

    // === impl SermuxTraceSettings ===

    impl SerialTraceSettings {
        pub const DEFAULT_PORT: u16 = services::serial_mux::WellKnown::BinaryTracing as u16;
        pub const DEFAULT_SENDBUF_CAPACITY: usize = 1024;
        pub const DEFAULT_TRACEBUF_CAPACITY: usize = Self::DEFAULT_SENDBUF_CAPACITY * 64;
        pub const DEFAULT_INITIAL_LEVEL: LevelFilter = LevelFilter::OFF;

        #[must_use]
        pub const fn new() -> Self {
            Self {
                port: Self::DEFAULT_PORT,
                sendbuf_capacity: Self::DEFAULT_SENDBUF_CAPACITY,
                tracebuf_capacity: Self::DEFAULT_TRACEBUF_CAPACITY,
                initial_level: Self::DEFAULT_INITIAL_LEVEL,
            }
        }

        /// Sets the [`serial_mux`] port on which the binary tracing service is
        /// served.
        ///
        /// By default, this is [`Self::DEFAULT_PORT`] (the value of
        /// [`serial_mux::WellKnown::BinaryTracing`]).
        #[must_use]
        pub fn with_port(self, port: impl Into<u16>) -> Self {
            Self {
                port: port.into(),
                ..self
            }
        }

        /// Sets the initial [`LevelFilter`] used when no trace client is connected
        /// or when the trace client does not select a level.
        ///
        /// By default, this set to [`Self::DEFAULT_INITIAL_LEVEL`] ([`LevelFilter::OFF`]).
        #[must_use]
        pub fn with_initial_level(self, level: impl Into<LevelFilter>) -> Self {
            Self {
                initial_level: level.into(),
                ..self
            }
        }

        /// Sets the maximum capacity of the serial port send buffer (the buffer
        /// used for communication between the trace service task and the serial mux
        /// server).
        ///
        /// By default, this set to [`Self::DEFAULT_SENDBUF_CAPACITY`] (1 KB).
        #[must_use]
        pub const fn with_sendbuf_capacity(self, capacity: usize) -> Self {
            Self {
                sendbuf_capacity: capacity,
                ..self
            }
        }

        /// Sets the maximum capacity of the trace ring buffer (the buffer into
        /// which new traces are serialized before being sent to the worker task).
        ///
        /// Note that *two* buffers of this size will be allocated. One buffer is
        /// used for traces emitted by non-interrupt kernel code, and the other is
        /// used for traces emitted inside of interrupt service routines (ISRs).
        ///
        /// By default, this set to [`Self::DEFAULT_TRACEBUF_CAPACITY`] (64 KB).
        #[must_use]
        pub const fn with_tracebuf_capacity(self, capacity: usize) -> Self {
            Self {
                tracebuf_capacity: capacity,
                ..self
            }
        }
    }

    impl Default for SerialTraceSettings {
        fn default() -> Self {
            Self::new()
        }
    }
}

/// Settings for all services spawned by default.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DefaultServiceSettings<'a> {
    pub keyboard_mux: Option<KeyboardMuxSettings>,
    pub serial_mux: Option<SerialMuxSettings>,
    pub spawnulator: Option<SpawnulatorSettings>,
    pub sermux_loopback: Option<daemons::sermux::LoopbackSettings>,
    #[serde(borrow)]
    pub sermux_hello: Option<daemons::sermux::HelloSettings<'a>>,
    pub sermux_trace: Option<serial_trace_settings::SerialTraceSettings>,
}

impl Kernel {
    /// Create a new kernel with the given settings.
    ///
    /// The allocator MUST be initialized if required, and be ready to allocate
    /// data.
    pub unsafe fn new(settings: KernelSettings) -> Result<Box<Self>, &'static str> {
        let registry = registry::Registry::new(settings.max_drivers);

        let scheduler = LocalScheduler::new();

        let inner = KernelInner {
            scheduler,
            timer: Timer::new(settings.timer_granularity),
        };

        let new_kernel = Box::try_new(Kernel {
            inner,
            registry: Mutex::new(registry),
        })
        .map_err(|_| "Kernel allocation failed.")?;

        Ok(new_kernel)
    }

    fn inner(&'static self) -> &'static KernelInner {
        &self.inner
    }

    #[inline]
    #[must_use]
    pub fn timer(&'static self) -> &'static Timer {
        &self.inner.timer
    }

    pub fn tick(&'static self) -> maitake::scheduler::Tick {
        let inner = self.inner();
        inner.scheduler.tick()
        // TODO: Send time to userspace?
    }

    /// Initialize the kernel's `maitake` timer as the global default timer.
    ///
    /// This allows the use of `sleep` and `timeout` free functions.
    /// TODO(eliza): can the kernel just "do this" once it becomes active? Or,
    /// have a "kernel.init()" or something that does this and other global inits?
    pub fn set_global_timer(&'static self) -> Result<(), maitake::time::AlreadyInitialized> {
        maitake::time::set_global_timer(self.timer())
    }

    #[track_caller]
    pub fn initialize<F>(&'static self, fut: F) -> Result<JoinHandle<F::Output>, &'static str>
    where
        F: Future + 'static,
    {
        Ok(self.inner.scheduler.spawn(fut))
    }

    pub async fn spawn<F>(&'static self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        let bx = Box::new(maitake::task::Task::new(fut))
            .await
            .into_alloc_box();
        self.spawn_allocated(bx)
    }

    pub async fn with_registry<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&mut Registry) -> R,
    {
        let mut guard = self.registry.lock().await;
        f(&mut guard)
    }

    #[track_caller]
    pub fn spawn_allocated<F>(
        &'static self,
        task: <BoxStorage as Storage<LocalScheduler, F>>::StoredTask,
    ) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        self.inner.scheduler.spawn_allocated(task)
    }

    /// Returns a [`Sleep`] future that sleeps for the specified [`Duration`].
    #[inline]
    pub fn sleep(&'static self, duration: Duration) -> Sleep<'static> {
        self.inner.timer.sleep(duration)
    }

    /// Returns a [`Timeout`] future that cancels `F` if the specified
    /// [`Duration`] has elapsed before it completes.
    #[inline]
    pub fn timeout<F: Future>(&'static self, duration: Duration, f: F) -> Timeout<'static, F> {
        self.inner.timer.timeout(duration, f)
    }

    /// Initialize the default set of cross-platform kernel [`services`] that
    /// are spawned on all hardware platforms.
    ///
    /// Calling this method is not *mandatory* for a hardware platform
    /// implementation. The platform implementation may manually spawn these
    /// services individually, or choose not to spawn them at all. However, this
    /// method is provided to ensure that a consistent set of cross-platform
    /// services are initialized on all hardware platforms *if they are
    /// desired*.
    ///
    /// Services spawned by this method include:
    ///
    /// - The [`KeyboardMuxService`], which multiplexes keyboard input from
    ///   multiple keyboards to tasks that depend on keyboard input,
    /// - The [`SerialMuxService`], which multiplexes serial I/O to virtual
    ///   serial ports
    /// - The [`SpawnulatorService`], which is responsible for spawning
    ///   new Forth tasks
    ///
    /// In addition, this method will initialize the following non-service
    /// daemons:
    ///
    /// - [`daemons::sermux::loopback`], which serves a loopback service on a
    ///   configured loopback port
    /// - [`daemons::sermux::hello`], which sends periodic "hello world" pings
    ///   to a configured serial mux port
    /// - If the "serial-trace" feature flag is enabled, the
    ///   [`serial_trace::SerialSubscriber`] worker task, which sends `tracing`
    ///   events over the serial port.
    ///
    /// If the kernel's [`maitake::time::Timer`] has not been set as the global
    /// timer, this method will also ensure that the global timer is set as the
    /// default.
    ///
    /// [`KeyboardMuxService`]:
    ///     crate::services::keyboard::mux::KeyboardMuxService
    /// [`SerialMuxService`]: crate::services::serial_mux::SerialMuxService
    /// [`SpawnulatorService`]:
    ///     crate::services::forth_spawnulator::SpawnulatorService
    pub fn initialize_default_services(&'static self, settings: DefaultServiceSettings<'static>) {
        // Set the kernel timer as the global timer.
        // Disregard errors --- they just mean someone else has already set up
        // the global timer.
        let _ = self.set_global_timer();

        // Initialize the kernel keyboard mux service.
        if let Some(keyboard_mux) = settings.keyboard_mux {
            self.initialize(KeyboardMuxServer::register(self, keyboard_mux))
                .expect("failed to spawn KeyboardMuxService initialization");
            }

        // Initialize the SerialMuxServer
        let sermux_up = if let Some(serial_mux) = settings.serial_mux {
            Some(self
                .initialize(SerialMuxServer::register(self, serial_mux))
                .expect("failed to spawn SerialMuxService initialization"))
        } else {
            None
        };

        // Initialize the Forth spawnulator.
        if let Some(spawnulator) = settings.spawnulator {
            self.initialize(SpawnulatorServer::register(self, spawnulator))
                .expect("failed to spawn SpawnulatorService initialization");
        }

        // Initialize Serial Mux daemons.
        if let Some(sermux_up) = sermux_up {
            self.initialize(async move {
                sermux_up
                    .await
                    .expect("SerialMuxService initialization should not be cancelled")
                    .expect("SerialMuxService initialization failed");

                #[cfg(feature = "serial-trace")]
                if let Some(sermux_trace) = settings.sermux_trace {
                    crate::serial_trace::SerialSubscriber::start(self, sermux_trace).await;
                }

                if let Some(sermux_loopback) = settings.sermux_loopback {
                    self.spawn(daemons::sermux::loopback(self, sermux_loopback))
                        .await;
                    tracing::debug!("SerMux loopback started");
                }

                if let Some(sermux_hello) = settings.sermux_hello {
                    self.spawn(daemons::sermux::hello(self, sermux_hello))
                        .await;
                    tracing::debug!("SerMux Hello World started");
                }
            })
            .expect("failed to spawn default serial mux service initialization");
        } else {
            let deps = [
                #[cfg(feature = "serial-trace")]
                settings.sermux_trace.is_some(),
                settings.sermux_loopback.is_some(),
                settings.sermux_hello.is_some()
            ];

            if deps.into_iter().any(identity) {
                tracing::error!("Sermux services configured without sermux! Skipping.");
            }
        }
    }
}
