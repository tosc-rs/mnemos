#![no_std]
#![no_main]

use core::cell::UnsafeCell;
use core::ptr::{NonNull, null_mut};
use core::sync::atomic::{compiler_fence, Ordering, fence, AtomicPtr, AtomicU64};

use kernel::comms::bbq::{new_bidi_channel, SpscProducer};
use kernel::comms::kchannel::{KConsumer, KChannel};
use kernel::comms::oneshot::Reusable;
use kernel::registry::simple_serial::{SimpleSerial, Request, Response, SimpleSerialError};
use kernel::registry::{RegisteredDriver, KernelHandle, ReplyTo, Envelope};
use kernel::{self, Kernel, registry::Message};

use d1_pac::{Interrupt, TIMER, UART0, DMAC};
use d1_playground::dmac::descriptor::{
    AddressMode, BModeSel, BlockSize, DataWidth, DescriptorConfig, DestDrqType, SrcDrqType, Descriptor,
};
use d1_playground::dmac::{Dmac, ChannelMode, Channel};
use maitake::sync::Mutex;
use maitake::wait::{WaitCell, WaitQueue, Closed};
use mnemos_alloc::containers::{HeapArc, HeapArray};

use d1_playground::plic::{Plic, Priority};
use d1_playground::timer::{Timer, TimerMode, TimerPrescaler, TimerSource, Timers};

use uuid::{Uuid, uuid};

struct Uart(d1_pac::UART0);
static mut PRINTER: Option<Uart> = None;
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
fn print_raw(data: &[u8]) {
    let uart = unsafe { PRINTER.as_mut().unwrap() };
    while uart.0.usr.read().tfnf().bit_is_clear() {}
    for byte in data {
        uart.0.thr().write(|w| unsafe { w.thr().bits(*byte) });
        while uart.0.usr.read().tfnf().bit_is_clear() {}
    }
}
pub fn _print(args: core::fmt::Arguments) -> Option<()> {
    use core::fmt::Write;
    unsafe {
        PRINTER.as_mut()?.write_fmt(args).ok()
    }
}
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::_print(core::format_args!($($arg)*));
    }
}
#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {
        $crate::_print(core::format_args!($($arg)*));
        $crate::print!("\r\n");
    }
}

static TICK_MS: AtomicU64 = AtomicU64::new(0);
static TICK_WAKER: AtomicPtr<WaitCell> = AtomicPtr::new(null_mut());
static UART_TX_WAKER: AtomicPtr<WaitCell> = AtomicPtr::new(null_mut());
static UART_RX_PROD: AtomicPtr<SpscProducer> = AtomicPtr::new(null_mut());

extern "C" {
    static _aheap_start: usize;
    static _aheap_size: usize;
}

#[riscv_rt::entry]
fn main() -> ! {
    let _ = main_inner();
    loop {

    }
}

