#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use kernel::maitake::sync::WaitCell;
use mnemos_beepy::i2c_puppet::{HsvColor, I2cPuppetClient, I2cPuppetServer, I2cPuppetSettings};
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
    let i2c0 = unsafe { twi::I2c0::mq_pro(p.TWI0, &mut ccu, &mut p.GPIO) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut ccu);
    let mut plic = Plic::new(p.PLIC);

    let i2c_puppet_irq = init_i2c_puppet_irq(&mut p.GPIO, &mut plic);

    let d1 = D1::initialize(timers, uart, spim, dmac, plic, i2c0);

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

    d1.initialize_sharp_display();

    let i2c_puppet_up = d1
        .kernel
        .initialize(async move {
            let settings = I2cPuppetSettings::default().with_poll_interval(Duration::from_secs(2));
            d1.kernel.sleep(Duration::from_secs(2)).await;
            I2cPuppetServer::register(d1.kernel, settings, i2c_puppet_irq)
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

// Note: pass by ref mut to enforce exclusive access
#[allow(clippy::needless_pass_by_ref_mut)]
fn init_i2c_puppet_irq(gpio: &mut d1_pac::GPIO, plic: &mut Plic) -> &'static WaitCell {
    use d1_pac::Interrupt;
    use mnemos_d1_core::plic::Priority;

    static I2C_PUPPET_IRQ: WaitCell = WaitCell::new();

    // configure pin mappings:

    // the i2c_puppet PI_INT line is on the GCLK0/GPIO4 pin on the Pi header,
    // according to this schematic:
    // https://github.com/sqfmi/beepy-hardware/blob/d051e65fd95fdadd83154378950b171a001125a8/KiCad/beepberry-schematic-v1.pdf
    //
    // according to the MangoPi MQ Pro schematic, that pin is routed to PB7 on
    // the D1: https://mangopi.org/_media/mq-pro-sch-v12.pdf
    //
    // we don't need to enable internal pullups, as the Beepy schematic
    // indicates that the i2c_puppet board has a 10k pullup on the PI_INT line.
    gpio.pb_cfg0.modify(|_r, w| {
        // set PB7 to interrupt mode.
        w.pb7_select().pb_eint7()
    });
    // i2c_puppet triggers an IRQ by asserting the IRQ line low, according
    // to https://github.com/solderparty/i2c_puppet#protocol
    gpio.pb_eint_cfg0.modify(|_r, w| {
        // set PB7 interrupts to negative edge triggered.
        w.eint7_cfg().negative_edge()
    });

    gpio.pb_eint_ctl.modify(|_r, w| {
        // enable PB7 interrupts.
        w.eint7_ctl().enable()
    });

    // configure ISR
    unsafe {
        plic.register(Interrupt::GPIOB_NS, handle_pb_eint_irq);
        plic.activate(Interrupt::GPIOB_NS, Priority::P1)
            .expect("could not activate GPIOB_NS ISR");
    }

    // XXX(eliza): right now, this will *only* handle the i2c_puppet IRQ. this isn't
    // going to scale well if we want to be able to handle other IRQs on PB7 pins,
    // especially if we want those to be defined in cross-platform code...
    fn handle_pb_eint_irq() {
        let gpio = { unsafe { &*d1_pac::GPIO::ptr() } };
        gpio.pb_eint_status.modify(|r, w| {
            if r.eint7_status().is_pending() {
                // wake the i2c_puppet waker.
                I2C_PUPPET_IRQ.wake();
            } else {
                unreachable!("no other PB EINTs should be enabled, what the heck!")
            }

            // writing back the interrupt bit clears it.
            w
        });

        // wait for the bit to clear to avoid spurious IRQs
        while gpio.pb_eint_status.read().eint7_status().is_pending() {}
    }

    &I2C_PUPPET_IRQ
}
