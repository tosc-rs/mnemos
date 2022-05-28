use std::{mem::MaybeUninit, sync::atomic::Ordering};

use abi::bbqueue_ipc::BBBuffer;

const RING_SIZE: usize = 4096;
const HEAP_SIZE: usize = 192 * 1024;

fn main() {
    let u2k = Box::into_raw(Box::new(BBBuffer::new()));
    let u2k_buf = Box::into_raw(Box::new([0u8; RING_SIZE]));
    let k2u = Box::into_raw(Box::new(BBBuffer::new()));
    let k2u_buf = Box::into_raw(Box::new([0u8; RING_SIZE]));

    let user_heap = Box::into_raw(Box::new([0u8; HEAP_SIZE]));

    // TODO: The kernel is supposed to do this...
    unsafe {
        (*u2k).initialize(u2k_buf.cast(), RING_SIZE);
        (*k2u).initialize(k2u_buf.cast(), RING_SIZE);
    }

    abi::U2K_RING.store(u2k, Ordering::Relaxed);
    abi::K2U_RING.store(k2u, Ordering::Relaxed);
    abi::HEAP_PTR.store(user_heap.cast(), Ordering::Relaxed);
    abi::HEAP_LEN.store(HEAP_SIZE, Ordering::Relaxed);

    println!("[Melpo]: You've met with a terrible fate, haven't you?");
}
