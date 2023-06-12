#![no_std]
#![no_main]

use core::{panic::PanicInfo, time::Duration, fmt::Write, sync::atomic::{AtomicUsize, Ordering}, ptr::NonNull};
use d1_pac::{Interrupt, TIMER, UART0, CCU, GPIO, DMAC};
use drivers::{Ram, uart::{kernel_uart, Uart}, timer::{Timers, Timer, TimerMode, TimerPrescaler}, plic::{Priority, Plic}, dmac::{Channel, descriptor::{BModeSel, DataWidth, AddressMode, BlockSize, DestDrqType, SrcDrqType, DescriptorConfig}, ChannelMode, Dmac}};
use kernel::{Kernel, KernelSettings, comms::bbq::{BidiHandle, new_bidi_channel}, maitake::sync::WaitCell};

const HEAP_SIZE: usize = 384 * 1024 * 1024;

#[link_section = ".aheap.AHEAP"]
#[used]
static AHEAP: Ram<HEAP_SIZE> = Ram::new();

static WFI_CT: AtomicUsize = AtomicUsize::new(0);

#[allow(non_snake_case)]
#[riscv_rt::entry]
fn main() -> ! {
    let mut p = unsafe { d1_pac::Peripherals::steal() };
    // let mut _uart = unsafe { kernel_uart(&mut p.CCU, &mut p.GPIO, p.UART0) };
    setup_uart(&mut p.CCU, &mut p.GPIO, p.UART0);

    p.GPIO.pc_cfg0.modify(|_r, w| {
        w.pc1_select().output();
        w
    });
    p.GPIO.pc_dat.modify(|_r, w| {
        w.pc_dat().variant(0b0000_0010);
        w
    });


    // Timer0 is used as a freewheeling rolling timer.
    // Timer1 is used to generate "sleep until" interrupts
    //
    // Both are at a time base of 3M ticks/s.
    //
    // In the future, we probably want to rework this to use the RTC timer for
    // both purposes, as this will likely play better with sleep power usage.
    let Timers { mut timer0, mut timer1 } = Timers::new(p.TIMER);

    let k = initialize_kernel().unwrap();
    let dmac = Dmac::new(p.DMAC, &mut p.CCU);
    let [ch0, ch1, ..] = dmac.channels;
    dmac.dmac.dmac_irq_en0.modify(|_r, w| {
        w.dma0_queue_irq_en().enabled();
        w
    });

    k.initialize(async move {
        loop {
            p.GPIO.pc_dat.modify(|_r, w| {
                w.pc_dat().variant(0b0000_0010);
                w
            });
            k.sleep(Duration::from_millis(250)).await;
            p.GPIO.pc_dat.modify(|_r, w| {
                w.pc_dat().variant(0b0000_0000);
                w
            });
            k.sleep(Duration::from_millis(250)).await;
        }
    }).unwrap();
    k.initialize(do_uart(k, ch0, ch1)).unwrap();

    timer0.set_prescaler(TimerPrescaler::P8); // 24M / 8:  3.00M ticks/s
    timer1.set_prescaler(TimerPrescaler::P8);
    timer0.set_mode(TimerMode::PERIODIC);
    timer1.set_mode(TimerMode::SINGLE_COUNTING);
    let _ = timer0.get_and_clear_interrupt();
    let _ = timer1.get_and_clear_interrupt();

    unsafe {
        riscv::interrupt::enable();
        riscv::register::mie::set_mext();
    }


    let plic = Plic::new(p.PLIC);

    unsafe {
        plic.register(Interrupt::TIMER1, timer1_int);
        plic.register(Interrupt::DMAC_NS, handle_dmac);
        plic.activate(Interrupt::DMAC_NS, Priority::P1).unwrap();
    }

    timer0.start_counter(0xFFFF_FFFF);

    loop {
        // Tick the scheduler
        let start = timer0.current_value();
        let tick = k.tick();

        // Timer is downcounting
        let elapsed = start.wrapping_sub(timer0.current_value());
        let turn = k.timer().force_advance_ticks(elapsed.into());

        // If there is nothing else scheduled, sleep for some amount of time
        if !tick.has_remaining {
            let wfi_start = timer0.current_value();

            // TODO(AJM): Sometimes there is no "next" in the timer wheel, even though there should
            // be. Don't take lack of timer wheel presence as the ONLY heuristic of whether we
            // should just wait for SOME interrupt to occur. For now, force a max sleep of 100ms
            // which is still probably wrong.
            let amount = turn.ticks_to_next_deadline()
                .unwrap_or(100 * 1000 * 3); // 3 ticks per us, 1000 us per ms, 100ms sleep

            // Don't sleep for too long until james figures out wrapping timers
            let amount = amount.min(0x4000_0000) as u32;
            let _ = timer1.get_and_clear_interrupt();
            unsafe {
                plic.activate(Interrupt::TIMER1, Priority::P1).unwrap();
            }
            timer1.set_interrupt_en(true);
            timer1.start_counter(amount);

            unsafe {
                WFI_CT.fetch_add(1, Ordering::Relaxed);
                riscv::asm::wfi();
            }
            // Disable the timer interrupt in case that wasn't what woke us up
            plic.deactivate(Interrupt::TIMER1).unwrap();
            timer1.set_interrupt_en(false);
            timer1.stop();

            // Account for time slept
            let elapsed = wfi_start.wrapping_sub(timer0.current_value());
            let _turn = k.timer().force_advance_ticks(elapsed.into());
        }
    }
}

