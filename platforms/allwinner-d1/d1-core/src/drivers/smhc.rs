// Note: We sometimes force a pass by ref mut to enforce exclusive access
#![allow(clippy::needless_pass_by_ref_mut)]

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

use d1_pac::{smhc, Interrupt, GPIO, SMHC0, SMHC1, SMHC2};
use kernel::{
    mnemos_alloc::containers::FixedVec,
    registry,
    services::sdmmc::{self, SdmmcService},
    tracing, Kernel,
};

use crate::ccu::Ccu;

pub struct Smhc {
    isr: &'static IsrData,
    smhc: &'static smhc::RegisterBlock,
    int: (Interrupt, fn()),
    num: u8,
}

/// Data used by a SMHC interrupt.
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
    None,
    Control,
    Read {
        buf: FixedVec<u8>,
        cnt: usize,
        auto_stop: bool,
    },
    Write {
        buf: FixedVec<u8>,
        cnt: usize,
        auto_stop: bool,
    },
}

/// SMHC state machine
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[allow(dead_code)]
enum State {
    Idle,
    /// Waiting for command to be completed.
    WaitForCommand,
    /// Waiting for the DMA operation to be completed.
    WaitForDma,
    /// Waiting for data transfer to be completed.
    WaitForDataTransfer,
    /// Waiting for the auto-stop command to be sent and completed.
    WaitForAutoStop,
}

