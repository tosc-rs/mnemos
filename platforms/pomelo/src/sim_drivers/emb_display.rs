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

use std::{
    cell::{OnceCell, RefCell},
    process::exit,
    rc::Rc,
    sync::OnceLock,
    time::Duration,
};

use embedded_graphics::{
    image::{Image, ImageRaw},
    pixelcolor::Gray8,
    prelude::*,
};
use embedded_graphics_web_simulator::{
    display::WebSimulatorDisplay as SimulatorDisplay, output_settings::OutputSettingsBuilder,
};
use futures::channel::mpsc;
use maitake::sync::Mutex;
use mnemos_alloc::containers::{Arc, HeapArray};
use mnemos_kernel::{
    comms::kchannel::{KChannel, KConsumer},
    registry::{Message, OpenEnvelope, ReplyTo},
    services::{
        emb_display::{
            DisplayMetadata, EmbDisplayService, FrameChunk, FrameError, FrameKind, MonoChunk,
            Request, Response,
        },
        keyboard::{
            key_event::{self, KeyCode, Modifiers},
            mux::KeyboardMuxClient,
            KeyEvent,
        },
    },
    Kernel,
};
use tracing::{debug, info, warn};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
/// Implements the [`EmbDisplayService`] driver using the `embedded-graphics`
/// simulator.
pub struct SimDisplay;
use futures::SinkExt;

impl SimDisplay {
    /// Register the driver instance
    ///
    /// Registration will also start the simulated display, meaning that the display
    /// window will appear.
    #[tracing::instrument(skip(kernel))]
    pub async fn register(
        kernel: &'static Kernel,
        width: u32,
        height: u32,
        irq_tx: mpsc::Sender<()>,
    ) -> Result<(), FrameError> {
        tracing::debug!("initializing SimDisplay server ({width}x{height})...");

        // TODO settings.kchannel_depth
        let (cmd_prod, cmd_cons) = KChannel::new_async(2).await.split();
        let commander = CommanderTask {
            cmd: cmd_cons,
            width,
            height,
        };

        kernel.spawn(commander.run(width, height)).await;

        kernel
            .with_registry(|reg| reg.register_konly::<EmbDisplayService>(&cmd_prod))
            .await
            .map_err(|_| FrameError::DisplayAlreadyExists)?;

        // listen for key events
        let closure = Closure::<dyn FnMut(_)>::new(move |event: web_sys::KeyboardEvent| {
            event.prevent_default();
            let key = event.key();
            let event = if key == "Enter" {
                KeyEvent::from_char('\n')
            } else if key == "Backspace" {
                KeyEvent {
                    kind: key_event::Kind::Pressed,
                    modifiers: Modifiers::new(),
                    code: KeyCode::Backspace,
                }
            } else if key.len() == 1 {
                let char = key.chars().nth(0).unwrap();
                KeyEvent::from_char(char)
            } else {
                warn!("unable to handle key event: {key:?}");
                return;
            };
            let mut irq_tx = irq_tx.clone();

            let tx_c: OnceCell<
                std::sync::mpsc::Receiver<(
                    Request,
                    OpenEnvelope<Result<Response, FrameError>>,
                    ReplyTo<EmbDisplayService>,
                )>,
            > = OnceCell::new();
            spawn_local(async move {
                // TODO conflict with sleep logic
                // https://github.com/tosc-rs/mnemos/issues/256
                let mut keymux = KeyboardMuxClient::from_registry(kernel).await;

                // TODO conflict with sleep logic
                match keymux.publish_key(event).await {
                    Ok(_) => {
                        let _ = irq_tx.send(()).await;
                    }
                    Err(e) => warn!("could not publish key event: {e:?}"),
                }
            });
        });
        graphics_container()
            .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
            .unwrap();
        closure.forget();

        info!("SimDisplayServer initialized!");

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
    cmd: KConsumer<Message<EmbDisplayService>>,
    width: u32,
    height: u32,
}

struct Context {
    display: SimulatorDisplay<Gray8>,
    framebuf: HeapArray<u8>,
    width: u32,
    height: u32,
}

impl CommanderTask {
    /// The entrypoint for the driver execution
    async fn run(self, width: u32, height: u32) {
        let output_settings = OutputSettingsBuilder::new()
            .scale(1)
            .pixel_spacing(1)
            .build();

        let bytes = (width * height) as usize;

        let display = SimulatorDisplay::<Gray8>::new(
            (width, height),
            &output_settings,
            Some(graphics_container().as_ref()),
        );
        let framebuf = HeapArray::new(bytes, 0x00).await;
        let context = Context {
            display,
            framebuf,
            width,
            height,
        };
        self.message_loop(context).await;
    }

