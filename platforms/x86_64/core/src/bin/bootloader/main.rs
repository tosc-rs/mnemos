#![no_std]
#![no_main]
#![feature(panic_info_message)]

#[cfg(not(feature = "bootloader_api"))]
compile_error!(
    "building the `mnemos-x86_64-bootloader` binary requires the \
    'bootloader_api' Cargo feature to be enabled",
);
extern crate alloc;

use bootloader_api::config::{BootloaderConfig, Mapping};
use hal_core::{
    framebuffer::{Draw, RgbColor},
    PAddr, VAddr,
};
use hal_x86_64::cpu;
mod bootinfo;
mod framebuf;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    // the kernel is mapped into the higher half of the virtual address space.
    config.mappings.dynamic_range_start = Some(0xFFFF_8000_0000_0000);
    config.mappings.page_table_recursive = Some(Mapping::Dynamic);

    config
};

bootloader_api::entry_point!(kernel_start, config = &BOOTLOADER_CONFIG);

pub fn kernel_start(info: &'static mut bootloader_api::BootInfo) -> ! {
    unsafe {
        cpu::intrinsics::cli();
    }

    let cfg = {
        let phys_offset = info
            .physical_memory_offset
            .into_option()
            // TODO(eliza): does `None` here mean "physical mem is identity
            // mapped" or "we don't know where the physical mem is mapped"?
            // check the bootloader docs...
            .unwrap_or(0);
        mnemos_x86_64_core::PlatformConfig {
            rsdp_addr: info.rsdp_addr.into_option().map(PAddr::from_u64),
            physical_mem_offset: VAddr::from_u64(phys_offset),
        }
    };
    let bootinfo = bootinfo::BootloaderApiBootInfo::from_bootloader(info);

    let subscriber = {
        let framebuf = (|| unsafe { framebuf::mk_framebuf() }) as fn() -> _;
        framebuf().fill(RgbColor::BLACK);
        mnemos_x86_64_core::trace::TraceSubscriber::new(framebuf)
    };
    tracing::subscriber::set_global_default(subscriber)
        .expect("tracing subscriber should not have already been set!");

    let k = mnemos_x86_64_core::init(&bootinfo, cfg);
    mnemos_x86_64_core::run(&bootinfo, k)
}

#[cold]
#[cfg_attr(target_os = "none", panic_handler)]
fn panic(panic: &core::panic::PanicInfo<'_>) -> ! {
    use core::fmt::Write;
    use embedded_graphics::{
        mono_font::MonoTextStyleBuilder,
        pixelcolor::{Rgb888, RgbColor as _},
        prelude::*,
    };
    use mnemos_x86_64_core::drivers::framebuf::TextWriter;

    // /!\ disable all interrupts, unlock everything to prevent deadlock /!\
    //
    // Safety: it is okay to do this because we are panicking and everything
    // is going to die anyway.
    unsafe {
        // disable all interrupts.
        cpu::intrinsics::cli();

        // TODO(eliza): claim serial

        // unlock the frame buffer
        framebuf::force_unlock();
    }

    let mut framebuf = unsafe { framebuf::mk_framebuf() };

    let mut writer = {
        let font = &profont::PROFONT_12_POINT;
        let char_height = font.character_size.height;
        // write the panic message at the bottom of the framebuffer, so that we
        // don't clobber any existing text preceeding the panic (useful for
        // debugging).
        let point = {
            let height_px = framebuf.height() as u32;
            let last_line = (height_px - char_height - 10) as i32;
            Point::new(10, last_line)
        };

        // scroll the framebuffer up by one line of text to make space for the
        // panic message.
        framebuf.scroll_vert(char_height as isize);

        let style = MonoTextStyleBuilder::new()
            .font(font)
            .text_color(Rgb888::WHITE)
            .background_color(Rgb888::RED)
            .build();
        TextWriter::new(&mut framebuf, style, point)
    };

    let _ = writer.write_str("mnemOS panicked");
    if let Some(message) = panic.message() {
        let _ = writeln!(&mut writer, ":\n  {message}");
    } else if let Some(payload) = panic.payload().downcast_ref::<&'static str>() {
        let _ = writeln!(&mut writer, ":\n  {payload}");
    } else if let Some(payload) = panic.payload().downcast_ref::<alloc::string::String>() {
        let _ = writeln!(&mut writer, ":\n  {payload}");
    }

    if let Some(location) = panic.location() {
        let _ = writeln!(&mut writer, "  at {location}");
    }

    unsafe {
        cpu::halt();
    }
}
