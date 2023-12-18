use core::{
    any::{self, TypeId},
    fmt,
};

use crate::comms::{
    bidi,
    mpsc::{self, error::RecvError, ErasedSender},
};
pub use calliope::{
    message::Reset,
    req_rsp::{Request, Response},
    tricky_pipe::oneshot,
    Service as UserService,
};
use maitake::sync::{RwLock, WaitQueue};
use mnemos_alloc::containers::{Arc, FixedVec};
use portable_atomic::{AtomicU32, Ordering};
use postcard::experimental::max_size::MaxSize;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tracing::{self, debug, info, warn, Level};
pub use uuid::{uuid, Uuid};

pub mod listener;
pub use self::listener::{Listener, Registration};
mod req_rsp;

#[cfg(test)]
mod tests;

/// A partial list of known UUIDs of driver services
pub mod known_uuids {
    use super::*;

    /// Kernel UUIDs
    pub mod kernel {
        use super::*;

        pub const SERIAL_MUX: Uuid = uuid!("54c983fa-736f-4223-b90d-c4360a308647");
        pub const SIMPLE_SERIAL_PORT: Uuid = uuid!("f06aac01-2773-4266-8681-583ffe756554");
        #[deprecated(note = "Use EMB_DISPLAY_V2 instead")]
        pub const EMB_DISPLAY: Uuid = uuid!("b54db574-3eb7-4c89-8bfb-1a20890be68e");
        pub const FORTH_SPAWNULATOR: Uuid = uuid!("4ae4a406-005a-4bde-be91-afc1900f76fa");
        pub const I2C: Uuid = uuid!("011ebd3e-1b14-4bfd-b581-6138239b82f3");
        pub const KEYBOARD: Uuid = uuid!("524d77b1-499c-440b-bd62-e63c0918efb5");
        pub const KEYBOARD_MUX: Uuid = uuid!("70861d1c-9f01-4e9b-89e6-ede77d8f26d8");
        pub const EMB_DISPLAY_V2: Uuid = uuid!("aa6a2af8-afd8-40e3-83c2-2c501c698aa8");
    }

    // In case you need to iterate over every UUID
    #[allow(deprecated)]
    pub static ALL: &[Uuid] = &[
        kernel::SERIAL_MUX,
        kernel::SIMPLE_SERIAL_PORT,
        kernel::EMB_DISPLAY,
        kernel::FORTH_SPAWNULATOR,
        kernel::I2C,
        kernel::KEYBOARD,
        kernel::KEYBOARD_MUX,
        kernel::EMB_DISPLAY_V2,
    ];
}

/// A marker trait designating a registerable driver service.
///
/// Typically used with [`Registry::register`] or [`Registry::register_konly`].
/// A connection to the service can be established using [`Registry::connect`],
/// [`Registry::try_connect`], [`Registry::connect_userspace`], or
/// [`Registry::try_connect_userspace] (depending on the service), after
/// the service has been registered.
pub trait Service: Send + Sync + 'static {
    type ClientMsg: Send + Sync + 'static;
    type ServerMsg: Send + Sync + 'static;
    /// An initial message sent to the service by a client when establishing a
    /// connection.
    ///
    /// This may be used by the service to route connections to specific
    /// resources owned by that service, or to determine whether or not the
    /// connection can be established. If the service does not require initial
    /// data from the client, this type can be set to [`()`].
    // XXX(eliza): ideally, we could default `Hello` to () and `ConnectError` to
    // `Infallible`...do we want to do that? it requires a nightly feature.
    type Hello: Send + Sync + 'static;

    /// Errors returned by the service if an incoming connection handshake is
    /// rejected.
    ///
    /// If the service does not reject connections, this should be set to
    /// [`core::convert::Infallible`].
    type ConnectError: Send + Sync + 'static;

    /// This is the UUID of the driver service
    const UUID: Uuid;
}

/// The driver registry used by the kernel.
pub struct Registry {
    items: RwLock<FixedVec<RegistryItem>>,
    counter: AtomicU32,
    service_added: WaitQueue,
}

pub struct RegistryType {
    tuple_type_id: TypeId,
}

// TODO: This probably goes into the ABI crate, here is fine for now
#[derive(Serialize, Deserialize)]
pub struct UserRequest<'a> {
    // TODO: Maybe not the UUID, maybe pre-discover a shorter UID?
    uid: Uuid,
    nonce: u32,
    #[serde(borrow)]
    req_bytes: &'a [u8],
}

