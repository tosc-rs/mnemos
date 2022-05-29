//! NOTE: this borrows heavily from [mycelium]'s executor.
//!
//! [mycelium]: https://github.com/hawkw/mycelium

pub mod task;

use core::{marker::PhantomData, future::Future, sync::atomic::AtomicUsize, ptr::NonNull, task::Poll};
use crate::alloc::{HeapBox, HeapGuard};

use abi::bbqueue_ipc::framed::{FrameProducer, FrameConsumer};
use task::Header;
use cordyceps::mpsc_queue::{Links, MpscQueue};

use self::task::{Vtable, TaskRef, Task};

// https://main--magnificent-halva-1c2bb0.netlify.app/cordyceps/mpsc_queue/struct.mpscqueue

pub struct Terpsichore {
    pub(crate) run_queue: MpscQueue<Header>,
}

static RUN_QUEUE_STUB: Header = task::Header {
    links: Links::new_stub(),
    vtable: &Vtable { poll: nop },
    refcnt: AtomicUsize::new(0),
    status: AtomicUsize::new(task::Header::ERROR),
};
unsafe fn nop(_: NonNull<Header>) -> Poll<()> {
    Poll::Pending
}

pub static EXECUTOR: Terpsichore = Terpsichore {
    run_queue: unsafe { MpscQueue::new_with_static_stub(&RUN_QUEUE_STUB) },
};

pub fn spawn<F: Future + 'static>(task: HeapBox<Task<F>>) -> JoinHandle<F::Output> {
    let tr = TaskRef::new(task);
    let nntr = unsafe {
        (*tr.0.as_ref()).incr_refcnt();
        tr.0
    };
    EXECUTOR.run_queue.enqueue(tr);
    JoinHandle {
        marker: PhantomData,
        ptr: nntr,
    }
}

impl Terpsichore {
    pub fn run(
        &self,
        u2k: &mut FrameProducer,
        k2u: &mut FrameConsumer,
        hg: &mut HeapGuard,
    ) {
        loop {
            // TODO: Process messages

            // TODO: Process heap allocations

            for task in self.run_queue.consume() {
                task.poll();
            }
        }
    }
}

#[derive(Debug)]
struct Stub {
    header: task::Header,
}

// impl Schedule for Stub {
//     fn schedule(&self, _: TaskRef) {
//         unimplemented!("stub task should never be woken!")
//     }
// }

// impl Future for Stub {
//     type Output = ();
//     fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> task::Poll<Self::Output> {
//         unreachable!("the stub task should never be polled!")
//     }
// }

pub struct JoinHandle<T> {
    marker: PhantomData<T>,
    ptr: NonNull<Header>,
}

// pub fn spawn<F: Future>(fut: Pin<HeapBox<F>>) -> JoinHandle<F::Output> {
//     todo!()
// }

// pub(crate) fn schedule()
