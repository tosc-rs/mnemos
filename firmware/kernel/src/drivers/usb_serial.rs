//! A USB-Serial driver for the nRF52840

use bbqueue::{BBBuffer, Consumer, Producer};
use nrf52840_hal::usbd::{Usbd, UsbPeripheral};
use usb_device::{device::UsbDevice, UsbError};
use usbd_serial::SerialPort;

const USB_BUF_SZ: usize = 4096;
static UART_INC: BBBuffer<USB_BUF_SZ> = BBBuffer::new();
static UART_OUT: BBBuffer<USB_BUF_SZ> = BBBuffer::new();

/// A type alias for the nRF52840 USB Peripheral type
pub type AUsbPeripheral = Usbd<UsbPeripheral<'static>>;

/// A type alias for the nRF52840 USB Device type
pub type AUsbDevice = UsbDevice<'static, AUsbPeripheral>;

/// A type alias for the nRF52840 CDC-ACM USB Serial port type
pub type ASerialPort = SerialPort<'static, AUsbPeripheral>;

/// The handle necessary for servicing USB interrupts
pub struct UsbUartIsr {
    dev: AUsbDevice,
    ser: ASerialPort,
    out: Consumer<'static, USB_BUF_SZ>,
    inc: Producer<'static, USB_BUF_SZ>,
}

impl UsbUartIsr {
    /// Service the USB ISR, which is triggered by either a regular polling timer,
    /// or some kind of USB interrupt.
    pub fn poll(&mut self) {
        // Service the relevant hardware logic
        self.dev.poll(&mut [&mut self.ser]);

        // If there is data to be sent...
        if let Ok(rgr) = self.out.read() {
            match self.ser.write(&rgr) {
                // ... and there is room to send it, then send it.
                Ok(sz) if sz > 0 => {
                    rgr.release(sz);
                },
                // ... and there is no room to send it, then just bail.
                Ok(_) | Err(UsbError::WouldBlock) => {
                    // Just silently drop the read grant
                }
                // ... and there is a USB error, then panic.
                Err(_) => defmt::panic!("Usb Error Write!"),
            }
        }

        // If there is room to receive data...
        if let Ok(mut wgr) = self.inc.grant_max_remaining(128) {
            match self.ser.read(&mut wgr) {
                // ... and there is data to be read, then take it.
                Ok(sz) if sz > 0 => {
                    wgr.commit(sz);
                },
                // ... and there is no data to be read, then just bail.
                Ok(_) | Err(UsbError::WouldBlock) => {
                    // Just silently drop the write grant
                }
                // ... and there is a USB error, then panic.
                Err(_) => defmt::panic!("Usb Error Read!"),
            }
        }
    }
}

/// The "userspace" handle for the driver
pub struct UsbUartSys {
    out: Producer<'static, USB_BUF_SZ>,
    inc: Consumer<'static, USB_BUF_SZ>,
}

/// A struct containing both the "interrupt" and "userspace" handles
/// for this USB-Serial driver
pub struct UsbUartParts {
    pub isr: UsbUartIsr,
    pub sys: UsbUartSys,
}

/// Obtain the "userspace" and "interrupt" portions of the USB-Serial driver
///
/// This only returns `Ok` once, as this driver is a singleton. Subsequent
/// calls will return an `Err`.
pub fn setup_usb_uart(dev: AUsbDevice, ser: ASerialPort) -> Result<UsbUartParts, ()> {
    let (inc_prod, inc_cons) = UART_INC.try_split().map_err(drop)?;
    let (out_prod, out_cons) = UART_OUT.try_split().map_err(drop)?;

    Ok(UsbUartParts {
        isr: UsbUartIsr {
            dev,
            ser,
            out: out_cons,
            inc: inc_prod,
        },
        sys: UsbUartSys {
            out: out_prod,
            inc: inc_cons,
        }
    })
}

// Implement the "userspace" traits for the USB UART
impl crate::traits::Serial for UsbUartSys {
    fn recv<'a>(&mut self, buf: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        // Use a split read to get ALL available data, even if
        // there has been a wraparound.
        match self.inc.split_read() {
            // There is some data available to read
            Ok(sgr) => {
                // Get the full amount available
                let (buf_a, buf_b) = sgr.bufs();
                let buflen = buf.len();

                // How much of buffer A should we use? Cap to the smaller buffer
                // size, and copy that to the destination
                let use_a = buflen.min(buf_a.len());
                buf[..use_a].copy_from_slice(&buf_a[..use_a]);

                // Is there still space remaining in the outgoing buffer, and if so,
                // is buffer B empty?
                let used = if (use_a < buflen) && !buf_b.is_empty() {
                    // Still room and contents in buffer B! Repeat the process,
                    // appending the contents of buffer B to the remaining space
                    // in the output buffer. Again, limit this to the shortest
                    // length available
                    let use_b = (buflen - use_a).min(buf_b.len());
                    buf[use_a..][..use_b].copy_from_slice(&buf_b[..use_b]);

                    // We used all of A and some/all of B
                    use_a + use_b
                } else {
                    // We only used some/all of A, and none of B
                    use_a
                };

                // Release the used portion of the buffers, and return the
                // relevant slice
                sgr.release(used);
                Ok(&mut buf[..used])
            }

            // This indicates that NO data is ready to be read from the
            // queue. Indicate this to the user by returning an empty slice
            Err(bbqueue::Error::InsufficientSize) => {
                Ok(&mut [])
            }

            // This error case generally represents some kind of logic error
            // such as retaining a grant (our problem), or an internal fault
            // of bbqueue. Either way, this is not likely to be a recoverable
            // error. Until we have better fault recovery logic in place,
            // just panic and get it over with.
            Err(_e) => {
                defmt::panic!("ERROR: USB UART Recv!");
            }
        }
    }

    fn send<'a>(&mut self, buf: &'a [u8]) -> Result<(), &'a [u8]> {
        let mut remaining = buf;

        // We loop here, as the bbqueue may be in a "wraparound" situation,
        // where there is only a little space available at the "tail" of the
        // ring buffer, but there is space available at the front. This will
        // generally only execute once (no wraparound) or twice (some wraparound),
        // unless the driver clears some more space while we are processing.
        while !remaining.is_empty() {
            let rem_len = remaining.len();

            // Attempt to get a write grant to send to the driver...
            match self.out.grant_max_remaining(rem_len) {
                // We got some (or all) necessary space.
                // Copy the relevant data, and slide the window over.
                // (If this was "all", then `remaining` will be empty)
                Ok(mut wgr) => {
                    let to_use = wgr.len().min(rem_len);
                    let (now, later) = remaining.split_at(to_use);
                    wgr[..to_use].copy_from_slice(&now[..to_use]);
                    wgr.commit(to_use);
                    remaining = later;
                },

                // We have exhausted the available size in the outgoing buffer.
                // Give the user the remaining, unsent part, so they can try again
                // later.
                Err(bbqueue::Error::InsufficientSize) => {
                    return Err(remaining);
                },

                // This error case generally represents some kind of logic error
                // such as retaining a grant (our problem), or an internal fault
                // of bbqueue. Either way, this is not likely to be a recoverable
                // error. Until we have better fault recovery logic in place,
                // just panic and get it over with.
                Err(_e) => {
                    defmt::panic!("ERROR: USB UART Send!");
                }
            }
        }

        // This means that we reached `remaining.is_empty()`, and all
        // data has been successfully sent.
        Ok(())
    }
}