    /// This loop services incoming client requests.
    ///
    /// Generally, don't handle errors when replying to clients, this indicates that they
    /// sent us a message and "hung up" without waiting for a response.
    async fn message_loop(&self, context: Context) {
        debug!("display message loop");

        let context = Rc::new(RefCell::new(context));

        let (tx, rx): (
            std::sync::mpsc::Sender<(
                Rc<RefCell<Context>>,
                OpenEnvelope<Result<Response, FrameError>>,
                ReplyTo<EmbDisplayService>,
                MonoChunk,
            )>,
            std::sync::mpsc::Receiver<(
                Rc<RefCell<Context>>,
                OpenEnvelope<Result<Response, FrameError>>,
                ReplyTo<EmbDisplayService>,
                MonoChunk,
            )>,
        ) = std::sync::mpsc::channel();

        let rx = Rc::new(RefCell::new(rx));

        let leakme = Closure::<dyn FnMut()>::new({
            let rx = rx.clone();
            move || {
                let rx = rx.borrow_mut();
                while let Ok((context, env, reply_tx, fc)) = rx.try_recv() {
                    if let Ok(_) = draw_mono(&fc, &mut *context.borrow_mut()) {
                        let response = env.fill(Ok(Response::DrawComplete(fc.into())));
                        spawn_local(async {
                            // TODO conflict with sleep logic
                            // https://github.com/tosc-rs/mnemos/issues/256
                            reply_tx.reply_konly(response).await.unwrap();
                        });
                    }
                }
            }
        });
        let l = Box::leak(Box::new(leakme));

        loop {
            let msg = self.cmd.dequeue_async().await.map_err(drop).unwrap();
            let (req, env, reply_tx) = msg.split();

            match req {
                Request::Draw(FrameChunk::Mono(fc)) => {
                    tx.send((context.clone(), env, reply_tx, fc)).unwrap();
                    request_animation_frame(&l);
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
}

/// Draw the given MonoChunk to the persistent framebuffer
fn draw_mono(fc: &MonoChunk, context: &mut Context) -> Result<(), ()> {
    draw_to(&mut context.framebuf, fc, context.width, context.height);
    let raw_img = frame_display(&context.framebuf, context.width).map_err(|_| ())?;
    let image = Image::new(&raw_img, Point::new(0, 0));
    image.draw(&mut context.display).map_err(|_| ())?;
    context.display.flush().ok();
    Ok(())
}

fn request_animation_frame(f: &Closure<dyn FnMut()>) {
    window()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .expect("should register `requestAnimationFrame` OK");
}

fn window() -> web_sys::Window {
    web_sys::window().expect("no global `window` exists")
}

fn document() -> web_sys::Document {
    window()
        .document()
        .expect("should have a document on window")
}

fn graphics_container() -> web_sys::Element {
    document()
        .get_element_by_id("graphics")
        .expect("document should have our text container")
}

// TODO: move to shared helper module - https://github.com/tosc-rs/mnemos/issues/260
// TODO: blocked on e-g update https://github.com/tosc-rs/mnemos/issues/259
pub fn draw_to(dest: &mut HeapArray<u8>, src: &MonoChunk, width: u32, height: u32) {
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

// TODO: move to shared helper module - https://github.com/tosc-rs/mnemos/issues/260
// TODO: blocked on e-g update https://github.com/tosc-rs/mnemos/issues/259

/// Create and return a Simulator display object from raw pixel data.
///
/// Pixel data is turned into a raw image, and then drawn onto a SimulatorDisplay object
/// This is necessary as a e-g Window only accepts SimulatorDisplay object
/// On a physical display, the raw pixel data can be sent over to the display directly
/// Using the display's device interface
pub fn frame_display(fc: &HeapArray<u8>, width: u32) -> Result<ImageRaw<Gray8>, ()> {
    // TODO: We use Gray8 instead of BinaryColor here because BinaryColor bitpacks to 1bpp,
    // while we are currently doing 8bpp.
    Ok(ImageRaw::<Gray8>::new(fc, width))
}
