#![no_std]

/*

Okay, what does the kernel need to be?

* It needs to allocate some global resources:
    * U2K and K2U rings
        * I guess these could be heap allocated
    * Some space to store driver handles
    * A (bump) allocator for drivers?
        * bump allocator means no hotplug
* It should probably be a single static object
* Users should probably boot, then:
    * Initalize(?) the kernel
    * Initialize and create the drivers
        * More on this later
        * Provides the following in a handle:
            * The driver task (alloc'd, not running)
                * Maybe this is just a separate call to spawn?
                * What if drivers want multiple tasks?
            * A Thingbuf producer handle
            * A function that does the NAK-on-full behavior
            * Some way to determine which messages to handle
    * Register them with the kernel
    * Begin operation
* There needs to be some way of launching programs

*/

use core::{sync::atomic::{AtomicUsize, Ordering}, mem::MaybeUninit, cell::UnsafeCell, ops::{Deref, DerefMut}};
use mnemos_alloc::{HEAP, HeapFixedVec, HeapBox, HeapArray, AHeap};
use abi::{bbqueue_ipc::{BBBuffer, framed::{FrameProducer, FrameConsumer}}, syscall::{DriverKind, UserRequest, KernelResponse}};
use maitake::{self, scheduler::{StaticScheduler, TaskStub}, task::Storage};
use maitake::task::Task as MaitakeTask;
use thingbuf::StaticThingBuf;

static TASK_STUB: TaskStub = TaskStub::new();
static KERNEL: Kernel = Kernel {
    status: AtomicUsize::new(Kernel::UNINIT),
    inner: UnsafeCell::new(MaybeUninit::uninit()),
    inner_mut: UnsafeCell::new(MaybeUninit::uninit()),
    u2k: UnsafeCell::new(MaybeUninit::uninit()),
    k2u: UnsafeCell::new(MaybeUninit::uninit()),
};

pub struct KernelSettings {
    pub heap_start: usize,
    pub heap_size: usize,
    pub max_drivers: usize,
    pub k2u_size: usize,
    pub u2k_size: usize,
}

pub struct Message {
    request: UserRequest,
    response: &'static StaticThingBuf<KernelResponse, THINGBUF_CAP>,
}

// TODO: Make it possible to heap allocate a thingbuf capacity
pub const THINGBUF_CAP: usize = 32;

pub struct Kernel {
    status: AtomicUsize,
    // Items that do not require a lock to access, and must only
    // be accessed with shared refs
    inner: UnsafeCell<MaybeUninit<KernelInner>>,
    // Items that require mutex'd access, and allow mutable access
    inner_mut: UnsafeCell<MaybeUninit<KernelInnerMut>>,
    k2u: UnsafeCell<MaybeUninit<FrameProducer<'static>>>,
    u2k: UnsafeCell<MaybeUninit<FrameConsumer<'static>>>,
}

unsafe impl Sync for Kernel {}

pub struct DriverHandle {
    kind: DriverKind,
    // TODO: Some kind of HeapArc to better reference count this info
    queue: &'static StaticThingBuf<Message, THINGBUF_CAP>
}

pub struct KernelInner {
    u2k_buf: HeapArray<u8>,
    k2u_buf: HeapArray<u8>,
    u2k_ring: BBBuffer,
    k2u_ring: BBBuffer,
    scheduler: StaticScheduler,
    user_reply: StaticThingBuf<KernelResponse, 32>,
}

pub struct KernelInnerMut {
    drivers: HeapFixedVec<DriverHandle>,
}

impl Kernel {
    const UNINIT: usize = 0;
    const INITIALIZING: usize = 1;
    const INIT_IDLE: usize = 2;
    const INIT_LOCK: usize = 3;

