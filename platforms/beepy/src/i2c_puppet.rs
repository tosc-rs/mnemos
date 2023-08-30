//! Driver for the [`i2c_puppet`](https://github.com/solderparty/i2c_puppet)
//! keyboard firmware.
use bbq10kbd::{AsyncBbq10Kbd, CapsLockState, FifoCount, NumLockState};
pub use bbq10kbd::{KeyRaw, KeyStatus, Version};
use core::{fmt, time::Duration};
use futures::{select_biased, FutureExt, TryFutureExt};
use kernel::{
    comms::{
        kchannel::{KChannel, KConsumer, KProducer},
        oneshot::Reusable,
    },
    embedded_hal_async::i2c::{self, I2c},
    maitake::sync::WaitCell,
    mnemos_alloc::containers::FixedVec,
    registry::{self, Envelope, KernelHandle, RegisteredDriver},
    retry::{AlwaysRetry, ExpBackoff, Retry, WithMaxRetries},
    services::{
        i2c::{I2cClient, I2cError},
        keyboard::{
            key_event::{self, KeyEvent, Modifiers},
            mux::KeyboardMuxClient,
        },
    },
    tracing::{self, instrument, Instrument, Level},
    Kernel,
};
use uuid::{uuid, Uuid};

////////////////////////////////////////////////////////////////////////////////
// Service Definition
////////////////////////////////////////////////////////////////////////////////
pub struct I2cPuppetService;

impl RegisteredDriver for I2cPuppetService {
    type Request = Request;
    type Response = Response;
    type Error = Error;
    type Hello = ();
    type ConnectError = core::convert::Infallible;

    const UUID: Uuid = uuid!("f5f26c40-6079-4233-8894-39887b878dec");
}

////////////////////////////////////////////////////////////////////////////////
// Message and Error Types
////////////////////////////////////////////////////////////////////////////////

#[derive(Debug)]
pub enum Request {
    GetVersion,
    SetColor(RgbColor),
    SetBacklight(u8),
    GetBacklight,
    ToggleLed(bool),
    GetLedStatus,
    SubscribeToRawKeys,
}

