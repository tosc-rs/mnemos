use crate::drivers::framebuf::TextWriter;
use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU64, Ordering},
};
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::Point,
    mono_font::{MonoTextStyle, MonoTextStyleBuilder},
    pixelcolor::{Rgb888, RgbColor},
};
use hal_core::framebuffer;
use hal_x86_64::framebuffer::Framebuffer;
use kernel::{
    serial_trace::SerialSubscriber,
    tracing::{
        level_filters::LevelFilter, span, subscriber::Interest, Event, Metadata, Subscriber,
    },
};
use mycelium_util::sync::InitOnce;

static SERIAL: InitOnce<SerialSubscriber> = InitOnce::uninitialized();

pub struct TraceSubscriber<F>
where
    F: Deref<Target = [u8]> + DerefMut + 'static,
{
    framebuf: fn() -> Framebuffer<'static, F>,
    point: AtomicU64,
    _f: PhantomData<fn(&'static F)>,
}

#[inline]
fn with_serial<T>(f: impl FnOnce(&SerialSubscriber) -> T) -> Option<T> {
    SERIAL.try_get().map(f)
}

impl<F> TraceSubscriber<F>
where
    F: Deref<Target = [u8]> + DerefMut + 'static,
{
    pub fn new(framebuf: fn() -> Framebuffer<'static, F>) -> Self {
        Self {
            framebuf,
            point: AtomicU64::new(pack_point(Point { x: 10, y: 10 })),
            _f: PhantomData,
        }
    }
}

fn style(color: Rgb888) -> MonoTextStyle<'static, Rgb888> {
    MonoTextStyleBuilder::new()
        .font(&profont::PROFONT_12_POINT)
        .text_color(color)
        .build()
}

impl<F> Subscriber for TraceSubscriber<F>
where
    F: Deref<Target = [u8]> + DerefMut + 'static,
    for<'a> framebuffer::DrawTarget<&'a mut Framebuffer<'static, F>>: DrawTarget<Color = Rgb888>, // jesus christ...
{
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        with_serial(|serial| serial.enabled(metadata)).unwrap_or(!metadata.is_span())
    }

    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        with_serial(|serial| serial.register_callsite(metadata)).unwrap_or_else(Interest::sometimes)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        with_serial(Subscriber::max_level_hint).unwrap_or(None)
    }

    fn new_span(&self, span: &span::Attributes<'_>) -> span::Id {
        with_serial(|serial| serial.new_span(span))
            .expect("spans are not enabled before serial comes up")
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {
        // todo!("eliza")
    }

    fn enter(&self, span: &span::Id) {
        with_serial(|serial| serial.enter(span));
    }

    fn exit(&self, span: &span::Id) {
        with_serial(|serial| serial.enter(span));
    }

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {
        // nop for now
    }

    fn event(&self, event: &Event<'_>) {
        use core::fmt::Write;

        if with_serial(|serial| serial.event(event)).is_none() {
            let point = unpack_point(self.point.load(Ordering::Acquire));
            let mut framebuf = (self.framebuf)();
            let meta = event.metadata();
            let (lvl_color, lvl_str) = match *meta.level() {
                tracing::Level::TRACE => (Rgb888::BLUE, "TRCE"),
                tracing::Level::DEBUG => (Rgb888::CYAN, "DBUG"),
                tracing::Level::INFO => (Rgb888::GREEN, "INFO"),
                tracing::Level::WARN => (Rgb888::YELLOW, "WARN"),
                tracing::Level::ERROR => (Rgb888::RED, "ERR!"),
            };

            // write the level in the per-level color.
            let mut writer = TextWriter::new(&mut framebuf, style(lvl_color), point);
            let _ = writer.write_str(lvl_str);

            writer.set_style(style(Rgb888::new(128, 128, 128)));
            let _ = write!(&mut writer, " {}:", meta.target());

            writer.set_style(style(Rgb888::WHITE));

            event.record(
                &mut (|field: &tracing::field::Field, value: &'_ (dyn core::fmt::Debug + '_)| {
                    if field.name() == "message" {
                        let _ = write!(&mut writer, " {value:?}");
                    } else {
                        let _ = write!(&mut writer, " {field}={value:?}");
                    }
                }) as &mut dyn tracing::field::Visit,
            );
            writeln!(&mut writer, "");

            let mut next_point = writer.next_point();
            drop(writer);

            self.point.store(pack_point(next_point), Ordering::Release);
        }
    }

    fn clone_span(&self, span: &span::Id) -> span::Id {
        with_serial(|serial| serial.clone_span(span))
            .expect("spans are not enabled until serial is enabled")
    }

    fn try_close(&self, span: span::Id) -> bool {
        with_serial(move |serial| serial.try_close(span))
            .expect("spans are not enabled until serial is enabled")
    }
}

const fn pack_point(Point { x, y }: Point) -> u64 {
    (x as u64) << 32 | y as u64
}

const fn unpack_point(u: u64) -> Point {
    const Y_MASK: u64 = u32::MAX as u64;
    let x = (u >> 32) as i32;
    let y = (u & Y_MASK) as i32;
    Point { x, y }
}
