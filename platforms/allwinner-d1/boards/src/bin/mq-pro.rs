#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use mnemos_d1_core::{
    dmac::Dmac,
    drivers::{sharp_display::SharpDisplay, spim::kernel_spim1, twi, uart::kernel_uart},
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
    let uart = unsafe { kernel_uart(&mut p.CCU, &mut p.GPIO, p.UART0) };
    let spim = unsafe { kernel_spim1(p.SPI_DBI, &mut p.CCU, &mut p.GPIO) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut p.CCU);
    let plic = Plic::new(p.PLIC);
    let twi0 = unsafe { twi::Twi0Engine::mq_pro(p.TWI0, &mut p.CCU, &mut p.GPIO) };

    let d1 = D1::initialize(timers, uart, spim, dmac, plic, twi0).unwrap();

    p.GPIO.pd_cfg2.modify(|_r, w| {
        w.pd18_select().output();
        w
    });
    p.GPIO.pd_dat.modify(|_r, w| {
        w.pd_dat().variant(1 << 18);
        w
    });

    // d1.kernel
    //     .initialize(SharpDisplay::register(d1.kernel, 4))
    //     .unwrap();

    // Initialize LED loop
    d1.kernel
        .initialize(async move {
            loop {
                p.GPIO.pd_dat.modify(|_r, w| {
                    w.pd_dat().variant(1 << 18);
                    w
                });
                d1.kernel.sleep(Duration::from_millis(250)).await;
                p.GPIO.pd_dat.modify(|_r, w| {
                    w.pd_dat().variant(0);
                    w
                });
                d1.kernel.sleep(Duration::from_millis(250)).await;
            }
        })
        .unwrap();

    d1.run()
}
