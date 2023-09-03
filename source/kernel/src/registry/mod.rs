use core::{
    any::{self, TypeId},
    fmt,
    marker::PhantomData,
    mem, ptr,
};

use crate::comms::{kchannel, oneshot::Reusable};
use mnemos_alloc::containers::FixedVec;
use postcard::experimental::max_size::MaxSize;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spitebuf::EnqueueError;
use tracing::{self, debug, info};
pub use uuid::{uuid, Uuid};

use crate::comms::{
    bbq,
    kchannel::{ErasedKProducer, KProducer},
    oneshot::{ReusableError, Sender},
};

pub mod listener;
pub use self::listener::{Listener, Registration};

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
/// [`Registry::connect_with_hello`], or
/// [`Registry::connect_userspace_with_hello`] (depending on the service), after
/// the service has been registered..
pub trait RegisteredDriver {
    /// This is the type of the request sent TO the driver service
    type Request: 'static;

    /// This is the type of a SUCCESSFUL response sent FROM the driver service
    type Response: 'static;

    /// This is the type of an UNSUCCESSFUL response sent FROM the driver service
    type Error: 'static;

    /// An initial message sent to the service by a client when establishing a
    /// connection.
    ///
    /// This may be used by the service to route connections to specific
    /// resources owned by that service, or to determine whether or not the
    /// connection can be established. If the service does not require initial
    /// data from the client, this type can be set to [`()`].
    // XXX(eliza): ideally, we could default `Hello` to () and `ConnectError` to
    // `Infallible`...do we want to do that? it requires a nightly feature.
    type Hello: 'static;

    /// Errors returned by the service if an incoming connection handshake is
    /// rejected.
    ///
    /// If the service does not reject connections, this should be set to
    /// [`core::convert::Infallible`].
    type ConnectError: 'static;

    /// This is the UUID of the driver service
    const UUID: Uuid;

    /// Get the [`TypeId`] used to make sure that driver instances are correctly typed.
    /// Corresponds to the same type ID as `(`[`Self::Request`]`, `[`Self::Response`]`,
    /// `[`Self::Error`]`, `[`Self::Hello`]`, `[`Self::ConnectError`]`)`.
    fn type_id() -> RegistryType {
        RegistryType {
            tuple_type_id: TypeId::of::<(
                Self::Request,
                Self::Response,
                Self::Error,
                Self::Hello,
                Self::ConnectError,
            )>(),
        }
    }
}

pub struct RegistryType {
    tuple_type_id: TypeId,
}

