#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use mnemos_d1_core::{
    ccu::Ccu,
    dmac::Dmac,
    drivers::{spim::kernel_spim1, twi, uart::kernel_uart},
    plic::Plic,
    timer::Timers,
    Ram, D1,
};

const HEAP_SIZE: usize = 384 * 1024 * 1024;

#[link_section = ".aheap.AHEAP"]
#[used]
static AHEAP_BUF: Ram<HEAP_SIZE> = Ram::new();

#[allow(non_snake_case)]
#[riscv_rt::entry]
fn main() -> ! {
    unsafe {
        mnemos_d1::initialize_heap(&AHEAP_BUF);
    }

    let mut p = unsafe { d1_pac::Peripherals::steal() };

    let mut ccu = Ccu::new(p.CCU);
    ccu.sys_clock_init();

    let uart = unsafe { kernel_uart(&mut ccu, &mut p.GPIO, p.UART0) };
    let spim = unsafe { kernel_spim1(p.SPI_DBI, &mut ccu, &mut p.GPIO) };
    let i2c0 = unsafe { twi::I2c0::lichee_rv_dock(p.TWI2, &mut ccu, &mut p.GPIO) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut ccu);
    let plic = Plic::new(p.PLIC);

    let d1 = D1::initialize(timers, uart, spim, dmac, plic, i2c0);

    p.GPIO.pc_cfg0.modify(|_r, w| {
        w.pc1_select().output();
        w
    });
    p.GPIO.pc_dat.modify(|_r, w| {
        w.pc_dat().variant(0b0000_0010);
        w
    });

    d1.initialize_sharp_display();

    // Initialize LED loop
    d1.kernel
        .initialize(async move {
            loop {
                p.GPIO.pc_dat.modify(|_r, w| {
                    w.pc_dat().variant(0b0000_0010);
                    w
                });
                d1.kernel.sleep(Duration::from_millis(250)).await;
                p.GPIO.pc_dat.modify(|_r, w| {
                    w.pc_dat().variant(0b0000_0000);
                    w
                });
                d1.kernel.sleep(Duration::from_millis(250)).await;
            }
        })
        .unwrap();

    d1.run()
}
