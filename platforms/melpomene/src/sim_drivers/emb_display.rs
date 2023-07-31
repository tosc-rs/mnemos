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

use std::{process::exit, time::Duration};

use embedded_graphics::{
    image::{Image, ImageRaw},
    pixelcolor::Gray8,
    prelude::*,
};
use embedded_graphics_simulator::{
    sdl2::{Keycode, Mod},
    BinaryColorTheme, OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
};
use maitake::sync::Mutex;
use mnemos_alloc::containers::{Arc, HeapArray};
use mnemos_kernel::{
    comms::kchannel::{KChannel, KConsumer},
    registry::Message,
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

        let (cmd_prod, cmd_cons) = KChannel::new_async(2).await.split();
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

async fn handle_key_event(kmc: &mut KeyboardMuxClient, evt: SimulatorEvent) -> bool {
    match evt {
        SimulatorEvent::KeyDown {
            keycode,
            keymod,
            repeat,
        } => {
            tracing::trace!(?evt, "Got key event from Simulator");
            if let Some(k) = sim_key_to_key_event(keycode, keymod, repeat, true) {
                kmc.publish_key(k).await.is_err()
            } else {
                false
            }
        }
        SimulatorEvent::KeyUp {
            keycode,
            keymod,
            repeat,
        } => {
            tracing::trace!(?evt, "Got key event from Simulator");
            if let Some(k) = sim_key_to_key_event(keycode, keymod, repeat, false) {
                kmc.publish_key(k).await.is_err()
            } else {
                false
            }
        }
        SimulatorEvent::Quit => true,
        _ => false,
    }
}

async fn render_loop(kernel: &'static Kernel, mutex: Arc<Mutex<Option<Context>>>) {
    let mut idle_ticks = 0;

    let mut keymux = KeyboardMuxClient::from_registry(kernel).await;
    let mut first_done = false;
    loop {
        kernel.sleep(Duration::from_micros(1_000_000 / 20)).await;
        let mut guard = mutex.lock().await;
        let mut done = false;
        if let Some(Context {
            sdisp,
            window,
            dirty,
            ..
        }) = (&mut *guard).as_mut()
        {
            // We can't poll the events until the first draw, or we'll panic.
            // But once we have: we want to always process events, even if there
            // is nothing to draw, to potentially feed the keymux or catch
            // a "time to die" event.
            if first_done {
                for evt in window.events().into_iter() {
                    if handle_key_event(&mut keymux, evt).await {
                        done = true;
                    }
                }
            }

            // If nothing has been drawn, only update the frame at 5Hz to save
            // CPU usage
            if *dirty || idle_ticks >= 4 {
                idle_ticks = 0;
                *dirty = false;
                window.update(&sdisp);
                first_done = true;
            } else {
                idle_ticks += 1;
            }
        } else {
            done = true;
        }
        if done {
            tracing::warn!("Display closed, stopping melpomene");
            kernel.sleep(Duration::from_millis(100)).await;
            exit(0);
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

fn sim_key_to_key_event(
    keycode: Keycode,
    keymod: Mod,
    repeat: bool,
    is_down: bool,
) -> Option<KeyEvent> {
    let key_kind = match (repeat, is_down) {
        (true, true) => key_event::Kind::Held,
        (false, true) => key_event::Kind::Pressed,
        (_, false) => key_event::Kind::Released,
    };

    let mut modi = Modifiers::new();

    if (keymod & Mod::LSHIFTMOD) != Mod::NOMOD {
        modi.set(Modifiers::SHIFT, true);
    }
    if (keymod & Mod::RSHIFTMOD) != Mod::NOMOD {
        modi.set(Modifiers::SHIFT, true);
    }
    if (keymod & Mod::LCTRLMOD) != Mod::NOMOD {
        modi.set(Modifiers::CTRL, true);
    }
    if (keymod & Mod::RCTRLMOD) != Mod::NOMOD {
        modi.set(Modifiers::CTRL, true);
    }
    if (keymod & Mod::LALTMOD) != Mod::NOMOD {
        modi.set(Modifiers::ALT, true);
    }
    if (keymod & Mod::RALTMOD) != Mod::NOMOD {
        modi.set(Modifiers::ALT, true);
    }
    if (keymod & Mod::LGUIMOD) != Mod::NOMOD {
        modi.set(Modifiers::META, true);
    }
    if (keymod & Mod::RGUIMOD) != Mod::NOMOD {
        modi.set(Modifiers::META, true);
    }
    if (keymod & Mod::NUMMOD) != Mod::NOMOD {
        modi.set(Modifiers::NUMLOCK, true);
    }
    if (keymod & Mod::CAPSMOD) != Mod::NOMOD {
        modi.set(Modifiers::CAPSLOCK, true);
    }
    if (keymod & Mod::MODEMOD) != Mod::NOMOD {
        tracing::warn!("Modemod not supported");
    }
    if (keymod & Mod::RESERVEDMOD) != Mod::NOMOD {
        tracing::warn!("Reservedmod not supported");
    }

    let upper = modi.get(Modifiers::SHIFT) || modi.get(Modifiers::CAPSLOCK);

    // Whew this is something.
    //
    // TODO(eliza): Fix this once keymux handles meta characters!
    let code: KeyCode = match keycode {
        Keycode::Backspace => KeyCode::Backspace,
        Keycode::Tab => KeyCode::Tab,
        Keycode::Return => KeyCode::Enter,
        Keycode::Escape => KeyCode::Esc,
        Keycode::Space => KeyCode::Char(' '),
        Keycode::Semicolon => KeyCode::Char(if upper { ':' } else { ';' }),
        Keycode::Less => KeyCode::Char('<'),
        Keycode::Equals => KeyCode::Char(if upper { '+' } else { '=' }),
        Keycode::Greater => KeyCode::Char('>'),
        Keycode::Question => KeyCode::Char('?'),
        Keycode::At => KeyCode::Char('@'),
        Keycode::Caret => KeyCode::Char('^'),
        Keycode::Underscore => KeyCode::Char('_'),
        Keycode::A => KeyCode::Char(if upper { 'A' } else { 'a' }),
        Keycode::B => KeyCode::Char(if upper { 'B' } else { 'b' }),
        Keycode::C => KeyCode::Char(if upper { 'C' } else { 'c' }),
        Keycode::D => KeyCode::Char(if upper { 'D' } else { 'd' }),
        Keycode::E => KeyCode::Char(if upper { 'E' } else { 'e' }),
        Keycode::F => KeyCode::Char(if upper { 'F' } else { 'f' }),
        Keycode::G => KeyCode::Char(if upper { 'G' } else { 'g' }),
        Keycode::H => KeyCode::Char(if upper { 'H' } else { 'h' }),
        Keycode::I => KeyCode::Char(if upper { 'I' } else { 'i' }),
        Keycode::J => KeyCode::Char(if upper { 'J' } else { 'j' }),
        Keycode::K => KeyCode::Char(if upper { 'K' } else { 'k' }),
        Keycode::L => KeyCode::Char(if upper { 'L' } else { 'l' }),
        Keycode::M => KeyCode::Char(if upper { 'M' } else { 'm' }),
        Keycode::N => KeyCode::Char(if upper { 'N' } else { 'n' }),
        Keycode::O => KeyCode::Char(if upper { 'O' } else { 'o' }),
        Keycode::P => KeyCode::Char(if upper { 'P' } else { 'p' }),
        Keycode::Q => KeyCode::Char(if upper { 'Q' } else { 'q' }),
        Keycode::R => KeyCode::Char(if upper { 'R' } else { 'r' }),
        Keycode::S => KeyCode::Char(if upper { 'S' } else { 's' }),
        Keycode::T => KeyCode::Char(if upper { 'T' } else { 't' }),
        Keycode::U => KeyCode::Char(if upper { 'U' } else { 'u' }),
        Keycode::V => KeyCode::Char(if upper { 'V' } else { 'v' }),
        Keycode::W => KeyCode::Char(if upper { 'W' } else { 'w' }),
        Keycode::X => KeyCode::Char(if upper { 'X' } else { 'x' }),
        Keycode::Y => KeyCode::Char(if upper { 'Y' } else { 'y' }),
        Keycode::Z => KeyCode::Char(if upper { 'Z' } else { 'z' }),
        Keycode::Delete => KeyCode::Delete,
        Keycode::F1 => KeyCode::F(1),
        Keycode::F2 => KeyCode::F(2),
        Keycode::F3 => KeyCode::F(3),
        Keycode::F4 => KeyCode::F(4),
        Keycode::F5 => KeyCode::F(5),
        Keycode::F6 => KeyCode::F(6),
        Keycode::F7 => KeyCode::F(7),
        Keycode::F8 => KeyCode::F(8),
        Keycode::F9 => KeyCode::F(9),
        Keycode::F10 => KeyCode::F(10),
        Keycode::F11 => KeyCode::F(11),
        Keycode::F12 => KeyCode::F(12),
        Keycode::F13 => KeyCode::F(13),
        Keycode::F14 => KeyCode::F(14),
        Keycode::F15 => KeyCode::F(15),
        Keycode::F16 => KeyCode::F(16),
        Keycode::F17 => KeyCode::F(17),
        Keycode::F18 => KeyCode::F(18),
        Keycode::F19 => KeyCode::F(19),
        Keycode::F20 => KeyCode::F(20),
        Keycode::F21 => KeyCode::F(21),
        Keycode::F22 => KeyCode::F(22),
        Keycode::F23 => KeyCode::F(23),
        Keycode::F24 => KeyCode::F(24),
        Keycode::PrintScreen => KeyCode::PrintScreen,
        Keycode::Pause => KeyCode::Pause,
        Keycode::Insert => KeyCode::Insert,
        Keycode::Home => KeyCode::Home,
        Keycode::PageUp => KeyCode::PageUp,
        Keycode::End => KeyCode::End,
        Keycode::PageDown => KeyCode::PageDown,
        Keycode::Right => KeyCode::Right,
        Keycode::Left => KeyCode::Left,
        Keycode::Down => KeyCode::Down,
        Keycode::Up => KeyCode::Up,
        Keycode::Num0 => KeyCode::Char(if upper { ')' } else { '0' }),
        Keycode::Num1 => KeyCode::Char(if upper { '!' } else { '1' }),
        Keycode::Num2 => KeyCode::Char(if upper { '@' } else { '2' }),
        Keycode::Num3 => KeyCode::Char(if upper { '#' } else { '3' }),
        Keycode::Num4 => KeyCode::Char(if upper { '$' } else { '4' }),
        Keycode::Num5 => KeyCode::Char(if upper { '%' } else { '5' }),
        Keycode::Num6 => KeyCode::Char(if upper { '^' } else { '6' }),
        Keycode::Num7 => KeyCode::Char(if upper { '&' } else { '7' }),
        Keycode::Num8 => KeyCode::Char(if upper { '*' } else { '8' }),
        Keycode::Num9 => KeyCode::Char(if upper { '(' } else { '9' }),
        Keycode::Quote => KeyCode::Char(if upper { '"' } else { '\'' }),
        Keycode::LeftBracket => KeyCode::Char(if upper { '{' } else { '[' }),
        Keycode::Backslash => KeyCode::Char(if upper { '|' } else { '\\' }),
        Keycode::RightBracket => KeyCode::Char(if upper { '}' } else { ']' }),
        Keycode::Backquote => KeyCode::Char(if upper { '~' } else { '`' }),
        Keycode::Plus => KeyCode::Char('+'),
        Keycode::Comma => KeyCode::Char(if upper { '<' } else { ',' }),
        Keycode::Minus => KeyCode::Char(if upper { '_' } else { '-' }),
        Keycode::Period => KeyCode::Char(if upper { '>' } else { '.' }),
        Keycode::Slash => KeyCode::Char(if upper { '?' } else { '/' }),

        // Ignored - these show up as meta keys
        Keycode::LCtrl => return None,
        Keycode::LShift => return None,
        Keycode::LAlt => return None,
        Keycode::LGui => return None,
        Keycode::RCtrl => return None,
        Keycode::RShift => return None,
        Keycode::RAlt => return None,
        Keycode::RGui => return None,
        Keycode::Mode => return None,

        // TODO
        // Keycode::Exclaim => todo!(),
        // Keycode::Quotedbl => todo!(),
        // Keycode::Hash => todo!(),
        // Keycode::Dollar => todo!(),
        // Keycode::Percent => todo!(),
        // Keycode::Ampersand => todo!(),
        // Keycode::LeftParen => todo!(),
        // Keycode::RightParen => todo!(),
        // Keycode::Asterisk => todo!(),
        // Keycode::Colon => todo!(),
        // Keycode::CapsLock => todo!(),
        // Keycode::ScrollLock => todo!(),
        // Keycode::NumLockClear => todo!(),
        // Keycode::KpDivide => todo!(),
        // Keycode::KpMultiply => todo!(),
        // Keycode::KpMinus => todo!(),
        // Keycode::KpPlus => todo!(),
        // Keycode::KpEnter => todo!(),
        // Keycode::Kp1 => todo!(),
        // Keycode::Kp2 => todo!(),
        // Keycode::Kp3 => todo!(),
        // Keycode::Kp4 => todo!(),
        // Keycode::Kp5 => todo!(),
        // Keycode::Kp6 => todo!(),
        // Keycode::Kp7 => todo!(),
        // Keycode::Kp8 => todo!(),
        // Keycode::Kp9 => todo!(),
        // Keycode::Kp0 => todo!(),
        // Keycode::KpPeriod => todo!(),
        // Keycode::Application => todo!(),
        // Keycode::Power => todo!(),
        // Keycode::KpEquals => todo!(),
        // Keycode::Execute => todo!(),
        // Keycode::Help => todo!(),
        // Keycode::Menu => todo!(),
        // Keycode::Select => todo!(),
        // Keycode::Stop => todo!(),
        // Keycode::Again => todo!(),
        // Keycode::Undo => todo!(),
        // Keycode::Cut => todo!(),
        // Keycode::Copy => todo!(),
        // Keycode::Paste => todo!(),
        // Keycode::Find => todo!(),
        // Keycode::Mute => todo!(),
        // Keycode::VolumeUp => todo!(),
        // Keycode::VolumeDown => todo!(),
        // Keycode::KpComma => todo!(),
        // Keycode::KpEqualsAS400 => todo!(),
        // Keycode::AltErase => todo!(),
        // Keycode::Sysreq => todo!(),
        // Keycode::Cancel => todo!(),
        // Keycode::Clear => todo!(),
        // Keycode::Prior => todo!(),
        // Keycode::Return2 => todo!(),
        // Keycode::Separator => todo!(),
        // Keycode::Out => todo!(),
        // Keycode::Oper => todo!(),
        // Keycode::ClearAgain => todo!(),
        // Keycode::CrSel => todo!(),
        // Keycode::ExSel => todo!(),
        // Keycode::Kp00 => todo!(),
        // Keycode::Kp000 => todo!(),
        // Keycode::ThousandsSeparator => todo!(),
        // Keycode::DecimalSeparator => todo!(),
        // Keycode::CurrencyUnit => todo!(),
        // Keycode::CurrencySubUnit => todo!(),
        // Keycode::KpLeftParen => todo!(),
        // Keycode::KpRightParen => todo!(),
        // Keycode::KpLeftBrace => todo!(),
        // Keycode::KpRightBrace => todo!(),
        // Keycode::KpTab => todo!(),
        // Keycode::KpBackspace => todo!(),
        // Keycode::KpA => todo!(),
        // Keycode::KpB => todo!(),
        // Keycode::KpC => todo!(),
        // Keycode::KpD => todo!(),
        // Keycode::KpE => todo!(),
        // Keycode::KpF => todo!(),
        // Keycode::KpXor => todo!(),
        // Keycode::KpPower => todo!(),
        // Keycode::KpPercent => todo!(),
        // Keycode::KpLess => todo!(),
        // Keycode::KpGreater => todo!(),
        // Keycode::KpAmpersand => todo!(),
        // Keycode::KpDblAmpersand => todo!(),
        // Keycode::KpVerticalBar => todo!(),
        // Keycode::KpDblVerticalBar => todo!(),
        // Keycode::KpColon => todo!(),
        // Keycode::KpHash => todo!(),
        // Keycode::KpSpace => todo!(),
        // Keycode::KpAt => todo!(),
        // Keycode::KpExclam => todo!(),
        // Keycode::KpMemStore => todo!(),
        // Keycode::KpMemRecall => todo!(),
        // Keycode::KpMemClear => todo!(),
        // Keycode::KpMemAdd => todo!(),
        // Keycode::KpMemSubtract => todo!(),
        // Keycode::KpMemMultiply => todo!(),
        // Keycode::KpMemDivide => todo!(),
        // Keycode::KpPlusMinus => todo!(),
        // Keycode::KpClear => todo!(),
        // Keycode::KpClearEntry => todo!(),
        // Keycode::KpBinary => todo!(),
        // Keycode::KpOctal => todo!(),
        // Keycode::KpDecimal => todo!(),
        // Keycode::KpHexadecimal => todo!(),
        // Keycode::AudioNext => todo!(),
        // Keycode::AudioPrev => todo!(),
        // Keycode::AudioStop => todo!(),
        // Keycode::AudioPlay => todo!(),
        // Keycode::AudioMute => todo!(),
        // Keycode::MediaSelect => todo!(),
        // Keycode::Www => todo!(),
        // Keycode::Mail => todo!(),
        // Keycode::Calculator => todo!(),
        // Keycode::Computer => todo!(),
        // Keycode::AcSearch => todo!(),
        // Keycode::AcHome => todo!(),
        // Keycode::AcBack => todo!(),
        // Keycode::AcForward => todo!(),
        // Keycode::AcStop => todo!(),
        // Keycode::AcRefresh => todo!(),
        // Keycode::AcBookmarks => todo!(),
        // Keycode::BrightnessDown => todo!(),
        // Keycode::BrightnessUp => todo!(),
        // Keycode::DisplaySwitch => todo!(),
        // Keycode::KbdIllumToggle => todo!(),
        // Keycode::KbdIllumDown => todo!(),
        // Keycode::KbdIllumUp => todo!(),
        // Keycode::Eject => todo!(),
        // Keycode::Sleep => todo!(),
        other => {
            tracing::error!(key = other.to_string(), "Key not supported!",);
            return None;
        }
    };

    Some(KeyEvent {
        kind: key_kind,
        modifiers: modi,
        code,
    })
}
