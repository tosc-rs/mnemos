//! SpiteBuf - an Async (or not, I'm not a cop) MpscQueue
//!
//! Based on some stuff
//!
//! # References
//!
//! This is an implementation of Dmitry Vyukov's ["Bounded MPMC queue"][0] minus the cache padding.
//!
//! Queue implementation from heapless::mpmc::MpMcQueue
//!
//! [0]: http://www.1024cores.net/home/lock-free-algorithms/queues/bounded-mpmc-queue

#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::marker::PhantomData;
use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use maitake::sync::{WaitCell, WaitQueue};

pub unsafe trait Storage<T> {
    fn buf(&self) -> (*const UnsafeCell<Cell<T>>, usize);
}

pub struct MpScQueue<T, STO: Storage<T>> {
    storage: STO,
    dequeue_pos: AtomicUsize,
    enqueue_pos: AtomicUsize,
    cons_wait: WaitCell,
    prod_wait: WaitQueue,
    closed: AtomicBool,
    pd: PhantomData<T>,
}

/// Represents a closed error
#[derive(Debug, Eq, PartialEq)]
pub enum EnqueueError<T> {
    Full(T),
    Closed(T),
}

#[derive(Debug, Eq, PartialEq)]
pub enum DequeueError {
    Closed,
}

impl<T, STO: Storage<T>> MpScQueue<T, STO> {
    /// Creates an empty queue
    ///
    /// The capacity of `storage` must be >= 2 and a power of two, or this code will panic.
    #[track_caller]
    pub fn new(storage: STO) -> Self {
        let (ptr, len) = storage.buf();
        assert_eq!(
            len,
            len.next_power_of_two(),
            "Capacity must be a power of two!"
        );
        assert!(len > 1, "Capacity must be larger than 1!");
        let sli = unsafe { core::slice::from_raw_parts(ptr, len) };
        sli.iter().enumerate().for_each(|(i, slot)| unsafe {
            slot.get().write(Cell {
                data: MaybeUninit::uninit(),
                sequence: AtomicUsize::new(i),
            });
        });

        Self {
            storage,
            dequeue_pos: AtomicUsize::new(0),
            enqueue_pos: AtomicUsize::new(0),
            cons_wait: WaitCell::new(),
            prod_wait: WaitQueue::new(),
            closed: AtomicBool::new(false),
            pd: PhantomData,
        }
    }

    // Mark the channel as permanently closed. Any already sent data
    // can be retrieved, but no further data will be allowed to be pushed.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.cons_wait.close();
        self.prod_wait.close();
    }

    /// Returns the item in the front of the queue, or `None` if the queue is empty
    pub fn dequeue_sync(&self) -> Option<T> {
        // Note: DON'T check the closed flag on dequeue. We want to be able
        // to drain any potential messages after closing.
        let (ptr, len) = self.storage.buf();
        let res = unsafe { dequeue((*ptr).get(), &self.dequeue_pos, len - 1) };
        if res.is_some() {
            self.prod_wait.wake_all();
        }
        res
    }

    /// Adds an `item` to the end of the queue
    ///
    /// Returns back the `item` if the queue is full
    pub fn enqueue_sync(&self, item: T) -> Result<(), EnqueueError<T>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(EnqueueError::Closed(item));
        }
        let (ptr, len) = self.storage.buf();
        let res = unsafe { enqueue((*ptr).get(), &self.enqueue_pos, len - 1, item) };
        if res.is_ok() {
            self.cons_wait.wake();
        }
        res.map_err(EnqueueError::Full)
    }

    pub async fn enqueue_async(&self, mut item: T) -> Result<(), EnqueueError<T>> {
        loop {
            match self.enqueue_sync(item) {
                // We succeeded or the queue is closed, propagate those errors
                ok @ Ok(_) => return ok,
                err @ Err(EnqueueError::Closed(_)) => return err,

                // It's full, let's wait until it isn't or the channel has closed
                Err(EnqueueError::Full(eitem)) => {
                    match self.prod_wait.wait().await {
                        Ok(()) => {}
                        Err(_) => return Err(EnqueueError::Closed(eitem)),
                    }
                    item = eitem;
                }
            }
        }
    }

    pub async fn dequeue_async(&self) -> Result<T, DequeueError> {
        loop {
            match self.dequeue_sync() {
                Some(t) => return Ok(t),

                // Note: if we have been closed, this wait will fail.
                None => match self.cons_wait.wait().await {
                    Ok(()) => {}
                    Err(_) => return Err(DequeueError::Closed),
                },
            }
        }
    }
}

unsafe impl<T, STO: Storage<T>> Sync for MpScQueue<T, STO> where T: Send {}

impl<T, STO: Storage<T>> Drop for MpScQueue<T, STO> {
    fn drop(&mut self) {
        while self.dequeue_sync().is_some() {}
        self.cons_wait.close();
        self.prod_wait.close();
    }
}

pub struct Cell<T> {
    data: MaybeUninit<T>,
    sequence: AtomicUsize,
}

pub const fn single_cell<T>() -> Cell<T> {
    Cell {
        data: MaybeUninit::uninit(),
        sequence: AtomicUsize::new(0),
    }
}

pub fn cell_array<const N: usize, T: Sized>() -> [Cell<T>; N] {
    [Cell::<T>::SINGLE_CELL; N]
}

impl<T> Cell<T> {
    const SINGLE_CELL: Self = Self::new(0);

    const fn new(seq: usize) -> Self {
        Self {
            data: MaybeUninit::uninit(),
            sequence: AtomicUsize::new(seq),
        }
    }
}

unsafe fn dequeue<T>(buffer: *mut Cell<T>, dequeue_pos: &AtomicUsize, mask: usize) -> Option<T> {
    let mut pos = dequeue_pos.load(Ordering::Relaxed);

    let mut cell;
    loop {
        cell = buffer.add(pos & mask);
        let seq = (*cell).sequence.load(Ordering::Acquire);
        let dif = (seq as i8).wrapping_sub((pos.wrapping_add(1)) as i8);

        match dif {
            0 => {
                if dequeue_pos
                    .compare_exchange_weak(
                        pos,
                        pos.wrapping_add(1),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    break;
                }
            }
            dif if dif < 0 => return None,
            _ => pos = dequeue_pos.load(Ordering::Relaxed),
        }
    }

    let data = (*cell).data.as_ptr().read();
    (*cell)
        .sequence
        .store(pos.wrapping_add(mask).wrapping_add(1), Ordering::Release);
    Some(data)
}

unsafe fn enqueue<T>(
    buffer: *mut Cell<T>,
    enqueue_pos: &AtomicUsize,
    mask: usize,
    item: T,
) -> Result<(), T> {
    let mut pos = enqueue_pos.load(Ordering::Relaxed);

    let mut cell;
    loop {
        cell = buffer.add(pos & mask);
        let seq = (*cell).sequence.load(Ordering::Acquire);
        let dif = (seq as i8).wrapping_sub(pos as i8);

        match dif {
            0 => {
                if enqueue_pos
                    .compare_exchange_weak(
                        pos,
                        pos.wrapping_add(1),
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    break;
                }
            }
            dif if dif < 0 => return Err(item),
            _ => pos = enqueue_pos.load(Ordering::Relaxed),
        }
    }

    (*cell).data.as_mut_ptr().write(item);
    (*cell)
        .sequence
        .store(pos.wrapping_add(1), Ordering::Release);
    Ok(())
}
