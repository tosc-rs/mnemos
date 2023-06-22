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
//! ## Creating the kernel
//!
//! To create the kernel, you give it a region of memory (as a `*mut u8` + `usize`), by calling
//! [`Kernel::new()`].
//!
//! At this point, the system is in "blocking" mode.
//!
//! Using the given region of memory, the kernel bootstraps itself, and creates the following:
//!
//! * An async executor, intended for "kernel services"
//! * A kernel service discovery registry
//! * An async heap allocator, intended for use by kernel services
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

pub mod comms;
pub mod drivers;
pub(crate) mod fmt;
pub mod forth;
pub mod registry;
#[cfg(feature = "tracing-02")]
pub mod trace;

use abi::{
    bbqueue_ipc::{
        framed::{FrameConsumer, FrameProducer},
        BBBuffer,
    },
    syscall::{KernelResponse, UserRequest},
};
use comms::{bbq::BidiHandle, kchannel::KChannel};
pub use maitake;
use maitake::{
    scheduler::{LocalStaticScheduler, TaskStub},
    sync::Mutex,
    task::{JoinHandle, Storage, Task as MaitakeTask},
    time::{Duration, Sleep, Timeout, Timer},
};
pub use mnemos_alloc;
use mnemos_alloc::{containers::HeapBox, heap::AHeap};
use registry::Registry;

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

use crate::tracing::info;

pub struct Rings {
    pub u2k: NonNull<BBBuffer>,
    pub k2u: NonNull<BBBuffer>,
}

pub struct KernelSettings {
    pub heap_start: *mut u8,
    pub heap_size: usize,
    pub max_drivers: usize,
    pub k2u_size: usize,
    pub u2k_size: usize,
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
    heap: NonNull<AHeap>,
}

unsafe impl Sync for Kernel {}

pub struct KernelInner {
    u2k_ring: BBBuffer,
    k2u_ring: BBBuffer,
    /// MnemOS currently only targets single-threaded platforms, so we can use a
    /// `maitake` scheduler capable of running `!Send` futures.
    scheduler: LocalStaticScheduler,

    /// Maitake timer wheel.
    timer: Timer,
}

impl Kernel {
    pub unsafe fn new(settings: KernelSettings) -> Result<HeapBox<Self>, &'static str> {
        info!(
            start = ?settings.heap_start,
            size = settings.heap_size,
            "Initializing heap"
        );
        let (nn_heap, mut guard) = AHeap::bootstrap(settings.heap_start, settings.heap_size)
            .map_err(|_| "failed to initialize heap")?;

        let registry = registry::Registry::new(&mut guard, settings.max_drivers);
        let (nn_u2k_buf, u2k_len) = guard
            .alloc_box_array_with(|| 0, settings.u2k_size)
            .map_err(|_| "failed to allocate u2k ring buf")?
            .leak();
        let (nn_k2u_buf, k2u_len) = guard
            .alloc_box_array_with(|| 0, settings.k2u_size)
            .map_err(|_| "failed to allocate k2u ring buf")?
            .leak();
        let u2k_ring = BBBuffer::new();
        let k2u_ring = BBBuffer::new();

        // SAFETY: The data buffers live in a heap allocation, which have a stable
        // location. Therefore it is acceptable to initialize the rings using these
        // buffers, then moving the HANDLES into the KernelInner structure.
        //
        // The BBBuffers themselves ONLY have a stable address AFTER they have been
        // written into the static `inner` field. DO NOT create producers/consumers
        // until that has happened.
        u2k_ring.initialize(nn_u2k_buf.as_ptr(), u2k_len);
        k2u_ring.initialize(nn_k2u_buf.as_ptr(), k2u_len);

        // Safety: We only use the static stub once
        let stub: &'static TaskStub = guard
            .alloc_box(TaskStub::new())
            .map_err(|_| "failed to allocate task stub")?
            .leak()
            .as_ref();
        let scheduler = LocalStaticScheduler::new_with_static_stub(stub);
        let timer = Timer::new(settings.timer_granularity);

        let inner = KernelInner {
            u2k_ring,
            k2u_ring,
            scheduler,
            timer,
        };

        let new_kernel = guard
            .alloc_box(Kernel {
                inner,
                registry: Mutex::new(registry),
                heap: nn_heap,
            })
            .map_err(|_| "failed to allocate new kernel box")?;

