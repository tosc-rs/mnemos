#![no_std]
#![no_main]

use heapless::String;
use userspace::common::porcelain::{serial, time, block_storage};
use core::fmt::Write;

#[no_mangle]
pub fn entry() -> ! {
    // First, open Port 1 (we will write to it)
    serial::open_port(1).unwrap();
    let mut strbuf: String<1024> = String::new();
    let mut name_buf = [0u8; 256];

    loop {
        serial::write_port(0, b"Current Flash Contents:\r\n").unwrap();

        let store_info = block_storage::store_info().unwrap();
        write!(&mut strbuf, "blocks: {}, capacity: {}\r\n", store_info.blocks, store_info.capacity).ok();
        serial::write_port(0, strbuf.as_bytes()).unwrap();
        strbuf.clear();

        for block in 0..store_info.blocks {
            let block_info = block_storage::block_info(block, &mut name_buf).unwrap();
            write!(
                &mut strbuf, "{:02}: {:?}, {:?}, {:?}, {}/{}\r\n",
                block,
                block_info.name,
                block_info.kind,
                block_info.status,
                block_info.length,
                store_info.capacity
            ).ok();
            serial::write_port(0, strbuf.as_bytes()).unwrap();
            strbuf.clear();
        }

        time::sleep_micros(5_000_000).ok();
    }
}