pub type KernelSendError<T> = mpsc::error::SendError<Reset, T>;

// TODO: This probably goes into the ABI crate, here is fine for now
#[derive(Serialize, Deserialize)]
pub struct UserResponse<U, E> {
    // TODO: Maybe not the UUID, maybe pre-discover a shorter UID?
    uuid: Uuid,
    nonce: u32,
    reply: Result<U, E>,
    //
    // KEEP IN SYNC WITH POSTCARD_MAX_SIZE BELOW!
    //
}

// UserResponse

impl<U: MaxSize, E: MaxSize> MaxSize for UserResponse<U, E> {
    //
    // KEEP IN SYNC WITH STRUCT DEFINITION ABOVE!
    //
    const POSTCARD_MAX_SIZE: usize = {
        <[u8; 16] as MaxSize>::POSTCARD_MAX_SIZE
            + <u32 as MaxSize>::POSTCARD_MAX_SIZE
            + <Result<U, E> as MaxSize>::POSTCARD_MAX_SIZE
    };
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ServiceId(pub(crate) u32);

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ClientId(pub(crate) u32);

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct RequestResponseId(u32);

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MessageKind {
    Request,
    Response,
}

impl RequestResponseId {
    pub fn new(id: u32, kind: MessageKind) -> Self {
        let bit = match kind {
            MessageKind::Request => 0b1,
            MessageKind::Response => 0b0,
        };
        Self((id << 1) | bit)
    }

    pub fn id(&self) -> u32 {
        self.0 >> 1
    }

    pub fn kind(&self) -> MessageKind {
        let bit = self.0 & 0b1;

        if bit == 1 {
            MessageKind::Request
        } else {
            MessageKind::Response
        }
    }
}

/*
/// A wrapper for a message TO and FROM a driver service.
/// Used to be able to add additional message metadata without
/// changing the fundamental message type.
#[non_exhaustive]
pub struct Envelope<P> {
    pub body: P,
    service_id: ServiceId,
    client_id: ClientId,
    request_id: RequestResponseId,
}

pub struct OpenEnvelope<P> {
    body: PhantomData<fn() -> P>,
    service_id: ServiceId,
    client_id: ClientId,
    request_id: RequestResponseId,
}

/// The [Message] kind represents a full reply/response sequence to
/// a driver service. This is the concrete type received by the driver
/// service.
///
/// It contains the Request, e.g. [RegisteredDriver::Request], as well
/// as a [ReplyTo] that allows the driver service to respond to a given
/// request
pub struct Message<RD: Service> {
    pub msg: Envelope<RD::Request>,
    pub reply: ReplyTo<RD>,
}

/// A `ReplyTo` is used to allow the CLIENT of a service to choose the
/// way that the driver SERVICE replies to us. Essentially, this acts
/// as a "self addressed stamped envelope" for the SERVICE to use to
/// reply to the CLIENT.
pub enum ReplyTo<RD: Service> {
    // This can be used to reply directly to another kernel entity,
    // without a serialization step
    KChannel(KProducer<Envelope<Result<RD::Response, RD::Error>>>),

    // This can be used to reply directly ONCE to another kernel entity,
    // without a serialization step
    OneShot(Sender<Envelope<Result<RD::Response, RD::Error>>>),

    // This can be used to reply to userspace. Responses are serialized
    // and sent over the bbq::MpscProducer
    Userspace {
        nonce: u32,
        outgoing: bbq::MpscProducer,
    },
}

*/
#[derive(Debug, Eq, PartialEq)]
pub enum ReplyError {
    KOnlyUserspaceResponse,
    ReplyChannelClosed,
    UserspaceSerializationError,
    InternalError,
}

#[derive(Debug, Eq, PartialEq)]
pub enum UserHandlerError {
    DeserializationFailed,
    QueueFull,
}

#[derive(Debug, Eq, PartialEq)]
pub enum RegistrationError {
    UuidAlreadyRegistered(Uuid),
    RegistryFull,
}

/// Errors returned by [`Registry::connect`] and [`Registry::try_connect`].
pub enum ConnectError<D: Service> {
    /// No [`RegisteredDriver`] of this type was found!
    ///
    /// The [`RegisteredDriver::Hello`] message is returned, so that it can be
    /// used again.
    NotFound(D::Hello),
    /// The remote [`RegisteredDriver`] rejected the connection.
    Rejected(D::ConnectError),
    /// The remote [`RegisteredDriver`] has been registered, but the service
    /// task has terminated.
    DriverDead,
}

/// Errors returned by [`Registry::connect_userspace`] and
/// [`Registry::try_connect_userspace`].
pub enum UserConnectError<D: Service> {
    /// Deserializing the userspace `Hello` message failed.
    DeserializationFailed(postcard::Error),
    /// Connecting to the service failed.
    Connect(ConnectError<D>),
}

/// A UserspaceHandle is used to process incoming serialized messages from
/// userspace. It contains a method that can be used to deserialize messages
/// from a given UUID, and send that request (if the deserialization is
/// successful) to a given driver service.
pub struct UserspaceHandle {
    chan: bidi::SerBiDi<Reset>,
    service_id: ServiceId,
    client_id: ClientId,
}

/// A KernelHandle is used to send typed messages to a kernelspace Driver
/// service.
pub struct KernelHandle<S: Service> {
    chan: bidi::BiDi<S::ServerMsg, S::ClientMsg, Reset>,
    service_id: ServiceId,
    client_id: ClientId,
}

/// The payload of a registry item.
///
/// The typeid is stored here to allow the userspace handle to look up the UUID key
/// without knowing the proper typeid. Kernel space drivers should always check that the
/// tuple type id is correct.
struct RegistryValue {
    type_id: TypeId,
    conns: ErasedSender,
    service_id: ServiceId,
}

/// Right now we don't use a real HashMap, but rather a hand-rolled index map.
/// Therefore our registry is basically a `Vec<RegistryItem>`.
struct RegistryItem {
    key: Uuid,
    value: RegistryValue,
}

// RegistryType

impl RegistryType {
    pub fn type_of(&self) -> TypeId {
        self.tuple_type_id
    }
}

// Registry

impl Registry {
    /// Create a new registry with room for up to `max_items` registered drivers.
    pub fn new(max_items: usize) -> Self {
        let items = FixedVec::try_new(max_items).unwrap();
        Self {
            items: RwLock::new(items),
            counter: AtomicU32::new(0),
            service_added: WaitQueue::new(),
        }
    }

