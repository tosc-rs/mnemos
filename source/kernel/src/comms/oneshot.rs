//! One-Shot Channels
//!
//! Often, clients of drivers only want to process one "in-flight" message at
//! a time. If request pipelining is not required, then a One-Shot Channel
//! is an easy way to perform an async/await request/response cycle.

use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicU8, Ordering},
};

use maitake::sync::{Closed, WaitCell};
use mnemos_alloc::containers::HeapArc;

use crate::Kernel;

/// Not waiting for anything.
const ROSC_IDLE: u8 = 0;
/// A Sender has been created, but no writes have begun yet
const ROSC_WAITING: u8 = 1;
/// A Sender has begun writing, and will be dropped shortly.
const ROSC_WRITING: u8 = 2;
/// The Sender has been dropped and the message has been send
const ROSC_READY: u8 = 3;
/// Reading has already started
const ROSC_READING: u8 = 4;
/// The receiver has been manually closed or dropped.
const ROSC_CLOSED: u8 = 5;

/// A reusable One-Shot channel.
///
/// Essentially, a Reusable is a single producer, single consumer, channel, with a max
/// depth of one. Many producers can be created over the lifecycle of a single consumer,
/// however only zero or one producers can be live at any given time.
///
/// A `Reusable<T>` can be used to hand out single-use [Sender] items, which can
/// be used to make a single reply.
///
/// A given `Reusable<T>` can only ever have zero or one `Sender<T>`s live at any
/// given time, and a response can be received through a call to [Reusable::receive].
pub struct Reusable<T> {
    inner: HeapArc<Inner<T>>,
}

/// A single-use One-Shot channel sender
///
/// It can be consumed to send a response back to the [Reusable] instance that created
/// the [Sender].
pub struct Sender<T> {
    inner: HeapArc<Inner<T>>,
}

// An error type for the Reusable channel and Sender
#[derive(Debug, Eq, PartialEq)]
pub enum ReusableError {
    SenderAlreadyActive,
    NoSenderActive,
    ChannelClosed,
    InternalError,
}

impl From<Closed> for ReusableError {
    fn from(_: Closed) -> Self {
        ReusableError::ChannelClosed
    }
}

/// An inner type shared between the Rosc and Sender.
struct Inner<T> {
    state: AtomicU8,
    cell: UnsafeCell<MaybeUninit<T>>,
    wait: WaitCell,
}

// impl Reusable

impl<T> Reusable<T> {
    /// Create a new `Reusable<T>` using the heap from the given kernel
    pub async fn new_async(kernel: &'static Kernel) -> Self {
        Self {
            inner: kernel.heap().allocate_arc(Inner::new()).await,
        }
    }

    /// Create a sender for the given `Reusable<T>`. If a sender is already
    /// active, or the previous response has not yet been retrieved, an
    /// error will be immediately returned.
    ///
    /// This error can be cleared by awaiting [Reusable::receive].
    pub async fn sender(&self) -> Result<Sender<T>, ReusableError> {
        loop {
            let swap = self.inner.state.compare_exchange(
                ROSC_IDLE,
                ROSC_WAITING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );

            match swap {
                Ok(_) => {
                    return Ok(Sender {
                        inner: self.inner.clone(),
                    })
                }
                Err(val) => {
                    if val == ROSC_READY {
                        let _ = self.receive().await;
                    } else if (val == ROSC_WAITING) | (val == ROSC_WRITING) {
                        return Err(ReusableError::SenderAlreadyActive);
                    } else {
                        return Err(ReusableError::InternalError);
                    }
                }
            }
        }
    }

    /// Await the response from a created sender.
    ///
    /// If a sender has not been created, this function will immediately return
    /// an error.
    ///
    /// If the sender is dropped without sending a response, this function will
    /// return an error after the sender has been dropped.
    pub async fn receive(&self) -> Result<T, ReusableError> {
        loop {
            let swap = self.inner.state.compare_exchange(
                ROSC_READY,
                ROSC_READING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );

            match swap {
                Ok(_) => {
                    // We just swapped from READY to READING, that's a success!
                    unsafe {
                        let mut ret = MaybeUninit::<T>::uninit();
                        core::ptr::copy_nonoverlapping(
                            self.inner.cell.get().cast(),
                            ret.as_mut_ptr(),
                            1,
                        );
                        self.inner.state.store(ROSC_IDLE, Ordering::Release);
                        return Ok(ret.assume_init());
                    }
                }
                Err(ROSC_WAITING | ROSC_WRITING) => {
                    // We are still waiting for the Sender to start or complete.
                    // Trigger another wait cycle.
                    //
                    // NOTE: it's impossible for the wait to fail here, as we only
                    // close the channel when dropping the Reusable, which can't be
                    // done while the borrow of self is active in this function.
                    self.inner.wait.wait().await?;
                }
                Err(ROSC_IDLE) => {
                    // We are currently idle, i.e. no sender has been created,
                    // or the existing one was dropped unused.
                    break Err(ReusableError::NoSenderActive);
                }
                Err(_) => {
                    // Something has gone terribly wrong. Return an error.
                    break Err(ReusableError::InternalError);
                }
            }
        }
    }

