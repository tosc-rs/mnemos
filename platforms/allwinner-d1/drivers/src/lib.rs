#![no_std]

pub mod uart;

use core::{mem::MaybeUninit, cell::UnsafeCell};

pub struct Ram<const N: usize> {
    inner: MaybeUninit<UnsafeCell<[u8; N]>>,
}

unsafe impl<const N: usize> Sync for Ram<N> {}
impl<const N: usize> Ram<N> {
    pub const fn new() -> Self {
        Self {
            inner: MaybeUninit::uninit(),
        }
    }

    pub fn as_ptr(&'static self) -> *mut u8 {
        let p: *mut UnsafeCell<[u8; N]> = self.inner.as_ptr().cast_mut();
        let p: *mut u8 = p.cast();
        p
    }
}