    /// Bind a [`Listener`] for a driver service of type `S`.
    ///
    /// This is a helper method which creates a [`Listener`] using
    /// [`Listener::new`] and then registers that [`Listener`]'s
    /// [`listener::Registration`] with the registry using
    /// [`Registry::register`].
    pub async fn bind<S: Service>(&self, capacity: u8) -> Result<Listener<S>, RegistrationError> {
        let (listener, registration) = Listener::new(capacity).await;
        self.register(registration).await?;
        Ok(listener)
    }

    /// Register a service with this registry.
    #[tracing::instrument(
        name = "Registry::register",
        level = Level::INFO,
        skip(self, registration),
        fields(svc = %any::type_name::<S>(), svc.uuid = ?S::UUID),
        err(Display),
    )]
    pub async fn register<S: Service>(
        &self,
        registration: listener::Registration<S>,
    ) -> Result<(), RegistrationError> {
        // construct the registry entry for the new service.
        let conns = registration.tx.into_erased();
        let service_id = self.counter.fetch_add(1, Ordering::Relaxed);
        let entry = RegistryItem {
            key: S::UUID,
            value: RegistryValue {
                type_id: tuple_type_id::<S>(),
                conns,
                service_id: ServiceId(service_id),
            },
        };

        // insert the entry into the registry.
        let mut lock = self.items.write().await;
        if lock.as_slice().iter().any(|i| i.key == entry.key) {
            warn!("Failed to register service: the UUID is already registered!");
            return Err(RegistrationError::UuidAlreadyRegistered(entry.key));
        }

        lock.try_push(entry).map_err(|_| {
            warn!("Failed to register service: the registry is full!");
            // close the "service added" waitcell, because no new services will
            // ever be added.
            self.service_added.close();
            RegistrationError::RegistryFull
        })?;

        // release the lock on the registry *before* waking any tasks waiting
        // for new services to be added, so that they can access the new
        // service's entry without waiting for the lock to be released.
        drop(lock);
        self.service_added.wake_all();

        info!(svc.id = service_id, "Registered service");

        Ok(())
    }

    /// Attempt to get a kernelspace (including drivers) handle of a given driver service,
    /// which does not require sending a [`RegisteredDriver::Hello`] message.
    ///
    /// This can be used by drivers and tasks to interface with a registered driver
    /// service.
    ///
    /// The driver service MUST have already been registered using [Registry::register] or
    /// [Registry::register_konly] prior to making this call, otherwise no handle will
    /// be returned. To wait until a driver is registered, use
    /// [`Registry::connect`] instead.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[KernelHandle`]`)` if the requested service was found and
    ///   a connection was successfully established.
    ///
    /// - [`Ok`]`(`[KernelHandle`]`)` if the requested service was found and
    ///   a connection was successfully established.
    ///
    /// - [`Err`]`(`[`ConnectError::Rejected`]`)` if the service [rejected] the
    ///   incoming connection.
    ///
    /// - [`Err`]`(`[`ConnectError::DriverDead`]`)` if the service has been
    ///   registered but is no longer running.
    ///
    /// - [`Err`]`(`[`ConnectError::NotFound`]`)` if no service matching the
    ///   requested [`RegisteredDriver`] type exists in the registry.
    ///
    /// [rejected]: listener::Handshake::reject
    #[tracing::instrument(
        name = "Registry::try_connect",
        level = Level::DEBUG,
        skip(self, hello),
        fields(svc = %any::type_name::<S>()),
    )]
    pub async fn try_connect<S: Service>(
        &self,
        hello: S::Hello,
    ) -> Result<KernelHandle<S>, ConnectError<S>> {
        let (conns, service_id) = {
            // /!\ WARNING: Load-bearing scope /!\
            //
            // We need to ensure that we only hold the lock on `self.items`
            // while we're accessing the item; *not* while we're `await`ing a
            // bunch of other stuff to connect to the service. This is
            // important, because if we held the lock, no other task would be
            // able to connect while we're waiting for the handshake,
            // potentially causing a deadlock...
            let items = self.items.read().await;
            let Some(item) = Self::get::<S>(&items) else {
                return Err(ConnectError::NotFound(hello));
            };
            let conns = item.value.conns.clone();
            let service_id = item.value.service_id;
            (conns, service_id)
        };

        let permit = conns
            .reserve()
            .await
            .map_err(|_| ConnectError::DriverDead)?
            .downcast::<listener::Handshake<S>>()
            .expect("downcasting a service entry to the expected service type must succeed!");

        let handshake_rx = Arc::new(oneshot::Oneshot::<listener::HandshakeResult<S>>::new())
            .await
            .into_inner()
            .arc_receiver()
            .expect("Arc was freshly allocated and is safe to use");
        let reply = handshake_rx.sender().await.expect("no sender should exist");
        permit.send(listener::Handshake {
            hello,
            accept: listener::Accept { reply },
        });

        let chan = handshake_rx
            .recv()
            .await
            .map_err(|_| ConnectError::DriverDead)?
            .map_err(ConnectError::Rejected)?;

        let client_id = self.counter.fetch_add(1, Ordering::Relaxed);

        info!(
            svc = %any::type_name::<S>(),
            uuid = ?S::UUID,
            service_id = service_id.0,
            client_id,
            "Got KernelHandle from Registry",
        );

        Ok(KernelHandle {
            chan,
            service_id,
            client_id: ClientId(client_id),
        })
    }

    /// Get a kernelspace (including drivers) handle of a given driver service,
    /// waiting until the service is registered if it does not already exist.
    ///
    /// This can be used by drivers and tasks to interface with a registered
    /// driver service.
    ///
    /// If no service matching the requested [`RegisteredDriver`] type has been
    /// registered, this method will wait until that service is added to the
    /// registry, unless the registry becomes full.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[KernelHandle`]`)` if the requested service was found and
    ///   a connection was successfully established.
    ///
    /// - [`Err`]`(`[`ConnectError::Rejected`]`)` if the service [rejected] the
    ///   incoming connection.
    ///
    /// - [`Err`]`(`[`ConnectError::DriverDead`]`)` if the service has been
    ///   registered but is no longer running.
    ///
    /// - [`Err`]`(`[`ConnectError::NotFound`]`)` if no service matching the
    ///   requested [`RegisteredDriver`] type exists *and* the registry was
    ///   full.
    ///
    /// [rejected]: listener::Handshake::reject
    #[tracing::instrument(
        name = "Registry::connect",
        level = Level::DEBUG,
        skip(self, hello),
        fields(svc = %any::type_name::<RD>()),
    )]
    pub async fn connect<RD>(&self, hello: RD::Hello) -> Result<KernelHandle<RD>, ConnectError<RD>>
    where
        RD: Service,
    {
        let mut hello = Some(hello);
        let mut is_full = false;
        loop {
            match self.try_connect(hello.take().unwrap()).await {
                Ok(handle) => return Ok(handle),
                Err(ConnectError::NotFound(h)) if !is_full => {
                    hello = Some(h);
                    debug!("no service found; waiting for one to be added...");
                    // wait for a service to be added to the registry
                    is_full = self.service_added.wait().await.is_err();
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Get a kernelspace (including drivers) handle of a given driver service,
    /// waiting until the service is registered if it does not already exist.
    ///
    /// This can be used by drivers and tasks to interface with a registered driver
    /// service.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[KernelHandle`]`)` if the requested service was found and
    ///   a connection was successfully established.
    ///
    /// - [`Err`]`(`[`ConnectError`]`)` if the requested service was not
    ///   found in the registry, or if the service [rejected] the incoming
    ///   connection. Note that [`ConnectError::NotFound`] is not returned
    ///   _unless_ the registry is full and no more services will be added.
    ///
    /// [rejected]: listener::Handshake::reject
    #[tracing::instrument(
        name = "Registry::connect_userspace",
        level = Level::DEBUG,
        skip(self),
        fields(svc = %any::type_name::<S>()),
    )]
    pub async fn connect_userspace<S>(
        &self,
        user_hello: &[u8],
    ) -> Result<UserspaceHandle, UserConnectError<S>>
    where
        S: UserService + Send + Sync + 'static,
    {
        let mut is_full = false;
        loop {
            match self.try_connect_userspace(user_hello).await {
                Ok(handle) => return Ok(handle),
                Err(UserConnectError::Connect(ConnectError::NotFound(_))) if !is_full => {
                    debug!("no service found; waiting for one to be added...");
                    // wait for a service to be added to the registry
                    is_full = self.service_added.wait().await.is_err();
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Try to get a handle capable of processing serialized userspace messages to a
    /// registered driver service, given a byte buffer for the userspace
    /// [`RegisteredDriver::Hello`] message.
    ///
    /// The driver service MUST have already been registered using [Registry::register] or
    /// prior to making this call, otherwise no handle will be returned.
    ///
    /// Driver services registered with [`Registry::register_konly`] cannot be
    /// retrieved via a call to [`Registry::try_connect_userspace`].
    #[tracing::instrument(
        name = "Registry::try_connect_userspace",
        level = Level::DEBUG,
        skip(self),
        fields(svc = %any::type_name::<S>()),
    )]
    pub async fn try_connect_userspace<S>(
        &self,
        user_hello: &[u8],
    ) -> Result<UserspaceHandle, UserConnectError<S>>
    where
        S: UserService + Send + Sync + 'static,
    {
        let hello = postcard::from_bytes::<<S as UserService>::Hello>(user_hello)
            .map_err(UserConnectError::DeserializationFailed)?;
        let KernelHandle {
            chan,
            service_id,
            client_id,
        } = self
            .try_connect(hello)
            .await
            .map_err(UserConnectError::Connect)?;
        let chan = chan.into_serde();
        Ok(UserspaceHandle {
            chan,
            service_id,
            client_id,
        })
    }

    fn get<S: Service>(items: &FixedVec<RegistryItem>) -> Option<&RegistryItem> {
        let Some(item) = items.as_slice().iter().find(|i| i.key == S::UUID) else {
            debug!(
                svc = %any::type_name::<S>(),
                uuid = ?S::UUID,
                "No service for this UUID exists in the registry!"
            );
            return None;
        };

        let expected_type_id = tuple_type_id::<S>();
        let actual_type_id = item.value.type_id;
        if expected_type_id != actual_type_id {
            warn!(
                svc = %any::type_name::<S>(),
                uuid = ?S::UUID,
                type_id.expected = ?expected_type_id,
                type_id.actual = ?actual_type_id,
                "Registry entry's type ID did not match driver's type ID. This is (probably?) a bug!"
            );
            return None;
        }

        Some(item)
    }
}
// UserspaceHandle

// impl UserspaceHandle {
//     pub fn process_msg(
//         &self,
//         user_msg: UserRequest<'_>,
//         user_ring: &bbq::MpscProducer,
//     ) -> Result<(), UserHandlerError> {
//         unsafe {
//             (self.req_deser)(
//                 user_msg,
//                 &self.req_producer_leaked,
//                 user_ring,
//                 self.service_id,
//                 self.client_id,
//             )
//         }
//     }
// }

// KernelHandle

impl<S: Service> KernelHandle<S> {
    pub async fn send(
        &self,
        msg: S::ClientMsg,
    ) -> Result<(), mpsc::error::SendError<Reset, S::ClientMsg>> {
        self.chan.tx().send(msg).await
    }

    pub async fn recv(&self) -> Result<S::ServerMsg, RecvError<Reset>> {
        self.chan.rx().recv().await
    }
}

// UserHandlerError

impl fmt::Display for UserHandlerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => f.pad("service queue full"),
            Self::DeserializationFailed => f.pad("failed to deserialize user request"),
        }
    }
}

