use core::any::TypeId;

use mnemos_alloc::containers::HeapFixedVec;
use serde::{Serialize, Deserialize, de::DeserializeOwned};

use crate::comms::{
    bbq,
    kchannel::{KProducer, LeakedKProducer},
};

type ErasedDeserHandler = unsafe fn(UserRequest<'_>, &LeakedKProducer, &bbq::MpscProducer) -> Result<(), ()>;

pub struct Registry {
    items: HeapFixedVec<RegistryItem>,
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
    uid: Uuid,
    nonce: u32,
    reply: Result<U, ()>,
}

#[derive(Serialize, Deserialize, Eq, PartialEq)]
pub struct Uuid {
    inner: u128,
}

struct RegistryValue {
    req_resp_tuple_id: TypeId,
    req_prod_leaked: LeakedKProducer,
    req_deser: Option<ErasedDeserHandler>,
}

unsafe fn map_deser<T, U>(
    umsg: UserRequest<'_>,
    req_tx: &LeakedKProducer,
    user_resp: &bbq::MpscProducer,
) -> Result<(), ()>
where
    T: Serialize + DeserializeOwned + 'static,
    U: Serialize + DeserializeOwned + 'static,
{
    // Un-type-erase the producer channel
    //
    // TODO: We don't really need to clone the producer, we just need a reference valid
    // for the lifetime of `req_tx`. Consider adding a method for this before merging
    // https://github.com/tosc-rs/mnemos/pull/25.
    //
    // This PROBABLY would require a "with"/closure method to make sure the producer ref
    // doesn't outlive the LeakedKProducer reference.
    let req_prod = req_tx.clone_typed::<Message<T, U>>();

    // Deserialize the request, if it doesn't have the right contents, deserialization will fail.
    let u_payload: T = postcard::from_bytes(umsg.req_bytes).map_err(drop)?;

    // Create the message type to be sent on the channel
    let msg: Message<T, U> = Message {
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

pub struct Message<T, U> {
    pub msg: HMessage<T>,
    pub reply: ReplyTo<U>,
}

pub enum ReplyTo<U> {
    // This can be used to reply directly to another kernel entity,
    // without a serialization step
    Kernel(KProducer<HMessage<U>>),

    // This can be used to reply to userspace. Responses are serialized
    // and sent over the bbq::MpscProducer
    Userspace {
        nonce: u32,
        outgoing: bbq::MpscProducer,
    },
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

pub struct KernelHandle<T, U> {
    prod: KProducer<Message<T, U>>,
}

impl<T, U> KernelHandle<T, U> {
    pub async fn send(&self, msg: T, reply: ReplyTo<U>) -> Result<(), ()> {
        self.prod
            .enqueue_async(Message { msg: HMessage { body: msg }, reply })
            .await
            .map_err(drop)
    }
}

impl Registry {
    pub fn set_konly<T: 'static, U: 'static>(
        &mut self,
        uuid: Uuid,
        kch: &KProducer<Message<T, U>>,
    ) -> Result<(), ()> {
        if self.items.iter().any(|i| i.key == uuid) {
            return Err(());
        }
        self.items.push(RegistryItem {
            key: uuid,
            value: RegistryValue {
                req_resp_tuple_id: TypeId::of::<(T, U)>(),
                req_prod_leaked: kch.clone().leak_erased(),
                req_deser: None,
            },
        }).map_err(drop)
    }

    pub fn set<T, U>(&mut self, uuid: Uuid, kch: &KProducer<Message<T, U>>) -> Result<(), ()>
    where
        T: Serialize + DeserializeOwned + 'static,
        U: Serialize + DeserializeOwned + 'static,
    {
        if self.items.iter().any(|i| i.key == uuid) {
            return Err(());
        }
        self.items.push(RegistryItem {
            key: uuid,
            value: RegistryValue {
                req_resp_tuple_id: TypeId::of::<(T, U)>(),
                req_prod_leaked: kch.clone().leak_erased(),
                req_deser: Some(map_deser::<T, U>),
            },
        }).map_err(drop)
    }

    pub fn get<T: 'static, U: 'static>(&self, uuid: &Uuid) -> Option<KernelHandle<T, U>> {
        let item = self.items.iter().find(|i| &i.key == uuid)?;
        if item.value.req_resp_tuple_id != TypeId::of::<(T, U)>() {
            return None;
        }
        unsafe {
            Some(KernelHandle {
                prod: item.value.req_prod_leaked.clone_typed(),
            })
        }
    }

    pub fn get_userspace(&mut self, uuid: &Uuid) -> Option<UserspaceHandle>
    {
        let item = self.items.iter().find(|i| &i.key == uuid)?;
        Some(UserspaceHandle {
            req_producer_leaked: item.value.req_prod_leaked.clone(),
            req_deser: item.value.req_deser?,
        })
    }
}
