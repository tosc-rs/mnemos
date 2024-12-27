pub use d1_pac::timer::tmr_ctrl::{
    TMR_CLK_PRES_A as TimerPrescaler, TMR_CLK_SRC_A as TimerSource, TMR_MODE_A as TimerMode,
};
use d1_pac::TIMER;
use kernel::maitake::time::Clock;

pub struct Timers {
    pub timer0: Timer0,
    pub timer1: Timer1,
}

impl Timer0 {
    pub fn into_maitake_clock(mut self) -> Clock {
        self.set_prescaler(TimerPrescaler::P8); // 24M / 8:  3.00M ticks/s
        self.set_mode(TimerMode::PERIODIC);

        let _ = self.get_and_clear_interrupt();

        // Start the timer counting down from 0xffff_ffff_ffff_ffff.

        // TODO(eliza): if it's zero, handle that lol --- when the IRQ fires
        // we need to increment some higher half counter and then reset the
        // timer to u32::MAX?
        self.start_counter(u32::MAX);

        Clock::new(core::time::Duration::from_nanos(333), || {
            let timer0 = unsafe {
                // Safety: we are just reading the current value and will not be
                // concurrently mutating the timer.
                Self::steal()
            };
            // Since timer 0 is counting *down*, we have to subtract its current
            // value from the intial value to get an increasing timestamp for
            // Maitake.
            (u32::MAX - timer0.current_value()) as u64
        })
        .named("CLOCK_D1_TIMER0")
    }

    unsafe fn steal() -> Self {
        Self { _x: () }
    }
}

mod sealed {
    use d1_pac::{
        generic::Reg,
        timer::{
            tmr_ctrl::TMR_CTRL_SPEC, tmr_cur_value::TMR_CUR_VALUE_SPEC,
            tmr_intv_value::TMR_INTV_VALUE_SPEC,
        },
    };

    use super::*;

    pub trait TimerSealed {
        fn ctrl(&self) -> &Reg<TMR_CTRL_SPEC>;
        fn interval(&self) -> &Reg<TMR_INTV_VALUE_SPEC>;
        fn value(&self) -> &Reg<TMR_CUR_VALUE_SPEC>;
        fn set_interrupt_en(&self, enabled: bool);
        fn get_and_clear_interrupt(&self) -> bool;
    }

    impl TimerSealed for Timer0 {
        #[inline(always)]
        fn ctrl(&self) -> &Reg<TMR_CTRL_SPEC> {
            let timer = unsafe { &*TIMER::PTR };
            &timer.tmr0_ctrl
        }

        #[inline(always)]
        fn interval(&self) -> &Reg<TMR_INTV_VALUE_SPEC> {
            let timer = unsafe { &*TIMER::PTR };
            &timer.tmr0_intv_value
        }

        #[inline(always)]
        fn value(&self) -> &Reg<TMR_CUR_VALUE_SPEC> {
            let timer = unsafe { &*TIMER::PTR };
            &timer.tmr0_cur_value
        }

        #[inline(always)]
        fn get_and_clear_interrupt(&self) -> bool {
            let timer = unsafe { &*TIMER::PTR };
            let mut active = false;
            timer.tmr_irq_sta.modify(|r, w| {
                if r.tmr0_irq_pend().bit_is_set() {
                    w.tmr0_irq_pend().set_bit();
                    active = true;
                }
                w
            });
            active
        }

        #[inline(always)]
        fn set_interrupt_en(&self, enabled: bool) {
            let timer = unsafe { &*TIMER::PTR };
            timer.tmr_irq_en.modify(|_r, w| {
                w.tmr0_irq_en().bit(enabled);
                w
            });
        }
    }

    impl TimerSealed for Timer1 {
        #[inline(always)]
        fn ctrl(&self) -> &Reg<TMR_CTRL_SPEC> {
            let timer = unsafe { &*TIMER::PTR };
            &timer.tmr1_ctrl
        }

