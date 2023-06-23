use crate::{comms::bbq, drivers::serial_mux};
use level_filters::LevelFilter;
use mnemos_trace_proto::TraceEvent;
use mycelium_util::sync::InitOnce;
use portable_atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};

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

    max_level: LevelFilter,
}

// === impl SerialCollector ===

impl SerialCollector {
    pub const PORT: u16 = 3;
    const CAPACITY: usize = 1024 * 4;

    pub const fn new(max_level: LevelFilter) -> Self {
        Self {
            tx: InitOnce::uninitialized(),
            current_span: AtomicU64::new(0),
            current_meta: AtomicPtr::new(core::ptr::null_mut()),
            next_id: AtomicU64::new(1),
            dropped_events: AtomicUsize::new(0),
            max_level,
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
        k.spawn(Self::worker(rx, port)).await;
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

    async fn worker(rx: bbq::Consumer, port: serial_mux::PortHandle) {
        loop {
            let rgr = rx.read_grant().await;
            let len = rgr.len();
            port.send(&rgr[..]).await;
            rgr.release(len);
        }
    }
}

impl Collect for SerialCollector {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        // TODO(eliza): more sophisticated filtering
        metadata.level() <= &self.max_level
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
        Some(self.max_level)
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
