#![no_main]
#![no_std]

use core::{arch::asm, sync::atomic::{AtomicU32, compiler_fence, AtomicBool}};
use cortex_m::{
    asm::isb,
    register::{control, psp},
};
use groundhog::RollingTimer;
use groundhog_nrf52::GlobalRollingTimer;
use kernel::monotonic::ExtU32;

#[allow(unused_imports)]
use kernel::{
    alloc::{HeapGuard, HEAP},
    drivers::{
        gd25q16::Gd25q16,
        nrf52_spim_nonblocking::{new_send_fut, Spim},
        usb_serial::{enable_usb_interrupts, setup_usb_uart, UsbUartIsr, UsbUartParts},
        vs1053b::Vs1053b,
        nrf52_rng::HWRng,
    },
    loader::validate_header,
    map_pins,
    monotonic::MonoTimer,
    qspi::{Qspi, QspiPins},
    syscall::{syscall_clear, try_recv_syscall},
    traits::{BlockStorage, Machine, Serial, SpiHandle, SpiTransactionKind, SpimNode, Spi, PcmSink},
    MAGIC_BOOT,
    DRIVER_QUEUE,
    DriverCommand,
};
use nrf52840_hal::{
    clocks::{ExternalOscillator, Internal, LfOscStopped},
    gpio::{Floating, Input, Level, Output, Pin, Port, PushPull},
    gpiote::Gpiote,
    pac::{GPIOTE, P0, P1, PPI, SPIM0, TIMER0, SCB, TIMER2},
    ppi::{ConfigurablePpi, Parts, Ppi, Ppi0},
    prelude::OutputPin,
    rng::Rng,
    usbd::{UsbPeripheral, Usbd},
    timer::Instance as _,
    Clocks,
};
use usb_device::{
    class_prelude::UsbBusAllocator,
    device::{UsbDeviceBuilder, UsbVidPid},
};
use usbd_serial::{SerialPort, USB_CLASS_CDC};

static DEFAULT_IMAGE: &[u8] = include_bytes!("../appbins/app-loader.bin");
static IDLE_TICKS: AtomicU32 = AtomicU32::new(0);
static SYSCALLS: AtomicU32 = AtomicU32::new(0);
static SNAP: AtomicBool = AtomicBool::new(false);

#[rtic::app(device = nrf52840_hal::pac, dispatchers = [SWI0_EGU0])]
mod app {
    use core::sync::atomic::Ordering;
    use kernel::traits::RandFill;

    use super::*;

    #[monotonic(binds = TIMER0, default = true)]
    type Monotonic = MonoTimer<TIMER0>;

    #[shared]
    struct Shared {
        machine: Machine,
    }

    #[local]
    struct Local {
        usb_isr: UsbUartIsr,
        prog_loaded: Option<(*const u8, usize)>,
        heap: HeapGuard,
        timer: TIMER2,
    }

    type UsbBusAlloc = UsbBusAllocator<Usbd<UsbPeripheral<'static>>>;
    type Clock = Clocks<ExternalOscillator, Internal, LfOscStopped>;

