use crate::plic::{Plic, Priority};
use core::time::Duration;
use d1_config::{I2cPuppetConfiguration, InterruptPin};
use d1_pac::Interrupt;
use kernel::{
    maitake::{sync::WaitCell, task::JoinHandle},
    Kernel,
};
use mnemos_beepy::i2c_puppet::{
    self, HsvColor, I2cPuppetClient, I2cPuppetServer, I2cPuppetSettings,
};

pub(crate) static I2C_PUPPET_IRQ: WaitCell = WaitCell::new();

pub(crate) fn initialize(
    config: I2cPuppetConfiguration,
    k: &'static Kernel,
    gpio: &d1_pac::GPIO,
    plic: &Plic,
) -> JoinHandle<Result<(), i2c_puppet::RegistrationError>> {
    let irq_waker = config.interrupt_pin.map(|pin| {
        match pin {
            InterruptPin::PB7 => init_i2c_puppet_irq_pb7(gpio, plic),
        }
        &I2C_PUPPET_IRQ
    });

    let up = k
        .initialize(async move {
            let settings = I2cPuppetSettings::default().with_poll_interval(config.poll_interval);
            I2cPuppetServer::register(k, settings, irq_waker).await
        })
        .unwrap();

    k.initialize(async move {
        // walk through the HSV color space. maybe eventually we'll use the RGB
        // LED to display useful information, but this is fun for now.
        let mut hue = 0;

        let mut i2c_puppet = I2cPuppetClient::from_registry(k)
            .await
            .expect("no i2c_puppet service running!");

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
            k.sleep(Duration::from_millis(50)).await;
        }
    })
    .unwrap();

    up
}

fn init_i2c_puppet_irq_pb7(gpio: &d1_pac::GPIO, plic: &crate::Plic) {
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
}
