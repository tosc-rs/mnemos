#![no_std]
#![no_main]

extern crate alloc;

use core::{time::Duration, panic::PanicInfo};
use mnemos_d1_core::{
    drivers::{
        self,
        uart::{kernel_uart},
        spim::{kernel_spim1},
    },
    dmac::Dmac,
    timer::Timers,
    plic::Plic,
    Ram,
    D1,
};

const HEAP_SIZE: usize = 384 * 1024 * 1024;

#[link_section = ".aheap.AHEAP"]
#[used]
static AHEAP_BUF: Ram<HEAP_SIZE> = Ram::new();

#[allow(non_snake_case)]
#[riscv_rt::entry]
fn main() -> ! {
    unsafe { crate::initialize_heap(&AHEAP_BUF); }

    let mut p = unsafe { d1_pac::Peripherals::steal() };
    let uart = unsafe { kernel_uart(&mut p.CCU, &mut p.GPIO, p.UART0) };
    let spim = unsafe { kernel_spim1(p.SPI_DBI, &mut p.CCU, &mut p.GPIO) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut p.CCU);
    let plic = Plic::new(p.PLIC);

    let d1 = D1::initialize(&AHEAP_BUF, timers, uart, spim, dmac, plic).unwrap();

    p.GPIO.pc_cfg0.modify(|_r, w| {
        w.pc1_select().output();
        w
    });
    p.GPIO.pc_dat.modify(|_r, w| {
        w.pc_dat().variant(0b0000_0010);
        w
    });

    d1.kernel.initialize(drivers::sharp_display::sharp_memory_display(d1.kernel)).unwrap();

    // Initialize LED loop
    d1.kernel.initialize(async move {
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