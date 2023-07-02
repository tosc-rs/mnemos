//! # sermux-proto
//!
//! Wire types used by the `SerialMuxService` in the kernel. Extracted as a
//! separate crate to allow external decoders (like `crowtty`) to share protocol
//! definitions

#![cfg_attr(not(any(test, feature = "use-std")), no_std)]

use core::{fmt::Display, mem::size_of};

////////////////////////////////////////////////////////////////////////////////
// Well Known Ports
////////////////////////////////////////////////////////////////////////////////

/// Well known `SerialMuxService` ports
#[repr(u16)]
#[non_exhaustive]
pub enum WellKnown {
    /// A bidirectional loopback channel - echos all characters back
    Loopback = 0,
    /// An output-only channel for sending periodic sign of life messages
    HelloWorld = 1,
    /// An input-only channel to act as a keyboard for a GUI application
    /// such as a forth console, when there is no hardware keyboard available.
    ///
    /// Unlike the ForthShell ports, which serve as ssh/telnet like bidirectional
    /// items, PsuedoKeyboard is only used to receive the input, as the output is
    /// shown on a graphical terminal
    PsuedoKeyboard = 2,
    /// A bidirectional for binary encoded tracing messages
    BinaryTracing = 3,

    /// A bidirectional interactive forth shell (1/4)
    ForthShell0 = 10,
    /// A bidirectional interactive forth shell (2/4)
    ForthShell1 = 11,
    /// A bidirectional interactive forth shell (3/4)
    ForthShell2 = 12,
    /// A bidirectional interactive forth shell (4/4)
    ForthShell3 = 13,
}

impl Into<u16> for WellKnown {
    fn into(self) -> u16 {
        self as u16
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum EncodeError {
    /// The provided buffer is not suitable in size
    InsufficientSize,
    /// Ran out of room while filling a buffer, this is likely
    /// an error in the `sermux-proto` library.
    UnexpectedBufferFull,
}

impl Display for EncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let st = match self {
            EncodeError::InsufficientSize => "InsufficientSize",
            EncodeError::UnexpectedBufferFull => "UnexpectedBufferFull",
        };
        f.write_str(st)
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum DecodeError {
    /// The cobs decoding process failed. The message was likely
    /// malformed or not a sermux-proto frame
    CobsDecodeFailed,
    /// Cobs decoding succeeded, but the resulting data was not
    /// a valid sermux-proto frame
    MalformedFrame,
}

impl Display for DecodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let st = match self {
            DecodeError::CobsDecodeFailed => "CobsDecodeFailed",
            DecodeError::MalformedFrame => "MalformedFrame",
        };
        f.write_str(st)
    }
}

#[derive(Debug, PartialEq)]
pub struct PortChunk<'a> {
    pub port: u16,
    pub chunk: &'a [u8],
}

impl<'a> PortChunk<'a> {
    /// Create a new PortChunk from the given port and data
    #[inline]
    pub fn new(port: impl Into<u16>, chunk: &'a [u8]) -> Self {
        Self {
            port: port.into(),
            chunk,
        }
    }

    /// Calculate the size required to encode the given data payload size
    #[inline]
    #[must_use]
    pub fn buffer_required(&self) -> usize {
        // Room for COBS(port:u16 + data:[u8; len]) plus a terminating zero
        cobs::max_encoding_length(self.chunk.len() + size_of::<u16>() + 1)
    }

    /// Encodes the current [PortChunk] into the given buffer
    pub fn encode_to<'b>(&self, out_buf: &'b mut [u8]) -> Result<&'b mut [u8], EncodeError> {
        let PortChunk { port, chunk } = self;
        if out_buf.len() < self.buffer_required() {
            return Err(EncodeError::InsufficientSize);
        }

        let mut encoder = cobs::CobsEncoder::new(out_buf);
        encoder
            .push(&port.to_le_bytes())
            .map_err(|_| EncodeError::UnexpectedBufferFull)?;
        encoder
            .push(chunk)
            .map_err(|_| EncodeError::UnexpectedBufferFull)?;
        let used = encoder
            .finalize()
            .map_err(|_| EncodeError::UnexpectedBufferFull)?;
        // Get the encoded amount, with room for an extra zero terminator
        let res = out_buf
            .get_mut(..used + 1)
            .ok_or(EncodeError::UnexpectedBufferFull)?;
        res[used] = 0;
        Ok(res)
    }

    /// Decodes a [PortChunk] from the given buffer
    ///
    /// NOTE: This MAY mutate `data`, even if the decoding fails.
    pub fn decode_from(data: &'a mut [u8]) -> Result<Self, DecodeError> {
        let dec_len = cobs::decode_in_place(data).map_err(|_| DecodeError::CobsDecodeFailed)?;

        // Messages must have a port and at least one data byte to be
        // well formed
        if dec_len < (size_of::<u16>() + 1) {
            return Err(DecodeError::MalformedFrame);
        }

        let frame = data.get(..dec_len).ok_or(DecodeError::MalformedFrame)?;

        let mut port_bytes = [0u8; size_of::<u16>()];
        let (port_data, chunk) = frame.split_at(size_of::<u16>());
        port_bytes.copy_from_slice(port_data);
        let port = u16::from_le_bytes(port_bytes);

        Ok(PortChunk { port, chunk })
    }

    /// Convert into an [OwnedPortChunk]
    ///
    /// Only available with the `use-std` feature active
    #[cfg(feature = "use-std")]
    pub fn into_owned(self) -> OwnedPortChunk {
        OwnedPortChunk {
            port: self.port,
            chunk: self.chunk.to_vec(),
        }
    }
}

