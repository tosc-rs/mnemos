use crate::{comms::bbq, services::serial_mux};
use core::time::Duration;
use level_filters::LevelFilter;
use mnemos_trace_proto::{HostRequest, TraceEvent};
use portable_atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use tracing::subscriber::Interest;
pub use tracing::*;
use tracing_serde_structured::{AsSerde, SerializeRecordFields, SerializeSpanFields};

pub struct SerialSubscriber {
    tx: bbq::SpscProducer,
    isr_tx: bbq::SpscProducer,

    /// ID of the next span.
    next_id: AtomicU64,

    /// Tracks whether we are inside of the collector's `send_event` method, so
    /// that BBQueue tracing can be disabled.
    in_send: AtomicBool,

    shared: &'static Shared,
}

#[derive(Debug)]
pub struct SerialTraceSettings {
    /// SerialMux port for sermux tracing.
    port: u16,

    /// Capacity for the serial port's send buffer.
    sendbuf_capacity: usize,

    /// Capacity for the trace ring buffer.
    ///
    /// Note that *two* buffers of this size will be allocated. One buffer is
    /// used for the normal trace ring buffer, and another is used for the
    /// interrupt service routine trace ring buffer.
    tracebuf_capacity: usize,

    /// Initial level filter used if the debug host does not select a max level.
    initial_level: LevelFilter,
}

struct Shared {
    /// Counter of events that were dropped due to insufficient buffer capacity.
    dropped_events: AtomicUsize,

    /// Counter of new spans that were dropped due to insufficient buffer capacity.
    dropped_spans: AtomicUsize,

    /// Counter of metadata that was dropped due to insufficient buffer capacity.
    dropped_metas: AtomicUsize,

    /// Counter of span enter/exit/clone/drop messages that were dropped due to
    /// insufficient buffer capacity.
    dropped_span_activity: AtomicUsize,

    max_level: AtomicU8,
}

static SHARED: Shared = Shared {
    dropped_events: AtomicUsize::new(0),
    dropped_spans: AtomicUsize::new(0),
    dropped_metas: AtomicUsize::new(0),
    dropped_span_activity: AtomicUsize::new(0),
    max_level: AtomicU8::new(level_to_u8(LevelFilter::OFF)),
};

// === impl SerialSubscriber ===

impl SerialSubscriber {
    pub async fn start(k: &'static crate::Kernel, settings: SerialTraceSettings) {
        SHARED
            .max_level
            .store(level_to_u8(settings.initial_level), Ordering::Release);
        // acquire sermux port 3
        let port = serial_mux::PortHandle::open(k, settings.port, settings.sendbuf_capacity)
            .await
            .expect("cannot initialize serial tracing, cannot open port 3!");

        let (tx, rx) = bbq::new_spsc_channel(settings.tracebuf_capacity).await;
        let (isr_tx, isr_rx) = bbq::new_spsc_channel(settings.tracebuf_capacity).await;

        let subscriber = Self {
            tx,
            isr_tx,
            next_id: AtomicU64::new(1),
            in_send: AtomicBool::new(false),
            shared: &SHARED,
        };

        // set the default tracing collector
        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set global default subscriber");

        // spawn a worker to read from the channel and write to the serial port.
        k.spawn(Self::worker(&SHARED, rx, isr_rx, port, k)).await;
    }

