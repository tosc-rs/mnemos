#![no_std]

extern crate alloc;

mod i2c_puppet;

use self::{
    ccu::Ccu,
    dmac::Dmac,
    drivers::{
        smhc::Smhc,
        spim::{self, SpiSenderServer},
        twi,
        uart::{self, D1Uart, Uart},
    },
    plic::{Plic, Priority},
    timer::{Timer, TimerMode, TimerPrescaler, Timers},
    trap::Trap,
};
use core::{
    fmt::Write,
    panic::PanicInfo,
    sync::atomic::{AtomicBool, Ordering},
};
use d1_pac::{Interrupt, TIMER};
use kernel::{
    mnemos_alloc::containers::Box,
    tracing::{self, Instrument},
    Kernel, KernelServiceSettings, KernelSettings,
};
pub use mnemos_d1_core::*;

pub use d1_config::PlatformConfig;
use d1_config::{LedBlinkPin, Mapping};

const HEAP_SIZE: usize = 384 * 1024 * 1024;

#[link_section = ".aheap.AHEAP"]
#[used]
static AHEAP_BUF: Ram<HEAP_SIZE> = Ram::new();

pub fn kernel_entry(config: mnemos_config::MnemosConfig<PlatformConfig>) -> ! {
    unsafe {
        initialize_heap(&AHEAP_BUF);
    }

    let mut p = unsafe { d1_pac::Peripherals::steal() };

    let mut ccu = Ccu::new(p.CCU);
    ccu.sys_clock_init();

    let uart = unsafe { uart::kernel_uart(&mut ccu, &mut p.GPIO, p.UART0) };
    let spim = unsafe { spim::kernel_spim1(p.SPI_DBI, &mut ccu, &mut p.GPIO) };
    let smhc0 = unsafe { Smhc::smhc0(p.SMHC0, &mut ccu, &mut p.GPIO) };

    let i2c0 = match config.platform.i2c {
        d1_config::I2cConfiguration { enabled: false, .. } => None,
        d1_config::I2cConfiguration {
            enabled: true,
            mapping: Mapping::Twi2,
        } => unsafe { Some(twi::I2c0::lichee_rv_dock(p.TWI2, &mut ccu, &mut p.GPIO)) },
        d1_config::I2cConfiguration {
            enabled: true,
            mapping: Mapping::Twi0,
        } => unsafe { Some(twi::I2c0::mq_pro(p.TWI0, &mut ccu, &mut p.GPIO)) },
        d1_config::I2cConfiguration {
            enabled: true,
            mapping,
        } => unimplemented!("unsupported I2C0 TWI mapping: {mapping:?}"),
    };

    #[cfg(feature = "i2c_puppet")]
    let i2c_puppet_enabled = i2c0.is_some() && config.platform.i2c_puppet.enabled;

    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut ccu);
    let plic = Plic::new(p.PLIC);

    let d1 = D1::initialize(
        timers,
        dmac,
        uart,
        spim,
        smhc0,
        plic,
        i2c0,
        config.kernel,
        config.services,
    );

    #[cfg(feature = "i2c_puppet")]
    if i2c_puppet_enabled {
        i2c_puppet::initialize(config.platform.i2c_puppet, d1.kernel, &p.GPIO, &d1.plic);
    }

    if config.platform.blink_service.enabled {
        let interval = config.platform.blink_service.blink_interval;
        match config.platform.blink_service.blink_pin {
            LedBlinkPin::PC1 => {
                p.GPIO.pc_cfg0.modify(|_r, w| {
                    w.pc1_select().output();
                    w
                });
                p.GPIO.pc_dat.modify(|_r, w| {
                    w.pc_dat().variant(0b0000_0010);
                    w
                });

                // Initialize LED loop
                d1.kernel
                    .initialize(async move {
                        loop {
                            p.GPIO.pc_dat.modify(|_r, w| {
                                w.pc_dat().variant(0b0000_0010);
                                w
                            });
                            d1.kernel.sleep(interval).await;
                            p.GPIO.pc_dat.modify(|_r, w| {
                                w.pc_dat().variant(0b0000_0000);
                                w
                            });
                            d1.kernel.sleep(interval).await;
                        }
                    })
                    .unwrap();
            }
            LedBlinkPin::PD18 => {
                p.GPIO.pd_cfg2.modify(|_r, w| {
                    w.pd18_select().output();
                    w
                });
                p.GPIO.pd_dat.modify(|_r, w| {
                    w.pd_dat().variant(1 << 18);
                    w
                });

                // Initialize LED loop
                d1.kernel
                    .initialize(async move {
                        loop {
                            p.GPIO.pd_dat.modify(|_r, w| {
                                w.pd_dat().variant(1 << 18);
                                w
                            });
                            d1.kernel.sleep(interval).await;
                            p.GPIO.pd_dat.modify(|_r, w| {
                                w.pd_dat().variant(0);
                                w
                            });
                            d1.kernel.sleep(interval).await;
                        }
                    })
                    .unwrap();
            }
        }
    }

    #[cfg(feature = "sharp-display")]
    d1.initialize_sharp_display();

    d1.run()
}

