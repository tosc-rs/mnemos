use std::ops::Deref;
use std::ptr::NonNull;

use mnemos_alloc::{
    containers::{HeapArray, HeapBox},
    heap::AHeap,
};

#[derive(Debug, Eq, PartialEq)]
struct Demo {
    one: u64,
    two: u8,
    three: [u16; 7],
}

#[test]
fn basic() {
    const SIZE: usize = 16 * 1024;

    let bufptr = Box::into_raw(Box::new([0u8; SIZE]));
    let (_heap, mut guard) = unsafe { AHeap::bootstrap(bufptr.cast::<u8>(), SIZE).unwrap() };

    let alloc_1 = guard
        .alloc_box(Demo {
            one: 123,
            two: 222,
            three: [0xABAB; 7],
        })
        .map_err(drop)
        .unwrap();
    let alloc_2 = guard
        .alloc_box(Demo {
            one: 111,
            two: 212,
            three: [0xCACA; 7],
        })
        .map_err(drop)
        .unwrap();

    drop(alloc_1);
    drop(guard);
    drop(alloc_2);
}

#[test]
fn basic_arr() {
    const SIZE: usize = 16 * 1024;

    let bufptr = Box::into_raw(Box::new([0u8; SIZE]));
    let (_heap, mut guard) = unsafe { AHeap::bootstrap(bufptr.cast::<u8>(), SIZE).unwrap() };

    let alloc_1: HeapArray<u16> = guard.alloc_box_array_with(|| 0xACAC, 42).unwrap();
    let alloc_2: HeapArray<u16> = guard.alloc_box_array_with(|| 0x4242, 27).unwrap();

    drop(alloc_1);
    drop(guard);
    drop(alloc_2);
}

#[test]
fn leak_unleak() {
    const SIZE: usize = 16 * 1024;

    let bufptr = Box::into_raw(Box::new([0u8; SIZE]));
    let (_heap, mut guard) = unsafe { AHeap::bootstrap(bufptr.cast::<u8>(), SIZE).unwrap() };

    let alloc_1 = guard
        .alloc_box(Demo {
            one: 123,
            two: 222,
            three: [0xABAB; 7],
        })
        .map_err(drop)
        .unwrap();

    let leaked_nn: NonNull<Demo> = alloc_1.leak();

    assert_eq!(
        unsafe { leaked_nn.as_ref() },
        &Demo {
            one: 123,
            two: 222,
            three: [0xABAB; 7],
        }
    );

    let unleaked: HeapBox<Demo> = unsafe { HeapBox::from_leaked(leaked_nn) };

    assert_eq!(
        unleaked.deref(),
        &Demo {
            one: 123,
            two: 222,
            three: [0xABAB; 7],
        }
    );
}
