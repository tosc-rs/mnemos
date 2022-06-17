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

use core::{cell::UnsafeCell, mem::MaybeUninit, sync::atomic::{AtomicUsize, Ordering}};
use std::marker::PhantomData;
use maitake::wait::{WaitCell, WaitQueue};

pub unsafe trait Storage<T> {
    fn buf(&self) -> (*const UnsafeCell<Cell<T>>, usize);
}

pub struct MpMcQueue<T, STO: Storage<T>> {
    storage: STO,
    dequeue_pos: AtomicUsize,
    enqueue_pos: AtomicUsize,
    cons_wait: WaitCell,
    prod_wait: WaitQueue,
    pd: PhantomData<T>,
}

impl<T, STO: Storage<T>> MpMcQueue<T, STO> {
    /// Creates an empty queue
    pub fn new(storage: STO) -> Self {
        let (ptr, len) = storage.buf();
        assert_eq!(len, len.next_power_of_two());
        let sli = unsafe { core::slice::from_raw_parts(ptr, len) };
        sli.iter().enumerate().for_each(|(i, slot)| unsafe {
            slot.get().write(Cell { data: MaybeUninit::uninit(), sequence: AtomicUsize::new(i) });
        });

        Self {
            storage,
            dequeue_pos: AtomicUsize::new(0),
            enqueue_pos: AtomicUsize::new(0),
            cons_wait: WaitCell::new(),
            prod_wait: WaitQueue::new(),
            pd: PhantomData,
        }
    }

    /// Returns the item in the front of the queue, or `None` if the queue is empty
    pub fn dequeue_sync(&self) -> Option<T> {
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
    pub fn enqueue_sync(&self, item: T) -> Result<(), T> {
        let (ptr, len) = self.storage.buf();
        let res = unsafe {
            enqueue(
                (*ptr).get(),
                &self.enqueue_pos,
                len - 1,
                item,
            )
        };
        if res.is_ok() {
            self.cons_wait.notify();
        }
        res
    }

    pub async fn enqueue_async(&self, mut item: T) -> Result<(), T> {
        while let Err(eitem) = self.enqueue_sync(item) {
            match self.prod_wait.wait().await {
                Ok(()) => {},
                Err(_) => return Err(eitem),
            }
            item = eitem;
        }
        Ok(())
    }

    pub async fn dequeue_async(&self) -> Result<T, ()> {
        loop {
            match self.dequeue_sync() {
                Some(t) => return Ok(t),
                None => {
                    match self.cons_wait.wait().await {
                        Ok(()) => {},
                        Err(_) => return Err(())
                    }
                },
            }
        }
    }
}

unsafe impl<T, STO: Storage<T>> Sync for MpMcQueue<T, STO> where T: Send {}

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

unsafe fn dequeue<T>(
    buffer: *mut Cell<T>,
    dequeue_pos: &AtomicUsize,
    mask: usize,
) -> Option<T> {
    let mut pos = dequeue_pos.load(Ordering::Relaxed);

    let mut cell;
    loop {
        cell = buffer.add(usize::from(pos & mask));
        let seq = (*cell).sequence.load(Ordering::Acquire);
        let dif = (seq as i8).wrapping_sub((pos.wrapping_add(1)) as i8);

        if dif == 0 {
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
        } else if dif < 0 {
            return None;
        } else {
            pos = dequeue_pos.load(Ordering::Relaxed);
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
        cell = buffer.add(usize::from(pos & mask));
        let seq = (*cell).sequence.load(Ordering::Acquire);
        let dif = (seq as i8).wrapping_sub(pos as i8);

        if dif == 0 {
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
        } else if dif < 0 {
            return Err(item);
        } else {
            pos = enqueue_pos.load(Ordering::Relaxed);
        }
    }

    (*cell).data.as_mut_ptr().write(item);
    (*cell)
        .sequence
        .store(pos.wrapping_add(1), Ordering::Release);
    Ok(())
}
