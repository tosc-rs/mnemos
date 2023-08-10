use futures::FutureExt;
use hal_x86_64::serial::Port;
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
use mycelium_util::io::{Read, Write};

pub struct Uart {
    port: &'static Port,
    rx_irq: &'static WaitCell,
    tx_irq: &'static WaitCell,
}

impl Uart {
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

    async fn work(self, chan: BidiHandle) {
        let (tx, rx) = chan.split();
        let mut port = self.port.lock().set_non_blocking();
        // preemptively subscribe to RX interrupts.
        let mut rx_ready = self.rx_irq.subscribe().await;
        loop {
            futures::select_biased! {
                // RX bytes available!
                _ = (&mut rx_ready).fuse() => {

                    let mut reading = true;
                    while reading {
                        let mut wgr = tx.send_grant_max(64).await; // 64 chosen completely arbitrarily...
                        let mut len = 0;
                        // read until WouldBlock is returned or we've read the
                        // entire read grant
                        for byte in wgr.chunks_mut(1) {
                            if port.read(byte).is_err() {
                                reading = false;
                                break;
                            } else {
                                len += 1;
                            }
                        }

                        wgr.commit(len);
                    }

                    // re-subscribe to the interrupt
                    rx_ready = self.rx_irq.subscribe().await;
                },
                rgr = rx.read_grant().fuse() => {
                    todo!("eliza")
                },
            }
        }
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

        k.spawn(self.work(fifo_a)).await;

        k.with_registry(|reg| reg.register_konly::<SimpleSerialService>(&kprod))
            .await?;

        Ok(())
    }
}