// ConnectError

impl<D> PartialEq for ConnectError<D>
where
    D: Service,
    D::ConnectError: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::DriverDead, Self::DriverDead) => true,
            (Self::NotFound(_), Self::NotFound(_)) => true,
            (Self::Rejected(this), Self::Rejected(that)) => this == that,
            _ => false,
        }
    }
}

impl<D> Eq for ConnectError<D>
where
    D: Service,
    D::ConnectError: Eq,
{
}

impl<D> fmt::Debug for ConnectError<D>
where
    D: Service,
    D::ConnectError: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbs = match self {
            Self::DriverDead => f.debug_struct("DriverDead"),
            Self::NotFound(_) => f.debug_struct("NotFound"),
            Self::Rejected(error) => {
                let mut d = f.debug_struct("Rejected");
                d.field("error", error);
                d
            }
        };
        dbs.field("svc", &mycelium_util::fmt::display(any::type_name::<D>()))
            .finish()
    }
}

impl<D> fmt::Display for ConnectError<D>
where
    D: Service,
    D::ConnectError: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = any::type_name::<D>();
        match self {
            Self::DriverDead => write!(f, "the {name} service has terminated"),
            Self::NotFound(_) => write!(f, "no {name} service found in the registry",),
            Self::Rejected(err) => write!(f, "the {name} service rejected the connection: {err}",),
        }
    }
}

