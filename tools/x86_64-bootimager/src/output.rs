use clap::Args;
use heck::ToTitleCase;
use tracing::{field::Field, Event, Level, Subscriber};
use tracing_subscriber::{
    field::Visit,
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields, FormattedFields},
    registry::LookupSpan,
};
use std::fmt;

pub use owo_colors::{style, OwoColorize, Style};


#[derive(Clone, Debug, Args)]
#[command(next_help_heading = "Output Options")]
pub struct Options {
    /// Tracing filter for the bootimage builder.
    #[clap(
        long = "trace",
        alias = "log",
        env = "RUST_LOG",
        default_value = "info",
        global = true
    )]
    trace_filter: tracing_subscriber::filter::Targets,

    /// Whether to emit colors in output.
    #[clap(
            long,
            env = "CARGO_TERM_COLORS",
            default_value_t = ColorMode::Auto,
            global = true,
        )]
    color: ColorMode,
}

pub const CARGO_LOG_WIDTH: usize = 12;

#[derive(Copy, Clone, Debug, Eq, PartialEq, clap::ValueEnum)]
#[repr(u8)]
#[clap(rename_all = "lower")]
enum ColorMode {
    /// Determine whether to color output based on whether or not stderr is a
    /// TTY.
    Auto = 0,
    /// Always color output.
    Always = 1,
    /// Never color output.
    Never = 2,
}

// === impl OutputOptions ===

impl Options {
    pub fn init(&self) -> anyhow::Result<()> {
        use tracing_subscriber::prelude::*;
        let fmt = tracing_subscriber::fmt::layer()
            .event_format(CargoFormatter {
                styles: Styles::new(self.color),
            })
            .with_writer(std::io::stderr);

        tracing_subscriber::registry()
            .with(fmt)
            .with(self.trace_filter.clone())
            .try_init()?;
        Ok(())
    }
}

// === impl ColorMode ===

impl fmt::Display for ColorMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl ColorMode {
    fn if_color(self, style: owo_colors::Style) -> owo_colors::Style {
        if self.should_color_stderr() {
            style
        } else {
            owo_colors::style()
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            ColorMode::Auto => "auto",
            ColorMode::Always => "always",
            ColorMode::Never => "never",
        }
    }

    fn should_color_stderr(self) -> bool {
        match self {
            ColorMode::Auto => supports_color::on(supports_color::Stream::Stderr)
                .map(|c| c.has_basic)
                .unwrap_or(false),
            ColorMode::Always => true,
            ColorMode::Never => false,
        }
    }
}

#[derive(Debug)]
struct CargoFormatter {
    styles: Styles,
}

struct Visitor<'styles, 'writer> {
    level: Level,
    writer: Writer<'writer>,
    is_empty: bool,
    styles: &'styles Styles,
    did_cargo_format: bool,
}

#[derive(Debug)]
struct Styles {
    error: Style,
    warn: Style,
    info: Style,
    debug: Style,
    trace: Style,
    pipes: Style,
    bold: Style,
}

struct Prefixed<T> {
    prefix: &'static str,
    val: T,
}

impl<S, N> FormatEvent<S, N> for CargoFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let level = metadata.level();

        let include_spans = {
            let mut visitor = self.visitor(*level, writer.by_ref());
            event.record(&mut visitor);
            !visitor.did_cargo_format && ctx.lookup_current().is_some()
        };

        writer.write_char('\n')?;

        if include_spans {
            writeln!(
                writer,
                "   {} {}{}",
                "-->".style(self.styles.pipes),
                metadata.file().unwrap_or_else(|| metadata.target()),
                DisplayOpt(metadata.line().map(Prefixed::prefix(":"))),
            )?;
            ctx.visit_spans(|span| {
                let exts = span.extensions();
                let fields = exts
                    .get::<FormattedFields<N>>()
                    .map(|f| f.fields.as_str())
                    .unwrap_or("");
                writeln!(
                    writer,
                    "    {} {}{}{}",
                    "|".style(self.styles.pipes),
                    span.name().style(self.styles.bold),
                    if fields.is_empty() { "" } else { ": " },
                    fields
                )
            })?;

            writer.write_char('\n')?;
        }

        Ok(())
    }
}

