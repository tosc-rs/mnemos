use esp32c3_hal::{peripherals::USB_DEVICE, prelude::*};

use futures::FutureExt;
use kernel::{
    comms::{
        bbq::{new_bidi_channel, BidiHandle, GrantW},
        kchannel::{KChannel, KConsumer},
    },
    maitake::sync::WaitCell,
    registry::{self, Message},
    services::simple_serial::{Request, Response, SimpleSerialError, SimpleSerialService},
    Kernel,
};

pub struct UsbSerialServer {
    dev: USB_DEVICE,
}

struct GrantWriter {
    grant: GrantW,
    used: usize,
}

impl core::fmt::Write for GrantWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let glen = self.grant.len();
        let slen = s.len();
        let new_used = self.used + slen;
        if new_used <= glen {
            self.grant[self.used..][..slen].copy_from_slice(s.as_bytes());
            self.used = new_used;
            Ok(())
        } else {
            Err(core::fmt::Error)
        }
    }
}

static TX_DONE: WaitCell = WaitCell::new();
static RX_READY: WaitCell = WaitCell::new();

/// Per [the datasheet][1], the USB serial FIFO has a capacity of up to 64
/// bytes:
///
/// [1]: https://www.espressif.com/sites/default/files/documentation/esp32-c3_technical_reference_manual_en.pdf#usbserialjtag
const FIFO_CAPACITY: usize = 64;

impl UsbSerialServer {
    pub fn new(dev: USB_DEVICE) -> Self {
        dev.int_ena.modify(|_r, w| {
            w.serial_in_empty_int_ena().clear_bit();
            w.serial_out_recv_pkt_int_ena().clear_bit();
            w
        });
        dev.int_clr.write(|w| {
            w.serial_in_empty_int_clr().set_bit();
            w.serial_out_recv_pkt_int_clr().set_bit();
            w
        });
        Self { dev }
    }

    async fn serial_server(handle: BidiHandle, kcons: KConsumer<Message<SimpleSerialService>>) {
        loop {
            // esp_println::println!("get port");
            if let Ok(req) = kcons.dequeue_async().await {
                let Request::GetPort = req.msg.body;
                let resp = req.msg.reply_with(Ok(Response::PortHandle { handle }));
                let _ = req.reply.reply_konly(resp).await;
                break;
            }
        }

        // And deny all further requests after the first
        loop {
            if let Ok(req) = kcons.dequeue_async().await {
                let Request::GetPort = req.msg.body;
                let resp = req
                    .msg
                    .reply_with(Err(SimpleSerialError::AlreadyAssignedPort));
                let _ = req.reply.reply_konly(resp).await;
            }
        }
    }

    /// The same MMIO register field (`EP1[RDWR_BYTE]`) is used for both data
    /// read and data written, so ownership of that register must be assigned
    /// exclusively to this task.
    async fn worker(mut self, chan: BidiHandle) {
        // make sure crowtty is happy
        self.dev.ep1.write(|w| unsafe { w.rdwr_byte().bits(b'\0') });
        self.dev
            .int_ena
            .modify(|_, w| w.serial_in_empty_int_ena().set_bit());

        if self.dev.jfifo_st.read().out_fifo_empty().bit_is_clear() {
            self.flush().await;
        }

        let (tx, rx) = chan.split();
        // preemptively subscribe to RX interrupts.
        let mut rx_ready = RX_READY.subscribe().await;
        // enable the RX ready interrupt
        self.dev
            .int_ena
            .modify(|_, w| w.serial_out_recv_pkt_int_ena().set_bit());
        loop {
            futures::select_biased! {
                // RX bytes available!
                _ = (&mut rx_ready).fuse() => {
                    let mut wgr = tx.send_grant_max(FIFO_CAPACITY).await;
                    let mut used = 0;
                    for byte in &mut wgr[..] {
                        if self.dev.ep1_conf.read().serial_out_ep_data_avail().bit_is_clear() {
                            // we've read everything!
                            break;
                        }

                        *byte = self.dev.ep1.read().rdwr_byte().bits();
                        used += 1;
                    }

                    wgr.commit(used);

                    // re-subscribe to the interrupt
                    rx_ready = RX_READY.subscribe().await;
                },
                rgr = rx.read_grant().fuse() => {
                    // disable the RX interrupt until we're done writing
                    self.dev
                        .int_ena
                        .modify(|_, w| w.serial_out_recv_pkt_int_ena().clear_bit());

                    // we can write up to `FIFO_CAPACITY` bytes before we have
                    // to yield until the fifo has flushed, so only consume that
                    // much of the read grant.
                    let len = core::cmp::min(rgr.len(), FIFO_CAPACITY);

                    // actually write the bytes
                    for &byte in rgr.iter().take(len) {
                        self.dev.ep1.write(|w| unsafe { w.rdwr_byte().bits(byte) })
                    }

                    // release the number of bytes we wrote prior to yielding.
                    rgr.release(len);

                    // we've written 64 bytes (or less, if we're on the last 64-byte
                    // chunk of the input). Signal that we're done.
                    self.flush().await;
                },
            }

            // re-enable the RX ready interrupt --- it was either cleared
            // when we started writing, or cleared by the ISR when we
            // received an RX packet.
            self.dev
                .int_ena
                .modify(|_, w| w.serial_out_recv_pkt_int_ena().set_bit());
        }
    }

    async fn flush(&mut self) {
        // subscribe to a wakeup *before* enabling the interrupt.
        let wait = TX_DONE.subscribe().await;

        self.dev.ep1_conf.write(|w| w.wr_done().set_bit());

        if self.dev.jfifo_st.read().out_fifo_empty().bit_is_clear() {
            // already cleared!
            return;
        }

        wait.await.expect("TX_DONE waitcell should never be closed")
    }

    pub async fn register(
        self,
        k: &'static Kernel,
        cap_in: usize,
        cap_out: usize,
    ) -> Result<(), registry::RegistrationError> {
        let (kprod, kcons) = KChannel::<Message<SimpleSerialService>>::new_async(4)
            .await
            .split();
        let (fifo_a, fifo_b) = new_bidi_channel(cap_in, cap_out).await;

        k.spawn(Self::serial_server(fifo_b, kcons)).await;

        k.spawn(self.worker(fifo_a)).await;

        k.with_registry(|reg| reg.register_konly::<SimpleSerialService>(&kprod))
            .await?;

        Ok(())
    }
}

#[interrupt]
fn USB_SERIAL_JTAG() {
    let _isr = kernel::isr::Isr::enter();
    let dev = unsafe { USB_DEVICE::steal() };

    let state = dev.int_st.read();
    if state.serial_out_recv_pkt_int_st().bit_is_set() {
        dev.int_ena
            .modify(|_r, w| w.serial_out_recv_pkt_int_ena().clear_bit());
        dev.int_clr
            .write(|w| w.serial_out_recv_pkt_int_clr().set_bit());
        RX_READY.wake();
    }

    if state.serial_in_empty_int_st().bit_is_set() {
        dev.int_ena
            .modify(|_r, w| w.serial_out_recv_pkt_int_ena().clear_bit());
        dev.int_clr.write(|w| w.serial_in_empty_int_clr().set_bit());
        TX_DONE.wake();
    }
}
