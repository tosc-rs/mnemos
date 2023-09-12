use embassy_net_driver_channel::{RxRunner, TxRunner};
use futures::{select_biased, FutureExt};
use kernel::{
    comms::{bbq, oneshot::Reusable},
    mnemos_alloc::containers::FixedVec,
    registry::{
        listener::RequestStream, uuid, Envelope, KernelHandle, OneshotRequestError,
        RegisteredDriver, Uuid,
    },
};

const MTU: usize = 1492; // this is the default MTU from esp-wifi...

pub struct WifiControlService;

impl RegisteredDriver for WifiControlService {
    type Request = ControlRequest;
    type Response = ControlResponse;
    type Error = ControlError;
    type Hello = ();
    type ConnectError = core::convert::Infallible;

    const UUID: Uuid = uuid!("be574150-6013-4fa9-97c0-a43a56ecb44d");
}

pub enum ControlRequest {
    ListAps,
    ConnectToAp {
        conn: ConnectionSettings,
        // TODO: when service channels become bidis, this could potentially be the
        // channel of a `RawWifiService`/`WifiEthernetFrameService` thingy, and the rest
        // of the connect could be the hello.
        frames: bbq::BidiHandle,
    },
}

#[non_exhaustive]
pub struct ConnectionSettings {
    pub ssid: FixedVec<u8>,
    pub password: FixedVec<u8>,
}

pub enum ControlResponse {
    ListAps { aps: FixedVec<()> },
}

#[derive(Debug)]
pub enum ControlError {
    Connect(ConnectError),
}

#[derive(Debug)]
pub enum ConnectError {
    Request(OneshotRequestError),
}

pub struct Wifi {}

pub struct WifiClient {
    handle: KernelHandle<WifiControlService>,
    reply: Reusable<Envelope<Result<ControlResponse, ControlError>>>,
}

pub struct Ap {
    pub frames: bbq::BidiHandle,
}

impl WifiClient {
    pub async fn connect(
        &mut self,
        settings: ConnectionSettings,
        bbq_capacity: usize,
    ) -> Result<Ap, ConnectError> {
        let (frames, frames_out) = bbq::new_bidi_channel(bbq_capacity, bbq_capacity).await;
        let rsp = self
            .handle
            .request_oneshot(
                ControlRequest::ConnectToAp {
                    conn: settings,
                    frames: frames_out,
                },
                &self.reply,
            )
            .await
            .map_err(ConnectError::Request)?;
        match rsp.body {
            Ok(_) => Ok(Ap { frames }),
            Err(ControlError::Connect(error)) => Err(error),
            Err(error) => unreachable!("random unexpected error: {error:?}"),
        }
    }
}

impl Wifi {
    async fn run(mut self, control_reqs: RequestStream<WifiControlService>) {
        loop {
            select_biased! {
                req = control_reqs.next_request().fuse() => {
                    todo!()
                }
            }
        }
    }

    async fn run_tx(mut tx: TxRunner<'static, MTU>, bbq: bbq::Consumer) {
        loop {
            let rgr = bbq.read_grant().await;
            let mut frame = &rgr[..];
            let len = frame.len();
            while !frame.is_empty() {
                let buf = tx.tx_buf().await;
                if buf.len() >= frame.len() {
                    buf[..frame.len()].copy_from_slice(frame);
                    frame = &[];
                } else {
                    let (chunk, rest) = frame.split_at(buf.len());
                    buf.copy_from_slice(chunk);
                    frame = rest;
                }
                tx.tx_done();
            }
            rgr.release(len);
        }
    }

    async fn run_rx(mut rx: RxRunner<'static, MTU>, bbq: bbq::SpscProducer) {
        loop {
            let len = {
                let buf = rx.rx_buf().await;
                let len = buf.len();
                // TODO(eliza): we might be able to use `send_grant_max` and
                // only call `rx_done` with the amount we consumed?
                let mut wgr = bbq.send_grant_exact(len).await;
                wgr.copy_from_slice(buf);
                wgr.commit(len);
                len
            };
            rx.rx_done(len);
        }
    }

    // async fn run_rx(mut rx: RxRunner<'static, MTU>, bbq: bbq::SpscProducer) {
    //     loop {
    //         let len = {
    //             let buf = rx.rx_buf().await;
    //             let mut wgr = bbq.send_grant_max(buf.len()).await;
    //             let len = wgr.len();
    //             wgr.copy_from_slice(&buf[..len]);
    //             wgr.commit(len);
    //             len
    //         };
    //         rx.rx_done(len);
    //     }
    // }
}
