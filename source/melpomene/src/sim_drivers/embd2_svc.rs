
#![allow(dead_code)]

use core::time::Duration;

use embedded_graphics::{
    // pixelcolor::{Gray8, GrayColor},
    pixelcolor::{BinaryColor, Gray8},
    prelude::*,
};
use mnemos_kernel::{
    comms::oneshot::{Reusable, ReusableError},
    mnemos_alloc::containers::HeapArray,
    registry::{self, Envelope, KernelHandle, RegisteredDriver, ReplyTo},
    Kernel,
};
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

/// Registered driver type for the `EmbDisplay2` service.
///
/// This module provides an implementation of the client for this service, but
/// not the server. A server implementing this service must be provided by the
/// hardware platform implementation.
pub struct EmbDisplay2Service;

// impl EmbDisplay2
impl RegisteredDriver for EmbDisplay2Service {
    type Request = Request;
    type Response = Response;
    type Error = FrameError;
    const UUID: Uuid = registry::known_uuids::kernel::EMB_DISPLAY;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

#[derive(Clone, Copy, Debug)]
pub enum FrameKind {
    Mono,
}

pub enum FrameChunk {
    Mono(MonoChunk),
}

/// These are all of the possible requests from client to server
pub enum Request {
    GetMeta,
    Draw(FrameChunk),
}

#[derive(Copy, Clone, Debug)]
pub struct FrameMeta {
    pub kind: FrameKind,
    pub width: u32,
    pub height: u32,
}

pub enum Response {
    FrameMeta(FrameMeta),
    /// Successful draw
    DrawComplete(FrameChunk),
}

#[derive(Debug, Eq, PartialEq)]
pub enum FrameError {
    /// Failed to register a display, the kernel reported that there is already
    /// an existing EmbDisplay2
    DisplayAlreadyExists,
    /// We are still waiting for a response from the last request
    Busy,
    /// Internal Error
    InternalError,
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// Client interface to [`EmbDisplay2Service`].
pub struct EmbDisplay2Client {
    prod: KernelHandle<EmbDisplay2Service>,
    reply: Reusable<Envelope<Result<Response, FrameError>>>,
}

pub struct FrameLocSize {
    pub offset_x: u32,
    pub offset_y: u32,
    pub width: u32,
    pub height: u32,
}

impl EmbDisplay2Client {
    /// Obtain a new client handle by querying the registry for a registered
    /// [`EmbDisplay2Service`].
    ///
    /// Will retry until success
    pub async fn from_registry(kernel: &'static Kernel) -> Self {
        loop {
            match Self::from_registry_no_retry(kernel).await {
                Some(me) => return me,
                None => {
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Obtain a new client handle by querying the registry for a registered
    /// [`EmbDisplay2Service`].
    ///
    /// Will not retry if not immediately successful
    pub async fn from_registry_no_retry(kernel: &'static Kernel) -> Option<Self> {
        let prod = kernel
            .with_registry(|reg| reg.get::<EmbDisplay2Service>())
            .await?;

        Some(EmbDisplay2Client {
            prod,
            reply: Reusable::new_async().await,
        })
    }

    pub async fn draw_mono(&mut self, chunk: MonoChunk) -> Result<MonoChunk, ()> {
        let resp = self
            .prod
            .request_oneshot(Request::Draw(chunk.into()), &self.reply)
            .await
            .unwrap()
            .body
            .unwrap();
        Ok(match resp {
            Response::FrameMeta(M) => todo!(),
            Response::DrawComplete(FrameChunk::Mono(fc)) => fc,
        })
    }

    pub async fn get_meta(&mut self) -> Result<FrameMeta, ()> {
        let resp = self
            .prod
            .request_oneshot(Request::GetMeta, &self.reply)
            .await
            .unwrap()
            .body
            .unwrap();

        Ok(match resp {
            Response::FrameMeta(m) => m,
            Response::DrawComplete(_) => panic!(),
        })
    }
}

impl OriginDimensions for MonoChunk {
    fn size(&self) -> Size {
        Size::new(self.meta.width, self.meta.height)
    }
}

impl OriginDimensions for FrameChunk {
    #[inline]
    fn size(&self) -> Size {
        match self {
            FrameChunk::Mono(mc) => mc.size(),
        }
    }
}

impl MonoChunk {

}

////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////
////////////////////////////////////////////////////////////////////////////////

// TODO: wrapper types instead of aliases?

impl From<MonoChunk> for FrameChunk {
    fn from(value: MonoChunk) -> Self {
        FrameChunk::Mono(value)
    }
}

pub struct MonoChunk {
    pub meta: FrameBufMeta,
    pub data: BufBit,
    pub mask: BufBit,
}

impl MonoChunk {
    pub fn clear(&mut self) {
        self.mask.bytes.iter_mut().for_each(|b| *b = 0);
    }

    pub async fn allocate_mono(size: FrameLocSize) -> Self {
        let meta = FrameBufMeta {
            start_x: size.offset_x,
            start_y: size.offset_y,
            width: size.width,
            height: size.height,
        };
        let ttl = (size.width * size.height) as usize;
        let data = BufBit {
            bytes: HeapArray::new(ttl, 0).await,
        };
        let mask = BufBit {
            bytes: HeapArray::new(ttl, 0).await,
        };
        MonoChunk { meta, data, mask }
    }

    pub fn invert_masked(&mut self) {
        let pixels = self.data.bytes.iter_mut().zip(self.mask.bytes.iter()).for_each(|(d, m)| {
            if *m != 0 {
                *d = !*d;
            }
        });
    }
}


/// FrameChunk implements embedded-graphics's `DrawTarget` trait so that clients
/// can directly use embedded-graphics primitives for drawing into the framebuffer.
impl DrawTarget for MonoChunk {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {

        let idx = |x: u32, y: u32| {
            ((y * self.meta.width) + x) as usize
        };

        let data = &mut self.data.bytes;
        let mask = &mut self.mask.bytes;

        for Pixel(coord, color) in pixels.into_iter() {
            let Ok((x, y)): Result<(u32, u32) , _> =  coord.try_into() else {
                continue;
            };
            if x >= self.meta.width {
                continue;
            }
            if y >= self.meta.height {
                continue;
            }

            let i = idx(x, y);
            data[i] = match color {
                BinaryColor::Off => Gray8::BLACK.into_storage(),
                BinaryColor::On => Gray8::WHITE.into_storage(),
            };
            mask[idx(x, y)] = 0x01;
        }

        Ok(())
    }
}

// type FrameBufMono = FrameBuf<BufBit>;
// type FrameBufGrey8 = FrameBuf<Buf8>;
// type FrameBufRgb565 = FrameBuf<Buf16>;
// TODO: Do a 32-bit version with 8-bit transparent + 3x8 RGB inline?

// struct FrameBuf<T> {
//     meta: FrameBufMeta,
//     buf: T,
// }

pub struct FrameBufMeta {
    pub start_x: u32,
    pub start_y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct BufBit {
    pub bytes: HeapArray<u8>,
}

struct Buf8 {
    pub bytes: HeapArray<u8>,
}

struct Buf16 {
    pub bytes: HeapArray<u16>,
}

struct Buf32 {
    pub words: HeapArray<u32>,
}
