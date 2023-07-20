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
    mnemos_alloc::containers::{Arc, FixedVec, HeapArray},
    registry::Message,
    services::emb_display::{EmbDisplayService, FrameChunk, FrameError, Request, Response, MonoChunk, FrameKind, DisplayMetadata},
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
const LINE_COMMAND_IDX: usize = 0;
const LINE_COMMAND_BYTES: usize = 1;
const LINE_DUMMY_BYTES: usize = 1;
const LINE_DATA_BYTES: usize = WIDTH_BYTES;
const LINE_BYTES: usize = LINE_COMMAND_BYTES + LINE_DATA_BYTES + LINE_DUMMY_BYTES;

// Every FRAME gets a 1 byte command, all of the lines, and one extra dummy byte
const FRAME_COMMAND_IDX: usize = 0;
const FRAME_COMMAND_BYTES: usize = 1;
const FRAME_DUMMY_BYTES: usize = 1;
const FRAME_DATA_BYTES: usize = HEIGHT * LINE_BYTES;
const FRAME_BYTES: usize = FRAME_COMMAND_BYTES + FRAME_DATA_BYTES + FRAME_DUMMY_BYTES;

mod commands {
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
        let mut linebuf = FixedVec::new(FRAME_BYTES).await;
        for _ in 0..FRAME_BYTES {
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
                width: WIDTH as u32,
                height: HEIGHT as u32,
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
/// buffer, or at a base rate of 2Hz. At the moment, we only do a full redraw to
/// ensure that VCOM is updated periodically.
///
/// ## Room for improvement
///
/// In the future, we can likely send a "no-op" command instead of a full frame
/// redraw if the dirty flag has not been set, as a power optimization or latency
/// improvement. This is not necessary if we are using GPIO VCOM instead of SPI
/// VCOM toggling.
///
/// We could also keep track of "dirty lines" instead of just a whole "dirty frame",
/// and only pull lines that have changed. This would help when typing a on a single
/// line, and only one font-height needs to be redrawn.
///
/// For example: a single line is 52 * 8 bits, or 416 bits, or 208us at 2MHz. For
/// sending an entire 240 line frame, this is 208 * 240us, or 49_920 us. If we can
/// only are updating one 12pt font line, which is 15 pixels tall, this would reduce
/// our sending time from 49.9ms to 3.1ms.
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
            let c = self.ctxt.lock().await;
            // render into the buffer
            let mut cmd = commands::WRITE_LINE;
            if c.vcom {
                cmd |= commands::VCOM_MASK;
            }

            // Write the command
            self.buf.as_slice_mut()[FRAME_COMMAND_IDX] = cmd;

            // Now we need to write all the lines, zip together the dest buffer
            // with our current frame buffer
            let out_lines =
                self.buf.as_slice_mut()[FRAME_COMMAND_BYTES..].chunks_exact_mut(LINE_BYTES);
            let in_lines = c.sdisp.frame.iter();
            let lines = out_lines.zip(in_lines);

            for (line, (oline, iline)) in &mut lines.enumerate() {
                // Lines are 1-indexed on the wire
                oline[LINE_COMMAND_IDX] = (line as u8) + 1;
                // We keep our internal frame buffer in the same format as the wire
                oline[LINE_COMMAND_BYTES..][..LINE_DATA_BYTES].copy_from_slice(iline);
                // We don't need to write the dummy byte for the line
            }

            // Drop the mutex once we're done using the framebuffer data
            drop(c);

            // We don't need to write the (extra) dummy byte for the frame

            self.buf = self.spim.send_wait(self.buf).await.map_err(drop).unwrap();

            // Wait a reasonable amount of time to redraw
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
    ctxt: Arc<Mutex<Context>>,
    width: u32,
    height: u32,
}

impl CommanderTask {
    /// The entrypoint for the driver execution
    #[tracing::instrument(skip(self))]
    async fn cmd_run(mut self) {
        // This loop services incoming client requests.
        //
        // Generally, don't handle errors when replying to clients, this indicates that they
        // sent us a message and "hung up" without waiting for a response.
        loop {
            let msg = self.cmd.dequeue_async().await.map_err(drop).unwrap();
            let (req, env, reply_tx) = msg.split();
            match req {
                Request::Draw(FrameChunk::Mono(fc)) => {
                    if self.draw_mono(&fc, &self.ctxt).await.is_err() {
                        break;
                    } else {
                        let response = env.fill(Ok(Response::DrawComplete(fc.into())));
                        let _ = reply_tx.reply_konly(response).await;
                    }
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
                _ => todo!(),
            }
        }
    }

    /// Draw the given MonoChunk to the persistent framebuffer
    async fn draw_mono(&self, fc: &MonoChunk, mutex: &Mutex<Context>) -> Result<(), ()> {
        let mut guard = mutex.lock().await;
        let ctx: &mut Context = &mut guard;

        let Context {
            sdisp,
            // dirty,
            // framebuf,
            ..
        } = ctx;

        // draw_to(sdisp, fc, self.width, self.height);
        // let raw_img = frame_display(framebuf, self.width).unwrap();
        // let image = Image::new(&raw_img, Point::new(0, 0));
        // image.draw(sdisp).unwrap();
        // *dirty = true;

        Ok(())
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


fn draw_to(dest: &mut FullFrame, src: &MonoChunk, width: u32, height: u32) {
    // let meta = src.meta();
    // let data = src.data();
    // let mask = src.mask();

    // let start_x = meta.start_x();
    // let start_y = meta.start_y();
    // let src_width = meta.width();

    // if start_y >= height {
    //     return;
    // }
    // if start_x >= width {
    //     return;
    // }

    // // Take all destination rows, starting at the start_y line
    // let all_dest_rows = dest.chunks_exact_mut(width as usize);
    // let dest_rows = all_dest_rows.skip(start_y as usize);

    // // Then take all source rows, and zip together the mask bits
    // let all_src_rows = data.chunks(src_width as usize);
    // let all_src_mask_rows = mask.chunks(src_width as usize);
    // let all_src = all_src_rows.zip(all_src_mask_rows);

    // // Combine them together, this gives us automatic "early return"
    // // when either we run out of source rows, or destination rows
    // let zip_rows = dest_rows.zip(all_src);
    // for (dest_row, (src_data, src_mask)) in zip_rows {
    //     // Zip the data and mask lines together so we can use them
    //     let src = src_data.iter().zip(src_mask.iter());

    //     dest_row
    //         .iter_mut()
    //         // Skip to the start of the subframe
    //         .skip(start_x as usize)
    //         // Again, zipping means we stop as soon as we run out of
    //         // source OR destination pixesl on this line
    //         .zip(src)
    //         .filter_map(|(d, (s_d, s_m))| {
    //             // look at the mask, to see if the subframe should modify
    //             // the total frame
    //             if *s_m != 0 {
    //                 Some((d, s_d))
    //             } else {
    //                 None
    //             }
    //         })
    //         .for_each(|(d, s)| {
    //             *d = *s;
    //         });
    // }
    todo!()
}

/// Create and return a Simulator display object from raw pixel data.
///
/// Pixel data is turned into a raw image, and then drawn onto a SimulatorDisplay object
/// This is necessary as a e-g Window only accepts SimulatorDisplay object
/// On a physical display, the raw pixel data can be sent over to the display directly
/// Using the display's device interface
fn frame_display(fc: &HeapArray<u8>, width: u32) -> Result<ImageRaw<Gray8>, ()> {
    let raw_image: ImageRaw<Gray8>;
    // TODO: We use Gray8 instead of BinaryColor here because BinaryColor bitpacks to 1bpp,
    // while we are currently doing 8bpp.
    raw_image = ImageRaw::<Gray8>::new(&fc, width);
    Ok(raw_image)
}
