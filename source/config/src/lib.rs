#![cfg_attr(not(any(feature = "use-std", test)), no_std)]

use core::marker::PhantomData;

use serde::{Deserialize, Serialize};

pub struct MnemosConfig<'a, K, P>
where
    K: Serialize + Deserialize<'a>,
    P: Serialize + Deserialize<'a>,
{
    kernel_cfg: K,
    platform_cfg: P,
    _plt: PhantomData<&'a ()>,
}
