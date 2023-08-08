use core::fmt;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::Point,
    mono_font::{MonoFont, MonoTextStyle, MonoTextStyleBuilder},
    pixelcolor::PixelColor,
    text::{self, Text},
    Drawable,
};
use hal_core::framebuffer::{self, Draw};

#[derive(Debug)]
pub struct TextWriter<'style, 'target, D, C> {
    target: framebuffer::DrawTarget<&'target mut D>,
    width_px: u32,
    start_x: i32,
    point: Point,
    style: MonoTextStyle<'style, C>,
    last_line: i32,
}

impl<'style, 'target, D, C> TextWriter<'style, 'target, D, C>
where
    framebuffer::DrawTarget<&'target mut D>: DrawTarget<Color = C>,
    D: Draw,
    C: PixelColor,
{
    pub fn new(target: &'target mut D, style: MonoTextStyle<'style, C>, point: Point) -> Self {
        let width_px = target.width() as u32;
        let height_px = target.height() as u32;
        let last_line = {
            let char_height = style.font.character_size.height;
            (height_px - char_height - 10) as i32
        };
        Self {
            target: target.as_draw_target(),
            start_x: point.x,
            last_line,
            width_px,
            style,
            point,
        }
    }

    pub fn next_point(&self) -> Point {
        self.point
    }

    pub fn set_style(&mut self, style: MonoTextStyle<'style, C>) {
        self.style = style;
    }

    fn len_to_px(&self, len: usize) -> u32 {
        len as u32 * self.style.font.character_size.width
    }

    fn px_to_len(&self, px: u32) -> usize {
        (px / self.style.font.character_size.width) as usize
    }

    fn newline(&mut self) {
        if self.point.y > self.last_line {
            let ydiff = self.point.y - self.last_line;
            self.target.inner_mut().scroll_vert(ydiff as isize);
            self.point = Point {
                y: self.last_line,
                x: 10,
            };
        }

        // if we have reached the bottom of the screen, we'll need to scroll
        // previous framebuffer contents up to make room for new line(s) of
        // text.
        self.point.y = self.point.y + self.style.font.character_size.height as i32;
        self.point.x = self.start_x;
    }
}

impl<'style, 'target, D, C> fmt::Write for TextWriter<'style, 'target, D, C>
where
    framebuffer::DrawTarget<&'target mut D>: DrawTarget<Color = C>,
    D: Draw,
    C: PixelColor,
{
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // for a couple of reasons, we don't trust the `embedded-graphics` crate
        // to handle newlines for us:
        //
        // 1. it currently only actually produces a carriage return when a
        //    newline character appears in the *middle* of a string. this means
        //    that strings beginning or ending with newlines (and strings that
        //    are *just* newlines) won't advance the write position the way we'd
        //    expect them to. so, we have to do that part ourself --- it turns
        //    out that most `fmt::Debug`/`fmt::Display` implementations will
        //    write a bunch of strings that begin or end with `\n`.
        // 2. when we reach the bottom of the screen, we want to scroll the
        //    previous text up to make room for a new line of text.
        //    `embedded-graphics` doesn't implement this behav'tior. because we
        //    want to scroll every time we encounter a newline if we have
        //    reached the bottom of the screen, this means we have to implement
        //    *all* newline handling ourselves.
        //
        // TODO(eliza): currently, our newline handling doesn't honor
        // configurable line height. all lines are simply a single character
        // high. if we want to do something nicer about line height, we'd have
        // to implement that here...
        for mut line in s.split_inclusive('\n') {
            while !line.is_empty() {
                // does this line begin with a newline?
                if line.starts_with('\n') {
                    line = &line[1..];
                    self.newline();
                }

                let mut has_newline = false;
                // does this chunk end with a newline? it might not, if:
                // (a) it's the last chunk in a string where newlines only occur in
                //     the beginning/middle.
                // (b) the string being written has no newlines (so
                //     `split_inclusive` will only yield a single chunk)
                has_newline = line.ends_with('\n');
                if has_newline {
                    // if there's a trailing newline, trim it off --- no sense
                    // making the `embedded-graphics` crate draw an extra character
                    // it will essentially nop for.
                    line = &line[..line.len() - 1];
                }

                let mut chunk = line;

                // if the line is longer than the remaining space on the current
                // line, wrap the line.
                let rem = self.px_to_len(self.width_px - (self.point.x as u32));
                if line.len() > rem {
                    let (curr, next) = line.split_at(rem as usize);
                    line = next;
                    chunk = curr;
                    has_newline = true;
                } else {
                    line = "";
                }

                // if this line is now empty, it was *just* a newline character,
                // so all we have to do is advance the write position.
                if !chunk.is_empty() {
                    self.point =
                        Text::with_alignment(chunk, self.point, self.style, text::Alignment::Left)
                            .draw(&mut self.target)
                            .map_err(|_| fmt::Error)?
                };

                if has_newline {
                    // carriage return
                    self.newline();
                }
            }
        }

        Ok(())
    }
}
