use crate::{comms::bbq, drivers::serial_mux};
use level_filters::LevelFilter;
use mnemos_trace_proto::{HostRequest, TraceEvent};
use mycelium_util::sync::InitOnce;
use portable_atomic::{AtomicPtr, AtomicU64, AtomicU8, AtomicUsize, Ordering};

pub use tracing_02::*;
use tracing_core_02::span::Current;
use tracing_serde_structured::{AsSerde, SerializeRecordFields, SerializeSpanFields};

pub struct SerialCollector {
    tx: InitOnce<bbq::SpscProducer>,

    /// ID of the current span.
    ///
    /// **Note**: This collector only works correctly on single-threaded hardware!
    current_span: AtomicU64,
    current_meta: AtomicPtr<Metadata<'static>>,

    /// ID of the next span.
    next_id: AtomicU64,

    /// Counter of events that were dropped due to insufficient buffer capacity.
    ///
    // TODO(eliza): Currently, this is recorded but not actually consumed...
    dropped_events: AtomicUsize,

    max_level: AtomicU8,
}

// === impl SerialCollector ===

impl SerialCollector {
    pub const PORT: u16 = 3;
    const CAPACITY: usize = 1024 * 4;

    pub const fn new() -> Self {
        Self::with_max_level(LevelFilter::OFF)
    }

    pub const fn with_max_level(max_level: LevelFilter) -> Self {
        Self {
            tx: InitOnce::uninitialized(),
            current_span: AtomicU64::new(0),
            current_meta: AtomicPtr::new(core::ptr::null_mut()),
            next_id: AtomicU64::new(1),
            dropped_events: AtomicUsize::new(0),
            max_level: AtomicU8::new(level_to_u8(max_level)),
        }
    }

    pub async fn start(&'static self, k: &'static crate::Kernel) {
        let mut mux = serial_mux::SerialMuxClient::from_registry(k)
            .await
            .expect("cannot initialize serial tracing, no serial mux exists!");
        let port = mux
            .open_port(3, 1024)
            .await
            .expect("cannot initialize serial tracing, cannot open port 3!");
        let (tx, rx) = bbq::new_spsc_channel(k.heap(), Self::CAPACITY).await;
        self.tx.init(tx);
        k.spawn(Self::worker(self, rx, port)).await;
        let dispatch = tracing_02::Dispatch::from_static(self);
        tracing_02::dispatch::set_global_default(dispatch)
            .expect("cannot set global default tracing dispatcher");
    }

    /// Serialize a `TraceEvent`, returning `true` if the event was correctly serialized.
    fn send_event<'a>(&self, sz: usize, event: impl FnOnce() -> TraceEvent<'a>) -> bool {
        let Some(mut wgr) = self.tx.get().send_grant_exact_sync(sz) else {
            self.dropped_events.fetch_add(1, Ordering::Relaxed);
            return false;
        };

        // got a write grant! generate the event payload.
        let ev = event();

        // encode the event to our write grant, and commit however many bytes
        // were written. if encoding fails, commit 0 bytes, so the region we got
        // a write grant for can be reused.
        let len = match postcard::to_slice_cobs(&ev, &mut wgr[..]) {
            Ok(encoded) => encoded.len(),
            Err(_) => {
                self.dropped_events.fetch_add(1, Ordering::Relaxed);
                0
            }
        };
        wgr.commit(len);

        // return true if we committed a non-zero number of bytes.
        len > 0
    }

    async fn worker(&'static self, rx: bbq::Consumer, port: serial_mux::PortHandle) {
        use futures::FutureExt;
        use maitake::time;
        use postcard::accumulator::{CobsAccumulator, FeedResult};
        let mut heartbeat = [0u8; 3];
        let heartbeat = postcard::to_slice_cobs(&TraceEvent::Heartbeat, &mut heartbeat[..])
            .expect("failed to encode heartbeat msg");

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
                                let lvl = lvl.map(|lvl| lvl as u8).unwrap_or(5);
                                info!("setting max level to {lvl}");
                                // self.max_level.store(lvl, Ordering::Relaxed);
                                // tracing_core_02::callsite::rebuild_interest_cache();
                            }
                        }

                        remaining
                    }
                };
            }
            rgr.release(len);
        };

        // loop {
        'idle: loop {
            port.send(heartbeat).await;
            if let Ok(rgr) =
                time::timeout(time::Duration::from_secs(2), port.consumer().read_grant()).await
            {
                read_level(rgr);
                break 'idle;
            }
        }

        loop {
            futures::select_biased! {
                rgr = rx.read_grant().fuse() => {
                    let len = rgr.len();
                    port.send(&rgr[..]).await;
                    rgr.release(len)
                },
                rgr = port.consumer().read_grant().fuse() => {
                    read_level(rgr);
                },
                // TODO(eliza): make the host also send a heartbeat, and
                // if we don't get it, break back to the idle loop...
            }
        }
        // }
    }
}

