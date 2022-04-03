#![no_main]
#![no_std]

use kernel as _; // global logger + panicking-behavior + memory layout

#[rtic::app(device = nrf52840_hal::pac, dispatchers = [SWI0_EGU0])]
mod app {
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
        monotonic::{ExtU32, MonoTimer},
        drivers::usb_serial::{UsbUartParts, setup_usb_uart, UsbUartIsr},
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

        usb_tick::spawn().ok();

        (
            Shared {},
            Local {
                usb_isr: isr,
                machine,
            },
            init::Monotonics(mono),
        )
    }

    #[task(local = [usb_isr])]
    fn usb_tick(cx: usb_tick::Context) {
        cx.local.usb_isr.poll();
        usb_tick::spawn_after(1.millis()).ok();
    }

    // TODO: I am currently polling the syscall interfaces in the idle function,
    // since I don't have syscalls yet. In the future, the `machine` will be given
    // to the SWI handler, and idle will basically just launch a program. I think.
    // Maybe idle will use SWIs too.
    #[idle(local = [machine])]
    fn idle(cx: idle::Context) -> ! {
        defmt::println!("Hello, world!");
        let machine = cx.local.machine;
        let mut buf = [0u8; 128];
        let timer = GlobalRollingTimer::default();
        let mut last_mem = timer.get_ticks();

        loop {
            if timer.millis_since(last_mem) >= 1000 {
                if let Some(hg) = HEAP.try_lock() {
                    last_mem = timer.get_ticks();
                    let used = hg.used_space();
                    let free = hg.free_space();

                    defmt::println!("used: {=usize}, free: {=usize}", used, free);
                }
            }
            match machine.serial.recv(&mut buf) {
                Ok(sli) => {
                    let mut remain: &[u8] = sli;

                    while !remain.is_empty() {
                        match machine.serial.send(remain) {
                            Ok(()) => {
                                remain = &[];
                            },
                            Err(rem) => {
                                remain = rem;
                            },
                        }
                    }
                },
                Err(_) => todo!(),
            }
        }
    }
}
