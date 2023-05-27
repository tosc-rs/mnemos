use std::time::Duration;

use embedded_graphics::{
    image::{Image, ImageRaw},
    pixelcolor::{Gray8, GrayColor},
    prelude::*,
};
use embedded_graphics_simulator::{
    BinaryColorTheme, OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent, Window,
};
use maitake::sync::{Mutex, MutexGuard};
use mnemos_alloc::containers::{HeapArc, HeapArray};
use mnemos_kernel::{
    comms::{
        kchannel::{KChannel, KConsumer},
        oneshot::Reusable,
    },
    registry::{Envelope, KernelHandle, Message, RegisteredDriver, ReplyTo},
    Kernel,
};
use uuid::Uuid;

use super::delay::Delay;

// Registered driver
pub struct EmbDisplay;

// FrameChunk is recieved after client has sent a request for one
pub struct FrameChunk {
    frame_id: u16,
    bytes: HeapArray<u8>,
    start_x: i32,
    start_y: i32,
    width: u32,
    height: u32,
}

struct FrameInfo {
    frame: u16,
}

#[derive(Debug, Eq, PartialEq)]
pub enum FrameError {
    DuplicateItem,
    RegistryFull,
    NoSuchFrame,
}

struct DisplayInfo {
    kernel: &'static Kernel,
    // TODO: HeapFixedVec has like none of the actual vec methods. For now
    // use HeapArray with optionals to make it easier to find + pop individual items
    frames: HeapArray<Option<FrameInfo>>,
}

// Client interface to EmbDisplay
pub struct EmbDisplayHandle {
    prod: KernelHandle<EmbDisplay>,
    reply: Reusable<Envelope<Result<Response, FrameError>>>,
}

pub enum Request {
    NewFrameChunk {
        frame_id: u16,
        start_x: i32,
        start_y: i32,
        width: u32,
        height: u32,
    },
    Draw(FrameChunk),
    Drop(FrameChunk),
}

pub enum Response {
    FrameChunkAllocated(FrameChunk),
    FrameDrawn,
}

pub enum EmbDisplayError {
    BufferFull,
}

struct CommanderTask {
    kernel: &'static Kernel,
    cmd: KConsumer<Message<EmbDisplay>>,
    fmutex: HeapArc<Mutex<DisplayInfo>>,
}

// impl EmbDisplay
impl RegisteredDriver for EmbDisplay {
    type Request = Request;
    type Response = Response;
    type Error = FrameError;
    const UUID: Uuid = mnemos_kernel::registry::known_uuids::kernel::EMB_DISPLAY;
}

#[derive(Debug, Eq, PartialEq)]
pub enum RegistrationError {
    DisplayAlreadyExists,
}

impl EmbDisplay {
    /// Register the driver instance
    pub async fn register(
        kernel: &'static Kernel,
        max_frames: usize,
        width: usize,
        height: usize,
    ) -> Result<(), ()> {
        let frames = kernel.heap().allocate_array_with(|| None, max_frames).await;

        let imutex = kernel
            .heap()
            .allocate_arc(Mutex::new(DisplayInfo { kernel, frames }))
            .await;
        let (cmd_prod, cmd_cons) = KChannel::new_async(kernel, 1).await.split();
        let commander = CommanderTask {
            kernel,
            cmd: cmd_cons,
            fmutex: imutex,
        };

        kernel
            .spawn(commander.run(width as u32, height as u32))
            .await;

        kernel
            .with_registry(|reg| reg.register_konly::<EmbDisplay>(&cmd_prod))
            .await
            .map_err(|_| RegistrationError::DisplayAlreadyExists)
            .unwrap();

        Ok(())
    }
}

impl FrameChunk {
    /// Create and return a Simulator display object from raw pixel data.
    ///
    /// Pixel data is turned into a raw image, and then drawn onto a SimulatorDisplay object
    /// This is necessary as a e-g Window only accepts SimulatorDisplay object
    /// On a physical display, the raw pixel data can be sent over to the display directly
    /// Using the display's device interface
    pub fn frame_display(&mut self) -> Result<ImageRaw<Gray8>, ()> {
        let raw_image: ImageRaw<Gray8>;
        raw_image = ImageRaw::<Gray8>::new(self.bytes.as_ref(), self.width);
        Ok(raw_image)
    }

    pub fn frame_clear(&mut self) {
        for elem in self.bytes.iter_mut() {
            *elem = 0;
        }
    }
}

impl EmbDisplayHandle {
    pub async fn from_registry(kernel: &'static Kernel) -> Option<Self> {
        let prod = kernel.with_registry(|reg| reg.get::<EmbDisplay>()).await?;

        Some(EmbDisplayHandle {
            prod,
            reply: Reusable::new_async(kernel).await,
        })
    }

    pub async fn drop_framechunk(&mut self, chunk: FrameChunk) -> Result<(), ()> {
        self.prod
            .send(
                Request::Drop(chunk),
                ReplyTo::OneShot(self.reply.sender().map_err(drop)?),
            )
            .await?;
        Ok(())
    }

    pub async fn draw_framechunk(&mut self, chunk: FrameChunk) -> Result<(), ()> {
        self.prod
            .send(
                Request::Draw(chunk),
                ReplyTo::OneShot(self.reply.sender().map_err(drop)?),
            )
            .await?;
        Ok(())
    }

