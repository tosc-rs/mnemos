// Note: We sometimes force a pass by ref mut to enforce exclusive access
#![allow(clippy::needless_pass_by_ref_mut)]

//! Spi Sender

use core::ptr::NonNull;

use crate::ccu::Ccu;
use crate::dmac::{
    descriptor::{
        AddressMode, BModeSel, BlockSize, DataWidth, DescriptorConfig, DestDrqType, SrcDrqType,
    },
    ChannelMode, Dmac,
};
use d1_pac::{GPIO, SPI_DBI};
use kernel::{
    comms::oneshot::Reusable,
    maitake::sync::WaitCell,
    mnemos_alloc::containers::FixedVec,
    registry::{self, uuid, Envelope, KernelHandle, Message, RegisteredDriver, ReplyTo, Uuid},
    Kernel,
};

pub static SPI1_TX_DONE: WaitCell = WaitCell::new();

pub struct Spim1 {
    _x: (),
}

/// # Safety
///
/// - The `SPI_DBI``s register block must not be concurrently written to.
/// - This function should be called only while running on an Allwinner D1.
pub unsafe fn kernel_spim1(mut spi1: SPI_DBI, ccu: &mut Ccu, gpio: &mut GPIO) -> Spim1 {
    // Set clock rate (fixed to 2MHz), and enable the SPI peripheral
    // TODO: ccu should provide a higher-level abstraction for this
    ccu.borrow_raw().spi1_clk.write(|w| {
        // Enable clock
        w.clk_gating().on();
        // base:  24 MHz
        w.clk_src_sel().hosc();
        // /1:    24 MHz
        w.factor_n().n1();
        // /12:    2 MHz
        w.factor_m().variant(11);
        w
    });
    ccu.enable_module(&mut spi1);

    // Map the pins
    gpio.pd_cfg1.write(|w| {
        // Select SPI pin mode
        w.pd10_select().spi1_cs_dbi_csx();
        w.pd11_select().spi1_clk_dbi_sclk();
        w.pd12_select().spi1_mosi_dbi_sdo();
        w
    });
    gpio.pd_pull0.write(|w| {
        // Disable pull up/downs
        w.pd10_pull().pull_disable();
        w.pd11_pull().pull_disable();
        w.pd12_pull().pull_disable();
        w
    });

    // Hard coded configuration for specifically supporting the SHARP memory display
    spi1.spi_tcr.write(|w| {
        // Allow the hardware to control the chip select
        w.ss_owner().spi_controller();
        // LSB first bit-order
        w.fbs().lsb();
        // Chip select active HIGH (the sharp display is weird and is active high)
        w.spol().clear_bit();
        w
    });
    spi1.spi_gcr.write(|w| {
        // Transmit pause Enable - ignore RXFIFO being full
        w.tp_en().normal();
        // Master/Controller mode
        w.mode().master();
        // Enable
        w.en().enable();
        w
    });
    spi1.spi_fcr.modify(|_r, w| {
        // TX FIFO DMA Request Enable
        w.tf_drq_en().enable();
        w
    });

    Spim1 { _x: () }
}

impl RegisteredDriver for SpiSender {
    type Request = SpiSenderRequest;
    type Response = SpiSenderResponse;
    type Error = SpiSenderError;
    type Hello = ();
    type ConnectError = core::convert::Infallible;

    const UUID: Uuid = uuid!("b5fd3487-08c4-4c0c-ae97-65dd1b151138");
}

pub struct SpiSender;
pub struct SpiSenderServer;

