// Use profont 12pt, 7(+1)x15
//
// at  320 x 240: 40w*16h
// at  400 x 240: 50w*16h
// at 1280 x 720: 90w*48h (45w*24h 2x?)

use embedded_graphics::{
    mono_font::{MonoFont, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::{Dimensions, DrawTarget, IntoStorage, Point, RgbColor, Size, WebColors},
    primitives::{PrimitiveStyleBuilder, Rectangle, StyledDrawable},
    text::Text,
    Drawable, Pixel,
};
use fancy::{Line, RingLine, Source};
use forth3::{
    leakbox::{LBForth, LBForthParams},
    Forth,
};
use minifb::{Key, Scale, Window, WindowOptions};
use profont::PROFONT_12_POINT;
use std::time::Duration;

pub mod bricks;
pub mod fancy;

const CHARS_X: usize = 40;
const CHARS_Y: usize = 16;

// const DEFAULT_CHAR: u8 = b' ';
// const ONE_ATOMIC: AtomicU8 = AtomicU8::new(DEFAULT_CHAR);
// const ONE_LINE: [AtomicU8; CHARS_X] = [ONE_ATOMIC; CHARS_X];

const FONT: MonoFont = PROFONT_12_POINT;

const CHAR_PIXELS_X: u32 = FONT.character_size.width + FONT.character_spacing;
const CHAR_PIXELS_Y: u32 = FONT.character_size.height;

const DISP_PIXELS_X: usize = (CHAR_PIXELS_X as usize) * CHARS_X;
const DISP_PIXELS_Y: usize = (CHAR_PIXELS_Y as usize) * CHARS_Y;
const DISP_PIXELS_TTL: usize = DISP_PIXELS_X * DISP_PIXELS_Y;
const DISP_DEFAULT: [u32; DISP_PIXELS_TTL] = [0; DISP_PIXELS_TTL];

struct GloboChar {
    grid: crate::fancy::RingLine<CHARS_Y, { CHARS_X - 4 }>,
    dirty: bool,
    lb_forth: LBForth<()>,
}

impl GloboChar {
    fn key(&mut self, key: Key, shift: bool) {
        let draw = match key {
            Key::Key0 => {
                if !shift {
                    Some(b'0')
                } else {
                    Some(b')')
                }
            }
            Key::Key1 => {
                if !shift {
                    Some(b'1')
                } else {
                    Some(b'!')
                }
            }
            Key::Key2 => {
                if !shift {
                    Some(b'2')
                } else {
                    Some(b'@')
                }
            }
            Key::Key3 => {
                if !shift {
                    Some(b'3')
                } else {
                    Some(b'#')
                }
            }
            Key::Key4 => {
                if !shift {
                    Some(b'4')
                } else {
                    Some(b'$')
                }
            }
            Key::Key5 => {
                if !shift {
                    Some(b'5')
                } else {
                    Some(b'%')
                }
            }
            Key::Key6 => {
                if !shift {
                    Some(b'6')
                } else {
                    Some(b'^')
                }
            }
            Key::Key7 => {
                if !shift {
                    Some(b'7')
                } else {
                    Some(b'&')
                }
            }
            Key::Key8 => {
                if !shift {
                    Some(b'8')
                } else {
                    Some(b'*')
                }
            }
            Key::Key9 => {
                if !shift {
                    Some(b'9')
                } else {
                    Some(b'(')
                }
            }
            Key::A => {
                if !shift {
                    Some(b'a')
                } else {
                    Some(b'A')
                }
            }
            Key::B => {
                if !shift {
                    Some(b'b')
                } else {
                    Some(b'B')
                }
            }
            Key::C => {
                if !shift {
                    Some(b'c')
                } else {
                    Some(b'C')
                }
            }
            Key::D => {
                if !shift {
                    Some(b'd')
                } else {
                    Some(b'D')
                }
            }
            Key::E => {
                if !shift {
                    Some(b'e')
                } else {
                    Some(b'E')
                }
            }
            Key::F => {
                if !shift {
                    Some(b'f')
                } else {
                    Some(b'F')
                }
            }
            Key::G => {
                if !shift {
                    Some(b'g')
                } else {
                    Some(b'G')
                }
            }
            Key::H => {
                if !shift {
                    Some(b'h')
                } else {
                    Some(b'H')
                }
            }
            Key::I => {
                if !shift {
                    Some(b'i')
                } else {
                    Some(b'I')
                }
            }
            Key::J => {
                if !shift {
                    Some(b'j')
                } else {
                    Some(b'J')
                }
            }
            Key::K => {
                if !shift {
                    Some(b'k')
                } else {
                    Some(b'K')
                }
            }
            Key::L => {
                if !shift {
                    Some(b'l')
                } else {
                    Some(b'L')
                }
            }
            Key::M => {
                if !shift {
                    Some(b'm')
                } else {
                    Some(b'M')
                }
            }
            Key::N => {
                if !shift {
                    Some(b'n')
                } else {
                    Some(b'N')
                }
            }
            Key::O => {
                if !shift {
                    Some(b'o')
                } else {
                    Some(b'O')
                }
            }
            Key::P => {
                if !shift {
                    Some(b'p')
                } else {
                    Some(b'P')
                }
            }
            Key::Q => {
                if !shift {
                    Some(b'q')
                } else {
                    Some(b'Q')
                }
            }
            Key::R => {
                if !shift {
                    Some(b'r')
                } else {
                    Some(b'R')
                }
            }
            Key::S => {
                if !shift {
                    Some(b's')
                } else {
                    Some(b'S')
                }
            }
            Key::T => {
                if !shift {
                    Some(b't')
                } else {
                    Some(b'T')
                }
            }
            Key::U => {
                if !shift {
                    Some(b'u')
                } else {
                    Some(b'U')
                }
            }
            Key::V => {
                if !shift {
                    Some(b'v')
                } else {
                    Some(b'V')
                }
            }
            Key::W => {
                if !shift {
                    Some(b'w')
                } else {
                    Some(b'W')
                }
            }
            Key::X => {
                if !shift {
                    Some(b'x')
                } else {
                    Some(b'X')
                }
            }
            Key::Y => {
                if !shift {
                    Some(b'y')
                } else {
                    Some(b'Y')
                }
            }
            Key::Z => {
                if !shift {
                    Some(b'z')
                } else {
                    Some(b'Z')
                }
            }
            Key::F1 => None,
            Key::F2 => None,
            Key::F3 => None,
            Key::F4 => None,
            Key::F5 => None,
            Key::F6 => None,
            Key::F7 => None,
            Key::F8 => None,
            Key::F9 => None,
            Key::F10 => None,
            Key::F11 => None,
            Key::F12 => None,
            Key::F13 => None,
            Key::F14 => None,
            Key::F15 => None,
            Key::Down => {
                // if (self.curs_y + 1) < CHARS_Y {
                //     self.curs_y += 1;
                // }
                // self.dirty = true;
                None
            }
            Key::Left => {
                // if self.curs_x != 0 {
                //     self.curs_x -= 1;
                // } else if self.curs_y != 0 {
                //     self.curs_x = CHARS_X - 1;
                //     self.curs_y -= 1;
                // }
                // self.dirty = true;
                None
            }
            Key::Right => {
                // if (self.curs_x + 1) < CHARS_X {
                //     self.curs_x += 1;
                // } else if (self.curs_y + 1) < CHARS_Y {
                //     self.curs_x = 0;
                //     self.curs_y += 1;
                // }
                // self.dirty = true;
                None
            }
            Key::Up => {
                // if self.curs_y != 0 {
                //     self.curs_y -= 1;
                // }
                // self.dirty = true;
                None
            }
            Key::Apostrophe => {
                if !shift {
                    Some(b'\'')
                } else {
                    Some(b'"')
                }
            }
            Key::Backquote => {
                if !shift {
                    Some(b'`')
                } else {
                    Some(b'~')
                }
            }
            Key::Backslash => {
                if !shift {
                    Some(b'\\')
                } else {
                    Some(b'|')
                }
            }
            Key::Comma => {
                if !shift {
                    Some(b',')
                } else {
                    Some(b'<')
                }
            }
            Key::Equal => {
                if !shift {
                    Some(b'=')
                } else {
                    Some(b'+')
                }
            }
            Key::LeftBracket => {
                if !shift {
                    Some(b'[')
                } else {
                    Some(b'{')
                }
            }
            Key::Minus => {
                if !shift {
                    Some(b'-')
                } else {
                    Some(b'_')
                }
            }
            Key::Period => {
                if !shift {
                    Some(b'.')
                } else {
                    Some(b'>')
                }
            }
            Key::RightBracket => {
                if !shift {
                    Some(b']')
                } else {
                    Some(b'}')
                }
            }
            Key::Semicolon => {
                if !shift {
                    Some(b';')
                } else {
                    Some(b':')
                }
            }
            Key::Slash => {
                if !shift {
                    Some(b'/')
                } else {
                    Some(b'?')
                }
            }
            Key::Backspace => {
                self.grid.pop_char();
                self.dirty = true;
                None
            }
            Key::Delete => None,
            Key::End => {
                // self.grid.iter_mut().flatten().for_each(|b| *b = b' ');
                // self.curs_x = 0;
                // self.curs_y = 0;
                // self.dirty = true;
                None
            }
            Key::Enter => {
                if !shift {
                    let input: Vec<&str> = self
                        .grid
                        .brick
                        .iter_user_editable(&self.grid.lines)
                        .map(Line::as_str)
                        .collect();
                    let input: String = input.iter().rev().map(|s| *s).collect();

                    self.lb_forth.forth.input.fill(&input).unwrap();
                    self.lb_forth.forth.output.clear();
                    println!("PREPROCESS...");
                    let out = match self.lb_forth.forth.process_line() {
                        Ok(_) => {
                            println!("POSTOK...");
                            self.lb_forth.forth.output.as_str().to_string()
                        }
                        Err(e) => {
                            println!("POSTERR...");
                            let mut o = format!("ERROR: {:?}\n", e);
                            o += "Unprocessed Tokens:\n";
                            while let Some(tok) = self.lb_forth.forth.input.cur_word() {
                                o += &format!("'{}', ", tok);
                                self.lb_forth.forth.input.advance();
                            }
                            o += "\n";
                            o
                        }
                    };
                    let RingLine { lines, brick } = &mut self.grid;
                    for line in out.lines() {
                        let idx = brick.insert_ie_front().unwrap();
                        let cur = &mut lines[idx];
                        cur.clear();
                        cur.status = Source::Remote;
                        cur.extend(line).unwrap();
                    }
                    brick.release_ue();
                    brick.release_ie();
                    self.dirty = true;
                }
                let RingLine { lines, brick } = &mut self.grid;
                if let Ok(n) = brick.insert_ue_front() {
                    lines[n].status = Source::Local;
                    lines[n].clear();
                }
                self.dirty = true;
                // self.curs_x = 0;
                // if (self.curs_y + 1) < CHARS_Y {
                //     self.curs_y += 1;
                // } else {
                //     self.scrollup();
                // }
                // self.dirty = true;
                None
            }
            Key::Escape => None,
            Key::Home => None,
            Key::Insert => None,
            Key::Menu => None,
            Key::PageDown => {
                // self.scrollup();
                // self.dirty = true;
                None
            }
            Key::PageUp => {
                // self.scrolldn();
                // self.dirty = true;
                None
            }
            Key::Pause => None,
            Key::Space => Some(b' '),
            Key::Tab => None,
            Key::NumLock => None,
            Key::CapsLock => None,
            Key::ScrollLock => None,
            Key::LeftShift => None,
            Key::RightShift => None,
            Key::LeftCtrl => None,
            Key::RightCtrl => None,
            Key::NumPad0 => Some(b'0'),
            Key::NumPad1 => Some(b'1'),
            Key::NumPad2 => Some(b'2'),
            Key::NumPad3 => Some(b'3'),
            Key::NumPad4 => Some(b'4'),
            Key::NumPad5 => Some(b'5'),
            Key::NumPad6 => Some(b'6'),
            Key::NumPad7 => Some(b'7'),
            Key::NumPad8 => Some(b'8'),
            Key::NumPad9 => Some(b'9'),
            Key::NumPadDot => None,
            Key::NumPadSlash => None,
            Key::NumPadAsterisk => None,
            Key::NumPadMinus => None,
            Key::NumPadPlus => None,
            Key::NumPadEnter => None,
            Key::LeftAlt => None,
            Key::RightAlt => None,
            Key::LeftSuper => None,
            Key::RightSuper => None,
            Key::Unknown => None,
            Key::Count => None,
        };
        if let Some(k) = draw {
            println!("{}", core::str::from_utf8(&[k]).unwrap());
            self.grid.append_char(k).unwrap();
            self.dirty = true;
        }
    }
}

struct Display {
    pixels: [u32; DISP_PIXELS_X * DISP_PIXELS_Y],
}

impl Default for Display {
    fn default() -> Self {
        Self {
            pixels: DISP_DEFAULT,
        }
    }
}

fn main() {
    let mut disp = Display::default();
    let mut options = WindowOptions::default();
    options.scale = Scale::X4;
    let mut window =
        Window::new("Test - ESC to exit", DISP_PIXELS_X, DISP_PIXELS_Y, options).unwrap();
    window.limit_update_rate(Some(Duration::from_micros(1_000_000 / 60)));
    let style = MonoTextStyle::new(&FONT, Rgb888::WHITE);
    let style_dark = MonoTextStyle::new(&FONT, Rgb888::BLACK);

    let lb_forth = LBForth::from_params(LBForthParams::default(), (), Forth::FULL_BUILTINS);

    let mut the_grid = GloboChar {
        grid: RingLine::new(),
        dirty: true,
        lb_forth,
    };
    let _ = the_grid.grid.brick.insert_ue_front();

    // let mut input_tick = Instant::now();
    // let mut input_idx = 0;
    // let mut loop_idx = 0;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // if input_tick.elapsed() >= Duration::from_millis(500) {
        //     input_tick = Instant::now();
        //     let RingLine { lines, brick } = &mut the_grid.grid;
        //     match input_idx {
        //         0 => {
        //             if let Ok(idx) = brick.insert_ie_front() {
        //                 lines[idx].clear();
        //                 lines[idx].status = Source::Remote;
        //                 input_idx += 1;
        //                 the_grid.dirty = true;
        //             }
        //         }
        //         1 => {
        //             let line = &mut lines[brick.ie_front().unwrap()];
        //             for c in b"hello" {
        //                 line.push(*c).unwrap();
        //             }
        //             input_idx += 1;
        //             the_grid.dirty = true;
        //         }
        //         2 => {
        //             let line = &mut lines[brick.ie_front().unwrap()];
        //             for c in b", world!" {
        //                 line.push(*c).unwrap();
        //             }
        //             input_idx += 1;
        //             the_grid.dirty = true;
        //         }
        //         3 => {
        //             let line = &mut lines[brick.ie_front().unwrap()];
        //             for c in format!(" loop: {}", loop_idx).as_bytes() {
        //                 line.push(*c).unwrap();
        //             }
        //             loop_idx += 1;
        //             input_idx += 1;
        //             the_grid.dirty = true;
        //         }
        //         _ => {
        //             brick.release_ie();
        //             input_idx = 0;
        //             the_grid.dirty = true;
        //         }
        //     }
        // }

        let shift = [Key::LeftShift, Key::RightShift]
            .iter()
            .any(|k| window.is_key_down(*k));
        for akey in window.get_keys_pressed(minifb::KeyRepeat::No) {
            the_grid.key(akey, shift);
        }
        if the_grid.dirty {
            the_grid.dirty = false;
            Rectangle::new(
                Point {
                    x: CHAR_PIXELS_X as i32,
                    y: 0,
                },
                Size {
                    width: DISP_PIXELS_X as u32 - CHAR_PIXELS_X,
                    height: DISP_PIXELS_Y as u32,
                },
            )
            .draw_styled(
                &PrimitiveStyleBuilder::new()
                    .fill_color(Rgb888::BLACK)
                    .build(),
                &mut disp,
            )
            .unwrap();
            let mut height = CHARS_Y - 1;
            the_grid
                .grid
                .brick
                .iter_user_editable(&the_grid.grid.lines)
                .for_each(|line| {
                    let bar_color = Rgb888::CSS_DARK_BLUE;

                    let bar = Rectangle::new(
                        Point {
                            x: CHAR_PIXELS_X as i32,
                            y: ((height as i32) * CHAR_PIXELS_Y as i32),
                        },
                        Size {
                            width: DISP_PIXELS_X as u32 - 2 * CHAR_PIXELS_X,
                            height: CHAR_PIXELS_Y,
                        },
                    );
                    let bstyle = PrimitiveStyleBuilder::new().fill_color(bar_color).build();
                    bar.draw_styled(&bstyle, &mut disp).unwrap();

                    let y = height as i32 * CHAR_PIXELS_Y as i32 + FONT.baseline as i32;
                    Text::new(
                        line.as_str(),
                        Point {
                            x: CHAR_PIXELS_X as i32 * 2,
                            y,
                        },
                        style,
                    )
                    .draw(&mut disp)
                    .unwrap();
                    height -= 1;
                });
            the_grid
                .grid
                .brick
                .iter_inco_editable(&the_grid.grid.lines)
                .for_each(|line| {
                    let bar_color = Rgb888::CSS_DARK_GREEN;

                    let bar = Rectangle::new(
                        Point {
                            x: CHAR_PIXELS_X as i32,
                            y: ((height as i32) * CHAR_PIXELS_Y as i32),
                        },
                        Size {
                            width: DISP_PIXELS_X as u32 - 2 * CHAR_PIXELS_X,
                            height: CHAR_PIXELS_Y,
                        },
                    );
                    let bstyle = PrimitiveStyleBuilder::new().fill_color(bar_color).build();
                    bar.draw_styled(&bstyle, &mut disp).unwrap();

                    let y = height as i32 * CHAR_PIXELS_Y as i32 + FONT.baseline as i32;
                    Text::new(
                        line.as_str(),
                        Point {
                            x: CHAR_PIXELS_X as i32 * 2,
                            y,
                        },
                        style,
                    )
                    .draw(&mut disp)
                    .unwrap();
                    height -= 1;
                });
            the_grid
                .grid
                .brick
                .iter_history(&the_grid.grid.lines)
                .for_each(|line| {
                    let bar_color = match line.status {
                        fancy::Source::Local => Rgb888::CSS_LIGHT_BLUE,
                        fancy::Source::Remote => Rgb888::CSS_LIGHT_GREEN,
                    };

                    let bar = Rectangle::new(
                        Point {
                            x: CHAR_PIXELS_X as i32,
                            y: ((height as i32) * CHAR_PIXELS_Y as i32),
                        },
                        Size {
                            width: DISP_PIXELS_X as u32 - 2 * CHAR_PIXELS_X,
                            height: CHAR_PIXELS_Y,
                        },
                    );
                    let bstyle = PrimitiveStyleBuilder::new().fill_color(bar_color).build();
                    bar.draw_styled(&bstyle, &mut disp).unwrap();

                    let y = height as i32 * CHAR_PIXELS_Y as i32 + FONT.baseline as i32;
                    Text::new(
                        line.as_str(),
                        Point {
                            x: CHAR_PIXELS_X as i32 * 2,
                            y,
                        },
                        style_dark,
                    )
                    .draw(&mut disp)
                    .unwrap();
                    height -= 1;
                });
        }
        window
            .update_with_buffer(&disp.pixels, DISP_PIXELS_X, DISP_PIXELS_Y)
            .unwrap();
    }
}

