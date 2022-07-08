pub(crate) use core::fmt::*;

#[inline]
pub(crate) fn ptr<P: Pointer>(ptr: P) -> DebugPtr<P> {
    DebugPtr(ptr)
}

#[derive(Copy, Clone)]
pub(crate) struct DebugPtr<P: Pointer>(P);

impl<P: Pointer> Debug for DebugPtr<P> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{:p}", self.0)
    }
}
