const ENV_FILTER: &str = "MELPOMENE_TRACE";

pub fn setup_tracing() {
    use tracing_subscriber::prelude::*;

    let subscriber = tracing_subscriber::registry();

    // if `trace-fmt` is enabled, add a `tracing-subscriber::fmt` layer along
    // with an `EnvFilter`
    #[cfg(feature = "trace-fmt")]
    let subscriber = {
        use tracing_subscriber::{filter, fmt};

        let filter = filter::EnvFilter::builder()
            .with_default_directive(filter::LevelFilter::INFO.into())
            .with_env_var(ENV_FILTER)
            .from_env_lossy();

        let fmt = fmt::layer()
            .with_timer(fmt::time::uptime())
            .with_ansi(atty::is(atty::Stream::Stdout))
            .with_filter(filter);
        subscriber.with(fmt)
    };

    // if `trace-console` is enabled, add a `console-subscriber` layer.
    #[cfg(feature = "trace-console")]
    let subscriber = subscriber.with(console_subscriber::spawn());

    // if `trace-modality` is enabled, add the Modality layer as well.
    #[cfg(feature = "trace-modality")]
    let subscriber = {
        let options = tracing_modality::Options::new().with_name("melpomene");
        let layer = tracing_modality::ModalityLayer::init_with_options(options).unwrap();
        subscriber.with(layer)
    };

    subscriber.init();
}