    #[init(local = [
        clocks: Option<Clock> = None,
        usb_bus: Option<UsbBusAlloc> = None,
    ])]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        let device = cx.device;

        // Setup clocks early in the process. We need this for USB later
        let clocks = Clocks::new(device.CLOCK);
        let clocks = clocks.enable_ext_hfosc();
        let clocks = cx.local.clocks.insert(clocks);

        // Enable instruction caches for MAXIMUM SPEED
        device.NVMC.icachecnf.write(|w| w.cacheen().set_bit());
        isb();

        // Configure the monotonic timer, currently using TIMER0, a 32-bit, 1MHz timer
        let mono = Monotonic::new(device.TIMER0);

        // I am annoying, and prefer my own libraries.
        GlobalRollingTimer::init(device.TIMER1);

        let timer = device.TIMER2;

        // Setup the heap
        let mut heap_guard = HEAP.init_exclusive().unwrap();

        // Reset the syscall contents
        syscall_clear();

        // Before we give away the USB peripheral, enable the relevant interrupts
        enable_usb_interrupts(&device.USBD);

        let usb_bus = Usbd::new(UsbPeripheral::new(device.USBD, clocks));
        let usb_bus = cx.local.usb_bus.insert(usb_bus);

        let usb_serial = SerialPort::new(usb_bus);
        let usb_dev = UsbDeviceBuilder::new(usb_bus, UsbVidPid(0x16c0, 0x27dd))
            .manufacturer("OVAR Labs")
            .product("Anachro Pellegrino")
            // TODO: Use some kind of unique ID. This will probably require another singleton,
            // as the storage must be static. Probably heapless::String -> singleton!()
            .serial_number("ajm001")
            .device_class(USB_CLASS_CDC)
            .max_packet_size_0(64) // (makes control transfers 8x faster)
            .build();

        let UsbUartParts { isr, sys } = defmt::unwrap!(setup_usb_uart(usb_dev, usb_serial));

        let pins = map_pins(device.P0, device.P1);
        let qsp = QspiPins {
            qspi_copi_io0: pins.qspi_d0.degrade(),
            qspi_cipo_io1: pins.qspi_d1.degrade(),
            qspi_io2: pins.qspi_d2.degrade(),
            qspi_io3: pins.qspi_d3.degrade(),
            qspi_csn: pins.qspi_csn.degrade(),
            qspi_sck: pins.qspi_sck.degrade(),
        };
        let qspi = Qspi::new(device.QSPI, qsp);
        let mut block = defmt::unwrap!(Gd25q16::new(qspi, &mut heap_guard));

        let prog_loaded = if let Some(blk) = MAGIC_BOOT.read_clear() {
            unsafe {
                extern "C" {
                    static _app_start: u32;
                    static _app_len: u32;
                }
                defmt::println!("Told to boot block {=u32}!", blk);
                let app_start = (&_app_start) as *const u32 as *const u8 as *mut u8;
                let app_len = (&_app_len) as *const u32 as usize;
                block.block_load_to(blk, app_start, app_len).ok()
            }
        } else {
            None
        };

        let to_uart: &'static mut dyn Serial =
            defmt::unwrap!(heap_guard.leak_send(sys).map_err(drop));
        let to_block: &'static mut dyn BlockStorage =
            defmt::unwrap!(heap_guard.leak_send(block).map_err(drop));

        //
        // Map GPIO pins
        //

        // DREQ
        let d05 = pins.d05.degrade().into_floating_input();
        let d11 = pins.d11.degrade().into_push_pull_output(Level::High);
        let d06 = pins.d06.degrade().into_push_pull_output(Level::High);

        let command_pin = d11;
        let data_pin = d06;
        let dreq = d05;

        let gpiote = Gpiote::new(device.GPIOTE);
        let ppi = Parts::new(device.PPI);
        let ppi0 = ppi.ppi0;
        let (cmd_node, data_node) =
            crate::make_nodes(dreq, command_pin, data_pin, ppi0, gpiote, &device.SPIM0);

        let cmd_node: &'static mut dyn SpimNode =
            heap_guard.leak_send(cmd_node).map_err(drop).unwrap();
        let data_node: &'static mut dyn SpimNode =
            heap_guard.leak_send(data_node).map_err(drop).unwrap();

        let spi = Spim::new(
            device.SPIM0,
            kernel::drivers::nrf52_spim_nonblocking::Pins {
                sck: pins.sclk.into_push_pull_output(Level::Low).degrade(),
                mosi: Some(pins.mosi.into_push_pull_output(Level::Low).degrade()),
                miso: Some(pins.miso.into_floating_input().degrade()),
            },
            embedded_hal::spi::MODE_0,
        );

        let spi: &'static mut dyn Spi = heap_guard.leak_send(spi).map_err(drop).unwrap();

        let cmd_hdl = spi.register_handle(cmd_node).map_err(drop).unwrap();
        let data_hdl = spi.register_handle(data_node).map_err(drop).unwrap();

        let vs1053b = Vs1053b::from_handles(cmd_hdl, data_hdl);
        let vs1053b: &'static mut dyn PcmSink = heap_guard.leak_send(vs1053b).map_err(drop).unwrap();

        let rng = Rng::new(device.RNG);
        let hwrng = HWRng::new(rng);
        let rand: &'static mut dyn RandFill = heap_guard.leak_send(hwrng).map_err(drop).unwrap();

        let machine = Machine {
            serial: to_uart,
            block_storage: Some(to_block),
            spi: Some(spi),
            pcm: Some(vs1053b),
            gpios: &mut [],
            rand: Some(rand),
        };

        ticky::spawn_after(1000u32.millis()).ok();

        (
            Shared {
                machine,
            },
            Local {
                usb_isr: isr,
                prog_loaded,
                heap: heap_guard,
                timer,
            },
            init::Monotonics(mono),
        )
    }

    #[task(binds = TIMER2, priority = 5)]
    fn timer_stub(_cx: timer_stub::Context) {
        unsafe {
            let timer = &*TIMER2::ptr();
            timer.events_compare[0].write(|w| w);
        }
        SNAP.store(true, Ordering::Release);
    }

    #[task(binds = USBD, local = [usb_isr], priority = 4)]
    fn usb_tick(cx: usb_tick::Context) {
        cx.local.usb_isr.poll();
    }

    #[task(binds = GPIOTE, priority = 3)]
    fn gpiote(_cx: gpiote::Context) {
        // TODO: NOT this
        let gpiote = unsafe { &*GPIOTE::ptr() };

        // Clear channel 1 events
        gpiote.events_in[1].write(|w| w);

        DRIVER_QUEUE.enqueue(DriverCommand::SpiStart).map_err(drop).unwrap();
        SCB::set_pendsv();
    }

    #[task(binds = SPIM0_SPIS0_TWIM0_TWIS0_SPI0_TWI0, priority = 3)]
    fn spim0(_cx: spim0::Context) {
        // TODO: NOT this
        let gpiote = unsafe { &*GPIOTE::ptr() };

        // Clear channel 0 events (which probably stopped our SPI device)
        gpiote.events_in[0].write(|w| w);

        let spim0 = unsafe { &*SPIM0::ptr() };
        spim0.events_stopped.reset();
        spim0.events_end.reset();

        // defmt::println!("[INT]: SPIM0");
        DRIVER_QUEUE.enqueue(DriverCommand::SpiEnd).map_err(drop).unwrap();
        SCB::set_pendsv();
    }

    #[task(binds = SVCall, local = [heap], shared = [machine], priority = 2)]
    fn svc(cx: svc::Context) {
        SYSCALLS.fetch_add(1, Ordering::Release);
        let mut machine = cx.shared.machine;
        if let Ok(()) = try_recv_syscall(|req| {
            machine.lock(|machine| machine.handle_syscall(cx.local.heap, req))
        }) {
            // defmt::println!("Handled syscall!");
        }
    }

    #[task(binds = PendSV, shared = [machine], local = [timer], priority = 1)]
    fn pendsv(cx: pendsv::Context) {
        let mut machine = cx.shared.machine;
        let hwtimer = cx.local.timer;
        let mut sleep: Option<u32> = None;

        while let Some(msg) = DRIVER_QUEUE.dequeue() {
            match msg {
                DriverCommand::SpiStart => {
                    machine.lock(|machine| {
                        if let Some(spi) = machine.spi.as_mut() {
                            spi.start_send();
                        }
                    })
                }
                DriverCommand::SpiEnd => {
                    machine.lock(|machine| {
                        if let Some(spi) = machine.spi.as_mut() {
                            spi.end_send();
                        }
                    })
                }
                DriverCommand::SleepMicros(us) => {
                    if let Some(dly) = sleep.as_mut() {
                        *dly = (*dly).max(us);
                    } else {
                        sleep = Some(us);
                    }
                }
            }
        }

        // Implement a loop that counts the amount of time the CPU is idle.
        if let Some(dly) = sleep {
            // Clear the "timer expired flag"
            SNAP.store(false, Ordering::Release);

            // Set up the timer to delay for the requested number of microseconds.
            // Start the timer, and enable the interrupt
            hwtimer.set_oneshot();
            hwtimer.shorts.write(|w| w.compare0_stop().enabled());
            hwtimer.timer_reset_event();
            hwtimer.enable_interrupt();
            hwtimer.timer_start(dly);

            loop {
                // In order to make sure that we measure ALL interrupts, we disable them,
                // which will still *pend* them, but not *service* them
                cortex_m::interrupt::disable();

                // Don't check the "interrupt done" flag until AFTER interrupts are disabled,
                // to prevent racing with the interrupt
                let done = SNAP.load(Ordering::Acquire);

                // If the timer hasn't expired yet, go to sleep with a WFI, and measure how
                // long we were in that WFI condition
                if !done {
                    let start = hwtimer.read_counter();

                    // Despite being in an interrupt at the moment, WFI will still return IF
                    // a higher priority interrupt is fired (which our timer interrupt is).
                    //
                    // This means that we know when we make it past this block, SOME interrupt
                    // has fired, either our high prio timer, or some other interrupt which will
                    // cause us to use CPU.
                    compiler_fence(Ordering::SeqCst);
                    cortex_m::asm::wfi();
                    compiler_fence(Ordering::SeqCst);

                    // Increment the time spent sleeping, based on how long the time was between
                    // starting WFI, and ending WFI.
                    let end = hwtimer.read_counter();
                    IDLE_TICKS.fetch_add(end - start, Ordering::SeqCst);
                }

                // Whether we are done or not, re-enable interrupts, to allow them to be immediately
                // serviced. This will either handle our timer event, or whatever other interrupt
                // is pending.
                unsafe {
                    cortex_m::interrupt::enable();
                }

                // If we noticed our timer interrupt is done, break out of the loop.
                if done {
                    hwtimer.disable_interrupt();
                    break;
                }
            }
        }
    }

    #[task]
    fn ticky(_cx: ticky::Context) {
        let used = 1_000_000 - IDLE_TICKS.swap(0, Ordering::SeqCst);
        let scc = SYSCALLS.swap(0, Ordering::AcqRel);

        let used = used / 100;
        let pct_used = used / 100;
        let dec_used = used % 100;

        defmt::println!("CPU usage: {=u32}.{=u32:02}% - syscalls: {=u32}", pct_used, dec_used, scc);
        ticky::spawn_after(1000u32.millis()).ok();
    }

    #[idle(local = [prog_loaded])]
    fn idle(cx: idle::Context) -> ! {
        defmt::println!("Hello, world!");

        let timer = GlobalRollingTimer::default();
        let start = timer.get_ticks();

        // Wait, to allow RTT to attach
        while timer.millis_since(start) < 1000 { }

        defmt::println!("!!! - ENTERING USERSPACE - !!!");
        let loaded = *cx.local.prog_loaded;

        let pws = if let Some((ptr, len)) = loaded {
            {
                // Slice MUST be dropped before we run ram_ram_setup
                let slice = unsafe { core::slice::from_raw_parts(ptr, len) };
                validate_header(slice)
            }.map(|rh| {
                rh.ram_ram_setup()
            }).ok()
        } else {
            None
        };

        let pws = pws.unwrap_or_else(|| {
            let rh = validate_header(DEFAULT_IMAGE).unwrap();
            rh.oc_flash_setup(DEFAULT_IMAGE)
        });

        core::sync::atomic::compiler_fence(Ordering::SeqCst);

        unsafe {
            letsago(pws.stack_start, pws.entry_point);
        }
    }
}

