#![no_std]
#![no_main]

use core::cell::UnsafeCell;
use core::ptr::NonNull;
use core::sync::atomic::{compiler_fence, Ordering, fence};

use kernel;

use d1_pac::{Interrupt, TIMER, UART0};
use d1_playground::dmac::descriptor::{
    AddressMode, BModeSel, BlockSize, DataWidth, DescriptorConfig, DestDrqType, SrcDrqType, Descriptor,
};
use d1_playground::dmac::{Dmac, ChannelMode};
use panic_halt as _;

use d1_playground::plic::{Plic, Priority};
use d1_playground::timer::{Timer, TimerMode, TimerPrescaler, TimerSource, Timers};

static HOUND: &str = include_str!("../hound.txt");

struct Uart(d1_pac::UART0);
static mut PRINTER: Option<Uart> = None;
impl core::fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        while self.0.usr.read().tfnf().bit_is_clear() {}
        for byte in s.as_bytes() {
            self.0.thr().write(|w| unsafe { w.thr().bits(*byte) });
            while self.0.usr.read().tfnf().bit_is_clear() {}
        }
        Ok(())
    }
}
fn print_raw(data: &[u8]) {
    let uart = unsafe { PRINTER.as_mut().unwrap() };
    while uart.0.usr.read().tfnf().bit_is_clear() {}
    for byte in data {
        uart.0.thr().write(|w| unsafe { w.thr().bits(*byte) });
        while uart.0.usr.read().tfnf().bit_is_clear() {}
    }
}
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    unsafe {
        PRINTER.as_mut().unwrap().write_fmt(args).ok();
    }
}
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::_print(core::format_args!($($arg)*));
    }
}
#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {
        $crate::_print(core::format_args!($($arg)*));
        $crate::print!("\r\n");
    }
}

