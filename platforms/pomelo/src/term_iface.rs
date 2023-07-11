use std::sync::OnceLock;

use futures::channel::mpsc::Sender;
use mnemos_kernel::{
    daemons::sermux::{hello, HelloSettings},
    Kernel,
};
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use tracing::{debug, error, info, trace, warn};
use wasm_bindgen::{closure::Closure, prelude::*};

use crate::sim_drivers::serial::echo;
pub static ECHO_TX: OnceLock<Sender<u8>> = OnceLock::new();
pub static COMMAND_TX: OnceLock<Sender<Command>> = OnceLock::new();

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
            Command::Echo(s) => echo(s),
            Command::Forth(s) => todo!(),
        }
    }
}
