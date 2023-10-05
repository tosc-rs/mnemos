#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod ccu;
pub mod clint;
pub mod dmac;
pub mod drivers;
pub mod plic;
mod ram;
pub mod timer;
pub mod trap;
pub use self::ram::Ram;
