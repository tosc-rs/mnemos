use core::time::Duration;
use d1_pac::{gpio, Interrupt, GPIO};
use kernel::{
    comms::{
        kchannel::{KChannel, KConsumer},
        oneshot::Reusable,
    },
    isr::Isr,
    maitake::sync::WaitQueue,
    registry::{self, uuid, Envelope, KernelHandle, RegisteredDriver, Uuid},
    trace, Kernel,
};

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////
pub struct GpioService;

impl RegisteredDriver for GpioService {
    type Request = Request;
    type Response = Response;
    type Error = Error;

    const UUID: Uuid = uuid!("155b4ea1-cb42-495c-8db1-7fa13e7ed976");
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////
pub enum Request {
    RegisterIrq {
        pin: Pin,
        mode: InterruptMode,
    },
    RegisterCustom {
        pin: Pin,
        name: &'static str,
        register: fn(&gpio::RegisterBlock),
    },
    PinState(Pin),
}
pub enum Response {
    RegisterIrq(&'static WaitQueue),
    RegisterCustom,
    PinState(Pin, PinState),
    // actually do IO...
}

pub enum Error {
    PinInUse(Pin, PinState),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Pin {
    /// Pin group PB
    B(PinB),
    /// Pin group PC
    C(PinC),
    /// Pin group PD
    D(PinD),
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PinB {
    B0 = 0,
    B1 = 1,
    B2 = 2,
    B3 = 3,
    B4 = 4,
    B5 = 5,
    B6 = 6,
    B7 = 7,
    B8 = 8,
    B9 = 9,
    B10 = 10,
    B11 = 11,
    B12 = 12,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PinC {
    C0 = 0,
    C1 = 1,
    C2 = 2,
    C3 = 3,
    C4 = 4,
    C5 = 5,
    C6 = 6,
    C7 = 7,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PinD {
    D0 = 0,
    D1 = 1,
    D2 = 2,
    D3 = 3,
    D4 = 4,
    D5 = 5,
    D6 = 6,
    D7 = 7,
    D8 = 8,
    D9 = 9,
    D10 = 10,
    D11 = 11,
    D12 = 12,
    D13 = 13,
    D14 = 14,
    D15 = 15,
    D16 = 16,
    D17 = 17,
    D18 = 18,
    D19 = 19,
    D20 = 20,
    D21 = 21,
    D22 = 22,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum InterruptMode {
    PositiveEdge,
    NegativeEdge,
    HighLevel,
    LowLevel,
    DoubleEdge,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PinState {
    Unregistered,
    Interrupt(InterruptMode),
    Input,
    Output,
    Other(&'static str),
}

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

pub struct GpioClient {
    handle: KernelHandle<GpioService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}

impl GpioClient {
    /// Obtain a `GpioClient`
    ///
    /// If the [`GpioServer`] hasn't been registered yet, we will retry until it
    /// has been registered.
    pub async fn from_registry(kernel: &'static Kernel) -> Self {
        loop {
            match GpioClient::from_registry_no_retry(kernel).await {
                Some(port) => return port,
                None => {
                    // I2C probably isn't registered yet. Try again in a bit
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Obtain an `GpioClient`
    ///
    /// Does NOT attempt to get an [`GpioService`] handle more than once.
    ///
    /// Prefer [`GpioClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    pub async fn from_registry_no_retry(kernel: &'static Kernel) -> Option<Self> {
        let handle = kernel.with_registry(|reg| reg.get::<GpioService>()).await?;

        Some(GpioClient {
            handle,
            reply: Reusable::new_async().await,
        })
    }
}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

pub struct GpioServer {
    pb: [PinState; PB_COUNT],
    pc: [PinState; PC_COUNT],
    pd: [PinState; PD_COUNT],
    pe: [PinState; PE_COUNT],
    pf: [PinState; PF_COUNT],
    pg: [PinState; PG_COUNT],
    rx: KConsumer<registry::Message<GpioService>>,
}

impl GpioServer {
    pub async fn register(
        kernel: &'static Kernel,
        gpio: GPIO,
        queued: usize,
    ) -> Result<(), registry::RegistrationError> {
        let (tx, rx) = KChannel::new_async(queued).await.split();
        let this = Self {
            pb: [PinState::Unregistered; PB_COUNT],
            pc: [PinState::Unregistered; PC_COUNT],
            pd: [PinState::Unregistered; PD_COUNT],
            pe: [PinState::Unregistered; PE_COUNT],
            pf: [PinState::Unregistered; PF_COUNT],
            pg: [PinState::Unregistered; PG_COUNT],
            rx,
        };
        kernel.spawn(this.run(gpio)).await;
        kernel
            .with_registry(move |reg| reg.register_konly::<GpioService>(&tx))
            .await
    }

    #[trace::instrument(level = trace::Level::INFO, name = "GpioServer", skip(self, gpio))]
    async fn run(mut self, gpio: GPIO) {
        while let Ok(registry::Message { msg, reply }) = self.rx.dequeue_async().await {
            let rsp = match msg.body {
                Request::RegisterIrq { pin, mode } => {
                    tracing::debug!(?pin, ?mode, "registering GPIO interrupt...");
                    let (state, irq) = self.pin(pin);
                    match *state {
                        PinState::Unregistered => {
                            *state = PinState::Interrupt(mode);
                            // TODO(eliza): configure the interrupt mode!

                            tracing::info!(?pin, ?mode, "GPIO interrupt registered!");
                            Ok(Response::RegisterIrq(irq))
                        }
                        PinState::Interrupt(cur_mode) if cur_mode == mode => {
                            tracing::info!(?pin, ?mode, "GPIO interrupt subscribed.");
                            Ok(Response::RegisterIrq(irq))
                        }
                        state => {
                            tracing::warn!(
                                ?pin,
                                ?state,
                                ?mode,
                                "can't register GPIO interrupt, pin already in use!"
                            );
                            // TODO(eliza): add a way for a requester to wait
                            // for a pin to become available?
                            Err(Error::PinInUse(pin, state))
                        }
                    }
                }
                Request::RegisterCustom {
                    pin,
                    name,
                    register,
                } => {
                    let (state, _) = self.pin(pin);
                    match *state {
                        PinState::Unregistered => {
                            register(&gpio);
                            *state = PinState::Other(name);
                            Ok(Response::RegisterCustom)
                        }
                        PinState::Other(cur_mode) if cur_mode == name => {
                            Ok(Response::RegisterCustom)
                        }
                        state => Err(Error::PinInUse(pin, state)),
                    }
                }
                Request::PinState(pin) => Ok(Response::PinState(pin, *self.pin(pin).0)),
            };
            if let Err(error) = reply.reply_konly(msg.reply_with(rsp)).await {
                tracing::warn!(?error, "requester cancelled request!")
                // TODO(eliza): we should probably undo any pin state changes here...
            }
        }
    }

    fn pin(&mut self, pin: Pin) -> (&mut PinState, &'static WaitQueue) {
        match pin {
            Pin::B(pin) => {
                let idx = pin as usize;
                (&mut self.pb[idx], &PB_IRQS[idx])
            }
            Pin::C(pin) => {
                let idx = pin as usize;
                (&mut self.pc[idx], &PC_IRQS[idx])
            }
            Pin::D(pin) => {
                let idx = pin as usize;
                (&mut self.pd[idx], &PD_IRQS[idx])
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Helpers
////////////////////////////////////////////////////////////////////////////////

pub(crate) const INTERRUPTS: [(Interrupt, fn()); 5] = [
    (Interrupt::GPIOB_NS, handle_pb_irq),
    (Interrupt::GPIOC_NS, handle_pc_irq),
    (Interrupt::GPIOD_NS, handle_pd_irq),
    (Interrupt::GPIOE_NS, handle_pe_irq),
    (Interrupt::GPIOF_NS, handle_pf_irq),
    // is there not an interrupt for GPIO pin group G? the manual says
    // those pins can also have interrupts, but there's no `Interrupt` variant
    // in `d1_pac`...
    // (Interrupt::GPIOG_NS, handle_pg_irq)
];

#[allow(clippy::declare_interior_mutable_const)]
const NEW_WAITQ: WaitQueue = WaitQueue::new();
static PB_IRQS: [WaitQueue; PB_COUNT] = [NEW_WAITQ; PB_COUNT];
static PC_IRQS: [WaitQueue; PC_COUNT] = [NEW_WAITQ; PC_COUNT];
static PD_IRQS: [WaitQueue; PD_COUNT] = [NEW_WAITQ; PD_COUNT];
static PE_IRQS: [WaitQueue; PE_COUNT] = [NEW_WAITQ; PE_COUNT];
static PF_IRQS: [WaitQueue; PF_COUNT] = [NEW_WAITQ; PF_COUNT];
// static PG_IRQS: [WaitQueue; 19] = [NEW_WAITQ; 19];

const PB_COUNT: usize = 13;
const PC_COUNT: usize = 8;
const PD_COUNT: usize = 23;
const PE_COUNT: usize = 18;
const PF_COUNT: usize = 7;
const PG_COUNT: usize = 19;

macro_rules! isrs {
    ($($vis:vis fn $name:ident($register:ident, $waiters:ident);)+) => {
        $(
            $vis fn $name() {
                debug_assert!(Isr::is_in_isr());
                let gpio = unsafe { &*GPIO::ptr() };
                gpio.$register.modify(|r, w| {
                    tracing::trace!($register = ?format_args!("{:#b}", r.bits()), "GPIO interrupt");
                    for (bit, waiters) in $waiters.iter().enumerate() {
                        let bit = unsafe {
                            // Safety: the length of each IRQ waker array is the
                            // same length as the register.
                            r.eint_status(bit as u8)
                        };
                        if bit.is_pending() {
                            waiters.wake_all();
                        }
                    }
                    // write back any set bits to clear those IRQs.
                    w
                })
            }
        )+

    }
}

isrs! {
    pub(crate) fn handle_pb_irq(pb_eint_status, PB_IRQS);
    pub(crate) fn handle_pc_irq(pc_eint_status, PC_IRQS);
    pub(crate) fn handle_pd_irq(pd_eint_status, PD_IRQS);
    pub(crate) fn handle_pe_irq(pe_eint_status, PE_IRQS);
    pub(crate) fn handle_pf_irq(pf_eint_status, PF_IRQS);
    // pub(crate) fn handle_pg_irq(pg_eint_status, PG_IRQS);
}

// struct IrqLock<T> {
//     data: UnsafeCell<T>,
// }

// struct IrqGuard<'a, T> {
//     data: &'a mut T,
// }

// unsafe impl<T: Sync> Sync for IrqLock<T> {}

// impl<T> IrqLock<T> {
//     unsafe fn get_irq(&self) -> *mut T {
//         debug_assert!(Isr::is_in_isr());
//         self.data.get()
//     }

//     fn lock(&self) -> IrqGuard<'_, T> {
//         unsafe {
//             riscv::interrupt::disable();
//             IrqGuard {
//                 data: &mut *self.data.get(),
//             }
//         }
//     }
// }

// impl<T> Deref for IrqGuard<'_, T> {
//     type Target = T;
//     fn deref(&self) -> &Self::Target {
//         self.data
//     }
// }

// impl<T> DerefMut for IrqGuard<'_, T> {
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         self.data
//     }
// }

// impl<T> Drop for IrqGuard<'_, T> {
//     fn drop(&mut self) {
//         unsafe {
//             riscv::interrupt::enable();
//         }
//     }
// }
