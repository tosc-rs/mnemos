use embassy_net_driver_channel::{RxRunner, TxRunner};
use futures::select_biased;
use kernel::{
    comms::bbq,
    mnemos_alloc::containers::FixedVec,
    registry::{listener::RequestStream, uuid, RegisteredDriver, Uuid},
};

const MTU: usize = 1492; // this is the default MTU from esp-wifi...

pub struct WifiService;

impl RegisteredDriver for WifiService {
    type Request = ControlRequest;
    type Response = ControlResponse;
    type Error = ControlError;
    type Hello = ();
    type ConnectError = core::convert::Infallible;

    const UUID: Uuid = uuid!("be574150-6013-4fa9-97c0-a43a56ecb44d");
}

pub enum ControlRequest {
    ListAps,
    ConnectToAp(Connect),
}

pub struct Connect {
    pub ssid: FixedVec<u8>,
    pub password: FixedVec<u8>,
    pub dhcp: bool,
    frames: bbq::BidiHandle,
}

pub enum ControlResponse {
    ListAps { aps: FixedVec<()> },
}

pub enum ControlError {}

pub struct Wifi {}

impl Wifi {
    async fn run(mut self, control_reqs: RequestStream<WifiService>) {
        // select_biased! {}
        todo!("implement wifi control service")
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
