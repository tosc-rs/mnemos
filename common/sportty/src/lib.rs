#![cfg_attr(not(test), no_std)]

use core::mem::size_of;

use cobs::{CobsEncoder, decode};
use postcard_cobs as cobs;

// Note: this sort of assumes this is some uN primative type. Thats fine for now.
pub type Port = u16;

pub struct Message<'a> {
    pub port: Port,
    pub data: &'a [u8],
}

pub enum Error {
    InsufficientSpace,
    DecodingError,
}

impl<'a> Message<'a> {
    pub fn encode_to<'b>(&self, dest: &'b mut [u8]) -> Result<&'b [u8], Error> {
        let mut encoder = CobsEncoder::new(dest);
        let port_le = self.port.to_le_bytes();
        encoder.push(&port_le).map_err(|_| Error::InsufficientSpace)?;
        encoder.push(self.data).map_err(|_| Error::InsufficientSpace)?;
        let used = encoder.finalize().map_err(|_| Error::InsufficientSpace)?;
        let end = dest.get_mut(used).ok_or(Error::InsufficientSpace)?;
        *end = 0;

        Ok(&dest[..(used+1)])
    }

    pub fn decode_to<'b>(src: &'b [u8], dst_buf: &'a mut [u8]) -> Result<Self, Error> {
        let src = match src.last() {
            Some(0) => &src[..src.len() - 1],
            Some(_) => src,
            None => return Err(Error::InsufficientSpace),
        };

        let used = decode(src, dst_buf).map_err(|_| Error::DecodingError)?;

        if (used < size_of::<Port>()) || used > dst_buf.len() {
            return Err(Error::DecodingError);
        }

        let relevant = &dst_buf[..used];

        let mut pbuf = [0u8; size_of::<Port>()];

        let (pbytes, dbytes) = relevant.split_at(size_of::<Port>());

        pbuf.copy_from_slice(pbytes);

        let port = Port::from_le_bytes(pbuf);

        Ok(Self {
            port,
            data: dbytes,
        })
    }
}
