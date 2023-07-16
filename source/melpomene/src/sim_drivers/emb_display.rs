//! Simulated display driver
//!
//! This is an early attempt at a "frame buffer" style display driver. It uses the
//! embedded-graphics simulator crate to act as a display in simulated environments.
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

use std::time::Duration;

use embedded_graphics::{
    image::{Image, ImageRaw},
    pixelcolor::Gray8,
    prelude::*,
};
use embedded_graphics_simulator::{
    BinaryColorTheme, OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
};
use maitake::sync::Mutex;
use mnemos_alloc::containers::{Arc, HeapArray};
use mnemos_kernel::{
    comms::kchannel::{KChannel, KConsumer},
    registry::Message,
    services::emb_display::{
        DisplayMetadata, EmbDisplayService, FrameChunk, FrameError, FrameKind, MonoChunk, Request,
        Response,
    },
    Kernel,
};

/// Implements the [`EmbDisplayService`] driver using the `embedded-graphics`
/// simulator.
pub struct SimDisplay;

impl SimDisplay {
    /// Register the driver instance
    ///
    /// Registration will also start the simulated display, meaning that the display
    /// window will appear.
    #[tracing::instrument(skip(kernel))]
    pub async fn register(
        kernel: &'static Kernel,
        max_frames: usize,
        width: u32,
        height: u32,
    ) -> Result<(), FrameError> {
        tracing::debug!("initializing SimDisplay server ({width}x{height})...");

        let (cmd_prod, cmd_cons) = KChannel::new_async(1).await.split();
        let commander = CommanderTask {
            kernel,
            cmd: cmd_cons,
            width,
            height,
        };

        kernel.spawn(commander.run(width, height)).await;

        kernel
            .with_registry(|reg| reg.register_konly::<EmbDisplayService>(&cmd_prod))
            .await
            .map_err(|_| FrameError::DisplayAlreadyExists)?;

        tracing::info!("SimDisplayServer initialized!");

        Ok(())
    }
}

//////////////////////////////////////////////////////////////////////////////
// CommanderTask - This is the "driver server"
//////////////////////////////////////////////////////////////////////////////

/// This task is spawned by the call to [`SimDisplay::register`]. It is a single
/// async function that will process requests, and periodically redraw the
/// framebuffer.
struct CommanderTask {
    kernel: &'static Kernel,
    cmd: KConsumer<Message<EmbDisplayService>>,
    width: u32,
    height: u32,
}

struct Context {
    sdisp: SimulatorDisplay<Gray8>,
    framebuf: HeapArray<u8>,
    window: Window,
    dirty: bool,
}

impl CommanderTask {
    /// The entrypoint for the driver execution
    async fn run(self, width: u32, height: u32) {
        let output_settings = OutputSettingsBuilder::new()
            .theme(BinaryColorTheme::OledBlue)
            .scale(1)
            .build();

        let bytes = (width * height) as usize;

        // Create a mutex for the embedded graphics simulator objects.
        //
        // We do this because if we don't call "update" regularly, the window just
        // sort of freezes. We also make the update loop check for "quit" events,
        // because otherwise the gui window just swallows all the control-c events,
        // which means you have to send a sigkill to actually get the simulator to
        // fully stop.
        //
        // The update loop *needs* to drop the egsim items, otherwise they just exist
        // in the mutex until the next time a frame is displayed, which right now is
        // only whenever line characters actually arrive.
        let sdisp = SimulatorDisplay::<Gray8>::new(Size::new(width, height));
        let window = Window::new("mnemOS", &output_settings);
        let framebuf = HeapArray::new(bytes, 0x00).await;
        let mutex = Arc::new(Mutex::new(Some(Context {
            sdisp,
            framebuf,
            window,
            dirty: true,
        })))
        .await;

        // Spawn a task that draws the framebuffer at a regular rate of 15Hz.
        self.kernel
            .spawn({
                let mutex = mutex.clone();
                render_loop(self.kernel, mutex)
            })
            .await;

        self.message_loop(mutex).await;
    }

    /// This loop services incoming client requests.
    ///
    /// Generally, don't handle errors when replying to clients, this indicates that they
    /// sent us a message and "hung up" without waiting for a response.
    async fn message_loop(&self, mutex: Arc<Mutex<Option<Context>>>) {
        loop {
            let msg = self.cmd.dequeue_async().await.map_err(drop).unwrap();
            let (req, env, reply_tx) = msg.split();
            match req {
                Request::Draw(FrameChunk::Mono(fc)) => {
                    if self.draw_mono(&fc, &mutex).await.is_err() {
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
    async fn draw_mono(&self, fc: &MonoChunk, mutex: &Mutex<Option<Context>>) -> Result<(), ()> {
        let mut guard = mutex.lock().await;
        let ctx = if let Some(c) = (&mut *guard).as_mut() {
            c
        } else {
            return Err(());
        };

        let Context {
            sdisp,
            dirty,
            framebuf,
            ..
        } = ctx;

        draw_to(framebuf, fc, self.width, self.height);
        let raw_img = frame_display(framebuf, self.width).unwrap();
        let image = Image::new(&raw_img, Point::new(0, 0));
        image.draw(sdisp).unwrap();
        *dirty = true;

        Ok(())
    }
}

async fn render_loop(kernel: &'static Kernel, mutex: Arc<Mutex<Option<Context>>>) {
    let mut idle_ticks = 0;
    loop {
        kernel.sleep(Duration::from_micros(1_000_000 / 15)).await;
        let mut guard = mutex.lock().await;
        let mut done = false;
        if let Some(Context {
            sdisp,
            window,
            dirty,
            ..
        }) = (&mut *guard).as_mut()
        {
            // If nothing has been drawn, only update the frame at 5Hz to save
            // CPU usage
            if *dirty || idle_ticks >= 3 {
                idle_ticks = 0;
                *dirty = false;
                window.update(&sdisp);
            } else {
                idle_ticks += 1;
            }

            if window.events().any(|e| e == SimulatorEvent::Quit) {
                done = true;
            }
        } else {
            done = true;
        }
        if done {
            let _ = guard.take();
            break;
        }
    }
}

fn draw_to(dest: &mut HeapArray<u8>, src: &MonoChunk, width: u32, height: u32) {
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

    // Take all destination rows, starting at the start_y line
    let all_dest_rows = dest.chunks_exact_mut(width as usize);
    let dest_rows = all_dest_rows.skip(start_y as usize);

    // Then take all source rows, and zip together the mask bits
    let all_src_rows = data.chunks(src_width as usize);
    let all_src_mask_rows = mask.chunks(src_width as usize);
    let all_src = all_src_rows.zip(all_src_mask_rows);

    // Combine them together, this gives us automatic "early return"
    // when either we run out of source rows, or destination rows
    let zip_rows = dest_rows.zip(all_src);
    for (dest_row, (src_data, src_mask)) in zip_rows {
        // Zip the data and mask lines together so we can use them
        let src = src_data.iter().zip(src_mask.iter());

        dest_row
            .iter_mut()
            // Skip to the start of the subframe
            .skip(start_x as usize)
            // Again, zipping means we stop as soon as we run out of
            // source OR destination pixesl on this line
            .zip(src)
            .filter_map(|(d, (s_d, s_m))| {
                // look at the mask, to see if the subframe should modify
                // the total frame
                if *s_m != 0 {
                    Some((d, s_d))
                } else {
                    None
                }
            })
            .for_each(|(d, s)| {
                *d = *s;
            });
    }
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
