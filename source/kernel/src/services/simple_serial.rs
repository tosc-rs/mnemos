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

use crate::registry::{self, known_uuids, KernelHandle, Service};

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

pub struct SimpleSerialService;

impl Service for SimpleSerialService {
    type ClientMsg = Request;
    type ServerMsg = Result<BidiHandle, SimpleSerialError>;

    // TODO(eliza): maybe we should do a v2 of this trait where the `Hello`
    // message is `GetPort` and the request/response types are serial frames?
    // but we can't do this until services can be bidi pipes instead of req
    // channels...
    type Hello = ();
    type ConnectError = SimpleSerialError;

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
    chan: KernelHandle<SimpleSerialService>,
}

impl SimpleSerialClient {
    pub async fn from_registry(
        kernel: &'static Kernel,
    ) -> Result<Self, registry::ConnectError<SimpleSerialService>> {
        let chan = kernel.registry().connect::<SimpleSerialService>(()).await?;

        Ok(SimpleSerialClient { chan })
    }

    pub async fn from_registry_no_retry(
        kernel: &'static Kernel,
    ) -> Result<Self, registry::ConnectError<SimpleSerialService>> {
        let chan = kernel
            .registry()
            .try_connect::<SimpleSerialService>(())
            .await?;

        Ok(SimpleSerialClient { chan })
    }

    pub async fn get_port(&mut self) -> Option<BidiHandle> {
        self.chan.send(Request::GetPort).await.ok()?;
        self.chan.recv().await.ok()?.ok()
    }
}
