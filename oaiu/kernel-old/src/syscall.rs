use core::cell::UnsafeCell;
use abi::{bbqueue_ipc::{BBBuffer, framed::{FrameProducer, FrameConsumer, FrameGrantR}}, syscall::request::SysCallRequest};
use abi::SysCallRings;
use core::sync::atomic::AtomicPtr;

pub const RING_SIZE: usize = 4096;
pub static BBQ_U2K_BUF: UnsafeBuffer<RING_SIZE> = UnsafeBuffer::new();
pub static BBQ_K2U_BUF: UnsafeBuffer<RING_SIZE> = UnsafeBuffer::new();
pub static BBQ_U2K: BBBuffer = BBBuffer::new();
pub static BBQ_K2U: BBBuffer = BBBuffer::new();

unsafe impl<const N: usize> Sync for UnsafeBuffer<N> { }

pub struct UnsafeBuffer<const N: usize> {
    pub buf: UnsafeCell<[u8; N]>,
}

impl<const N: usize> UnsafeBuffer<N> {
    pub const fn new() -> Self {
        Self {
            buf: UnsafeCell::new([0u8; N])
        }
    }
}

pub struct KernelRings {
    pub user_to_kernel: FrameConsumer<'static>,
    pub kernel_to_user: FrameProducer<'static>,
    cur_grant: Option<FrameGrantR<'static>>,
}

impl KernelRings {
    pub unsafe fn initialize() -> Self {
        BBQ_U2K.initialize(BBQ_U2K_BUF.buf.get().cast::<u8>(), RING_SIZE);
        BBQ_K2U.initialize(BBQ_K2U_BUF.buf.get().cast::<u8>(), RING_SIZE);
        Self {
            user_to_kernel: BBBuffer::take_framed_consumer((&BBQ_U2K) as *const BBBuffer as *mut BBBuffer),
            kernel_to_user: BBBuffer::take_framed_producer((&BBQ_K2U) as *const BBBuffer as *mut BBBuffer),
            cur_grant: None,
        }
    }

    pub unsafe fn user_rings(&self) -> SysCallRings {
        SysCallRings {
            user_to_kernel: AtomicPtr::new((&BBQ_U2K) as *const BBBuffer as *mut BBBuffer),
            kernel_to_user: AtomicPtr::new((&BBQ_K2U) as *const BBBuffer as *mut BBBuffer),
        }
    }

    pub fn read(&mut self) -> Option<SysCallRequest> {
        // todo: report bad messages?
        self.cur_grant = self.user_to_kernel.read();
        todo!()
    }
}
