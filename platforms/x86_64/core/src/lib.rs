#![no_std]
extern crate alloc;

use core::time::Duration;
use hal_core::{boot::BootInfo, PAddr, VAddr};
use hal_x86_64::cpu::local::GsLocalData;
pub use hal_x86_64::cpu::{local::LocalKey, wait_for_interrupt};
use kernel::{mnemos_alloc::containers::Box, Kernel, KernelSettings};

pub mod acpi;
pub mod allocator;
pub mod drivers;
pub mod interrupt;
pub mod trace;

#[derive(Debug)]
pub struct PlatformConfig {
    pub rsdp_addr: Option<PAddr>,
    pub physical_mem_offset: VAddr,
}

pub fn init(bootinfo: &impl BootInfo, cfg: PlatformConfig) -> &'static Kernel {
    interrupt::enable_exceptions();
    bootinfo.init_paging();
    allocator::init(bootinfo, cfg.physical_mem_offset);

    let k = {
        let settings = KernelSettings {
            max_drivers: 64, // we are a big x86 system with lots of RAM, this can probably be an even bigger number!
            timer_granularity: interrupt::TIMER_INTERVAL,
        };

        unsafe {
            Box::into_raw(Kernel::new(settings).expect("cannot initialize kernel"))
                .as_ref()
                .unwrap()
        }
    };

    init_acpi(k, bootinfo, cfg.rsdp_addr);
    // TODO: PCI?

    // init boot processor's core-local data
    unsafe {
        GsLocalData::init();
    }
    tracing::info!("set up the boot processor's local data");

    // TODO: spawn drivers (UART, keyboard, ...)
    k.initialize(async {
        loop {
            k.timer().sleep(Duration::from_secs(5)).await;
            tracing::info!("help im trapped in an x86_64 operating system kernel!");
        }
    })
    .unwrap();

    k
}

pub fn run(bootinfo: &impl BootInfo, kernel: &'static Kernel) -> ! {
    tracing::info!("started kernel run loop\n--------------------\n");
    kernel.set_global_timer().unwrap();

    // TODO(eliza): this currently uses a periodic timer, rather than a
    // freewheeling timer like other MnemOS kernels. The periodic timer is
    // somewhat less efficient, as it results in us being woken every 10ms
    // regardless of what timeouts are pending. If we used a freewheeling timer
    // instead, we could sleep until a task is actually ready.
    //
    // However, this would require some upstream changes to the mycelium HAL to
    // better support freewheeling timers. For now, the simpler periodic timer
    // runloop works fine, I guess...
    loop {
        // drive the task scheduler
        let tick = kernel.tick();

        // turn the timer wheel if it wasn't turned recently and no one else is
        // holding a lock, ensuring any pending timer ticks are consumed.
        let turn = kernel.timer().force_advance_ticks(0);

        // if there are no woken tasks, wait for an interrupt. otherwise,
        // continue ticking.
        let has_remaining = tick.has_remaining || turn.has_remaining();
        if !has_remaining {
            interrupt::wait_for_interrupt();
        }
    }
}

fn init_acpi(k: &'static Kernel, bootinfo: &impl BootInfo, rsdp_addr: Option<PAddr>) {
    if let Some(rsdp) = rsdp_addr {
        let acpi = acpi::acpi_tables(rsdp);
        let platform_info = acpi.and_then(|acpi| acpi.platform_info());
        match platform_info {
            Ok(platform) => {
                tracing::debug!("found ACPI platform info");
                interrupt::enable_hardware_interrupts(Some(&platform.interrupt_model), k.timer());
                acpi::bringup_smp(&platform)
                    .expect("failed to bring up application processors! this is bad news!");
                return;
            }
            Err(error) => tracing::warn!(?error, "missing ACPI platform info"),
        }
    } else {
        // TODO(eliza): try using MP Table to bringup application processors?
        tracing::warn!("no RSDP from bootloader, skipping SMP bringup");
    }

    // no ACPI
    interrupt::enable_hardware_interrupts(None, k.timer())
}