    pub async fn get_framechunk(
        &mut self,
        frame_id: u16,
        start_x: i32,
        start_y: i32,
        width: u32,
        height: u32,
    ) -> Option<FrameChunk> {
        self.prod
            .send(
                Request::NewFrameChunk {
                    frame_id,
                    start_x,
                    start_y,
                    width,
                    height,
                },
                ReplyTo::OneShot(self.reply.sender().ok()?),
            )
            .await
            .ok()?;

        let resp = self.reply.receive().await.ok()?;
        let body = resp.body.ok()?;

        if let Response::FrameChunkAllocated(frame) = body {
            Some(frame)
        } else {
            None
        }
    }
}

impl DisplayInfo {
    // Returns a new frame chunk
    async fn new_frame(
        &mut self,
        frame_id: u16,
        start_x: i32,
        start_y: i32,
        width: u32,
        height: u32,
    ) -> Result<FrameChunk, FrameError> {
        if self
            .frames
            .iter()
            .any(|f| matches!(f, Some(FrameInfo { frame }) if *frame == frame_id))
        {
            return Err(FrameError::DuplicateItem);
        }

        let found = self.frames.iter_mut().find(|f| f.is_none());

        if let Some(slot) = found {
            *slot = Some(FrameInfo { frame: frame_id });

            let size = (width * height) as usize;
            let bytes = self.kernel.heap().allocate_array_with(|| 0, size).await;
            let fc = FrameChunk {
                frame_id,
                bytes,
                start_x,
                start_y,
                width,
                height,
            };

            Ok(fc)
        } else {
            Err(FrameError::RegistryFull)
        }
    }

    fn remove_frame(&mut self, frame_id: u16) -> Result<(), FrameError> {
        let found = self
            .frames
            .iter_mut()
            .find(|f| matches!(f, Some(FrameInfo { frame }) if *frame == frame_id));

        if let Some(slot) = found {
            *slot = None;
            Ok(())
        } else {
            Err(FrameError::NoSuchFrame)
        }
    }
}

impl DrawTarget for FrameChunk {
    type Color = Gray8;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            let (x, y): (u32, u32) = match coord.try_into() {
                Ok(c) => c,
                Err(_) => continue,
            };
            if x >= self.width {
                continue;
            }
            if y >= self.height {
                continue;
            }

            let index: u32 = x + y * self.width;
            // TODO: Implement bound checks and return BufferFull if needed
            self.bytes[index as usize] = color.luma();
        }

        Ok(())
    }
}

impl OriginDimensions for FrameChunk {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}

impl CommanderTask {
    async fn run(self, width: u32, height: u32) {
        let output_settings = OutputSettingsBuilder::new()
            .theme(BinaryColorTheme::OledBlue)
            .build();

        // Create a mutex for the embedded graphics simulator objects.
        //
        // We do this because if we don't call "update" regularly, the window just
        // sort of freezes. We also make the update loop check for "quit" events,
        // because otherwise the gui window just swallows all the control-c events,
        // which means you have to send a sigkill to actually get the simulator to
        // fully stop.
        //
        // The update loop *needs* to drop the egsim items, otherwise they just exist
        // in the mutex until the next time a frame is displayed, which right now is
        // only whenever line characters actually arrive.
        let sdisp = SimulatorDisplay::<Gray8>::new(Size::new(width, height));
        let window = Window::new("mnemOS", &output_settings);
        let mutex = self
            .kernel
            .heap()
            .allocate_arc(Mutex::new(Some((sdisp, window))))
            .await;

        self.kernel
            .spawn({
                let mutex = mutex.clone();
                async move {
                    loop {
                        Delay::new(Duration::from_micros(1_000_000 / 15)).await;
                        let mut guard = mutex.lock().await;
                        let mut done = false;
                        if let Some((sdisp, window)) = (&mut *guard).as_mut() {
                            window.update(&sdisp);
                            if window.events().any(|e| e == SimulatorEvent::Quit) {
                                done = true;
                            }
                        } else {
                            done = true;
                        }
                        if done {
                            let _ = guard.take();
                            break;
                        }
                    }
                }
            })
            .await;

        loop {
            let msg = self.cmd.dequeue_async().await.map_err(drop).unwrap();
            let Message {
                msg: mut req,
                reply,
            } = msg;
            match &mut req.body {
                Request::NewFrameChunk {
                    frame_id,
                    start_x,
                    start_y,
                    width,
                    height,
                } => {
                    let res = {
                        let mut fmutex = self.fmutex.lock().await;
                        fmutex
                            .new_frame(*frame_id, *start_x, *start_y, *width, *height)
                            .await
                    }
                    .map(Response::FrameChunkAllocated);

                    let resp = req.reply_with(res);

                    reply.reply_konly(resp).await.map_err(drop).unwrap();
                }
                Request::Draw(fc) => {
                    let mut fmutex: MutexGuard<DisplayInfo> = self.fmutex.lock().await;
                    match fmutex.remove_frame(fc.frame_id) {
                        Ok(_) => {
                            let (x, y) = (fc.start_x, fc.start_y);
                            let raw_img = fc.frame_display().unwrap();
                            let image = Image::new(&raw_img, Point::new(x, y));

                            let mut guard = mutex.lock().await;
                            if let Some((sdisp, _window)) = (&mut *guard).as_mut() {
                                image.draw(sdisp).unwrap();
                            } else {
                                break;
                            }
                        }
                        Err(e) => {
                            reply
                                .reply_konly(req.reply_with(Err(e)))
                                .await
                                .map_err(drop)
                                .unwrap();
                        }
                    }
                }
                Request::Drop(_) => todo!(),
            }
        }
    }
}
