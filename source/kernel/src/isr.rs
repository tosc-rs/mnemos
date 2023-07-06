use portable_atomic::{AtomicU8, Ordering};

static IN_ISR: AtomicU8 = AtomicU8::new(0);

pub struct Isr(());

impl Drop for Isr {
    fn drop(&mut self) {
        IN_ISR.fetch_sub(1, Ordering::Release);
    }
}

impl Isr {
    /// Enter an interrupt service routine (ISR) context.
    ///
    /// When the returned guard is dropped, the system is no longer considered
    /// to be inside an ISR.
    #[must_use]
    #[inline]
    pub fn enter() -> Self {
        IN_ISR.fetch_add(1, Ordering::Release);
        Self(())
    }

    #[must_use]
    #[inline]
    pub fn is_in_isr() -> bool {
        Self::level() > 0
    }

    #[must_use]
    #[inline]
    pub(crate) fn level() -> u8 {
        IN_ISR.load(Ordering::Acquire)
    }
}
