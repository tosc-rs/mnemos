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
    pixelcolor::BinaryColor,
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
    Kernel,
};

use super::embd2_svc::{
    EmbDisplay2Service, FrameBufMeta, FrameChunk, FrameError, FrameMeta, MonoChunk, Request,
    Response,
};

/// Implements the [`EmbDisplay2Service`] driver using the `embedded-graphics`
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
        };

        kernel.spawn(commander.run(width, height)).await;

        kernel
            .with_registry(|reg| reg.register_konly::<EmbDisplay2Service>(&cmd_prod))
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
    cmd: KConsumer<Message<EmbDisplay2Service>>,
}

struct Context {
    sdisp: SimulatorDisplay<BinaryColor>,
    framebuf: HeapArray<u8>,
    window: Window,
    dirty: bool,
    width: u32,
    height: u32,
}

impl CommanderTask {
    /// The entrypoint for the driver execution
    async fn run(mut self, width: u32, height: u32) {
        let output_settings = OutputSettingsBuilder::new()
            .theme(BinaryColorTheme::OledBlue)
            .build();

        let bytes = (((width + 7) / 8) * height) as usize;

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
        let sdisp = SimulatorDisplay::<BinaryColor>::new(Size::new(width, height));
        let window = Window::new("mnemOS", &output_settings);
        let framebuf = HeapArray::new(bytes, 0).await;
        let mutex = Arc::new(Mutex::new(Some(Context {
            sdisp,
            framebuf,
            window,
            dirty: true,
            width,
            height,
        })))
        .await;

        // Spawn a task that draws the framebuffer at a regular rate of 15Hz.
        self.kernel
            .spawn({
                let mutex = mutex.clone();
                async move {
                    let mut idle_ticks = 0;
                    loop {
                        self.kernel
                            .sleep(Duration::from_micros(1_000_000 / 15))
                            .await;
                        let mut guard = mutex.lock().await;
                        let mut done = false;
                        if let Some(Context {
                            sdisp,
                            window,
                            dirty,
                            framebuf,
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
            })
            .await;

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
                Request::Draw(FrameChunk::Mono(fc)) => {
                    let mut guard = mutex.lock().await;
                    if let Some(Context {
                        sdisp,
                        dirty,
                        framebuf,
                        width,
                        height,
                        ..
                    }) = (&mut *guard).as_mut()
                    {
                        draw_to(framebuf, fc, *width, *height);
                        let raw_img = frame_display(framebuf, *width).unwrap();
                        let image = Image::new(&raw_img, Point::new(0, 0));
                        image.draw(sdisp).unwrap();
                        *dirty = true;

                        // Drop the guard before we reply so we don't hold it too long.
                        drop(guard);

                        let _ = reply
                            .reply_konly(req.reply_with_body(|fc| {
                                // hate this
                                let frame = match fc {
                                    Request::GetMeta => todo!(),
                                    Request::Draw(fc) => fc,
                                };
                                Ok(Response::DrawComplete(frame))
                            }))
                            .await;
                    } else {
                        break;
                    }
                }
                Request::GetMeta => todo!(),
                _ => todo!(),
            }
        }
    }
}

fn draw_to(dest: &mut HeapArray<u8>, src: &MonoChunk, width: u32, height: u32) {
    let MonoChunk {
        meta:
            FrameBufMeta {
                start_x,
                start_y,
                width: src_width,
                height: src_height,
            },
        data,
        mask,
    } = src;

    if *start_y >= height {
        return;
    }
    if *start_x >= width {
        return;
    }

    let max_y = (*start_y + *src_height).min(height);
    let max_x = (*start_x + *src_width).min(width);

    // Well this is just terrible.
    for y in *start_y..max_y {
        let src_y = y - *start_y;
        for x in *start_x..max_x {
            let src_x = x - *start_x;
        }
    }

}

/// Create and return a Simulator display object from raw pixel data.
///
/// Pixel data is turned into a raw image, and then drawn onto a SimulatorDisplay object
/// This is necessary as a e-g Window only accepts SimulatorDisplay object
/// On a physical display, the raw pixel data can be sent over to the display directly
/// Using the display's device interface
fn frame_display(fc: &HeapArray<u8>, width: u32) -> Result<ImageRaw<BinaryColor>, ()> {
    let raw_image: ImageRaw<BinaryColor>;
    raw_image = ImageRaw::<BinaryColor>::new(&fc, width);
    Ok(raw_image)
}
