//! Kernel Communications Interfaces

pub mod bbq;
pub mod kchannel;
pub mod oneshot;
pub use calliope::tricky_pipe::{bidi, mpsc};
