use super::nrf52_spim_nonblocking::SpimHandle;

pub struct Vs1053b {
    xcs: SpimHandle,
    xdcs: SpimHandle,
}

impl Vs1053b {
    pub fn from_handles(xcs: SpimHandle, xdcs: SpimHandle) -> Self {
        Self { xcs, xdcs }
    }
}