    /// Close the receiver. This will cause any pending senders to fail.
    pub fn close(self) {
        drop(self);
    }
}

impl<T> Drop for Reusable<T> {
    fn drop(&mut self) {
        // Immediately mark the state as closed
        let old = self.inner.state.swap(ROSC_CLOSED, Ordering::AcqRel);
        // Mark the waiter as closed (shouldn't be necessary - you can only create
        // a waiter from the Reusable type, which we are now dropping).
        self.inner.wait.close();

        // Determine if we need to drop the payload, if there is one.
        match old {
            ROSC_IDLE => {
                // Nothing to do, already idle, no contents
            }
            ROSC_WAITING => {
                // We are waiting for the sender, but it will fail to send.
                // Nothing to do.
            }
            ROSC_WRITING => {
                // We are cancelling mid-send. This will cause the sender
                // to fail, and IT is responsible for dropping the almost-
                // sent message.
            }
            ROSC_READY => {
                // We have received a message, but are dropping before reception.
                // We are responsible to drop the contents.
                unsafe {
                    let ptr: *mut MaybeUninit<T> = self.inner.cell.get();
                    let ptr: *mut T = ptr.cast();
                    core::ptr::drop_in_place(ptr);
                }
            }
            ROSC_READING => {
                // This SHOULD be impossible, as this is a transient state while
                // receiving, which shouldn't be possible if we are dropping the
                // receiver. Make this a debug assert to catch if this ever happens
                // during development or testing, otherwise do nothing.
                debug_assert!(false, "Dropped receiver while reading?");
            }
            ROSC_CLOSED => {
                // This SHOULD be impossible, as closing requires dropping the
                // receiver. Make this a debug assert to catch if this ever happens
                // during development or testing, otherwise do nothing.
                debug_assert!(false, "Receiver already closed while closing?");
            }
            _ => {}
        }
    }
}

// Impl Sender

impl<T> Sender<T> {
    /// Consume the sender, providing it with a reply.
    pub fn send(self, item: T) -> Result<(), ReusableError> {
        let swap = self.inner.state.compare_exchange(
            ROSC_WAITING,
            ROSC_WRITING,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );

        match swap {
            Ok(_) => {}
            Err(ROSC_CLOSED) => return Err(ReusableError::ChannelClosed),
            Err(_) => return Err(ReusableError::InternalError),
        };

        unsafe { self.inner.cell.get().write(MaybeUninit::new(item)) };

        // Attempt to swap back to READY. This COULD fail if we just swapped to closed,
        // but in that case we won't override the CLOSED state, and it becomes OUR
        // responsibility to drop the contents.
        let swap = self.inner.state.compare_exchange(
            ROSC_WRITING,
            ROSC_READY,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );

        match swap {
            Ok(_) => {}
            Err(ROSC_CLOSED) => {
                // Yup, a close happened WHILE we were writing. Go ahead and drop the contents
                unsafe {
                    let ptr: *mut MaybeUninit<T> = self.inner.cell.get();
                    let ptr: *mut T = ptr.cast();
                    core::ptr::drop_in_place(ptr);
                }
                return Err(ReusableError::ChannelClosed);
            }
            Err(_) => return Err(ReusableError::InternalError),
        }

        self.inner.wait.wake();
        Ok(())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // Attempt to move the state from WAITING to IDLE, and wake any
        // pending waiters. This will cause an Err(()) on the receive side.
        let _ = self.inner.state.compare_exchange(
            ROSC_WAITING,
            ROSC_IDLE,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        self.inner.wait.wake();
    }
}

// impl Inner

unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

// NOTE: A drop impl is not necessary, as the drop of the contents is handled
// by the Sender or Reusable.
impl<T> Inner<T> {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(ROSC_IDLE),
            cell: UnsafeCell::new(MaybeUninit::uninit()),
            wait: WaitCell::new(),
        }
    }
}
