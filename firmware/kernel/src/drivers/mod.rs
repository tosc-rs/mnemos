// NOTE: These are all probably going to be nrf52840 specific
// for now. Later I'll probably break these out into some kind
// of crate with a defined interface.

pub mod usb_serial;
pub mod gd25q16;
pub mod nrf52_pin;
pub mod nrf52_spim_blocking;
pub mod nrf52_spim_nonblocking;
