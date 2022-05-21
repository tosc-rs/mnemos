//! A Framed flavor of BBQueue, useful for variable length packets
//!
//! This module allows for a `Framed` mode of operation,
//! where a size header is included in each grant, allowing for
//! "chunks" of data to be passed through a BBQueue, rather than
//! just a stream of bytes. This is convenient when receiving
//! packets of variable sizes.
//!
//! ## Example
//!
//! ```rust
//! # // bbqueue test shim!
//! # fn bbqtest() {
//! use bbqueue::BBBuffer;
//!
//! let bb: BBBuffer<1000> = BBBuffer::new();
//! let (mut prod, mut cons) = bb.try_split_framed().unwrap();
//!
//! // One frame in, one frame out
//! let mut wgrant = prod.grant(128).unwrap();
//! assert_eq!(wgrant.len(), 128);
//! for (idx, i) in wgrant.iter_mut().enumerate() {
//!     *i = idx as u8;
//! }
//! wgrant.commit(128);
//!
//! let rgrant = cons.read().unwrap();
//! assert_eq!(rgrant.len(), 128);
//! for (idx, i) in rgrant.iter().enumerate() {
//!     assert_eq!(*i, idx as u8);
//! }
//! rgrant.release();
//! # // bbqueue test shim!
//! # }
//! #
//! # fn main() {
//! # #[cfg(not(feature = "thumbv6"))]
//! # bbqtest();
//! # }
//! ```
//!
//! ## Frame header
//!
//! An internal header is required for each frame stored
//! inside of the `BBQueue`. This header is never exposed to end
//! users of the bbqueue library.
//!
//! A variable sized integer is used for the header size, and the
//! size of this header is based on the max size requested for the grant.
//! This header size must be factored in when calculating an appropriate
//! total size of your buffer.
//!
//! Even if a smaller portion of the grant is committed, the original
//! requested grant size will be used for the header size calculation.
//!
//! For example, if you request a 128 byte grant, the header size will
//! be two bytes. If you then only commit 32 bytes, two bytes will still
//! be used for the header of that grant.
//!
//! | Grant Size (bytes)    | Header size (bytes)  |
//! | :---                  | :---                 |
//! | 1..(2^7)              | 1                    |
//! | (2^7)..(2^14)         | 2                    |
//! | (2^14)..(2^21)        | 3                    |
//! | (2^21)..(2^28)        | 4                    |
//! | (2^28)..(2^35)        | 5                    |
//! | (2^35)..(2^42)        | 6                    |
//! | (2^42)..(2^49)        | 7                    |
//! | (2^49)..(2^56)        | 8                    |
//! | (2^56)..(2^64)        | 9                    |
//!

use crate::bbqueue_ipc::{Consumer, GrantR, GrantW, Producer};

use crate::bbqueue_ipc::Result;

use core::{
    cmp::min,
    ops::{Deref, DerefMut},
};

const HDR_LEN: usize = core::mem::size_of::<u16>();

/// A producer of Framed data
pub struct FrameProducer<'a> {
    pub(crate) producer: Producer<'a>,
}

impl<'a> FrameProducer<'a> {
    /// Receive a grant for a frame with a maximum size of `max_sz` in bytes.
    ///
    /// This size does not include the size of the frame header. The exact size
    /// of the frame can be set on `commit`.
    pub fn grant(&self, max_sz: usize) -> Result<FrameGrantW<'a>> {
        Ok(FrameGrantW {
            grant_w: self.producer.grant_exact(max_sz + HDR_LEN)?,
        })
    }
}

/// A consumer of Framed data
pub struct FrameConsumer<'a> {
    pub(crate) consumer: Consumer<'a>,
}

impl<'a> FrameConsumer<'a> {
    /// Obtain the next available frame, if any
    pub fn read(&self) -> Option<FrameGrantR<'a>> {
        // Get all available bytes. We never wrap a frame around,
        // so if a header is available, the whole frame will be.
        let mut grant_r = self.consumer.read().ok()?;

        // Additionally, we never commit less than a full frame with
        // a header, so if we have ANY data, we'll have a full header
        // and frame. `Consumer::read` will return an Error when
        // there are 0 bytes available.

        // The header consists of a single u16, in little endian order
        let mut len_b = [0u8; 2];
        len_b.copy_from_slice(&grant_r[..HDR_LEN]);
        let frame_len = u16::from_le_bytes(len_b) as usize;
        let total_len = frame_len + HDR_LEN;

        debug_assert!(grant_r.len() >= total_len);

        // Reduce the grant down to the size of the frame with a header
        grant_r.shrink(total_len);

        Some(FrameGrantR { grant_r })
    }
}

/// A write grant for a single frame
///
/// NOTE: If the grant is dropped without explicitly commiting
/// the contents without first calling `to_commit()`, then no
/// frame will be comitted for writing.
#[derive(Debug, PartialEq)]
pub struct FrameGrantW<'a> {
    grant_w: GrantW<'a>,
}

/// A read grant for a single frame
///
/// NOTE: If the grant is dropped without explicitly releasing
/// the contents, then no frame will be released.
#[derive(Debug, PartialEq)]
pub struct FrameGrantR<'a> {
    grant_r: GrantR<'a>,
}

impl<'a> Deref for FrameGrantW<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.grant_w.buf[HDR_LEN..]
    }
}

impl<'a> DerefMut for FrameGrantW<'a> {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.grant_w.buf[HDR_LEN..]
    }
}

impl<'a> Deref for FrameGrantR<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.grant_r.buf[HDR_LEN..]
    }
}

impl<'a> DerefMut for FrameGrantR<'a> {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.grant_r.buf[HDR_LEN..]
    }
}

impl<'a> FrameGrantW<'a> {
    /// Commit a frame to make it available to the Consumer half.
    ///
    /// `used` is the size of the payload, in bytes, not
    /// including the frame header
    pub fn commit(mut self, used: usize) {
        let total_len = self.set_header(used);

        // Commit the header + frame
        self.grant_w.commit(total_len);
    }

    /// Set the header and return the total size
    fn set_header(&mut self, used: usize) -> usize {
        // Saturate the commit size to the available frame size
        let grant_len = self.grant_w.len();
        let frame_len = min(used, grant_len - HDR_LEN);
        let total_len = frame_len + HDR_LEN;

        // Write the actual frame length to the header

        self.grant_w[..HDR_LEN].copy_from_slice(&(total_len as u16).to_le_bytes());

        total_len
    }

    /// Configures the amount of bytes to be commited on drop.
    pub fn to_commit(&mut self, amt: usize) {
        if amt == 0 {
            self.grant_w.to_commit(0);
        } else {
            let size = self.set_header(amt);
            self.grant_w.to_commit(size);
        }
    }
}

impl<'a> FrameGrantR<'a> {
    /// Release a frame to make the space available for future writing
    ///
    /// Note: The full frame is always released
    pub fn release(mut self) {
        // For a read grant, we have already shrunk the grant
        // size down to the correct size
        let len = self.grant_r.len();
        self.grant_r.release_inner(len);
    }

    /// Set whether the read fram should be automatically released
    pub fn auto_release(&mut self, is_auto: bool) {
        self.grant_r
            .to_release(if is_auto { self.grant_r.len() } else { 0 });
    }
}
