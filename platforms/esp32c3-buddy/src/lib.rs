#![no_std]
extern crate alloc;

pub mod drivers;
pub mod heap;

use critical_section::Mutex;
use esp32c3_hal::{
    interrupt,
    peripherals::{self, Interrupt},
    prelude::*,
    system,
    systimer::{Alarm, SystemTimer, Target},
    Cpu,
};
use esp_backtrace as _;

use core::{cell::RefCell, time::Duration};
use kernel::{daemons, maitake, mnemos_alloc::containers::Box, services, Kernel, KernelSettings};

static ALARM1: Mutex<RefCell<Option<Alarm<Target, 1>>>> = Mutex::new(RefCell::new(None));

pub fn init() -> &'static Kernel {
    let k_settings = KernelSettings { max_drivers: 16 };
    let clock = {
        // the system timer has a period of `SystemTimer::TICKS_PER_SECOND` ticks.
        // `TICKS_PER_SECOND` is 16_000_000, so the base granularity is
        // 62.5ns. let's multiply it by 2 so that we have a non-fractional
        // number of nanoseconds.
        maitake::time::Clock::new(
            Duration::from_nanos(125),
            || SystemTimer::now() / 2u64, // well...that was easy!
        )
        .named("CLOCK_SYSTEM_TIMER_NOW")
    };
    unsafe {
        Box::into_raw(Kernel::new(k_settings, clock).expect("cannot initialize kernel"))
            .as_ref()
            .unwrap()
    }
}

pub fn spawn_daemons(k: &'static Kernel) {
    // initialize tracing first, so we can trace the boot process.
    k.initialize(async move {
        use kernel::serial_trace;
        let trace_settings = serial_trace::SerialTraceSettings::default()
            // our heap is only 32KB, so allocate a much smaller trace buffer.
            .with_tracebuf_capacity(1024);
        serial_trace::SerialSubscriber::start(k, trace_settings).await;
    })
    .expect("failed to spawn serial tracing daemon");

    // Initialize the SerialMuxServer
    k.initialize(services::serial_mux::SerialMuxServer::register(
        k,
        Default::default(),
    ))
    .expect("failed to spawn SerialMuxService initialization");

    // Initialize Serial Mux daemons.
    k.initialize(daemons::sermux::hello(k, Default::default()))
        .expect("failed to spawn default serial mux service initialization");
}

pub fn spawn_serial(
    k: &'static Kernel,
    dev: peripherals::USB_DEVICE,
    pcc: &mut system::PeripheralClockControl,
) {
    pcc.enable(system::Peripheral::Sha);

    // spawn SimpleSerial service
    k.initialize(drivers::usb_serial::UsbSerialServer::new(dev).register(k, 512, 512))
        .expect("failed to spawn UsbSerialServer!");

    interrupt::enable(Interrupt::USB_DEVICE, interrupt::Priority::Priority1)
        .expect("failed to enable USB_DEVICE interrupt");
}

pub fn run(k: &'static Kernel, alarm1: Alarm<Target, 1>) -> ! {
    // Alarm 1 will be used to generate "sleep until" interrupts.
    critical_section::with(|cs| {
        ALARM1.borrow_ref_mut(cs).replace(alarm1);
    });

    interrupt::set_kind(
        Cpu::ProCpu,
        interrupt::CpuInterrupt::Interrupt1, // Interrupt 1 handles priority one interrupts
        interrupt::InterruptKind::Edge,
    );
    interrupt::enable(Interrupt::SYSTIMER_TARGET1, interrupt::Priority::Priority1)
        .expect("failed to enable SYSTIMER_TARGET1 interrupt");

    loop {
        tracing::debug!("tick");
        let tick = k.tick();
        let turn = k.timer().turn();

        // If there is nothing else scheduled, and we didn't just wake something up,
        // sleep for some amount of time
        if turn.expired == 0 && !tick.has_remaining {
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
                alarm1.set_target(SystemTimer::now() + (amount * 2));
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
            let _turn = k.timer().turn();
        }
    }
}

/// Systimer ALARM1 ISR handler
///
/// We don't actually do anything in the ALARM0 interrupt. It is only here to
/// knock us out of WFI. Just disable the IRQ to prevent refires
#[interrupt]
#[allow(non_snake_case)]
fn SYSTIMER_TARGET1() {
    critical_section::with(|cs| {
        ALARM1
            .borrow_ref_mut(cs)
            .as_mut()
            .unwrap()
            .clear_interrupt();
    });
}
