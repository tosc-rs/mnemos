#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use mnemos_d1_core::{
    dmac::Dmac,
    drivers::{spim::kernel_spim1, twi, uart::kernel_uart},
    plic::Plic,
    timer::Timers,
    Ram, D1,
};
use mnemos_beepy::i2c_puppet::{self, I2cPuppetClient, I2cPuppetServer};

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
    let i2c0 = unsafe { twi::I2c0::mq_pro(p.TWI0, &mut p.CCU, &mut p.GPIO) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut p.CCU);
    let plic = Plic::new(p.PLIC);

    let d1 = D1::initialize(timers, uart, spim, dmac, plic, i2c0).unwrap();

    p.GPIO.pd_cfg2.modify(|_r, w| {
        w.pd18_select().output();
        w
    });
    p.GPIO.pd_dat.modify(|_r, w| {
        w.pd_dat().variant(1 << 18);
        w
    });

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

    // d1.initialize_sharp_display();
    let i2c_puppet_up = d1.kernel.initialize(async move {
        d1.kernel.sleep(Duration::from_secs(2)).await;
        I2cPuppetServer::register(d1.kernel, Default::default()).await.expect("failed to register i2c_puppet driver!");
    }).unwrap();

    d1.kernel.initialize({
        let k = d1.kernel;
        async move {
            use i2c_puppet::I2cPuppetClient;
            i2c_puppet_up.await.unwrap();
            let mut i2c_puppet = I2cPuppetClient::from_registry(k).await;
            tracing::info!("got i2c puppet client");
            let mut keys = i2c_puppet.subscribe_to_keys().await.expect("can't get keys");
            tracing::info!("got key subscription");
            while let Ok((status, key)) = keys.next_raw().await {
                tracing::info!(?status, ?key, "got keypress");
            }
        }
    }).unwrap();

    d1.run()
}
