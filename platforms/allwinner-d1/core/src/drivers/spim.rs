// Spi Sender

use core::ptr::NonNull;

use crate::dmac::{
    descriptor::{
        AddressMode, BModeSel, BlockSize, DataWidth, DescriptorConfig, DestDrqType, SrcDrqType,
    },
    Channel, ChannelMode,
};
use d1_pac::{CCU, GPIO, SPI_DBI};
use kernel::{
    comms::{kchannel::KChannel, oneshot::Reusable},
    maitake::sync::WaitQueue,
    mnemos_alloc::containers::FixedVec,
    registry::{uuid, Envelope, KernelHandle, Message, RegisteredDriver, ReplyTo, Uuid},
    Kernel,
};

pub static SPI1_TX_DONE: WaitQueue = WaitQueue::new();

pub struct Spim1 {
    _x: (),
}

pub unsafe fn kernel_spim1(spi1: SPI_DBI, ccu: &mut CCU, gpio: &mut GPIO) -> Spim1 {
    // Set clock rate (fixed to 2MHz), and enable the SPI peripheral
    ccu.spi1_clk.write(|w| {
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
    ccu.spi_bgr.modify(|_r, w| {
        w.spi1_gating().pass().spi1_rst().deassert();
        w
    });

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

    const UUID: Uuid = uuid!("b5fd3487-08c4-4c0c-ae97-65dd1b151138");
}

pub struct SpiSender;
pub struct SpiSenderServer;

impl SpiSenderServer {
    pub async fn register(kernel: &'static Kernel, queued: usize) -> Result<(), ()> {
        let (kprod, kcons) = KChannel::new_async(queued).await.split();

        kernel
            .spawn(async move {
                let kcons = kcons;
                let spi = unsafe { &*SPI_DBI::PTR };

                let txd_ptr: *mut u32 = spi.spi_txd.as_ptr();
                let txd_ptr: *mut u8 = txd_ptr.cast();
                let txd_ptr: *mut () = txd_ptr.cast();

                loop {
                    let msg: Message<SpiSender> = kcons.dequeue_async().await.unwrap();
                    // println!("DEQUEUE");
                    let Message { msg, reply } = msg;
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
                    let mut chan;
                    unsafe {
                        chan = Channel::summon_channel(1);
                        chan.set_channel_modes(ChannelMode::Wait, ChannelMode::Wait);
                        chan.start_descriptor(NonNull::from(&descriptor));
                    }
                    SPI1_TX_DONE
                        .wait()
                        .await
                        .expect("SPI1_TX_DONE WaitQueue should never be closed");
                    unsafe {
                        chan.stop_dma();
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

        kernel
            .with_registry(move |reg| reg.register_konly::<SpiSender>(&kprod).map_err(drop))
            .await?;

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
    pub async fn from_registry(kernel: &'static Kernel) -> Result<SpiSenderClient, ()> {
        let hdl = kernel.with_registry(|reg| reg.get()).await.ok_or(())?;

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
