use core::{fmt, ops, time::Duration};
use d1_pac::{gpio, Interrupt, GPIO};
use kernel::{
    comms::{
        kchannel::{KChannel, KConsumer},
        oneshot::Reusable,
    },
    isr::Isr,
    maitake::sync::WaitQueue,
    mnemos_alloc::containers::{Arc, FixedVec},
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
    ClaimIrq {
        pin: Pin,
        mode: InterruptMode,
    },
    ClaimCustom {
        pins: FixedVec<(Pin, &'static str)>,
        register: fn(&gpio::RegisterBlock),
    },
    ClaimOutput(Pin),
    PinState(Pin),
}

#[derive(Debug)]
pub enum Response {
    ClaimIrq(&'static WaitQueue),
    ClaimCustom,
    PinState(Pin, PinState),
    ClaimOutput(OutputPin),
}

#[derive(Debug)]
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
    /// Pin group PE
    E(PinE),
    /// Pin group PF
    F(PinF),
    /// Pin group PG
    G(PinG),
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinB {
    B0 = 0,
    B1,
    B2,
    B3,
    B4,
    B5,
    B6,
    B7,
    B8,
    B9,
    B10,
    B11,
    B12,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinC {
    C0 = 0,
    C1,
    C2,
    C3,
    C4,
    C5,
    C6,
    C7,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinD {
    D0 = 0,
    D1,
    D2,
    D3,
    D4,
    D5,
    D6,
    D7,
    D8,
    D9,
    D10,
    D11,
    D12,
    D13,
    D14,
    D15,
    D16,
    D17,
    D18,
    D19,
    D20,
    D21,
    D22,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinE {
    E0 = 0,
    E1,
    E2,
    E3,
    E4,
    E5,
    E6,
    E7,
    E8,
    E9,
    E10,
    E11,
    E12,
    E13,
    E14,
    E15,
    E16,
    E17,
    E18,
    E19,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinF {
    F0 = 0,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub enum PinG {
    G0 = 0,
    G1,
    G2,
    G3,
    G4,
    G5,
    G6,
    G7,
    G8,
    G9,
    G10,
    G11,
    G12,
    G13,
    G14,
    G15,
    G16,
    G17,
    G18,
    G19,
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

pub struct OutputPin {
    pin: Pin,
    _live: Arc<()>,
}

macro_rules! impl_from_pins {
    ($($P:ty => $pin:ident),+ $(,)?) => {
        $(
            impl From<$P> for Pin {
                fn from(p: $P) -> Self {
                    Self::$pin(p)
                }
            }
        )+
    }
}

impl_from_pins! {
    PinB => B,
    PinC => C,
    PinD => D,
    PinE => E,
    PinF => F,
    PinG => G,
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

    /// Configure a pin with a custom configuration.
    ///
    /// # Warnings
    ///
    /// The provided function must only modify the pin configuration for `pin`.
    /// There is currently no way to enforce this.
    pub async fn claim_custom(
        &mut self,
        pins: impl Into<FixedVec<(Pin, &'static str)>>,
        register: fn(&gpio::RegisterBlock),
    ) -> Result<(), Error> {
        let pins = pins.into();
        let resp = self
            .handle
            .request_oneshot(Request::ClaimCustom { pins, register }, &self.reply)
            .await
            .unwrap();
        match resp.body {
            Ok(Response::ClaimCustom) => Ok(()),
            Ok(resp) => {
                unreachable!("expected the GpioService to respond with ClaimCustom, got {resp:?}")
            }
            Err(e) => Err(e),
        }
    }

    pub async fn claim_output(&mut self, pin: impl Into<Pin>) -> Result<OutputPin, Error> {
        let resp = self
            .handle
            .request_oneshot(Request::ClaimOutput(pin.into()), &self.reply)
            .await
            .unwrap();
        match resp.body {
            Ok(Response::ClaimOutput(pin)) => Ok(pin),
            Ok(resp) => {
                unreachable!("expected the GpioService to respond with ClaimCustom, got {resp:?}")
            }
            Err(e) => Err(e),
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

pub struct GpioServer {
    pb: [InternalPinState; PB_COUNT],
    pc: [InternalPinState; PC_COUNT],
    pd: [InternalPinState; PD_COUNT],
    pe: [InternalPinState; PE_COUNT],
    pf: [InternalPinState; PF_COUNT],
    pg: [InternalPinState; PG_COUNT],
    rx: KConsumer<registry::Message<GpioService>>,
}

#[derive(Clone)]
enum InternalPinState {
    Unregistered,
    Interrupt(InterruptMode),
    Input(Arc<()>),
    Output(Arc<()>),
    Other(&'static str),
}

impl GpioServer {
    pub async fn register(
        kernel: &'static Kernel,
        gpio: GPIO,
        queued: usize,
    ) -> Result<(), registry::RegistrationError> {
        let (tx, rx) = KChannel::new_async(queued).await.split();
        const UNREGISTERED: InternalPinState = InternalPinState::Unregistered;
        let this = Self {
            pb: [UNREGISTERED; PB_COUNT],
            pc: [UNREGISTERED; PC_COUNT],
            pd: [UNREGISTERED; PD_COUNT],
            pe: [UNREGISTERED; PE_COUNT],
            pf: [UNREGISTERED; PF_COUNT],
            pg: [UNREGISTERED; PG_COUNT],
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
            let rsp = self.handle_msg(&gpio, &msg.body).await;
            if let Err(error) = reply.reply_konly(msg.reply_with(rsp)).await {
                tracing::warn!(?error, "requester cancelled request!")
                // TODO(eliza): we should probably undo any pin state changes here...
            }
        }
    }

    async fn handle_msg(&mut self, gpio: &GPIO, msg: &Request) -> Result<Response, Error> {
        macro_rules! set_mode_all {
            ($pin:ident, $mode:ident) => {
                // XXX(eliza): this fucking sucks. maybe we should just do some
                // math and do a "raw" write to the register, instead of trying
                // to use the giant mess of `d1_pac` types...
                set_mode! {
                    match $pin {
                        Pin::C(PinC::C0) => pc_cfg0, pc0_select, $mode,
                        Pin::C(PinC::C1) => pc_cfg0, pc1_select, $mode,
                        Pin::C(PinC::C2) => pc_cfg0, pc2_select, $mode,
                        Pin::C(PinC::C3) => pc_cfg0, pc3_select, $mode,
                        Pin::C(PinC::C4) => pc_cfg0, pc4_select, $mode,
                        Pin::C(PinC::C5) => pc_cfg0, pc5_select, $mode,
                        Pin::C(PinC::C6) => pc_cfg0, pc6_select, $mode,
                        Pin::C(PinC::C7) => pc_cfg0, pc7_select, $mode,
                        Pin::D(PinD::D18) => pd_cfg2, pd18_select, $mode,
                        // TODO(eliza): add everything else :(
                    }
                }
            };
        }

        macro_rules! set_mode {
            (match $input:ident { $(Pin::$grp:ident($grpvar:ident::$pin:ident) => $reg:ident, $field:ident, $mode:ident),+ $(,)? }) => {
                match $input {
                    $(
                        Pin::$grp($grpvar::$pin) => gpio.$reg.modify(|_r, w| w.$field().$mode()),
                    )+
                    pin => todo!("eliza: {pin:?}"),
                }
            }
        }

        match msg {
            &Request::ClaimIrq { pin, mode } => {
                tracing::debug!(?pin, ?mode, "registering GPIO interrupt...");
                let (state, irq) = self.pin(pin);
                match state {
                    InternalPinState::Unregistered => {
                        *state = InternalPinState::Interrupt(mode);
                        // TODO(eliza): configure the interrupt mode!

                        tracing::info!(?pin, ?mode, "GPIO interrupt registered!");
                        Ok(Response::ClaimIrq(irq))
                    }
                    InternalPinState::Interrupt(cur_mode) if *cur_mode == mode => {
                        tracing::info!(?pin, ?mode, "GPIO interrupt subscribed.");
                        Ok(Response::ClaimIrq(irq))
                    }
                    InternalPinState::Output(refs) | InternalPinState::Input(refs)
                        if alloc::sync::Arc::strong_count(refs) == 1 =>
                    {
                        *state = InternalPinState::Interrupt(mode);
                        tracing::info!(?pin, ?mode, "GPIO interrupt registered!");
                        Ok(Response::ClaimIrq(irq))
                    }
                    state => {
                        let state = state.external();
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
            &Request::ClaimOutput(pin) => {
                tracing::debug!(?pin, "claiming GPIO output pin...");
                let (state, _) = self.pin(pin);
                match state {
                    InternalPinState::Unregistered => {}
                    InternalPinState::Output(refs) | InternalPinState::Input(refs)
                        if alloc::sync::Arc::strong_count(refs) == 1 => {}
                    state => {
                        let state = state.external();
                        tracing::warn!(
                            ?pin,
                            ?state,
                            "can't claim GPIO output pin, pin already in use!"
                        );
                        // TODO(eliza): add a way for a requester to wait
                        // for a pin to become available?
                        return Err(Error::PinInUse(pin, state));
                    }
                }
                set_mode_all!(pin, output);
                let refs = Arc::new(()).await;
                *state = InternalPinState::Output(refs.clone());

                tracing::info!(?pin, "claimed GPIO output pin");
                Ok(Response::ClaimOutput(OutputPin { pin, _live: refs }))
            }
            Request::ClaimCustom { pins, register } => {
                // first, try to claim all the requested pins --- don't do
                // the register block manipulation if we can't claim any of
                // the requested pins.
                for &(pin, name) in pins.as_slice() {
                    match self.pin(pin) {
                        (InternalPinState::Unregistered, _) => {}
                        (state, _) => {
                            let state = state.external();
                            tracing::warn!(
                                ?pin,
                                ?state,
                                "can't claim pin for {name}, already in use!"
                            );
                            return Err(Error::PinInUse(pin, state));
                        }
                    }
                }
                // now that we've confirmed that all pins are claimable,
                // actually perform the registration and set the pin states.
                register(gpio);
                for &(pin, name) in pins.as_slice() {
                    let (state, _) = self.pin(pin);
                    match state {
                        InternalPinState::Unregistered => {
                            tracing::info!(?pin, state = %name, "claimed pin");
                            *state = InternalPinState::Other(name);
                        }
                        InternalPinState::Other(_) => {}
                        state => {
                            let state = state.external();
                            unreachable!("we just checked the pin's state, and it was claimable! but now it's {state:?}?")
                        }
                    }
                }

                Ok(Response::ClaimCustom)
            }
            &Request::PinState(pin) => Ok(Response::PinState(pin, self.pin(pin).0.external())),
        }
    }

    fn pin(&mut self, pin: Pin) -> (&mut InternalPinState, &'static WaitQueue) {
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
            Pin::E(pin) => {
                let idx = pin as usize;
                (&mut self.pe[idx], &PE_IRQS[idx])
            }
            Pin::F(pin) => {
                let idx = pin as usize;
                (&mut self.pf[idx], &PF_IRQS[idx])
            }
            Pin::G(pin) => {
                let idx = pin as usize;
                (&mut self.pg[idx], &PG_IRQS[idx])
            }
        }
    }
}
impl OutputPin {
    pub fn set(&mut self, high: bool) {
        let gpio = unsafe { &*GPIO::PTR };
        match self.pin {
            Pin::B(pin) => gpio.pb_dat.modify(|r, w| {
                let bits = r.pb_dat().bits();
                w.pb_dat().variant(toggle_bit(bits, pin as u16, high))
            }),
            Pin::C(pin) => gpio.pc_dat.modify(|r, w| {
                let bits = r.pc_dat().bits();
                w.pc_dat().variant(toggle_bit(bits, pin as u8, high))
            }),
            Pin::D(pin) => gpio.pd_dat.modify(|r, w| {
                let bits = r.pd_dat().bits();
                w.pd_dat().variant(toggle_bit(bits, pin as u32, high))
            }),
            Pin::E(pin) => gpio.pe_dat.modify(|r, w| {
                let bits = r.pe_dat().bits();
                w.pe_dat().variant(toggle_bit(bits, pin as u32, high))
            }),
            Pin::F(pin) => gpio.pf_dat.modify(|r, w| {
                let bits = r.pf_dat().bits();
                w.pf_dat().variant(toggle_bit(bits, pin as u8, high))
            }),
            Pin::G(pin) => gpio.pg_dat.modify(|r, w| {
                let bits = r.pg_dat().bits();
                w.pg_dat().variant(toggle_bit(bits, pin as u32, high))
            }),
        }
    }
}

impl fmt::Debug for OutputPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OutputPin").field(&self.pin).finish()
    }
}

fn toggle_bit<U>(bits: U, bit: U, set: bool) -> U
where
    U: ops::BitOr<Output = U>
        + ops::BitAnd<Output = U>
        + ops::Not<Output = U>
        + ops::Shl<Output = U>
        + From<u8>,
{
    if set {
        bits | (U::from(1) << bit)
    } else {
        bits & !(U::from(1) << bit)
    }
}

impl InternalPinState {
    fn external(&self) -> PinState {
        match self {
            InternalPinState::Input(_) => PinState::Input,
            InternalPinState::Output(_) => PinState::Output,
            &InternalPinState::Interrupt(mode) => PinState::Interrupt(mode),
            InternalPinState::Other(name) => PinState::Other(name),
            InternalPinState::Unregistered => PinState::Unregistered,
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
static PG_IRQS: [WaitQueue; PG_COUNT] = [NEW_WAITQ; PG_COUNT];

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
