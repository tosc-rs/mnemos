#![no_main]
#![no_std]

use core::arch::asm;
use cortex_m::{
    asm::isb,
    register::{control, psp},
    singleton,
};
use defmt::unwrap;
use groundhog::RollingTimer;
use groundhog_nrf52::GlobalRollingTimer;

#[allow(unused_imports)]
use kernel::{
    alloc::{HeapGuard, HEAP},
    drivers::{
        gd25q16::Gd25q16,
        nrf52_spim_nonblocking::{new_send_fut, Spim},
        usb_serial::{enable_usb_interrupts, setup_usb_uart, UsbUartIsr, UsbUartParts},
        vs1053b::Vs1053b,
    },
    loader::validate_header,
    map_pins,
    monotonic::MonoTimer,
    qspi::{Qspi, QspiPins},
    syscall::{syscall_clear, try_recv_syscall},
    traits::{BlockStorage, Machine, Serial, SpiHandle, SpiTransactionKind, SpimNode, Spi, PcmSink},
    MAGIC_BOOT,
};
use nrf52840_hal::{
    clocks::{ExternalOscillator, Internal, LfOscStopped},
    gpio::{Floating, Input, Level, Output, Pin, Port, PushPull},
    gpiote::Gpiote,
    pac::{GPIOTE, P0, P1, PPI, SPIM0, TIMER0},
    ppi::{ConfigurablePpi, Parts, Ppi, Ppi0},
    prelude::OutputPin,
    rng::Rng,
    usbd::{UsbPeripheral, Usbd},
    Clocks,
};
use usb_device::{
    class_prelude::UsbBusAllocator,
    device::{UsbDeviceBuilder, UsbVidPid},
};
use usbd_serial::{SerialPort, USB_CLASS_CDC};

static DEFAULT_IMAGE: &[u8] = include_bytes!("../appbins/tony.bin");



#[rtic::app(device = nrf52840_hal::pac, dispatchers = [SWI0_EGU0])]
mod app {
    use core::sync::atomic::Ordering;

    use super::*;

    #[monotonic(binds = TIMER0, default = true)]
    type Monotonic = MonoTimer<TIMER0>;

    #[shared]
    struct Shared {
        heap: HeapGuard,
        machine: Machine,
    }

    #[local]
    struct Local {
        usb_isr: UsbUartIsr,
        rng: Rng,
        prog_loaded: Option<(*const u8, usize)>,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        let device = cx.device;

        // Setup clocks early in the process. We need this for USB later
        let clocks = Clocks::new(device.CLOCK);
        let clocks = clocks.enable_ext_hfosc();
        let clocks =
            unwrap!(singleton!(: Clocks<ExternalOscillator, Internal, LfOscStopped> = clocks));

        // Enable instruction caches for MAXIMUM SPEED
        device.NVMC.icachecnf.write(|w| w.cacheen().set_bit());
        isb();

        // Configure the monotonic timer, currently using TIMER0, a 32-bit, 1MHz timer
        let mono = Monotonic::new(device.TIMER0);

        // I am annoying, and prefer my own libraries.
        GlobalRollingTimer::init(device.TIMER1);

        let rng = Rng::new(device.RNG);

        // Setup the heap
        let mut heap_guard = HEAP.init_exclusive().unwrap();

        // Reset the syscall contents
        syscall_clear();

        // Before we give away the USB peripheral, enable the relevant interrupts
        enable_usb_interrupts(&device.USBD);

        let (usb_dev, usb_serial) = {
            let usb_bus = Usbd::new(UsbPeripheral::new(device.USBD, clocks));
            let usb_bus =
                defmt::unwrap!(singleton!(:UsbBusAllocator<Usbd<UsbPeripheral>> = usb_bus));

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

            (usb_dev, usb_serial)
        };

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


        let machine = Machine {
            serial: to_uart,
            block_storage: Some(to_block),
            spi: Some(spi),
            pcm: Some(vs1053b),
            gpios: &mut [],
        };

        (
            Shared {
                heap: heap_guard,
                machine,
            },
            Local {
                usb_isr: isr,
                rng,
                prog_loaded,
            },
            init::Monotonics(mono),
        )
    }

    #[task(binds = SVCall, shared = [machine, heap], priority = 1)]
    fn svc(cx: svc::Context) {
        if let Ok(()) = try_recv_syscall(|req| {
            (cx.shared.heap, cx.shared.machine).lock(|heap, machine| machine.handle_syscall(heap, req))
        }) {
            // defmt::println!("Handled syscall!");
        }
    }

    #[task(binds = GPIOTE, shared = [machine], priority = 2)]
    fn gpiote(mut cx: gpiote::Context) {
        // TODO: NOT this
        let gpiote = unsafe { &*GPIOTE::ptr() };

        // Clear channel 1 events
        gpiote.events_in[1].write(|w| w);

        cx.shared.machine.lock(|machine| {
            machine.spi.as_mut().map(|spi| spi.start_send());
        })
    }

