//! Reusable One-Shot Channel

use core::{cell::UnsafeCell, mem::MaybeUninit, sync::atomic::{AtomicU8, Ordering}};

use maitake::wait::WaitCell;
use mnemos_alloc::containers::HeapArc;

use crate::Kernel;

unsafe impl<T: Send> Send for Inner<T> { }
unsafe impl<T: Send> Sync for Inner<T> { }

struct Inner<T> {
    state: AtomicU8,
    cell: UnsafeCell<MaybeUninit<T>>,
    wait: WaitCell,
}

// TODO: Should probably try to impl drop, at least if state == READY
impl<T> Inner<T> {
    /// Not waiting for anything
    const IDLE: u8 = 0;
    /// Waiting, but no write has started
    const WAITING: u8 = 1;
    /// Writing has already started
    const WRITING: u8 = 2;
    /// Ready to start reading, valid data in cell
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

pub struct Rosc<T> {
    inner: HeapArc<Inner<T>>,
}

pub struct Sender<T> {
    inner: HeapArc<Inner<T>>,
}

impl<T> Sender<T> {
    pub fn send(self, item: T) -> Result<(), ()> {
        self.inner.state.compare_exchange(
            Inner::<T>::WAITING,
            Inner::<T>::WRITING,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ).map_err(drop)?;

        unsafe { self.inner.cell.get().write(MaybeUninit::new(item)) };
        self.inner.state.store(Inner::<T>::READY, Ordering::Release);
        self.inner.wait.wake();
        Ok(())
    }
}

impl<T> Rosc<T> {
    pub async fn new_async(kernel: &'static Kernel) -> Self {
        Self {
            inner: kernel.heap().allocate_arc(Inner::new()).await,
        }
    }

    pub fn sender(&self) -> Result<Sender<T>, ()> {
        self.inner.state.compare_exchange(
            Inner::<T>::IDLE,
            Inner::<T>::WAITING,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ).map_err(drop)?;

        Ok(Sender {
            inner: self.inner.clone(),
        })
    }

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
                    // We just swapped from READY to IDLE, that's a success!
                    unsafe {
                        let mut ret = MaybeUninit::<T>::uninit();
                        core::ptr::copy_nonoverlapping(self.inner.cell.get().cast(), ret.as_mut_ptr(), 1);
                        self.inner.state.store(Inner::<T>::IDLE, Ordering::Release);
                        return Ok(ret.assume_init());
                    }
                }
                Err(Inner::<T>::WAITING | Inner::<T>::WRITING) => {
                    self.inner.wait.wait().await.map_err(drop)?;
                }
                Err(_) => {
                    return Err(());
                }
            }
        }
    }
}
