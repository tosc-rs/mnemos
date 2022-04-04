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
