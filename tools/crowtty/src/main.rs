use clap::Parser;
use connection::Connect;
use miette::{Context, IntoDiagnostic};
use tracing::level_filters::LevelFilter;

mod connection;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    connect: Connect,

    /// whether to include verbose logging of bytes in/out.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[clap(flatten)]
    settings: libcrowtty::Settings,

    /// a comma-separated list of `tracing` targets and levels to enable.
    ///
    /// for example, `info,kernel=debug,kernel::comms::bbq=trace` will enable:
    ///
    /// - the `INFO` level globally (regardless of module path),
    /// - the `DEBUG` level for all modules in the `kernel` crate,
    /// - and the `TRACE` level for the `comms::bbq` submodule in `kernel`.
    ///
    /// enabling a more verbose level enables all levels less verbose than that
    /// level. for example, enabling the `INFO` level for a given target will also
    /// enable the `WARN` and `ERROR` levels for that target.
    ///
    /// see <https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/targets/struct.Targets.html#filtering-with-targets>
    /// for more details on this syntax.
    #[arg(
        short,
        long = "trace",
        global = true,
        env = "MNEMOS_TRACE",
        default_value_t = tracing_subscriber::filter::Targets::new().with_default(LevelFilter::INFO),
    )]
    trace_filter: tracing_subscriber::filter::Targets,
}

fn main() -> miette::Result<()> {
    let Args {
        connect,
        settings,
        verbose,
        trace_filter,
    } = Args::parse();
    let conn = connect
        .connect()
        .into_diagnostic()
        .with_context(|| format!("failed to connect to {connect}"))?;
    libcrowtty::Crowtty::new(conn.log_tag().verbose(verbose))
        .settings(settings)
        .trace_filter(trace_filter)
        .run(conn)
}
