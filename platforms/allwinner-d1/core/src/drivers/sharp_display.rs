//! Sharp display driver
//!
//! This is an early attempt at a "frame buffer" style display driver. It provides a
//! [emb_display service][kernel::services::emb_display] server, and uses the
//! d1-core specific [SpiSender][crate::drivers::spim::SpiSender] service as an SPI
//! "backend" for rendering.
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
//! ## Wire format
//!
//! Reference: <https://www.sharpsde.com/fileadmin/products/Displays/2016_SDE_App_Note_for_Memory_LCD_programming_V1.3.pdf>

use core::{time::Duration, convert::identity};

use embedded_graphics::{
    pixelcolor::Gray8,
    prelude::*,
    primitives::Rectangle,
};
use kernel::{
    comms::kchannel::{KChannel, KConsumer},
    maitake::sync::{Mutex, WaitCell, WaitQueue},
    mnemos_alloc::containers::{Arc, FixedVec},
    registry::Message,
    services::emb_display::{
        DisplayMetadata, EmbDisplayService, FrameChunk, FrameError, FrameKind, MonoChunk, Request,
        Response,
    },
    Kernel,
};

use crate::spim::SpiSenderClient;

const WIDTH: usize = 400;
const HEIGHT: usize = 240;

// Every pixel is one bit
const WIDTH_BYTES: usize = WIDTH / 8;

// Foreach LINE (240x) - 52 bytes total:
//   * 1 byte for line number
//   * (400bits / 8 = 50bytes) of data (one bit per pixel)
//   * 1 "dummy" byte
const LINE_COMMAND_BYTES: usize = 1;
const LINE_DUMMY_BYTES: usize = 1;
const LINE_DATA_BYTES: usize = WIDTH_BYTES;
const LINE_BYTES: usize = LINE_COMMAND_BYTES + LINE_DATA_BYTES + LINE_DUMMY_BYTES;

// Every FRAME gets a 1 byte command, all of the lines, and one extra dummy byte
const FRAME_COMMAND_BYTES: usize = 1;
const FRAME_DUMMY_BYTES: usize = 1;
const FRAME_DATA_BYTES: usize = HEIGHT * LINE_BYTES;
const FRAME_BYTES: usize = FRAME_COMMAND_BYTES + FRAME_DATA_BYTES + FRAME_DUMMY_BYTES;

mod commands {
    pub const TOGGLE_VCOM: u8 = 0b0000_0000;
    pub const WRITE_LINE: u8 = 0b0000_0001;
    pub const VCOM_MASK: u8 = 0b0000_0010;
}

/// Implements the [`EmbDisplayService`] service interface
pub struct SharpDisplay;

impl SharpDisplay {
    pub const WIDTH: usize = WIDTH;
    pub const HEIGHT: usize = HEIGHT;

    /// Register the driver instance
    ///
    /// Registration will also start the simulated display, meaning that the display
    /// window will appear.
    pub async fn register(kernel: &'static Kernel) -> Result<(), FrameError> {
        let linebuf = FixedVec::new(FRAME_BYTES).await;

        let ctxt = Arc::new(Mutex::new(Context {
            sdisp: FullFrame::new(),
            vcom: false,
        }))
        .await;

        let (cmd_prod, cmd_cons) = KChannel::new_async(1).await.split();
        let commander = CommanderTask {
            cmd: cmd_cons,
            ctxt: ctxt.clone(),
            height: HEIGHT as u32,
            width: WIDTH as u32,
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

        kernel.spawn(commander.cmd_run()).await;
        kernel.spawn(vcom.vcom_run()).await;
        kernel.spawn(draw.draw_run()).await;

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
    dirty_line: [bool; HEIGHT],
}

impl FullFrame {
    pub fn new() -> Self {
        Self {
            frame: [[0u8; WIDTH_BYTES]; HEIGHT],
            dirty_line: [true; HEIGHT],
        }
    }
}

impl FullFrame {
    #[inline]
    fn set_px(&mut self, x: usize, y: usize, color: Gray8) {
        if x >= WIDTH || y > HEIGHT {
            return;
        }
        self.dirty_line[y] = true;
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
                width: WIDTH as u32,
                height: HEIGHT as u32,
            },
        )
    }
}

// impl DrawTarget for FullFrame {
//     type Color = Gray8;

//     type Error = ();

//     fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
//     where
//         I: IntoIterator<Item = Pixel<Self::Color>>,
//     {
//         for px in pixels {
//             self.set_px(px.0.x as usize, px.0.y as usize, px.1)
//         }
//         Ok(())
//     }
// }

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
    #[tracing::instrument(skip(self))]
    pub async fn vcom_run(self) {
        loop {
            self.kernel.sleep(Duration::from_secs(1)).await;
            let mut c = self.ctxt.lock().await;
            c.vcom = !c.vcom;
            tracing::debug!(vcom = c.vcom, "Toggling vcom");
        }
    }
}

/// Drawing task
///
/// This task draws whenever there are pending (or "dirty") changes to the frame
/// buffer, or at a base rate of 2Hz.
struct Draw {
    kernel: &'static Kernel,
    buf: FixedVec<u8>,
    spim: SpiSenderClient,
    ctxt: Arc<Mutex<Context>>,
}