/// The driver registry used by the kernel.
pub struct Registry {
    items: FixedVec<RegistryItem>,
    counter: u32,
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
pub struct Message<RD: RegisteredDriver> {
    pub msg: Envelope<RD::Request>,
    pub reply: ReplyTo<RD>,
}

/// A `ReplyTo` is used to allow the CLIENT of a service to choose the
/// way that the driver SERVICE replies to us. Essentially, this acts
/// as a "self addressed stamped envelope" for the SERVICE to use to
/// reply to the CLIENT.
pub enum ReplyTo<RD: RegisteredDriver> {
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
    UuidAlreadyRegistered,
    RegistryFull,
}

/// Errors returned by [`Registry::connect`] and
/// [`Registry::connect_with_hello`].
pub enum ConnectError<D: RegisteredDriver> {
    /// No [`RegisteredDriver`] of this type was found!
    NotFound,
    /// The remote [`RegisteredDriver`] rejected the connection.
    Rejected(D::ConnectError),
    /// The remote [`RegisteredDriver`] has been registered, but the service
    /// task has terminated.
    DriverDead,
}

/// Errors returned by [`Registry::connect_userspace_with_hello`]
pub enum UserConnectError<D: RegisteredDriver> {
    /// A connection error occurred: either the driver was not found in the
    /// registry, it was no longer running, or it rejected the connection.
    Connect(ConnectError<D>),
    /// Deserializing the userspace `Hello` message failed.
    DeserializationFailed(postcard::Error),
    /// The requested driver is not exposed.
    NotUserspace,
}

#[derive(Debug, Eq, PartialEq)]
pub enum OneshotRequestError {
    /// An error occurred while acquiring a sender.
    Sender(ReusableError),
    /// Sending the request failed.
    Send,
    /// An error occurred while receiving the response.
    Receive(ReusableError),
}

#[derive(Debug, Eq, PartialEq)]
pub enum SendError {
    /// The service on the other end of the [`KernelHandle`] has terminated!
    Closed,
}

impl From<ReusableError> for ReplyError {
    fn from(err: ReusableError) -> Self {
        match err {
            ReusableError::ChannelClosed => ReplyError::ReplyChannelClosed,
            _ => ReplyError::InternalError,
        }
    }
}

impl<T> From<EnqueueError<T>> for ReplyError {
    fn from(enq: EnqueueError<T>) -> Self {
        match enq {
            // Should not be possible with async calls
            EnqueueError::Full(_) => ReplyError::InternalError,
            EnqueueError::Closed(_) => ReplyError::ReplyChannelClosed,
        }
    }
}

/// A UserspaceHandle is used to process incoming serialized messages from
/// userspace. It contains a method that can be used to deserialize messages
/// from a given UUID, and send that request (if the deserialization is
/// successful) to a given driver service.
pub struct UserspaceHandle {
    req_producer_leaked: ErasedKProducer,
    req_deser: ErasedReqDeser,
    service_id: ServiceId,
    client_id: ClientId,
}

/// A KernelHandle is used to send typed messages to a kernelspace Driver
/// service.
pub struct KernelHandle<RD: RegisteredDriver> {
    prod: KProducer<Message<RD>>,
    service_id: ServiceId,
    client_id: ClientId,
    request_ctr: u32,
}

type ErasedReqDeser = unsafe fn(
    UserRequest<'_>,
    &ErasedKProducer,
    &bbq::MpscProducer,
    ServiceId,
    ClientId,
) -> Result<(), UserHandlerError>;

type ErasedHandshake = unsafe fn(
    &maitake::scheduler::LocalScheduler,
    &[u8],
    &ErasedKProducer,
    ptr::NonNull<()>,
) -> maitake::task::JoinHandle<Result<(), postcard::Error>>;

/// The payload of a registry item.
///
/// The typeid is stored here to allow the userspace handle to look up the UUID key
/// without knowing the proper typeid. Kernel space drivers should always check that the
/// tuple type id is correct.
struct RegistryValue {
    req_resp_tuple_id: TypeId,
    conn_prod: ErasedKProducer,
    user_vtable: Option<UserVtable>,
    service_id: ServiceId,
}

/// A [virtual function pointer table][vtable] (vtable) that specifies how
/// userspace requests are serialized and deserialized.
///
/// [vtable]: https://en.wikipedia.org/wiki/Virtual_method_table
struct UserVtable {
    /// Deserializes userspace requests.
    req_deser: ErasedReqDeser,
    /// Deserializes handshakes from userspace.
    handshake: ErasedHandshake,
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
        Self {
            items: FixedVec::try_new(max_items).unwrap(),
            counter: 0,
        }
    }

