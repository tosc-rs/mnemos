use mnemos_trace_proto::{HostRequest, MetaId, TraceEvent};
use postcard::accumulator::{CobsAccumulator, FeedResult};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::{self, Write},
    num::NonZeroU64,
    sync::mpsc,
    time::Instant,
};
use tracing::{level_filters::LevelFilter, subscriber::NoSubscriber, Level};
use tracing_serde_structured::{
    CowString, SerializeLevel, SerializeMetadata, SerializeRecordFields, SerializeSpanFields,
    SerializeValue,
};
use tracing_subscriber::{filter::Targets, layer::Layer};

use crate::LogTag;
use owo_colors::{OwoColorize, Stream};

pub(crate) struct TraceWorker {
    tx: mpsc::Sender<Vec<u8>>,
    rx: mpsc::Receiver<Vec<u8>>,
    tag: LogTag,
    spans: HashMap<NonZeroU64, Span>,
    metas: HashMap<MetaId, SerializeMetadata<'static>>,
    stack: Vec<NonZeroU64>,
    textbuf: String,
    has_set_max_level: bool,
    /// A set of `tracing` targets and levels to enable.
    ///
    /// Currently, this is processed on the crowtty side, and we only send the
    /// max level to enable to the target. In the future, it would be more
    /// efficient if the target could handle more granular filtering, since we
    /// could disable more target-side instrumentation points. But, this works
    /// fine for the time being.
    filter: Targets,
    ser_max_level: Option<SerializeLevel>,
}

impl TraceWorker {
    pub fn new(
        filter: Targets,
        tx: mpsc::Sender<Vec<u8>>,
        rx: mpsc::Receiver<Vec<u8>>,
        tag: LogTag,
    ) -> Self {
        let ser_max_level = <Targets as Layer<NoSubscriber>>::max_level_hint(&filter).and_then(
            |level| match level {
                LevelFilter::OFF => None,
                LevelFilter::ERROR => Some(SerializeLevel::Error),
                LevelFilter::WARN => Some(SerializeLevel::Warn),
                LevelFilter::INFO => Some(SerializeLevel::Info),
                LevelFilter::DEBUG => Some(SerializeLevel::Debug),
                LevelFilter::TRACE => Some(SerializeLevel::Trace),
            },
        );
        Self {
            tx,
            rx,
            tag,
            spans: HashMap::new(),
            metas: HashMap::new(),
            stack: Vec::new(),
            textbuf: String::new(),
            ser_max_level,
            has_set_max_level: false,
            filter,
        }
    }
}

struct Span {
    repr: String,
    level: DisplayLevel,
    target: String,
    start: Instant,
    // TODO(eliza): reference count spans
    refs: usize,
}

impl TraceWorker {
    pub(crate) fn run(mut self) {
        let mut cobs_buf: CobsAccumulator<1024> = CobsAccumulator::new();

        while let Ok(chunk) = self.rx.recv() {
            let mut window = &chunk[..];

            'cobs: while !window.is_empty() {
                window = match cobs_buf.feed_ref::<TraceEvent<'_>>(window) {
                    FeedResult::Consumed => break 'cobs,
                    FeedResult::OverFull(new_wind) => new_wind,
                    FeedResult::DeserError(new_wind) => new_wind,
                    FeedResult::Success { data, remaining } => {
                        self.event(data);

                        remaining
                    }
                };
            }
        }
        println!("trace channel over");
    }

    fn event(&mut self, ev: TraceEvent<'_>) {
        match ev {
            TraceEvent::Heartbeat(level) => {
                if self.tag.verbose {
                    println!(
                        "{} {} Found a heartbeat (level: {:?}; desired: {:?})",
                        self.tag,
                        "BEAT".if_supports_color(Stream::Stdout, |x| x.bright_red()),
                        level.map(DisplayLevel),
                        self.ser_max_level.map(DisplayLevel),
                    );
                }

                if level == self.ser_max_level {
                    if !self.has_set_max_level || self.tag.verbose {
                        println!(
                            "{} {} Max level set to {:?}",
                            self.tag,
                            "BEAT".if_supports_color(Stream::Stdout, |x| x.bright_red()),
                            level.map(DisplayLevel)
                        );
                    }

                    self.has_set_max_level = true;
                    return;
                } else {
                    self.has_set_max_level = false;
                }

                let req = postcard::to_allocvec_cobs(&HostRequest::SetMaxLevel(self.ser_max_level))
                    .expect("failed to serialize max level request");
                self.tx.send(req).expect("failed to send host request");
                if self.tag.verbose {
                    println!(
                        "{} {} Sent request for {:?}",
                        self.tag,
                        "BEAT".if_supports_color(Stream::Stdout, |x| x.bright_red()),
                        self.ser_max_level.map(DisplayLevel),
                    );
                }
            }
            TraceEvent::RegisterMeta { id, meta } => {
                if self.tag.verbose {
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
                            .if_supports_color(Stream::Stdout, |x| x.dimmed()),
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
                let Some(meta) = self.metas.get(&meta) else {
                    println!(
                        "{} {} UNKNOWN: {meta:?}",
                        self.tag,
                        "META".if_supports_color(Stream::Stdout, |x| x.bright_blue())
                    );
                    return;
                };
                let target = meta.target.as_str();

                // if the filter disables this event, skip it.
                if !self.filter.would_enable(target, &ser_lvl(meta.level)) {
                    return;
                }

                let level = DisplayLevel(meta.level);
                write!(
                    &mut self.textbuf,
                    "{} {level} {} ",
                    self.tag,
                    format_args!("{target}:")
                        .if_supports_color(Stream::Stdout, |target| target.dimmed())
                )
                .unwrap();
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
                    println!(
                        "{} {} UNKNOWN: {meta:?}",
                        self.tag,
                        "META".if_supports_color(Stream::Stdout, |x| x.bright_blue())
                    );
                    return;
                };

                let target = meta.target.as_str();
                let level = DisplayLevel(meta.level);
                let name = meta.name.as_str();

                // does our filter actually enable this span?
                if !self.filter.would_enable(target, &ser_lvl(meta.level)) {
                    return;
                }

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
                    "{}{repr} ({:04})",
                    format_args!("{target}::")
                        .if_supports_color(Stream::Stdout, |target| target.dimmed()),
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
                // only put a span on the stack if we enabled it when it was created.
                if self.spans.contains_key(&id.id) {
                    self.stack.push(id.id);
                }
            }
            TraceEvent::Exit(id) => {
                // only popped the span if we enabled it when it was created.
                if self.spans.contains_key(&id.id) {
                    let popped = self.stack.pop();
                    debug_assert_eq!(popped, Some(id.id));
                }
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
                    // we previously skipped this span, so ignore it.
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
                    write!(
                        &mut self.textbuf,
                        "{} {level}  {end} {}{repr} ({:04}): {:?}",
                        self.tag,
                        format_args!("{target}::")
                            .if_supports_color(Stream::Stdout, |target| target.dimmed()),
                        id.id,
                        start.elapsed()
                    )
                    .unwrap();
                    println!("{}", self.textbuf);
                    self.textbuf.clear();
                }
            }
            dropped @ TraceEvent::Discarded { .. } => {
                println!("{} {dropped:?}", self.tag);
            }
        }
    }
}

