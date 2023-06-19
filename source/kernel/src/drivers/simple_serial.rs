use uuid::Uuid;

use crate::comms::bbq::BidiHandle;
use crate::comms::oneshot::Reusable;
use crate::Kernel;

use crate::registry::{known_uuids, Envelope, KernelHandle, RegisteredDriver, ReplyTo};

pub struct SimpleSerial {
    kprod: KernelHandle<SimpleSerial>,
    rosc: Reusable<Envelope<Result<Response, SimpleSerialError>>>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum SimpleSerialError {
    AlreadyAssignedPort,
}

impl SimpleSerial {
    pub async fn from_registry(kernel: &'static Kernel) -> Option<Self> {
        let kprod = kernel
            .with_registry(|reg| reg.get::<SimpleSerial>())
            .await?;

        Some(SimpleSerial {
            kprod,
            rosc: Reusable::new_async(kernel).await,
        })
    }

    pub async fn get_port(&mut self) -> Option<BidiHandle> {
        self.kprod
            .send(
                Request::GetPort,
                ReplyTo::OneShot(self.rosc.sender().await.ok()?),
            )
            .await
            .ok()?;
        let resp = self.rosc.receive().await.ok()?;

        let Response::PortHandle { handle } = resp.body.ok()?;
        Some(handle)
    }
}

impl RegisteredDriver for SimpleSerial {
    type Request = Request;
    type Response = Response;
    type Error = SimpleSerialError;

    const UUID: Uuid = known_uuids::kernel::SIMPLE_SERIAL_PORT;
}

pub enum Request {
    GetPort,
}

pub enum Response {
    PortHandle { handle: BidiHandle },
}
