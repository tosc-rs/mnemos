#![no_std]

extern crate alloc;

pub mod dmac;
pub mod drivers;
pub mod plic;
mod ram;
pub mod timer;

use core::{
    fmt::Write,
    panic::PanicInfo,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use d1_pac::{Interrupt, DMAC, TIMER};
use kernel::{
    daemons::sermux::{hello, loopback, HelloSettings, LoopbackSettings},
    mnemos_alloc::containers::Box,
    services::serial_mux::SerialMuxServer,
    trace::{self, Instrument},
    Kernel, KernelSettings,
};

pub use self::ram::Ram;
use self::{
    dmac::Dmac,
    drivers::{
        spim::{self, SpiSenderServer},
        twi,
        uart::{D1Uart, Uart},
    },
    plic::{Plic, Priority},
    timer::{Timer, TimerMode, TimerPrescaler, Timers},
};

pub struct D1 {
    pub kernel: &'static Kernel,
    pub timers: Timers,
    pub plic: Plic,
    _uart: Uart,
    _spim: spim::Spim1,
}

static COLLECTOR: trace::SerialCollector = trace::SerialCollector::new();

impl D1 {
    /// Initialize MnemOS for the D1.
    ///
    /// This function configures the hardware platform and spawns driver
    /// services for SPI and UART, as well as the Serial Mux and Tracing
    /// services.
    ///
    /// **Note**: Initialize the global allocator prior to calling this
    /// function.
    pub fn initialize(
        timers: Timers,
        uart: Uart,
        spim: spim::Spim1,
        dmac: Dmac,
        plic: Plic,
        mut twi0: twi::Twi0Engine,
    ) -> Result<Self, ()> {
        let k_settings = KernelSettings {
            max_drivers: 16,
            // Note: The timers used will be configured to 3MHz, leading to (approximately)
            // 333ns granularity.
            timer_granularity: Duration::from_nanos(333),
        };
        let k = unsafe {
            Box::into_raw(Kernel::new(k_settings).map_err(drop)?)
                .as_ref()
                .unwrap()
        };

        let [ch0, _, ch2, ch3, ..] = dmac.channels;
        dmac.dmac.dmac_irq_en0.modify(|_r, w| {
            // used for UART0 DMA sending
            w.dma0_queue_irq_en().enabled();
            // used for SPI1 DMA sending
            w.dma1_queue_irq_en().enabled();
            // // used for TWI0 driver DMA sending
            // w.dma2_queue_irq_en().enabled();
            // // used for TWI0 driver DMA recv
            // w.dma3_queue_irq_en().enabled();
            w
        });

        // Initialize SPI stuff
        k.initialize(async move {
            // Register a new SpiSenderServer
            SpiSenderServer::register(k, 4).await.unwrap();
        })
        .unwrap();

        // Initialize SimpleSerial driver
        k.initialize(async move {
            D1Uart::register(k, 4096, 4096, ch0).await.unwrap();
        })
        .unwrap();

        // Initialize the SerialMuxServer
        k.initialize({
            const PORTS: usize = 16;
            const FRAME_SIZE: usize = 512;
            async {
                // * Up to 16 virtual ports max
                // * Framed messages up to 512 bytes max each
                tracing::debug!("initializing SerialMuxServer...");
                SerialMuxServer::register(k, PORTS, FRAME_SIZE)
                    .await
                    .unwrap();
                tracing::info!("SerialMuxServer initialized!");
            }
            .instrument(tracing::info_span!(
                "SerialMuxServer",
                ports = PORTS,
                frame_size = FRAME_SIZE
            ))
        })
        .unwrap();

        // initialize tracing
        k.initialize(async move {
            COLLECTOR.start(k).await;
        })
        .unwrap();

        // Spawn a loopback port
        let loopback_settings = LoopbackSettings::default();
        k.initialize(loopback(k, loopback_settings)).unwrap();

        // Spawn a hello port
        let hello_settings = HelloSettings::default();
        k.initialize(hello(k, hello_settings)).unwrap();

        k.initialize(async move {
            use kernel::services::i2c;
            k.sleep(Duration::from_secs(4)).await;
            trace::info!("trying TWI0 tx...");
            // try to read the part ID from the ENS160...
            let res = twi0.write(i2c::Addr::SevenBit(0x53), &[0x00]).await;
            trace::info!("TWI0 send to 0x53: {res:?}");

            let mut buf = [core::mem::MaybeUninit::<u8>::new(0); 2];
            let res = twi0.read(i2c::Addr::SevenBit(0x53), &mut buf[..]).await;
            match res {
                Ok(_) => unsafe {
                    let lo = buf[0].assume_init();
                    let hi = buf[1].assume_init();
                    trace::info!("TWI0 read 2 bytes from 0x53: [{lo:#x}, {hi:#x}]");
                },
                Err(error) => trace::error!("TWI0 read from 0x53: {error:?}"),
            }
        })
        .unwrap();

        Ok(Self {
            kernel: k,
            _uart: uart,
            _spim: spim,
            timers,
            plic,
        })
    }

    pub fn run(self) -> ! {
        let Self {
            kernel: k,
            timers,
            plic,
            _uart,
            _spim,
        } = self;

        // Timer0 is used as a freewheeling rolling timer.
        // Timer1 is used to generate "sleep until" interrupts
        //
        // Both are at a time base of 3M ticks/s.
        //
        // In the future, we probably want to rework this to use the RTC timer for
        // both purposes, as this will likely play better with sleep power usage.
        let Timers {
            mut timer0,
            mut timer1,
        } = timers;

        // NOTE: if you change the timer frequency, make sure you update
        // initialize_kernel() below to correct the kernel timer wheel
        // granularity setting!
        timer0.set_prescaler(TimerPrescaler::P8); // 24M / 8:  3.00M ticks/s
        timer1.set_prescaler(TimerPrescaler::P8);
        timer0.set_mode(TimerMode::PERIODIC);
        timer1.set_mode(TimerMode::SINGLE_COUNTING);
        let _ = timer0.get_and_clear_interrupt();
        let _ = timer1.get_and_clear_interrupt();

        unsafe {
            riscv::interrupt::enable();
            riscv::register::mie::set_mext();
        }

        unsafe {
            plic.register(Interrupt::TIMER1, Self::timer1_int);
            plic.register(Interrupt::DMAC_NS, Self::handle_dmac);
            plic.register(Interrupt::UART0, D1Uart::handle_uart0_int);
            plic.register(Interrupt::TWI0, twi::Twi0Engine::handle_interrupt);

            plic.activate(Interrupt::DMAC_NS, Priority::P1).unwrap();
            plic.activate(Interrupt::UART0, Priority::P1).unwrap();
            plic.activate(Interrupt::TWI0, Priority::P1).unwrap();
        }

        timer0.start_counter(0xFFFF_FFFF);

        loop {
            // Tick the scheduler
            let start = timer0.current_value();
            let tick = k.tick();

            // Timer is downcounting
            let elapsed = start.wrapping_sub(timer0.current_value());
            let turn = k.timer().force_advance_ticks(elapsed.into());

            // If there is nothing else scheduled, and we didn't just wake something up,
            // sleep for some amount of time
            if turn.expired == 0 && !tick.has_remaining {
                let wfi_start = timer0.current_value();

                // TODO(AJM): Sometimes there is no "next" in the timer wheel, even though there should
                // be. Don't take lack of timer wheel presence as the ONLY heuristic of whether we
                // should just wait for SOME interrupt to occur. For now, force a max sleep of 100ms
                // which is still probably wrong.
                let amount = turn.ticks_to_next_deadline().unwrap_or(100 * 1000 * 3); // 3 ticks per us, 1000 us per ms, 100ms sleep

                // Don't sleep for too long until james figures out wrapping timers
                let amount = amount.min(0x4000_0000) as u32;
                let _ = timer1.get_and_clear_interrupt();
                unsafe {
                    plic.activate(Interrupt::TIMER1, Priority::P1).unwrap();
                }
                timer1.set_interrupt_en(true);
                timer1.start_counter(amount);

                unsafe {
                    riscv::asm::wfi();
                }
                // Disable the timer interrupt in case that wasn't what woke us up
                plic.deactivate(Interrupt::TIMER1).unwrap();
                timer1.set_interrupt_en(false);
                timer1.stop();

                // Account for time slept
                let elapsed = wfi_start.wrapping_sub(timer0.current_value());
                let _turn = k.timer().force_advance_ticks(elapsed.into());
            }
        }
    }

    /// DMAC ISR handler
    ///
    /// At the moment, we service the interrupts on the following channels:
    /// * Channel 0: UART0 TX
    /// * Channel 1: SPI1 TX
    /// * Channel 2: TWI0 driver TX
    fn handle_dmac() {
        let dmac = unsafe { &*DMAC::PTR };
        dmac.dmac_irq_pend0.modify(|r, w| {
            if r.dma0_queue_irq_pend().bit_is_set() {
                D1Uart::tx_done_waker().wake();
            }

            if r.dma1_queue_irq_pend().bit_is_set() {
                spim::SPI1_TX_DONE.wake();
            }

            if r.dma2_queue_irq_pend().bit_is_set() {
                twi::TWI0_DRV_TX_DONE.wake();
            }

            if r.dma3_queue_irq_pend().bit_is_set() {
                twi::TWI0_DRV_RX_DONE.wake();
            }

            // Will write-back and high bits
            w
        });
    }

    /// Timer1 ISR handler
    ///
    /// We don't actually do anything in the TIMER1 interrupt. It is only here to
    /// knock us out of WFI. Just disable the IRQ to prevent refires
    fn timer1_int() {
        let timer = unsafe { &*TIMER::PTR };
        timer
            .tmr_irq_sta
            .modify(|_r, w| w.tmr1_irq_pend().set_bit());

        // Wait for the interrupt to clear to avoid repeat interrupts
        while timer.tmr_irq_sta.read().tmr1_irq_pend().bit_is_set() {}
    }

    pub fn handle_panic(info: &PanicInfo) -> ! {
        // Disable interrupts.
        unsafe {
            riscv::interrupt::disable();
        }

        // Avoid double panics.
        static PANICKING: AtomicBool = AtomicBool::new(false);
        if PANICKING.swap(true, Ordering::SeqCst) {
            die();
        }

        // Cancel any in-flight DMA requests. It's particularly important to
        // cancel the UART TX DMA channel, because we're about to dump the panic
        // message to the UART, but we may as well tear down any other in-flight
        // DMAs.
        for idx in 0..Dmac::CHANNEL_COUNT {
            unsafe {
                let mut ch = dmac::Channel::summon_channel(idx);
                ch.stop_dma();
            }
        }

        // Ugly but works
        let mut uart: Uart = unsafe { core::mem::transmute(()) };

        // end any existing SerMux frame on the UART
        uart.write(&[0]);

        // write out the panic message in plaintext
        write!(&mut uart, "\r\n{info}\r\n").ok();
        // end the SerMux frame so crowtty can decode the panic message as utf8
        uart.write(&[0]);

        write!(
            &mut uart,
            "you've met with a terrible fate, haven't you?\r\n"
        )
        .ok();
        uart.write(&[0]);

        die();

        /// to sleep, perchance to dream; aye, there's the rub,
        /// for in that sleep of death, what dreams may come?
        fn die() -> ! {
            loop {
                // wait for an interrupt to pause the CPU. since we just
                // disabled interrupts above, this will keep the CPU in a low
                // power state until it's reset.
                unsafe {
                    riscv::asm::wfi();
                }
            }
        }
    }
}