/// Like [PortChunk], but owns the storage instead
///
/// Only available with the `use-std` feature active
#[cfg(feature = "use-std")]
pub struct OwnedPortChunk {
    pub port: u16,
    pub chunk: Vec<u8>,
}

#[cfg(feature = "use-std")]
impl OwnedPortChunk {
    /// Create a new OwnedPortChunk from the given port and data
    #[inline]
    #[must_use]
    pub fn new(port: u16, chunk: Vec<u8>) -> Self {
        Self { port, chunk }
    }

    /// Calculate the size required to encode the given data payload size
    #[inline]
    #[must_use]
    pub fn buffer_required(&self) -> usize {
        // Room for COBS(port:u16 + data:[u8; len]) plus a terminating zero
        cobs::max_encoding_length(self.chunk.len() + size_of::<u16>() + 1)
    }

    /// Encodes the current [PortChunk] into the given buffer
    pub fn encode_to<'b>(&self, out_buf: &'b mut [u8]) -> Result<&'b mut [u8], EncodeError> {
        let pc = self.as_port_chunk();
        pc.encode_to(out_buf)
    }

    /// Decodes an [OwnedPortChunk] from the given buffer
    ///
    /// Unlike [PortChunk::decode_from], this will not mutate the given buffer.
    pub fn decode(data: &[u8]) -> Result<Self, DecodeError> {
        let mut data = data.to_vec();
        let pc = PortChunk::decode_from(&mut data)?;
        let port = pc.port;
        let len = pc.chunk.len();
        data.shrink_to(len);
        Ok(OwnedPortChunk { port, chunk: data })
    }

    /// Borrows self as a [PortChunk]
    pub fn as_port_chunk(&self) -> PortChunk<'_> {
        PortChunk {
            port: self.port,
            chunk: &self.chunk,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn len_calc_right() {
        let data = [1, 2, 3, 4];
        let pc = PortChunk::new(0x4269u16, &data);
        let reqd = pc.buffer_required();
        assert_eq!(8, reqd);
        let mut buf = [0u8; 8];
        let res = pc.encode_to(&mut buf).unwrap();
        assert_eq!(&[7, 0x69, 0x42, 1, 2, 3, 4, 0], res);

        let data = [1u8; 256];
        let pc = PortChunk::new(0x4269u16, &data);
        let reqd = pc.buffer_required();
        assert_eq!(261, reqd);
        let mut buf = [0u8; 261];
        let res = pc.encode_to(&mut buf).unwrap();
        assert_eq!(res.len(), 261);
    }

    #[test]
    fn round_trip() {
        let pc = PortChunk {
            port: 1234,
            chunk: &[1, 2, 3, 4],
        };
        assert_eq!(pc.buffer_required(), 8);
        let mut buf = [0u8; 8];
        let enc = pc.encode_to(&mut buf).unwrap();

        let dec = PortChunk::decode_from(enc).unwrap();
        assert_eq!(dec.port, 1234);
        assert_eq!(dec.chunk, &[1, 2, 3, 4]);
    }

    #[test]
    fn too_short() {
        // ONLY cobs delim (zero size)
        let mut data = [0];
        assert_eq!(
            PortChunk::decode_from(&mut data),
            Err(DecodeError::MalformedFrame)
        );

        // cobs header + delim (zero size)
        let mut data = [1, 0];
        assert_eq!(
            PortChunk::decode_from(&mut data),
            Err(DecodeError::MalformedFrame)
        );

        // cobs header + 1 data byte
        let mut data = [1, 1, 0];
        assert_eq!(
            PortChunk::decode_from(&mut data),
            Err(DecodeError::MalformedFrame)
        );

        // cobs header + 2 data byte
        let mut data = [1, 1, 1, 0];
        assert_eq!(
            PortChunk::decode_from(&mut data),
            Err(DecodeError::MalformedFrame)
        );

        // cobs header + 3 data byte (2 byte port, 1 byte chunk)
        let mut data = [1, 1, 1, 1, 0];
        let _ = PortChunk::decode_from(&mut data).unwrap();
    }

    #[test]
    fn bad_cobs() {
        // cobs pointer goes past the end
        let mut data = [100, 2, 3, 0];
        assert_eq!(
            PortChunk::decode_from(&mut data),
            Err(DecodeError::CobsDecodeFailed)
        );

        // secondary cobs pointer goes past the end
        let mut data = [2, 2, 2, 0];
        assert_eq!(
            PortChunk::decode_from(&mut data),
            Err(DecodeError::CobsDecodeFailed)
        );
    }
}
