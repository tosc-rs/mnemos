use mnemos_trace_proto::{MetaId, TraceEvent};
use postcard::accumulator::{CobsAccumulator, FeedResult};
use std::{
    collections::HashMap,
    fmt::{self, Write},
    num::NonZeroU64,
    sync::mpsc,
    time::Instant,
};
use tracing_serde_structured::{
    CowString, SerializeLevel, SerializeMetadata, SerializeRecordFields, SerializeSpanFields,
    SerializeValue,
};

use crate::LogTag;
use owo_colors::{OwoColorize, Stream};

pub(crate) fn decode(rx: mpsc::Receiver<Vec<u8>>, tag: LogTag, verbose: bool) {
    let mut cobs_buf: CobsAccumulator<1024> = CobsAccumulator::new();
    let mut state: TraceState = TraceState {
        tag,
        verbose,
        spans: HashMap::new(),
        metas: HashMap::new(),
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
    tag: LogTag,
    verbose: bool,
    spans: HashMap<NonZeroU64, Span>,
    metas: HashMap<MetaId, SerializeMetadata<'static>>,
    stack: Vec<NonZeroU64>,
    textbuf: String,
}

struct Span {
    repr: String,
    level: DisplayLevel,
    target: String,
    start: Instant,
    // TODO(eliza): reference count spans
    refs: usize,
}

impl TraceState {
    fn event(&mut self, ev: TraceEvent<'_>) {
        match ev {
            TraceEvent::RegisterMeta { id, meta } => {
                if self.verbose {
                    write!(
                        &mut self.textbuf,
                        "{} {} {} {}{}{id:?}: {} {} [{}:{}]",
                        self.tag,
                        "META".if_supports_color(Stream::Stdout, |x| x.bright_blue()),
                        DisplayLevel(meta.level),
                        if meta.is_event { "EVNT " } else { "" }
                            .if_supports_color(Stream::Stdout, |x| x.bright_yellow()),
                        if meta.is_span { "SPAN " } else { "" }
                            .if_supports_color(Stream::Stdout, |x| x.bright_magenta()),
                        meta.target
                            .as_str()
                            .if_supports_color(Stream::Stdout, |x| x.italic()),
                        meta.name
                            .as_str()
                            .if_supports_color(Stream::Stdout, |x| x.bold()),
                        meta.file
                            .as_ref()
                            .map(CowString::as_str)
                            .unwrap_or("<unknown>"),
                        meta.line.unwrap_or(0),
                    )
                    .unwrap();
                    println!("{}", self.textbuf);
                    self.textbuf.clear();
                }
                self.metas.insert(id, meta.to_owned());
            }
            TraceEvent::Event {
                meta,
                parent: _,
                fields,
            } => {
                let Some(meta) =  self.metas.get(&meta) else {
                    println!("{} {} UNKNOWN: {meta:?}", self.tag, "META".if_supports_color(Stream::Stdout, |x| x.bright_blue()));
                    return;
                };
                let target = meta.target.as_str();
                let target = target.if_supports_color(Stream::Stdout, |target| target.italic());
                let level = DisplayLevel(meta.level);
                write!(&mut self.textbuf, "{} {level} {target}: ", self.tag).unwrap();
                write_span_cx(&self.stack, &self.spans, &mut self.textbuf);
                let SerializeRecordFields::De(ref fields) = fields else {
                    unreachable!("we are deserializing!");
                };
                write_fields(&mut self.textbuf, fields);
                println!("{}", self.textbuf);
                self.textbuf.clear();
            }
            TraceEvent::NewSpan {
                id,
                meta,
                fields,
                parent: _,
                is_root: _,
            } => {
                let start = Instant::now();
                let mut repr = String::new();
                let Some(meta) = self.metas.get(&meta) else {
                    println!("{} {} UNKNOWN: {meta:?}", self.tag, "META".if_supports_color(Stream::Stdout, |x| x.bright_blue()));
                    return;
                };

                let name = meta.name.as_str();
                write!(
                    repr,
                    "{}",
                    format_args!("{name}{{").if_supports_color(Stream::Stdout, |x| x.bold())
                )
                .unwrap();
                let SerializeSpanFields::De(ref fields) = fields else {
                    unreachable!("we are deserializing!");
                };
                write_fields(&mut repr, fields);
                write!(
                    repr,
                    "{}",
                    "}".if_supports_color(Stream::Stderr, |x| x.bold())
                )
                .unwrap();

                let level = DisplayLevel(meta.level);
                let target = meta.target.as_str();
                write!(
                    &mut self.textbuf,
                    "{} {level} {} ",
                    self.tag,
                    "SPAN".if_supports_color(Stream::Stdout, |x| x.bright_magenta())
                )
                .unwrap();
                write_span_cx(&self.stack, &self.spans, &mut self.textbuf);
                write!(
                    &mut self.textbuf,
                    "{}::{repr} ({:04})",
                    target.if_supports_color(Stream::Stdout, |target| target.italic()),
                    id.id,
                )
                .unwrap();
                println!("{}", self.textbuf);
                self.textbuf.clear();

                self.spans.insert(
                    id.id,
                    Span {
                        target: target.to_string(),
                        level,
                        repr,
                        start,
                        refs: 1,
                    },
                );
            }
            TraceEvent::Enter(id) => {
                self.stack.push(id.id);
            }
            TraceEvent::Exit(_id) => {
                self.stack.pop();
            }
            TraceEvent::CloneSpan(id) => {
                if let Some(span) = self.spans.get_mut(&id.id) {
                    span.refs += 1;
                }
            }
            TraceEvent::DropSpan(id) => {
                let end = if let Some(span) = self.spans.get_mut(&id.id) {
                    span.refs -= 1;
                    span.refs == 0
                } else {
                    return;
                };

                if end {
                    let Span {
                        repr,
                        target,
                        level,
                        start,
                        refs: _,
                    } = self.spans.remove(&id.id).unwrap();
                    let end = "END".if_supports_color(Stream::Stdout, |x| x.bright_red());
                    let target = target.if_supports_color(Stream::Stdout, |x| x.italic());
                    write!(
                        &mut self.textbuf,
                        "{} {level}  {end} {target}::{repr} ({:04}): {:?}",
                        self.tag,
                        id.id,
                        start.elapsed()
                    )
                    .unwrap();
                    println!("{}", self.textbuf);
                    self.textbuf.clear();
                }
            }
        }
    }
}

fn write_span_cx(stack: &[NonZeroU64], spans: &HashMap<NonZeroU64, Span>, textbuf: &mut String) {
    let spans = stack.iter().filter_map(|id| spans.get(id));
    let mut any = false;
    for span in spans {
        textbuf.push_str(span.repr.as_str());
        textbuf.push(':');
        any = true;
    }
    if any {
        textbuf.push(' ');
    }
}

fn write_fields<'a>(
    to: &mut String,
    fields: impl IntoIterator<Item = (&'a CowString<'a>, &'a SerializeValue<'a>)>,
) {
    let mut fields = fields.into_iter();
    if let Some((key, val)) = fields.next() {
        write_kv(key, val, to);
        for (key, val) in fields {
            write!(
                to,
                "{}",
                ", ".if_supports_color(Stream::Stdout, |delim| delim.dimmed())
            )
            .unwrap();
            write_kv(key, val, to);
        }
    }
}

