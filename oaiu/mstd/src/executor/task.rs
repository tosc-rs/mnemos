
use core::{pin::Pin, ptr::NonNull, future::Future, cell::UnsafeCell, task::{Poll, RawWakerVTable, RawWaker}, sync::atomic::{AtomicUsize, Ordering}};
use cordyceps::{Linked, mpsc_queue::{Links, MpscQueue}};
use crate::alloc::HeapBox;

use super::EXECUTOR;


#[repr(C)]
#[derive(Debug)]
pub(crate) struct Header {
    pub(crate) links: Links<Header>,
    pub(crate) vtable: &'static Vtable,
    pub(crate) refcnt: AtomicUsize,
    pub(crate) status: AtomicUsize,
}

impl Header {
    pub(crate) const PENDING: usize = 0;
    pub(crate) const COMPLETE: usize = 1;
    pub(crate) const ERROR: usize = 2;

    #[inline]
    pub(crate) fn incr_refcnt(&self) {
        // TODO: Is this different if we move to multi core?
        self.refcnt.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    #[must_use = "You should check this flag to see if the allocation should be dropped!"]
    pub(crate) fn decr_refcnt(&self) -> bool {
        self.refcnt.fetch_sub(1, Ordering::Relaxed) == 1
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Task<F: Future> {
    header: Header,
    inner: UnsafeCell<Cell<F>>,
}

impl<F: Future> Task<F> {
    pub async fn new(f: F) -> HeapBox<Self> {
        let header = Header {
            links: Links::new(),
            vtable: &Self::TASK_VTABLE,
            refcnt: AtomicUsize::new(1),
            status: AtomicUsize::new(Header::PENDING),
        };
        let inner = UnsafeCell::new(Cell { future: ManuallyDrop::new(f) });
        crate::alloc::allocate(Self {
            header,
            inner,
        }).await
    }
}

use core::mem::ManuallyDrop;

pub union Cell<F: Future> {
    future: ManuallyDrop<F>,
    output: ManuallyDrop<F::Output>,
}

#[derive(Debug)]
pub(crate) struct Vtable {
    /// Poll the future.
    pub(crate) poll: unsafe fn(NonNull<Header>) -> Poll<()>,
}

impl<F: Future> Task<F> {
    const TASK_VTABLE: Vtable = Vtable {
        poll: Self::poll,
        // deallocate: Self::deallocate,
    };

    const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
        Self::clone_waker,
        Self::wake_by_val,
        Self::wake_by_ref,
        Self::drop_waker,
    );

    unsafe fn clone_waker(ptr: *const ()) -> RawWaker {
        // trace_task!(ptr, F, "clone_waker");
        Self::raw_waker(ptr as *const Self)
    }

    fn raw_waker(this: *const Self) -> RawWaker {
        unsafe { (*this).header.incr_refcnt() };
        RawWaker::new(this as *const (), &Self::WAKER_VTABLE)
    }

    unsafe fn wake_by_val(ptr: *const ()) {
        EXECUTOR.run_queue.enqueue(TaskRef(NonNull::new_unchecked((ptr as *mut ()).cast())))
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        EXECUTOR.run_queue.enqueue(TaskRef(NonNull::new_unchecked((ptr as *mut ()).cast())))
    }

    unsafe fn drop_waker(ptr: *const ()) {
        let ptr = ptr as *mut ();
        let ptr = ptr.cast::<Self>();
        if (*ptr).header.decr_refcnt() {
            let hb = HeapBox::from_leaked(ptr);
            match hb.header.status.load(Ordering::Relaxed) {
                Header::COMPLETE => {
                    ManuallyDrop::drop(&mut (*hb.inner.get()).output);
                }
                _ => {
                    ManuallyDrop::drop(&mut (*hb.inner.get()).future);
                }
            }
        }
    }

    unsafe fn poll(ptr: NonNull<Header>) -> Poll<()> {
        // trace_task!(ptr, F, "poll");
        // let ptr = ptr.cast::<Self>();
        // let waker = Waker::from_raw(Self::raw_waker(ptr.as_ptr()));
        // let cx = Context::from_waker(&waker);
        // let pin = Pin::new_unchecked(ptr.cast::<Self>().as_mut());
        // let poll = pin.poll_inner(cx);
        // if poll.is_ready() {
        //     Self::drop_ref(ptr);
        // }

        // poll
        todo!()
    }

    unsafe fn schedule(this: NonNull<Self>) {
        EXECUTOR.run_queue.enqueue(TaskRef(this.cast()));
    }
}

#[derive(Debug)]
pub(crate) struct TaskRef(pub(crate) NonNull<Header>);

impl TaskRef {
    pub(crate) fn new<F: Future + 'static>(task: HeapBox<Task<F>>) -> Self {
        let ltr = task.leak() as *mut Task<F>;
        TaskRef(unsafe { NonNull::new_unchecked(ltr.cast()) })
    }
}

unsafe impl Linked<Links<Header>> for Header {
    type Handle = TaskRef;

    fn into_ptr(r: Self::Handle) -> core::ptr::NonNull<Self> {
        r.0
    }

    unsafe fn from_ptr(ptr: core::ptr::NonNull<Self>) -> Self::Handle {
        TaskRef(ptr)
    }

    unsafe fn links(ptr: core::ptr::NonNull<Self>) -> core::ptr::NonNull<Links<Header>> {
        ptr.cast()
    }
}