    #[task(binds = SPIM0_SPIS0_TWIM0_TWIS0_SPI0_TWI0, shared = [machine], priority = 2)]
    fn spim0(mut cx: spim0::Context) {
        // TODO: NOT this
        let gpiote = unsafe { &*GPIOTE::ptr() };

        // Clear channel 0 events (which probably stopped our SPI device)
        gpiote.events_in[0].write(|w| w);

        // defmt::println!("[INT]: SPIM0");

        cx.shared.machine.lock(|machine| {
            machine.spi.as_mut().map(|spi| spi.end_send());
        })
    }

    #[task(binds = USBD, local = [usb_isr], priority = 3)]
    fn usb_tick(cx: usb_tick::Context) {
        cx.local.usb_isr.poll();
    }

    // TODO: I am currently polling the syscall interfaces in the idle function,
    // since I don't have syscalls yet. In the future, the `machine` will be given
    // to the SWI handler, and idle will basically just launch a program. I think.
    // Maybe idle will use SWIs too.
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

//     // TODO: I am currently polling the syscall interfaces in the idle function,
//     // since I don't have syscalls yet. In the future, the `machine` will be given
//     // to the SWI handler, and idle will basically just launch a program. I think.
//     // Maybe idle will use SWIs too.
//     // #[idle(local = [prog_loaded])]
//     #[idle(local = [rng, cmd_hdl, data_hdl], shared = [heap, machine])]
//     fn idle(mut cx: idle::Context) -> ! {
//         let freq = cx.local.rng.random_u8();
//         let freq = (freq as f32) + 256.0;
//
//         defmt::println!("Hello, world!");
//
//         let timer = GlobalRollingTimer::default();
//         let start = timer.get_ticks();
//
//         // Wait, to allow RTT to attach
//         while timer.millis_since(start) < 1000 {}
//
//         let samp_per_cyc: f32 = 44100.0 / freq; // 141.7
//         let fincr = 256.0 / samp_per_cyc; // 1.81
//         let mut incr: i32 = (((1 << 24) as f32) * fincr) as i32;
//
//         // generate the next 256 samples...
//         let mut cur_offset = 0i32;
//
//         let mut last_change = timer.get_ticks();
//         let mut ttl_timer_sec = timer.get_ticks();
//         let mut idl_timer_sec = 0u32;
//
//         let machine = &mut cx.shared.machine;
//         let heap = &mut cx.shared.heap;
//
//         defmt::println!("Enabling...");
//         (machine, heap).lock(|machine, heap| {
//             let Machine { spi, pcm, .. } = machine;
//
//             if let (Some(spi), Some(pcm)) = (spi, pcm) {
//                 pcm.enable(heap, &mut **spi).unwrap();
//             } else {
//                 panic!()
//             }
//         });
//         defmt::println!("Enabled!");
//
//         let mut iters = 0;
//         while iters < 10_000 {
//             if timer.millis_since(ttl_timer_sec) >= 1_000 {
//                 let act_elapsed = timer.micros_since(ttl_timer_sec);
//                 defmt::println!(
//                     "idle pct: {=f32}%",
//                     (idl_timer_sec as f32 * 100.0) / (act_elapsed as f32)
//                 );
//                 idl_timer_sec = 0;
//                 ttl_timer_sec = timer.get_ticks();
//             }
//
//             if timer.millis_since(last_change) > 250 {
//                 last_change = timer.get_ticks();
//                 incr = new_freq_incr(cx.local.rng);
//             }
//
//             let machine = &mut cx.shared.machine;
//             let heap = &mut cx.shared.heap;
//
//             let tx = (machine, heap).lock(|machine, heap| {
//                 let Machine { spi, pcm, .. } = machine;
//
//                 if let (Some(spi), Some(pcm)) = (spi, pcm) {
//                     pcm.allocate_stereo_samples(heap, &mut **spi, 512)
//                 } else {
//                     None
//                 }
//             });
//
//             if let Some(mut tx) = tx {
//                 fill_sample_buf(&mut tx.data, incr, &mut cur_offset);
//                 tx.release_to_kernel();
//                 iters += 1;
//             } else {
//                 let start = timer.get_ticks();
//                 idl_timer_sec += 5_000;
//                 while timer.micros_since(start) < 5_000 {}
//             }
//         }
//         let start = timer.get_ticks();
//         while timer.millis_since(start) <= 1000 {}
//         kernel::exit();
//     }
}

fn new_freq_incr(rng: &mut Rng) -> i32 {
    let f = rng.random_u8();
    let freq = (f as f32) + 256.0;
    defmt::println!("Freq: {=f32}", freq);
    let samp_per_cyc: f32 = 44100.0 / freq; // 141.7
    let fincr = 256.0 / samp_per_cyc; // 1.81
    let incr = (((1 << 24) as f32) * fincr) as i32;
    incr
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
        self.dreq.pin_high()
    }
}