impl Draw {
    #[tracing::instrument(skip(self))]
    async fn draw_run(mut self) {
        loop {
            // tracing::info!("ding");
            let mut c = self.ctxt.lock().await;
            self.buf.clear();

            let mut drawn = 0;

            // Are there ANY dirty lines?
            if c.sdisp.dirty_line.iter().copied().any(identity) {
                // render into the buffer
                let mut cmd = commands::WRITE_LINE;
                if c.vcom {
                    cmd |= commands::VCOM_MASK;
                }

                // Write the command
                let _ = self.buf.try_push(cmd);

                let FullFrame { frame, dirty_line } = &mut c.sdisp;

                // Filter out to only the dirty lines, clearing the dirty flag
                let all_lines = dirty_line.iter_mut().zip(frame.iter()).enumerate();
                let dirty_lines = all_lines.filter_map(|(idx, (dirty, line))| {
                    if *dirty {
                        *dirty = false;
                        Some((idx, line))
                    } else {
                        None
                    }
                });


                // Now we need to write all the dirty lines, zip together the dest buffer
                // with our current frame buffer
                for (line, iline) in dirty_lines {
                    // Lines are 1-indexed on the wire
                    let _ = self.buf.try_push((line as u8) + 1);
                    // We keep our internal frame buffer in the same format as the wire
                    let _ = self.buf.try_extend_from_slice(iline);
                    // dummy byte
                    let _ = self.buf.try_push(0);
                    drawn += 1;
                }
            } else {
                // No buffer to write, just toggle vcom
                let mut cmd = commands::TOGGLE_VCOM;
                if c.vcom {
                    cmd |= commands::VCOM_MASK;
                }

                // Write the command
                let _ = self.buf.try_push(cmd);
            }

            // Write one final dummy byte
            let _ = self.buf.try_push(0);

            // Drop the mutex once we're done using the framebuffer data
            drop(c);

            tracing::info!(
                drawn,
                "Drew lines",
            );

            self.buf = self.spim.send_wait(self.buf).await.map_err(drop).unwrap();

            // Wait a reasonable amount of time to redraw
            if self
                .kernel
                .timeout(Duration::from_millis(500), DIRTY.wait())
                .await.is_err()
            {
                tracing::info!("TIMEOUT DING");
            } else {
                tracing::info!("WAITCELL DING");
            }
        }
    }
}

/// This task is spawned by the call to [`SharpDisplay::register`]. It is a single
/// async function that will process requests, and periodically redraw the
/// framebuffer.
struct CommanderTask {
    cmd: KConsumer<Message<EmbDisplayService>>,
    ctxt: Arc<Mutex<Context>>,
    width: u32,
    height: u32,
}

impl CommanderTask {
    /// The entrypoint for the driver execution
    #[tracing::instrument(skip(self))]
    async fn cmd_run(self) {
        // This loop services incoming client requests.
        //
        // Generally, don't handle errors when replying to clients, this indicates that they
        // sent us a message and "hung up" without waiting for a response.
        loop {
            let msg = self.cmd.dequeue_async().await.map_err(drop).unwrap();
            let (req, env, reply_tx) = msg.split();
            match req {
                Request::Draw(FrameChunk::Mono(fc)) => {
                    tracing::info!("Draw Command");
                    self.draw_mono(&fc, &self.ctxt).await;
                    DIRTY.wake_all();
                    let response = env.fill(Ok(Response::DrawComplete(fc.into())));
                    let _ = reply_tx.reply_konly(response).await;
                }
                Request::GetMeta => {
                    let meta = DisplayMetadata {
                        kind: FrameKind::Mono,
                        width: self.width,
                        height: self.height,
                    };
                    let response = env.fill(Ok(Response::FrameMeta(meta)));
                    let _ = reply_tx.reply_konly(response).await;
                }
                _ => {
                    let response = env.fill(Err(FrameError::InternalError));
                    let _ = reply_tx.reply_konly(response).await;
                },
            }
        }
    }

    /// Draw the given MonoChunk to the persistent framebuffer
    async fn draw_mono(&self, fc: &MonoChunk, mutex: &Mutex<Context>) {
        let mut guard = mutex.lock().await;
        let ctx: &mut Context = &mut guard;

        let Context {
            sdisp,
            ..
        } = ctx;

        draw_to(sdisp, fc, self.width, self.height);
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
static DIRTY: WaitQueue = WaitQueue::new();

fn draw_to(dest: &mut FullFrame, src: &MonoChunk, width: u32, height: u32) {
    tracing::info!("drawing to");
    let meta = src.meta();
    let data = src.data();
    let mask = src.mask();

    let start_x = meta.start_x();
    let start_y = meta.start_y();
    let src_width = meta.width();

    if start_y >= height {
        return;
    }
    if start_x >= width {
        return;
    }

    let s = data
        .chunks(src_width as usize)
        .zip(mask.chunks(src_width as usize));

    let before = dest.dirty_line.iter().filter(|b| **b).count();

    for (src_y, (src_data_line, src_mask_line)) in s.enumerate() {
        // Any data on this line?
        if src_mask_line.iter().all(|b| *b == 0) {
            continue;
        }
        let sl = src_data_line.iter().zip(src_mask_line.iter());
        for (src_x, (s_data, s_mask)) in sl.enumerate() {
            if *s_mask != 0 {
                let val = if *s_data < 128 {
                    Gray8::BLACK
                } else {
                    Gray8::WHITE
                };
                dest.set_px(start_x as usize + src_x, start_y as usize + src_y, val);
            }
        }
    }

    let after = dest.dirty_line.iter().filter(|b| **b).count();
    // tracing::info!(made_dirty = (after-before), "gross");

}