#[inline(always)]
unsafe fn letsago(sp: u32, entry: u32) -> ! {
    // Do the not-so-dangerous stuff in Rust.

    // Calculate the desired CONTROL register value.
    let mut cur_ctl = control::read();
    cur_ctl.set_npriv(control::Npriv::Unprivileged);
    cur_ctl.set_spsel(control::Spsel::Psp);
    let cur_ctl = cur_ctl.bits();

    // Write the PSP. Note: This won't take effect until after we write control.
    psp::write(sp);

    // Here's where the spooky stuff happens.
    asm!(
        // Write the CONTROL register, disabling privilege and enabling the PSP
        "msr CONTROL, {}",

        // Writing the CONTROL register means we need to emit an isb instruction
        "isb",

        // Branch directly to the loaded program. No coming back.
        "bx {}",
        in(reg) cur_ctl,
        in(reg) entry,
        options(noreturn, nomem, nostack),
    );
}

pub fn make_nodes(
    dreq: Pin<Input<Floating>>,
    xcs: Pin<Output<PushPull>>,
    xdcs: Pin<Output<PushPull>>,
    mut ppi0: Ppi0,
    gpiote: Gpiote,
    spim0: &SPIM0,
) -> (CommandNode, DataNode) {
    let ch0 = gpiote.channel0();
    let ch_ev = ch0.input_pin(&dreq);
    ch_ev.none();

    let ch1 = gpiote.channel1();
    let ch_ev = ch1.input_pin(&dreq);
    ch_ev.lo_to_hi().enable_interrupt();

    ppi0.set_event_endpoint(ch0.event());
    ppi0.set_task_endpoint(&spim0.tasks_stop);
    ppi0.disable();

    let dreq_pin = dreq.pin();

    (
        CommandNode {
            cs: xcs,
            dreq: BadInputPin {
                dreq_port: dreq.port(),
                dreq_pin,
            },
        },
        DataNode {
            cs: xdcs,
            dreq: BadInputPin {
                dreq_port: dreq.port(),
                dreq_pin,
            },
            last_low: None,
        },
    )
}