// UserConnectError

impl<D> fmt::Debug for UserConnectError<D>
where
    D: Service,
    D::ConnectError: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeserializationFailed(error) => {
                let mut d = f.debug_struct("DeserializationFailed");
                d.field("error", error);
                d
            }
            Self::Connect(error) => {
                let mut d = f.debug_struct("Connect");

                d.field("error", error);
                d
            }
        }
        .field("svc", &mycelium_util::fmt::display(any::type_name::<D>()))
        .finish()
    }
}

impl<D> fmt::Display for UserConnectError<D>
where
    D: Service,
    D::ConnectError: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(err) => fmt::Display::fmt(err, f),
            Self::DeserializationFailed(err) => write!(
                f,
                "failed to deserialize userspace Hello for the {} service: {err}",
                any::type_name::<D>()
            ),
        }
    }
}

// RegistrationError

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryFull => "the registry is full".fmt(f),
            Self::UuidAlreadyRegistered(uuid) => {
                write!(f, "a service with UUID {uuid} has already been registered")
            }
        }
    }
}

impl<S> Service for S
where
    S: UserService + Send + Sync + 'static,
{
    const UUID: Uuid = S::UUID;
    type ClientMsg = S::ClientMsg;
    type ServerMsg = S::ServerMsg;
    type Hello = S::Hello;
    type ConnectError = S::ConnectError;
}

fn tuple_type_id<S: Service>() -> TypeId {
    TypeId::of::<(S::Hello, S::ClientMsg, S::ServerMsg, S::ConnectError)>()
}