fn main_inner() -> Result<(), ()> {
    let mut p = d1_pac::Peripherals::take().ok_or(())?;

    // Enable UART0 clock.
    let ccu = &mut p.CCU;
    ccu.uart_bgr
        .write(|w| w.uart0_gating().pass().uart0_rst().deassert());

    // DMAC enable
    let dmac = Dmac::new(p.DMAC, ccu);
    dmac.dmac.dmac_irq_en_reg0.modify(|_r, w| {
        w.dma0_queue_irq_en().enabled()
    });

    // Set PC1 LED to output.
    let gpio = &p.GPIO;
    gpio.pc_cfg0
        .write(|w| w.pc1_select().output().pc0_select().ledc_do());

    // Set PB8 and PB9 to function 6, UART0, internal pullup.
    gpio.pb_cfg1
        .write(|w| w.pb8_select().uart0_tx().pb9_select().uart0_rx());
    gpio.pb_pull0
        .write(|w| w.pc8_pull().pull_up().pc9_pull().pull_up());

    // Configure UART0 for 115200 8n1.
    // By default APB1 is 24MHz, use divisor 13 for 115200.
    let uart0 = p.UART0;

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
    // uart0.hsk.write(|w| w.hsk().handshake());
    // uart0.dma_req_en.modify(|_r, w| w.timeout_enable().set_bit());
    // uart0.fcr().write(|w| w.fifoe().set_bit().dmam().mode_1());
    uart0.fcr().write(|w| {
        w.fifoe().set_bit();
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

    unsafe { PRINTER = Some(Uart(uart0)) };

    let heap_start = unsafe {
        core::ptr::addr_of!(_aheap_start) as *mut u8
    };

    let heap_size = unsafe {
        core::ptr::addr_of!(_aheap_size) as usize
    };

    println!("Bootstrapping Kernel...");
    println!("Heap Start: {:016X}", heap_start as usize);
    println!("Heap Size:  {:016X}", heap_size);

    let k_settings = kernel::KernelSettings {
        heap_start,
        heap_size,
        max_drivers: 16,
        k2u_size: 4096,
        u2k_size: 4096,
    };
    let k = unsafe {
        Kernel::new(k_settings).map_err(drop)?.leak().as_ref()
    };

    println!("Kernel configured. Waiting for initialization...");

    // Set up timers
    let Timers {
        mut timer0,
        ..
    } = Timers::new(p.TIMER);

    // timer0.set_source(TimerSource::OSC24_M);

    timer0.set_prescaler(TimerPrescaler::P8); // 24M / 8:  3.00M ticks/s
    timer0.set_mode(TimerMode::PERIODIC);
    let _ = timer0.get_and_clear_interrupt();

    unsafe {
        riscv::interrupt::enable();
        riscv::register::mie::set_mext();
    }

    // Set up interrupts
    timer0.set_interrupt_en(true);
    let plic = Plic::new(p.PLIC);

    unsafe {
        plic.set_priority(Interrupt::UART0, Priority::P1);
        plic.set_priority(Interrupt::TIMER0, Priority::P1);
        plic.set_priority(Interrupt::DMAC_NS, Priority::P1);
        plic.unmask(Interrupt::UART0);
        plic.unmask(Interrupt::TIMER0);
        plic.unmask(Interrupt::DMAC_NS);
    }

    timer0.start_counter(3_000_000 / 1_000);

    k.initialize(async {
        let hawc = TimerQueue::register(k).await?.leak();
        TICK_WAKER.store(hawc.as_ptr(), Ordering::Release);
        D1Uart::register(k, 1024, 1024).await?;

        k.spawn(async {
            let mut tq = TimerQueue::from_registry(k).await?;
            let mut ctr = 0u64;
            loop {
                tq.delay_ms(1_000).await;
                println!("[TASK 0, ct {:05}] lol. lmao.", ctr);
                ctr += 1;
            }

            #[allow(unreachable_code)]
            Result::<(), ()>::Ok(())
        }).await;

        k.spawn(async {
            let mut tq = TimerQueue::from_registry(k).await?;
            let mut ctr = 0u64;
            loop {
                tq.delay_ms(3_000).await;
                println!("[TASK 1, ct {:05}] beep, boop.", ctr);
                ctr += 1;
            }

            #[allow(unreachable_code)]
            Result::<(), ()>::Ok(())
        }).await;

        k.spawn(async {
            let mut serial = SimpleSerial::from_registry(k).await?;
            let ser_bidi = serial.get_port().await?;

            loop {
                let rgr = ser_bidi.consumer().read_grant().await;
                let rlen = rgr.len();
                let mut wgr = ser_bidi.producer().send_grant_exact(rlen).await;
                wgr.copy_from_slice(&rgr);
                rgr.release(rlen);
                wgr.commit(rlen);
            }

            #[allow(unreachable_code)]
            Option::<()>::None
        }).await;

        Result::<(), ()>::Ok(())
    })?;

    println!("Initalized. Starting Run Loop.");

    loop {
        k.tick();
        unsafe { riscv::asm::wfi() };
    }

    // let thr_addr = unsafe { &*UART0::PTR }.thr() as *const _ as *mut ();

    // for chunk in HOUND.lines() {
    //     let d_cfg = DescriptorConfig {
    //         source: chunk.as_ptr().cast(),
    //         destination: thr_addr,
    //         byte_counter: chunk.len(),
    //         link: None,
    //         wait_clock_cycles: 0,
    //         bmode: BModeSel::Normal,
    //         dest_width: DataWidth::Bit8,
    //         dest_addr_mode: AddressMode::IoMode,
    //         dest_block_size: BlockSize::Byte1,
    //         dest_drq_type: DestDrqType::Uart0Tx,
    //         src_data_width: DataWidth::Bit8,
    //         src_addr_mode: AddressMode::LinearMode,
    //         src_block_size: BlockSize::Byte1,
    //         src_drq_type: SrcDrqType::Dram,
    //     };
    //     let descriptor = d_cfg.try_into().unwrap();
    //     unsafe {
    //         dmac.channels[0].set_channel_modes(ChannelMode::Wait, ChannelMode::Handshake);
    //         dmac.channels[0].start_descriptor(NonNull::from(&descriptor));
    //     }

    //     timer0.start_counter(1_500_000);
    //     unsafe { riscv::asm::wfi() };

    //     println!("");

    //     unsafe {
    //         dmac.channels[0].stop_dma();
    //     }
    // }
}

#[export_name = "MachineExternal"]
fn im_an_interrupt() {
    let plic = unsafe { Plic::summon() };
    let timer = unsafe { &*TIMER::PTR };
    let uart0 = unsafe { &*UART0::PTR };
    let dmac = unsafe { &*DMAC::PTR };

    let claim = plic.claim();
    // println!("claim: {}", claim.bits());

    match claim {
        Interrupt::TIMER0 => {
            TICK_MS.fetch_add(1, Ordering::AcqRel);
            timer
                .tmr_irq_sta
                .modify(|_r, w| w.tmr0_irq_pend().set_bit());
            let ptr = TICK_WAKER.load(Ordering::Acquire);

            if !ptr.is_null() {
                unsafe {
                    (&*ptr).wake();
                }
            }

            // Wait for the interrupt to clear to avoid repeat interrupts
            while timer.tmr_irq_sta.read().tmr0_irq_pend().bit_is_set() {}
        }
        Interrupt::UART0 => {
            // println!("UART0 INT");
            let prod = UART_RX_PROD.load(Ordering::Acquire);
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
                        },
                        Err(used) => {
                            wgr.commit(used);
                            break;
                        },
                    }
                }
            }

            // We've processed all possible bytes. Discard any remaining.
            while uart0.usr.read().rfne().bit_is_set() {
                let _byte = uart0.rbr().read().rbr().bits();
            }
        }
        Interrupt::DMAC_NS => {
            // println!("DMAC INT");
            dmac.dmac_irq_pend_reg0.modify(|r, w| {
                if r.dma0_queue_irq_pend().bit_is_set() {
                    let waker = UART_TX_WAKER.load(Ordering::Acquire);
                    if !waker.is_null() {
                        unsafe {
                            (&*waker).wake();
                        }
                    }
                }
                // Will write-back and high bits
                w
            })
        }
        x => {
            println!("Unexpected claim: {:?}", x);
            panic!();
        }
    }

    // Release claim
    plic.complete(claim);
}

