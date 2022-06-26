//! NOTE: this borrows heavily from [mycelium]'s executor.
//!
//! [mycelium]: https://github.com/hawkw/mycelium

pub mod time;
pub mod mailbox;

use maitake::{self, scheduler::{StaticScheduler, TaskStub}, task::Storage};
use maitake::task::Task as MaitakeTask;

use core::{future::Future, ptr::NonNull, sync::atomic::{AtomicPtr, Ordering}};
use mnemos_alloc::{
    containers::HeapBox,
    heap::AHeap,
};

#[repr(transparent)]
pub struct Task<F: Future + 'static>(MaitakeTask<&'static StaticScheduler, F, HBStorage>);

impl<F: Future + 'static> Task<F> {
    pub fn new(fut: F) -> Self {
        Self(MaitakeTask::new(&EXECUTOR.scheduler, fut))
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

impl<F: Future + 'static> Storage<&'static StaticScheduler, F> for HBStorage {
    type StoredTask = HeapBox<Task<F>>;

    fn into_raw(task: HeapBox<Task<F>>) -> NonNull<MaitakeTask<&'static StaticScheduler, F, Self>> {
        task.leak()
            .cast::<MaitakeTask<&'static StaticScheduler, F, HBStorage>>()
    }

    fn from_raw(ptr: NonNull<MaitakeTask<&'static StaticScheduler, F, Self>>) -> HeapBox<Task<F>> {
        unsafe {
            HeapBox::from_leaked(ptr.cast::<Task<F>>())
        }
    }
}

impl Terpsichore {
    pub fn run(
        &'static self,
    ) {
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

    pub async fn spawn<F: Future + 'static>(&'static self, fut: F) {
        let task = Task(MaitakeTask::new(&EXECUTOR.scheduler, fut));
        let atask = self.get_alloc().allocate(task).await;
        self.spawn_allocated(atask);
    }

    pub fn spawn_allocated<F: Future + 'static>(&'static self, task: HeapBox<Task<F>>) -> () {
        self.scheduler.spawn_allocated::<F, HBStorage>(task)
    }
}
