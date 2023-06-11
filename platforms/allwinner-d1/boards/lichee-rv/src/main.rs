#![no_std]
#![no_main]

use core::{panic::PanicInfo, time::Duration};
use drivers::Ram;
use kernel::{Kernel, KernelSettings};
use embedded_hal::blocking::delay::DelayMs;

const HEAP_SIZE: usize = 384 * 1024 * 1024;

#[link_section = ".aheap.AHEAP"]
#[used]
static AHEAP: Ram<HEAP_SIZE> = Ram::new();

#[allow(non_snake_case)]
#[riscv_rt::entry]
fn main() -> ! {
    let k = initialize_kernel().unwrap();

    // k.initialize(async {}).unwrap();

    loop {
        let tick = k.tick();

        if !tick.has_remaining {
            let mut delay = riscv::delay::McycleDelay::new(1_080_000_000);
            delay.delay_ms(1);
            let _turn = k.timer().force_advance_ticks(1);
        }
    }
}

fn initialize_kernel() -> Result<&'static Kernel, ()> {
    let k_settings = KernelSettings {
        heap_start: AHEAP.as_ptr(),
        heap_size: HEAP_SIZE,
        max_drivers: 16,
        k2u_size: 4096,
        u2k_size: 4096,
        timer_granularity: Duration::from_millis(1),
    };
    let k = unsafe {
        Kernel::new(k_settings).map_err(drop)?.leak().as_ref()
    };
    Ok(k)
}



#[panic_handler]
fn handler(_info: &PanicInfo) -> ! {
    loop {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}
