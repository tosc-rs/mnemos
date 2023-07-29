#![no_std]
extern crate alloc;

pub mod drivers;
pub mod heap;

use critical_section::Mutex;
use esp32c3_hal::{
    interrupt, peripherals,
    prelude::*,
    systimer::{Alarm, SystemTimer, Target},
    Cpu,
};
use esp_backtrace as _;
use esp_println::println;

use core::{time::Duration, cell::RefCell};
use kernel::{daemons, services, Kernel, KernelSettings, mnemos_alloc::containers::Box};

static ALARM1: Mutex<RefCell<Option<Alarm<Target, 1>>>> = Mutex::new(RefCell::new(None));

pub fn init() -> &'static Kernel {
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
}

pub fn spawn_daemons(k: &'static Kernel) {

    // Initialize the SerialMuxServer
    let sermux_up = k
        .initialize(services::serial_mux::SerialMuxServer::register(
            k,
            Default::default(),
        ))
        .expect("failed to spawn SerialMuxService initialization");

    // Initialize Serial Mux daemons.
    k.initialize(async move {
        sermux_up
            .await
            .expect("SerialMuxService initialization should not be cancelled")
            .expect("SerialMuxService initialization failed");

        kernel::serial_trace::SerialSubscriber::start(k, Default::default()).await;

        k.spawn(daemons::sermux::loopback(k, Default::default()))
            .await;
        println!("SerMux loopback started");

        k.spawn(daemons::sermux::hello(k, Default::default())).await;
        tracing::debug!("SerMux Hello World started");
    })
    .expect("failed to spawn default serial mux service initialization");

    k.initialize(async move {
        loop {
            k.sleep(Duration::from_secs(1)).await;
            tracing::info!("i'm alive!");
        }
    })
    .unwrap();
}

pub fn run(k: &'static Kernel, alarm1: Alarm<Target, 1>) -> ! {
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
        let elapsed = SystemTimer::now() - start;

        println!("Start: {start:}, Elapsed: {elapsed:?}");
        let turn = k.timer().force_advance_ticks(elapsed as u64 / 2u64);

        println!("Tick: {tick:?}\nTurn: {turn:?}");

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
            println!("amount: {amount}");
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
            let elapsed = SystemTimer::now() - wfi_start;

            println!("WFI Start: {wfi_start:}, Elapsed: {elapsed:?}");
            let turn = k.timer().force_advance_ticks(elapsed as u64 / 2u64);
            println!("Turn: {turn:?}; elapsed = {elapsed:?}");
        }
    }
}

/// Systimer ALARM1 ISR handler
///
/// We don't actually do anything in the ALARM0 interrupt. It is only here to
/// knock us out of WFI. Just disable the IRQ to prevent refires
#[interrupt]
fn SYSTIMER_TARGET1() {
    println!("ALARM1");
    critical_section::with(|cs| {
        ALARM1
            .borrow_ref_mut(cs)
            .as_mut()
            .unwrap()
            .clear_interrupt();
    });
}
