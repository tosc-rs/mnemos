#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use mnemos_beepy::i2c_puppet::{HsvColor, I2cPuppetClient, I2cPuppetServer, I2cPuppetSettings};
use mnemos_d1_core::{
    ccu::Ccu,
    dmac::Dmac,
    drivers::{spim::kernel_spim1, twi, uart::kernel_uart},
    gpio,
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
    let i2c0 = unsafe { twi::I2c0::mq_pro(p.TWI0, &mut ccu, &mut p.GPIO) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut ccu);
    let plic = Plic::new(p.PLIC);

    let d1 = D1::initialize(timers, uart, spim, dmac, plic, i2c0);

    p.GPIO.pd_cfg2.modify(|_r, w| {
        w.pd18_select().output();
        w
    });
    p.GPIO.pd_dat.modify(|_r, w| {
        w.pd_dat().variant(1 << 18);
        w
    });

    let i2c_puppet_up = {
        d1.kernel
            .initialize(async move {
                let pin = gpio::IrqPin::new(gpio::PinB::B7, &p.GPIO);
                let settings =
                    I2cPuppetSettings::default().with_poll_interval(Duration::from_secs(2));
                d1.kernel.sleep(Duration::from_secs(2)).await;
                I2cPuppetServer::register_with_irq(d1.kernel, settings, pin)
                    .await
                    .expect("failed to register i2c_puppet driver!");
            })
            .unwrap()
    };

    // // Initialize LED loop
    // d1.kernel
    //     .initialize(async move {
    //         loop {
    //             p.GPIO.pd_dat.modify(|_r, w| {
    //                 w.pd_dat().variant(1 << 18);
    //                 w
    //             });
    //             d1.kernel.sleep(Duration::from_millis(250)).await;
    //             p.GPIO.pd_dat.modify(|_r, w| {
    //                 w.pd_dat().variant(0);
    //                 w
    //             });
    //             d1.kernel.sleep(Duration::from_millis(250)).await;
    //         }
    //     })
    //     .unwrap();

    d1.initialize_sharp_display();

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