        #[inline(always)]
        fn interval(&self) -> &Reg<TMR_INTV_VALUE_SPEC> {
            let timer = unsafe { &*TIMER::PTR };
            &timer.tmr1_intv_value
        }

        #[inline(always)]
        fn value(&self) -> &Reg<TMR_CUR_VALUE_SPEC> {
            let timer = unsafe { &*TIMER::PTR };
            &timer.tmr1_cur_value
        }

        #[inline(always)]
        fn get_and_clear_interrupt(&self) -> bool {
            let timer = unsafe { &*TIMER::PTR };
            let mut active = false;
            timer.tmr_irq_sta.modify(|r, w| {
                if r.tmr1_irq_pend().bit_is_set() {
                    w.tmr1_irq_pend().set_bit();
                    active = true;
                }
                w
            });
            active
        }

        #[inline(always)]
        fn set_interrupt_en(&self, enabled: bool) {
            let timer = unsafe { &*TIMER::PTR };
            timer.tmr_irq_en.modify(|_r, w| {
                w.tmr1_irq_en().bit(enabled);
                w
            });
        }
    }

    impl Timer for Timer0 {}
    impl Timer for Timer1 {}
}

pub struct Timer0 {
    _x: (),
}

pub struct Timer1 {
    _x: (),
}

pub trait Timer: sealed::TimerSealed {
    #[inline]
    fn set_source(&mut self, variant: TimerSource) {
        self.ctrl().modify(|_r, w| {
            w.tmr_clk_src().variant(variant);
            w
        });
    }

    #[inline]
    fn set_prescaler(&mut self, variant: TimerPrescaler) {
        self.ctrl().modify(|_r, w| {
            w.tmr_clk_pres().variant(variant);
            w
        });
    }

    #[inline]
    fn set_mode(&mut self, variant: TimerMode) {
        self.ctrl().modify(|_r, w| {
            w.tmr_mode().variant(variant);
            w
        });
    }

    #[inline]
    fn stop(&mut self) {
        self.ctrl().modify(|_r, w| {
            w.tmr_en().clear_bit();
            w
        });
    }

    #[inline]
    fn start_counter(&mut self, interval: u32) {
        self.interval().write(|w| unsafe {
            w.bits(interval);
            w
        });
        // Set the reload AND enable bits at the same time
        // TODO: Reset status flag or interrupt flag?
        self.ctrl().modify(|_r, w| {
            w.tmr_reload().set_bit();
            w.tmr_en().set_bit();
            w
        });
    }

    #[inline]
    fn current_value(&self) -> u32 {
        self.value().read().bits()
    }

    #[inline]
    fn get_and_clear_interrupt(&self) -> bool {
        sealed::TimerSealed::get_and_clear_interrupt(self)
    }

    #[inline]
    fn set_interrupt_en(&self, enabled: bool) {
        sealed::TimerSealed::set_interrupt_en(self, enabled)
    }
}

impl Timers {
    pub fn new(periph: TIMER) -> Self {
        // 1. Configure the timer parameters clock source, prescale factor, and timing mode by writing **TMRn_CTRL_REG**. There is no sequence requirement of configuring the parameters.
        // 2. Write the interval value.
        //     * Write TMRn_INTV_VALUE_REG to configure the interval value for the timer.
        //     * Write bit[1] of TMRn_CTRL_REG to load the interval value to the timer. The value of the bit will be cleared automatically after loading the interval value.
        // 3. Write bit[0] of TMRn_CTRL_REG to start the timer. To get the current value of the timer, read
        // TMRn_CUR_VALUE_REG.
        periph.tmr_irq_en.write(|w| {
            w.tmr0_irq_en().clear_bit();
            w.tmr1_irq_en().clear_bit();
            w
        });

        Self {
            timer0: Timer0 { _x: () },
            timer1: Timer1 { _x: () },
        }
    }
}