#[riscv_rt::entry]
fn main() -> ! {
    let mut p = d1_pac::Peripherals::take().unwrap();

    // Enable UART0 clock.
    let ccu = &mut p.CCU;
    ccu.uart_bgr
        .write(|w| w.uart0_gating().pass().uart0_rst().deassert());

    // DMAC enable
    let mut dmac = Dmac::new(p.DMAC, ccu);

    // Set PC1 LED to output.
    let gpio = &p.GPIO;
    gpio.pc_cfg0
        .write(|w| w.pc1_select().output().pc0_select().ledc_do());

    // Set PB8 and PB9 to function 6, UART0, internal pullup.
    gpio.pb_cfg1
        .write(|w| w.pb8_select().uart0_tx().pb9_select().uart0_rx());
    gpio.pb_pull0
        .write(|w| w.pc8_pull().pull_up().pc9_pull().pull_up());

    // Configure UART0 for 115200 8n1.
    // By default APB1 is 24MHz, use divisor 13 for 115200.
    let uart0 = p.UART0;

    // UART Mode
    // No Auto Flow Control
    // No Loop Back
    // No RTS_N
    // No DTR_N
    uart0.mcr.write(|w| unsafe { w.bits(0) });

    // RCVR INT Trigger: 1 char in FIFO
    // TXMT INT Trigger: FIFO Empty
    // DMA Mode 0 - (???)
    // FIFOs Enabled
    // uart0.hsk.write(|w| w.hsk().handshake());
    // uart0.dma_req_en.modify(|_r, w| w.timeout_enable().set_bit());
    // uart0.fcr().write(|w| w.fifoe().set_bit().dmam().mode_1());
    uart0.fcr().write(|w| {
        w.fifoe().set_bit();
        w.rt().half_full();
        w
    });
    uart0.ier().write(|w| {
        w.erbfi().set_bit();
        w
    });

    // TX Halted
    // Also has some DMA relevant things? Not set currently
    uart0.halt.write(|w| w.halt_tx().enabled());

    // Enable control of baudrates
    uart0.lcr.write(|w| w.dlab().divisor_latch());

    // Baudrates
    uart0.dll().write(|w| unsafe { w.dll().bits(13) });
    uart0.dlh().write(|w| unsafe { w.dlh().bits(0) });

    // Unlatch baud rate, set width
    uart0.lcr.write(|w| w.dlab().rx_buffer().dls().eight());

    // Re-enable sending
    uart0.halt.write(|w| w.halt_tx().disabled());

    unsafe { PRINTER = Some(Uart(uart0)) };

    // Set up timers
    let Timers {
        mut timer0,
        mut timer1,
        ..
    } = Timers::new(p.TIMER);

    timer0.set_source(TimerSource::OSC24_M);
    timer1.set_source(TimerSource::OSC24_M);

    timer0.set_prescaler(TimerPrescaler::P8); // 24M / 8:  3.00M ticks/s
    timer1.set_prescaler(TimerPrescaler::P32); // 24M / 32: 0.75M ticks/s

    timer0.set_mode(TimerMode::SINGLE_COUNTING);
    timer1.set_mode(TimerMode::SINGLE_COUNTING);

    let _ = timer0.get_and_clear_interrupt();
    let _ = timer1.get_and_clear_interrupt();

    unsafe {
        riscv::interrupt::enable();
        riscv::register::mie::set_mext();
    }

    // Set up interrupts
    timer0.set_interrupt_en(true);
    timer1.set_interrupt_en(true);
    let plic = Plic::new(p.PLIC);

    unsafe {
        plic.set_priority(Interrupt::UART0, Priority::P1);
        plic.set_priority(Interrupt::TIMER0, Priority::P1);
        plic.set_priority(Interrupt::TIMER1, Priority::P1);
        plic.unmask(Interrupt::UART0);
        plic.unmask(Interrupt::TIMER0);
        plic.unmask(Interrupt::TIMER1);
    }

    let thr_addr = unsafe { &*UART0::PTR }.thr() as *const _ as *mut ();

    for chunk in HOUND.lines() {
        let d_cfg = DescriptorConfig {
            source: chunk.as_ptr().cast(),
            destination: thr_addr,
            byte_counter: chunk.len(),
            link: None,
            wait_clock_cycles: 0,
            bmode: BModeSel::Normal,
            dest_width: DataWidth::Bit8,
            dest_addr_mode: AddressMode::IoMode,
            dest_block_size: BlockSize::Byte1,
            dest_drq_type: DestDrqType::Uart0Tx,
            src_data_width: DataWidth::Bit8,
            src_addr_mode: AddressMode::LinearMode,
            src_block_size: BlockSize::Byte1,
            src_drq_type: SrcDrqType::Dram,
        };
        let descriptor = d_cfg.try_into().unwrap();
        unsafe {
            dmac.channels[0].set_channel_modes(ChannelMode::Wait, ChannelMode::Handshake);
            dmac.channels[0].start_descriptor(NonNull::from(&descriptor));
        }

        timer0.start_counter(1_500_000);
        unsafe { riscv::asm::wfi() };

        println!("");

        unsafe {
            dmac.channels[0].stop_dma();
        }
    }
    panic!();
}

#[export_name = "MachineExternal"]
fn im_an_interrupt() {
    let plic = unsafe { Plic::summon() };
    let timer = unsafe { &*TIMER::PTR };
    let uart0 = unsafe { &*UART0::PTR };

    let claim = plic.claim();
    // println!("claim: {}", claim.bits());

    match claim {
        Interrupt::TIMER0 => {
            timer
                .tmr_irq_sta
                .modify(|_r, w| w.tmr0_irq_pend().set_bit());
            // Wait for the interrupt to clear to avoid repeat interrupts
            while timer.tmr_irq_sta.read().tmr0_irq_pend().bit_is_set() {}
        }
        Interrupt::TIMER1 => {
            timer
                .tmr_irq_sta
                .modify(|_r, w| w.tmr1_irq_pend().set_bit());
            // Wait for the interrupt to clear to avoid repeat interrupts
            while timer.tmr_irq_sta.read().tmr1_irq_pend().bit_is_set() {}
        }
        Interrupt::UART0 => {
            println!("");
            println!("UART SAYS: ");
            while uart0.usr.read().rfne().bit_is_set() {
                let byte = uart0.rbr().read().rbr().bits();
                uart0.thr().write(|w| unsafe { w.thr().bits(byte) });
                while uart0.usr.read().tfnf().bit_is_clear() {}
            }
            println!("");
        }
        x => {
            println!("Unexpected claim: {:?}", x);
            panic!();
        }
    }

    // Release claim
    plic.complete(claim);
}

// Main config register:
// DMAC_CFG_REGN
// Mode:
// DMAC_MODE_REGN
