//! Driver for the Allwinner D1's SMHC peripheral.
//!
//! The D1 contains three separate SD/MMC Host Controllers: [`SMHC0`], [`SMHC1`] and [`SMHC2`].
//! - [`SMHC0`] controls *Secure Digital* memory devices (SD cards)
//! - [`SMHC1`] controls *Secure Digital I/O* devices (SDIO)
//! - [`SMHC2`] controls *MultiMedia Card* devices (MMC)
//!
//! Each SMHC also has an internal DMA controller that can be used for offloading
//! the transfer and reception of large amounts of data to/from the device.
use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    task::{Poll, Waker},
};

use crate::ccu::{BusGatingResetRegister, Ccu};
use d1_pac::{smhc, Interrupt, GPIO, SMHC0, SMHC1, SMHC2};
use kernel::{
    comms::kchannel::{KChannel, KConsumer},
    mnemos_alloc::containers::FixedVec,
    registry,
    services::sdmmc::{messages::Transfer, SdmmcService, Transaction},
    tracing, Kernel,
};

/// TODO
pub struct Smhc {
    isr: &'static IsrData,
    smhc: &'static smhc::RegisterBlock,
    int: (Interrupt, fn()),
}

/// TODO
struct IsrData {
    data: UnsafeCell<SmhcData>,
}
unsafe impl Sync for IsrData {}

struct SmhcDataGuard<'a> {
    smhc: &'a smhc::RegisterBlock,
    data: &'a mut SmhcData,
}

struct SmhcData {
    state: State,
    op: SmhcOp,
    err: Option<ErrorKind>,
    waker: Option<Waker>,
}

static SMHC0_ISR: IsrData = IsrData {
    data: UnsafeCell::new(SmhcData {
        state: State::Idle,
        op: SmhcOp::None,
        err: None,
        waker: None,
    }),
};

enum SmhcOp {
    // TODO
    None,
}

/// TODO
#[derive(Debug, Copy, Clone)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A transmit bit error, end bit error, or CMD index error has occurred.
    Response,
    /// Invalid CRC in response.
    ResponseCrc,
    /// When receiving data, this means that the received data has data CRC error.
    /// When transmitting data, this means that the received CRC status taken is negative.
    DataCrc,
    /// Did not receive a response in time.
    ResponseTimeout,
    /// Did not receive data in time.
    DataTimeout,
    /// Data starvation detected.
    DataStarvationTimeout,
    /// FIFO underrun or overflow.
    FifoUnderrunOverflow,
    /// Command busy and illegal write. TODO: understand this + add better explanation
    CommandBusyIllegalWrite,
    /// When receiving data, this means that the host controller found an error start bit.
    /// When transmitting data, this means that the busy signal is cleared after the last block.
    DataStart,
    /// When receiving data, this means that we did not receive a valid data end bit.
    /// When transmitting data, this means that we did not receive the CRC status token.
    DataEnd,
    /// An error occurred in the internal DMA controller.
    Dma,
    /// A different error occurred. The original error may contain more information.
    Other,
}

/// TODO
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[allow(dead_code)]
enum State {
    Idle,
    // TODO
}

impl Smhc {
    /// Initialize SMHC0 for SD cards.
    ///
    /// # Safety
    /// TODO
    pub unsafe fn new(mut smhc: SMHC0, ccu: &mut Ccu, gpio: &mut GPIO) -> Self {
        // Configure default pin mapping for TF (micro SD) card socket.
        // This is valid for the Lichee RV SOM and Mango Pi MQ Pro.
        gpio.pf_cfg0.modify(|_, w| {
            w.pf0_select().sdc0_d1();
            w.pf1_select().sdc0_d0();
            w.pf2_select().sdc0_clk();
            w.pf3_select().sdc0_cmd();
            w.pf4_select().sdc0_d3();
            w.pf5_select().sdc0_d2();
            w
        });

        // Make sure the card clock is turned off before changing the module clock
        smhc.smhc_clkdiv.write(|w| w.cclk_enb().off());

        ccu.disable_module(&mut smhc);
        // Set module clock rate to 200 MHz
        // TODO: ccu should provide a higher-level abstraction for this
        ccu.borrow_raw().smhc0_clk.write(|w| {
            w.clk_src_sel().pll_peri_1x();
            w.factor_n().variant(d1_pac::ccu::smhc0_clk::FACTOR_N_A::N1);
            w.factor_m().variant(2);
            w.clk_gating().set_bit();
            w
        });
        ccu.enable_module(&mut smhc);

        Self::init(
            unsafe { &*SMHC0::ptr() },
            Interrupt::SMHC0,
            Self::handle_smhc0_interrupt,
        )
    }

