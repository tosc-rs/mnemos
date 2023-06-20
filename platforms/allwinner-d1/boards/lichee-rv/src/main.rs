#![no_std]
#![no_main]

use core::{fmt::Write, panic::PanicInfo, sync::atomic::Ordering, time::Duration};
use d1_pac::{Interrupt, DMAC, TIMER};
use drivers::{
    dmac::Dmac,
    plic::{Plic, Priority},
    timer::{Timer, TimerMode, TimerPrescaler, Timers},
    uart::{kernel_uart, Uart},
    Ram,
};
use kernel::{
    drivers::serial_mux::{RegistrationError, SerialMuxClient, SerialMuxServer},
    Kernel, KernelSettings,
};
use uart::D1Uart;
mod uart;

const HEAP_SIZE: usize = 384 * 1024 * 1024;

#[link_section = ".aheap.AHEAP"]
#[used]
static AHEAP: Ram<HEAP_SIZE> = Ram::new();

/// A helper to initialize the kernel
fn initialize_kernel() -> Result<&'static Kernel, ()> {
    let k_settings = KernelSettings {
        heap_start: AHEAP.as_ptr(),
        heap_size: HEAP_SIZE,
        max_drivers: 16,
        k2u_size: 4096,
        u2k_size: 4096,
        // Note: The timers used will be configured to 3MHz, leading to (approximately)
        // 333ns granularity.
        timer_granularity: Duration::from_nanos(333),
    };
    let k = unsafe { Kernel::new(k_settings).map_err(drop)?.leak().as_ref() };
    Ok(k)
}

#[allow(non_snake_case)]
#[riscv_rt::entry]
fn main() -> ! {
    let mut p = unsafe { d1_pac::Peripherals::steal() };
    let _uart = unsafe { kernel_uart(&mut p.CCU, &mut p.GPIO, p.UART0) };

    p.GPIO.pc_cfg0.modify(|_r, w| {
        w.pc1_select().output();
        w
    });
    p.GPIO.pc_dat.modify(|_r, w| {
        w.pc_dat().variant(0b0000_0010);
        w
    });

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
    } = Timers::new(p.TIMER);

    let k = initialize_kernel().unwrap();
    let dmac = Dmac::new(p.DMAC, &mut p.CCU);
    let [ch0, ..] = dmac.channels;
    dmac.dmac.dmac_irq_en0.modify(|_r, w| {
        w.dma0_queue_irq_en().enabled();
        w
    });

    // Initialize LED loop
    k.initialize(async move {
        loop {
            p.GPIO.pc_dat.modify(|_r, w| {
                w.pc_dat().variant(0b0000_0010);
                w
            });
            k.sleep(Duration::from_millis(250)).await;
            p.GPIO.pc_dat.modify(|_r, w| {
                w.pc_dat().variant(0b0000_0000);
                w
            });
            k.sleep(Duration::from_millis(250)).await;
        }
    })
    .unwrap();

    // Initialize SimpleSerial driver
    k.initialize(async move {
        D1Uart::register(k, 4096, 4096, ch0).await.unwrap();
    })
    .unwrap();

    // Initialize SerialMux
    k.initialize(async move {
        loop {
            // Now, right now this is a little awkward, but what I'm doing here is spawning
            // a new virtual mux, and configuring it with:
            // * Up to 16 virtual ports max
            // * Framed messages up to 512 bytes max each
            match SerialMuxServer::register(k, 16, 512).await {
                Ok(_) => break,
                Err(RegistrationError::SerialPortNotFound) => {
                    // Uart probably isn't registered yet. Try again in a bit
                    k.sleep(Duration::from_millis(10)).await;
                }
                Err(e) => {
                    panic!("uhhhh {e:?}");
                }
            }
        }
    })
    .unwrap();

    // Loopback on virtual port zero
    k.initialize(async move {
        let mut hdl = loop {
            match SerialMuxClient::from_registry(k).await {
                Some(c) => break c,
                None => {
                    k.sleep(Duration::from_millis(100)).await;
                }
            }
        };

        let hdl = hdl.open_port(0, 1024).await.unwrap();

        loop {
            let rx = hdl.consumer().read_grant().await;
            let all_len = rx.len();
            hdl.send(&rx).await;
            rx.release(all_len);
        }
    })
    .unwrap();

    k.initialize(async move {
        let mut hdl = loop {
            match SerialMuxClient::from_registry(k).await {
                Some(c) => break c,
                None => {
                    k.sleep(Duration::from_millis(100)).await;
                }
            }
        };

        let hdl = hdl.open_port(1, 1024).await.unwrap();

        loop {
            hdl.send(b"Hello, world!\r\n").await;
            k.sleep(Duration::from_secs(1)).await;
        }
    })
    .unwrap();

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

    let plic = Plic::new(p.PLIC);

    unsafe {
        plic.register(Interrupt::TIMER1, timer1_int);
        plic.register(Interrupt::DMAC_NS, handle_dmac);
        plic.register(Interrupt::UART0, D1Uart::handle_uart0_int);

        plic.activate(Interrupt::DMAC_NS, Priority::P1).unwrap();
        plic.activate(Interrupt::UART0, Priority::P1).unwrap();
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
        if !tick.has_remaining && turn.expired != 0 {
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
/// At the moment, we only service the Channel 0 interrupt,
/// which indicates that the serial transmission is complete.
fn handle_dmac() {
    let dmac = unsafe { &*DMAC::PTR };
    dmac.dmac_irq_pend0.modify(|r, w| {
        if r.dma0_queue_irq_pend().bit_is_set() {
            D1Uart::tx_done_waker().wake();
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

#[panic_handler]
fn handler(info: &PanicInfo) -> ! {
    // Ugly but works
    let mut uart: Uart = unsafe { core::mem::transmute(()) };

    write!(&mut uart, "\r\n").ok();
    write!(&mut uart, "{}\r\n", info).ok();

    loop {
        core::sync::atomic::fence(Ordering::SeqCst);
    }
}