        Ok(new_kernel)
    }

    fn inner(&'static self) -> &'static KernelInner {
        &self.inner
    }

    pub fn rings(&'static self) -> Rings {
        unsafe {
            Rings {
                u2k: NonNull::new_unchecked(&self.inner.u2k_ring as *const _ as *mut _),
                k2u: NonNull::new_unchecked(&self.inner.k2u_ring as *const _ as *mut _),
            }
        }
    }

    pub fn heap(&'static self) -> &'static AHeap {
        unsafe { self.heap.as_ref() }
    }

    #[inline]
    #[must_use]
    pub fn timer(&'static self) -> &'static Timer {
        &self.inner.timer
    }

    pub fn tick(&'static self) -> maitake::scheduler::Tick {
        // Process heap allocations
        self.heap().poll();

        // process mailbox messages
        let inner = self.inner();
        let u2k_buf: *mut BBBuffer = &self.inner.u2k_ring as *const _ as *mut _;
        let k2u_buf: *mut BBBuffer = &self.inner.k2u_ring as *const _ as *mut _;
        let u2k: FrameConsumer<'static> = unsafe { BBBuffer::take_framed_consumer(u2k_buf) };
        let _k2u: FrameProducer<'static> = unsafe { BBBuffer::take_framed_producer(k2u_buf) };

        #[allow(unreachable_code)]
        if let Some(mut _reg) = self.registry.try_lock() {
            // Incoming messages
            while let Some(msg) = u2k.read() {
                match postcard::from_bytes::<UserRequest>(&msg) {
                    Ok(_req) => {
                        // let kind = req.driver_kind();
                        // if let Some(drv) = inner_mut.drivers.iter().find(|drv| drv.kind == kind) {
                        //     drv.queue
                        //         .enqueue_sync(Message {
                        //             request: req,
                        //             response: inner.user_reply.clone(),
                        //         })
                        //         .map_err(drop)
                        //         .unwrap();
                        // }
                        todo!("Driver registry");
                    }
                    Err(_) => panic!(),
                }
                msg.release();
            }
        }

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

    /// Spawn the initial Forth task (task ID 0), returning a [`BidiHandle`] for
    /// the spawned task's standard input and standard output streams.
    ///
    /// This should only be called once.
    pub fn initialize_forth_tid0(&'static self, params: forth::Params) -> JoinHandle<BidiHandle> {
        use forth::{Forth, Spawnulator};
        self.initialize(async move {
            tracing::debug!("spawning Task 0...");
            // TODO(eliza): maybe the spawnulator should live in the driver registry
            // or something.
            let spawnulator = Spawnulator::start_spawnulating(self).await;
            let (tid0, tid0_streams) = Forth::new(self, params, spawnulator)
                .await
                .expect("spawning forth TID0 should succeed");
            self.spawn(tid0.run()).await;
            tracing::info!("Task 0 spawned!");
            tid0_streams
        })
        .expect("spawning forth TID0 should succeed")
    }

    pub fn initialize<F>(&'static self, fut: F) -> Result<JoinHandle<F::Output>, &'static str>
    where
        F: Future + 'static,
    {
        let task = self.new_task(fut);
        let mut guard = self
            .heap()
            .lock()
            .map_err(|_| "kernel heap already locked")?;
        let task_box = guard
            .alloc_box(task)
            .map_err(|_| "could not allocate task storage")?;
        Ok(self.spawn_allocated(task_box))
    }

    pub fn new_task<F>(&'static self, fut: F) -> Task<F>
    where
        F: Future + 'static,
    {
        Task(MaitakeTask::new(fut))
    }

    pub async fn spawn<F>(&'static self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        let task = Task(MaitakeTask::new(fut));
        let atask = self.heap().allocate(task).await;
        self.spawn_allocated(atask)
    }

    pub async fn with_registry<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&mut Registry) -> R,
    {
        let mut guard = self.registry.lock().await;
        f(&mut guard)
    }

    pub fn spawn_allocated<F>(&'static self, task: HeapBox<Task<F>>) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        self.inner.scheduler.spawn_allocated::<F, HBStorage>(task)
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
}

// TODO: De-dupe with userspace?
use core::{future::Future, ptr::NonNull};

#[repr(transparent)]
pub struct Task<F: Future + 'static>(MaitakeTask<&'static LocalStaticScheduler, F, HBStorage>);

struct HBStorage;

impl<F: Future + 'static> Storage<&'static LocalStaticScheduler, F> for HBStorage {
    type StoredTask = HeapBox<Task<F>>;

    fn into_raw(
        task: HeapBox<Task<F>>,
    ) -> NonNull<MaitakeTask<&'static LocalStaticScheduler, F, Self>> {
        task.leak()
            .cast::<MaitakeTask<&'static LocalStaticScheduler, F, HBStorage>>()
    }

    fn from_raw(
        ptr: NonNull<MaitakeTask<&'static LocalStaticScheduler, F, Self>>,
    ) -> HeapBox<Task<F>> {
        unsafe { HeapBox::from_leaked(ptr.cast::<Task<F>>()) }
    }
}
