use std::{ops::Deref, alloc::Layout};
use std::ptr::{NonNull, addr_of_mut};

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
fn basic_raw() {
    const SIZE: usize = 16 * 1024;

    const ALLOC_1_F64S: usize = 12;
    const ALLOC_2_U64S: usize = 17;

    let bufptr = Box::into_raw(Box::new([0u8; SIZE]));
    let (_heap, mut guard) = unsafe { AHeap::bootstrap(bufptr.cast::<u8>(), SIZE).unwrap() };

    #[repr(C)]
    struct Tail {
        a: u8,
        b: u128,
        c: [u64; 0]
    }

    let layout_1 = Layout::array::<f64>(ALLOC_1_F64S).unwrap();
    let (layout_2, _) = Layout::new::<Tail>().extend(Layout::array::<u64>(ALLOC_2_U64S).unwrap()).unwrap();

    let alloc_1: NonNull<()> = guard.alloc_raw(layout_1).unwrap();
    let alloc_2: NonNull<()> = guard.alloc_raw(layout_2).unwrap();

    // Write the full contents of alloc 1
    for i in 0..ALLOC_1_F64S {
        unsafe {
            alloc_1.cast::<f64>().as_ptr().add(i).write(1.2345f64);
        }
    }
    // read it back
    let sli = unsafe { core::slice::from_raw_parts(alloc_1.cast::<f64>().as_ptr(), ALLOC_1_F64S) };
    assert!(sli.iter().all(|f| *f == 1.2345f64));

    // Write the full contents of alloc 2
    unsafe {
        let ptr2 = alloc_2.cast::<Tail>().as_ptr();
        addr_of_mut!((*ptr2).a).write(10);
        addr_of_mut!((*ptr2).b).write(u128::MAX - 3);
        let base = addr_of_mut!((*ptr2).c).cast::<u64>();
        for i in 0..ALLOC_2_U64S {
            base.add(i).write(u64::MAX - (i as u64));
        }
        let sli = core::slice::from_raw_parts(base, ALLOC_2_U64S);
        assert!(sli.iter().enumerate().all(|(i, u)| *u == (u64::MAX - i as u64)));
    }

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
