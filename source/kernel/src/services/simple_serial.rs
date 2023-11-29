//! # Simple Serial
//!
//! This is a basic service that defines some kind of serial port.
//!
//! This module only contains the service definition and client definition,
//! the server must be implemented for the given target platform.

use uuid::Uuid;

use crate::comms::bbq::BidiHandle;
use crate::comms::oneshot::Reusable;
use crate::Kernel;

use crate::registry::{self, known_uuids, Envelope, KernelHandle, ReplyTo, Service};

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

pub struct SimpleSerialService;

impl Service for SimpleSerialService {
    type Request = Request;
    type Response = Response;
    type Error = SimpleSerialError;

    // TODO(eliza): maybe we should do a v2 of this trait where the `Hello`
    // message is `GetPort` and the request/response types are serial frames?
    // but we can't do this until services can be bidi pipes instead of req
    // channels...
    type Hello = ();
    type ConnectError = core::convert::Infallible;

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
    pub async fn from_registry(
        kernel: &'static Kernel,
    ) -> Result<Self, registry::ConnectError<SimpleSerialService>> {
        let kprod = kernel.registry().connect::<SimpleSerialService>(()).await?;

        Ok(SimpleSerialClient {
            kprod,
            rosc: Reusable::new_async().await,
        })
    }

    pub async fn from_registry_no_retry(
        kernel: &'static Kernel,
    ) -> Result<Self, registry::ConnectError<SimpleSerialService>> {
        let kprod = kernel
            .registry()
            .try_connect::<SimpleSerialService>(())
            .await?;

        Ok(SimpleSerialClient {
            kprod,
            rosc: Reusable::new_async().await,
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