    /// This assumes the GPIO pin mappings and module clock are already configured.
    unsafe fn init(smhc: &'static smhc::RegisterBlock, int: Interrupt, isr: fn()) -> Self {
        // Closure to change the card clock
        let prg_clk = || {
            smhc.smhc_cmd.write(|w| {
                w.wait_pre_over().wait();
                w.prg_clk().change();
                w.cmd_load().set_bit();
                w
            });

            while smhc.smhc_cmd.read().cmd_load().bit_is_set() {
                core::hint::spin_loop()
            }

            smhc.smhc_rintsts.write(|w| unsafe { w.bits(0xFFFFFFFF) });
        };

        // Reset the SD/MMC controller
        smhc.smhc_ctrl.modify(|_, w| w.soft_rst().reset());
        while smhc.smhc_ctrl.read().soft_rst().is_reset() {
            core::hint::spin_loop();
        }

        // Reset the FIFO
        smhc.smhc_ctrl.modify(|_, w| w.fifo_rst().reset());
        while smhc.smhc_ctrl.read().fifo_rst().is_reset() {
            core::hint::spin_loop();
        }

        // Global interrupt disable
        smhc.smhc_ctrl.modify(|_, w| w.ine_enb().disable());

        prg_clk();

        // Set card clock = module clock / 2
        smhc.smhc_clkdiv.modify(|_, w| w.cclk_div().variant(1));

        // Set the sample delay to 0 (also done in Linux and Allwinner BSP)
        smhc.smhc_smap_dl.write(|w| {
            w.samp_dl_sw().variant(0);
            w.samp_dl_sw_en().set_bit()
        });

        // Enable the card clock
        smhc.smhc_clkdiv.modify(|_, w| w.cclk_enb().on());

        prg_clk();

        // Default bus width after power up or idle is 1-bit
        smhc.smhc_ctype.write(|w| w.card_wid().b1());
        // Blocksize is fixed to 512 bytes
        smhc.smhc_blksiz.write(|w| unsafe { w.bits(0x200) });

        Self {
            smhc,
            isr: &SMHC0_ISR,
            int: (int, isr),
        }
    }

    fn handle_smhc0_interrupt() {
        let _isr = kernel::isr::Isr::enter();
        let smhc = unsafe { &*SMHC0::ptr() };
        let data = unsafe { &mut (*SMHC0_ISR.data.get()) };

        data.advance_isr(smhc, 0);
    }

    pub async fn register(self, kernel: &'static Kernel, queued: usize) -> Result<(), ()> {
        let (tx, rx) = KChannel::new_async(queued).await.split();

        kernel.spawn(self.run(rx)).await;
        tracing::info!("SMHC driver task spawned");
        kernel
            .with_registry(move |reg| reg.register_konly::<SdmmcService>(&tx).map_err(drop))
            .await?;

        Ok(())
    }

    #[tracing::instrument(name = "SMHC", level = tracing::Level::INFO, skip(self, rx))]
    async fn run(self, rx: KConsumer<registry::Message<SdmmcService>>) {
        tracing::info!("starting SMHC driver task");
        while let Ok(registry::Message { msg, reply }) = rx.dequeue_async().await {
            todo!()
        }
    }

    #[tracing::instrument(level = tracing::Level::DEBUG, skip(self, txn))]
    async fn transaction(&self, txn: KConsumer<Transfer>) {
        todo!()
    }
}

impl IsrData {
    #[must_use]
    fn lock<'a>(&'a self, smhc: &'a smhc::RegisterBlock) -> SmhcDataGuard<'a> {
        // disable interrupts while holding the guard.
        smhc.smhc_ctrl.modify(|_, w| w.ine_enb().disable());
        let data = unsafe { &mut *(self.data.get()) };
        SmhcDataGuard { data, smhc }
    }
}

impl Drop for SmhcDataGuard<'_> {
    fn drop(&mut self) {
        self.smhc.smhc_ctrl.modify(|_, w| w.ine_enb().enable());
    }
}

impl SmhcDataGuard<'_> {
    async fn wait_for_irq(&mut self) {
        futures::future::poll_fn(|cx| {
            // TODO
            Poll::Ready(())
        })
        .await;
    }
}

impl Deref for SmhcDataGuard<'_> {
    type Target = SmhcData;

    fn deref(&self) -> &Self::Target {
        &*self.data
    }
}

impl DerefMut for SmhcDataGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

impl SmhcData {
    fn advance_isr(&mut self, smhc: &smhc::RegisterBlock, num: u8) {
        todo!()
    }
}
