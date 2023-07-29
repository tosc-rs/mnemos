#![no_std]
#![no_main]

extern crate alloc;

use critical_section::Mutex;
use esp32c3_hal::{
    clock::ClockControl,
    interrupt, peripherals,
    peripherals::Peripherals,
    prelude::*,
    systimer::{Alarm, SystemTimer, Target},
    timer::TimerGroup,
    Cpu, Rtc, IO,
};
use esp_backtrace as _;
use esp_println::println;
use mnemos_esp32c3_buddy::drivers;

use core::{cell::RefCell, time::Duration};
use kernel::{mnemos_alloc::containers::Box, Kernel, KernelSettings};

static ALARM1: Mutex<RefCell<Option<Alarm<Target, 1>>>> = Mutex::new(RefCell::new(None));

#[entry]
fn main() -> ! {
    unsafe {
        mnemos_esp32c3_buddy::heap::init();
    }

    let peripherals = Peripherals::take();
    let mut system = peripherals.SYSTEM.split();
    let clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    // Disable the RTC and TIMG watchdog timers
    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    let timer_group0 = TimerGroup::new(
        peripherals.TIMG0,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt0 = timer_group0.wdt;
    let timer_group1 = TimerGroup::new(
        peripherals.TIMG1,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt1 = timer_group1.wdt;
    rtc.swd.disable();
    rtc.rwdt.disable();
    wdt0.disable();
    wdt1.disable();
    println!("Hello world!");

    let k = {
        let k_settings = KernelSettings {
            max_drivers: 16,
            // the system timer has a period of `SystemTimer::TICKS_PER_SECOND` ticks.
            // `TICKS_PER_SECOND` is 16_000_000, so the base granularity is
            // 62.5ns. let's multiply it by 2 so that we have a non-fractional
            // number of nanoseconds.
            timer_granularity: Duration::from_nanos(125),
        };
        unsafe {
            Box::into_raw(Kernel::new(k_settings).expect("cannot initialize kernel"))
                .as_ref()
                .unwrap()
        }
    };

    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);

    // initialize SimpleSerial driver
    k.initialize({
        use esp32c3_hal::uart::{
            config::{Config, DataBits, Parity, StopBits},
            TxRxPins, Uart,
        };

        let config = Config {
            baudrate: 115200,
            data_bits: DataBits::DataBits8,
            parity: Parity::ParityNone,
            stop_bits: StopBits::STOP1,
        };

        let pins = TxRxPins::new_tx_rx(
            io.pins.gpio1.into_push_pull_output(),
            io.pins.gpio2.into_floating_input(),
        );

        let uart0 = Uart::new_with_config(
            peripherals.UART0,
            Some(config),
            Some(pins),
            &clocks,
            &mut system.peripheral_clock_control,
        );

        drivers::uart::C3Uart::uart0(uart0).register(k, 4096, 4096)
    })
    .unwrap();

    k.initialize(async move {
        k.sleep(Duration::from_secs(1)).await;
        tracing::info!("i'm alive!");
    })
    .unwrap();

    // configure system timer
    let syst = SystemTimer::new(peripherals.SYSTIMER);

    println!("SYSTIMER Current value = {}", SystemTimer::now());

    // Alarm 1 will be used to generate "sleep until" interrupts.
    let alarm1 = syst.alarm1;

    critical_section::with(|cs| {
        ALARM1.borrow_ref_mut(cs).replace(alarm1);
    });

    interrupt::enable(
        peripherals::Interrupt::UART0,
        interrupt::Priority::Priority1,
    )
    .unwrap();
    interrupt::set_kind(
        Cpu::ProCpu,
        interrupt::CpuInterrupt::Interrupt1, // Interrupt 1 handles priority one interrupts
        interrupt::InterruptKind::Edge,
    );
    interrupt::enable(
        peripherals::Interrupt::SYSTIMER_TARGET1,
        interrupt::Priority::Priority1,
    )
    .unwrap();

    loop {
        // Tick the scheduler
        let start = SystemTimer::now();
        let tick = k.tick();

        // Timer is downcounting
        let elapsed = start.wrapping_sub(SystemTimer::now());
        let turn = k.timer().force_advance_ticks(elapsed.into());

        // If there is nothing else scheduled, and we didn't just wake something up,
        // sleep for some amount of time
        if turn.expired == 0 && !tick.has_remaining {
            let wfi_start = SystemTimer::now();

            // TODO(AJM): Sometimes there is no "next" in the timer wheel, even though there should
            // be. Don't take lack of timer wheel presence as the ONLY heuristic of whether we
            // should just wait for SOME interrupt to occur. For now, force a max sleep of 100ms
            // which is still probably wrong.
            let amount = turn.ticks_to_next_deadline().unwrap_or(800_000); // 100 ms / 125 ms ticks = 800,000

            // TODO(eliza): what is the max duration of the C3's timer?

            critical_section::with(|cs| {
                let mut alarm1 = ALARM1.borrow_ref_mut(cs);
                let alarm1 = alarm1.as_mut().unwrap();
                alarm1.clear_interrupt();
                alarm1.set_target(SystemTimer::now() + amount);
                alarm1.interrupt_enable(true);
            });

            unsafe {
                riscv::asm::wfi();
            }
            // Disable the timer interrupt in case that wasn't what woke us up
            critical_section::with(|cs| {
                ALARM1
                    .borrow_ref_mut(cs)
                    .as_mut()
                    .unwrap()
                    .interrupt_enable(false);
            });

            // Account for time slept
            let elapsed = wfi_start.wrapping_sub(SystemTimer::now());
            let _turn = k.timer().force_advance_ticks(elapsed.into());
        }
    }
}

/// Systimer ALARM1 ISR handler
///
/// We don't actually do anything in the ALARM0 interrupt. It is only here to
/// knock us out of WFI. Just disable the IRQ to prevent refires
#[interrupt]
fn SYSTIMER_TARGET1() {
    critical_section::with(|cs| {
        ALARM1
            .borrow_ref_mut(cs)
            .as_mut()
            .unwrap()
            .clear_interrupt();
    });
}
