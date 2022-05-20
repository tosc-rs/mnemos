use crate::traits::RandFill;
use nrf52840_hal::rng::Rng;

pub struct HWRng {
    rng: Rng,
}

impl HWRng {
    pub fn new(rng: Rng) -> Self {
        Self {
            rng
        }
    }
}

impl RandFill for HWRng {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), ()> {
        buf.iter_mut().for_each(|b| {
            *b = self.rng.random_u8();
        });
        Ok(())
    }
}
