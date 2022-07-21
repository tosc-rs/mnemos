#![no_std]
#![allow(clippy::missing_safety_doc)]

pub mod comms;
pub mod drivers;
pub mod registry;
pub(crate) mod fmt;

use abi::{
    bbqueue_ipc::{
        framed::{FrameConsumer, FrameProducer},
        BBBuffer,
    },
    syscall::{DriverKind, KernelResponse, UserRequest},
};
use comms::kchannel::KChannel;
use registry::Registry;
use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};
use maitake::{task::Task as MaitakeTask, sync::{Mutex, MutexGuard}};
use maitake::{
    self,
    scheduler::{StaticScheduler, TaskStub},
    task::Storage,
};
use mnemos_alloc::{
    containers::HeapBox,
    heap::AHeap,
};
use tracing::info;

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
    pub user_reply_max_ct: usize,
}

pub struct Message {
    pub request: UserRequest,
    pub response: KChannel<KernelResponse>,
}

pub struct Kernel {
    // Items that do not require a lock to access, and must only
    // be accessed with shared refs
    inner: KernelInner,
    // Items that require mutex'd access, and allow mutable access
    registry: Mutex<Registry>,
    heap: NonNull<AHeap>,
}

unsafe impl Sync for Kernel {}

pub struct KernelInner {
    u2k_ring: BBBuffer,
    k2u_ring: BBBuffer,
    scheduler: StaticScheduler,
    user_reply: KChannel<KernelResponse>,
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
        let scheduler = StaticScheduler::new_with_static_stub(stub);

        let inner = KernelInner {
            u2k_ring,
            k2u_ring,
            scheduler,
            user_reply: KChannel::new(&mut guard, settings.user_reply_max_ct),
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

    pub fn tick(&'static self) {
        // Process heap allocations
        self.heap().poll();

        // process mailbox messages
        let inner = self.inner();
        let registry = self.registry.try_lock().unwrap();
        let u2k_buf: *mut BBBuffer = &self.inner.u2k_ring as *const _ as *mut _;
        let k2u_buf: *mut BBBuffer = &self.inner.k2u_ring as *const _ as *mut _;
        let u2k: FrameConsumer<'static> = unsafe { BBBuffer::take_framed_consumer(u2k_buf) };
        let k2u: FrameProducer<'static> = unsafe { BBBuffer::take_framed_producer(k2u_buf) };

        // Incoming messages
        while let Some(msg) = u2k.read() {
            match postcard::from_bytes::<UserRequest>(&msg) {
                Ok(req) => {
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

        // Outgoing messages
        while let Ok(mut grant) = k2u.grant(256) {
            match inner.user_reply.dequeue_sync() {
                Some(msg) => {
                    let used = postcard::to_slice(&msg, &mut grant).unwrap().len();

                    grant.commit(used);
                }
                None => break,
            }
        }

        inner.scheduler.tick();

        // TODO: Send time to userspace?
    }

    pub fn new_task<F: Future + 'static>(&'static self, fut: F) -> Task<F> {
        Task(MaitakeTask::new(&self.inner.scheduler, fut))
    }

    pub async fn spawn<F: Future + 'static>(&'static self, fut: F) {
        let task = Task(MaitakeTask::new(&self.inner.scheduler, fut));
        let atask = self.heap().allocate(task).await;
        self.spawn_allocated(atask);
    }

    pub async fn with_registry<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&mut Registry) -> R,
    {
        let mut guard = self.registry.lock().await;
        f(&mut guard)
    }

    pub fn spawn_allocated<F: Future + 'static>(&'static self, task: HeapBox<Task<F>>) {
        self.inner.scheduler.spawn_allocated::<F, HBStorage>(task)
    }
}

// TODO: De-dupe with userspace?
use core::{future::Future, ptr::NonNull};

#[repr(transparent)]
pub struct Task<F: Future + 'static>(MaitakeTask<&'static StaticScheduler, F, HBStorage>);

struct HBStorage;

impl<F: Future + 'static> Storage<&'static StaticScheduler, F> for HBStorage {
    type StoredTask = HeapBox<Task<F>>;

    fn into_raw(task: HeapBox<Task<F>>) -> NonNull<MaitakeTask<&'static StaticScheduler, F, Self>> {
        task.leak()
            .cast::<MaitakeTask<&'static StaticScheduler, F, HBStorage>>()
    }

    fn from_raw(ptr: NonNull<MaitakeTask<&'static StaticScheduler, F, Self>>) -> HeapBox<Task<F>> {
        unsafe { HeapBox::from_leaked(ptr.cast::<Task<F>>()) }
    }
}