pub struct D1 {
    pub kernel: &'static Kernel,
    pub timers: Timers,
    pub plic: Plic,
    pub dmac: Dmac,
    _uart: Uart,
    _spim: spim::Spim1,
    i2c0_int: Option<(Interrupt, fn())>,
}

impl D1 {
    /// Initialize MnemOS for the D1.
    ///
    /// This function configures the hardware platform and spawns driver
    /// services for SPI and UART, as well as the Serial Mux and Tracing
    /// services.
    ///
    /// **Note**: Initialize the global allocator prior to calling this
    /// function.
    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        timers: Timers,
        dmac: Dmac,
        uart: Uart,
        spim: spim::Spim1,
        smhc: Smhc,
        plic: Plic,
        i2c0: Option<twi::I2c0>,
        kernel_settings: KernelSettings,
        service_settings: KernelServiceSettings,
    ) -> Self {
        let k = unsafe {
            Box::into_raw(Kernel::new(kernel_settings).expect("cannot initialize kernel"))
                .as_ref()
                .unwrap()
        };

        k.initialize_default_services(service_settings);

        // Initialize SPI stuff
        k.initialize(async move {
            // Register a new SpiSenderServer
            SpiSenderServer::register(k, dmac, 4).await.unwrap();
        })
        .unwrap();

        // Initialize SimpleSerial driver
        k.initialize(async move {
            D1Uart::register(k, dmac, Default::default()).await.unwrap();
        })
        .unwrap();

        // Initialize the I2C0 TWI
        let i2c0_int = i2c0.map(|i2c0| {
            let i2c0_int = i2c0.interrupt();
            k.initialize(
                async {
                    tracing::debug!("initializing I2C0 TWI...");
                    i2c0.register(k, 4).await.unwrap();
                    tracing::info!("I2C0 TWI initialized!");
                }
                .instrument(tracing::info_span!("I2C0")),
            )
            .unwrap();
            i2c0_int
        });

        // Initialize SMHC driver
        k.initialize(async {
            tracing::debug!("initializing SMHC...");
            smhc.register(k, 4).await.unwrap();
            tracing::debug!("SMHC initialized!");
        })
        .unwrap();

        Self {
            kernel: k,
            _uart: uart,
            _spim: spim,
            timers,
            plic,
            dmac,
            i2c0_int,
        }
    }

    #[cfg(feature = "sharp-display")]
    /// Spawns a SHARP Memory Display driver and a graphical Forth REPL on the Sharp
    /// Memory Display.
    ///
    /// This function requires a SHARP memory display to be connected to the D1's
    /// SPI_DBI pins (SPI1).
    ///
    /// # Panics
    ///
    /// If the SHARP Memory Display driver or the graphical Forth REPL tasks
    /// could not be spawned.
    pub fn initialize_sharp_display(&self) {
        use drivers::sharp_display::SharpDisplay;
        use kernel::daemons::shells;

        // the `'static` kernel reference is the only thing from `self` that
        // must be moved into the spawned tasks.
        let k = self.kernel;

        let sharp_display = self
            .kernel
            .initialize(SharpDisplay::register(k))
            .expect("failed to spawn SHARP display driver");

        // spawn Forth shell
        self.kernel
            .initialize(async move {
                tracing::debug!("waiting for SHARP display driver...");
                sharp_display
                    .await
                    .expect("display driver task isn't cancelled")
                    .expect("display driver must come up");
                tracing::debug!("display driver ready!");
                let settings = shells::GraphicalShellSettings::with_display_size(
                    SharpDisplay::WIDTH as u32,
                    SharpDisplay::HEIGHT as u32,
                );
                k.spawn(shells::graphical_shell_mono(k, settings)).await;
                tracing::info!("graphical shell running.");
            })
            .expect("failed to spawn graphical forth shell");
    }

    pub fn run(self) -> ! {
        let Self {
            kernel: k,
            timers,
            plic,
            dmac: _,
            _uart,
            _spim,
            i2c0_int,
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
            plic.register(Interrupt::DMAC_NS, Dmac::handle_interrupt);
            plic.register(Interrupt::UART0, D1Uart::handle_uart0_int);
            plic.register(Interrupt::SMHC0, Smhc::handle_smhc0_interrupt);

            plic.activate(Interrupt::DMAC_NS, Priority::P1).unwrap();
            plic.activate(Interrupt::UART0, Priority::P1).unwrap();
            plic.activate(Interrupt::SMHC0, Priority::P1).unwrap();

            if let Some((i2c0_int, i2c0_isr)) = i2c0_int {
                plic.register(i2c0_int, i2c0_isr);
                plic.activate(i2c0_int, Priority::P1).unwrap();
            }
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
        unsafe {
            Dmac::cancel_all();
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

// ----

use core::ptr::NonNull;
use kernel::mnemos_alloc::heap::{MnemosAlloc, SingleThreadedLinkedListAllocator};

#[global_allocator]
static AHEAP: MnemosAlloc<SingleThreadedLinkedListAllocator> = MnemosAlloc::new();

/// Initialize the heap.
///
/// # Safety
///
/// Only call this once!
pub unsafe fn initialize_heap<const HEAP_SIZE: usize>(buf: &'static Ram<HEAP_SIZE>) {
    AHEAP
        .init(NonNull::new(buf.as_ptr()).unwrap(), HEAP_SIZE)
        .expect("heap should only be initialized once!");
}

#[panic_handler]
fn handler(info: &PanicInfo) -> ! {
    D1::handle_panic(info)
}

#[export_name = "ExceptionHandler"]
fn exception_handler(trap_frame: &riscv_rt::TrapFrame) -> ! {
    match Trap::from_mcause().expect("mcause should never be invalid") {
        Trap::Interrupt(int) => {
            unreachable!("the exception handler should only recieve exception traps, but got {int}")
        }
        Trap::Exception(exn) => {
            let mepc = riscv::register::mepc::read();
            panic!(
                "CPU exception: {exn} ({exn:#X}) at {mepc:#X}\n\n{:#X}",
                trap::PrettyTrapFrame::from(trap_frame),
            );
        }
    }
}

#[export_name = "MachineExternal"]
fn im_an_interrupt() {
    // tell the kernel that we are inside an ISR. currently, this just results
    // in switching tracing buffers to use a special ISR tracebuf, in case the
    // interrupt fired while someone was holding a tracebuf WGR.
    let _in_isr = kernel::isr::Isr::enter();
    unsafe { Plic::summon().dispatch_interrupt() };
}
