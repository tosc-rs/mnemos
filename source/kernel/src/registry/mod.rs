use core::{marker::PhantomData, any::TypeId};

use spitebuf::MpMcQueue;
use serde::{Serialize, Deserialize, de::DeserializeOwned};

use crate::comms::{
    bbq,
    kchannel::{KProducer, LeakedKProducer},
};

type ErasedDeserHandler = unsafe fn(UserRequest<'_>, &LeakedKProducer, &bbq::MpscProducer) -> Result<(), ()>;

// TODO: This probably goes into the ABI crate, here is fine for now
#[derive(Serialize, Deserialize)]
struct UserRequest<'a> {
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

#[derive(Serialize, Deserialize)]
struct Uuid {
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
    let req_prod = req_tx.clone_leaked::<Message<T, U>>();

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

}

pub struct HMessage<P> {
    body: P,
}

pub struct Message<T, U> {
    msg: HMessage<T>,
    reply: ReplyTo<U>,
}

enum ReplyTo<U> {
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
