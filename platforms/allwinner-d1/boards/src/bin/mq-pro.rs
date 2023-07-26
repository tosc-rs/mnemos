#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use mnemos_beepy::i2c_puppet::{HsvColor, I2cPuppetClient, I2cPuppetServer};
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
    let i2c0 = unsafe { twi::I2c0::mq_pro(p.TWI0, &mut p.CCU) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut p.CCU);
    let plic = Plic::new(p.PLIC);
    
    let d1 = D1::initialize(timers, uart, spim, dmac, plic, i2c0, p.GPIO);

    // Initialize LED loop
    d1.kernel
        .initialize(async move {
            let mut pin = {
                let mut gpio = gpio::GpioClient::from_registry(d1.kernel).await;
                gpio.claim_output(gpio::PinD::D18).await.expect("can't claim D18 as output!")
            };
            loop {
                pin.set(true);
                d1.kernel.sleep(Duration::from_millis(250)).await;
                pin.set(false);
                d1.kernel.sleep(Duration::from_millis(250)).await;
            }
        })
        .unwrap();

    d1.initialize_sharp_display();

    let i2c_puppet_up = d1
        .kernel
        .initialize(async move {
            d1.kernel.sleep(Duration::from_secs(2)).await;
            I2cPuppetServer::register(d1.kernel, Default::default())
                .await
                .expect("failed to register i2c_puppet driver!");
        })
        .unwrap();

    d1.kernel
        .initialize(async move {
            // i2c_puppet demo: print each keypress to the console.
            i2c_puppet_up.await.unwrap();
            let mut i2c_puppet = I2cPuppetClient::from_registry(d1.kernel).await;
            tracing::info!("got i2c puppet client");

            let mut keys = i2c_puppet
                .subscribe_to_raw_keys()
                .await
                .expect("can't get keys");
            tracing::info!("got key subscription");
            while let Ok(key) = keys.next_char().await {
                tracing::info!(?key, "got keypress");
            }
        })
        .unwrap();

    d1.kernel
        .initialize(async move {
            // walk through the HSV color space. maybe eventually we'll use the RGB
            // LED to display useful information, but this is fun for now.
            let mut hue = 0;

            let mut i2c_puppet = I2cPuppetClient::from_registry(d1.kernel).await;

            i2c_puppet
                .toggle_led(true)
                .await
                .expect("can't turn on LED");
            loop {
                i2c_puppet
                    .set_led_color(HsvColor::from_hue(hue))
                    .await
                    .expect("can't set color");
                hue = hue.wrapping_add(1);
                d1.kernel.sleep(Duration::from_millis(50)).await;
            }
        })
        .unwrap();

    d1.run()
}
