use std::net::SocketAddr;
#[cfg(feature = "trace-console")]
use std::path::PathBuf;
use tracing_modality::TimelineInfo;
#[cfg(feature = "trace-fmt")]
use tracing_subscriber::filter;

#[derive(Debug, clap::Args)]
#[clap(
    next_help_heading = "TRACING OPTIONS",
    group = clap::ArgGroup::new("tracing-opts")
)]
pub struct TracingOpts {
    /// Trace filter for `tracing-subscriber::fmt`.
    ///
    /// This requires that Melpomene be built with the "trace-fmt" feature flag
    /// enabled.
    #[cfg(feature = "trace-fmt")]
    #[clap(
        long = "trace",
        env = ENV_FILTER,
        parse(try_from_str = parse_envfilter),
        default_value_t = filter::EnvFilter::new("info"),
    )]
    env_filter: filter::EnvFilter,

    #[cfg(feature = "trace-modality")]
    #[clap(flatten)]
    modality: ModalityOpts,

    #[cfg(feature = "trace-console")]
    #[clap(flatten)]
    console: ConsoleOpts,
}

#[cfg(feature = "trace-modality")]
#[derive(Debug, clap::Args)]
#[clap(
    next_help_heading = "TRACING OPTIONS (MODALITY)",
    group = clap::ArgGroup::new("modality-opts")
)]
struct ModalityOpts {
    /// Address of `modalityd or a modality reflector where trace data should be sent.
    ///
    /// This requires that Melpomene be built with the "trace-modality" feature
    /// flag enabled.
    #[clap(long = "modality-addr")]
    modality_addr: Option<SocketAddr>,
}

#[cfg(feature = "trace-console")]
#[derive(Debug, clap::Args)]
#[clap(
    next_help_heading = "TRACING OPTIONS (TOKIO-CONSOLE)",
    group = clap::ArgGroup::new("console-opts")
)]
struct ConsoleOpts {
    /// Address to bind the `tokio-console` listener on.
    ///
    /// This requires that Melpomene be built with the "trace-console" feature
    /// flag enabled.
    #[clap(long = "console-addr", env = "TOKIO_CONSOLE_BIND", default_value_t = default_console_addr())]
    console_addr: SocketAddr,

    /// The interval between publishing updates to connected `tokio-console`
    /// clients.
    ///
    /// This requires that Melpomene be built with the "trace-console" feature
    /// flag enabled.
    #[clap(
        long = "console-publish-interval",
        env = "TOKIO_CONSOLE_PUBLISH_INTERVAL",
        default_value_t = duration_secs(1),
    )]
    publish_interval: humantime::Duration,

    /// How long to retain `tokio-console` data for completed tasks.
    ///
    /// This requires that Melpomene be built with the "trace-console" feature
    /// flag enabled.
    #[clap(
        long = "console-retention",
        env = "TOKIO_CONSOLE_RETENTION",
        default_value_t = duration_secs(3600),
    )]
    retention: humantime::Duration,

    /// A file path to save a `tokio-console` recording to.
    ///
    /// If a value is present, a recording will be output to that file.
    /// Otherwise, no recording will be saved.
    ///
    /// This requires that Melpomene be built with the "trace-console" feature
    /// flag enabled.
    #[clap(long = "console-record-path", env = "TOKIO_CONSOLE_RECORD_PATH", value_hint = clap::ValueHint::FilePath)]
    record_path: Option<PathBuf>,
}

#[cfg(feature = "trace-fmt")]
const ENV_FILTER: &str = "MELPOMENE_TRACE";

#[cfg(feature = "trace-console")]
fn default_console_addr() -> SocketAddr {
    use console_subscriber::Server;
    SocketAddr::from((Server::DEFAULT_IP, Server::DEFAULT_PORT))
}

#[cfg(feature = "trace-console")]
fn duration_secs(secs: u64) -> humantime::Duration {
    humantime::Duration::from(std::time::Duration::from_secs(secs))
}

#[cfg(feature = "trace-fmt")]
fn parse_envfilter(s: &str) -> Result<filter::EnvFilter, filter::ParseError> {
    filter::EnvFilter::builder()
        .with_default_directive(filter::LevelFilter::INFO.into())
        .parse(s)
}

impl TracingOpts {
    pub async fn setup_tracing(mut self) {
        use tracing_subscriber::prelude::*;

        let subscriber = tracing_subscriber::registry();

        // if `trace-fmt` is enabled, add a `tracing-subscriber::fmt` layer along
        // with an `EnvFilter`
        #[cfg(feature = "trace-fmt")]
        let subscriber = {
            use tracing_subscriber::fmt;
            let filter = self.env_filter;

            println!("'trace-fmt' active. 'tracing' information will be printed to the console.");

            let fmt = fmt::layer()
                .with_timer(fmt::time::uptime())
                .with_ansi(atty::is(atty::Stream::Stdout))
                .with_filter(filter);

            subscriber.with(fmt)
        };

        #[cfg(not(feature = "trace-fmt"))]
        println!("'trace-fmt' feature not enabled. 'tracing' information will not be printed to the console.");

        // if `trace-console` is enabled, add a `console-subscriber` layer.
        #[cfg(feature = "trace-console")]
        let subscriber = {
            let mut console = console_subscriber::ConsoleLayer::builder()
                .publish_interval(self.console.publish_interval.into())
                .retention(self.console.retention.into())
                .server_addr(self.console.console_addr);
            eprintln!("Serving tokio-console on {}", self.console.console_addr);

            if let Some(path) = self.console.record_path.take() {
                eprintln!("Saving tokio-console recording to {}", path.display());
                console = console.recording_path(path);
            }

            subscriber.with(console.spawn())
        };

        #[cfg(not(feature = "trace-console"))]
        println!("'trace-console' feature not enabled.");

        // if `trace-modality` is enabled, add the Modality layer as well.
        #[cfg(feature = "trace-modality")]
        let subscriber = {
            let mut options = tracing_modality::Options::new().with_name("melpomene");

            if let Some(modality_addr) = self.modality.modality_addr {
                eprintln!("Sending traces to Modality at {modality_addr}");
                options.set_server_address(modality_addr);
            } else {
                eprintln!("Sending traces to Modality with default configuration");
            }

            let (layer, handle) = tracing_modality::ModalityLayer::init_with_options(options).await.unwrap();

            // AJM: TODO - leaking handle to avoid shutdown
            core::mem::forget(handle);

            subscriber.with(layer)
        };

        #[cfg(not(feature = "trace-modality"))]
        println!("'trace-modality' feature not enabled.");

        subscriber.init();
    }
}

fn modality_identifier() -> TimelineInfo {
    todo!()
}
