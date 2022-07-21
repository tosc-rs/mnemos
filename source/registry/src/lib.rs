#![no_std]

use core::any::TypeId;
pub use uuid::{uuid, Uuid};

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
