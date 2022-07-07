#[cfg(feature = "trace-fmt")]
pub fn setup_tracing() {
    tracing_subscriber::fmt::init();
}

#[cfg(feature = "trace-modality")]
pub fn setup_tracing() {
    tracing_modality::TracingModality::init().expect("init");
}
