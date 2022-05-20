#![no_main]
#![no_std]

use core::{arch::asm, sync::atomic::{AtomicU32, AtomicBool}};
use cortex_m::{
    asm::isb,
    register::{control, psp},
    peripheral::SCB,
};
use groundhog_nrf52::GlobalRollingTimer;
use kernel::monotonic::ExtU32;

use kernel::{
    alloc::HEAP,
    drivers::usb_serial::{enable_usb_interrupts, setup_usb_uart, UsbUartIsr, UsbUartParts},
    monotonic::MonoTimer,
    traits::{Machine, Serial},
};
use nrf52840_hal::{
    clocks::{ExternalOscillator, Internal, LfOscStopped},
    pac::{TIMER0, TIMER2},
    usbd::{UsbPeripheral, Usbd},
    Clocks,
};
use usb_device::{
    class_prelude::UsbBusAllocator,
    device::{UsbDeviceBuilder, UsbVidPid},
};
use usbd_serial::{SerialPort, USB_CLASS_CDC};

static IDLE_TICKS: AtomicU32 = AtomicU32::new(0);
static SYSCALLS: AtomicU32 = AtomicU32::new(0);
static SNAP: AtomicBool = AtomicBool::new(false);

#[rtic::app(device = nrf52840_hal::pac, dispatchers = [SWI0_EGU0])]
mod app {
    use core::sync::atomic::Ordering;

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

        // TODO: setup syscall queues

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

        let to_uart: &'static mut dyn Serial =
            defmt::unwrap!(heap_guard.leak_send(sys).map_err(drop));

        let machine = Machine {
            serial: to_uart,
            block_storage: None,
            spi: None,
            pcm: None,
            gpios: &mut [],
            rand: None,
        };

        ticky::spawn_after(1000u32.millis()).ok();

        (
            Shared {
                machine,
            },
            Local {
                usb_isr: isr,
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

    #[task(binds = SVCall, priority = 2)]
    fn svc(_cx: svc::Context) {
        SYSCALLS.fetch_add(1, Ordering::Release);
        SCB::set_pendsv();
    }

    #[task(binds = PendSV, shared = [machine], local = [timer], priority = 1)]
    fn pendsv(_cx: pendsv::Context) {

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

    #[idle]
    fn idle(_cx: idle::Context) -> ! {
        defmt::println!("Hello, world!");

        defmt::panic!("Oops no userspace yet")
    }
}

#[allow(dead_code)]
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