    /// Serialize a `TraceEvent`, returning `true` if the event was correctly serialized.
    fn send_event<'a>(&self, sz: usize, event: impl FnOnce() -> TraceEvent<'a>) -> bool {
        self.in_send.store(true, Ordering::Release);
        let tx = if crate::isr::Isr::is_in_isr() {
            &self.isr_tx
        } else {
            &self.tx
        };
        let Some(mut wgr) = tx.send_grant_exact_sync(sz) else {
            return false;
        };

        // got a write grant! generate the event payload.
        let ev = event();

        // encode the event to our write grant, and commit however many bytes
        // were written. if encoding fails, commit 0 bytes, so the region we got
        // a write grant for can be reused.
        let len = match postcard::to_slice_cobs(&ev, &mut wgr[..]) {
            Ok(encoded) => encoded.len(),
            Err(_) => 0,
        };
        wgr.commit(len);
        self.in_send.store(false, Ordering::Release);

        // return true if we committed a non-zero number of bytes.
        len > 0
    }

    async fn worker(
        shared: &'static Shared,
        rx: bbq::Consumer,
        isr_rx: bbq::Consumer,
        port: serial_mux::PortHandle,
        k: &'static crate::Kernel,
    ) {
        use futures::FutureExt;
        use maitake::time;
        use postcard::accumulator::{CobsAccumulator, FeedResult};

        // we probably won't use 256 whole bytes of cobs yet since all the host
        // -> target messages are quite small
        let mut cobs_buf: CobsAccumulator<16> = CobsAccumulator::new();
        let mut read_level = |rgr: bbq::GrantR| {
            let mut window = &rgr[..];
            let len = rgr.len();
            'cobs: while !window.is_empty() {
                window = match cobs_buf.feed_ref::<HostRequest>(window) {
                    FeedResult::Consumed => break 'cobs,
                    FeedResult::OverFull(new_wind) => new_wind,
                    FeedResult::DeserError(new_wind) => new_wind,
                    FeedResult::Success { data, remaining } => {
                        match data {
                            HostRequest::SetMaxLevel(lvl) => {
                                let level = lvl
                                    .map(|lvl| lvl as u8)
                                    .unwrap_or(level_to_u8(LevelFilter::OFF));
                                shared.max_level.store(level, Ordering::Release);
                                tracing::callsite::rebuild_interest_cache();
                                info!(
                                    message = %"hello from mnemOS",
                                    version = %env!("CARGO_PKG_VERSION"),
                                    git = %format_args!(
                                        "{}@{}",
                                        env!("VERGEN_GIT_BRANCH"),
                                        env!("VERGEN_GIT_DESCRIBE")
                                    ),
                                    target = %env!("VERGEN_CARGO_TARGET_TRIPLE"),
                                    profile = %if cfg!(debug_assertions) { "debug" } else { "release" },
                                );
                            }
                        }

                        remaining
                    }
                };
            }
            rgr.release(len);
        };

        loop {
            'idle: loop {
                let mut heartbeat = [0u8; 8];
                let heartbeat = {
                    let level = u8_to_level(shared.max_level.load(Ordering::Acquire))
                        .into_level()
                        .as_ref()
                        .map(AsSerde::as_serde);
                    postcard::to_slice_cobs(&TraceEvent::Heartbeat(level), &mut heartbeat[..])
                        .expect("failed to encode heartbeat msg")
                };
                port.send(heartbeat).await;
                if let Ok(rgr) = k
                    .timer()
                    .timeout(time::Duration::from_secs(1), port.consumer().read_grant())
                    .await
                {
                    read_level(rgr);

                    // ack the new max level
                    let mut ack = [0u8; 8];
                    let ack = {
                        let level = u8_to_level(shared.max_level.load(Ordering::Acquire))
                            .into_level()
                            .as_ref()
                            .map(AsSerde::as_serde);
                        postcard::to_slice_cobs(&TraceEvent::Heartbeat(level), &mut ack[..])
                            .expect("failed to encode heartbeat msg")
                    };
                    port.send(ack).await;
                    break 'idle;
                }
            }

            loop {
                futures::select_biased! {
                    // something to send to the serial port!
                    rgr = isr_rx.read_grant().fuse() => {
                        let len = rgr.len();
                        port.send(&rgr[..]).await;
                        rgr.release(len);
                    },
                    rgr = rx.read_grant().fuse() => {
                        let len = rgr.len();
                        port.send(&rgr[..]).await;
                        rgr.release(len);
                    },
                    // got a host message!
                    rgr = port.consumer().read_grant().fuse() => {
                        read_level(rgr);
                    },
                    // every few seconds, check if we left anything good on the floor
                    _ = k.sleep(Duration::from_secs(3)).fuse() => {
                        let new_spans = shared.dropped_spans.swap(0, Ordering::Relaxed);
                        let events = shared.dropped_events.swap(0, Ordering::Relaxed);
                        let span_activity = shared.dropped_events.swap(0, Ordering::Relaxed);
                        let metas = shared.dropped_metas.swap(0, Ordering::Relaxed);
                        if new_spans + events + span_activity + metas > 0 {
                            let mut buf = [0u8; 256];
                            let buf = {
                                let ev = TraceEvent::Discarded {
                                    new_spans,
                                    events,
                                    span_activity,
                                    metas,
                                };
                                postcard::to_slice_cobs(&ev, &mut buf[..])
                                    .expect("failed to encode dropped msg")
                            };
                            port.send(buf).await;
                        }
                    }
                    // TODO(eliza): make the host also send a heartbeat, and
                    // if we don't get it, break back to the idle loop...
                }
            }
        }
    }

    #[inline]
    fn level_enabled(&self, metadata: &Metadata<'_>) -> bool {
        // TODO(eliza): more sophisticated filtering
        metadata.level() <= &u8_to_level(self.shared.max_level.load(Ordering::Relaxed))
    }
}