    pub fn initialize(
        &'static self,
        settings: KernelSettings,
    ) -> Result<(), ()> {
        self.status.compare_exchange(
            Self::UNINIT,
            Self::INITIALIZING,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ).map_err(drop)?;

        let mut hr = HEAP.init_exclusive(
            settings.heap_start,
            settings.heap_size,
        )?;


        let drivers = hr.alloc_fixed_vec(settings.max_drivers)?;
        let mut u2k_buf = hr.alloc_box_array(0, settings.u2k_size)?;
        let mut k2u_buf = hr.alloc_box_array(0, settings.k2u_size)?;
        let u2k_ring = BBBuffer::new();
        let k2u_ring = BBBuffer::new();

        // SAFETY: The data buffers live in a heap allocation, which have a stable
        // location. Therefore it is acceptable to initialize the rings using these
        // buffers, then moving the HANDLES into the KernelInner structure.
        //
        // The BBBuffers themselves ONLY have a stable address AFTER they have been
        // written into the static `inner` field. DO NOT create producers/consumers
        // until that has happened.
        unsafe {
            u2k_ring.initialize(u2k_buf.as_mut_ptr(), u2k_buf.len());
            k2u_ring.initialize(k2u_buf.as_mut_ptr(), k2u_buf.len());
        }

        // Safety: We only use the static stub once
        let scheduler = unsafe { StaticScheduler::new_with_static_stub(&TASK_STUB) };

        let inner = KernelInner {
            u2k_buf,
            k2u_buf,
            u2k_ring,
            k2u_ring,
            scheduler,
            user_reply: StaticThingBuf::new(),
        };
        let inner_mut = KernelInnerMut {
            drivers,
        };

        unsafe {
            self.inner.get().write(MaybeUninit::new(inner));
            self.inner_mut.get().write(MaybeUninit::new(inner_mut));

            let inner = &mut (*self.inner.get().cast::<KernelInner>());

            let k2u_ring: *mut BBBuffer = &mut inner.k2u_ring;
            self.k2u.get().write(MaybeUninit::new(BBBuffer::take_framed_producer(k2u_ring)));

            let u2k_ring: *mut BBBuffer = &mut inner.u2k_ring;
            self.u2k.get().write(MaybeUninit::new(BBBuffer::take_framed_consumer(u2k_ring)));
        }



        self.status.store(Self::INIT_IDLE, Ordering::SeqCst);

        Ok(())
    }

    fn inner(&'static self) -> &'static KernelInner {
        let status = self.status.load(Ordering::SeqCst);
        if status == Self::UNINIT {
            panic!();
        }
        unsafe {
            &*self.inner.get().cast::<KernelInner>()
        }
    }

    fn inner_mut(&'static self) -> Result<KimGuard, ()> {
        self.status.compare_exchange(
            Self::INIT_IDLE,
            Self::INIT_LOCK,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ).map_err(drop)?;

        Ok(KimGuard {
            kim: self.inner_mut.get().cast(),
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

    // TODO: at some point I shouldn't make the heap a global static from the alloc crate.
    pub fn heap(&'static self) -> &'static AHeap {
        &HEAP
    }

    pub fn tick(&'static self) {
        self.heap().poll();

        // TODO: process mailbox messages
        let inner = self.inner();
        let inner_mut = self.inner_mut().unwrap();
        let u2k = unsafe { (*self.u2k.get()).assume_init_ref() };
        let k2u = unsafe { (*self.k2u.get()).assume_init_ref() };
        while let Some(msg) = u2k.read() {
            match postcard::from_bytes::<UserRequest>(&msg) {
                Ok(req) => {
                    let kind = req.driver_kind();
                    if let Some(drv) = inner_mut.drivers.iter().find(|drv| drv.kind == kind) {
                        drv.queue.push(Message { request: req, response: &inner.user_reply }).map_err(drop).unwrap();
                    }
                },
                Err(_) => panic!(),
            }

            msg.release();
        }


        inner.scheduler.tick();

        // TODO: Send time to userspace?
    }
}

pub struct KimGuard {
    kim: *mut KernelInnerMut,
}

impl Deref for KimGuard {
    type Target = KernelInnerMut;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.kim }
    }
}

impl DerefMut for KimGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.kim }
    }
}

impl Drop for KimGuard {
    fn drop(&mut self) {
        KERNEL.status.store(Kernel::INIT_IDLE, Ordering::SeqCst);
    }
}

// TODO: De-dupe with userspace?
use core::{future::Future, ptr::NonNull};

#[repr(transparent)]
pub struct Task<F: Future + 'static>(MaitakeTask<&'static StaticScheduler, F, HBStorage>);

impl<F: Future + 'static> Task<F> {
    pub fn new(fut: F) -> Self {
        Self(MaitakeTask::new(&KERNEL.inner().scheduler, fut))
    }
}

struct HBStorage;

impl<F: Future + 'static> Storage<&'static StaticScheduler, F> for HBStorage {
    type StoredTask = HeapBox<Task<F>>;

    fn into_raw(task: Self::StoredTask) -> NonNull<MaitakeTask<&'static StaticScheduler, F, Self>> {
        unsafe {
            let ptr = &mut task.leak().0 as *mut MaitakeTask<&'static StaticScheduler, F, HBStorage>;
            NonNull::new_unchecked(ptr)
        }
    }

    fn from_raw(ptr: NonNull<MaitakeTask<&'static StaticScheduler, F, Self>>) -> Self::StoredTask {
        unsafe {
            HeapBox::from_leaked(ptr.as_ptr().cast())
        }
    }
}

pub async fn spawn<F: Future + 'static>(fut: F) {
    let task = Task(MaitakeTask::new(&KERNEL.inner().scheduler, fut));
    let atask = mnemos_alloc::allocate(task).await;
    spawn_allocated(atask);
}

pub fn spawn_allocated<F: Future + 'static>(task: HeapBox<Task<F>>) -> () {
    KERNEL.inner().scheduler.spawn_allocated::<F, HBStorage>(task)
}
