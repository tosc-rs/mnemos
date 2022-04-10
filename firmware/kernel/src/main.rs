#![no_main]
#![no_std]

static DEFAULT_IMAGE: &[u8] = include_bytes!("../appbins/p1echo.bin");

#[rtic::app(device = nrf52840_hal::pac, dispatchers = [SWI0_EGU0])]
mod app {
    use core::sync::atomic::Ordering;
    use cortex_m::singleton;
    use defmt::unwrap;
    use groundhog_nrf52::GlobalRollingTimer;
    use nrf52840_hal::{
        clocks::{ExternalOscillator, Internal, LfOscStopped},
        pac::TIMER0,
        usbd::{UsbPeripheral, Usbd},
        Clocks,
    };
    use kernel::{
        alloc::HEAP,
        monotonic::MonoTimer,
        drivers::usb_serial::{UsbUartParts, setup_usb_uart, UsbUartIsr, enable_usb_interrupts},
        syscall::{syscall_clear, try_recv_syscall},
        loader::validate_header,
    };
    use usb_device::{
        class_prelude::UsbBusAllocator,
        device::{UsbDeviceBuilder, UsbVidPid},
    };
    use usbd_serial::{SerialPort, USB_CLASS_CDC};
    use groundhog::RollingTimer;
    use super::{DEFAULT_IMAGE, letsago};

    #[monotonic(binds = TIMER0, default = true)]
    type Monotonic = MonoTimer<TIMER0>;

    #[shared]
    struct Shared {}

    #[local]
    struct Local {
        usb_isr: UsbUartIsr,
        machine: kernel::traits::Machine,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        let device = cx.device;

        // Setup clocks early in the process. We need this for USB later
        let clocks = Clocks::new(device.CLOCK);
        let clocks = clocks.enable_ext_hfosc();
        let clocks =
            unwrap!(singleton!(: Clocks<ExternalOscillator, Internal, LfOscStopped> = clocks));

        // Configure the monotonic timer, currently using TIMER0, a 32-bit, 1MHz timer
        let mono = Monotonic::new(device.TIMER0);

        // I am annoying, and prefer my own libraries.
        GlobalRollingTimer::init(device.TIMER1);

        // Setup the heap
        HEAP.init().ok();

        // Reset the syscall contents
        syscall_clear();

        // Before we give away the USB peripheral, enable the relevant interrupts
        enable_usb_interrupts(&device.USBD);

        let (usb_dev, usb_serial) = {
            let usb_bus = Usbd::new(UsbPeripheral::new(device.USBD, clocks));
            let usb_bus = defmt::unwrap!(singleton!(:UsbBusAllocator<Usbd<UsbPeripheral>> = usb_bus));

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

        let mut hg = defmt::unwrap!(HEAP.try_lock());

        let UsbUartParts { isr, sys } = defmt::unwrap!(setup_usb_uart(usb_dev, usb_serial));
        let box_uart = defmt::unwrap!(hg.alloc_box(sys));
        let leak_uart = box_uart.leak();
        let to_uart: &'static mut dyn kernel::traits::Serial = leak_uart;

        let pins = kernel::map_pins(device.P0, device.P1);
        let qsp = kernel::qspi::QspiPins {
            qspi_copi_io0: pins.qspi_d0.degrade(),
            qspi_cipo_io1: pins.qspi_d1.degrade(),
            qspi_io2: pins.qspi_d2.degrade(),
            qspi_io3: pins.qspi_d3.degrade(),
            qspi_csn: pins.qspi_csn.degrade(),
            qspi_sck: pins.qspi_sck.degrade(),
        };
        let qspi = kernel::qspi::Qspi::new(device.QSPI, qsp);
        let block = defmt::unwrap!(kernel::drivers::gd25q16::Gd25q16::new(qspi));
        let box_block = defmt::unwrap!(hg.alloc_box(block));
        let leak_block = box_block.leak();
        let to_block: &'static mut dyn kernel::traits::BlockStorage = leak_block;

        let machine = kernel::traits::Machine {
            serial: to_uart,
            block_storage: Some(to_block),
        };

        (
            Shared {},
            Local {
                usb_isr: isr,
                machine,
            },
            init::Monotonics(mono),
        )
    }

    #[task(binds = SVCall, local = [machine], priority = 1)]
    fn svc(cx: svc::Context) {
        let machine = cx.local.machine;

        if let Ok(()) = try_recv_syscall(|req| {
            machine.handle_syscall(req)
        }) {
            // defmt::println!("Handled syscall!");
        }
    }

    #[task(binds = USBD, local = [usb_isr], priority = 2)]
    fn usb_tick(cx: usb_tick::Context) {
        cx.local.usb_isr.poll();
    }

    // TODO: I am currently polling the syscall interfaces in the idle function,
    // since I don't have syscalls yet. In the future, the `machine` will be given
    // to the SWI handler, and idle will basically just launch a program. I think.
    // Maybe idle will use SWIs too.
    #[idle]
    fn idle(_cx: idle::Context) -> ! {
        defmt::println!("Hello, world!");

        let timer = GlobalRollingTimer::default();
        let start = timer.get_ticks();

        // Wait, to allow RTT to attach
        while timer.millis_since(start) < 100 { }

        defmt::println!("!!! - ENTERING USERSPACE - !!!");

        let rh = validate_header(DEFAULT_IMAGE).unwrap();
        let pws = rh.oc_flash_setup(DEFAULT_IMAGE);

        core::sync::atomic::compiler_fence(Ordering::SeqCst);

        unsafe {
            letsago(pws.stack_start, pws.entry_point);
        }
    }
}

use core::arch::asm;
use cortex_m::register::{control, psp};

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
