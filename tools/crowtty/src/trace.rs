use postcard::accumulator::{CobsAccumulator, FeedResult};
use std::{time::Instant, sync::mpsc, fmt::Write, collections::HashMap, num::NonZeroU64};
use tracing_serde_structured::{SerializeRecordFields, SerializeSpanFields, SerializeValue, CowString};
use mnemos_trace_proto::TraceEvent;

pub(crate) fn decode(rx: mpsc::Receiver<Vec<u8>>, start: Instant) {
    let mut cobs_buf: CobsAccumulator<1024> = CobsAccumulator::new();
    let mut state: TraceState = TraceState {
        trace_start: start,
        spans: HashMap::new(),
        stack: Vec::new(),
        textbuf: String::new(),
    };

    while let Ok(chunk) = rx.recv() {
        let mut window = &chunk[..];

        'cobs: while !window.is_empty() {
            window = match cobs_buf.feed_ref::<TraceEvent<'_>>(window) {
                FeedResult::Consumed => break 'cobs,
                FeedResult::OverFull(new_wind) => new_wind,
                FeedResult::DeserError(new_wind) => new_wind,
                FeedResult::Success { data, remaining } => {
                    state.event(data);

                    remaining
                }
            };
        }
    }
    println!("trace channel over");
}

struct TraceState {
    trace_start: Instant,
    spans: HashMap<NonZeroU64, Span>,
    stack: Vec<NonZeroU64>,
    textbuf: String,
}

struct Span {
    repr: String,
    start: Instant,
    // TODO(eliza): reference count spans
    refs: usize,
}

impl TraceState {
    fn write_span_cx(&mut self) {
        let spans = self.stack.iter().filter_map(|id| self.spans.get(id));
        for span in spans {
            self.textbuf.push_str(span.repr.as_str());
            self.textbuf.push(':');
        }
    }

    fn event(&mut self, ev: TraceEvent<'_>) {
        let now = Instant::now();
        let elapsed = now - self.trace_start;
        match ev {
            TraceEvent::Event(ev) => {
                let target = ev.metadata.target.as_str();
                let level = ev.metadata.level;
                write!(&mut self.textbuf, "[3 +{elapsed:4.8?}] {level:<5?} ").unwrap();
                self.write_span_cx();
                write!(&mut self.textbuf, "{target}: ").unwrap();
                let SerializeRecordFields::De(ref fields) = ev.fields else {
                    unreachable!("we are deserializing!");
                };
                write_fields(&mut self.textbuf, fields);
                println!("{}", self.textbuf);
                self.textbuf.clear();
            }
            TraceEvent::NewSpan { id, attributes} => {
                let mut repr = String::new();
                repr.push_str(attributes.metadata.name.as_str());
                repr.push('{');
                let SerializeSpanFields::De(ref fields) = attributes.fields else {
                    unreachable!("we are deserializing!");
                };
                write_fields(&mut repr, fields);
                repr.push('}');

                let level = attributes.metadata.level;
                let target = attributes.metadata.target.as_str();
                write!(&mut self.textbuf, "[3 +{elapsed:4.8?}] {level:<5?} ").unwrap();
                self.write_span_cx();
                write!(&mut self.textbuf, "-> {target}::{repr}").unwrap();
                println!("{}", self.textbuf);
                self.textbuf.clear();

                self.spans.insert(id.id, Span {
                    repr,
                    start: now,
                    refs: 1,
                });
            }
            TraceEvent::Enter(id) => { self.stack.push(id.id); },
            TraceEvent::Exit(_id) => { self.stack.pop(); },
            // TODO(eliza)
            TraceEvent::CloneSpan(_) => {},
            // TODO(eliza)
            TraceEvent::DropSpan(_) => {},
        }
    }
}

fn write_fields<'a>(to: &mut String, fields: impl IntoIterator<Item = (&'a CowString<'a>, &'a SerializeValue<'a>)>) {

    let mut fields = fields.into_iter();
    if let Some((key, val)) = fields.next() {
        write_kv(key, val, to);
        for (key, val) in fields {
            to.push_str(", ");
            write_kv(key, val, to);
        }
    }
}

fn write_kv(key: &CowString<'_>, val: &SerializeValue<'_>, to: &mut String) {
    use tracing_serde_structured::DebugRecord;

    let key = key.as_str();
    to.push_str(key);
    to.push('=');

    match val {
        SerializeValue::Debug(DebugRecord::De(d)) => to.push_str(d.as_str()),
        SerializeValue::Debug(DebugRecord::Ser(d)) => write!(to, "{d}").unwrap(),
        SerializeValue::Str(s) => write!(to, "{s:?}").unwrap(),
        SerializeValue::F64(x) => write!(to, "{x}").unwrap(),
        SerializeValue::I64(x) => write!(to, "{x}").unwrap(),
        SerializeValue::U64(x) => write!(to, "{x}").unwrap(),
        SerializeValue::Bool(x)  => write!(to, "{x}").unwrap(),
        _ => to.push_str("???"),
    }
}