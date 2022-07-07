const ENV_FILTER: &str = "MELPOMENE_TRACE";

pub fn setup_tracing() {
    use tracing_subscriber::prelude::*;

    let subscriber = tracing_subscriber::registry();

    // if `trace-fmt` is enabled, add a `tracing-subscriber::fmt` layer along
    // with an `EnvFilter`
    #[cfg(feature = "trace-fmt")]
    let subscriber = {
        let filter = tracing_subscriber::EnvFilter::from_env(ENV_FILTER);
        subscriber.with(tracing_subscriber::fmt::layer().with_filter(filter))
    };

    // if `trace-modality` is enabled, add the Modality layer as well.
    #[cfg(feature = "trace-modality")]
    let subscriber = subscriber.with(tracing_modality::ModalityLayer::new());

    subscriber.init();
}