impl SpiSenderServer {
    pub async fn register(
        kernel: &'static Kernel,
        dmac: Dmac,
        queued: usize,
    ) -> Result<(), registry::RegistrationError> {
        let reqs = kernel
            .registry()
            .bind_konly::<SpiSender>(queued)
            .await?
            .into_request_stream(queued)
            .await;
        kernel
            .spawn(async move {
                let spi = unsafe { &*SPI_DBI::PTR };

                let txd_ptr: *mut u32 = spi.spi_txd.as_ptr();
                let txd_ptr: *mut u8 = txd_ptr.cast();
                let txd_ptr: *mut () = txd_ptr.cast();

                loop {
                    let Message { msg, reply } = reqs.next_request().await;
                    let SpiSenderRequest::Send(ref payload) = msg.body;

                    let len = payload.as_slice().len();

                    spi.spi_bcc.modify(|_r, w| {
                        // "Single Mode Transmit Counter" - the number of bytes to send
                        w.stc().variant(len as u32);
                        w
                    });
                    spi.spi_mbc.modify(|_r, w| {
                        // Master Burst Counter
                        w.mbc().variant(len as u32);
                        w
                    });
                    spi.spi_mtc.modify(|_r, w| {
                        w.mwtc().variant(len as u32);
                        w
                    });
                    // Start transfer
                    spi.spi_tcr.modify(|_r, w| {
                        w.xch().initiate_exchange();
                        w
                    });

                    let d_cfg = DescriptorConfig {
                        source: payload.as_slice().as_ptr().cast(),
                        destination: txd_ptr,
                        byte_counter: len,
                        link: None,
                        wait_clock_cycles: 0,
                        bmode: BModeSel::Normal,
                        dest_width: DataWidth::Bit8,
                        dest_addr_mode: AddressMode::IoMode,
                        dest_block_size: BlockSize::Byte1,
                        dest_drq_type: DestDrqType::Spi1Tx,
                        src_data_width: DataWidth::Bit8,
                        src_addr_mode: AddressMode::LinearMode,
                        src_block_size: BlockSize::Byte1,
                        src_drq_type: SrcDrqType::Dram,
                    };
                    let descriptor = d_cfg.try_into().map_err(drop)?;

                    // start the DMA transfer.
                    unsafe {
                        dmac.transfer(
                            ChannelMode::Wait,
                            ChannelMode::Handshake,
                            NonNull::from(&descriptor),
                        )
                        .await;
                    }

                    reply
                        .reply_konly(msg.reply_with_body(|req| {
                            let SpiSenderRequest::Send(payload) = req;
                            Ok(SpiSenderResponse::Sent(payload))
                        }))
                        .await
                        .unwrap();
                }

                #[allow(unreachable_code)]
                Result::<(), ()>::Ok(())
            })
            .await;

        Ok(())
    }
}

pub enum SpiSenderRequest {
    Send(FixedVec<u8>),
}

pub enum SpiSenderResponse {
    Sent(FixedVec<u8>),
}

pub enum SpiSenderError {
    Oops,
}

pub struct SpiSenderClient {
    hdl: KernelHandle<SpiSender>,
    osc: Reusable<Envelope<Result<SpiSenderResponse, SpiSenderError>>>,
}

impl SpiSenderClient {
    pub async fn from_registry(
        kernel: &'static Kernel,
    ) -> Result<SpiSenderClient, registry::ConnectError<SpiSender>> {
        let hdl = kernel.registry().connect(()).await?;

        Ok(SpiSenderClient {
            hdl,
            osc: Reusable::new_async().await,
        })
    }

    pub async fn from_registry_no_retry(
        kernel: &'static Kernel,
    ) -> Result<SpiSenderClient, registry::ConnectError<SpiSender>> {
        let hdl = kernel.registry().try_connect(()).await?;

        Ok(SpiSenderClient {
            hdl,
            osc: Reusable::new_async().await,
        })
    }

    pub async fn send_wait(&mut self, data: FixedVec<u8>) -> Result<FixedVec<u8>, SpiSenderError> {
        self.hdl
            .send(
                SpiSenderRequest::Send(data),
                ReplyTo::OneShot(self.osc.sender().await.unwrap()),
            )
            .await
            .map_err(|_| SpiSenderError::Oops)?;
        self.osc
            .receive()
            .await
            .map_err(|_| SpiSenderError::Oops)?
            .body
            .map(|resp| {
                let SpiSenderResponse::Sent(payload) = resp;
                payload
            })
    }
}
