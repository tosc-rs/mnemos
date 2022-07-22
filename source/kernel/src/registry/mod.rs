use core::any::TypeId;

use mnemos_alloc::{containers::HeapFixedVec, heap::HeapGuard};
use postcard::experimental::max_size::MaxSize;
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use uuid::{uuid, Uuid};

use crate::comms::{
    bbq,
    kchannel::{KProducer, LeakedKProducer}, rosc::Sender,
};

type ErasedDeserHandler = unsafe fn(UserRequest<'_>, &LeakedKProducer, &bbq::MpscProducer) -> Result<(), ()>;

pub struct Registry {
    items: HeapFixedVec<RegistryItem>,
}

impl Registry {
    pub fn new(guard: &mut HeapGuard, max_items: usize) -> Self {
        Self {
            items: guard.alloc_fixed_vec(max_items).map_err(drop).unwrap()
        }
    }
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
struct UserResponse<U> {
    // TODO: Maybe not the UUID, maybe pre-discover a shorter UID?
    uuid: Uuid,
    nonce: u32,
    reply: Result<U, ()>,
}

impl<U: MaxSize> MaxSize for UserResponse<U> {
    const POSTCARD_MAX_SIZE: usize = {
        <[u8; 16] as MaxSize>::POSTCARD_MAX_SIZE +
        <u32 as MaxSize>::POSTCARD_MAX_SIZE +
        <Result<U, ()> as MaxSize>::POSTCARD_MAX_SIZE
    };
}

struct RegistryValue {
    req_resp_tuple_id: TypeId,
    req_prod_leaked: LeakedKProducer,
    req_deser: Option<ErasedDeserHandler>,
}

unsafe fn map_deser<RD>(
    umsg: UserRequest<'_>,
    req_tx: &LeakedKProducer,
    user_resp: &bbq::MpscProducer,
) -> Result<(), ()>
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
    let u_payload: RD::Request = postcard::from_bytes(umsg.req_bytes).map_err(drop)?;

    // Create the message type to be sent on the channel
    let msg: Message<RD> = Message {
        msg: HMessage { body: u_payload },
        reply: ReplyTo::Userspace {
            nonce: umsg.nonce,
            outgoing: user_resp.clone(),
        },
    };

    // Send the message, and report any failures
    req_prod.enqueue_sync(msg).map_err(drop)
}

struct RegistryItem {
    key: Uuid,
    value: RegistryValue,
}

pub struct HMessage<P> {
    pub body: P,
}

pub struct Message<RD: RegisteredDriver> {
    pub msg: HMessage<RD::Request>,
    pub reply: ReplyTo<RD::Response>,
}

pub enum ReplyTo<U> {
    // This can be used to reply directly to another kernel entity,
    // without a serialization step
    KChannel(KProducer<HMessage<Result<U, ()>>>),

    Rosc(Sender<HMessage<Result<U, ()>>>),

    // This can be used to reply to userspace. Responses are serialized
    // and sent over the bbq::MpscProducer
    Userspace {
        nonce: u32,
        outgoing: bbq::MpscProducer,
    },
}

impl<U> ReplyTo<U> {
    pub async fn reply_konly(self, payload: Result<U, ()>) -> Result<(), ()> {
        let hmsg = HMessage { body: payload };
        match self {
            ReplyTo::KChannel(kprod) => kprod.enqueue_async(hmsg).await.map_err(drop),
            ReplyTo::Rosc(sender) => sender.send(hmsg),
            ReplyTo::Userspace { .. } => Err(()),
        }
    }
}

impl<U> ReplyTo<U>
where
    U: Serialize + MaxSize
{
    pub async fn reply(self, uuid_source: Uuid, payload: Result<U, ()>) -> Result<(), ()> {
        match self {
            ReplyTo::KChannel(kprod) => {
                let hmsg = HMessage { body: payload };
                kprod.enqueue_async(hmsg).await.map_err(drop)
            }
            ReplyTo::Rosc(sender) => {
                let hmsg = HMessage { body: payload };
                sender.send(hmsg)
            }
            ReplyTo::Userspace { nonce, outgoing } => {
                let mut wgr = outgoing.send_grant_exact(<UserResponse<U> as MaxSize>::POSTCARD_MAX_SIZE).await;
                let used = postcard::to_slice(&UserResponse { uuid: uuid_source, nonce, reply: payload }, &mut wgr).map_err(drop)?;
                let len = used.len();
                wgr.commit(len);
                Ok(())
            },
        }
    }
}

pub struct UserspaceHandle {
    req_producer_leaked: LeakedKProducer,
    req_deser: ErasedDeserHandler,
}

impl UserspaceHandle {
    pub fn process_msg(
        &self,
        user_msg: UserRequest<'_>,
        user_ring: &bbq::MpscProducer,
    ) -> Result<(), ()> {
        unsafe {
            (self.req_deser)(user_msg, &self.req_producer_leaked, user_ring)
        }
    }
}

pub struct KernelHandle<RD: RegisteredDriver> {
    prod: KProducer<Message<RD>>,
}

impl<RD: RegisteredDriver> KernelHandle<RD> {
    pub async fn send(&self, msg: RD::Request, reply: ReplyTo<RD::Response>) -> Result<(), ()> {
        self.prod
            .enqueue_async(Message { msg: HMessage { body: msg }, reply })
            .await
            .map_err(drop)
    }
}

impl Registry {
    pub fn set_konly<RD: RegisteredDriver>(
        &mut self,
        kch: &KProducer<Message<RD>>,
    ) -> Result<(), ()> {
        if self.items.iter().any(|i| i.key == RD::UUID) {
            return Err(());
        }
        self.items.push(RegistryItem {
            key: RD::UUID,
            value: RegistryValue {
                req_resp_tuple_id: RD::type_id(),
                req_prod_leaked: kch.clone().leak_erased(),
                req_deser: None,
            },
        }).map_err(drop)
    }

    pub fn set<RD>(&mut self, kch: &KProducer<Message<RD>>) -> Result<(), ()>
    where
        RD: RegisteredDriver,
        RD::Request: Serialize + DeserializeOwned,
        RD::Response: Serialize + DeserializeOwned,
    {
        if self.items.iter().any(|i| i.key == RD::UUID) {
            return Err(());
        }
        self.items.push(RegistryItem {
            key: RD::UUID,
            value: RegistryValue {
                req_resp_tuple_id: RD::type_id(),
                req_prod_leaked: kch.clone().leak_erased(),
                req_deser: Some(map_deser::<RD>),
            },
        }).map_err(drop)
    }

    pub fn get<RD: RegisteredDriver>(&self) -> Option<KernelHandle<RD>> {
        let item = self.items.iter().find(|i| i.key == RD::UUID)?;
        if item.value.req_resp_tuple_id != RD::type_id() {
            return None;
        }
        unsafe {
            Some(KernelHandle {
                prod: item.value.req_prod_leaked.clone_typed(),
            })
        }
    }

    pub fn get_userspace<RD>(&mut self) -> Option<UserspaceHandle>
    where
        RD: RegisteredDriver,
        RD::Request: Serialize + DeserializeOwned,
        RD::Response: Serialize + DeserializeOwned,
    {
        let item = self.items.iter().find(|i| &i.key == &RD::UUID)?;
        Some(UserspaceHandle {
            req_producer_leaked: item.value.req_prod_leaked.clone(),
            req_deser: item.value.req_deser?,
        })
    }
}

pub trait RegisteredDriver {
    type Request: 'static;
    type Response: 'static;
    const UUID: Uuid;

    fn type_id() -> TypeId {
        TypeId::of::<(Self::Request, Self::Response)>()
    }
}

pub static KNOWN_UUIDS: &[Uuid] = &[
    //
    // Kernel UUIDs
    //

    // SerialMux
    uuid!("54c983fa-736f-4223-b90d-c4360a308647"),


    //
    // Simulator UUIDs
    //

    // Serial Port over TCP
    uuid!("f06aac01-2773-4266-8681-583ffe756554"),
];

pub mod simple_serial {
    use crate::Kernel;
    use crate::comms::bbq::BidiHandle;
    use crate::comms::rosc::Rosc;
    use super::*;

    use super::RegisteredDriver;

    pub struct SimpleSerial {
        kprod: KernelHandle<SimpleSerial>,
        rosc: Rosc<HMessage<Result<Response, ()>>>,
    }

    impl SimpleSerial {
        pub async fn get_registry(kernel: &'static Kernel) -> Option<Self> {
            let kprod = kernel.with_registry(|reg| {
                reg.get::<SimpleSerial>()
            }).await?;

            Some(SimpleSerial {
                kprod,
                rosc: Rosc::new_async(kernel).await,
            })
        }

        pub async fn get_port(&self) -> Option<BidiHandle> {
            self.kprod.send(Request::GetPort, ReplyTo::Rosc(self.rosc.sender().ok()?)).await.ok()?;
            let resp = self.rosc.receive().await.ok()?;

            let Response::PortHandle { handle } = resp.body.ok()?;
            Some(handle)
        }
    }

    impl RegisteredDriver for SimpleSerial {
        type Request = Request;
        type Response = Response;

        const UUID: Uuid = uuid!("f06aac01-2773-4266-8681-583ffe756554");
    }

    pub enum Request {
        GetPort,
    }

    pub enum Response {
        PortHandle { handle: BidiHandle },
    }
}
