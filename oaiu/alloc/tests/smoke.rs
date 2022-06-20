use mnemos_alloc::heap::AHeap;

#[test]
fn basic() {
    const SIZE: usize = 16 * 1024;

    let bufptr = Box::into_raw(Box::new([0u8; SIZE]));
    let (_heap, _guard) = unsafe {
        AHeap::bootstrap(bufptr.cast::<u8>(), SIZE).unwrap()
    };
}
