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
    services::sdmmc::{self, SdmmcService},
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
        op: SmhcOp::Control,
        err: None,
        waker: None,
    }),
};

enum SmhcOp {
    Control,
    Read {
        buf: FixedVec<u8>,
        cnt: u32,
        auto_stop: bool,
    },
    Write {
        buf: FixedVec<u8>,
        cnt: u32,
        auto_stop: bool,
    },
}

/// TODO
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[allow(dead_code)]
enum State {
    Idle,
    /// Waiting for command to be completed.
    WaitForCommand,
    /// Waiting for data transfer to be completed.
    WaitForDataTransfer,
    /// Waiting for the auto-stop command to be sent and completed.
    WaitForAutoStop,
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

        // Enable interrupts that are relevant for an SD card
        smhc.smhc_intmask.write(|w| {
            w.dee_int_en().set_bit();
            w.acd_int_en().set_bit();
            w.dse_bc_int_en().set_bit();
            w.cb_iw_int_en().set_bit();
            w.fu_fo_int_en().set_bit();
            w.dsto_vsd_int_en().set_bit();
            w.dto_bds_int_en().set_bit();
            w.dce_int_en().set_bit();
            w.rce_int_en().set_bit();
            w.dtc_int_en().set_bit();
            w.cc_int_en().set_bit();
            w.re_int_en().set_bit();
            w
        });

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

        // Set the sample delay to 0 (as done in Linux and Allwinner BSP)
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

    pub fn handle_smhc0_interrupt() {
        let _isr = kernel::isr::Isr::enter();
        let smhc = unsafe { &*SMHC0::ptr() };
        // safety: it's okay to do this since this function can only be called from inside the ISR.
        let data = unsafe { &mut (*SMHC0_ISR.data.get()) };

        data.advance_isr(smhc, 0);
    }

    pub async fn register(
        self,
        kernel: &'static Kernel,
        queued: usize,
    ) -> Result<(), registry::RegistrationError> {
        let rx = kernel
            .registry()
            .bind_konly::<SdmmcService>(queued)
            .await?
            .into_request_stream(queued)
            .await;

        kernel.spawn(self.run(rx)).await;
        tracing::info!("SMHC driver task spawned");

        Ok(())
    }

    #[tracing::instrument(name = "SMHC", level = tracing::Level::INFO, skip(self, rx))]
    async fn run(self, rx: registry::listener::RequestStream<SdmmcService>) {
        tracing::info!("starting SMHC driver task");
        loop {
            let registry::Message { mut msg, reply } = rx.next_request().await;
            let response = self.command(msg.body).await;
            // TODO: we don't need `msg.body` anymore, but since it has been moved
            // we need to supply another value if we want to use `msg` later to reply.
            msg.body = sdmmc::Command::default();
            if let Err(error) = reply.reply_konly(msg.reply_with(response)).await {
                tracing::warn!(?error, "client hung up...");
            }
        }
    }

    #[tracing::instrument(level = tracing::Level::DEBUG, skip(self, params))]
    async fn command(&self, params: sdmmc::Command) -> Result<sdmmc::Response, sdmmc::Error> {
        if self.smhc.smhc_cmd.read().cmd_load().bit_is_set() {
            return Err(sdmmc::Error::Busy);
        }

        let cmd_idx = params.index;
        // TODO: naive `auto_stop` selection, probably only works in case of SD memory cards.
        // Should this (auto_stop select) be part of the params?
        let (data_trans, trans_dir, auto_stop) = match params.kind {
            sdmmc::CommandKind::Control => (false, false, false),
            sdmmc::CommandKind::Read(len) => (true, false, len > 512),
            sdmmc::CommandKind::Write(len) => (true, true, len > 512),
        };
        let chk_resp_crc = params.rsp_crc;
        let long_resp = params.rsp_type == sdmmc::ResponseType::Long;
        let resp_rcv = params.rsp_type != sdmmc::ResponseType::None;

        // Configure and start command
        self.smhc.smhc_cmd.write(|w| {
            w.cmd_load().set_bit();
            w.wait_pre_over().wait();
            w.stop_cmd_flag().bit(auto_stop);
            w.data_trans().bit(data_trans);
            w.trans_dir().bit(trans_dir);
            w.chk_resp_crc().bit(chk_resp_crc);
            w.long_resp().bit(long_resp);
            w.resp_rcv().bit(resp_rcv);
            w.cmd_idx().variant(cmd_idx);
            w
        });

        // Now wait for completion or error interrupt
        let mut guard = self.isr.lock(self.smhc);
        guard.data.state = State::WaitForCommand;
        guard.data.op = match (params.kind, params.buffer) {
            (sdmmc::CommandKind::Control, _) => SmhcOp::Control,
            (sdmmc::CommandKind::Read(cnt), Some(buf)) => SmhcOp::Read {
                buf,
                cnt,
                auto_stop,
            },
            (sdmmc::CommandKind::Write(cnt), Some(buf)) => SmhcOp::Write {
                buf,
                cnt,
                auto_stop,
            },
            _ => {
                tracing::warn!("did not provide a buffer for read/write");
                return Err(sdmmc::Error::Other);
            }
        };

        guard.wait_for_irq().await;
        tracing::trace!("SMHC operation completed");
        let res = if let Some(error) = guard.data.err.take() {
            tracing::warn!(?error, "SMHC error");
            Err(sdmmc::Error::Other) // TODO
        } else {
            if long_resp {
                Ok(sdmmc::Response::Long([
                    self.smhc.smhc_resp0.read().bits(),
                    self.smhc.smhc_resp1.read().bits(),
                    self.smhc.smhc_resp2.read().bits(),
                    self.smhc.smhc_resp3.read().bits(),
                ]))
            } else {
                Ok(sdmmc::Response::Short(self.smhc.smhc_resp0.read().bits()))
            }
        };
        res
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
        let mut waiting = false;
        futures::future::poll_fn(|cx| {
            if waiting {
                self.smhc.smhc_ctrl.modify(|_, w| w.ine_enb().disable());
                return Poll::Ready(());
            }

            self.data.waker = Some(cx.waker().clone());
            waiting = true;
            self.smhc.smhc_ctrl.modify(|_, w| w.ine_enb().enable());

            Poll::Pending
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
        let mut needs_wake = false;
        tracing::trace!(state = ?self.state, smhc = num, "SMHC{num} interrupt");

        match self.state {
            State::Idle => (),
            State::WaitForCommand => (),
            State::WaitForDataTransfer => (),
            State::WaitForAutoStop => (),
        }

        if needs_wake {
            if let Some(waker) = self.waker.take() {
                waker.wake();
                // If we are waking the driver task, we need to disable interrupts
                // until the driver can prepare the next phase of the transaction.
                smhc.smhc_ctrl.modify(|_, w| w.ine_enb().disable());
            }
        }
        // TODO
    }
}
