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
pub mod services;
#[cfg(feature = "tracing-02")]
pub mod trace;

use abi::{
    bbqueue_ipc::BBBuffer,
    syscall::{KernelResponse, UserRequest},
};
use comms::kchannel::KChannel;
use core::{future::Future, ptr::NonNull};
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
use services::{
    forth_spawnulator::{SpawnulatorServer, SpawnulatorSettings},
    keyboard::mux::{KeyboardMuxServer, KeyboardMuxSettings},
    serial_mux::{SerialMuxServer, SerialMuxSettings},
};

/// Shim to handle tracing v0.1 vs v0.2
///
/// ## NOTE for features used
///
/// Unfortunately, we can't support the case where tracing 0.1 and 0.2 are both selected
/// yet. This might be changed in the future. The following truth table shows the outcome
/// when you select various feature flags.
///
/// The `_oops_all_tracing_features` feature is a "trap" for when the package is built
/// with `--all-features`, which is usually just for docs and testing. In that case, the
/// feature then ignores the other feature settings, and just picks tracing-01. This is
/// an unfortunate hack that works too well not to use for now.
///
/// | `_oops_all_tracing_features`  | `tracing-01`  | `tracing-02`  | Outcome           |
/// | :---:                         | :---:         | :---:         | :---:             |
/// | true                          | don't care    | don't care    | `tracing-01` used |
/// | false                         | false         | false         | Compile Error     |
/// | false                         | false         | true          | `tracing-02` used |
/// | false                         | true          | false         | `tracing-01` used |
/// | false                         | true          | true          | Compile Error     |
///
pub(crate) mod tracing {
    #[cfg(all(
        not(feature = "_oops_all_tracing_features"),
        all(feature = "tracing-01", feature = "tracing-02")
    ))]
    compile_error!("Must select one of 'tracing-01' or 'tracing-02' features!");

    #[cfg(any(feature = "_oops_all_tracing_features", feature = "tracing-01"))]
    pub use tracing_01::*;

    #[cfg(all(not(feature = "_oops_all_tracing_features"), feature = "tracing-02"))]
    pub use tracing_02::*;
}

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

/// Settings for all services spawned by default.
#[derive(Debug, Default)]
pub struct DefaultServiceSettings {
    pub keyboard_mux: KeyboardMuxSettings,
    pub serial_mux: SerialMuxSettings,
    pub spawnulator: SpawnulatorSettings,
    pub sermux_loopback: daemons::sermux::LoopbackSettings,
    pub sermux_hello: daemons::sermux::HelloSettings,
    #[cfg(feature = "tracing-02")]
    pub sermux_trace: trace::SerialTraceSettings,
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
    /// - if the "tracing-02" feature flag is enabled, the worker task for the
    ///   binary serial mux trace protocol
    ///
    /// If the kernel's [`maitake::time::Timer`] has not been set as the global
    /// timer, this method will also ensure that the global timer is set as the
    /// default.
    pub fn initialize_default_services(&'static self, settings: DefaultServiceSettings) {
        // Set the kernel timer as the global timer.
        // Disregard errors --- they just mean someone else has already set up
        // the global timer.
        let _ = self.set_global_timer();

        // Initialize the kernel keyboard mux service.
        self.initialize(KeyboardMuxServer::register(self, settings.keyboard_mux))
            .expect("failed to spawn KeyboardMuxService initialization");

        // Initialize the SerialMuxServer
        let sermux_up = self
            .initialize(SerialMuxServer::register(self, settings.serial_mux))
            .expect("failed to spawn SerialMuxService initialization");

        // Initialize the Forth spawnulator.
        self.initialize(SpawnulatorServer::register(self, settings.spawnulator))
            .expect("failed to spawn SpawnulatorService initialization");

        // Initialize Serial Mux daemons.
        self.initialize(async move {
            sermux_up
                .await
                .expect("SerialMuxService initialization should not be cancelled")
                .expect("SerialMuxService initialization failed");

            #[cfg(feature = "tracing-02")]
            crate::trace::COLLECTOR
                .start(self, settings.sermux_trace)
                .await;

            self.spawn(daemons::sermux::loopback(self, settings.sermux_loopback))
                .await;
            tracing::debug!("SerMux loopback started");

            self.spawn(daemons::sermux::hello(self, settings.sermux_hello))
                .await;
            tracing::debug!("SerMux Hello World started");
        })
        .expect("failed to spawn default serial mux service initialization");
    }
}