    /// Register a driver service ONLY for use in the kernel, including drivers.
    ///
    /// Driver services registered with [Registry::register_konly] can NOT be queried
    /// or interfaced with from Userspace. If a registered service has request
    /// and response types that are serializable, it can instead be registered
    /// with [Registry::register] which allows for userspace access.
    #[tracing::instrument(
        name = "Registry::register_konly",
        level = "debug",
        skip(self, registration),
        fields(svc = %any::type_name::<RD>()),
    )]
    pub fn register_konly<RD: RegisteredDriver>(
        &mut self,
        registration: listener::Registration<RD>,
    ) -> Result<(), RegistrationError> {
        if self.items.as_slice().iter().any(|i| i.key == RD::UUID) {
            return Err(RegistrationError::UuidAlreadyRegistered);
        }
        let conn_prod = registration.tx.type_erase();
        self.items
            .try_push(RegistryItem {
                key: RD::UUID,
                value: RegistryValue {
                    req_resp_tuple_id: RD::type_id().type_of(),
                    conn_prod,
                    user_vtable: None,
                    service_id: ServiceId(self.counter),
                },
            })
            .map_err(|_| RegistrationError::RegistryFull)?;
        info!(uuid = ?RD::UUID, service_id = self.counter, "Registered KOnly");
        self.counter = self.counter.wrapping_add(1);
        Ok(())
    }

    /// Register a driver service for use in the kernel (including drivers) as
    /// well as in userspace.
    ///
    /// See [Registry::register_konly] if the request and response types are not
    /// serializable.
    #[tracing::instrument(
        name = "Registry::register",
        level = "debug",
        skip(self, registration),
        fields(svc = %any::type_name::<RD>()),
    )]
    pub fn register<RD>(
        &mut self,
        registration: listener::Registration<RD>,
    ) -> Result<(), RegistrationError>
    where
        RD: RegisteredDriver + 'static,
        RD::Hello: Serialize + DeserializeOwned,
        RD::ConnectError: Serialize + DeserializeOwned,
        RD::Request: Serialize + DeserializeOwned,
        RD::Response: Serialize + DeserializeOwned,
    {
        if self.items.as_slice().iter().any(|i| i.key == RD::UUID) {
            return Err(RegistrationError::UuidAlreadyRegistered);
        }

        let conn_prod = registration.tx.type_erase();
        self.items
            .try_push(RegistryItem {
                key: RD::UUID,
                value: RegistryValue {
                    req_resp_tuple_id: RD::type_id().type_of(),
                    conn_prod,
                    user_vtable: Some(UserVtable::new::<RD>()),
                    service_id: ServiceId(self.counter),
                },
            })
            .map_err(|_| RegistrationError::RegistryFull)?;
        info!(svc = %any::type_name::<RD>(), uuid = ?RD::UUID, service_id = self.counter, "Registered");
        self.counter = self.counter.wrapping_add(1);
        Ok(())
    }

    /// Get a kernelspace (including drivers) handle of a given driver service,
    /// which does not require sending a [`RegisteredDriver::Hello`] message.
    ///
    /// This can be used by drivers and tasks to interface with a registered driver
    /// service.
    ///
    /// The driver service MUST have already been registered using [Registry::register] or
    /// [Registry::register_konly] prior to making this call, otherwise no handle will
    /// be returned.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[KernelHandle`]`)` if the requested service was found and
    ///   a connection was successfully established.
    ///
    /// - [`Err`]`(`[`ConnectError`]`)` if the requested service was not
    ///   found in the registry, or if the service [rejected] the incoming
    ///   connection.
    ///
    /// [rejected]: listener::Handshake::reject
    #[tracing::instrument(
        name = "Registry::connect_with_hello",
        level = "debug",
        skip(self, hello),
        fields(svc = %any::type_name::<RD>()),
    )]
    pub async fn connect_with_hello<RD: RegisteredDriver>(
        &mut self,
        hello: RD::Hello,
    ) -> Result<KernelHandle<RD>, ConnectError<RD>> {
        let item = Self::get(&self.items)?;

        // cast the erased connection sender back to a typed sender.
        let tx = unsafe {
            // Safety: we just checked that the type IDs match above.
            item.value
                .conn_prod
                .clone_typed::<listener::Handshake<RD>>()
        };

        // TODO(eliza): it would be nice if we could reuse the oneshot receiver
        // every time this driver is connected to? This would require type
        // erasing it...
        let rx = Reusable::new_async().await;
        let reply = rx
            .sender()
            .await
            .expect("we just created the oneshot, so this should never fail");
        // send the connection request...
        tx.enqueue_async(listener::Handshake {
            hello,
            accept: listener::Accept { reply }
        }).await.map_err(|err| match err {
            kchannel::EnqueueError::Closed(_) => ConnectError::DriverDead,
            kchannel::EnqueueError::Full(_) => unreachable!("the channel should not be full, as we are using `enqueue_async`, which waits for capacity")
        })?;
        // ...and wait for a response with an established connection.
        let prod = rx
            .receive()
            .await
            // this is a `Reusable<Result<KProducer, RD::ConnectError>>>`, so
            // the outer `Result` is the error returned by `receive()`...
            .map_err(|_| ConnectError::DriverDead)?
            // ...and the inner `Result` is the error returned by the driver.
            .map_err(ConnectError::Rejected)?;

        let res = Ok(KernelHandle {
            prod,
            service_id: item.value.service_id,
            client_id: ClientId(self.counter),
            request_ctr: 0,
        });
        info!(svc = %any::type_name::<RD>(), uuid = ?RD::UUID, service_id = item.value.service_id.0, client_id = self.counter, "Got KernelHandle from Registry");
        self.counter = self.counter.wrapping_add(1);
        res
    }

    /// Get a kernelspace (including drivers) handle of a given driver service,
    /// which does not require sending a [`RegisteredDriver::Hello`] message.
    ///
    /// This method is equivalent to [`Registry::connect_with_hello`] when the
    /// [`RegisteredDriver::Hello`] type is [`()`].
    ///
    /// This can be used by drivers and tasks to interface with a registered driver
    /// service.
    ///
    /// The driver service MUST have already been registered using [Registry::register] or
    /// [Registry::register_konly] prior to making this call, otherwise no handle will
    /// be returned.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[KernelHandle`]`)` if the requested service was found and
    ///   a connection was successfully established.
    ///
    /// - [`Err`]`(`[`ConnectError`]`)` if the requested service was not
    ///   found in the registry, or if the service [rejected] the incoming
    ///   connection.
    ///
    /// [rejected]: listener::Handshake::reject
    pub async fn connect<RD>(&mut self) -> Result<KernelHandle<RD>, ConnectError<RD>>
    where
        RD: RegisteredDriver<Hello = ()>,
    {
        self.connect_with_hello(()).await
    }

    /// Get a handle capable of processing serialized userspace messages to a
    /// registered driver service, given a byte buffer for the userspace
    /// [`RegisteredDriver::Hello`] message.
    ///
    /// The driver service MUST have already been registered using [Registry::register] or
    /// prior to making this call, otherwise no handle will be returned.
    ///
    /// Driver services registered with [Registry::register_konly] cannot be retrieved via
    /// a call to [Registry::connect_userspace_with_hello].
    #[tracing::instrument(
        name = "Registry::connect_userspace_with_hello",
        level = "debug",
        skip(self, scheduler),
        fields(svc = %any::type_name::<RD>()),
    )]
    pub async fn connect_userspace_with_hello<RD>(
        &mut self,
        scheduler: &maitake::scheduler::LocalScheduler,
        user_hello: &[u8],
    ) -> Result<UserspaceHandle, UserConnectError<RD>>
    where
        RD: RegisteredDriver,
        RD::Hello: Serialize + DeserializeOwned,
        RD::ConnectError: Serialize + DeserializeOwned,
        RD::Request: Serialize + DeserializeOwned,
        RD::Response: Serialize + DeserializeOwned,
    {
        let item = Self::get::<RD>(&self.items).map_err(UserConnectError::Connect)?;
        let vtable = item
            .value
            .user_vtable
            .as_ref()
            // if the registry item has no userspace vtable, it's not exposed to
            // userspace.
            // this is *weird*, since this method requires that `RD`'s message
            // types be serializable/deserializable, but it's possible that the
            // driver was (accidentally?) registered with `register_konly` even
            // though it didn't *need* to be due to serializability...
            .ok_or(UserConnectError::NotUserspace)?;

        let mut handshake_result = mem::MaybeUninit::<UserHandshakeResult<RD>>::uninit();
        let outptr = ptr::NonNull::from(&mut handshake_result).cast::<()>();

        let handshake =
            unsafe { (vtable.handshake)(scheduler, user_hello, &item.value.conn_prod, outptr) };
        let req_producer_leaked = match handshake.await {
            // Outer `Result` is the `JoinError` from `maitake` --- it should
            // always succeed, because we own the task's joinhandle, and we
            // never cancel it.
            Err(_) => unreachable!("handshake task should not be canceled"),
            // Couldn't deserialize the userspace handshake bytes!
            Ok(Err(error)) => {
                return Err(UserConnectError::DeserializationFailed(error));
            }
            // Safe to touch the out pointer!
            Ok(Ok(())) => unsafe {
                // Safety: `handshake_result` is guaranteed to be initialized by
                // `erased_handshake` if and only if its future completes with
                // an `Ok(())`. and it did!
                handshake_result
                    .assume_init()
                    .map_err(UserConnectError::Connect)?
                    .type_erase()
            },
        };

        let client_id = self.counter;
        info!(
            svc = %any::type_name::<RD>(),
            uuid = ?RD::UUID,
            service_id = item.value.service_id.0,
            client_id = self.counter,
            "Got KernelHandle from Registry",
        );
        self.counter = self.counter.wrapping_add(1);

        Ok(UserspaceHandle {
            req_producer_leaked,
            req_deser: vtable.req_deser,
            service_id: item.value.service_id,
            client_id: ClientId(client_id),
        })
    }

    // This isn't a method on `self` because it borrows `items`, and if it
    // borrowed `self`, it would also borrow `self.counter`, which we need to be
    // able to mutate while borrowing `self`.
    // TODO(eliza): could fix that by just making the counter atomic...
    fn get<RD: RegisteredDriver>(
        items: &FixedVec<RegistryItem>,
    ) -> Result<&RegistryItem, ConnectError<RD>> {
        let Some(item) = items.as_slice().iter().find(|i| i.key == RD::UUID) else {
            tracing::debug!(
                svc = %any::type_name::<RD>(),
                uuid = ?RD::UUID,
                "No service for this UUID exists in the registry!"
            );
            return Err(ConnectError::NotFound);
        };

        let expected_type_id = RD::type_id().type_of();
        let actual_type_id = item.value.req_resp_tuple_id;
        if expected_type_id != actual_type_id {
            tracing::warn!(
                svc = %any::type_name::<RD>(),
                uuid = ?RD::UUID,
                type_id.expected = ?expected_type_id,
                type_id.actual = ?actual_type_id,
                "Registry entry's type ID did not match driver's type ID. This is (probably?) a bug!"
            );
            return Err(ConnectError::NotFound);
        }

        Ok(item)
    }
}