fn write_span_cx(stack: &[NonZeroU64], spans: &HashMap<NonZeroU64, Span>, textbuf: &mut String) {
    let spans = stack.iter().filter_map(|id| spans.get(id));
    let mut any = false;
    let delim = ":".if_supports_color(Stream::Stdout, |x| x.dimmed());
    for span in spans {
        textbuf.push_str(span.repr.as_str());
        write!(textbuf, "{delim}").unwrap();
        any = true;
    }
    if any {
        textbuf.push(' ');
    }
}

fn write_fields<'a>(to: &mut String, fields: &BTreeMap<CowString<'a>, SerializeValue<'a>>) {
    let comma = ", ".if_supports_color(Stream::Stdout, |delim| delim.dimmed());
    let mut wrote_anything = false;

    if let Some(message) = fields.get(&CowString::Borrowed("message")) {
        let message = DisplayVal(message);
        let message = message.if_supports_color(Stream::Stdout, |msg| msg.bold());
        write!(to, "{message}",).unwrap();
        wrote_anything = true;
    }

    for (key, val) in fields {
        if key.as_str() == "message" {
            continue;
        }
        if wrote_anything {
            write!(to, "{comma}").unwrap();
        } else {
            wrote_anything = true;
        }
        write_kv(key, val, to);
    }
}

fn write_kv(key: &CowString<'_>, val: &SerializeValue<'_>, to: &mut String) {
    let key = key.as_str();
    let key = key.if_supports_color(Stream::Stdout, |k| k.italic());
    let val = DisplayVal(val);
    write!(
        to,
        "{key}{}{val}",
        "=".if_supports_color(Stream::Stdout, |delim| delim.dimmed())
    )
    .unwrap();
}

struct DisplayVal<'a>(&'a SerializeValue<'a>);

impl fmt::Display for DisplayVal<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use tracing_serde_structured::DebugRecord;
        match self.0 {
            SerializeValue::Debug(DebugRecord::De(d)) => f.write_str(d.as_str()),
            SerializeValue::Debug(DebugRecord::Ser(d)) => write!(f, "{d}"),
            SerializeValue::Str(s) => write!(f, "{s:?}"),
            SerializeValue::F64(x) => write!(f, "{x}"),
            SerializeValue::I64(x) => write!(f, "{x}"),
            SerializeValue::U64(x) => write!(f, "{x}"),
            SerializeValue::Bool(x) => write!(f, "{x}"),
            _ => f.write_str("???"),
        }
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

impl fmt::Debug for DisplayLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

fn ser_lvl(lvl: SerializeLevel) -> Level {
    match lvl {
        SerializeLevel::Trace => Level::TRACE,
        SerializeLevel::Debug => Level::DEBUG,
        SerializeLevel::Info => Level::INFO,
        SerializeLevel::Warn => Level::WARN,
        SerializeLevel::Error => Level::ERROR,
    }
}