/// The different errors that can occur in this module.
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
    /// - The `SMHC0` register block must not be concurrently written to.
    /// - This function should be called only while running on an Allwinner D1.
    pub unsafe fn smhc0(mut smhc: SMHC0, ccu: &mut Ccu, gpio: &mut GPIO) -> Self {
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
        // Set module clock rate to 100 MHz
        // TODO: ccu should provide a higher-level abstraction for this
        ccu.borrow_raw().smhc0_clk.write(|w| {
            w.clk_src_sel().pll_peri_1x();
            w.factor_n().variant(d1_pac::ccu::smhc0_clk::FACTOR_N_A::N2);
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
        smhc.smhc_idie.write(|w| {
            w.des_unavl_int_enb().set_bit();
            w.ferr_int_enb().set_bit();
            w.rx_int_enb().set_bit();
            w.tx_int_enb().set_bit();
            w
        });

        Self::init(
            unsafe { &*SMHC0::ptr() },
            Interrupt::SMHC0,
            Self::handle_smhc0_interrupt,
            0,
        )
    }

    /// Initialize SMHC1 for SDIO cards.
    ///
    /// # Safety
    /// TODO
    pub unsafe fn smhc1(_smhc: SMHC1, _ccu: &mut Ccu, _gpio: &mut GPIO) -> Self {
        todo!()
    }

    /// Initialize SMHC2 for MMC cards.
    ///
    /// # Safety
    /// TODO
    pub unsafe fn smhc2(_smhc: SMHC2, _ccu: &mut Ccu, _gpio: &mut GPIO) -> Self {
        todo!()
    }

    /// This assumes the GPIO pin mappings and module clock are already configured.
    unsafe fn init(smhc: &'static smhc::RegisterBlock, int: Interrupt, isr: fn(), num: u8) -> Self {
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

        Self::reset_fifo(smhc);

        // Global interrupt disable
        smhc.smhc_ctrl.modify(|_, w| w.ine_enb().disable());

        prg_clk();

        // Set card clock = module clock / 2
        smhc.smhc_clkdiv.modify(|_, w| w.cclk_div().variant(1));

        // Set the sample delay to 0 (as done in Linux and Allwinner BSP)
        smhc.smhc_smap_dl.write(|w| {
            w.samp_dl_sw().variant(0);
            w.samp_dl_sw_en().set_bit();
            w
        });

        // Enable the card clock
        smhc.smhc_clkdiv.modify(|_, w| w.cclk_enb().on());

        prg_clk();

        // Default bus width after power up or idle is 1-bit
        smhc.smhc_ctype.write(|w| w.card_wid().b1());
        // Blocksize is fixed to 512 bytes
        smhc.smhc_blksiz.write(|w| unsafe { w.bits(512) });

        Self {
            smhc,
            isr: &SMHC0_ISR,
            int: (int, isr),
            num,
        }
    }

    #[inline(always)]
    fn reset_fifo(regs: &smhc::RegisterBlock) {
        regs.smhc_ctrl.modify(|_, w| w.fifo_rst().reset());
        while regs.smhc_ctrl.read().fifo_rst().is_reset() {
            core::hint::spin_loop();
        }
    }

    /// # Safety
    /// The descriptor chain needs to live at least as long as the DMA transfer.
    /// Additionally, their content (e.g., the validity of the buffers they point to)
    /// also needs to be verified by the user.
    unsafe fn prepare_dma(&self, descriptor: &idmac::Descriptor, byte_cnt: u32) {
        self.smhc.smhc_ctrl.modify(|_, w| {
            w.dma_enb().set_bit();
            w.dma_rst().set_bit();
            w
        });
        while self.smhc.smhc_ctrl.read().dma_rst().bit_is_set() {
            core::hint::spin_loop();
        }

        // Configure the address of the first DMA descriptor
        // Right-shift by 2 because it is a *word-address*.
        self.smhc
            .smhc_dlba
            .write(|w| unsafe { w.bits((descriptor as *const _ as u32) >> 2) });

        // Set number of bytes that will be read or written.
        self.smhc.smhc_bytcnt.write(|w| unsafe { w.bits(byte_cnt) });

        // Soft reset of DMA controller
        self.smhc.smhc_idmac.write(|w| w.idmac_rst().set_bit());

        // Configure the burst size and TX/RX trigger level
        // to the same values as used in the Linux implementation:
        // Burst size = 8, RX_TL = 7, TX_TL = 8
        self.smhc.smhc_fifoth.write(|w| {
            w.tx_tl().variant(8);
            w.rx_tl().variant(7);
            w.bsize_of_trans().t8();
            w
        });

        // Configure the transfer interrupt, receive interrupt, and abnormal interrupt.
        self.smhc.smhc_idie.write(|w| {
            w.rx_int_enb().set_bit();
            w.tx_int_enb().set_bit();
            w.err_sum_int_enb().set_bit();
            w
        });

        // Enable the IDMAC and configure burst transfers
        self.smhc.smhc_idmac.write(|w| {
            w.idmac_enb().set_bit();
            w.fix_bust_ctrl().set_bit();
            w
        });

        Self::reset_fifo(self.smhc);
    }

    /// Returns the interrupt and ISR for this SMHC.
    pub fn interrupt(&self) -> (Interrupt, fn()) {
        self.int
    }

    /// Handle an interrupt for SMHC0
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

    #[tracing::instrument(name = "SMHC", fields(num = self.num), level = tracing::Level::INFO, skip(self, rx))]
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
            return Err(sdmmc::Error::from(sdmmc::ErrorKind::Busy));
        }

        let cmd_idx = params.index;
        // TODO: naive `auto_stop` selection, this depends on the command (argument) that is used.
        // Should this (auto_stop select) be part of the params?
        let (data_trans, trans_dir, auto_stop) = match params.kind {
            sdmmc::CommandKind::Control => (false, false, false),
            sdmmc::CommandKind::Read(_len) => (true, false, true),
            sdmmc::CommandKind::Write(_len) => (true, true, true),
        };
        let chk_resp_crc = params.rsp_crc;
        let long_resp = params.rsp_type == sdmmc::ResponseType::Long;
        let resp_rcv = params.rsp_type != sdmmc::ResponseType::None;

        // Do any required configuration
        match params.options {
            sdmmc::HardwareOptions::None => (),
            sdmmc::HardwareOptions::SetBusWidth(width) => {
                self.smhc.smhc_ctype.write(|w| match width {
                    sdmmc::BusWidth::Single => w.card_wid().b1(),
                    sdmmc::BusWidth::Quad => w.card_wid().b4(),
                    sdmmc::BusWidth::Octo => w.card_wid().b8(),
                });
            }
        }

        let mut guard = self.isr.lock(self.smhc);
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
                tracing::error!("did not provide a buffer for read/write");
                return Err(sdmmc::Error::from(sdmmc::ErrorKind::Buffer));
            }
        };

        // Statically allocate space for 16 DMA descriptors.
        // Each descriptor can do a transfer of 4KB, giving a max total transfer of 64KB.
        // By declaring them in this scope the descriptor memory will live long enough,
        // however currently this is up to the user to guarantuee.
        // Safety: a zero'ed descriptor is valid and will simply be ignored by the IDMAC.
        let mut descriptors: [idmac::Descriptor; 16] = unsafe { core::mem::zeroed() };

        // Perform checks on arguments to make sure we won't overflow the buffer
        match &mut guard.data.op {
            SmhcOp::Read { buf, cnt, .. } | SmhcOp::Write { buf, cnt, .. } => {
                const DESCR_BUFF_SIZE: usize = 0x1000;

                // Currently we limit the number of data that can be read at once
                if *cnt > DESCR_BUFF_SIZE * descriptors.len() || buf.capacity() < *cnt {
                    return Err(sdmmc::Error::from(sdmmc::ErrorKind::Buffer));
                }

                tracing::debug!(cnt, "Creating descriptor chain from buffer");
                let buf_ptr = buf.as_slice_mut().as_mut_ptr();
                let mut remaining = *cnt;
                let mut index = 0;
                while remaining > 0 {
                    let buff_size = core::cmp::min(DESCR_BUFF_SIZE, remaining);
                    let buff_addr = unsafe { buf_ptr.add(index * DESCR_BUFF_SIZE) };
                    // Having to construct the slice in this manual way is not ideal,
                    // but it allows us to use `&mut [u8]` in the DescriptorBuilder.
                    let slice = unsafe { core::slice::from_raw_parts_mut(buff_addr, buff_size) };
                    remaining = remaining.saturating_sub(DESCR_BUFF_SIZE);
                    let first = index == 0;
                    let last = remaining == 0;
                    descriptors[index] = idmac::DescriptorBuilder::new()
                        .disable_irq(!last)
                        .first(first)
                        .last(last)
                        .link((!last).then(|| (&descriptors[index + 1]).into()))
                        .expect("Should be able to link to next descriptor")
                        .buff_slice(slice)
                        .map_err(|_| sdmmc::Error::from(sdmmc::ErrorKind::Buffer))?
                        .build();
                    index += 1;
                }

                // TODO: should this return some kind of guard, which needs to be used later
                // to make sure the descriptor has a long enough lifetime?
                unsafe { self.prepare_dma(&descriptors[0], *cnt as u32) };
            }
            _ => (),
        }

        // Set the argument to be sent
        self.smhc
            .smhc_cmdarg
            .write(|w| unsafe { w.bits(params.argument) });

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

        guard.data.state = State::WaitForCommand;
        // Now wait for completion or error interrupt
        guard.wait_for_irq().await;
        tracing::trace!("SMHC operation completed");
        if let Some(error) = guard.data.err.take() {
            tracing::warn!(?error, "SMHC error");
            let (kind, msg) = match error {
                ErrorKind::Response => (sdmmc::ErrorKind::Response, "Response"),
                ErrorKind::ResponseCrc => (sdmmc::ErrorKind::Response, "CRC"),
                ErrorKind::DataCrc => (sdmmc::ErrorKind::Data, "CRC"),
                ErrorKind::ResponseTimeout => (sdmmc::ErrorKind::Timeout, "Response"),
                ErrorKind::DataTimeout => (sdmmc::ErrorKind::Timeout, "Data"),
                ErrorKind::DataStarvationTimeout => (sdmmc::ErrorKind::Timeout, "DataStarvation"),
                ErrorKind::FifoUnderrunOverflow => (sdmmc::ErrorKind::Data, "FIFO"),
                ErrorKind::CommandBusyIllegalWrite => (sdmmc::ErrorKind::Busy, "Command"),
                ErrorKind::DataStart => (sdmmc::ErrorKind::Data, "DataStart"),
                ErrorKind::DataEnd => (sdmmc::ErrorKind::Data, "DataEnd"),
                ErrorKind::Dma => (sdmmc::ErrorKind::Other, "DMA"),
                ErrorKind::Other => (sdmmc::ErrorKind::Other, "Unknown"),
            };
            Err(sdmmc::Error::new(kind, msg))
        } else if long_resp {
            let rsp: [u32; 4] = [
                self.smhc.smhc_resp0.read().bits(),
                self.smhc.smhc_resp1.read().bits(),
                self.smhc.smhc_resp2.read().bits(),
                self.smhc.smhc_resp3.read().bits(),
            ];
            Ok(sdmmc::Response::Long(unsafe {
                core::mem::transmute::<[u32; 4], u128>(rsp)
            }))
        } else {
            Ok(sdmmc::Response::Short {
                value: self.smhc.smhc_resp0.read().bits(),
                data: match core::mem::replace(&mut guard.data.op, SmhcOp::None) {
                    SmhcOp::Read { buf, .. } => Some(buf),
                    _ => None,
                },
            })
        }
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
    /// Check for error bits in the interrupt status registers.
    fn error_status(smhc: &smhc::RegisterBlock) -> Option<ErrorKind> {
        let mint = smhc.smhc_mintsts.read();
        let idst = smhc.smhc_idst.read();

        if mint.m_dsto_vsd_int().bit_is_set() {
            Some(ErrorKind::DataStarvationTimeout)
        } else if mint.m_fu_fo_int().bit_is_set() {
            Some(ErrorKind::FifoUnderrunOverflow)
        } else if mint.m_cb_iw_int().bit_is_set() {
            Some(ErrorKind::CommandBusyIllegalWrite)
        } else if mint.m_rto_back_int().bit_is_set() {
            Some(ErrorKind::ResponseTimeout)
        } else if mint.m_rce_int().bit_is_set() {
            Some(ErrorKind::ResponseCrc)
        } else if mint.m_re_int().bit_is_set() {
            Some(ErrorKind::Response)
        } else if mint.m_dto_bds_int().bit_is_set() {
            Some(ErrorKind::DataTimeout)
        } else if mint.m_dce_int().bit_is_set() {
            Some(ErrorKind::DataCrc)
        } else if mint.m_dse_bc_int().bit_is_set() {
            Some(ErrorKind::DataStart)
        } else if mint.m_dee_int().bit_is_set() {
            Some(ErrorKind::DataEnd)
        } else if idst.des_unavl_int().bit_is_set() || idst.fatal_berr_int().bit_is_set() {
            Some(ErrorKind::Dma)
        } else {
            None
        }
    }

    fn advance_isr(&mut self, smhc: &smhc::RegisterBlock, num: u8) {
        tracing::trace!(state = ?self.state, smhc = num, "SMHC{num} interrupt");

        let mut needs_wake = false;

        // NOTE: to clear bits in the interrupt status registers,
        // you have to *write* a 1 to their location (W1C = write 1 to clear).

        self.err = Self::error_status(smhc);
        if self.err.is_some() {
            // Clear all interrupt bits
            smhc.smhc_rintsts.write(|w| unsafe { w.bits(0xFFFF_FFFF) });
            smhc.smhc_idst.write(|w| unsafe { w.bits(0x3FF) });

            needs_wake = true;
        }

        self.state = match self.state {
            State::Idle => State::Idle,
            State::WaitForCommand => {
                if smhc.smhc_mintsts.read().m_cc_int().bit_is_set() {
                    smhc.smhc_rintsts.write(|w| w.cc().set_bit());
                    match self.op {
                        SmhcOp::None => State::Idle,
                        SmhcOp::Control => {
                            needs_wake = true;
                            State::Idle
                        }
                        SmhcOp::Read { .. } | SmhcOp::Write { .. } => State::WaitForDma,
                    }
                } else {
                    self.state
                }
            }
            State::WaitForDma => {
                // TODO: better way to check for RX and TX,
                // *normal interrupt summary* does not seem to work
                if smhc.smhc_idst.read().bits() != 0 {
                    smhc.smhc_idst.write(|w| {
                        w.nor_int_sum().set_bit();
                        w.rx_int().set_bit();
                        w.tx_int().set_bit();
                        w
                    });
                    State::WaitForDataTransfer
                } else {
                    self.state
                }
            }
            State::WaitForDataTransfer => {
                if smhc.smhc_mintsts.read().m_dtc_int().bit_is_set() {
                    smhc.smhc_rintsts.write(|w| w.dtc().set_bit());
                    match &mut self.op {
                        SmhcOp::None | SmhcOp::Control => State::Idle,
                        SmhcOp::Read {
                            auto_stop,
                            buf,
                            cnt,
                        }
                        | SmhcOp::Write {
                            auto_stop,
                            buf,
                            cnt,
                        } => {
                            tracing::trace!("setting buf len: {cnt}");
                            // Safety: we have already checked that cnt <= buf capacity
                            unsafe { buf.as_vec_mut().set_len(*cnt) };
                            if *auto_stop {
                                State::WaitForAutoStop
                            } else {
                                needs_wake = true;
                                State::Idle
                            }
                        }
                    }
                } else {
                    self.state
                }
            }
            State::WaitForAutoStop => {
                if smhc.smhc_mintsts.read().m_acd_int().bit_is_set() {
                    smhc.smhc_rintsts.write(|w| w.acd().set_bit());
                    needs_wake = true;
                    State::Idle
                } else {
                    self.state
                }
            }
        };

        if needs_wake {
            if let Some(waker) = self.waker.take() {
                waker.wake();
                // If we are waking the driver task, we need to disable interrupts
                smhc.smhc_ctrl.modify(|_, w| w.ine_enb().disable());
            }
        }
    }
}

