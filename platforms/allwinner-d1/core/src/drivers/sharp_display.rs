//! Sharp display driver
//!
//! This is an early attempt at a "frame buffer" style display driver. It provides a
//! [emb_display service][kernel::services::emb_display] server, and uses the
//!
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

use core::time::Duration;

use embedded_graphics::{
    image::{Image, ImageRaw},
    pixelcolor::Gray8,
    prelude::*,
    primitives::Rectangle,
};
use kernel::{
    comms::kchannel::{KChannel, KConsumer},
    maitake::sync::{Mutex, WaitCell},
    mnemos_alloc::containers::{Arc, FixedVec},
    registry::Message,
    services::emb_display::{EmbDisplayService, FrameChunk, FrameError, Request, Response},
    Kernel,
};

use crate::spim::SpiSenderClient;

const WIDTH: usize = 400;
const HEIGHT: usize = 240;
const WIDTH_BYTES: usize = WIDTH / 8;

/// Implements the [`EmbDisplayService`] service interface
pub struct SharpDisplay;

impl SharpDisplay {
    /// Register the driver instance
    ///
    /// Registration will also start the simulated display, meaning that the display
    /// window will appear.
    pub async fn register(kernel: &'static Kernel, max_frames: usize) -> Result<(), FrameError> {
        let frames = FixedVec::new(max_frames).await;
        let mut linebuf = FixedVec::new((52 * 240) + 2).await;
        for _ in 0..(52 * 240) + 2 {
            let _ = linebuf.try_push(0);
        }

        let ctxt = Arc::new(Mutex::new(Context {
            sdisp: FullFrame::new(),
            vcom: false,
        }))
        .await;

        let (cmd_prod, cmd_cons) = KChannel::new_async(1).await.split();
        let commander = CommanderTask {
            cmd: cmd_cons,
            display_info: DisplayInfo {
                frames,
                frame_idx: 0,
            },
            ctxt: ctxt.clone(),
        };

        let vcom = VCom {
            kernel,
            ctxt: ctxt.clone(),
        };
        let draw = Draw {
            kernel,
            buf: linebuf,
            spim: SpiSenderClient::from_registry(kernel).await.unwrap(),
            ctxt,
        };

        kernel.spawn(commander.run()).await;
        kernel.spawn(vcom.run()).await;
        kernel.spawn(draw.run()).await;

        kernel
            .with_registry(|reg| reg.register_konly::<EmbDisplayService>(&cmd_prod))
            .await
            .map_err(|_| FrameError::DisplayAlreadyExists)?;

        Ok(())
    }
}

/// One entire frame, stored one bit per pixel
struct FullFrame {
    frame: [[u8; WIDTH_BYTES]; HEIGHT],
}

impl FullFrame {
    pub fn new() -> Self {
        Self {
            frame: [[0u8; WIDTH_BYTES]; HEIGHT],
        }
    }
}

impl FullFrame {
    #[inline]
    fn set_px(&mut self, x: usize, y: usize, color: Gray8) {
        if x >= WIDTH || y > HEIGHT {
            return;
        }
        let byte_x = x / 8;
        let bit_x = x % 8;

        if color.luma() > 128 {
            self.frame[y][byte_x] |= 1 << (bit_x as u8);
        } else {
            self.frame[y][byte_x] &= !(1 << (bit_x as u8))
        }
    }
}

impl Dimensions for FullFrame {
    fn bounding_box(&self) -> embedded_graphics::primitives::Rectangle {
        Rectangle::new(
            Point { x: 0, y: 0 },
            Size {
                width: 400,
                height: 240,
            },
        )
    }
}

impl DrawTarget for FullFrame {
    type Color = Gray8;

    type Error = ();

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for px in pixels {
            self.set_px(px.0.x as usize, px.0.y as usize, px.1)
        }
        Ok(())
    }
}

//////////////////////////////////////////////////////////////////////////////
// Helper tasks
//
// Friends to help us make things happen
//////////////////////////////////////////////////////////////////////////////

/// VCom - Once a second, update the vcom flag which is sent in every message.
///
/// TODO(AJM): The Beepberry uses a GPIO to toggle this rather than a bit in the
/// SPI messages. This struct should be expanded to toggle the EXT pin at this
/// rate instead, possibly as an optional field.
struct VCom {
    kernel: &'static Kernel,
    ctxt: Arc<Mutex<Context>>,
}

impl VCom {
    pub async fn run(self) {
        loop {
            self.kernel.sleep(Duration::from_secs(1)).await;
            let mut c = self.ctxt.lock().await;
            c.vcom = !c.vcom;
        }
    }
}

/// Drawing task
///
/// This task draws whenever there are pending (or "dirty") changes to the frame
/// buffer, or at a base rate of 2Hz. At the moment, we only do a full redraw to
/// ensure that VCOM is updated periodically. In the future, we can likely send
/// a "no-op" command instead of a full frame redraw if the dirty flag has not
/// been set, as a power optimization or latency improvement.
struct Draw {
    kernel: &'static Kernel,
    buf: FixedVec<u8>,
    spim: SpiSenderClient,
    ctxt: Arc<Mutex<Context>>,
}