// UserRequest

// Envelope

impl<P> OpenEnvelope<P> {
    pub fn fill(self, contents: P) -> Envelope<P> {
        Envelope {
            body: contents,
            service_id: self.service_id,
            client_id: self.client_id,
            request_id: self.request_id,
        }
    }
}

impl<P> Envelope<P> {
    // NOTE: proper types are constrained by [Message::split]
    fn split_reply<R>(self) -> (P, OpenEnvelope<R>) {
        let env = OpenEnvelope {
            body: PhantomData,
            service_id: self.service_id,
            client_id: self.client_id,
            request_id: RequestResponseId::new(self.request_id.id(), MessageKind::Response),
        };
        (self.body, env)
    }

    /// Create a response Envelope from a given request Envelope.
    ///
    /// Maintains the same Service ID and Client ID, and increments the
    /// request ID by one.
    pub fn reply_with<U>(&self, body: U) -> Envelope<U> {
        Envelope {
            body,
            service_id: self.service_id,
            client_id: self.client_id,
            request_id: RequestResponseId::new(self.request_id.id(), MessageKind::Response),
        }
    }

    /// Create a response Envelope from a given request Envelope.
    ///
    /// Maintains the same Service ID and Client ID, and increments the
    /// request ID by one.
    ///
    /// This variant also gives you the request body in case you need it for
    /// the response.
    pub fn reply_with_body<F, U>(self, f: F) -> Envelope<U>
    where
        F: FnOnce(P) -> U,
    {
        Envelope {
            service_id: self.service_id,
            client_id: self.client_id,
            request_id: RequestResponseId::new(self.request_id.id(), MessageKind::Response),
            body: f(self.body),
        }
    }
}

