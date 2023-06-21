pub use maitake::time::{Duration, Sleep, Timeout, TimerError, sleep, try_sleep, timeout, try_timeout};

/// Implementation of [`embedded_hal_async::delay::DelayUs`] for the `maitake`
/// timer.
pub struct Delay;

impl embedded_hal_async::delay::DelayUs for Delay {
    async fn delay_us(&mut self, us: u32) {
        sleep(Duration::from_micros(us as u64)).await;
    }

    async fn delay_ms(&mut self, ms: u32) {
        sleep(Duration::from_millis(ms as u64)).await;
    }
}