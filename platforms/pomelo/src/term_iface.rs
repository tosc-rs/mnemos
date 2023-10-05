use std::sync::OnceLock;

use futures::channel::mpsc::Sender;
use mnemos_kernel::{
    daemons::sermux::{hello, HelloSettings},
    Kernel,
};
use serde::{Deserialize, Serialize};
use sermux_proto::WellKnown;
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};
use wasm_bindgen::{closure::Closure, prelude::*};

use crate::sim_drivers::{self, serial::echo};
pub static SERMUX_TX: OnceLock<Sender<u8>> = OnceLock::new();

#[wasm_bindgen(module = "/src/js/glue.js")]
extern "C" {
    pub fn to_term(data: String);
    pub fn init_term(command_cb: &Closure<dyn Fn(JsValue)>);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    StartHello,
    Echo(String),
    Forth(String),
}

impl Command {
    pub fn dispatch(self, kernel: &'static Kernel) {
        match self {
            Command::StartHello => {
                // Spawn a hello port
                let hello_settings = HelloSettings::default();
                kernel.initialize(hello(kernel, hello_settings)).unwrap();
            }
            Command::Echo(s) => echo(s + "\n"),
            Command::Forth(s) => sim_drivers::io::send(WellKnown::ForthShell0.into(), s.as_bytes()),
        }
    }
}