// Message

impl<RD: RegisteredDriver> Message<RD> {
    // Would adding type aliases really make this any better? Who cares.
    #[allow(clippy::type_complexity)]
    pub fn split(
        self,
    ) -> (
        RD::Request,
        OpenEnvelope<Result<RD::Response, RD::Error>>,
        ReplyTo<RD>,
    ) {
        let Self { msg, reply } = self;
        let (req, env) = msg.split_reply();
        (req, env, reply)
    }
}

// ReplyTo

impl<RD: RegisteredDriver> ReplyTo<RD> {
    pub async fn reply_konly(
        self,
        envelope: Envelope<Result<RD::Response, RD::Error>>,
    ) -> Result<(), ReplyError> {
        debug!(
            service_id = envelope.service_id.0,
            client_id = envelope.client_id.0,
            response_id = envelope.request_id.id(),
            "Replying KOnly",
        );
        match self {
            ReplyTo::KChannel(kprod) => {
                kprod.enqueue_async(envelope).await?;
            }
            ReplyTo::OneShot(sender) => {
                sender.send(envelope)?;
            }
            ReplyTo::Userspace { .. } => return Err(ReplyError::KOnlyUserspaceResponse),
        }
        Ok(())
    }
}

impl<RD: RegisteredDriver> ReplyTo<RD>
where
    RD::Response: Serialize + MaxSize,
    RD::Error: Serialize + MaxSize,
{
    pub async fn reply(
        self,
        uuid_source: Uuid,
        envelope: Envelope<Result<RD::Response, RD::Error>>,
    ) -> Result<(), ReplyError> {
        debug!(
            service_id = envelope.service_id.0,
            client_id = envelope.client_id.0,
            response_id = envelope.request_id.id(),
            "Replying",
        );
        match self {
            ReplyTo::KChannel(kprod) => {
                kprod.enqueue_async(envelope).await?;
                Ok(())
            }
            ReplyTo::OneShot(sender) => {
                sender.send(envelope)?;
                Ok(())
            }
            ReplyTo::Userspace { nonce, outgoing } => {
                let mut wgr = outgoing
                    .send_grant_exact(
                        <UserResponse<RD::Response, RD::Error> as MaxSize>::POSTCARD_MAX_SIZE,
                    )
                    .await;
                let used = postcard::to_slice(
                    &UserResponse {
                        uuid: uuid_source,
                        nonce,
                        reply: envelope.body,
                    },
                    &mut wgr,
                )
                .map_err(|_| ReplyError::UserspaceSerializationError)?;
                let len = used.len();
                wgr.commit(len);
                Ok(())
            }
        }
    }
}

