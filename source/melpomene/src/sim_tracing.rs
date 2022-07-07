use tracing_subscriber;

pub fn setup_tracing() {
    tracing_subscriber::fmt::init();
}