impl CargoFormatter {
    fn visitor<'styles, 'writer>(
        &'styles self,
        level: Level,
        writer: Writer<'writer>,
    ) -> Visitor<'styles, 'writer> {
        Visitor {
            level,
            writer,
            is_empty: true,
            styles: &self.styles,
            did_cargo_format: false,
        }
    }
}

// === impl Visitor ===

impl<'styles, 'writer> Visitor<'styles, 'writer> {
    const MESSAGE: &'static str = "message";
    const INDENT: usize = 12;
}

impl<'styles, 'writer> Visit for Visitor<'styles, 'writer> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        // If we're writing the first field of the event, either emit cargo
        // formatting, or a level header.
        if self.is_empty {
            // If the level is `INFO` and it has a message that's
            // shaped like a cargo log tag, emit the cargo tag followed by the
            // rest of the message.
            if self.level == Level::INFO && field.name() == Self::MESSAGE {
                let message = format!("{value:?}");
                if let Some((tag, message)) = message.as_str().split_once(' ') {
                    if tag.len() <= Self::INDENT {
                        let tag = tag.to_title_case();
                        let style = match self.level {
                            Level::DEBUG => self.styles.debug,
                            _ => self.styles.info,
                        };

                        let _ = write!(
                            self.writer,
                            "{:>indent$} ",
                            tag.style(style),
                            indent = Self::INDENT
                        );

                        let _ = self.writer.write_str(message);
                        self.is_empty = false;
                        self.did_cargo_format = true;
                        return;
                    }
                }
            }

            // Otherwise, emit a level tag.
            let _ = match self.level {
                Level::ERROR => write!(
                    self.writer,
                    "{}{} ",
                    "error".style(self.styles.error),
                    ":".style(self.styles.bold)
                ),
                Level::WARN => write!(
                    self.writer,
                    "{}{} ",
                    "warning".style(self.styles.warn),
                    ":".style(self.styles.bold),
                ),
                Level::INFO => write!(
                    self.writer,
                    "{}{} ",
                    "info".style(self.styles.info),
                    ":".style(self.styles.bold)
                ),
                Level::DEBUG => write!(
                    self.writer,
                    "{}{} ",
                    "debug".style(self.styles.debug),
                    ":".style(self.styles.bold)
                ),
                Level::TRACE => write!(
                    self.writer,
                    "{}{} ",
                    "trace".style(self.styles.trace),
                    ":".style(self.styles.bold)
                ),
            };
        } else {
            // If this is *not* the first field of the event, prefix it with a
            // comma for the preceding field, instead of a cargo tag or level tag.
            let _ = self.writer.write_str(", ");
        }

        if field.name() == Self::MESSAGE {
            let _ = write!(self.writer, "{value:?}");
        } else {
            let _ = write!(
                self.writer,
                "{}{} {:?}",
                field.name().style(self.styles.bold),
                ":".style(self.styles.bold),
                value
            );
        }

        self.is_empty = false;
    }
}

// === impl Styles ===

impl Styles {
    fn new(colors: ColorMode) -> Self {
        Self {
            error: colors.if_color(style().red().bold()),
            warn: colors.if_color(style().yellow().bold()),
            info: colors.if_color(style().green().bold()),
            debug: colors.if_color(style().blue().bold()),
            trace: colors.if_color(style().purple().bold()),
            bold: colors.if_color(style().bold()),
            pipes: colors.if_color(style().blue().bold()),
        }
    }
}

impl<T> Prefixed<T> {
    fn prefix(prefix: &'static str) -> impl Fn(T) -> Prefixed<T> {
        move |val| Prefixed { val, prefix }
    }
}

impl<T> fmt::Display for Prefixed<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.prefix, self.val)
    }
}

impl<T> fmt::Debug for Prefixed<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{:?}", self.prefix, self.val)
    }
}

struct DisplayOpt<T>(Option<T>);

impl<T> fmt::Display for DisplayOpt<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref val) = self.0 {
            fmt::Display::fmt(val, f)?;
        }

        Ok(())
    }
}
