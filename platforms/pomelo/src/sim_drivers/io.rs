use std::sync::OnceLock;

use mnemos_kernel::comms::kchannel::KProducer;
use sermux_proto::PortChunk;
use tracing::{debug, error};

use crate::term_iface::SERMUX_TX;
pub static IRQ_TX: OnceLock<KProducer<()>> = OnceLock::new();

pub(crate) fn irq_sync() {
    IRQ_TX.get().unwrap().enqueue_sync(()).ok();
}

pub async fn irq_async() {
    IRQ_TX.get().unwrap().enqueue_async(()).await.ok();
}

pub fn send(port: u16, data: &[u8]) {
    let chunk = PortChunk::new(port, data);
    let required_size = data.len() + 4;
    let mut buf = vec![0; required_size];
    match chunk.encode_to(&mut buf) {
        Ok(ser) => {
            debug!("sermux: sending on {port} {ser:?}");
            let mut tx = SERMUX_TX.get().expect("sermux unavailable, bruh").clone();
            for byte in ser {
                if let Err(e) = tx.try_send(*byte) {
                    tracing::error!("sermux: could not send: {e:?}");
                }
            }
            irq_sync();
        }
        Err(e) => error!("sermux: {e:?}"),
    }
}
