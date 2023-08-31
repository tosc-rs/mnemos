//! EmbDisplay Service v2
//!
//! EmbDisplayV2 is the second generation display driver interface.
//!
//! Users are expected to allocate [FrameChunk]s, which are a subsection of the
//! total screen, and send them to the Display service for drawing. Once drawn,
//! the display returns the chunks to the user for potential re-use.
//!
//! See the docs of [FrameChunk] and [EmbDisplayClient] for additional details
//! of use.
use core::time::Duration;

use embedded_graphics::{
    pixelcolor::{BinaryColor, Gray8},
    prelude::*,
};
use uuid::Uuid;

use crate::{
    comms::oneshot::Reusable,
    mnemos_alloc::containers::HeapArray,
    registry::{self, Envelope, KernelHandle, RegisteredDriver},
    Kernel,
};

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

/// Registered driver type for the `EmbDisplay` service.
///
/// This module provides an implementation of the client for this service, but
/// not the server. A server implementing this service must be provided by the
/// hardware platform implementation.
pub struct EmbDisplayService;

// impl EmbDisplay
impl RegisteredDriver for EmbDisplayService {
    type Request = Request;
    type Response = Response;
    type Error = FrameError;
    const UUID: Uuid = registry::known_uuids::kernel::EMB_DISPLAY_V2;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

/// These are all of the possible requests from client to server
pub enum Request {
    GetMeta,
    Draw(FrameChunk),
}

pub enum Response {
    FrameMeta(DisplayMetadata),
    /// Successful draw
    DrawComplete(FrameChunk),
}

#[derive(Debug, Eq, PartialEq)]
pub enum FrameError {
    /// Failed to register a display, the kernel reported that there is already
    /// an existing EmbDisplay
    DisplayAlreadyExists,
    /// We are still waiting for a response from the last request
    Busy,
    /// Internal Error
    InternalError,
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// Client interface to [`EmbDisplayService`].
pub struct EmbDisplayClient {
    prod: KernelHandle<EmbDisplayService>,
    reply: Reusable<Envelope<Result<Response, FrameError>>>,
}

impl EmbDisplayClient {
    /// Obtain a new client handle by querying the registry for a registered
    /// [`EmbDisplayService`].
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
    /// [`EmbDisplayService`].
    ///
    /// Will not retry if not immediately successful
    pub async fn from_registry_no_retry(kernel: &'static Kernel) -> Option<Self> {
        let prod = kernel
            .with_registry(|reg| reg.get::<EmbDisplayService>())
            .await?;

        Some(EmbDisplayClient {
            prod,
            reply: Reusable::new_async().await,
        })
    }

    pub async fn draw<C: Into<FrameChunk>>(&mut self, chunk: C) -> Result<FrameChunk, FrameError> {
        let chunk = chunk.into();
        let resp = self
            .prod
            .request_oneshot(Request::Draw(chunk), &self.reply)
            .await
            .map_err(|_| FrameError::InternalError)?
            .body?;
        Ok(match resp {
            Response::DrawComplete(fc) => fc,
            _ => return Err(FrameError::InternalError),
        })
    }

    pub async fn draw_mono(&mut self, chunk: MonoChunk) -> Result<MonoChunk, FrameError> {
        match self.draw(chunk).await {
            Ok(FrameChunk::Mono(mfc)) => Ok(mfc),
            _ => Err(FrameError::InternalError),
        }
    }

