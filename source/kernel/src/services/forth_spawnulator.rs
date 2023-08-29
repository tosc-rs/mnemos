//! Service for spawning new Forth tasks.
//!
//! This is a channel producer that communicates with the background task
//! created by [`SpawnulatorServer::register`].
//!
//! # The Unfortunate Necessity of the Spawnulator
//!
//! Forth tasks may spawn other, child Forth tasks. This is currently
//! accomplished by sending the forked child [`Forth`] VM over a channel to a
//! background task, which actually spawns its [`Forth::run()`] method.  At a
//! glance, this indirection seems unnecessary (and inefficient): why can't the
//! parent task simply call `kernel.spawn(child.run()).await` in the
//! implementation of its `spawn` builtin?
//!
//! The answer is that this is, unfortunately, not possible. The function
//! implementing the `spawn` builtin, `spawn_forth_task()`, *must* be `async`,
//! as it needs to perform allocations for the child task's dictionary, stacks,
//! etc Therefore, calling `spawn_forth_task()` returns an `impl Future` which
//! is awaited inside the `Dispatcher::dispatch_async()` future, which is itself
//! awaited inside `Forth::process_line()` in the  parent VM's [`Forth::run()`]
//! async method. This means the *layout* of the future generated for
//! `spawn_forth_task()` must be known in order to determine the layout of the
//! future generated for [`Forth::run()`]. In order to spawn a new child task, we
//! must call [`Forth::run()`] and then pass the returned `impl Future` to
//! [`Kernel::spawn()`]. This means that the generated `impl Future` for
//! [`Forth::run()`] becomes a local variable in [`Forth::run()`] --- meaning
//! that, in order to compute the layout for [`Forth::run()`], the compiler must
//! first compute the layout for [`Forth::run()`]...which is, naturally,
//! impossible.
//!
//! We can solve this problem by moving the actual
//! `kernel.spawn(forth.run()).await` into a separate task (the spawnulator), to
//! which we send new child [`Forth`] VMs to over a channel, without having
//! called their `run()` methods. Now, the [`Forth::run()`] call does not occur
//! inside of [`Forth::run()`], and its layout is no longer cyclical. I don't
//! feel great about the fact that this requires us to, essentially, place child
//! tasks in a queue in order to wait for the priveliege of being put in a
//! different queue (the scheduler's run queue), but I couldn't easily come up
//! with another solution...

use core::{convert::Infallible, time::Duration};

use crate::{
    comms::{
        kchannel::{KChannel, KConsumer},
        oneshot::Reusable,
    },
    forth::{self, Forth},
    registry::{
        self, known_uuids::kernel::FORTH_SPAWNULATOR, Envelope, KernelHandle, Message,
        RegisteredDriver,
    },
    Kernel,
};
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////
pub struct SpawnulatorService;

impl RegisteredDriver for SpawnulatorService {
    type Request = Request;
    type Response = Response;
    type Error = Infallible;

    type Hello = ();
    type ConnectError = core::convert::Infallible;

    const UUID: Uuid = FORTH_SPAWNULATOR;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////
pub struct Request(forth::Forth);
pub struct Response;

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

pub struct SpawnulatorClient {
    hdl: KernelHandle<SpawnulatorService>,
    reply: Reusable<Envelope<Result<Response, Infallible>>>,
}

impl SpawnulatorClient {
    pub async fn from_registry(kernel: &'static Kernel) -> Self {
        loop {
            match Self::from_registry_no_retry(kernel).await {
                Ok(me) => return me,
                Err(registry::ConnectError::Rejected(_)) => {
                    unreachable!("the SpawnulatorService does not return connect errors!")
                }
                Err(_) => {
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    pub async fn from_registry_no_retry(
        kernel: &'static Kernel,
    ) -> Result<Self, registry::ConnectError<SpawnulatorService>> {
        let prod = kernel.registry().await.get::<SpawnulatorService>().await?;

        Ok(SpawnulatorClient {
            hdl: prod,
            reply: Reusable::new_async().await,
        })
    }

    pub async fn spawn(&mut self, vm: Forth) -> Result<(), forth3::Error> {
        let id = vm.forth.host_ctxt().id();
        tracing::trace!(task.id = id, "spawn u later...");
        match self.hdl.request_oneshot(Request(vm), &self.reply).await {
            Ok(_) => {
                tracing::trace!(task.id = id, "enqueued");
                Ok(())
            }
            Err(_) => {
                tracing::info!(task.id = id, "spawnulator task seems to be dead");
                Err(forth3::Error::InternalError)
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

pub struct SpawnulatorServer;

#[derive(Debug)]
pub enum RegistrationError {
    SpawnulatorAlreadyRegistered,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct SpawnulatorSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "SpawnulatorSettings::default_capacity")]
    pub capacity: usize,
}

impl SpawnulatorServer {
    /// Start the spawnulator background task, returning a handle that can be
    /// used to spawn new `Forth` VMs.
    #[tracing::instrument(
        name = "SpawnulatorServer::register",
        level = tracing::Level::DEBUG,
        skip(kernel),
        ret(Debug),
    )]
    pub async fn register(
        kernel: &'static Kernel,
        settings: SpawnulatorSettings,
    ) -> Result<(), RegistrationError> {
        let (listener, r) = registry::Listener::new(settings.capacity).await;
        let vms = listener.into_request_stream(settings.capacity).await;
        tracing::debug!("who spawns the spawnulator?");
        kernel
            .spawn(SpawnulatorServer::spawnulate(kernel, vms))
            .await;
        tracing::debug!("spawnulator spawnulated!");
        kernel
            .with_registry(|reg| reg.register_konly::<SpawnulatorService>(r))
            .await
            .map_err(|_| RegistrationError::SpawnulatorAlreadyRegistered)?;
        tracing::info!("ForthSpawnulatorService registered");
        Ok(())
    }

    #[tracing::instrument(skip(kernel, vms))]
    async fn spawnulate(
        kernel: &'static Kernel,
        mut vms: registry::listener::RequestStream<SpawnulatorService>,
    ) {
        tracing::debug!("spawnulator running...");
        loop {
            let msg = vms.next_request().await;
            let mut vm = None;

            // TODO(AJM): I really need a better "extract request contents" function
            let resp = msg.msg.reply_with_body(|msg| {
                vm = Some(msg.0);
                Ok(Response)
            });

            let vm = vm.unwrap();
            let id = vm.forth.host_ctxt().id();
            kernel.spawn(vm.run()).await;
            let _ = msg.reply.reply_konly(resp).await;
            tracing::trace!(task.id = id, "spawnulated!");
        }
    }
}

impl SpawnulatorSettings {
    pub const DEFAULT_CAPACITY: usize = 16;

    const fn default_capacity() -> usize {
        Self::DEFAULT_CAPACITY
    }

    pub fn with_capacity(self, capacity: usize) -> Self {
        Self {
            enabled: true, // Should this default to false?
            capacity,
        }
    }
}

impl Default for SpawnulatorSettings {
    fn default() -> Self {
        Self {
            enabled: true, // Should this default to false?
            capacity: Self::DEFAULT_CAPACITY,
        }
    }
}
