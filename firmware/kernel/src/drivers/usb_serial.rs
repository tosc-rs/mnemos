use bbqueue::{BBBuffer, Consumer, Producer};
use nrf52840_hal::usbd::{Usbd, UsbPeripheral};
use usb_device::{device::UsbDevice, UsbError};
use usbd_serial::SerialPort;

const USB_BUF_SZ: usize = 4096;
static UART_INC: BBBuffer<USB_BUF_SZ> = BBBuffer::new();
static UART_OUT: BBBuffer<USB_BUF_SZ> = BBBuffer::new();

type AUsbPeripheral = Usbd<UsbPeripheral<'static>>;
type AUsbDevice = UsbDevice<'static, AUsbPeripheral>;
type ASerialPort = SerialPort<'static, AUsbPeripheral>;

pub struct UsbUartIsr {
    dev: AUsbDevice,
    ser: ASerialPort,
    out: Consumer<'static, USB_BUF_SZ>,
    inc: Producer<'static, USB_BUF_SZ>,
    ctr: u32,
}

impl UsbUartIsr {
    pub fn poll(&mut self) {
        self.ctr += 1;

        if self.ctr >= 1000 {
            defmt::println!("USB TICK");
            self.ctr = 0;
        }

        self.dev.poll(&mut [&mut self.ser]);

        if let Ok(rgr) = self.out.read() {
            match self.ser.write(&rgr) {
                Ok(sz) if sz > 0 => {
                    defmt::println!("USB Wrote {=usize}", sz);
                    rgr.release(sz);
                },
                Ok(_) | Err(UsbError::WouldBlock) => {
                    // Just silently drop the read grant
                }
                Err(_) => defmt::panic!("Usb Error Write!"),
            }
        }
        if let Ok(mut wgr) = self.inc.grant_max_remaining(128) {
            match self.ser.read(&mut wgr) {
                Ok(sz) if sz > 0 => {
                    defmt::println!("USB Read {=usize}", sz);
                    wgr.commit(sz);
                },
                Ok(_) | Err(UsbError::WouldBlock) => {
                    // Just silently drop the write grant
                }
                Err(_) => defmt::panic!("Usb Error Read!"),
            }
        }
    }
}

pub struct UsbUartSys {
    out: Producer<'static, USB_BUF_SZ>,
    inc: Consumer<'static, USB_BUF_SZ>,
}

pub struct UsbUartParts {
    pub isr: UsbUartIsr,
    pub sys: UsbUartSys,
}

pub fn setup_usb_uart(dev: AUsbDevice, ser: ASerialPort) -> Result<UsbUartParts, ()> {
    let (inc_prod, inc_cons) = UART_INC.try_split().map_err(drop)?;
    let (out_prod, out_cons) = UART_OUT.try_split().map_err(drop)?;

    Ok(UsbUartParts {
        isr: UsbUartIsr {
            dev,
            ser,
            out: out_cons,
            inc: inc_prod,
            ctr: 0,
        },
        sys: UsbUartSys {
            out: out_prod,
            inc: inc_cons,
        }
    })
}

impl crate::traits::Serial for UsbUartSys {
    fn recv<'a>(&mut self, buf: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        match self.inc.split_read() {
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
                defmt::println!("RELEASE {=usize}", used);
                sgr.release(used);
                Ok(&mut buf[..used])
            }
            Err(bbqueue::Error::InsufficientSize) => {
                Ok(&mut [])
            }
            Err(_e) => {
                defmt::panic!("ERROR: USB UART Recv!");
            }
        }
    }

    fn send<'a>(&mut self, buf: &'a [u8]) -> Result<(), &'a [u8]> {
        let mut remaining = buf;

        while !remaining.is_empty() {
            let rem_len = remaining.len();

            match self.out.grant_max_remaining(rem_len) {
                Ok(mut wgr) => {
                    let to_use = wgr.len().min(rem_len);
                    let (now, later) = remaining.split_at(to_use);
                    wgr[..to_use].copy_from_slice(&now[..to_use]);
                    defmt::println!("COMMIT {=usize}", to_use);
                    wgr.commit(to_use);
                    remaining = later;
                },
                Err(bbqueue::Error::InsufficientSize) => {
                    return Err(remaining);
                },
                Err(_e) => {
                    defmt::panic!("ERROR: USB UART Send!");
                }
            }
        }

        Ok(())
    }
}
