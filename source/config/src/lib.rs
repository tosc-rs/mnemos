#![cfg_attr(not(any(feature = "use-std", test)), no_std)]

use core::marker::PhantomData;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct MnemosConfig<K, P> {
    pub kernel_cfg: K,
    pub platform_cfg: P,
}

#[cfg(feature = "use-std")]
pub mod buildtime {
    use serde::de::DeserializeOwned;

    use super::*;

    pub fn from_toml<K, P>(s: &str) -> Result<MnemosConfig<K, P>, ()>
    where
        K: DeserializeOwned + 'static,
        P: DeserializeOwned + 'static,
    {
        Ok(toml::from_str(s).unwrap())
    }

    pub fn to_postcard<K, P>(mc: &MnemosConfig<K, P>) -> Result<Vec<u8>, ()>
    where
        K: Serialize,
        P: Serialize,
    {
        postcard::to_stdvec(&mc).map_err(drop)
    }
}

// A fancy container for owned or borrowed Strings
//
// So, sometimes you want to use owned types, and have the std
// library. And other times you don't, and borrowed types are
// okay. This handles both cases, based on a feature flag.
//
// Inspired by @whitequark's `managed` crate.
// Stolen again from postcard-infomem
// Stolen again from soupstone

use core::fmt::Debug;

#[cfg(feature = "use-std")]
use std::{string::String, vec::Vec};

#[derive(Clone)]
pub enum ManagedBytes<'a> {
    /// Borrowed variant.
    Borrowed(&'a [u8]),
    #[cfg(feature = "use-std")]
    /// Owned variant, only available with the std or alloc feature enabled.
    Owned(Vec<u8>),
}

impl<'a> ManagedBytes<'a> {
    /// Create an Managed from a borrowed slice
    pub fn from_borrowed(s: &'a [u8]) -> Self {
        ManagedBytes::Borrowed(s)
    }

    /// Create an Managed from an owned Vec
    #[cfg(feature = "use-std")]
    pub fn from_vec(v: Vec<u8>) -> ManagedBytes<'static> {
        ManagedBytes::Owned(v)
    }

    /// View the Managed as a slice
    pub fn as_slice(&'a self) -> &'a [u8] {
        match self {
            ManagedBytes::Borrowed(s) => s,
            #[cfg(feature = "use-std")]
            ManagedBytes::Owned(s) => s.as_slice(),
        }
    }

    #[cfg(feature = "use-std")]
    pub fn to_owned(&'a self) -> ManagedBytes<'static> {
        match self {
            ManagedBytes::Borrowed(b) => ManagedBytes::Owned(b.to_vec()),
            ManagedBytes::Owned(s) => ManagedBytes::Owned(s.clone()),
        }
    }
}

// Optional impls

#[cfg(feature = "use-std")]
impl From<Vec<u8>> for ManagedBytes<'static> {
    fn from(s: Vec<u8>) -> Self {
        ManagedBytes::Owned(s)
    }
}

#[cfg(feature = "use-std")]
impl From<ManagedBytes<'static>> for Vec<u8> {
    fn from(is: ManagedBytes<'static>) -> Self {
        match is {
            ManagedBytes::Borrowed(s) => s.to_vec(),
            ManagedBytes::Owned(s) => s,
        }
    }
}

// Implement a couple traits by passing through to &[u8]'s methods

impl<'a> From<&'a [u8]> for ManagedBytes<'a> {
    fn from(s: &'a [u8]) -> Self {
        ManagedBytes::Borrowed(s)
    }
}

impl<'a> From<&'a ManagedBytes<'a>> for &'a [u8] {
    fn from(is: &'a ManagedBytes<'a>) -> &'a [u8] {
        is.as_slice()
    }
}

impl<'a> PartialEq for ManagedBytes<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice().eq(other.as_slice())
    }
}

impl<'a> Debug for ManagedBytes<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl<'a> Serialize for ManagedBytes<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.as_slice().serialize(serializer)
    }
}

impl<'a, 'de: 'a> Deserialize<'de> for ManagedBytes<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&'de [u8] as Deserialize<'de>>::deserialize(deserializer)?;
        Ok(ManagedBytes::Borrowed(s))
    }
}

// ---

#[derive(Clone)]
pub enum ManagedString<'a> {
    /// Borrowed variant.
    Borrowed(&'a str),
    #[cfg(feature = "use-std")]
    /// Owned variant, only available with the std or alloc feature enabled.
    Owned(String),
}

impl<'a> ManagedString<'a> {
    /// Create an Managed from a borrowed slice
    pub fn from_borrowed(s: &'a str) -> Self {
        ManagedString::Borrowed(s)
    }

    /// Create an Managed from an owned String
    #[cfg(feature = "use-std")]
    pub fn from_string(s: String) -> ManagedString<'static> {
        ManagedString::Owned(s)
    }

    /// View the Managed as a slice
    pub fn as_str(&'a self) -> &'a str {
        match self {
            ManagedString::Borrowed(s) => s,
            #[cfg(feature = "use-std")]
            ManagedString::Owned(s) => s.as_str(),
        }
    }

    #[cfg(feature = "use-std")]
    pub fn to_owned(&'a self) -> ManagedString<'static> {
        match self {
            ManagedString::Borrowed(b) => ManagedString::Owned(b.to_string()),
            ManagedString::Owned(s) => ManagedString::Owned(s.clone()),
        }
    }
}

// Optional impls

#[cfg(feature = "use-std")]
impl From<String> for ManagedString<'static> {
    fn from(s: String) -> Self {
        ManagedString::Owned(s)
    }
}

#[cfg(feature = "use-std")]
impl From<ManagedString<'static>> for String {
    fn from(is: ManagedString<'static>) -> Self {
        match is {
            ManagedString::Borrowed(s) => s.to_string(),
            ManagedString::Owned(s) => s,
        }
    }
}

// Implement a couple traits by passing through to &[u8]'s methods

impl<'a> From<&'a str> for ManagedString<'a> {
    fn from(s: &'a str) -> Self {
        ManagedString::Borrowed(s)
    }
}

impl<'a> From<&'a ManagedString<'a>> for &'a str {
    fn from(is: &'a ManagedString<'a>) -> &'a str {
        is.as_str()
    }
}

impl<'a> PartialEq for ManagedString<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl<'a> Debug for ManagedString<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_str().fmt(f)
    }
}

impl<'a> Serialize for ManagedString<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.as_str().serialize(serializer)
    }
}

#[cfg(not(feature = "use-std"))]
impl<'a, 'de: 'a> Deserialize<'de> for ManagedString<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&'de str as Deserialize<'de>>::deserialize(deserializer)?;
        Ok(ManagedString::Borrowed(s))
    }
}

#[cfg(feature = "use-std")]
impl<'de> Deserialize<'de> for ManagedString<'static> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&'de str as Deserialize<'de>>::deserialize(deserializer)?.to_string();
        Ok(ManagedString::Owned(s))
    }
}
