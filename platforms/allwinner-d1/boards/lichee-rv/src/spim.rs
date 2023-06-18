
// Spi Sender

use core::ptr::NonNull;

use d1_pac::SPI_DBI;
use drivers::dmac::{ChannelMode, Channel, descriptor::{BModeSel, DataWidth, AddressMode, BlockSize, DestDrqType, SrcDrqType, DescriptorConfig}};
use kernel::{comms::{kchannel::KChannel, oneshot::Reusable}, Kernel, maitake::sync::WaitCell, registry::{Uuid, uuid, RegisteredDriver, ReplyTo, Envelope, KernelHandle, Message}, mnemos_alloc::containers::HeapArray};

pub static SPI1_TX_DONE: WaitCell = WaitCell::new();

pub struct SpiSender {
    _x: (),
}

impl SpiSender {
    pub async fn register(kernel: &'static Kernel, queued: usize) -> Result<(), ()> {
        let (kprod, kcons) = KChannel::new_async(kernel, queued).await.split();

        kernel.spawn(async move {
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

                spi.spi_bcc.modify(|_r, w| {
                    w.stc().variant(payload.len() as u32);
                    w
                });
                spi.spi_mbc.modify(|_r, w| {
                    w.mbc().variant(payload.len() as u32);
                    w
                });
                spi.spi_mtc.modify(|_r, w| {
                    w.mwtc().variant(payload.len() as u32);
                    w
                });

                spi.spi_tcr.modify(|_r, w| {
                    w.xch().initiate_exchange();
                    w
                });

                let d_cfg = DescriptorConfig {
                    source: payload.as_ptr().cast(),
                    destination: txd_ptr,
                    byte_counter: payload.len(),
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
                unsafe {
                    let mut chan = Channel::summon_channel(1);
                    chan.set_channel_modes(ChannelMode::Wait, ChannelMode::Handshake);
                    chan.start_descriptor(NonNull::from(&descriptor));
                }
                match SPI1_TX_DONE.wait().await {
                    Ok(_) => {},
                    Err(_) => todo!(),
                }
                // println!("WOKE");
                reply.reply_konly(msg.reply_with2(|req| {
                    let SpiSenderRequest::Send(payload) = req;
                    Ok(SpiSenderResponse::Sent(payload))
                })).await.unwrap();
            }

            #[allow(unreachable_code)]
            Result::<(), ()>::Ok(())

        }).await;

        kernel.with_registry(move |reg| {
            reg.register_konly::<SpiSender>(&kprod).map_err(drop)
        }).await?;

        Ok(())
    }

    pub async fn from_registry(kernel: &'static Kernel) -> Result<SpiSenderClient, ()> {
        let hdl = kernel.with_registry(|reg| {
            reg.get()
        }).await.ok_or(())?;

        Ok(SpiSenderClient {
            hdl,
            osc: Reusable::new_async(kernel).await,
        })
    }
}

pub enum SpiSenderRequest {
    Send(HeapArray<u8>),
}

pub enum SpiSenderResponse {
    Sent(HeapArray<u8>),
}

pub enum SpiSenderError {
    Oops,
}

pub struct SpiSenderClient {
    hdl: KernelHandle<SpiSender>,
    osc: Reusable<Envelope<Result<SpiSenderResponse, SpiSenderError>>>,
}

impl SpiSenderClient {
    pub async fn send_wait(&mut self, data: HeapArray<u8>) -> Result<HeapArray<u8>, SpiSenderError> {
        self.hdl.send(
            SpiSenderRequest::Send(data),
            ReplyTo::OneShot(self.osc.sender().await.unwrap()),
        ).await.ok();
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

impl RegisteredDriver for SpiSender {
    type Request = SpiSenderRequest;
    type Response = SpiSenderResponse;
    type Error = SpiSenderError;

    const UUID: Uuid = uuid!("b5fd3487-08c4-4c0c-ae97-65dd1b151138");
}
