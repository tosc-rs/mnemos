use owo_colors::{OwoColorize, Stream};
use std::{
    io::{self, BufRead},
    sync::mpsc,
    thread,
};

use crate::{LogTag, WorkerHandle};

pub(crate) struct KeyboardWorker {
    tx: mpsc::Sender<Vec<u8>>,
    _rx: mpsc::Receiver<Vec<u8>>,
    tag: LogTag,
}

impl KeyboardWorker {
    pub fn spawn(tag: LogTag) -> WorkerHandle {
        let (inp_send, inp_recv) = mpsc::channel();
        let (out_send, out_recv) = mpsc::channel::<Vec<u8>>();
        let worker = Self {
            tx: inp_send,
            _rx: out_recv,
            tag,
        };
        let thread_hdl = thread::spawn(|| worker.run());
        WorkerHandle {
            out: out_send,
            inp: inp_recv,
            _thread_hdl: thread_hdl,
        }
    }

    fn run(self) {
        let mut stdin = io::stdin().lock();
        let keyb = "KEYB".if_supports_color(Stream::Stdout, |x| x.bright_yellow());
        loop {
            let mut buf = String::new();
            match stdin.read_line(&mut buf) {
                Ok(n) => {
                    self.tag.if_verbose(format_args!("{keyb} {n}B <- {buf:?}"));
                    self.tx.send(buf.into_bytes()).unwrap();
                }
                Err(error) => {
                    println!(
                        "{} {keyb} {} {error}",
                        self.tag,
                        "ERR!".if_supports_color(Stream::Stdout, |x| x.red())
                    );
                }
            }
        }
    }
}
