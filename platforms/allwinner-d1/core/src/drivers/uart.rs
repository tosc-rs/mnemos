use d1_pac::{CCU, GPIO, UART0};

use core::{
    ptr::{null_mut, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use crate::dmac::{
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
    drivers::simple_serial::{Request, Response, SimpleSerialError, SimpleSerialService},
    maitake::sync::WaitCell,
    mnemos_alloc::containers::Box,
    registry::Message,
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
        let mut handled_all = false;

        if !prod.is_null() {
            let prod = unsafe { &*prod };

            // Attempt to get a grant to write into...
            'read: while let Some(mut wgr) = prod.send_grant_max_sync(64) {
                // For each byte in the grant...
                for (used, b) in wgr.iter_mut().enumerate() {
                    // Check if there is NOT a data byte available...
                    if !uart0.usr.read().rfne().bit_is_set() {
                        // If not, commit the grant (with the number of used bytes),
                        // and mark that we have fully drained the FIFO.
                        wgr.commit(used);
                        handled_all = true;
                        break 'read;
                    }
                    // If there is, read it, and write it to the grant.
                    //
                    // Reading this register has the side effect of clearing the byte
                    // from the hardware fifo.
                    *b = uart0.rbr().read().rbr().bits();
                }

                // If we made it here - we've completely filled the grant.
                // Commit the entire capacity
                let len = wgr.len();
                wgr.commit(len);
            }
        }

        // If we didn't hit the "empty" case while draining, that means one of the following:
        //
        // * we have no producer
        // * We have one, and it is full
        //
        // Either way, we need to discard any bytes in the FIFO to ensure that the interrupt
        // is cleared, which won't happen until we discard at least enough bytes to drop
        // below the "threshold" level. For now: we just drain everything to make sure.
        if !handled_all {
            while uart0.usr.read().rfne().bit_is_set() {
                let _byte = uart0.rbr().read().rbr().bits();
            }
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

    async fn serial_server(handle: BidiHandle, kcons: KConsumer<Message<SimpleSerialService>>) {
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

        let (kprod, kcons) = KChannel::<Message<SimpleSerialService>>::new_async(4)
            .await
            .split();
        let (fifo_a, fifo_b) = new_bidi_channel(cap_in, cap_out).await;

        let _server_hdl = k.spawn(D1Uart::serial_server(fifo_b, kcons)).await;

        let (prod, cons) = fifo_a.split();
        let _send_hdl = k.spawn(D1Uart::sending(cons, tx_channel)).await;

        let boxed_prod = Box::new(prod).await;
        let leaked_prod = Box::into_raw(boxed_prod);
        let old = UART_RX.swap(leaked_prod, Ordering::AcqRel);
        assert_eq!(old, null_mut());

        k.with_registry(|reg| reg.register_konly::<SimpleSerialService>(&kprod))
            .await
            .map_err(drop)?;

        Ok(())
    }
}

pub unsafe fn kernel_uart(ccu: &mut CCU, gpio: &mut GPIO, uart0: UART0) -> Uart {
    // Enable UART0 clock.
    ccu.uart_bgr
        .write(|w| w.uart0_gating().pass().uart0_rst().deassert());

    // Set PB8 and PB9 to function 6, UART0, internal pullup.
    gpio.pb_cfg1
        .write(|w| w.pb8_select().uart0_tx().pb9_select().uart0_rx());
    gpio.pb_pull0
        .write(|w| w.pc8_pull().pull_up().pc9_pull().pull_up());

    // Configure UART0 for 115200 8n1.
    // By default APB1 is 24MHz, use divisor 13 for 115200.

    // UART Mode
    // No Auto Flow Control
    // No Loop Back
    // No RTS_N
    // No DTR_N
    uart0.mcr.write(|w| unsafe { w.bits(0) });

    // RCVR INT Trigger: 1 char in FIFO
    // TXMT INT Trigger: FIFO Empty
    // DMA Mode 0 - (???)
    // FIFOs Enabled
    uart0.hsk.write(|w| w.hsk().handshake());
    uart0
        .dma_req_en
        .modify(|_r, w| w.timeout_enable().set_bit());
    // uart0.fcr().write(|w| w.fifoe().set_bit().dmam().mode_1());
    uart0.fcr().write(|w| {
        w.fifoe().set_bit();
        w.dmam().mode_1();
        w.rt().half_full();
        w
    });
    uart0.ier().write(|w| {
        w.erbfi().set_bit();
        w
    });

    // TX Halted
    // Also has some DMA relevant things? Not set currently
    uart0.halt.write(|w| w.halt_tx().enabled());

    // Enable control of baudrates
    uart0.lcr.write(|w| w.dlab().divisor_latch());

    // Baudrates
    uart0.dll().write(|w| unsafe { w.dll().bits(13) });
    uart0.dlh().write(|w| unsafe { w.dlh().bits(0) });

    // Unlatch baud rate, set width
    uart0.lcr.write(|w| w.dlab().rx_buffer().dls().eight());

    // Re-enable sending
    uart0.halt.write(|w| w.halt_tx().disabled());

    Uart(uart0)
}

pub struct Uart(d1_pac::UART0);
impl core::fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        while self.0.usr.read().tfnf().bit_is_clear() {}
        for byte in s.as_bytes() {
            self.0.thr().write(|w| unsafe { w.thr().bits(*byte) });
            while self.0.usr.read().tfnf().bit_is_clear() {}
        }
        Ok(())
    }
}