// UserspaceHandle

impl UserspaceHandle {
    pub fn process_msg(
        &self,
        user_msg: UserRequest<'_>,
        user_ring: &bbq::MpscProducer,
    ) -> Result<(), UserHandlerError> {
        unsafe {
            (self.req_deser)(
                user_msg,
                &self.req_producer_leaked,
                user_ring,
                self.service_id,
                self.client_id,
            )
        }
    }
}

// KernelHandle

impl<RD: RegisteredDriver> KernelHandle<RD> {
    pub async fn send(&mut self, msg: RD::Request, reply: ReplyTo<RD>) -> Result<(), SendError> {
        let request_id = RequestResponseId::new(self.request_ctr, MessageKind::Request);
        self.request_ctr = self.request_ctr.wrapping_add(1);
        self.prod
            .enqueue_async(Message {
                msg: Envelope {
                    body: msg,
                    service_id: self.service_id,
                    client_id: self.client_id,
                    request_id,
                },
                reply,
            })
            .await
            .map_err(|_| SendError::Closed)?;
        debug!(
            service_id = self.service_id.0,
            client_id = self.client_id.0,
            request_id = request_id.id(),
            "Sent Request"
        );
        Ok(())
    }

    /// Send a [`ReplyTo::OneShot`] request using the provided [`Reusable`]
    /// oneshot channel, and await the response from that channel.
    pub async fn request_oneshot(
        &mut self,
        msg: RD::Request,
        reply: &Reusable<Envelope<Result<RD::Response, RD::Error>>>,
    ) -> Result<Envelope<Result<RD::Response, RD::Error>>, OneshotRequestError> {
        let tx = reply.sender().await.map_err(OneshotRequestError::Sender)?;
        self.send(msg, ReplyTo::OneShot(tx))
            .await
            .map_err(|_| OneshotRequestError::Send)?;
        reply.receive().await.map_err(OneshotRequestError::Receive)
    }
}

// UserVtable

