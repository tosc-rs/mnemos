use core::sync::atomic::AtomicU64;
use embedded_graphics::{text::mono::MonoTextStyleBuilder, Point};
use hal_x86_64::framebuffer::Framebuffer;
use kernel::{
    serial_trace::SerialSubscriber,
    tracing::{level_filters::LevelFilter, span, Event, Interest, Metadata, Subscriber},
};
use mycelium_util::sync::InitOnce;

static SERIAL: InitOnce<SerialSubscriber> = InitOnce::new();

pub struct TraceSubscriber<F> {
    framebuf: fn() -> Framebuffer<'static, F>,
    point: AtomicU64,
}

#[inline]
fn with_serial<T>(f: impl Fn(&SerialSubscriber) -> T) -> Option<T> {
    SERIAL.try_get().map(f)
}

impl<F> TraceSubscriber<F> {
    pub const fn new(framebuf: fn() -> Framebuffer<'static, F>) -> Self {
        Self {
            framebuf,
            point: AtomicU64::new(pack_point(Point { x: 10, y: 10 })),
        }
    }
}

impl<F> Subscriber for TraceSubscriber<F>
where
    F: Deref<[u8]>,
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
        with_serial(|serial| serial.enter(span))
    }

    fn exit(&self, span: &span::Id) {
        with_serial(|serial| serial.enter(span))
    }

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {
        // nop for now
    }

    fn event(&self, event: &Event<'_>) {
        if with_serial(|serial| serial.event(event)).is_none() {
            let mut framebuf = (self.framebuf)();
            let mut writer = {
                let style = MonoTextStyleBuilder::new()
                    .font(&profont::PROFONT_12_POINT)
                    .text_color(Rgb888::WHITE)
                    .build();
                let point = unpack_point(self.point.load(Ordering::Acquire));
                crate::drivers::framebuf::TextWriter::new(framebuf, style, point)
            };
            writeln!("{event:?}");
            self.point
                .store(pack_point(writer.next_point()), Ordering::Release);
        }
    }

    fn clone_span(&self, span: &span::Id) -> span::Id {
        with_serial(|serial| serial.clone_span(span))
            .expect("spans are not enabled until serial is enabled")
    }

    fn try_close(&self, span: span::Id) -> bool {
        with_serial(|serial| serial.try_close(span))
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
