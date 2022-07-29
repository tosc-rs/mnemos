use crate::{
    comms::{
        kchannel::{KChannel, KConsumer},
        oneshot::Reusable,
    },
    registry::{
        Envelope, KernelHandle, Message, RegisteredDriver, ReplyTo,
    },
    Kernel,
};
use maitake::sync::Mutex;
use mnemos_alloc::containers::{HeapArc, HeapArray, HeapFixedVec};
// use tracing::{debug, warn};
use uuid::Uuid;
use embedded_graphics::{
    pixelcolor::{Gray8, GrayColor},
    prelude::*,
    image::ImageRaw,
};

const BYTES_PER_PIXEL: u32 = 1;
const DISP_WIDTH: u32 = 319;
const DISP_HEIGHT: u32 = 239;

// Registered driver
pub struct EmbDisplay {
    kernel: &'static Kernel,
}

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
}

struct DisplayInfo{
    kernel: &'static Kernel,
    frames: HeapFixedVec<FrameInfo>,
    frame_size: usize,
}

// Client interface to EmbDisplay
pub struct EmbDisplayHandle {
    prod: KernelHandle<EmbDisplay>,
    reply: Reusable<Envelope<Result<Response, FrameError>>>,
}

pub enum Request {
    NewFrameChunk { frame_id: u16, start_x: i32, start_y: i32, width: u32, height: u32 },
}

pub enum Response {
    FrameChunkAllocated(FrameChunk),
}

pub enum EmbDisplayError {
    BufferFull,
}

struct CommanderTask {
    cmd: KConsumer<Message<EmbDisplay>>,
    fmutex: HeapArc<Mutex<DisplayInfo>>, 
}

// impl EmbDisplay
impl RegisteredDriver for EmbDisplay {
    type Request = Request;
    type Response = Response;
    type Error = FrameError;
    const UUID: Uuid = crate::registry::known_uuids::kernel::EMB_DISPLAY;
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
        let frames = kernel.heap().allocate_fixed_vec(max_frames).await;
        let frame_size = width * height * BYTES_PER_PIXEL as usize;

        let imutex = kernel
            .heap()
            .allocate_arc(Mutex::new(DisplayInfo { 
                kernel,
                frames,
                frame_size,
            }))
            .await;
        let (cmd_prod, cmd_cons) = KChannel::new_async(kernel, 1).await.split();
        let commander = CommanderTask {
            cmd: cmd_cons,
            fmutex: imutex,
        };

        kernel.spawn(commander.run()).await;

        kernel
            .with_registry(|reg| reg.register_konly::<EmbDisplay>(&cmd_prod))
            .await
            .map_err(|_| RegistrationError::DisplayAlreadyExists).unwrap();

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
        for elem in self.bytes.iter_mut() { *elem = 0; }
        
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

    pub async fn get_framechunk(&mut self, frame_id: u16, start_x: i32, start_y: i32, width: u32, height: u32) -> Option<FrameChunk> {
        self.prod
            .send(
                Request::NewFrameChunk { frame_id, start_x, start_y, width, height },
                ReplyTo::OneShot(self.reply.sender().ok()?),
            )
            .await
            .ok()?;

        let resp = self.reply.receive().await.ok()?;
        let body = resp.body.ok()?;

        let Response::FrameChunkAllocated(frame) = body;
        Some(frame)
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
        if self.frames.is_full() {
            return Err(FrameError::RegistryFull);
        }

        if self.frames.iter().any(|f| f.frame == frame_id) {
            return Err(FrameError::DuplicateItem);
        }

        self.frames.push(FrameInfo { frame: frame_id })
            .map_err(|_| FrameError::RegistryFull)?;

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
            // Check if the pixel coordinates are out of bounds (negative or greater than
            // (319,239)). `DrawTarget` implementation are required to discard any out of bounds
            // pixels without returning an error or causing a panic.
            if let Ok((x @ 0..=DISP_WIDTH, y @ 0..=DISP_HEIGHT)) = coord.try_into() {
                // Calculate the index in the framebuffer.
                let index: u32 = x + y * self.width;
                // TODO: Implement bound checks and return BufferFull if needed
                self.bytes[index as usize] = color.luma();
            }
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
    async fn run(self) {
        loop {
            let msg = self.cmd.dequeue_async().await.map_err(drop).unwrap();
            let Message { msg: req, reply } = msg;
            match req.body {
                Request::NewFrameChunk { frame_id, start_x, start_y, width, height } => {
                    let res = {
                        let mut fmutex = self.fmutex.lock().await;
                        fmutex.new_frame(frame_id, start_x, start_y, width, height).await
                    }
                    .map(Response::FrameChunkAllocated);

                    let resp = req.reply_with(res);

                    reply.reply_konly(resp).await.map_err(drop).unwrap();
                }
            }
        }
    }
}