impl Draw {
    async fn run(mut self) {
        loop {
            let c = self.ctxt.lock().await;
            // render into
            let vc = 0x01 | if c.vcom { 0x02 } else { 0x00 };
            self.buf.as_slice_mut()[0] = 0x01 | vc;

            let out_lines = self.buf.as_slice_mut()[1..].chunks_exact_mut(52);
            let in_lines = c.sdisp.frame.iter();
            let lines = out_lines.zip(in_lines);

            for (line, (oline, iline)) in &mut lines.enumerate() {
                oline[0] = (line as u8) + 1;
                oline[1..51].copy_from_slice(iline);
            }

            self.buf = self.spim.send_wait(self.buf).await.map_err(drop).unwrap();
            drop(c);
            let _ = self
                .kernel
                .timeout(Duration::from_millis(500), DIRTY.wait())
                .await;
        }
    }
}

/// This task is spawned by the call to [`SharpDisplay::register`]. It is a single
/// async function that will process requests, and periodically redraw the
/// framebuffer.
struct CommanderTask {
    cmd: KConsumer<Message<EmbDisplayService>>,
    display_info: DisplayInfo,
    ctxt: Arc<Mutex<Context>>,
}

impl CommanderTask {
    /// The entrypoint for the driver execution
    async fn run(mut self) {
        // This loop services incoming client requests.
        //
        // Generally, don't handle errors when replying to clients, this indicates that they
        // sent us a message and "hung up" without waiting for a response.
        loop {
            let msg = self.cmd.dequeue_async().await.map_err(drop).unwrap();
            let Message {
                msg: mut req,
                reply,
            } = msg;
            match &mut req.body {
                Request::NewFrameChunk {
                    start_x,
                    start_y,
                    width,
                    height,
                } => {
                    let res = self
                        .display_info
                        .new_frame(*start_x, *start_y, *width, *height)
                        .await
                        .map(Response::FrameChunkAllocated);

                    let resp = req.reply_with(res);

                    let _ = reply.reply_konly(resp).await;
                }
                Request::Draw(fc) => match self.display_info.remove_frame(fc.frame_id) {
                    Ok(_) => {
                        let (x, y) = (fc.start_x, fc.start_y);
                        let raw_img = frame_display(fc).unwrap();
                        let image = Image::new(&raw_img, Point::new(x, y));

                        let mut guard = self.ctxt.lock().await;

                        image.draw(&mut guard.sdisp).unwrap();
                        DIRTY.wake();

                        // Drop the guard before we reply so we don't hold it too long.
                        drop(guard);

                        let _ = reply
                            .reply_konly(req.reply_with(Ok(Response::FrameDrawn)))
                            .await;
                    }
                    Err(e) => {
                        let _ = reply.reply_konly(req.reply_with(Err(e))).await;
                    }
                },
                Request::Drop(fc) => {
                    let _ = match self.display_info.remove_frame(fc.frame_id) {
                        Ok(_) => {
                            reply
                                .reply_konly(req.reply_with(Ok(Response::FrameDropped)))
                                .await
                        }
                        Err(e) => reply.reply_konly(req.reply_with(Err(e))).await,
                    };
                }
            }
        }
    }
}

//////////////////////////////////////////////////////////////////////////////
// Helper types and methods
//////////////////////////////////////////////////////////////////////////////

/// Shared state between tasks
struct Context {
    sdisp: FullFrame,
    vcom: bool,
}

/// Waiter for "changes have been made to the working frame buffer"
static DIRTY: WaitCell = WaitCell::new();

/// Create and return a Simulator display object from raw pixel data.
///
/// Pixel data is turned into a raw image, and then drawn onto a SimulatorDisplay object
/// This is necessary as a e-g Window only accepts SimulatorDisplay object
/// On a physical display, the raw pixel data can be sent over to the display directly
/// Using the display's device interface
fn frame_display(fc: &mut FrameChunk) -> Result<ImageRaw<Gray8>, ()> {
    let raw_image: ImageRaw<Gray8>;
    raw_image = ImageRaw::<Gray8>::new(fc.bytes.as_slice(), fc.width);
    Ok(raw_image)
}

struct FrameInfo {
    frame: u16,
}

struct DisplayInfo {
    frame_idx: u16,
    frames: FixedVec<FrameInfo>,
}

impl DisplayInfo {
    // Returns a new frame chunk
    async fn new_frame(
        &mut self,
        start_x: i32,
        start_y: i32,
        width: u32,
        height: u32,
    ) -> Result<FrameChunk, FrameError> {
        let fidx = self.frame_idx;
        self.frame_idx = self.frame_idx.wrapping_add(1);

        self.frames
            .try_push(FrameInfo { frame: fidx })
            .map_err(|_| FrameError::NoFrameAvailable)?;

        let size = (width * height) as usize;

        // TODO: So, in the future, we might not want to ACTUALLY allocate here. Instead,
        // we might want to allocate ALL potential frame chunks at registration time and
        // hand those out, rather than requiring an allocation here.
        //
        // TODO: We might want to do ANY input checking here:
        //
        // * Making sure the request is smaller than the actual display
        // * Making sure the request exists entirely within the actual display
        let mut bytes = FixedVec::new(size).await;
        for _ in 0..size {
            let _ = bytes.try_push(0);
        }
        let fc = FrameChunk {
            frame_id: fidx,
            bytes,
            start_x,
            start_y,
            width,
            height,
        };

        Ok(fc)
    }

    fn remove_frame(&mut self, frame_id: u16) -> Result<(), FrameError> {
        let mut found = false;
        unsafe {
            // safety: This only removes items, and will not cause a realloc
            self.frames.as_vec_mut().retain(|fr| {
                let matches = fr.frame == frame_id;
                found |= matches;
                !matches
            });
        }
        if found {
            Ok(())
        } else {
            Err(FrameError::NoSuchFrame)
        }
    }
}
