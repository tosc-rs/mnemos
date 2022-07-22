//! Reusable One-Shot Channel
//!
//! Often, clients of drivers only want to process one "in-flight" message at
//! a time. If request pipelining is not required, then a Reusable One-Shot Channel
//! is an easy way to perform an async/await request/response cycle.
//!
//! Essentially, a Rosc is a single producer, single consumer, channel, with a max
//! depth of one. Many producers can be created over the lifecycle of a single consumer,
//! however only zero or one producers can be live at any given time.

use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicU8, Ordering},
};

use maitake::wait::WaitCell;
use mnemos_alloc::containers::HeapArc;

use crate::Kernel;

/// A reusable One-Shot channel.
///
/// A `Rosc<T>` can be used to hand out single-use [Sender] items, which can
/// be used to make a single reply.
///
/// A given `Rosc<T>` can only ever have zero or one `Sender<T>`s live at any
/// given time, and a response can be received through a call to [Rosc::receive].
pub struct Rosc<T> {
    inner: HeapArc<Inner<T>>,
}

/// A single-use One-Shot channel sender
///
/// It can be consumed to send a response back to the [Rosc] instance that created
/// the [Sender].
pub struct Sender<T> {
    inner: HeapArc<Inner<T>>,
}

/// An inner type shared between the Rosc and Sender.
struct Inner<T> {
    state: AtomicU8,
    cell: UnsafeCell<MaybeUninit<T>>,
    wait: WaitCell,
}

// impl Rosc

impl<T> Rosc<T> {
    /// Create a new `Rosc<T>` using the heap from the given kernel
    pub async fn new_async(kernel: &'static Kernel) -> Self {
        Self {
            inner: kernel.heap().allocate_arc(Inner::new()).await,
        }
    }

    /// Create a sender for the given `Rosc<T>`. If a sender is already
    /// active, or the previous response has not yet been retrieved, an
    /// error will be immediately returned.
    ///
    /// This error can be cleared by awaiting [Rosc::receive].
    pub fn sender(&self) -> Result<Sender<T>, ()> {
        self.inner
            .state
            .compare_exchange(
                Inner::<T>::IDLE,
                Inner::<T>::WAITING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .map_err(drop)?;

        Ok(Sender {
            inner: self.inner.clone(),
        })
    }

    /// Await the response from a created sender.
    ///
    /// If a sender has not been created, this function will immediately return
    /// an error.
    ///
    /// If the sender is dropped without sending a response, this function will
    /// return an error after the sender has been dropped.
    pub async fn receive(&self) -> Result<T, ()> {
        loop {
            let swap = self.inner.state.compare_exchange(
                Inner::<T>::READY,
                Inner::<T>::READING,
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
                        self.inner.state.store(Inner::<T>::IDLE, Ordering::Release);
                        return Ok(ret.assume_init());
                    }
                }
                Err(Inner::<T>::WAITING | Inner::<T>::WRITING) => {
                    // We are still waiting for the Sender to start or complete.
                    self.inner.wait.wait().await.map_err(drop)?;
                }
                Err(_) => {
                    // We are either currently idle, i.e. no sender has been created,
                    // or the existing one was dropped unused, or something has gone terribly
                    // wrong. Return an error.
                    return Err(());
                }
            }
        }
    }
}

// Impl Sender

impl<T> Sender<T> {
    /// Consume the sender, providing it with a reply.
    pub fn send(self, item: T) -> Result<(), ()> {
        self.inner
            .state
            .compare_exchange(
                Inner::<T>::WAITING,
                Inner::<T>::WRITING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .map_err(drop)?;

        unsafe { self.inner.cell.get().write(MaybeUninit::new(item)) };
        self.inner.state.store(Inner::<T>::READY, Ordering::Release);
        self.inner.wait.wake();
        Ok(())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // Attempt to move the state from WAITING to IDLE, and wake any
        // pending waiters. This will cause an Err(()) on the receive side.
        let _ = self.inner.state.compare_exchange(
            Inner::<T>::WAITING,
            Inner::<T>::IDLE,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        self.inner.wait.wake();
    }
}

// impl Inner

unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

// TODO: Should probably try to impl drop, at least if state == READY
impl<T> Inner<T> {
    /// Not waiting for anything.
    const IDLE: u8 = 0;
    /// A Sender has been created, but no writes have begun yet
    const WAITING: u8 = 1;
    /// A Sender has begun writing, and will be dropped shortly.
    const WRITING: u8 = 2;
    /// The Sender has been dropped and the message has been send
    const READY: u8 = 3;
    /// Reading has already started
    const READING: u8 = 4;

    fn new() -> Self {
        Self {
            state: AtomicU8::new(Self::IDLE),
            cell: UnsafeCell::new(MaybeUninit::uninit()),
            wait: WaitCell::new(),
        }
    }
}
