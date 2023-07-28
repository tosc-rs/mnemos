use core::{
    ptr::{null_mut, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use esp32c3_hal::{uart::{Uart, Instance}, interrupt, prelude::interrupt};

use kernel::{
    comms::{
        bbq::{new_bidi_channel, BidiHandle, Consumer, GrantW, SpscProducer},
        kchannel::{KChannel, KConsumer},
    },
    maitake::sync::WaitCell,
    mnemos_alloc::containers::Box,
    registry::{self, Message},
    services::simple_serial::{Request, Response, SimpleSerialError, SimpleSerialService},
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

static UART0_TX_DONE: WaitCell = WaitCell::new();
static UART_RX: AtomicPtr<SpscProducer> = AtomicPtr::new(null_mut());

pub struct C3Uart<T> {
    uart: Uart<Uart0>,
    tx_done: &'static WaitCell,
}

impl C3Uart<Uart0> {
    pub fn uart0(uart: Uart<'static, Uart0>) -> Self {
        Self { uart, tx_done: UART0_TX_DONE }
    }

    pub fn handle_uart0_int() {
        UART0_TX_DONE.wake();
    }
}

impl<T: Instance> C3Uart<T> {
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

    async fn sending(&mut self, handle: BidiHandle, kcons: KConsumer<Message<SimpleSerialService>>) {
        loop {
            let rx = cons.read_grant().await;
            let len = rx.len();

            // pre-register wait future to ensure the waker is in place before
            // starting thewrite.
            let wait = self.tx_done.subscribe().await;

            self.uart.listen_tx_done();
            // TODO(eliza): what should we do if this errors?
            self.uart.write_bytes(&rx).expect("UART write bytes should succeed?");

            // wait for the write to complete
            wait.await.expect("UART TX_DONE WaitCell is never closed!");

            rx.release(len);
        }
    }

    pub async fn register(
        self,
        k: &'static Kernel,
        cap_in: usize,
        cap_out: usize,
    ) -> Result<(), registry::RegistrationError> {

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
            .await?;

        Ok(())
    }
}

#[interrupt]
fn UART0() {
    let uart = unsafe { Uart0::steal() };

    if uart.int_raw.read().tx_done_int_raw().bit_is_set() {
        uart.int_clr
        .write(|w| w.tx_done_int_clr().set_bit());
        UART0_TX_DONE.wake();
    } else {
        panic!("unexpected UART interrupt!");
    }

}