impl Collect for SerialCollector {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        // TODO(eliza): more sophisticated filtering
        metadata.level() <= &u8_to_level(self.max_level.load(Ordering::Relaxed))
    }

    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> tracing_core_02::Interest {
        if !self.enabled(metadata) {
            return tracing_core_02::Interest::never();
        }

        let id = metadata.callsite();

        // TODO(eliza): if we can't write a metadata, that's bad news...
        let sent = self.send_event(1024, || TraceEvent::RegisterMeta {
            id: mnemos_trace_proto::MetaId::from(id),
            meta: metadata.as_serde(),
        });

        if sent {
            tracing_core_02::Interest::always()
        } else {
            // if we couldn't send the metadata, skip this callsite, because the
            // consumer will not be able to understand it without its metadata.
            tracing_core_02::Interest::never()
        }
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(u8_to_level(self.max_level.load(Ordering::Relaxed)))
    }

    fn new_span(&self, span: &span::Attributes<'_>) -> span::Id {
        let id = {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            span::Id::from_u64(id)
        };

        self.send_event(1024, || TraceEvent::NewSpan {
            id: id.as_serde(),
            meta: span.metadata().callsite().into(),
            parent: span.parent().map(AsSerde::as_serde),
            is_root: span.is_root(),
            fields: SerializeSpanFields::Ser(span.values()),
        });

        id
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {
        // todo!("eliza")
    }

    fn enter(&self, span: &span::Id) {
        self.send_event(16, || TraceEvent::Enter(span.as_serde()));
    }

    fn exit(&self, span: &span::Id) {
        self.send_event(16, || TraceEvent::Exit(span.as_serde()));
    }

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {
        // nop for now
    }

    fn event(&self, event: &Event<'_>) {
        self.send_event(1024, || TraceEvent::Event {
            meta: event.metadata().callsite().into(),
            fields: SerializeRecordFields::Ser(event),
            parent: event.parent().map(AsSerde::as_serde),
        });
    }

    fn current_span(&self) -> Current {
        let id = match core::num::NonZeroU64::new(self.current_span.load(Ordering::Acquire)) {
            Some(id) => Id::from_non_zero_u64(id),
            None => return Current::none(),
        };
        let meta = match core::ptr::NonNull::new(self.current_meta.load(Ordering::Acquire)) {
            Some(meta) => unsafe {
                // safety: it's guaranteed to have been an `&'static Metadata<'static>`
                meta.as_ref()
            },
            None => return Current::none(),
        };
        Current::new(id, meta)
    }

    fn clone_span(&self, span: &span::Id) -> span::Id {
        self.send_event(16, || TraceEvent::CloneSpan(span.as_serde()));
        span.clone()
    }

    fn try_close(&self, span: span::Id) -> bool {
        self.send_event(16, || TraceEvent::DropSpan(span.as_serde()));
        false
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