impl UserVtable {
    const fn new<RD>() -> Self
    where
        RD: RegisteredDriver + 'static,
        RD::Hello: Serialize + DeserializeOwned,
        RD::ConnectError: Serialize + DeserializeOwned,
        RD::Request: Serialize + DeserializeOwned,
        RD::Response: Serialize + DeserializeOwned,
    {
        Self {
            req_deser: map_deser::<RD>,
            handshake: erased_user_handshake::<RD>,
        }
    }
}

/// A monomorphizable function that allows us to store the serialization type within
/// the function itself, allowing for a type-erased function pointer to be stored
/// inside of the registry.
///
/// SAFETY:
///
/// This function MUST be called with a `RegisteredDriver` type matching the type
/// used to create the `ErasedKProducer`.
unsafe fn map_deser<RD>(
    umsg: UserRequest<'_>,
    req_tx: &ErasedKProducer,
    user_resp: &bbq::MpscProducer,
    service_id: ServiceId,
    client_id: ClientId,
) -> Result<(), UserHandlerError>
where
    RD: RegisteredDriver,
    RD::Request: Serialize + DeserializeOwned,
    RD::Response: Serialize + DeserializeOwned,
{
    // Un-type-erase the producer channel
    //
    // TODO: We don't really need to clone the producer, we just need a reference valid
    // for the lifetime of `req_tx`. Consider adding a method for this before merging
    // https://github.com/tosc-rs/mnemos/pull/25.
    //
    // This PROBABLY would require a "with"/closure method to make sure the producer ref
    // doesn't outlive the LeakedKProducer reference.
    let req_prod = req_tx.clone_typed::<Message<RD>>();

    // Deserialize the request, if it doesn't have the right contents, deserialization will fail.
    let u_payload: RD::Request = postcard::from_bytes(umsg.req_bytes)
        .map_err(|_| UserHandlerError::DeserializationFailed)?;

    // Create the message type to be sent on the channel
    let msg: Message<RD> = Message {
        msg: Envelope {
            body: u_payload,
            service_id,
            client_id,
            request_id: RequestResponseId::new(umsg.nonce, MessageKind::Request),
        },
        reply: ReplyTo::Userspace {
            nonce: umsg.nonce,
            outgoing: user_resp.clone(),
        },
    };

    // Send the message, and report any failures
    req_prod
        .enqueue_sync(msg)
        .map_err(|_| UserHandlerError::QueueFull)
}

type UserHandshakeResult<RD> = Result<KProducer<Message<RD>>, ConnectError<RD>>;

