//! [`embedded-graphics`] display driver
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

use core::time::Duration;

use crate::{
    comms::oneshot::{Reusable, ReusableError},
    registry::{Envelope, KernelHandle, RegisteredDriver, ReplyTo},
    Kernel,
};
use embedded_graphics::{
    pixelcolor::{Gray8, GrayColor},
    prelude::*,
};
use mnemos_alloc::containers::FixedVec;
use uuid::Uuid;

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

    /// Drop the provided framechunk without drawing
    pub async fn drop_framechunk(&mut self, chunk: FrameChunk) -> Result<(), FrameError> {
        self.prod
            .send(
                Request::Drop(chunk),
                ReplyTo::OneShot(self.reply.sender().await.map_err(|e| match e {
                    ReusableError::SenderAlreadyActive => FrameError::Busy,
                    _ => FrameError::InternalError,
                })?),
            )
            .await
            .map_err(|_| FrameError::InternalError)?;
        Ok(())
    }

    /// Draw the requested framechunk
    pub async fn draw_framechunk(&mut self, chunk: FrameChunk) -> Result<(), FrameError> {
        self.prod
            .send(
                Request::Draw(chunk),
                ReplyTo::OneShot(self.reply.sender().await.map_err(|e| match e {
                    ReusableError::SenderAlreadyActive => FrameError::Busy,
                    _ => FrameError::InternalError,
                })?),
            )
            .await
            .map_err(|_| FrameError::InternalError)?;
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
        let resp = self
            .prod
            .request_oneshot(
                Request::NewFrameChunk {
                    start_x,
                    start_y,
                    width,
                    height,
                },
                &self.reply,
            )
            .await
            .ok()?;
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
    pub bytes: FixedVec<u8>,
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
            let Ok((x, y)): Result<(u32, u32) , _> =  coord.try_into() else {
                continue;
            };
            if x >= self.width {
                continue;
            }
            if y >= self.height {
                continue;
            }

            let index: u32 = x + y * self.width;
            // TODO: Implement bound checks and return BufferFull if needed
            self.bytes.as_slice_mut()[index as usize] = color.luma();
        }

        Ok(())
    }
}

impl OriginDimensions for FrameChunk {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}
