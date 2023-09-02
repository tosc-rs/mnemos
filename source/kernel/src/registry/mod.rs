use core::{
    any::{self, TypeId},
    fmt,
    marker::PhantomData,
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

pub use self::listener::Listener;

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
/// Typically used with [Registry::register] or [Registry::register_konly].
/// Can typically be retrieved by [Registry::get] or [Registry::get_userspace]
/// After the service has been registered.
pub trait RegisteredDriver {
    /// This is the type of the request sent TO the driver service
    type Request: 'static;

    /// This is the type of a SUCCESSFUL response sent FROM the driver service
    type Response: 'static;

    /// This is the type of an UNSUCCESSFUL response sent FROM the driver service
    type Error: 'static;

    // XXX(eliza): ideally, we could default `Hello` to () and `ConnectError` to
    // `Infallible`...do we want to do that? it requires a nightly feature.
    type Hello: 'static;

    type ConnectError: 'static;

    /// This is the UUID of the driver service
    const UUID: Uuid;

    /// Get the type_id used to make sure that driver instances are correctly typed.
    /// Corresponds to the same type ID as `(Self::Request, Self::Response, Self::Error)`
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

pub enum ConnectError<D: RegisteredDriver> {
    /// No [`RegisteredDriver`] of this type was found!
    NotFound,
    /// The remote [`RegisteredDriver`] rejected the connection.
    Rejected(D::ConnectError),
    /// The remote [`RegisteredDriver`] has been registered, but the service
    /// task has terminated.
    DriverDead,
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
    req_deser: ErasedDeserHandler,
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

type ErasedDeserHandler = unsafe fn(
    UserRequest<'_>,
    &ErasedKProducer,
    &bbq::MpscProducer,
    ServiceId,
    ClientId,
) -> Result<(), UserHandlerError>;

/// The payload of a registry item.
///
/// The typeid is stored here to allow the userspace handle to look up the UUID key
/// without knowing the proper typeid. Kernel space drivers should always check that the
/// tuple type id is correct.
struct RegistryValue {
    req_resp_tuple_id: TypeId,
    conn_prod: ErasedKProducer,
    req_deser: Option<ErasedDeserHandler>,
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
        fields(uuid = ?RD::UUID),
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
                    req_deser: None,
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
        fields(uuid = ?RD::UUID),
    )]
    pub fn register<RD>(
        &mut self,
        registration: listener::Registration<RD>,
    ) -> Result<(), RegistrationError>
    where
        RD: RegisteredDriver,
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
                    req_deser: Some(map_deser::<RD>),
                    service_id: ServiceId(self.counter),
                },
            })
            .map_err(|_| RegistrationError::RegistryFull)?;
        info!(uuid = ?RD::UUID, service_id = self.counter, "Registered");
        self.counter = self.counter.wrapping_add(1);
        Ok(())
    }

    #[tracing::instrument(
        name = "Registry::get_with_hello",
        level = "debug",
        skip(self, hello),
        fields(uuid = ?RD::UUID),
    )]
    pub async fn get_with_hello<RD: RegisteredDriver>(
        &mut self,
        hello: RD::Hello,
    ) -> Result<KernelHandle<RD>, ConnectError<RD>> {
        let item = self
            .items
            .as_slice()
            .iter()
            .find(|i| i.key == RD::UUID)
            .ok_or(ConnectError::NotFound)?;
        if item.value.req_resp_tuple_id != RD::type_id().type_of() {
            return Err(ConnectError::NotFound);
        }

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
        info!(uuid = ?RD::UUID, service_id = item.value.service_id.0, client_id = self.counter, "Got KernelHandle from Registry");
        self.counter = self.counter.wrapping_add(1);
        res
    }

    /// Get a kernelspace (including drivers) handle of a given driver service.
    ///
    /// This can be used by drivers and tasks to interface with a registered driver
    /// service.
    ///
    /// The driver service MUST have already been registered using [Registry::register] or
    /// [Registry::register_konly] prior to making this call, otherwise no handle will
    /// be returned.
    #[tracing::instrument(
        name = "Registry::get",
        level = "debug",
        skip(self),
        fields(uuid = ?RD::UUID),
    )]
    pub async fn get<RD>(&mut self) -> Result<KernelHandle<RD>, ConnectError<RD>>
    where
        RD: RegisteredDriver<Hello = ()>,
    {
        self.get_with_hello(()).await
    }

    /// Get a handle capable of processing serialized userspace messages to a
    /// registered driver service.
    ///
    /// The driver service MUST have already been registered using [Registry::register] or
    /// prior to making this call, otherwise no handle will be returned.
    ///
    /// Driver services registered with [Registry::register_konly] cannot be retrieved via
    /// a call to [Registry::get_userspace].
    #[tracing::instrument(
        name = "Registry::get_userspace",
        level = "debug",
        skip(self),
        fields(uuid = ?RD::UUID),
    )]
    pub fn get_userspace<RD>(&mut self) -> Option<UserspaceHandle>
    where
        RD: RegisteredDriver,
        RD::Request: Serialize + DeserializeOwned,
        RD::Response: Serialize + DeserializeOwned,
    {
        todo!("eliza: make this work with the new listener/accept design")
        // let item = self.items.as_slice().iter().find(|i| i.key == RD::UUID)?;
        // let client_id = self.counter;
        // info!(uuid = ?RD::UUID, service_id = item.value.service_id.0, client_id = self.counter, "Got KernelHandle from Registry");
        // self.counter = self.counter.wrapping_add(1);
        // Some(UserspaceHandle {
        //     req_producer_leaked: item.value.req_prod.clone(),
        //     req_deser: item.value.req_deser?,
        //     service_id: item.value.service_id,
        //     client_id: ClientId(client_id),
        // })
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

// -- other --

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
        match self {
            Self::DriverDead => {
                write!(f, "ConnectError::<{}>::DriverDead", any::type_name::<D>())
            }
            Self::NotFound => write!(f, "ConnectError::<{}>::NotFound", any::type_name::<D>()),
            Self::Rejected(err) => write!(
                f,
                "ConnectError::<{}>::Rejected({err:?})",
                any::type_name::<D>()
            ),
        }
    }
}

impl<D> fmt::Display for ConnectError<D>
where
    D: RegisteredDriver,
    D::ConnectError: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DriverDead => write!(f, "the {} service has terminated", any::type_name::<D>()),
            Self::NotFound => write!(
                f,
                "no {} service found in the registry",
                any::type_name::<D>()
            ),
            Self::Rejected(err) => write!(
                f,
                "the {} service rejected the connection: {err}",
                any::type_name::<D>()
            ),
        }
    }
}
