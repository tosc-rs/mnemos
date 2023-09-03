//! Shells
//!
//! This module provides daemons that serve a forth shell

use core::time::Duration;

use crate::{
    comms::bbq::{BidiHandle, GrantR},
    forth::Params,
    services::{
        emb_display::{EmbDisplayClient, FrameLocSize, MonoChunk},
        keyboard::{key_event, KeyClient, KeyClientError},
        serial_mux::{PortHandle, WellKnown},
    },
    Kernel,
};
use embedded_graphics::{
    mono_font::{MonoFont, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::Point,
    primitives::{Line, Primitive, PrimitiveStyle},
    text::Text,
    Drawable,
};

use futures::FutureExt;
use input_mgr::RingLine;
use key_event::KeyEvent;
use profont::PROFONT_12_POINT;

use crate::forth::Forth;

/// Settings for the [sermux_shell] daemon
#[derive(Debug)]
#[non_exhaustive]
pub struct SermuxShellSettings {
    /// Sermux port to serve the shell on
    ///
    /// Defaults to [WellKnown::ForthShell0]
    pub port: u16,
    /// Number of bytes used for the sermux buffer
    ///
    /// Defaults to 256
    pub capacity: usize,
    /// Forth parameters for the shell
    ///
    /// Uses the default value of [Params]
    pub forth_settings: Params,
}

impl Default for SermuxShellSettings {
    fn default() -> Self {
        Self {
            port: WellKnown::ForthShell0.into(),
            capacity: 256,
            forth_settings: Default::default(),
        }
    }
}

/// Spawns a forth shell on the given port
#[tracing::instrument(skip(k))]
pub async fn sermux_shell(k: &'static Kernel, settings: SermuxShellSettings) {
    let SermuxShellSettings {
        port,
        capacity,
        forth_settings,
    } = settings;
    let port = PortHandle::open(k, port, capacity).await.unwrap();
    let (task, tid_io) = Forth::new(k, forth_settings)
        .await
        .expect("Forth spawning must succeed");
    k.spawn(task.run()).await;
    k.spawn(async move {
        loop {
            futures::select_biased! {
                rgr = port.consumer().read_grant().fuse() => {
                    let needed = rgr.len();
                    let mut tid_io_wgr = tid_io.producer().send_grant_exact(needed).await;
                    tid_io_wgr.copy_from_slice(&rgr);
                    tid_io_wgr.commit(needed);
                    rgr.release(needed);
                },
                output = tid_io.consumer().read_grant().fuse() => {
                    let needed = output.len();
                    port.send(&output).await;
                    output.release(needed);
                }
            }
        }
    })
    .await;
}

/// Settings for the [graphical_shell_mono] daemon
///
/// This does NOT implement [Default]. Instead use [GraphicalShellSettings::with_display_size].
///
/// For example:
/// ```skip
/// use kernel::daemons::shells::GraphicalShellSettings;
/// let shell = GraphicalShellSettings {
///     // override the capacity with a larger value:
///     capacity: 512,
///    ..GraphicalShellSettings::with_display_size(420, 69) // nice!
/// };
/// # drop(shell);
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub struct GraphicalShellSettings {
    /// Number of bytes used for the sermux buffer
    ///
    /// Defaults to 256
    pub capacity: usize,
    /// Forth parameters for the shell
    ///
    /// Uses the default value of [Params]
    pub forth_settings: Params,
    /// Display width in pixels
    pub disp_width_px: u32,
    /// Display height in pixels
    pub disp_height_px: u32,
    /// Redraw debounce time
    pub redraw_debounce: Duration,
    /// Font used for the shell
    ///
    /// Defaults to [PROFONT_12_POINT]
    pub font: MonoFont<'static>,
}

impl GraphicalShellSettings {
    pub fn with_display_size(width_px: u32, height_px: u32) -> Self {
        Self {
            capacity: 256,
            forth_settings: Default::default(),
            disp_width_px: width_px,
            disp_height_px: height_px,
            redraw_debounce: Duration::from_millis(50),
            font: PROFONT_12_POINT,
        }
    }
}

/// Spawns a graphical shell using the [EmbDisplayService](crate::services::emb_display::EmbDisplayService) service
// TODO: tracing the `settings` field draws the whole PROFONT_12_POINT, which is hilarious but annoying
#[tracing::instrument(skip(k, settings))]
pub async fn graphical_shell_mono(k: &'static Kernel, settings: GraphicalShellSettings) {
    let GraphicalShellSettings {
        capacity: _cap,
        forth_settings,
        disp_width_px,
        disp_height_px,
        redraw_debounce,
        font,
    } = settings;

    let mut keyboard = KeyClient::from_registry(k, Default::default())
        .await
        .expect("failed to get keyboard service");
    let mut disp_hdl = EmbDisplayClient::from_registry(k)
        .await
        .expect("failed to get EmbDisplayClient");
    let char_y = font.character_size.height;
    let char_x = font.character_size.width + font.character_spacing;

    // Draw titlebar
    {
        let mut fc_0 = MonoChunk::allocate_mono(FrameLocSize {
            height: char_y,
            width: disp_width_px,
            offset_x: 0,
            offset_y: 0,
        })
        .await;

        let text_style = MonoTextStyle::new(&font, BinaryColor::On);
        let text1 = Text::new("mnemOS", Point::new(0, font.baseline as i32), text_style);
        text1.draw(&mut fc_0).unwrap();

        let title = "forth shell";
        let text2 = Text::new(
            title,
            Point::new(
                (disp_width_px as i32) - ((title.len() as u32) * char_x) as i32,
                font.baseline as i32,
            ),
            text_style,
        );
        text2.draw(&mut fc_0).unwrap();

        let line_style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
        Line::new(
            Point {
                x: 0,
                y: font.underline.offset as i32,
            },
            Point {
                x: disp_width_px as i32,
                y: font.underline.offset as i32,
            },
        )
        .into_styled(line_style)
        .draw(&mut fc_0)
        .unwrap();
        disp_hdl.draw(fc_0).await.unwrap();
    }

    let style = ring_drawer::BwStyle {
        background: BinaryColor::Off,
        font: MonoTextStyle::new(&font, BinaryColor::On),
    };

    // At 12-pt font, there is enough room for 16 lines, with 50 chars/line.
    //
    // Leave out 4 for the implicit margin of two characters on each gutter.
    let mut rline = RingLine::<16, 46>::new();

    let (task, tid_io) = Forth::new(k, forth_settings)
        .await
        .expect("Forth spawning must succeed");

    // Spawn the forth task
    k.spawn(task.run()).await;

    let mut fc_0 = MonoChunk::allocate_mono(FrameLocSize {
        offset_x: 0,
        offset_y: char_y,
        width: disp_width_px,
        height: disp_height_px - char_y,
    })
    .await;

    loop {
        // Draw to the display
        ring_drawer::drawer_bw(&mut fc_0, &rline, style.clone()).unwrap();
        fc_0 = disp_hdl.draw_mono(fc_0).await.unwrap();

        // Poll ONCE until there is progress, with unlimited time
        io_poll(PollStyle::OneShot, &mut keyboard, &mut rline, &tid_io).await;

        // SOMETHING happened, so now try and grab as many things as possible
        // until the debounce timer expires
        let _ = k
            .timeout(
                redraw_debounce,
                io_poll(PollStyle::Forever, &mut keyboard, &mut rline, &tid_io),
            )
            .await;
    }
}

#[derive(Debug, Clone, Copy)]
enum Productive {
    No,
    Yes,
}

#[derive(Debug, Clone, Copy)]
enum PollStyle {
    OneShot,
    Forever,
}

/// Poll the IO interfaces until something interesting happens.
///
/// If called with `OneShot` style: return as soon as SOMETHING
/// productive occurs.
///
/// If called with `Forever` style: Never return, requires the
/// use of an outer timeout.
async fn io_poll(
    style: PollStyle,
    keyboard: &mut KeyClient,
    rline: &mut RingLine<16, 46>,
    tid_io: &BidiHandle,
) {
    loop {
        let was_productive = futures::select_biased! {
            event = keyboard.next().fuse() => kbd_event(event, rline, tid_io).await,
            output = tid_io.consumer().read_grant().fuse() => {
                stdout_event(output, rline).await
            }
        };

        if let (Productive::Yes, PollStyle::OneShot) = (was_productive, style) {
            return;
        }
    }
}

async fn stdout_event(output: GrantR, rline: &mut RingLine<16, 46>) -> Productive {
    let len = output.len();
    tracing::trace!(len, "Received output from tid_io");
    for &b in output.iter() {
        // TODO(eliza): what if this errors lol
        if b == b'\n' {
            rline.submit_remote_editing();
        } else {
            let _ = rline.append_remote_char(b);
        }
    }
    output.release(len);
    Productive::Yes
}

async fn kbd_event(
    event: Result<KeyEvent, KeyClientError>,
    rline: &mut RingLine<16, 46>,
    tid_io: &BidiHandle,
) -> Productive {
    let Ok(event) = event else {
        tracing::error!("Keyboard service is dead???");
        return Productive::No;
    };
    tracing::info!(?event);
    if event.kind == key_event::Kind::Released {
        return Productive::No;
    }

    if matches!(
        event.code,
        key_event::KeyCode::Backspace | key_event::KeyCode::Delete
    ) {
        rline.pop_local_char();
        Productive::Yes
    } else {
        let Some(ch) = event.code.into_char() else {
            return Productive::No;
        };
        if !ch.is_ascii() {
            tracing::warn!("skipping non-ASCII character: {ch:?}");
            return Productive::No;
        }

        let b = ch as u8;
        match rline.append_local_char(b) {
            Ok(_) => Productive::Yes,
            // backspace
            Err(_) if b == 0x7F => {
                rline.pop_local_char();
                Productive::Yes
            }
            Err(_) if b == b'\n' => {
                let needed = rline.local_editing_len();
                if needed != 0 {
                    let mut tid_io_wgr = tid_io.producer().send_grant_exact(needed).await;
                    rline.copy_local_editing_to(&mut tid_io_wgr).unwrap();
                    tid_io_wgr.commit(needed);
                    rline.submit_local_editing();
                }
                Productive::Yes
            }
            Err(error) => {
                tracing::warn!(?error, "Error appending char: {ch:?}");
                Productive::No
            }
        }
    }
}