// We don't actually do anything in the TIMER1 interrupt. It is only here to
// knock us out of WFI. Just disable the IRQ to prevent refires
fn timer1_int() {
    let timer = unsafe { &*TIMER::PTR };
    timer
        .tmr_irq_sta
        .modify(|_r, w| w.tmr1_irq_pend().set_bit());

    // Wait for the interrupt to clear to avoid repeat interrupts
    while timer.tmr_irq_sta.read().tmr1_irq_pend().bit_is_set() {}
}

fn initialize_kernel() -> Result<&'static Kernel, ()> {
    let k_settings = KernelSettings {
        heap_start: AHEAP.as_ptr(),
        heap_size: HEAP_SIZE,
        max_drivers: 16,
        k2u_size: 4096,
        u2k_size: 4096,
        timer_granularity: Duration::from_nanos(333),
    };
    let k = unsafe {
        Kernel::new(k_settings).map_err(drop)?.leak().as_ref()
    };
    Ok(k)
}

#[panic_handler]
fn handler(info: &PanicInfo) -> ! {
    // Ugly but works
    let mut uart: Uart = unsafe { core::mem::transmute(()) };

    write!(&mut uart, "\r\n").ok();
    write!(&mut uart, "{}\r\n", info).ok();

    loop {
        core::sync::atomic::fence(Ordering::SeqCst);
    }
}

static RX_DONE: WaitCell = WaitCell::new();
static TX_DONE: WaitCell = WaitCell::new();

fn handle_dmac() {

    let dmac = unsafe { &*DMAC::PTR };
    // println!("DMAC INT");
    dmac.dmac_irq_pend0.modify(|r, w| {
        if r.dma0_queue_irq_pend().bit_is_set() {
            TX_DONE.wake();
        }

        if r.dma1_queue_irq_pend().bit_is_set() {
            panic!("HITTA");
            // // println!("SPI WAKE");
            // let waker = SPI1_TX_WAKER.load(Ordering::Acquire);
            // if !waker.is_null() {
            //     unsafe {
            //         (&*waker).wake();
            //     }
            // } else {                // TODO: LOAD BEARING UB
            //     panic!("HEH");      // TODO: LOAD BEARING UB
            // }                       // TODO: LOAD BEARING UB
        }
        // Will write-back and high bits
        w
    });
}

async fn do_uart(
    k: &'static Kernel,
    mut tx_channel: Channel,
    mut rx_channel: Channel,
) {
    let (fifo_a, fifo_b) = new_bidi_channel(k.heap(), 4096, 4096).await;

    // This is the sw side
    let _jhsw = k.spawn(async move {
        loop {
            // let rx = fifo_b.consumer().read_grant().await;
            let rx = b"Hello, this is a message of reasonable length\r\n";
            let all_len = rx.len();
            let mut all = &rx[..];
            while !all.is_empty() {
                let mut wgr = fifo_b.producer().send_grant_max(all.len()).await;
                let len = all.len().min(wgr.len());
                wgr[..len].copy_from_slice(all);
                wgr.commit(len);
                all = &all[len..];
            }
            // rx.release(all_len);
            //
            // temp
            k.sleep(Duration::from_secs(3)).await;
        }
    }).await;

    let (prod, cons) = fifo_a.split();
    let _send_hdl = k.spawn(async move {
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
    }).await;
}

// James move this to the uart driver
fn setup_uart(
    ccu: &mut CCU,
    gpio: &mut GPIO,
    uart0: UART0,
) {
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
}
