
pub trait Serial {
    // On success: The valid received part (<= buf.len()). Can be &[] (if no bytes)
    // On error: TODO
    fn recv<'a>(&mut self, buf: &'a mut [u8]) -> Result<&'a mut [u8], ()>;

    // On success: All bytes were sent/enqueued.
    // On error: the portion of bytes that were NOT sent (the remainder). (<= buf.len()).
    // CANNOT be &[].
    fn send<'a>(&mut self, buf: &'a [u8]) -> Result<(), &'a [u8]>;
}

pub struct Machine {
    pub serial: &'static mut dyn Serial,
    // TODO: port router?
    // TODO: flash manager?
}

// TODO: For now, assume all syscalls are blocking

// TODO: I'll probably want multiple byte-slabs. I should probably
//   add some way to type-erase them completely, as well as probably add
//   an async variant that has some kind of completion flag alongside the
//   refcount and storage contents. Aaaalso probably some abi-safe way
//   of dealing with them across the FFI barrier (probably with raw ptrs)
//
//   At some point I should probably just consider whether a general purpose
//   heap is actually what I want instead. I could likely just use alloc-cortex-m
//   (or the underlying linked list allocator) directly.
