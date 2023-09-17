#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod ccu;
pub mod clint;
pub mod dmac;
pub mod drivers;
mod ram;
pub mod timer;
pub use self::ram::Ram;