impl Dimensions for Display {
    fn bounding_box(&self) -> Rectangle {
        Rectangle {
            top_left: Point::new(0, 0),
            size: Size::new(DISP_PIXELS_X as u32, DISP_PIXELS_Y as u32),
        }
    }
}

// static MIN_X: AtomicI32 = AtomicI32::new(i32::MAX);
// static MAX_X: AtomicI32 = AtomicI32::new(i32::MIN);
// static MIN_Y: AtomicI32 = AtomicI32::new(i32::MAX);
// static MAX_Y: AtomicI32 = AtomicI32::new(i32::MIN);

impl DrawTarget for Display {
    type Color = Rgb888;

    type Error = ();

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(pt, col) in pixels.into_iter() {
            // let mut new_max = false;
            // if MIN_X.fetch_min(pt.x, Ordering::AcqRel) > pt.x {
            //     new_max = true;
            // }
            // if MIN_Y.fetch_min(pt.y, Ordering::AcqRel) > pt.y {
            //     new_max = true;
            // }
            // if MAX_X.fetch_max(pt.x, Ordering::AcqRel) < pt.x {
            //     new_max = true;
            // }
            // if MAX_Y.fetch_max(pt.y, Ordering::AcqRel) < pt.y {
            //     new_max = true;
            // }
            // if new_max {
            //     let (x0, x1) = (MIN_X.load(Ordering::Relaxed), MAX_X.load(Ordering::Relaxed));
            //     let (y0, y1) = (MIN_Y.load(Ordering::Relaxed), MAX_Y.load(Ordering::Relaxed));
            //     println!("({}, {}) -> ({}, {})", x0, y0, x1, y1);
            // }

            let idx = (pt.y.unsigned_abs() * DISP_PIXELS_X as u32) + pt.x.unsigned_abs();
            let idx = idx as usize;
            if let Some(pix) = self.pixels.get_mut(idx) {
                *pix = col.into_storage();
            }
        }
        Ok(())
    }
}