pub struct CommandNode {
    cs: Pin<Output<PushPull>>,
    dreq: BadInputPin,
}

pub struct DataNode {
    cs: Pin<Output<PushPull>>,
    dreq: BadInputPin,
    last_low: Option<u32>,
}

pub struct BadInputPin {
    dreq_port: Port,
    dreq_pin: u8,
}

impl BadInputPin {
    fn pin_high(&mut self) -> bool {
        let port = unsafe {
            &*match self.dreq_port {
                Port::Port0 => P0::ptr(),
                Port::Port1 => P1::ptr(),
            }
        };
        if self.dreq_pin >= 32 {
            return false;
        }
        (port.in_.read().bits() & (1 << self.dreq_pin as u32)) != 0
    }
}

impl SpimNode for CommandNode {
    fn set_active(&mut self) {
        self.cs.set_low().ok();
        let gpiote = unsafe { &*GPIOTE::ptr() };
        // hi-to-lo, used for shortcut
        gpiote.config[0].modify(|_r, w| w.polarity().hi_to_lo());

        // Enable hi-to-lo -> stop shortcut
        let ppi = unsafe { &*PPI::ptr() };
        ppi.chenset.write(|w| unsafe { w.bits(1 << 0) });
    }

    fn set_inactive(&mut self) {
        self.cs.set_high().ok();

        // Disable hi-to-lo -> stop shortcut
        let ppi = unsafe { &*PPI::ptr() };
        ppi.chenclr.write(|w| unsafe { w.bits(1 << 0) });

        let gpiote = unsafe { &*GPIOTE::ptr() };
        gpiote.config[0].modify(|_r, w| w.polarity().none());
    }

