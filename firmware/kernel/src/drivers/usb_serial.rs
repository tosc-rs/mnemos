use bbqueue::{BBBuffer, Consumer, Producer};
use nrf52840_hal::usbd::{Usbd, UsbPeripheral};
use usb_device::device::UsbDevice;
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
}

pub struct UsbUartSys {
    out: Producer<'static, USB_BUF_SZ>,
    inc: Consumer<'static, USB_BUF_SZ>,
}

pub struct UsbUartParts {
    isr: UsbUartIsr,
    sys: UsbUartSys,
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
        },
        sys: UsbUartSys {
            out: out_prod,
            inc: inc_cons,
        }
    })
}

impl crate::traits::Serial for UsbUartSys {
    fn recv<'a>(&mut self, buf: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        todo!()
    }

    fn send<'a>(&mut self, buf: &'a [u8]) -> Result<(), &'a [u8]> {
        todo!()
    }
}