pub enum Response {
    GetVersion(Version),
    SetColor(RgbColor),
    Backlight(u8),
    ToggleLed(bool),
    GetLedStatus(LedState),
    SubscribeToKeys(RawKeySubscription),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LedState {
    pub color: RgbColor,
    pub on: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct HsvColor {
    pub h: u8,
    pub s: u8,
    pub v: u8,
}

#[derive(Debug)]
pub enum Error {
    I2c(I2cError),
    AtMaxSubscriptions,
    SendRequest(registry::OneshotRequestError),
}

pub struct RawKeySubscription(KConsumer<(KeyStatus, KeyRaw)>);

////////////////////////////////////////////////////////////////////////////////
// Client Definition
////////////////////////////////////////////////////////////////////////////////

pub struct I2cPuppetClient {
    handle: KernelHandle<I2cPuppetService>,
    reply: Reusable<Envelope<Result<Response, Error>>>,
}

impl I2cPuppetClient {
    /// Obtain an `I2cPuppetClient`
    ///
    /// If the [`I2cPuppetService`] hasn't been registered yet, we will retry until it
    /// has been registered.
    pub async fn from_registry(kernel: &'static Kernel) -> Self {
        loop {
            match I2cPuppetClient::from_registry_no_retry(kernel).await {
                Some(port) => return port,
                None => {
                    // I2C probably isn't registered yet. Try again in a bit
                    kernel.sleep(Duration::from_millis(10)).await;
                }
            }
        }
    }

    /// Obtain an `I2cPuppetClient`
    ///
    /// Does NOT attempt to get an [`I2cPuppetService`] handle more than once.
    ///
    /// Prefer [`I2cPuppetClient::from_registry`] unless you will not be spawning one
    /// around the same time as obtaining a client.
    pub async fn from_registry_no_retry(kernel: &'static Kernel) -> Option<Self> {
        let handle = kernel
            .with_registry(|reg| reg.get::<I2cPuppetService>())
            .await?;

        Some(I2cPuppetClient {
            handle,
            reply: Reusable::new_async().await,
        })
    }

    /// Subscribe to raw keyboard input from `i2c_puppet`'s Blackberry Q10
    /// returning a [`RawKeySubscription`].
    ///
    /// The returned [`RawKeySubscription`] provides access to keyboard events
    /// in the [`bbq10kbd`] crate's representation, which is specific to the
    /// Blackberry Q10 and Q20 keyboards. In general, it's preferable to
    /// implement code that requires keyboard input against the more generic
    /// [`KeyboardService`] defined in the cross-platform kernel crate.
    ///
    /// [`KeyboardService`]: kernel::services::keyboard::KeyboardService
    pub async fn subscribe_to_raw_keys(&mut self) -> Result<RawKeySubscription, Error> {
        if let Response::SubscribeToKeys(sub) = self.request(Request::SubscribeToRawKeys).await? {
            Ok(sub)
        } else {
            unreachable!("service responded with wrong response variant!")
        }
    }

    /// Sets the `i2c_puppet` RGB LED to the provided color.
    pub async fn set_led_color(&mut self, color: impl Into<RgbColor>) -> Result<RgbColor, Error> {
        let color = color.into();
        if let Response::SetColor(set_color) = self.request(Request::SetColor(color)).await? {
            assert_eq!(set_color, color);
            Ok(set_color)
        } else {
            unreachable!("service responded with wrong response variant!")
        }
    }

    /// Turns on or off the `i2c_puppet` RGB LED.
    pub async fn toggle_led(&mut self, on: bool) -> Result<bool, Error> {
        if let Response::ToggleLed(set_on) = self.request(Request::ToggleLed(on)).await? {
            assert_eq!(on, set_on);
            Ok(on)
        } else {
            unreachable!("service responded with wrong response variant!")
        }
    }

    /// Returns the current state of the `i2c_puppet` RGB LED.
    pub async fn led_status(&mut self) -> Result<LedState, Error> {
        if let Response::GetLedStatus(status) = self.request(Request::GetLedStatus).await? {
            Ok(status)
        } else {
            unreachable!("service responded with wrong response variant!")
        }
    }

    /// Sets the `i2c_puppet` Blackberry Q10 keyboard's backlight brightness. 0
    /// is off, 255 is maximum brightness.
    pub async fn set_backlight(&mut self, brightness: u8) -> Result<u8, Error> {
        if let Response::Backlight(set_brightness) =
            self.request(Request::SetBacklight(brightness)).await?
        {
            assert_eq!(brightness, set_brightness);
            Ok(brightness)
        } else {
            unreachable!("service responded with wrong response variant!")
        }
    }

    /// Returns the `i2c_puppet` keyboard's backlight brightness. 0
    /// is off, 255 is maximum brightness.
    pub async fn backlight(&mut self) -> Result<u8, Error> {
        if let Response::Backlight(brightness) = self.request(Request::GetBacklight).await? {
            Ok(brightness)
        } else {
            unreachable!("service responded with wrong response variant!")
        }
    }

    async fn request(&mut self, msg: Request) -> Result<Response, Error> {
        self.handle
            .request_oneshot(msg, &self.reply)
            .await
            .map_err(|error| {
                tracing::warn!(?error, "failed to send request to i2c_puppet service");
                Error::SendRequest(error)
            })
            .and_then(|resp| resp.body)
    }
}

////////////////////////////////////////////////////////////////////////////////
// Server Definition
////////////////////////////////////////////////////////////////////////////////

/// Server implementation for the [`I2cPuppetService`].
pub struct I2cPuppetServer {
    settings: I2cPuppetSettings,
    rx: KConsumer<registry::Message<I2cPuppetService>>,
    i2c: I2cClient,
    subscriptions: FixedVec<KProducer<(KeyStatus, KeyRaw)>>,
    keymux: Option<KeyboardMuxClient>,
}

#[derive(Debug)]
pub enum RegistrationError {
    Registry(registry::RegistrationError),
    NoI2cPuppet(I2cError),
    InvalidSettings(&'static str),
}

// https://github.com/solderparty/i2c_puppet#protocol
const ADDR: u8 = 0x1f;

/// i2c_puppet I2C registers
mod reg {
    /// To write with a register, we must OR the register number with this mask:
    /// <https://github.com/solderparty/i2c_puppet#protocol>
    pub(super) const WRITE: u8 = 0x80;

    // RGB LED configuration registers:
    // https://beepy.sqfmi.com/docs/firmware/rgb-led#set-rgb-color

    /// Controls whether the RGB LED is on or off.
    pub(super) const LED_ON: u8 = 0x20;

    /// 8-bit RGB LED red value.
    pub(super) const LED_R: u8 = 0x21;
    /// 8-bit RGB LED green value.
    pub(super) const LED_G: u8 = 0x22;
    /// 8-bit RGB LED blue value.
    pub(super) const LED_B: u8 = 0x23;

    /// Configuration register.
    pub(super) const CFG: u8 = 0x02;

    mycelium_bitfield::bitfield! {
        #[derive(Eq, PartialEq)]
        pub(super) struct Cfg<u8> {
            /// When a FIFO overflow happens, should the new entry still be
            /// pushed, overwriting the oldest one. If 0 then new entry is lost.
            pub(super) const OVERFLOW_ON: bool;
            /// Should an interrupt be generated when a FIFO overflow happens.
            pub(super) const OVERFLOW_INT: bool;
            /// Should an interrupt be generated when Caps Lock is toggled.
            pub(super) const CAPSLOCK_INT: bool;
            // Should an interrupt be generated when Num Lock is toggled.
            pub(super) const NUMLOCK_INT: bool;
            /// Should an interrupt be generated when a key is pressed.
            pub(super) const KEY_INT: bool;
            /// Should an interrupt be generated when the firmware panics? This
            /// is currently not implemented.
            pub(super) const PANIC_INT: bool;
            /// Should Alt, Sym and the Shift keys be reported as well.
            pub(super) const REPORT_MODS: bool;
            /// Should Alt, Sym and the Shift keys modify the keys being
            /// reported.
            pub(super) const USE_MODS: bool;
        }
    }

    mycelium_bitfield::bitfield! {
        #[derive(Eq, PartialEq)]
        pub(super) struct IntStatus<u8> {
            /// The interrupt was generated by a FIFO overflow.
            ///
            /// This is only set if [`Cfg::OVERFLOW_INT`] is set.
            pub(super) const OVERFLOW: bool;
            /// The interrupt was generated by Caps Lock
            ///
            /// This is only set if [`Cfg::CAPSLOCK_INT`] is set.
            pub(super) const CAPSLOCK: bool;
            /// The interrupt was generated by Num Lock
            ///
            /// This is only set if [`Cfg::NUMLOCK_INT`] is set.
            pub(super) const NUMLOCK: bool;
            /// The interrupt was generated by a key press.
            ///
            /// This is only set if [`Cfg::KEY_INT`] is set.
            pub(super) const KEY: bool;
            /// The interrupt was generated by a panic.
            ///
            /// **Note**: this is currently not implemented.
            pub(super) const PANIC: bool;
            /// This interrupt was generated by an input GPIO changing level.
            pub(super) const GPIO: bool;
            /// This interrupt was generated by a trackpad motion.
            pub(super) const TRACKPAD: bool;
        }
    }
}

impl I2cPuppetServer {
    /// Registers a new [`I2cPuppetServer`].
    ///
    /// # Arguments
    ///
    /// * `kernel`: a reference to the [`Kernel`], used for spawning tasks and
    ///   registering the driver.
    /// * `settings`: [`I2cPuppetSettings`] to configure the driver's behavior.
    /// * `irq_waker`: an optional [`WaitCell`] that will be notified when the
    ///   `i2c_puppet` IRQ line is asserted.
    ///
    ///   If the [`WaitCell`] is [`Some`], the `i2c_puppet` driver will poll
    ///   key status when the `WaitCell` is woken. If the [`WaitCell`] is
    ///   [`None`], the driver will only poll the `i2c_puppet` device when
    ///   [`settings.poll_interval`](I2cPuppetSettings#structfield.poll_interval)
    ///   elapses.
    #[instrument(level = Level::DEBUG, skip(kernel, irq_waker))]
    pub async fn register(
        kernel: &'static Kernel,
        settings: I2cPuppetSettings,
        irq_waker: impl Into<Option<&'static WaitCell>>,
    ) -> Result<(), RegistrationError> {
        let keymux = if settings.keymux {
            let keymux = KeyboardMuxClient::from_registry(kernel).await;
            tracing::debug!("acquired keyboard mux client");
            Some(keymux)
        } else {
            None
        };
        let (tx, rx) = KChannel::new_async(settings.channel_capacity).await.split();
        let mut i2c = {
            // The longest read or write operation we will perform is two bytes
            // long. Thus, we can reuse a single 2-byte buffer forever.
            let buf = FixedVec::new(2).await;
            I2cClient::from_registry(kernel).await.with_cached_buf(buf)
        };
        let subscriptions = FixedVec::new(settings.max_subscriptions).await;

        // first, make sure we can get the version, to make sure there's
        // actually an i2c_puppet on the bus. otherwise, there's no use in
        // spawning the driver at all!
        let Version { major, minor } = AsyncBbq10Kbd::new(&mut i2c)
            .get_version()
            .await
            .map_err(RegistrationError::NoI2cPuppet)?;
        tracing::info!("i2c_puppet firmware version: v{major}.{minor}");

        let cfg = reg::Cfg::new()
            .with(reg::Cfg::KEY_INT, true)
            .with(reg::Cfg::USE_MODS, true)
            .with(reg::Cfg::OVERFLOW_INT, true)
            // overwrite older keypresses when the FIFO is full.
            // since we only poll the keyboard when there are active
            // subscriptions, enable this setting so that the
            // FIFO doesn't fill up with ancient keypresses.
            .with(reg::Cfg::OVERFLOW_ON, true);
        tracing::info!("setting i2c_puppet config:\n{cfg}");
        i2c.write(ADDR, &[reg::CFG | reg::WRITE, cfg.bits()])
            .await
            .map_err(RegistrationError::NoI2cPuppet)?;

        let this = Self {
            settings,
            rx,
            i2c,
            subscriptions,
            keymux,
        };

        let span = tracing::info_span!("I2cPuppetServer");

        match irq_waker.into() {
            Some(irq) => {
                kernel
                    .spawn(this.run_with_irq(kernel, irq).instrument(span))
                    .await;
            }
            None => {
                kernel.spawn(this.run_no_irq(kernel).instrument(span)).await;
            }
        }

        kernel
            .with_registry(|reg| reg.register_konly::<I2cPuppetService>(&tx))
            .await
            .map_err(RegistrationError::Registry)?;
        Ok(())
    }

    async fn run_no_irq(mut self, kernel: &'static Kernel) {
        tracing::info!("running in poll-only mode...");
        loop {
            if let Ok(Ok(msg)) = kernel
                .timer()
                .timeout(self.settings.poll_interval, self.rx.dequeue_async())
                .await
            {
                self.handle_message(msg).await;
            }

            self.poll_keys().await;
        }
    }

    async fn run_with_irq(mut self, kernel: &'static Kernel, irq: &'static WaitCell) {
        tracing::info!("running in IRQ-driven mode...");
        loop {
            select_biased! {
                _ = irq.wait().fuse() => {
                    tracing::trace!("i2c_puppet IRQ fired!");
                },

                dequeued = self.rx.dequeue_async().fuse() => {
                    if let Ok(msg) = dequeued {
                        self.handle_message(msg).await;
                    }
                },

                _ = kernel.sleep(self.settings.poll_interval).fuse() => {
                    tracing::trace!("`i2c_puppet` poll interval elapsed");
                }
            }

            self.poll_keys().await;
        }
    }

    async fn handle_message(
        &mut self,
        registry::Message { msg, reply }: registry::Message<I2cPuppetService>,
    ) {
        let request = &msg.body;
        let send_reply = |rsp: Result<Response, Error>| {
            reply.reply_konly(msg.reply_with(rsp)).map_err(|error| {
                tracing::warn!(?error, ?request, "failed to reply to request!!");
                error
            })
        };
        match request {
            Request::SubscribeToRawKeys => {
                let (sub_tx, sub_rx) = KChannel::new_async(self.settings.subscription_capacity)
                    .await
                    .split();
                match self.subscriptions.try_push(sub_tx) {
                    Ok(()) => {
                        tracing::debug!("new subscription to keys");
                        let reply =
                            send_reply(Ok(Response::SubscribeToKeys(RawKeySubscription(sub_rx))))
                                .await;

                        if reply.is_err() {
                            // if the client hung up, remove the
                            // subscriptions entry we created.
                            self.subscriptions.pop();
                        }
                    }
                    Err(_) => {
                        tracing::warn!("subscriptions at capacity");
                        // if the reply fails, that's fine, because we
                        // didn't do anything anyway.
                        let _ = send_reply(Err(Error::AtMaxSubscriptions)).await;
                    }
                }
            }
            &Request::SetColor(color) => {
                let res = self.set_color(color).await;
                match res {
                    Ok(color) => {
                        tracing::trace!(?color, "set i2c_puppet LED color");
                        let _ = send_reply(Ok(Response::SetColor(color))).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to set i2c_puppet LED color");
                        let _ = send_reply(Err(Error::I2c(error))).await;
                    }
                }
            }

            &Request::ToggleLed(on) => {
                tracing::trace!(on, "toggling i2c_puppet LED...");
                let res = self
                    .i2c
                    .write(ADDR, &[reg::LED_ON | reg::WRITE, on as u8])
                    .await;
                match res {
                    Ok(()) => {
                        tracing::trace!(on, "toggled i2c_puppet LED");
                        let _ = send_reply(Ok(Response::ToggleLed(on))).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to toggle i2c_puppet LED");
                        let _ = send_reply(Err(Error::I2c(error))).await;
                    }
                }
            }

            Request::GetLedStatus => match self.get_led_status().await {
                Ok(led) => {
                    tracing::trace!(?led.color, led.on, "got i2c_puppet LED status");
                    let _ = send_reply(Ok(Response::GetLedStatus(led))).await;
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to get i2c_puppet LED status");
                    let _ = send_reply(Err(Error::I2c(error))).await;
                }
            },

            &Request::SetBacklight(brightness) => {
                tracing::trace!(brightness, "setting i2c_puppet backlight");
                match AsyncBbq10Kbd::new(&mut self.i2c)
                    .set_backlight(brightness)
                    .await
                {
                    Ok(()) => {
                        tracing::trace!(brightness, "set i2c_puppet backlight",);
                        let _ = send_reply(Ok(Response::Backlight(brightness))).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to set i2c_puppet backlight");
                        let _ = send_reply(Err(Error::I2c(error))).await;
                    }
                }
            }

            Request::GetBacklight => {
                tracing::trace!("getting i2c_puppet backlight");
                match AsyncBbq10Kbd::new(&mut self.i2c).get_backlight().await {
                    Ok(brightness) => {
                        tracing::trace!(brightness, "got i2c_puppet backlight",);
                        let _ = send_reply(Ok(Response::Backlight(brightness))).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to set i2c_puppet backlight");
                        let _ = send_reply(Err(Error::I2c(error))).await;
                    }
                }
            }

            Request::GetVersion => {
                tracing::debug!("getting i2c_puppet version");
                match AsyncBbq10Kbd::new(&mut self.i2c).get_version().await {
                    Ok(version) => {
                        tracing::debug!(
                            "i2c_puppet firmware version: v{}.{}",
                            version.major,
                            version.minor
                        );
                        let _ = send_reply(Ok(Response::GetVersion(version))).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to get i2c_puppet version");
                        let _ = send_reply(Err(Error::I2c(error))).await;
                    }
                }
            }
        }
    }

    async fn get_led_status(&mut self) -> Result<LedState, I2cError> {
        tracing::debug!("getting i2c_puppet LED status");
        let mut r = [0; 1];
        let mut g = [0; 1];
        let mut b = [0; 1];
        let mut on = [0; 1];
        self.i2c
            .transaction(
                ADDR,
                &mut [
                    i2c::Operation::Write(&[reg::LED_R]),
                    i2c::Operation::Read(&mut r),
                    i2c::Operation::Write(&[reg::LED_G]),
                    i2c::Operation::Read(&mut g),
                    i2c::Operation::Write(&[reg::LED_B]),
                    i2c::Operation::Read(&mut b),
                    i2c::Operation::Write(&[reg::LED_ON]),
                    i2c::Operation::Read(&mut on),
                ],
            )
            .await?;

        let color = RgbColor {
            r: r[0],
            g: g[0],
            b: b[0],
        };
        let on = on[0] != 0;
        Ok(LedState { color, on })
    }

    async fn set_color(&mut self, color: RgbColor) -> Result<RgbColor, I2cError> {
        tracing::debug!(?color, "setting i2c_puppet LED color");
        self.i2c
            .write(ADDR, &[reg::LED_R | reg::WRITE, color.r])
            .await?;
        self.i2c
            .write(ADDR, &[reg::LED_G | reg::WRITE, color.g])
            .await?;
        self.i2c
            .write(ADDR, &[reg::LED_B | reg::WRITE, color.b])
            .await?;
        Ok(color)
    }

    async fn poll_keys(&mut self) {
        // If there are no raw key subscriptions *and* we are not talking to a
        // keyboard multiplexing service, don't do anything.
        if self.keymux.is_none() && self.subscriptions.is_empty() {
            return;
        }

        tracing::trace!("polling keys...");

        if let Err(error) = self.poll_keys_inner().await {
            tracing::warn!(%error, "i2c_puppet: error polling keys!");
        }
    }

    async fn poll_keys_inner(&mut self) -> Result<(), I2cError> {
        fn keycode(x: u8) -> key_event::KeyCode {
            match x {
                0x08 => key_event::KeyCode::Backspace,
                // TODO(eliza): figure out other keycodes
                x => key_event::KeyCode::Char(x as char),
            }
        }

        let mut retry = self.settings.retry();
        let mut kbd = AsyncBbq10Kbd::new(&mut self.i2c);
        loop {
            let status = retry
                .retry_with_input(&mut kbd, |kbd| async move {
                    let res = kbd.get_key_status().await;
                    (kbd, res)
                })
                .await?;
            if let FifoCount::Known(0) = status.fifo_count {
                return Ok(());
            }
            let key = retry
                .retry_with_input(&mut kbd, |kbd| async move {
                    let res = kbd.get_fifo_key_raw().await;
                    (kbd, res)
                })
                .await?;
            tracing::debug!(?key);

            // TODO(eliza): remove dead subscriptions...
            for sub in self.subscriptions.as_slice_mut() {
                if let Err(error) = sub.enqueue_async((status, key)).await {
                    tracing::warn!(?error, "subscription dropped...");
                }
            }

            if let Some(ref mut keymux) = self.keymux {
                let modifiers = Modifiers::new()
                    .with(Modifiers::NUMLOCK, status.num_lock == NumLockState::On)
                    .with(Modifiers::CAPSLOCK, status.caps_lock == CapsLockState::On);
                let event = match key {
                    KeyRaw::Held(x) => KeyEvent {
                        modifiers,
                        code: keycode(x),
                        kind: key_event::Kind::Held,
                    },
                    KeyRaw::Pressed(x) => KeyEvent {
                        modifiers,
                        code: keycode(x),
                        kind: key_event::Kind::Pressed,
                    },
                    KeyRaw::Released(x) => KeyEvent {
                        modifiers,
                        code: keycode(x),
                        kind: key_event::Kind::Released,
                    },
                    KeyRaw::Invalid => continue,
                };
                if let Err(error) = keymux.publish_key(event).await {
                    tracing::warn!(?error, "failed to publish event to keymux!");
                }
            }
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Helper types
////////////////////////////////////////////////////////////////////////////////

// === I2cPuppetSettings ===

#[derive(Debug)]
pub struct I2cPuppetSettings {
    pub channel_capacity: usize,
    pub subscription_capacity: usize,
    pub max_subscriptions: usize,
    /// If set, the `i2c_puppet` service will also forward keypresses to the kernel's
    /// [`KeyboardMuxService`](kernel::services::keyboard::mux::KeyboardMuxService).
    pub keymux: bool,
    pub min_backoff: Duration,
    pub max_retries: usize,

    pub poll_interval: Duration,
}

impl Default for I2cPuppetSettings {
    fn default() -> Self {
        Self {
            channel_capacity: 8,
            subscription_capacity: 32,
            max_subscriptions: 8,
            poll_interval: Self::DEFAULT_POLL_INTERVAL,
            keymux: true,
            max_retries: 10,
            min_backoff: Self::DEFAULT_MIN_BACKOFF,
        }
    }
}

impl I2cPuppetSettings {
    /// The default `i2c_puppet` poll interval.
    pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);

    /// The default initial retry backoff for key status polling.
    pub const DEFAULT_MIN_BACKOFF: Duration = Duration::from_micros(5);

    /// Configures the interval for polling `i2c_puppet` keyboard status.
    ///
    /// By default, this is [`Self::DEFAULT_POLL_INTERVAL`].
    pub fn with_poll_interval(self, poll_interval: Duration) -> Self {
        Self {
            poll_interval,
            ..self
        }
    }

    fn retry(&self) -> Retry<WithMaxRetries<AlwaysRetry>, ExpBackoff> {
        let backoff = ExpBackoff::new(self.min_backoff).with_max_backoff(self.poll_interval);
        Retry::new(AlwaysRetry, backoff).with_max_retries(self.max_retries)
    }
}
// === impl KeySubscription ===

pub enum KeySubscriptionError {
    Closed,
    Decode,
    InvalidKey,
}

impl RawKeySubscription {
    pub async fn next_char(&mut self) -> Result<char, KeySubscriptionError> {
        loop {
            let (status, key) = self.next_raw().await?;
            let x = match key {
                KeyRaw::Pressed(x) => x,
                // KeyRaw::Released(x) => x,
                KeyRaw::Invalid => return Err(KeySubscriptionError::InvalidKey),
                _ => continue,
            };
            if let Some(mut c) = char::from_u32(x as u32) {
                if status.caps_lock == CapsLockState::On {
                    c = c.to_ascii_uppercase();
                }
                return Ok(c);
            } else {
                return Err(KeySubscriptionError::Decode);
            }
        }
    }

    pub async fn next_raw(&mut self) -> Result<(KeyStatus, KeyRaw), KeySubscriptionError> {
        self.0
            .dequeue_async()
            .await
            .map_err(|_| KeySubscriptionError::Closed)
    }
}

// TODO(eliza): maybe the color stuff belongs in its own module...`

// === impl HsvColor ===

impl HsvColor {
    pub fn from_hue(h: u8) -> Self {
        Self { h, s: 255, v: 255 }
    }

    #[must_use]
    pub fn to_rgb_color(self) -> RgbColor {
        const SECTIONS: u16 = 43;
        let HsvColor { h, s, v } = self;
        // if the saturation of this color is 0, then it's grey/black/white;
        // thus we can return early & save ourselves a great deal of math.
        if s == 0 {
            // for achromatic colors, the red, green, and blue are all
            // just the value component of the HSV representation
            return RgbColor { r: v, g: v, b: v };
            // otherwise, we'll have to do some Real Work.
        }

        // do all calculations in 16-bit to avoid overflow

        // calculate which section of the color wheel this color's
        // hue places us in, and the offset within that section.
        let section = h as u16 / SECTIONS;
        let section_offset = (h as u16 - (section * SECTIONS)) * 6;

        let p = ((v as u16 * (255 - s as u16)) >> 8) as u8;
        let q = ((v as u16 * (255 - ((s as u16 * section_offset) >> 8))) >> 8) as u8;
        let t = ((v as u16 * (255 - ((s as u16 * (255 - section_offset)) >> 8))) >> 8) as u8;

        match section {
            0 => RgbColor { r: v, g: t, b: p },
            1 => RgbColor { r: q, g: v, b: p },
            2 => RgbColor { r: p, g: v, b: t },
            3 => RgbColor { r: p, g: q, b: v },
            4 => RgbColor { r: t, g: p, b: v },
            _ => RgbColor { r: v, g: p, b: q },
        }
    }
}

// === impl RgbColor ===

impl RgbColor {
    pub const RED: Self = Self { r: 255, g: 0, b: 0 };
    pub const GREEN: Self = Self { r: 0, g: 255, b: 0 };
    pub const BLUE: Self = Self { r: 0, g: 0, b: 255 };
}

impl From<HsvColor> for RgbColor {
    #[inline]
    fn from(hsv: HsvColor) -> Self {
        hsv.to_rgb_color()
    }
}

impl fmt::Display for RgbColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { r, g, b } = self;
        write!(f, "#{r:02x}{g:02x}{b:02x}")
    }
}