    pub async fn get_meta(&mut self) -> Result<DisplayMetadata, FrameError> {
        let resp = self
            .prod
            .request_oneshot(Request::GetMeta, &self.reply)
            .await
            .map_err(|_| FrameError::InternalError)?
            .body?;

        Ok(match resp {
            Response::FrameMeta(m) => m,
            Response::DrawComplete(_) => return Err(FrameError::InternalError),
        })
    }
}

/// A drawable buffer
///
/// The [FrameChunk] represents a section of allocated memory that can be drawn
/// into. It can be one of multiple "kinds" of buffer, representing different
/// color and transparency depths.
///
/// Users may use any kind of buffer they'd like, and displays are expected to
/// convert to a format that they can use, for example down or upsampling color
/// depth, converting color to grayscale, etc.
///
/// This [FrameChunk] is passed to [EmbDisplayClient::draw()] to be rendered to
/// the display
///
/// ## Subsizing
///
/// This frame chunk is expected to be equal or smaller than the total display
/// itself, and each of the kinds of framechunk will have [metadata] that contains
/// both the position and the size of the framechunk. Framechunks can be moved
/// but not resized (if resizing is necessary, the current chunk should be dropped
/// and a new one should be allocated).
///
/// [metadata]: [FrameBufMeta]
///
/// ## Transparency
///
/// FrameChunks also have a transparency component, which allows them to be used for
/// [blitting] or [compositing] onto the final display image.
///
/// [blitting]: https://en.wikipedia.org/wiki/Bit_blit
/// [compositing]: https://en.wikipedia.org/wiki/Compositing
///
/// For example, a circle with radius of 80 pixels could be drawn in a 100x100
/// FrameChunk, with the area outside of the circle marked as transparent. This
/// allows the 100x100 square to be "blitted" at the target location without
/// overwriting the existing content or background outside the circle.
///
/// This could also be used to keep persistently drawn [sprites] in memory,
/// layering them onto the target frame.
///
/// All FrameChunk kinds have some kind of transparency, though this may range from
/// a single bit transparency (transparent or not), to a more complex [alpha channel]
///
/// [alpha channel]: https://en.wikipedia.org/wiki/Alpha_compositing
/// [sprites]: https://en.wikipedia.org/wiki/Sprite_(computer_graphics)
#[non_exhaustive]
pub enum FrameChunk {
    Mono(MonoChunk),
}

impl From<MonoChunk> for FrameChunk {
    fn from(value: MonoChunk) -> Self {
        FrameChunk::Mono(value)
    }
}

// TODO: both the data and the mask could be stored 1bpp, however because
// that math was beyond me at the time, I am storing them 8bpp, which is
// very wasteful in terms of memory, but means that we don't need to do
// tricky bit operations.
//
// On the other hand, it is likely a bit less computationally intense to
// stick with byte addressing, as we don't need to do shifting and such
// for individual pixel operations, but there might be nice ways to accelerate
// that, though then you need to worry about "alignment" of data, e.g. if the
// start_x is not a multiple of 8.
//
// It may just be worth adding a "MonoChunk1bpp" variant in the future to allow
// users to make the size/perf tradeoff, particularly if we want to support
// targets with very small memory. For example, a 400x240 monochrome display would
// be 93.75KiB at 8bpp, but only 11.72KiB at 1bpp.
pub struct MonoChunk {
    meta: FrameChunkMetadata,
    data: Buf8,
    mask: Buf8,
}

impl MonoChunk {
    /// Mark all transparency data as fully transparent
    pub fn clear(&mut self) {
        self.mask.bytes.iter_mut().for_each(|b| *b = 0);
    }

    /// Allocate a monochrome framebuffer with the given metadata
    pub async fn allocate_mono(size: FrameLocSize) -> Self {
        let meta = FrameChunkMetadata {
            start_x: size.offset_x,
            start_y: size.offset_y,
            width: size.width,
            height: size.height,
        };
        let ttl = (size.width * size.height) as usize;
        let data = Buf8 {
            bytes: HeapArray::new(ttl, 0).await,
        };
        let mask = Buf8 {
            bytes: HeapArray::new(ttl, 0).await,
        };
        MonoChunk { meta, data, mask }
    }

    /// Invert all pixels that are NOT currently transparent
    ///
    /// This can be used to "unblit" an image
    pub fn invert_masked(&mut self) {
        self.data
            .bytes
            .iter_mut()
            .zip(self.mask.bytes.iter())
            .for_each(|(d, m)| {
                if *m != 0 {
                    *d = !*d;
                }
            });
    }

    /// Get the metadata of this FrameChunk
    pub fn meta(&self) -> &FrameChunkMetadata {
        &self.meta
    }

    /// Get the metadatae of this FrameChunk as a mutable reference
    pub fn meta_mut(&mut self) -> &mut FrameChunkMetadata {
        &mut self.meta
    }

