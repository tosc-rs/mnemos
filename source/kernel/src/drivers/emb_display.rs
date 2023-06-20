//! Simulated display driver
//!
//! This is an early attempt at a "frame buffer" style display driver.
//!
//! This implementation is sort of a work in progress, it isn't really a *great*
//! long-term solution, but rather "okay for now".
//!
//! A framebuffer of pixels is allocated for the entire display on registration.
//! This could be, for example, 400x240 pixels.
//!
//! The driver will then allow for a certain number of "sub frames" to be requested.
//!
//! These sub frames could be for the entire display (400x240), or a portion of it,
//! for example 200x120 pixels.
//!
//! Clients of the driver can draw into the sub-frames that they receive, then send
//! them back to be rendered into the total frame. Any data in the client's sub-frame
//! will replace the current contents of the whole frame buffer.
//!
//! The current server assumes 8 bits per pixel, which is implementation defined for
//! color/greyscale format (sorry).

use crate::{
    comms::oneshot::Reusable,
    registry::{Envelope, KernelHandle, RegisteredDriver, ReplyTo},
    Kernel,
};
use embedded_graphics::{
    pixelcolor::{Gray8, GrayColor},
    prelude::*,
};
use mnemos_alloc::containers::HeapArray;
use uuid::Uuid;

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////

// Registered driver
pub struct EmbDisplayService;

// impl EmbDisplay
impl RegisteredDriver for EmbDisplayService {
    type Request = Request;
    type Response = Response;
    type Error = FrameError;
    const UUID: Uuid = crate::registry::known_uuids::kernel::EMB_DISPLAY;
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

/// These are all of the possible requests from client to server
pub enum Request {
    /// Request a new frame chunk, with the given location and size
    NewFrameChunk {
        start_x: i32,
        start_y: i32,
        width: u32,
        height: u32,
    },
    /// Draw the provided framechunk
    Draw(FrameChunk),
    /// Drop the provided framechunk, without drawing it
    Drop(FrameChunk),
}

pub enum Response {
    /// Successful frame allocation
    FrameChunkAllocated(FrameChunk),
    /// Successful draw
    FrameDrawn,
    /// Successful drop
    FrameDropped,
}

#[derive(Debug, Eq, PartialEq)]
pub enum FrameError {
    /// Failed to register a display, the kernel reported that there is already
    /// an existing EmbDisplay
    DisplayAlreadyExists,
    /// No frames available from the driver
    NoFrameAvailable,
    /// Attempted to draw or drop an invalid FrameChunk
    NoSuchFrame,
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

/// Client interface to EmbDisplay
pub struct EmbDisplayClient {
    prod: KernelHandle<EmbDisplayService>,
    reply: Reusable<Envelope<Result<Response, FrameError>>>,
}

impl EmbDisplayClient {
    /// Obtain a new client handle by querying the registry for a registered
    /// [EmbDisplay] server
    pub async fn from_registry(kernel: &'static Kernel) -> Option<Self> {
        let prod = kernel
            .with_registry(|reg| reg.get::<EmbDisplayService>())
            .await?;

        Some(EmbDisplayClient {
            prod,
            reply: Reusable::new_async(kernel).await,
        })
    }

    /// Drop the provided framechunk without drawing
    pub async fn drop_framechunk(&mut self, chunk: FrameChunk) -> Result<(), ()> {
        self.prod
            .send(
                Request::Drop(chunk),
                ReplyTo::OneShot(self.reply.sender().await.map_err(drop)?),
            )
            .await?;
        Ok(())
    }

    /// Draw the requested framechunk
    pub async fn draw_framechunk(&mut self, chunk: FrameChunk) -> Result<(), ()> {
        self.prod
            .send(
                Request::Draw(chunk),
                ReplyTo::OneShot(self.reply.sender().await.map_err(drop)?),
            )
            .await?;
        Ok(())
    }

    /// Allocate a framechunk
    pub async fn get_framechunk(
        &mut self,
        start_x: i32,
        start_y: i32,
        width: u32,
        height: u32,
    ) -> Option<FrameChunk> {
        self.prod
            .send(
                Request::NewFrameChunk {
                    start_x,
                    start_y,
                    width,
                    height,
                },
                ReplyTo::OneShot(self.reply.sender().await.ok()?),
            )
            .await
            .ok()?;

        let resp = self.reply.receive().await.ok()?;
        let body = resp.body.ok()?;

        if let Response::FrameChunkAllocated(frame) = body {
            Some(frame)
        } else {
            None
        }
    }
}

/// FrameChunk is recieved after client has sent a request for one
pub struct FrameChunk {
    pub frame_id: u16,
    pub bytes: HeapArray<u8>,
    pub start_x: i32,
    pub start_y: i32,
    pub width: u32,
    pub height: u32,
}

/// FrameChunk implements embedded-graphics's `DrawTarget` trait so that clients
/// can directly use embedded-graphics primitives for drawing into the framebuffer.
impl DrawTarget for FrameChunk {
    type Color = Gray8;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            let (x, y): (u32, u32) = match coord.try_into() {
                Ok(c) => c,
                Err(_) => continue,
            };
            if x >= self.width {
                continue;
            }
            if y >= self.height {
                continue;
            }

            let index: u32 = x + y * self.width;
            // TODO: Implement bound checks and return BufferFull if needed
            self.bytes[index as usize] = color.luma();
        }

        Ok(())
    }
}

impl OriginDimensions for FrameChunk {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}
