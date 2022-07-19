use core::mem::MaybeUninit;

use mnemos_alloc::containers::{HeapArc, HeapArray, HeapFixedVec};
use tracing::{debug, warn};

use crate::{
    comms::kchannel::{KChannel, KConsumer, KProducer},
    Kernel,
};

use maitake::sync::Mutex;

use embedded_graphics::{
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{Circle, Line, Rectangle, PrimitiveStyle},
    mono_font::{ascii::FONT_6X9, MonoTextStyle},
    text::Text,
};

use embedded_graphics_simulator::{BinaryColorTheme, SimulatorDisplay, Window, OutputSettingsBuilder};

pub const FRAME_BUFFER_SIZE: usize = 153600;
pub const NUM_WINDOWS: usize = 1;

pub struct Msg {
    pub req: Req,
    pub resp: KProducer<Result<Resp, ()>>,
}

pub enum Req {
    OpenWindow,
}

pub enum Resp {
    WindowOpened(WindowHandle),
}

struct Commander {
    cmd: KConsumer<Msg>,
    fmutex: HeapArc<Mutex<FrameChunk>>, 
}

impl Commander {
    async fn run(self) {
        loop {
            let msg = self.cmd.dequeue_async().await.unwrap();
            let Msg { req, resp } = msg;
            match req {
                Req::OpenWindow  => {
                    let res = {
                        let mut fmutex = self.fmutex.lock().await;
                        fmutex.open_window().await
                    };
                    resp.enqueue_async(res.map(|wh| Resp::WindowOpened(wh)))
                        .await
                        .map_err(drop)
                        .unwrap();
                }
            }
        }
    }
}

pub struct FrameChunk {
    display: SimulatorDisplay::<BinaryColor>,
    buffer: HeapArray<MaybeUninit<u8>>,
}

impl FrameChunk {
    async fn open_window(&self) -> Result<WindowHandle, ()> {
        let mut disp = self.display.clone();
        let output_settings = OutputSettingsBuilder::new()
            .theme(BinaryColorTheme::OledBlue)
            .build();
        let mut w = Window::new("MnemOS", &output_settings);
        let text_style = MonoTextStyle::new(&FONT_6X9, BinaryColor::On);
        Text::new("Welcome to MnemOS", Point::new(5, 5), text_style).draw(&mut disp);
        w.show_static(&disp);
        let wh: WindowHandle = WindowHandle { window: w, handle: 1 };
        Ok(wh)
    }
}

pub struct WindowHandle {
    window: Window,
    handle: u16,
}

impl FrameChunk {
    pub async fn new(
        kernel: &'static Kernel,
        width: u32,
        height: u32,
    ) -> KProducer<Msg> {
        let buffer_size = (width * height * 2) as usize;
        let display = SimulatorDisplay::<BinaryColor>::new(Size::new(width, height));
        let buffer = kernel.heap().allocate_array_with(MaybeUninit::<u8>::uninit, buffer_size).await;
        let imutex = kernel
            .heap()
            .allocate_arc(Mutex::new(FrameChunk { 
                display, 
                buffer, 
            }))
            .await;
        let (cmd_prod, cmd_cons) = KChannel::new_async(kernel, NUM_WINDOWS).await.split();
        let commander: Commander = Commander {
            cmd: cmd_cons,
            fmutex: imutex,
        };

        kernel.spawn(async move{
            commander.run().await;
        }).await;

        cmd_prod
    }
}


