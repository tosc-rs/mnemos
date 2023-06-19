use uuid::Uuid;

use crate::comms::bbq::BidiHandle;
use crate::comms::oneshot::Reusable;
use crate::Kernel;

use crate::registry::{known_uuids, Envelope, KernelHandle, RegisteredDriver, ReplyTo};

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

pub struct SimpleSerialService;

impl RegisteredDriver for SimpleSerialService {
    type Request = Request;
    type Response = Response;
    type Error = SimpleSerialError;

    const UUID: Uuid = known_uuids::kernel::SIMPLE_SERIAL_PORT;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

pub enum Request {
    GetPort,
}

pub enum Response {
    PortHandle { handle: BidiHandle },
}

#[derive(Debug, Eq, PartialEq)]
pub enum SimpleSerialError {
    AlreadyAssignedPort,
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

pub struct SimpleSerialClient {
    kprod: KernelHandle<SimpleSerialService>,
    rosc: Reusable<Envelope<Result<Response, SimpleSerialError>>>,
}

impl SimpleSerialClient {
    pub async fn from_registry(kernel: &'static Kernel) -> Option<Self> {
        let kprod = kernel
            .with_registry(|reg| reg.get::<SimpleSerialService>())
            .await?;

        Some(SimpleSerialClient {
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
