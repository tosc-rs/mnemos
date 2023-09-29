#![cfg_attr(not(test), no_std)]
pub mod index;
pub(crate) mod loom;
pub mod slab;

mod sealed {
    pub trait Sealed {}
}