    fn is_ready(&mut self) -> bool {
        self.dreq.pin_high()
    }
}

impl SpimNode for DataNode {
    fn set_active(&mut self) {
        self.cs.set_low().ok();
        let gpiote = unsafe { &*GPIOTE::ptr() };
        // hi-to-lo, used for shortcut
        gpiote.config[0].modify(|_r, w| w.polarity().hi_to_lo());

        // Enable hi-to-lo -> stop shortcut
        let ppi = unsafe { &*PPI::ptr() };
        ppi.chenset.write(|w| unsafe { w.bits(1 << 0) });
    }

    fn set_inactive(&mut self) {
        self.cs.set_high().ok();

        // Disable hi-to-lo -> stop shortcut
        let ppi = unsafe { &*PPI::ptr() };
        ppi.chenclr.write(|w| unsafe { w.bits(1 << 0) });

        let gpiote = unsafe { &*GPIOTE::ptr() };
        gpiote.config[0].modify(|_r, w| w.polarity().none());
    }

    fn is_ready(&mut self) -> bool {
        let timer = GlobalRollingTimer::default();
        if let Some(time) = self.last_low.take() {
            if timer.millis_since(time) < 5 {
                self.last_low = Some(time);
                return false;
            }
        }

        let ready = self.dreq.pin_high();
        if !ready {
            self.last_low = Some(timer.get_ticks());
        }
        ready
    }
}
