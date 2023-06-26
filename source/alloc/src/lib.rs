//! # Mnemos Alloc (2023 edition)
//!
//! An async-aware wrapper for Global Allocators. See [heap] for details about
//! how the allocator wrappers work, and [containers] for async-aware collection
//! types that are intended for use in mnemos' kernel and services.

#![cfg_attr(not(feature = "use-std"), no_std)]

pub mod containers;
pub mod heap;

extern crate alloc;
