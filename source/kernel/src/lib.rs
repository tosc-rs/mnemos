#![no_std]

pub mod bbq;

use abi::{
    bbqueue_ipc::{
        framed::{FrameConsumer, FrameProducer},
        BBBuffer,
    },
    syscall::{DriverKind, KernelResponse, UserRequest},
};
use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};
use maitake::task::Task as MaitakeTask;
use maitake::{
    self,
    scheduler::{StaticScheduler, TaskStub},
    task::Storage,
};
use mnemos_alloc::{
    containers::{HeapArc, HeapArray, HeapBox, HeapFixedVec},
    heap::{AHeap, HeapGuard},
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
    status: AtomicUsize,
    // Items that do not require a lock to access, and must only
    // be accessed with shared refs
    inner: KernelInner,
    // Items that require mutex'd access, and allow mutable access
    inner_mut: UnsafeCell<KernelInnerMut>,
    heap: NonNull<AHeap>,
}

unsafe impl Sync for Kernel {}

pub struct DriverHandle {
    pub kind: DriverKind,
    pub queue: KChannel<Message>,
}

pub struct KernelInner {
    u2k_ring: BBBuffer,
    k2u_ring: BBBuffer,
    scheduler: StaticScheduler,
    user_reply: KChannel<KernelResponse>,
}

pub struct KernelInnerMut {
    drivers: HeapFixedVec<DriverHandle>,
}

impl Kernel {
    const INITIALIZING: usize = 1;
    const INIT_IDLE: usize = 2;
    const INIT_LOCK: usize = 3;

    pub unsafe fn new(settings: KernelSettings) -> Result<HeapBox<Self>, ()> {
        info!(
            start = ?settings.heap_start,
            size = settings.heap_size,
            "Initializing heap"
        );
        let (nn_heap, mut guard) = AHeap::bootstrap(settings.heap_start, settings.heap_size)?;

        let drivers = guard.alloc_fixed_vec(settings.max_drivers)?;
        let (nn_u2k_buf, u2k_len) = guard.alloc_box_array_with(|| 0, settings.u2k_size)?.leak();
        let (nn_k2u_buf, k2u_len) = guard.alloc_box_array_with(|| 0, settings.k2u_size)?.leak();
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
            .map_err(drop)?
            .leak()
            .as_ref();
        let scheduler = StaticScheduler::new_with_static_stub(stub);

        let inner = KernelInner {
            u2k_ring,
            k2u_ring,
            scheduler,
            user_reply: KChannel::new(&mut guard, settings.user_reply_max_ct),
        };
        let inner_mut = KernelInnerMut { drivers };

        let new_kernel = guard
            .alloc_box(Kernel {
                status: AtomicUsize::new(Kernel::INITIALIZING),
                inner,
                inner_mut: UnsafeCell::new(inner_mut),
                heap: nn_heap,
            })
            .map_err(drop)?;

        new_kernel.status.store(Self::INIT_IDLE, Ordering::SeqCst);

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

    fn inner_mut(&'static self) -> Result<KimGuard, ()> {
        self.status
            .compare_exchange(
                Self::INIT_IDLE,
                Self::INIT_LOCK,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map_err(drop)?;

        Ok(KimGuard {
            kim: NonNull::new(self as *const Self as *mut Self).ok_or(())?,
        })
    }

    pub fn register_driver(&'static self, hdl: DriverHandle) -> Result<(), DriverHandle> {
        let mut guard = match self.inner_mut() {
            Ok(g) => g,
            Err(_) => return Err(hdl),
        };

        // TODO: for now we only support a single instance of each driver
        if guard.drivers.iter().find(|d| d.kind == hdl.kind).is_some() {
            return Err(hdl);
        }

        guard.drivers.push(hdl)
    }

    pub fn heap(&'static self) -> &'static AHeap {
        unsafe { self.heap.as_ref() }
    }

    pub fn tick(&'static self) {
        // Process heap allocations
        self.heap().poll();

        // process mailbox messages
        let inner = self.inner();
        let inner_mut = self.inner_mut().unwrap();
        let u2k_buf: *mut BBBuffer = &self.inner.u2k_ring as *const _ as *mut _;
        let k2u_buf: *mut BBBuffer = &self.inner.k2u_ring as *const _ as *mut _;
        let u2k: FrameConsumer<'static> = unsafe { BBBuffer::take_framed_consumer(u2k_buf) };
        let k2u: FrameProducer<'static> = unsafe { BBBuffer::take_framed_producer(k2u_buf) };

        // Incoming messages
        while let Some(msg) = u2k.read() {
            match postcard::from_bytes::<UserRequest>(&msg) {
                Ok(req) => {
                    let kind = req.driver_kind();
                    if let Some(drv) = inner_mut.drivers.iter().find(|drv| drv.kind == kind) {
                        drv.queue
                            .enqueue_sync(Message {
                                request: req,
                                response: inner.user_reply.clone(),
                            })
                            .map_err(drop)
                            .unwrap();
                    }
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

    pub fn spawn_allocated<F: Future + 'static>(&'static self, task: HeapBox<Task<F>>) {
        self.inner.scheduler.spawn_allocated::<F, HBStorage>(task)
    }
}

pub struct KimGuard {
    kim: NonNull<Kernel>,
}

impl Deref for KimGuard {
    type Target = KernelInnerMut;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.kim.as_ref().inner_mut.get() }
    }
}

impl DerefMut for KimGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.kim.as_mut().inner_mut.get() }
    }
}

impl Drop for KimGuard {
    fn drop(&mut self) {
        unsafe {
            self.kim
                .as_ref()
                .status
                .store(Kernel::INIT_IDLE, Ordering::SeqCst);
        }
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

use spitebuf::MpMcQueue;

mod sealed {
    use super::*;

    pub struct SpiteData<T> {
        pub(crate) data: HeapArray<UnsafeCell<spitebuf::Cell<T>>>,
    }

    unsafe impl<T: Sized> spitebuf::Storage<T> for SpiteData<T> {
        fn buf(&self) -> (*const UnsafeCell<spitebuf::Cell<T>>, usize) {
            let ptr = self.data.as_ptr();
            let len = self.data.len();
            (ptr, len)
        }
    }
}

pub struct KChannel<T> {
    q: HeapArc<MpMcQueue<T, sealed::SpiteData<T>>>,
}

impl<T> Clone for KChannel<T> {
    fn clone(&self) -> Self {
        Self { q: self.q.clone() }
    }
}

impl<T> Deref for KChannel<T> {
    type Target = MpMcQueue<T, sealed::SpiteData<T>>;

    fn deref(&self) -> &Self::Target {
        &self.q
    }
}

impl<T> KChannel<T> {
    pub fn new(guard: &mut HeapGuard, count: usize) -> Self {
        let func = || UnsafeCell::new(spitebuf::single_cell::<T>());

        let ba = guard.alloc_box_array_with(func, count).unwrap();
        let q = MpMcQueue::new(sealed::SpiteData { data: ba });
        Self {
            q: guard.alloc_arc(q).map_err(drop).unwrap(),
        }
    }
}