/// Internal DMA controller
mod idmac {
    use core::{mem, ptr::NonNull};

    use mycelium_bitfield::bitfield;

    /// A descriptor that describes how memory needs to transfer data
    /// between the SMHC port and host memory. Multiple DMA transfers
    /// can be configured by creating a descriptor *chain* (linked list).
    #[derive(Clone, Debug)]
    #[repr(C, align(4))]
    pub(super) struct Descriptor {
        /// The descriptor configuration.
        configuration: Cfg,
        /// The size of the data buffer.
        buff_size: BuffSize,
        /// The (*word*) address of the data buffer.
        buff_addr: u32,
        /// The (*word*) address of the next descriptor, for creating a descriptor chain.
        next_desc: u32,
    }

    /// A builder for constructing an IDMAC [`Descriptor`].
    #[derive(Copy, Clone, Debug)]
    #[must_use = "a `DescriptorBuilder` does nothing unless `DescriptorBuilder::build()` is called"]
    pub(super) struct DescriptorBuilder<B = ()> {
        cfg: Cfg,
        buff: B,
        link: u32,
    }

    #[derive(Debug)]
    pub(super) enum Error {
        BufferAddr,
        BufferSize,
        Link,
    }

    bitfield! {
        /// The first 32-bit word of an IDMAC descriptor, containing configuration data.
        struct Cfg<u32> {
            // Bit 0 is reserved.
            const _RESERVED_0 = 1;

            /// Disable interrupts on completion.
            const CUR_TXRX_OVER_INT_DIS: bool;

            /// When set to 1, this bit indicates that the buffer this descriptor points to
            /// is the last data buffer.
            const LAST: bool;

            /// When set to 1, this bit indicates that this descriptor contains
            /// the first buffer of data. It must be set to 1 in the first DES.
            const FIRST: bool;

            /// When set to 1, this bit indicates that the second address in the descriptor
            /// is the next descriptor address. It must be set to 1.
            const CHAIN_MOD: bool;

            /// Bits 29:5 are reserved.
            const _RESERVED_1 = 25;

            /// When some errors happen in transfer, this bit will be set to 1 by the IDMAC.
            const ERR: bool;

            /// When set to 1, this bit indicates that the descriptor is owned by the IDMAC.
            /// When this bit is reset, it indicates that the descriptor is owned by the host.
            /// This bit is cleared when the transfer is over.
            const DES_OWN: bool;
        }
    }

