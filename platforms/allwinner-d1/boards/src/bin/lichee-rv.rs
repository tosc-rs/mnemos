#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use mnemos_d1_core::{
    dmac::Dmac,
    drivers::{spim::kernel_spim1, twi, uart::kernel_uart, gpio},
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
    let uart = unsafe { kernel_uart(&mut p.CCU, p.UART0) };
    let spim = unsafe { kernel_spim1(p.SPI_DBI, &mut p.CCU) };
    let i2c0 = unsafe { twi::I2c0::lichee_rv_dock(p.TWI2, &mut p.CCU) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut p.CCU);
    let plic = Plic::new(p.PLIC);

    let d1 = D1::initialize(timers, uart, spim, dmac, plic, i2c0, p.GPIO);

    d1.initialize_sharp_display();

    // Initialize LED loop
    d1.kernel
        .initialize(async move {
            let mut pin = {
                let mut gpio = gpio::GpioClient::from_registry(d1.kernel).await;
                gpio.claim_output(gpio::PinC::C1).await.expect("can't claim C1 as output!")
            };
            loop {
                pin.set(true);
                d1.kernel.sleep(Duration::from_millis(250)).await;
                pin.set(false);
                d1.kernel.sleep(Duration::from_millis(250)).await;
            }
        })
        .unwrap();


    d1.run()
}
