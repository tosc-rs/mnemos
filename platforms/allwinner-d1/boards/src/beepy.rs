//! Beepy (nee Beepberry) board support.
use core::time::Duration;
use kernel::{
    services::i2c::{I2cClient, I2cError},
    trace, Kernel,
};

/// Initialize all Beepy-specific peripherals.
pub fn initialize(d1: &mnemos_d1_core::D1) {
    d1.initialize_sharp_display();
    d1.kernel
        .initialize({
            let k = d1.kernel;
            async move {
                k.sleep(Duration::from_secs(2)).await;
                trace::info!("starting i2c_puppet test...");
                if let Err(error) = i2c_puppet(k).await {
                    trace::error!(?error, "i2c_puppet test failed");
                }
            }
        })
        .unwrap();
}

/// A rudimentary driver for Beepy's [i2c_puppet].
///
/// [i2c_puppet]: https://beepy.sqfmi.com/docs/firmware/keyboard
// TODO(eliza): turn this into a service!
#[trace::instrument(skip(k))]
pub async fn i2c_puppet(k: &'static Kernel) -> Result<(), I2cError> {
    use kernel::embedded_hal_async::i2c::{self, I2c};

    // https://github.com/solderparty/i2c_puppet#protocol
    const ADDR: u8 = 0x1f;
    // to write with a register, we must OR the register number with this mask:
    // https://github.com/solderparty/i2c_puppet#protocol
    const WRITE_MASK: u8 = 0x80;

    // firmware version register:
    // https://github.com/solderparty/i2c_puppet#the-fw-version-register-reg_ver--0x01
    const REG_VER: u8 = 0x01;

    // RGB LED configuration registers:
    // https://beepy.sqfmi.com/docs/firmware/rgb-led#set-rgb-color
    const REG_LED_ON: u8 = 0x20;
    const REG_LED_R: u8 = 0x21;
    const REG_LED_G: u8 = 0x22;
    const REG_LED_B: u8 = 0x23;

    let mut i2c = I2cClient::from_registry(k).await;

    trace::info!("reading i2c_puppet version...");
    let mut rdbuf = [0u8; 1];
    match i2c
        .transaction(
            ADDR,
            &mut [
                i2c::Operation::Write(&[REG_VER]),
                i2c::Operation::Read(&mut rdbuf[..]),
            ],
        )
        .await
    {
        Ok(_) => {
            let val = rdbuf[0];
            let major = (val & 0xf0) >> 4;
            let minor = val & 0x0f;
            trace::info!("i2c_puppet firmware version: v{major}.{minor}");
        }
        Err(error) => {
            trace::error!(%error, "error reading i2c_puppet version");
            return Err(error);
        }
    }

    trace::info!("setting i2c_puppet RGB LED to green...");
    match i2c
        .transaction(
            ADDR,
            &mut [
                // set red to 0
                i2c::Operation::Write(&[REG_LED_R | WRITE_MASK, 0]),
                // set green to 255
                i2c::Operation::Write(&[REG_LED_G | WRITE_MASK, 255]),
                // set blue to 0
                i2c::Operation::Write(&[REG_LED_B | WRITE_MASK, 0]),
                // turn on the LED
                i2c::Operation::Write(&[REG_LED_ON | WRITE_MASK, 255]),
            ],
        )
        .await
    {
        Ok(_) => trace::info!("i2c_puppet LED should now be green!"),
        Err(error) => {
            trace::error!(%error, "error writing to i2c_puppet LED");
            return Err(error);
        }
    };

    // TODO(eliza): keyboard driver (using https://crates.io/crates/bbq10kbd)
    Ok(())
}
