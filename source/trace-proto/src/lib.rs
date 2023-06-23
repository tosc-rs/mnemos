#![cfg_attr(not(feature = "std"), no_std)]

use core::{fmt, num::NonZeroU64};
use tracing_serde_structured::{
    SerializeId, SerializeMetadata, SerializeRecordFields, SerializeSpanFields,
};

#[derive(serde::Serialize, serde::Deserialize)]
pub enum TraceEvent<'a> {
    RegisterMeta {
        id: MetaId,

        #[serde(borrow)]
        meta: SerializeMetadata<'a>,
    },

    Event {
        parent: Option<SerializeId>,
        #[serde(borrow)]
        fields: SerializeRecordFields<'a>,
        meta: MetaId,
    },

    NewSpan {
        id: SerializeId,
        meta: MetaId,
        parent: Option<SerializeId>,
        is_root: bool,
        #[serde(borrow)]
        fields: SerializeSpanFields<'a>,
    },

    Enter(SerializeId),
    Exit(SerializeId),
    CloneSpan(SerializeId),
    DropSpan(SerializeId),
}

#[derive(Copy, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MetaId(NonZeroU64);

impl From<tracing_core::callsite::Identifier> for MetaId {
    fn from(id: tracing_core::callsite::Identifier) -> Self {
        Self(NonZeroU64::new(id.0 as *const _ as *const () as u64).expect("non-zero"))
    }
}

impl fmt::Debug for MetaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MetaId({:x})", self.0)
    }
}

impl fmt::Display for MetaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}", self.0)
    }
}