// Main config register:
// DMAC_CFG_REGN
// Mode:
// DMAC_MODE_REGN

pub struct D1Uart {
    _x: (),
}

impl D1Uart {
    pub async fn register(kernel: &'static Kernel, cap_in: usize, cap_out: usize) -> Result<(), ()> {
        let (kprod, kcons) = KChannel::new_async(kernel, 4).await.split();
        let (a_ring, b_ring) = new_bidi_channel(kernel.heap(), cap_in, cap_out).await;
        let tx_wake = kernel.heap().allocate_arc(WaitCell::new()).await;

        // Message request handler
        kernel.spawn(async move {
            let handle = b_ring;

            let req: Message<SimpleSerial> = kcons.dequeue_async().await.map_err(drop)?;
            let Request::GetPort = req.msg.body;

            let resp = req.msg.reply_with(Ok(Response::PortHandle { handle }));

            req.reply.reply_konly(resp).await.map_err(drop)?;

            // And deny all further requests after the first
            loop {
                let req = kcons.dequeue_async().await.map_err(drop)?;
                let Request::GetPort = req.msg.body;
                let resp = req
                    .msg
                    .reply_with(Err(SimpleSerialError::AlreadyAssignedPort));
                req.reply.reply_konly(resp).await.map_err(drop)?;
            }

            #[allow(unreachable_code)]
            Result::<(), ()>::Ok(())
        }).await;

        let (prod, cons) = a_ring.split();

        // Sender task
        let sender_wake = tx_wake.clone();
        UART_TX_WAKER.store(tx_wake.leak().as_ptr(), Ordering::Release);
        kernel.spawn(async move {
            let thr_addr = unsafe { &*UART0::PTR }.thr() as *const _ as *mut ();

            loop {
                let read = cons.read_grant().await;

                let d_cfg = DescriptorConfig {
                    source: read.as_ptr().cast(),
                    destination: thr_addr,
                    byte_counter: read.len(),
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
                let descriptor = d_cfg.try_into().map_err(drop)?;
                unsafe {
                    let mut chan = Channel::summon_channel(0);
                    chan.set_channel_modes(ChannelMode::Wait, ChannelMode::Handshake);
                    chan.start_descriptor(NonNull::from(&descriptor));
                }
                match sender_wake.wait().await {
                    Ok(_) => {},
                    Err(Closed) => todo!(),
                }
                let len = read.len();
                read.release(len);
            }

            #[allow(unreachable_code)]
            Result::<(), ()>::Ok(())
        }).await;

        // Receiver task
        // TODO: This Arc shouldn't be necessary
        // The reception is handled in the interrupt itself.
        let prod = kernel.heap().allocate_arc(prod).await;
        UART_RX_PROD.store(prod.leak().as_ptr(), Ordering::Release);

        kernel.with_registry(|reg| {
            reg.register_konly::<SimpleSerial>(&kprod)
        }).await.map_err(drop)?;

        Ok(())
    }
}

// ----

pub struct TimerQueue {
    _x: (),
}

pub struct TQPusher {
    arr: HeapArc<Mutex<HeapArray<Option<(u64, Message<TimerQueue>)>>>>,
    chan: KConsumer<Message<TimerQueue>>,
}

pub struct TQClient {
    hdl: KernelHandle<TimerQueue>,
    osc: Reusable<Envelope<Result<TQResponse, TQError>>>,
}

impl TQClient {
    pub async fn delay_ms(&mut self, ms: u64) {
        self.hdl.send(
            TQRequest::DelayMs(ms),
            ReplyTo::OneShot(self.osc.sender().unwrap()),
        ).await.ok();
        self.osc.receive().await.ok();
    }
}

impl TQPusher {
    pub async fn run(&mut self) -> Result<(), ()> {
        loop {
            match self.chan.dequeue_async().await {
                Ok(msg) => {
                    let now = TICK_MS.load(Ordering::Acquire);
                    let mut guard = self.arr.lock().await;
                    let space = guard.iter_mut().find(|i| i.is_none()).ok_or(())?;
                    let TQRequest::DelayMs(ms) = &msg.msg.body;
                    let end = ms.wrapping_add(now);
                    *space = Some((end, msg));
                },
                Err(_) => panic!(),
            }
        }
    }
}

pub struct TQPopper {
    arr: HeapArc<Mutex<HeapArray<Option<(u64, Message<TimerQueue>)>>>>,
    wait: HeapArc<WaitCell>,
}

impl TQPopper {
    pub async fn run(&mut self) -> Result<(), ()> {
        loop {
            self.wait.wait().await.map_err(drop)?;
            let mut guard = self.arr.lock().await;
            let now = TICK_MS.load(Ordering::Acquire);

            // lol. lmao.
            for item in guard.iter_mut() {
                match item.take() {
                    Some((time, msg)) => {
                        if time <= now {
                            let resp = msg.msg.reply_with(Ok(TQResponse::Delayed { now }));
                            msg.reply.reply_konly(resp).await.map_err(drop)?;
                        } else {
                            *item = Some((time, msg));
                        }
                    },
                    None => {},
                }
            }
        }
    }
}

impl TimerQueue {
    pub async fn register(kernel: &'static Kernel) -> Result<HeapArc<WaitCell>, ()> {
        let wait = kernel.heap().allocate_arc(WaitCell::new()).await;
        let (kprod, kcons) = KChannel::new_async(kernel, 32).await.split();
        let arr = kernel.heap().allocate_array_with(|| None, 128).await;
        let arr = kernel.heap().allocate_arc(Mutex::new(arr)).await;

        let mut push = TQPusher {
            arr: arr.clone(),
            chan: kcons,
        };

        let mut pop = TQPopper {
            arr,
            wait: wait.clone(),
        };

        kernel.spawn(async move {
            let _ = push.run().await;
        }).await;

        kernel.spawn(async move {
            let _ = pop.run().await;
        }).await;

        kernel.with_registry(move |reg| {
            reg.register_konly(&kprod).map_err(drop)
        }).await?;

        Ok(wait)
    }

    pub async fn from_registry(kernel: &'static Kernel) -> Result<TQClient, ()> {
        let hdl = kernel.with_registry(|reg| {
            reg.get()
        }).await.ok_or(())?;

        Ok(TQClient {
            hdl,
            osc: Reusable::new_async(kernel).await,
        })
    }
}

pub enum TQRequest {
    DelayMs(u64),
}

pub enum TQResponse {
    Delayed {
        now: u64,
    },
}

pub enum TQError {
    Oops,
}

impl RegisteredDriver for TimerQueue {
    type Request = TQRequest;
    type Response = TQResponse;
    type Error = TQError;

    const UUID: Uuid = uuid!("74a06fee-485b-427a-b965-e19a6c62dc60");
}

use core::panic::PanicInfo;

#[panic_handler]
fn handler(_info: &PanicInfo) -> ! {
    // println!("");
    // println!("PANIC HAS HAPPENED!");
    // println!("{:?}", info.payload());
    // println!("{:?}", info.location());
    loop {
        fence(Ordering::SeqCst);
    }
}
