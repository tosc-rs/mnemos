#![cfg_attr(not(feature = "std"), no_std)]

use tracing_serde_structured::{SerializeAttributes, SerializeEvent, SerializeId};

#[derive(serde::Serialize, serde::Deserialize)]
pub enum TraceEvent<'a> {
    #[serde(borrow)]
    Event(SerializeEvent<'a>),

    NewSpan {
        id: SerializeId,

        #[serde(borrow)]
        attributes: SerializeAttributes<'a>,
    },

    Enter(SerializeId),
    Exit(SerializeId),
    CloneSpan(SerializeId),
    DropSpan(SerializeId),
}
