//! NOTE: this borrows heavily from [mycelium]'s executor.
//!
//! [mycelium]: https://github.com/hawkw/mycelium

pub mod time;
pub mod mailbox;

use maitake::{self, scheduler::{StaticScheduler, TaskStub}, task::Storage};
use maitake::task::Task as MaitakeTask;

use core::{future::Future, ptr::NonNull};
use mnemos_alloc::HeapBox;

#[repr(transparent)]
pub struct Task<F: Future + 'static>(MaitakeTask<&'static StaticScheduler, F, HBStorage>);

impl<F: Future + 'static> Task<F> {
    pub fn new(fut: F) -> Self {
        Self(MaitakeTask::new(&EXECUTOR.scheduler, fut))
    }
}

pub struct Terpsichore {
    pub(crate) scheduler: StaticScheduler,
}

static TASK_STUB: TaskStub = TaskStub::new();
pub static EXECUTOR: Terpsichore = Terpsichore {
    scheduler: unsafe { StaticScheduler::new_with_static_stub(&TASK_STUB) },
};

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
    let task = Task(MaitakeTask::new(&EXECUTOR.scheduler, fut));
    let atask = mnemos_alloc::allocate(task).await;
    spawn_allocated(atask);
}

pub fn spawn_allocated<F: Future + 'static>(task: HeapBox<Task<F>>) -> () {
    EXECUTOR.scheduler.spawn_allocated::<F, HBStorage>(task)
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
        mnemos_alloc::HEAP.poll();

        self.scheduler.tick();
    }
}
