// NOTE: These are all probably going to be nrf52840 specific
// for now. Later I'll probably break these out into some kind
// of crate with a defined interface.

pub mod gd25q16;
pub mod nrf52_pin;
pub mod nrf52_spim_nonblocking;
pub mod usb_serial;
pub mod vs1053b;