impl Subscriber for SerialSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.level_enabled(metadata) && !self.in_send.load(Ordering::Acquire)
    }

    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if !self.level_enabled(metadata) {
            return Interest::never();
        }

        let id = metadata.callsite();

        // TODO(eliza): if we can't write a metadata, that's bad news...
        let sent = self.send_event(1024, || TraceEvent::RegisterMeta {
            id: mnemos_trace_proto::MetaId::from(id),
            meta: metadata.as_serde(),
        });

        // If we couldn't send the metadata, skip this callsite, because the
        // consumer will not be able to understand it without its metadata.
        if !sent {
            self.shared.dropped_metas.fetch_add(1, Ordering::Relaxed);
            return Interest::never();
        }

        // Due to the fact that the collector uses `bbq` internally, we must
        // return `Interest::sometimes` rather than `Interest::always` for
        // `bbq` callsites, so that they can be dynamically enabled/disabled
        // by the `enabled` method based on whether or not we are inside the
        // collector. This avoids an infinite loop that previously occurred
        // when enabling the `TRACE` level.
        if metadata.target().starts_with("kernel::comms::bbq") {
            return Interest::sometimes();
        }

        // Otherwise, always enable this callsite.
        Interest::always()
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(u8_to_level(self.shared.max_level.load(Ordering::Relaxed)))
    }

    fn new_span(&self, span: &span::Attributes<'_>) -> span::Id {
        let id = {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            span::Id::from_u64(id)
        };

        if !self.send_event(1024, || TraceEvent::NewSpan {
            id: id.as_serde(),
            meta: span.metadata().callsite().into(),
            parent: span.parent().map(AsSerde::as_serde),
            is_root: span.is_root(),
            fields: SerializeSpanFields::Ser(span.values()),
        }) {
            self.shared.dropped_spans.fetch_add(1, Ordering::Relaxed);
        }

        id
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {
        // todo!("eliza")
    }

    fn enter(&self, span: &span::Id) {
        if !self.send_event(16, || TraceEvent::Enter(span.as_serde())) {
            self.shared
                .dropped_span_activity
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn exit(&self, span: &span::Id) {
        if !self.send_event(16, || TraceEvent::Exit(span.as_serde())) {
            self.shared
                .dropped_span_activity
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {
        // nop for now
    }

    fn event(&self, event: &Event<'_>) {
        if !self.send_event(1024, || TraceEvent::Event {
            meta: event.metadata().callsite().into(),
            fields: SerializeRecordFields::Ser(event),
            parent: event.parent().map(AsSerde::as_serde),
        }) {
            self.shared.dropped_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn clone_span(&self, span: &span::Id) -> span::Id {
        if !self.send_event(16, || TraceEvent::CloneSpan(span.as_serde())) {
            self.shared
                .dropped_span_activity
                .fetch_add(1, Ordering::Relaxed);
        }
        span.clone()
    }

    fn try_close(&self, span: span::Id) -> bool {
        if !self.send_event(16, || TraceEvent::DropSpan(span.as_serde())) {
            self.shared
                .dropped_span_activity
                .fetch_add(1, Ordering::Relaxed);
        }
        false
    }
}

// === impl SermuxTraceSettings ===

impl SerialTraceSettings {
    pub const DEFAULT_PORT: u16 = serial_mux::WellKnown::BinaryTracing as u16;
    pub const DEFAULT_SENDBUF_CAPACITY: usize = 1024;
    pub const DEFAULT_TRACEBUF_CAPACITY: usize = Self::DEFAULT_SENDBUF_CAPACITY * 64;
    pub const DEFAULT_INITIAL_LEVEL: LevelFilter = LevelFilter::OFF;

    #[must_use]
    pub const fn new() -> Self {
        Self {
            port: Self::DEFAULT_PORT,
            sendbuf_capacity: Self::DEFAULT_SENDBUF_CAPACITY,
            tracebuf_capacity: Self::DEFAULT_TRACEBUF_CAPACITY,
            initial_level: Self::DEFAULT_INITIAL_LEVEL,
        }
    }

    /// Sets the [`serial_mux`] port on which the binary tracing service is
    /// served.
    ///
    /// By default, this is [`Self::DEFAULT_PORT`] (the value of
    /// [`serial_mux::WellKnown::BinaryTracing`]).
    #[must_use]
    pub fn with_port(self, port: impl Into<u16>) -> Self {
        Self {
            port: port.into(),
            ..self
        }
    }

    /// Sets the initial [`LevelFilter`] used when no trace client is connected
    /// or when the trace client does not select a level.
    ///
    /// By default, this set to [`Self::DEFAULT_INITIAL_LEVEL`] ([`LevelFilter::OFF`]).
    #[must_use]
    pub fn with_initial_level(self, level: impl Into<LevelFilter>) -> Self {
        Self {
            initial_level: level.into(),
            ..self
        }
    }

    /// Sets the maximum capacity of the serial port send buffer (the buffer
    /// used for communication between the trace service task and the serial mux
    /// server).
    ///
    /// By default, this set to [`Self::DEFAULT_SENDBUF_CAPACITY`] (1 KB).
    #[must_use]
    pub const fn with_sendbuf_capacity(self, capacity: usize) -> Self {
        Self {
            sendbuf_capacity: capacity,
            ..self
        }
    }

    /// Sets the maximum capacity of the trace ring buffer (the buffer into
    /// which new traces are serialized before being sent to the worker task).
    ///
    /// Note that *two* buffers of this size will be allocated. One buffer is
    /// used for traces emitted by non-interrupt kernel code, and the other is
    /// used for traces emitted inside of interrupt service routines (ISRs).
    ///
    /// By default, this set to [`Self::DEFAULT_TRACEBUF_CAPACITY`] (64 KB).
    #[must_use]
    pub const fn with_tracebuf_capacity(self, capacity: usize) -> Self {
        Self {
            tracebuf_capacity: capacity,
            ..self
        }
    }
}

impl Default for SerialTraceSettings {
    fn default() -> Self {
        Self::new()
    }
}

const fn level_to_u8(level: LevelFilter) -> u8 {
    match level {
        LevelFilter::TRACE => 0,
        LevelFilter::DEBUG => 1,
        LevelFilter::INFO => 2,
        LevelFilter::WARN => 3,
        LevelFilter::ERROR => 4,
        LevelFilter::OFF => 5,
    }
}

const fn u8_to_level(level: u8) -> LevelFilter {
    match level {
        0 => LevelFilter::TRACE,
        1 => LevelFilter::DEBUG,
        2 => LevelFilter::INFO,
        3 => LevelFilter::WARN,
        4 => LevelFilter::ERROR,
        _ => LevelFilter::OFF,
    }
}
