#![no_main]
#![no_std]

#[rtic::app(device = nrf52840_hal::pac, dispatchers = [SWI0_EGU0])]
mod app {
    use core::sync::atomic::Ordering;
    use cortex_m::{singleton, register::{psp, control}};
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
        monotonic::{MonoTimer},
        drivers::usb_serial::{UsbUartParts, setup_usb_uart, UsbUartIsr, enable_usb_interrupts},
        syscall::{syscall_clear, try_recv_syscall},
    };
    use usb_device::{
        class_prelude::UsbBusAllocator,
        device::{UsbDeviceBuilder, UsbVidPid},
    };
    use usbd_serial::{SerialPort, USB_CLASS_CDC};
    use groundhog::RollingTimer;

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

        let machine = kernel::traits::Machine {
            serial: to_uart,
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
        // Begin VERY CURSED userspace setup code
        //
        // TODO/UNSAFETY:
        // This is likely all kinds of UB, and should be replaced with
        // a divergent assembly function that:
        //
        // * Takes the new "userspace entry point" as an argument
        // * Takes the new PSP start address as an argument
        // * Sets the PSP
        // * Enables npriv and psp in the control register
        // * Calls the userspace entry point
        core::sync::atomic::compiler_fence(Ordering::SeqCst);

        // Try to set usermode! TODO! Should probably critical section!
        // but also don't use any stack variables!
        unsafe {
            extern "C" {
                static _app_stack_start: u32;
            }
            let stack_start = &_app_stack_start as *const u32 as u32;
            defmt::println!("Setting PSP to: 0x{=u32:08X}", stack_start);
            psp::write(stack_start);

            // Note: This is where the really cursed stuff happens. We simultaneously:
            // * Switch the stack pointer to use the newly-minted PSP instead of the
            //     default/exception mode MSP
            // * Disables privilege mode for thread mode (e.g. `idle`)
            //
            // This makes the compiler sad. See note above for how to fix this the
            // "right" way (spoiler: use ASM)
            let mut cur_ctl = control::read();
            cur_ctl.set_npriv(control::Npriv::Unprivileged);
            cur_ctl.set_spsel(control::Spsel::Psp);
            control::write(cur_ctl);
        }

        core::sync::atomic::compiler_fence(Ordering::SeqCst);

        // So, moving the stack pointer is all kinds of cursed, and I should probably
        // be using inline asm or something to do the above. Instead, immediately
        // jump into another inline(never) function, and hope this wards off all of the
        // UB goblins for now until I feel like learning ASM.
        crate::userspace::entry();
        // End VERY CURSED userspace setup code
    }
}

mod userspace {
    use kernel::{self as _, alloc::HEAP};
    use common::porcelain::{serial, time};

    #[inline(never)]
    pub fn entry() -> ! {
        // First, open Port 1 (we will write to it)
        defmt::unwrap!(serial::open_port(1));

        let mut buf = [0u8; 128];

        loop {
            for _ in 0..100 {
                if let Ok(data) = serial::read_port(0, &mut buf) {
                    match serial::write_port(1, data) {
                        Ok(None) => {},
                        Ok(Some(_)) => defmt::println!("Remainder?"),
                        Err(()) => defmt::println!("Error writing port 1!"),
                    }
                } else {
                    defmt::println!("Read port 0 failed!");
                }

                time::sleep_micros(10_000).ok();
            }

            if let Some(hg) = HEAP.try_lock() {
                let used = hg.used_space();
                let free = hg.free_space();
                defmt::println!("used: {=usize}, free: {=usize}", used, free);
            }
        }
    }
}