    /// Get the raw pixel data
    ///
    /// This is currently one byte per pixel, with 0x00 representing "OFF" and
    /// 0xFF representing "ON". All other values are invalid.
    ///
    // TODO: This interface would semantically change if we switch to 1bpp!
    pub fn data(&self) -> &[u8] {
        let bytes = self.meta.width * self.meta.height;
        let data_sli: &[u8] = &self.data.bytes;
        assert_eq!(bytes as usize, data_sli.len());
        data_sli
    }

    /// Get the raw mask data
    ///
    /// This is currently one byte per pixel, with 0x00 representing "Transparent"
    /// and 0xFF representing "Solid". All other values are invalid.
    ///
    // TODO: This interface would semantically change if we switch to 1bpp!
    pub fn mask(&self) -> &[u8] {
        let bytes = self.meta.width * self.meta.height;
        let mask_sli: &[u8] = &self.mask.bytes;
        assert_eq!(bytes as usize, mask_sli.len());
        mask_sli
    }

    /// Draw the given pixel, and mark the pixel as not transparent
    ///
    /// If you want to instead mark the pixel as transparent, see
    /// [MonoChunk::clear_pixel()].
    #[inline]
    pub fn draw_pixel(&mut self, x: u32, y: u32, state: bool) {
        let idx = match self.pix_idx(x, y) {
            Some(i) => i,
            None => return,
        };
        self.data.bytes[idx] = match state {
            false => Gray8::BLACK.into_storage(),
            true => Gray8::WHITE.into_storage(),
        };
        self.mask.bytes[idx] = 0xFF;
    }

    /// Get the data/mask array index of the given X/Y coordinates
    ///
    /// This does NOT account for `offset`, this is only in the coordinate system
    /// of the [MonoChunk], not the total display.
    ///
    /// Returns None if the given x/y coordinates are outside of the [MonoChunk]
    pub fn pix_idx(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.meta.width {
            return None;
        }
        if y >= self.meta.height {
            return None;
        }
        Some(((y * self.meta.width) + x) as usize)
    }

    /// Clear the given pixel by marking it as transparent
    ///
    /// NOTE: This is not used for setting the pixel as "off", instead
    /// use [MonoChunk::draw_pixel()].
    #[inline]
    pub fn clear_pixel(&mut self, x: u32, y: u32) {
        let idx = match self.pix_idx(x, y) {
            Some(i) => i,
            None => return,
        };
        self.mask.bytes[idx] = 0x00;
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
        for Pixel(coord, color) in pixels.into_iter() {
            let Ok((x, y)): Result<(u32, u32), _> = coord.try_into() else {
                continue;
            };
            self.draw_pixel(x, y, color.is_on());
        }

        Ok(())
    }
}

/// This is used for placing the [FrameChunk] in the overall display
pub struct FrameLocSize {
    /// Offset in pixels from the top left corner (rightward)
    pub offset_x: u32,
    /// Offset in pixels from the top left corner (downward)
    pub offset_y: u32,
    /// Width of the frame chunk in pixels
    pub width: u32,
    /// Height of the frame chunk in pixels
    pub height: u32,
}

/// Kinds of [FrameChunk]s
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum FrameKind {
    /// Monochrome - matches [FrameChunk::Mono]
    Mono,
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

/// Metadata of the entire display
#[derive(Copy, Clone, Debug)]
pub struct DisplayMetadata {
    pub kind: FrameKind,
    pub width: u32,
    pub height: u32,
}

/// Metadata of the [FrameChunk]
#[derive(Copy, Clone)]
pub struct FrameChunkMetadata {
    start_x: u32,
    start_y: u32,
    width: u32,
    height: u32,
}

impl FrameChunkMetadata {
    pub fn start_x(&self) -> u32 {
        self.start_x
    }

    pub fn start_y(&self) -> u32 {
        self.start_y
    }

    pub fn set_start_x(&mut self, start_x: u32) {
        self.start_x = start_x;
    }

    pub fn set_start_y(&mut self, start_y: u32) {
        self.start_y = start_y;
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

struct Buf8 {
    bytes: HeapArray<u8>,
}
