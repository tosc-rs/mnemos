use esp32c3_hal::{prelude::interrupt, peripherals::USB_DEVICE};

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
use futures::FutureExt;

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
        Self { dev }
    }

    async fn serial_server(handle: BidiHandle, kcons: KConsumer<Message<SimpleSerialService>>) {
        loop {
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
    async fn worker(&mut self, chan: BidiHandle) {
        let (tx, rx) = chan.split();
        // preemptively subscribe to RX interrupts.
        let mut rx_ready = RX_READY.subscribe().await.fuse();

        loop {
            // re-enable the RX ready interrupt
            self.dev
                .int_ena
                .modify(|_, w| w.serial_out_recv_pkt_int_ena().set_bit());

            futures::select_biased! {
                rgr = rx.read_grant().fuse() => {
                    let len = rgr.len();

                    // Write the bytes in chunks of up to the FIFO's capacity.
                    for chunk in rgr.chunks(FIFO_CAPACITY) {
                        for &byte in chunk {
                            self.dev.ep1.write(|w| unsafe { w.rdwr_byte().bits(byte) })
                        }
                        // We've written 64 bytes (or less, if we're on the last 64-byte
                        // chunk of the input). Wait for the FIFO to drain.
                        self.flush().await;
                    }

                    rgr.release(len);
                },
                _ = &mut rx_ready => {
                    let mut wgr = tx.send_grant_exact(FIFO_CAPACITY).await;
                    let mut i = 0;
                    while self.dev.ep1_conf.read().serial_out_ep_data_avail().bit_is_set() {
                        wgr[i] = self.dev.ep1.read().rdwr_byte().bits();
                        i += 1;
                    }
                    wgr.commit(i);

                    // re-subscribe to the interrupt
                    rx_ready = RX_READY.subscribe().await.fuse();
                }
            }
        }
    }

    async fn flush(&mut self) {
        // subscribe to a wakeup *before* enabling the interrupt.
        let wait = TX_DONE.subscribe().await;
        self.dev.ep1_conf.modify(|_r, w| w.wr_done().set_bit());
        self.dev
            .int_ena
            .modify(|_, w| w.serial_in_empty_int_ena().set_bit());
        wait.await.expect("TX_DONE waitcell should never be closed")
    }

    pub async fn register(
        mut self,
        k: &'static Kernel,
        cap_in: usize,
        cap_out: usize,
    ) -> Result<(), registry::RegistrationError> {

        let (kprod, kcons) = KChannel::<Message<SimpleSerialService>>::new_async(4)
            .await
            .split();
        let (fifo_a, fifo_b) = new_bidi_channel(cap_in, cap_out).await;

        k.spawn(Self::serial_server(fifo_b, kcons)).await;

        k.spawn(async move { self.worker(fifo_a).await }).await;

        k.with_registry(|reg| reg.register_konly::<SimpleSerialService>(&kprod))
            .await?;

        Ok(())
    }
}

#[interrupt]
fn USB_DEVICE() {
    let dev = unsafe { USB_DEVICE::steal() };
    dev.int_ena.modify(|r, w| {
        if r.serial_in_empty_int_ena().bit_is_set() {
            w.serial_in_empty_int_ena().clear_bit();
            TX_DONE.wake();
        }

        if r.serial_out_recv_pkt_int_ena().bit_is_set() {
            w.serial_out_recv_pkt_int_ena().clear_bit();
            RX_READY.wake();
        }

        w
    });
}