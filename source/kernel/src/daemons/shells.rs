//! Shells
//!
//! This module provides daemons that serve a forth shell

use core::time::Duration;

use crate::{
    forth::Params,
    services::{
        emb_display::EmbDisplayClient,
        keyboard::{key_event, KeyClient},
        serial_mux::{PortHandle, WellKnown},
    },
    tracing, Kernel,
};
use embedded_graphics::{
    mono_font::{MonoFont, MonoTextStyle},
    pixelcolor::Gray8,
    prelude::{GrayColor, Point},
    primitives::{Line, Primitive, PrimitiveStyle},
    text::Text,
    Drawable,
};

use futures::FutureExt;
use input_mgr::RingLine;
use profont::PROFONT_12_POINT;

use crate::forth::Forth;

/// Settings for the [sermux_shell] daemon
#[derive(Debug)]
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
    /// Hidden for forwards compat
    _priv: (),
}

impl Default for SermuxShellSettings {
    fn default() -> Self {
        Self {
            port: WellKnown::ForthShell0.into(),
            capacity: 256,
            forth_settings: Default::default(),
            _priv: (),
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
        _priv,
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
    /// Font used for the shell
    ///
    /// Defaults to [PROFONT_12_POINT]
    pub font: MonoFont<'static>,
    /// Hidden for forwards compat
    _priv: (),
}

impl GraphicalShellSettings {
    pub fn with_display_size(width_px: u32, height_px: u32) -> Self {
        Self {
            capacity: 256,
            forth_settings: Default::default(),
            disp_width_px: width_px,
            disp_height_px: height_px,
            font: PROFONT_12_POINT,
            _priv: (),
        }
    }
}

/// Spawns a graphical shell using the [EmbDisplayService](crate::services::emb_display::EmbDisplayService) service
#[tracing::instrument(skip(k))]
pub async fn graphical_shell_mono(k: &'static Kernel, settings: GraphicalShellSettings) {
    let GraphicalShellSettings {
        capacity: _cap,
        forth_settings,
        disp_width_px,
        disp_height_px,
        font,
        _priv,
    } = settings;

    let mut keyboard = KeyClient::from_registry(k, Default::default()).await;
    let mut disp_hdl = EmbDisplayClient::from_registry(k).await;
    let char_y = font.character_size.height;
    let char_x = font.character_size.width + font.character_spacing;

    // Draw titlebar
    {
        let mut fc_0 = disp_hdl
            .get_framechunk(0, 0, disp_width_px, char_y)
            .await
            .unwrap();
        let text_style = MonoTextStyle::new(&font, Gray8::WHITE);
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

        let line_style = PrimitiveStyle::with_stroke(Gray8::WHITE, 1);
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
        disp_hdl.draw_framechunk(fc_0).await.unwrap();
    }

    let style = ring_drawer::BwStyle {
        background: Gray8::BLACK,
        font: MonoTextStyle::new(&font, Gray8::WHITE),
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

    loop {
        // Wait until there is a frame buffer ready. There wouldn't be if we've spammed frames
        // before they've been consumed.
        let mut fc_0 = loop {
            let fc = disp_hdl
                .get_framechunk(0, char_y as i32, disp_width_px, disp_height_px - char_y)
                .await;
            if let Some(fc) = fc {
                break fc;
            } else {
                k.sleep(Duration::from_millis(10)).await;
            }
        };
        ring_drawer::drawer_bw(&mut fc_0, &rline, style.clone()).unwrap();
        disp_hdl.draw_framechunk(fc_0).await.unwrap();

        loop {
            // loop *just* for skipping released key events
            futures::select_biased! {
                event = keyboard.next().fuse() => {

                    let Some(event) = event else {
                        tracing::error!("Keyboard service is dead???");
                        continue;
                    };
                    if event.kind == key_event::Kind::Released {
                        continue;
                    }
                    let Some(ch) = event.code.into_char() else {
                        continue;
                    };
                    if !ch.is_ascii() {
                        tracing::warn!("skipping non-ASCII character: {ch:?}");
                        continue;
                    }

                    let b = ch as u8;
                    match rline.append_local_char(b) {
                        Ok(_) => {}
                        // backspace
                        Err(_) if b == 0x7F => rline.pop_local_char(),
                        Err(_) if b == b'\n' => {
                            let needed = rline.local_editing_len();
                            if needed != 0 {
                                let mut tid_io_wgr = tid_io.producer().send_grant_exact(needed).await;
                                rline.copy_local_editing_to(&mut tid_io_wgr).unwrap();
                                tid_io_wgr.commit(needed);
                                rline.submit_local_editing();
                            }
                        }
                        Err(error) => {
                            tracing::warn!(?error, "Error appending char: {ch:?}");
                        }
                    }
                },
                output = tid_io.consumer().read_grant().fuse() => {
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
                }
            }

            break;
        }
    }
}
