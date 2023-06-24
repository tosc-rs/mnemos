use core::time::Duration;

use crate::drivers::emb_display::EmbDisplayClient;
use crate::drivers::serial_mux::PortHandle;
use crate::tracing;
use crate::Kernel;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Gray8;
use embedded_graphics::prelude::GrayColor;
use embedded_graphics::prelude::Point;
use embedded_graphics::primitives::Line;
use embedded_graphics::primitives::Primitive;
use embedded_graphics::primitives::PrimitiveStyle;
use embedded_graphics::text::Text;
use embedded_graphics::Drawable;

use futures::FutureExt;
use input_mgr::RingLine;
use profont::PROFONT_12_POINT;

// ----

// .instrument(tracing::info_span!("Update clock")),
pub async fn graphical_shell_mono(
    k: &'static Kernel,
    disp_width_px: u32,
    disp_height_px: u32,
    port: u16,
    capacity: usize,
) {
    // TODO: Reconsider using a sermux port here once we have a more real keyboard thing
    let port = PortHandle::open(k, port, capacity).await.unwrap();

    let mut disp_hdl = EmbDisplayClient::from_registry(k).await;
    let char_y = PROFONT_12_POINT.character_size.height;
    let char_x = PROFONT_12_POINT.character_size.width + PROFONT_12_POINT.character_spacing;

    // Draw titlebar
    {
        let mut fc_0 = disp_hdl
            .get_framechunk(0, 0, disp_width_px, char_y)
            .await
            .unwrap();
        let text_style = MonoTextStyle::new(&PROFONT_12_POINT, Gray8::WHITE);
        let text1 = Text::new(
            "mnemOS",
            Point::new(0, PROFONT_12_POINT.baseline as i32),
            text_style,
        );
        text1.draw(&mut fc_0).unwrap();

        let title = "forth shell";
        let text2 = Text::new(
            title,
            Point::new(
                (disp_width_px as i32) - ((title.len() as u32) * char_x) as i32,
                PROFONT_12_POINT.baseline as i32,
            ),
            text_style,
        );
        text2.draw(&mut fc_0).unwrap();

        let line_style = PrimitiveStyle::with_stroke(Gray8::WHITE, 1);
        Line::new(
            Point {
                x: 0,
                y: PROFONT_12_POINT.underline.offset as i32,
            },
            Point {
                x: disp_width_px as i32,
                y: PROFONT_12_POINT.underline.offset as i32,
            },
        )
        .into_styled(line_style)
        .draw(&mut fc_0)
        .unwrap();
        disp_hdl.draw_framechunk(fc_0).await.unwrap();
    }

    let style = ring_drawer::BwStyle {
        background: Gray8::BLACK,
        font: MonoTextStyle::new(&PROFONT_12_POINT, Gray8::WHITE),
    };

    // At 12-pt font, there is enough room for 16 lines, with 50 chars/line.
    //
    // Leave out 4 for the implicit margin of two characters on each gutter.
    let mut rline = RingLine::<16, 46>::new();

    let tid0_future = k.initialize_forth_tid0(Default::default());
    let tid0 = tid0_future
        .await
        .expect("TID 0 initialization task must succeed");

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

        futures::select_biased! {
            rgr = port.consumer().read_grant().fuse() => {
                let mut used = 0;
                'input: for &b in rgr.iter() {
                    used += 1;
                    match rline.append_local_char(b) {
                        Ok(_) => {}
                        // backspace
                        Err(_) if b == 0x7F => {
                            rline.pop_local_char();
                        }
                        Err(_) if b == b'\n' => {
                            let needed = rline.local_editing_len();
                            if needed != 0 {
                                let mut tid0_wgr = tid0.producer().send_grant_exact(needed).await;
                                rline.copy_local_editing_to(&mut tid0_wgr).unwrap();
                                tid0_wgr.commit(needed);
                                rline.submit_local_editing();
                                break 'input;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(?error, "Error appending char: {:02X}", b);
                        }
                    }
                }

                rgr.release(used);
            },
            output = tid0.consumer().read_grant().fuse() => {
                let len = output.len();
                tracing::trace!(len, "Received output from TID0");
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
    }
}
