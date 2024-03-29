//! NOTE: this borrows heavily from [mycelium]'s executor.
//!
//! [mycelium]: https://github.com/hawkw/mycelium

pub mod mailbox;
pub mod time;

pub use maitake::task::JoinHandle;
use maitake::{
    self,
    scheduler::{StaticScheduler, TaskStub},
    task::{Storage, Task as MaitakeTask},
};

use core::{
    future::Future,
    ptr::NonNull,
    sync::atomic::{AtomicPtr, Ordering},
};
// use mnemos_alloc::{containers::HeapBox, heap::AHeap};

#[repr(transparent)]
pub struct Task<F>(MaitakeTask<&'static StaticScheduler, F, HBStorage>)
where
    F: Future + Send + 'static,
    F::Output: Send;

impl<F> Task<F>
where
    F: Future + Send + 'static,
    F::Output: Send,
{
    pub fn new(fut: F) -> Self {
        Self(MaitakeTask::new(fut))
    }
}

pub struct Terpsichore {
    pub(crate) scheduler: StaticScheduler,
    heap_ptr: AtomicPtr<AHeap>,
}

static TASK_STUB: TaskStub = TaskStub::new();
pub static EXECUTOR: Terpsichore = Terpsichore {
    scheduler: unsafe { StaticScheduler::new_with_static_stub(&TASK_STUB) },
    heap_ptr: AtomicPtr::new(core::ptr::null_mut()),
};

struct HBStorage;

impl<F> Storage<&'static StaticScheduler, F> for HBStorage
where
    F: Future + Send + 'static,
    F::Output: Send,
{
    type StoredTask = HeapBox<Task<F>>;

    fn into_raw(task: HeapBox<Task<F>>) -> NonNull<MaitakeTask<&'static StaticScheduler, F, Self>> {
        task.leak()
            .cast::<MaitakeTask<&'static StaticScheduler, F, HBStorage>>()
    }

    fn from_raw(ptr: NonNull<MaitakeTask<&'static StaticScheduler, F, Self>>) -> HeapBox<Task<F>> {
        unsafe { HeapBox::from_leaked(ptr.cast::<Task<F>>()) }
    }
}

impl Terpsichore {
    /// # Safety
    ///
    /// - `heap_start` must be a valid pointer to a region of memory of size `heap_len`.
    /// - The region of memory pointed to by `heap_start` must not be mutably aliased.
    // TODO: This is *probably* something that needs to be called by the entrypoint, which
    // might be provided per-platform.
    //
    // You must ALSO initialize the mailbox.
    pub unsafe fn initialize(&'static self, heap_start: *mut u8, heap_len: usize) {
        let (hptr, _guard) = AHeap::bootstrap(heap_start, heap_len).unwrap();
        EXECUTOR.heap_ptr.store(hptr.as_ptr(), Ordering::Release);
    }

    pub fn run(&'static self) {
        // Process timers
        crate::executor::time::CHRONOS.poll();

        // Process messages
        crate::executor::mailbox::MAILBOX.poll();

        // Process heap allocations
        self.get_alloc().poll();

        self.scheduler.tick();
    }

    pub fn get_alloc(&'static self) -> &'static AHeap {
        let cptr = self.heap_ptr.load(Ordering::Relaxed) as *const AHeap;
        unsafe { cptr.as_ref() }.unwrap()
    }

    pub async fn spawn<F>(&'static self, fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send,
    {
        let task = Task(MaitakeTask::new(fut));
        let atask = self.get_alloc().allocate(task).await;
        self.spawn_allocated(atask)
    }

    pub fn spawn_allocated<F>(&'static self, task: HeapBox<Task<F>>) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send,
    {
        self.scheduler.spawn_allocated::<F, HBStorage>(task)
    }
}