fn write_kv(key: &CowString<'_>, val: &SerializeValue<'_>, to: &mut String) {
    use tracing_serde_structured::DebugRecord;

    let key = key.as_str();
    let key = key.if_supports_color(Stream::Stdout, |k| k.bold());
    write!(
        to,
        "{key}{}",
        "=".if_supports_color(Stream::Stdout, |delim| delim.dimmed())
    )
    .unwrap();

    match val {
        SerializeValue::Debug(DebugRecord::De(d)) => to.push_str(d.as_str()),
        SerializeValue::Debug(DebugRecord::Ser(d)) => write!(to, "{d}").unwrap(),
        SerializeValue::Str(s) => write!(to, "{s:?}").unwrap(),
        SerializeValue::F64(x) => write!(to, "{x}").unwrap(),
        SerializeValue::I64(x) => write!(to, "{x}").unwrap(),
        SerializeValue::U64(x) => write!(to, "{x}").unwrap(),
        SerializeValue::Bool(x) => write!(to, "{x}").unwrap(),
        _ => to.push_str("???"),
    }
}

struct DisplayLevel(SerializeLevel);

impl fmt::Display for DisplayLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            SerializeLevel::Trace => write!(
                f,
                "{}",
                "TRCE".if_supports_color(Stream::Stdout, |l| l.purple())
            ),
            SerializeLevel::Debug => write!(
                f,
                "{}",
                "DBUG".if_supports_color(Stream::Stdout, |l| l.blue())
            ),
            SerializeLevel::Info => write!(
                f,
                "{}",
                "INFO".if_supports_color(Stream::Stdout, |l| l.green())
            ),
            SerializeLevel::Warn => write!(
                f,
                "{}",
                "WARN".if_supports_color(Stream::Stdout, |l| l.yellow())
            ),
            SerializeLevel::Error => write!(
                f,
                "{}",
                "ERR".if_supports_color(Stream::Stdout, |l| l.red())
            ),
        }
    }
}