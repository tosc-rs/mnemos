use core::{
    ptr::{null_mut, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use d1_pac::UART0;
use drivers::dmac::{
    descriptor::{
        AddressMode, BModeSel, BlockSize, DataWidth, DescriptorConfig, DestDrqType, SrcDrqType,
    },
    Channel, ChannelMode,
};
use kernel::{
    comms::{
        bbq::{new_bidi_channel, BidiHandle, Consumer, GrantW, SpscProducer},
        kchannel::{KChannel, KConsumer},
    },
    maitake::sync::WaitCell,
    registry::{
        simple_serial::{Request, Response, SimpleSerial, SimpleSerialError},
        Message,
    },
    Kernel,
};

struct GrantWriter {
    grant: GrantW,
    used: usize,
}

impl core::fmt::Write for GrantWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let glen = self.grant.len();
        let slen = s.len();
        let new_used = self.used + slen;
        if new_used <= glen {
            self.grant[self.used..][..slen].copy_from_slice(s.as_bytes());
            self.used = new_used;
            Ok(())
        } else {
            Err(core::fmt::Error)
        }
    }
}

static TX_DONE: WaitCell = WaitCell::new();
static UART_RX: AtomicPtr<SpscProducer> = AtomicPtr::new(null_mut());

pub struct D1Uart {
    _x: (),
}

impl D1Uart {
    pub fn tx_done_waker() -> &'static WaitCell {
        &TX_DONE
    }

    pub fn handle_uart0_int() {
        let uart0 = unsafe { &*UART0::PTR };
        let prod = UART_RX.load(Ordering::Acquire);
        if !prod.is_null() {
            let prod = unsafe { &*prod };

            while let Some(mut wgr) = prod.send_grant_max_sync(64) {
                let used_res = wgr.iter_mut().enumerate().try_for_each(|(i, b)| {
                    if uart0.usr.read().rfne().bit_is_set() {
                        *b = uart0.rbr().read().rbr().bits();
                        Ok(())
                    } else {
                        Err(i)
                    }
                });

                match used_res {
                    Ok(()) => {
                        let len = wgr.len();
                        wgr.commit(len);
                    }
                    Err(used) => {
                        wgr.commit(used);
                        break;
                    }
                }
            }
        }

        // We've processed all possible bytes. Discard any remaining.
        while uart0.usr.read().rfne().bit_is_set() {
            let _byte = uart0.rbr().read().rbr().bits();
        }
    }

    // Send loop that listens to the bbqueue consumer, and sends it as DMA transactions on the UART
    async fn sending(cons: Consumer, mut tx_channel: Channel) {
        loop {
            let rx = cons.read_grant().await;
            let len = rx.len();
            let thr_addr = unsafe { &*UART0::PTR }.thr() as *const _ as *mut ();

            let rx_sli: &[u8] = &rx;

            let d_cfg = DescriptorConfig {
                source: rx_sli.as_ptr().cast(),
                destination: thr_addr,
                byte_counter: rx_sli.len(),
                link: None,
                wait_clock_cycles: 0,
                bmode: BModeSel::Normal,
                dest_width: DataWidth::Bit8,
                dest_addr_mode: AddressMode::IoMode,
                dest_block_size: BlockSize::Byte1,
                dest_drq_type: DestDrqType::Uart0Tx,
                src_data_width: DataWidth::Bit8,
                src_addr_mode: AddressMode::LinearMode,
                src_block_size: BlockSize::Byte1,
                src_drq_type: SrcDrqType::Dram,
            };
            let descriptor = d_cfg.try_into().unwrap();
            unsafe {
                tx_channel.set_channel_modes(ChannelMode::Wait, ChannelMode::Handshake);
                tx_channel.start_descriptor(NonNull::from(&descriptor));
            }
            let _ = TX_DONE.wait().await;
            unsafe {
                tx_channel.stop_dma();
            }
            rx.release(len);
        }
    }

    async fn serial_server(handle: BidiHandle, kcons: KConsumer<Message<SimpleSerial>>) {
        loop {
            if let Ok(req) = kcons.dequeue_async().await {
                let Request::GetPort = req.msg.body;
                let resp = req.msg.reply_with(Ok(Response::PortHandle { handle }));
                let _ = req.reply.reply_konly(resp).await;
                break;
            }
        }

        // And deny all further requests after the first
        loop {
            if let Ok(req) = kcons.dequeue_async().await {
                let Request::GetPort = req.msg.body;
                let resp = req
                    .msg
                    .reply_with(Err(SimpleSerialError::AlreadyAssignedPort));
                let _ = req.reply.reply_konly(resp).await;
            }
        }
    }

    pub async fn register(
        k: &'static Kernel,
        cap_in: usize,
        cap_out: usize,
        tx_channel: Channel,
    ) -> Result<(), ()> {
        assert_eq!(tx_channel.channel_index(), 0);

        let (kprod, kcons) = KChannel::<Message<SimpleSerial>>::new_async(k, 4)
            .await
            .split();
        let (fifo_a, fifo_b) = new_bidi_channel(k.heap(), cap_in, cap_out).await;

        let _server_hdl = k.spawn(D1Uart::serial_server(fifo_b, kcons)).await;

        let (prod, cons) = fifo_a.split();
        let _send_hdl = k.spawn(D1Uart::sending(cons, tx_channel)).await;

        let boxed_prod = k.heap().allocate(prod).await;
        let leaked_prod = boxed_prod.leak();
        UART_RX.store(leaked_prod.as_ptr(), Ordering::Release);

        k.with_registry(|reg| reg.register_konly::<SimpleSerial>(&kprod))
            .await
            .map_err(drop)?;

        Ok(())
    }
}