    bitfield! {
        /// The second 32-bit word of an IDMAC descriptor, containing data buffer size.
        struct BuffSize<u32> {
            /// The data buffer byte size, which must be a multiple of 4 bytes.
            /// If this field is 0, the DMA ignores this buffer and proceeds to the next descriptor.
            const SIZE = 13;

            /// Bits 31:13 are reserved.
            const _RESERVED_0 = 19;
        }
    }

    impl DescriptorBuilder {
        pub const fn new() -> Self {
            Self {
                cfg: Cfg::new(),
                buff: (),
                link: 0,
            }
        }

        pub fn disable_irq(self, val: bool) -> Self {
            Self {
                cfg: self.cfg.with(Cfg::CUR_TXRX_OVER_INT_DIS, val),
                ..self
            }
        }

        pub fn first(self, val: bool) -> Self {
            Self {
                cfg: self.cfg.with(Cfg::FIRST, val),
                ..self
            }
        }

        pub fn last(self, val: bool) -> Self {
            Self {
                cfg: self.cfg.with(Cfg::LAST, val),
                ..self
            }
        }

        pub fn buff_slice(
            self,
            buff: &'_ mut [u8],
        ) -> Result<DescriptorBuilder<&'_ mut [u8]>, Error> {
            if buff.len() > Descriptor::MAX_LEN as usize {
                return Err(Error::BufferSize);
            }