/// Perform a type-erased userspace handshake, deserializing the
/// [`RegisteredDriver::Hello`] message from `hello_bytes` and returning a
/// future that writes the handshake result to the provided `outptr`, if the
/// future completes successfully.
///
/// # Safety
///
/// - This function MUST be called with a [`RegisteredDriver`] type matching the
///   type used to create the [`ErasedKProducer`].
/// - `outptr` MUST be a valid pointer to a
///   [`mem::MaybeUninit`]`<`[`UserHandshakeResult`]`<RD>>`, and  MUST live as
///   long as the future returned from this function.
/// - `outptr` is guaranteed to be initialized IF AND ONLY IF the future
///   returned by this method returns [`Ok`]`(())`. If this method returns an
///   [`Err`], `outptr` will NOT be initialized.
unsafe fn erased_user_handshake<RD>(
    scheduler: &maitake::scheduler::LocalScheduler,
    hello_bytes: &[u8],
    conn_tx: &ErasedKProducer,
    outptr: core::ptr::NonNull<()>,
) -> maitake::task::JoinHandle<Result<(), postcard::Error>>
where
    RD: RegisteredDriver + 'static,
    RD::Hello: Serialize + DeserializeOwned,
    RD::ConnectError: Serialize + DeserializeOwned,
    RD::Request: Serialize + DeserializeOwned,
    RD::Response: Serialize + DeserializeOwned,
{
    let conn_tx = conn_tx.clone_typed::<listener::Handshake<RD>>();
    // Deserialize the request, if it doesn't have the right contents, deserialization will fail.
    let hello: Result<RD::Hello, _> = postcard::from_bytes(hello_bytes);

    // spawn a task to allow us to perform async work from a type-erased context
    scheduler.spawn(async move {
        let hello = hello?;

        // TODO(eliza): it would be nice if we could reuse the oneshot receiver
        // every time this driver is connected to? This would require type
        // erasing it...
        let rx = Reusable::new_async().await;
        let reply = rx
            .sender()
            .await
            .expect("we just created the oneshot, so this should never fail");

        // send the connection request...
        conn_tx.enqueue_async(listener::Handshake {
            hello,
            accept: listener::Accept { reply }
        }).await.map_err(|err| match err {
            kchannel::EnqueueError::Closed(_) => todo!(),
            kchannel::EnqueueError::Full(_) => unreachable!("the channel should not be full, as we are using `enqueue_async`, which waits for capacity")
        })?;

        // ...and wait for a response with an established connection.
        let result = rx
            .receive()
            .await
            // this is a `Reusable<Result<KProducer, RD::ConnectError>>>`, so
            // the outer `Result` is the error returned by `receive()`...
            .map_err(|_| ConnectError::DriverDead)
            // ...and the inner result is the connect error returned by the service.
            .and_then(|res| res.map_err(ConnectError::Rejected));

        outptr
            // Safety: the caller is responsible for ensuring the out pointer is
            // correctly typed.
            .cast::<mem::MaybeUninit<UserHandshakeResult<RD>>>()
            .as_mut()
            .write(result);

        Ok(())
    })
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
    D: RegisteredDriver,
    D::ConnectError: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::DriverDead, Self::DriverDead) => true,
            (Self::NotFound, Self::NotFound) => true,
            (Self::Rejected(this), Self::Rejected(that)) => this == that,
            _ => false,
        }
    }
}

impl<D> Eq for ConnectError<D>
where
    D: RegisteredDriver,
    D::ConnectError: Eq,
{
}

impl<D> fmt::Debug for ConnectError<D>
where
    D: RegisteredDriver,
    D::ConnectError: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbs = match self {
            Self::DriverDead => f.debug_struct("DriverDead"),
            Self::NotFound => f.debug_struct("NotFound"),
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
    D: RegisteredDriver,
    D::ConnectError: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = any::type_name::<D>();
        match self {
            Self::DriverDead => write!(f, "the {name} service has terminated"),
            Self::NotFound => write!(f, "no {name} service found in the registry",),
            Self::Rejected(err) => write!(f, "the {name} service rejected the connection: {err}",),
        }
    }
}

// UserConnectError

impl<D> PartialEq for UserConnectError<D>
where
    D: RegisteredDriver,
    D::ConnectError: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::DeserializationFailed(this), Self::DeserializationFailed(that)) => this == that,
            (Self::Connect(this), Self::Connect(that)) => this == that,
            (Self::NotUserspace, Self::NotUserspace) => true,
            _ => false,
        }
    }
}

impl<D> Eq for UserConnectError<D>
where
    D: RegisteredDriver,
    D::ConnectError: Eq,
{
}

impl<D> fmt::Debug for UserConnectError<D>
where
    D: RegisteredDriver,
    D::ConnectError: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeserializationFailed(error) => f
                .debug_struct("DeserializationFailed")
                .field("error", error)
                .field("svc", &mycelium_util::fmt::display(any::type_name::<D>()))
                .finish(),
            Self::Connect(err) => f.debug_tuple("Connect").field(err).finish(),
            Self::NotUserspace => f
                .debug_tuple("NotUserspace")
                .field(&mycelium_util::fmt::display(any::type_name::<D>()))
                .finish(),
        }
    }
}

impl<D> fmt::Display for UserConnectError<D>
where
    D: RegisteredDriver,
    D::ConnectError: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(err) => write!(f, "failed to connect from userspace: {err}"),
            Self::DeserializationFailed(err) => write!(
                f,
                "failed to deserialize userspace Hello for the {} service: {err}",
                any::type_name::<D>()
            ),
            Self::NotUserspace => write!(
                f,
                "the {} service is not exposed to userspace",
                any::type_name::<D>()
            ),
        }
    }
}
