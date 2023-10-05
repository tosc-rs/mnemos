#![no_std]
#![no_main]

#[allow(non_snake_case)]
#[riscv_rt::entry]
fn main() -> ! {
    let config = mnemos_config::include_config!(mnemos_d1::PlatformConfig, "mq-pro").unwrap();
    mnemos_d1::kernel_entry(config);
}
