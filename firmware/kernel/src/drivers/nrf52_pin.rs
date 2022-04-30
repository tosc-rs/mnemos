use crate::traits::{GpioPin, OutputPin};
use common::syscall::request::GpioMode;
use nrf52840_hal::{
    gpio::{Disconnected, Floating, Input, Level, Output, Pin, PullDown, PullUp, PushPull},
    prelude::{InputPin, OutputPin as _},
};

pub enum MPin {
    Disabled(Pin<Disconnected>),
    InputFloating(Pin<Input<Floating>>),
    InputPullUp(Pin<Input<PullUp>>),
    InputPullDown(Pin<Input<PullDown>>),
    OutputPushPull(Pin<Output<PushPull>>),
    Invalid,
}

impl MPin {
    pub fn new(pin: Pin<Disconnected>) -> Self {
        Self::Disabled(pin)
    }

    pub fn new_input_floating(pin: Pin<Input<Floating>>) -> Self {
        MPin::InputFloating(pin)
    }

    pub fn into_disabled(&mut self) {
        let mut tmp = MPin::Invalid;
        core::mem::swap(self, &mut tmp);
        *self = MPin::Disabled(match tmp {
            MPin::Disabled(pin) => pin.into_disconnected(),
            MPin::InputFloating(pin) => pin.into_disconnected(),
            MPin::InputPullUp(pin) => pin.into_disconnected(),
            MPin::InputPullDown(pin) => pin.into_disconnected(),
            MPin::OutputPushPull(pin) => pin.into_disconnected(),
            MPin::Invalid => {
                defmt::panic!("Internal Error!");
            }
        });
    }

    pub fn into_floating_input(&mut self) {
        let mut tmp = MPin::Invalid;
        core::mem::swap(self, &mut tmp);
        *self = MPin::InputFloating(match tmp {
            MPin::Disabled(pin) => pin.into_floating_input(),
            MPin::InputFloating(pin) => pin.into_floating_input(),
            MPin::InputPullUp(pin) => pin.into_floating_input(),
            MPin::InputPullDown(pin) => pin.into_floating_input(),
            MPin::OutputPushPull(pin) => pin.into_floating_input(),
            MPin::Invalid => {
                defmt::panic!("Internal Error!");
            }
        });
    }

    pub fn into_pullup_input(&mut self) {
        let mut tmp = MPin::Invalid;
        core::mem::swap(self, &mut tmp);
        *self = MPin::InputPullUp(match tmp {
            MPin::Disabled(pin) => pin.into_pullup_input(),
            MPin::InputFloating(pin) => pin.into_pullup_input(),
            MPin::InputPullUp(pin) => pin.into_pullup_input(),
            MPin::InputPullDown(pin) => pin.into_pullup_input(),
            MPin::OutputPushPull(pin) => pin.into_pullup_input(),
            MPin::Invalid => {
                defmt::panic!("Internal Error!");
            }
        });
    }

    pub fn into_pulldown_input(&mut self) {
        let mut tmp = MPin::Invalid;
        core::mem::swap(self, &mut tmp);
        *self = MPin::InputPullDown(match tmp {
            MPin::Disabled(pin) => pin.into_pulldown_input(),
            MPin::InputFloating(pin) => pin.into_pulldown_input(),
            MPin::InputPullUp(pin) => pin.into_pulldown_input(),
            MPin::InputPullDown(pin) => pin.into_pulldown_input(),
            MPin::OutputPushPull(pin) => pin.into_pulldown_input(),
            MPin::Invalid => {
                defmt::panic!("Internal Error!");
            }
        });
    }

    pub fn into_push_pull_output(&mut self, is_high: bool) {
        let level = match is_high {
            true => Level::High,
            false => Level::Low,
        };
        let mut tmp = MPin::Invalid;
        core::mem::swap(self, &mut tmp);
        *self = MPin::OutputPushPull(match tmp {
            MPin::Disabled(pin) => pin.into_push_pull_output(level),
            MPin::InputFloating(pin) => pin.into_push_pull_output(level),
            MPin::InputPullUp(pin) => pin.into_push_pull_output(level),
            MPin::InputPullDown(pin) => pin.into_push_pull_output(level),
            MPin::OutputPushPull(pin) => pin.into_push_pull_output(level),
            MPin::Invalid => {
                defmt::panic!("Internal Error!");
            }
        });
    }

    pub fn read_nrf_pin(&self) -> Result<bool, ()> {
        match self {
            MPin::InputFloating(pin) => pin.is_high().map_err(drop),
            MPin::InputPullUp(pin) => pin.is_high().map_err(drop),
            MPin::InputPullDown(pin) => pin.is_high().map_err(drop),
            MPin::Invalid | MPin::Disabled(_) | MPin::OutputPushPull(_) => Err(()),
        }
    }

    pub fn set_nrf_pin(&mut self, is_high: bool) -> Result<(), ()> {
        if let MPin::OutputPushPull(pin) = self {
            if is_high {
                pin.set_high().map_err(drop)
            } else {
                pin.set_low().map_err(drop)
            }
        } else {
            Err(())
        }
    }
}

impl GpioPin for MPin {
    fn set_mode(&mut self, mode: GpioMode) -> Result<(), ()> {
        match mode {
            GpioMode::Disabled => self.into_disabled(),
            GpioMode::InputFloating => self.into_floating_input(),
            GpioMode::InputPullUp => self.into_pullup_input(),
            GpioMode::InputPullDown => self.into_pulldown_input(),
            GpioMode::OutputPushPull { is_high } => self.into_push_pull_output(is_high),
        };
        Ok(())
    }

    fn read_pin(&mut self) -> Result<bool, ()> {
        self.read_nrf_pin()
    }

    fn set_pin(&mut self, is_high: bool) -> Result<(), ()> {
        self.set_nrf_pin(is_high)
    }
}

impl OutputPin for Pin<Output<PushPull>> {
    fn set_pin(&mut self, is_high: bool) {
        use nrf52840_hal::prelude::OutputPin as _;
        if is_high {
            let _ = self.set_high();
        } else {
            let _ = self.set_low();
        }
    }
}