            if (buff.len() & 0b11) > 0 {
                return Err(Error::BufferSize);
            }

            let buff_addr = buff.as_mut_ptr() as *mut _ as u32;
            if (buff_addr & 0b11) > 0 {
                return Err(Error::BufferAddr);
            }

            Ok(DescriptorBuilder {
                cfg: self.cfg,
                buff,
                link: self.link,
            })
        }

        pub fn link(self, link: impl Into<Option<NonNull<Descriptor>>>) -> Result<Self, Error> {
            let link = link
                .into()
                .map(Descriptor::addr_to_link)
                .transpose()?
                .unwrap_or(0);
            Ok(Self { link, ..self })
        }
    }

    impl DescriptorBuilder<&'_ mut [u8]> {
        pub fn build(self) -> Descriptor {
            let buff_size = BuffSize::new().with(BuffSize::SIZE, self.buff.len() as u32);
            let buff_addr = (self.buff.as_mut_ptr() as *mut _ as u32) >> 2;

            Descriptor {
                configuration: self.cfg.with(Cfg::CHAIN_MOD, true).with(Cfg::DES_OWN, true),
                buff_size,
                buff_addr,
                next_desc: self.link,
            }
        }
    }

    impl Descriptor {
        /// Maximum length for arguments to [`DescriptorBuilder::buff_slice`].
        /// Must be 13 bits wide or less.
        pub const MAX_LEN: u32 = (1 << 13) - 1;

        fn addr_to_link(link: NonNull<Self>) -> Result<u32, Error> {
            let addr = link.as_ptr() as usize;
            if addr & (mem::align_of::<Self>() - 1) > 0 {
                return Err(Error::Link);
            }

            Ok((addr as u32) >> 2)
        }
    }
}
