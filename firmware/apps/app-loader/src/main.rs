#![no_std]
#![no_main]

use heapless::{String, Vec};
use userspace::common::{porcelain::{serial, time, block_storage, system}, syscall::BlockKind};
use core::fmt::Write;
use menu::{Menu, Item, Runner, Parameter};
use postcard::{CobsAccumulator, FeedResult};
use serde::{Serialize, Deserialize};

const ROOT_MENU: Menu<Context> = Menu {
    label: "root",
    items: &[
        &Item {
            item_type: menu::ItemType::Callback {
                function: store_info,
                parameters: &[],
            },
            command: "info",
            help: Some("Information about the storage device"),
        },
        &Item {
            item_type: menu::ItemType::Callback {
                function: block_info,
                parameters: &[
                    Parameter::Mandatory {
                        parameter_name: "idx",
                        help: Some("The block index to retrieve"),
                    }
                ],
            },
            command: "block",
            help: Some("Information about a specific block"),
        },
        &Item {
            item_type: menu::ItemType::Callback {
                function: upload,
                parameters: &[
                    Parameter::Mandatory {
                        parameter_name: "idx",
                        help: Some("The block index to upload"),
                    },
                ],
            },
            command: "upload",
            help: Some("Information about a specific block"),
        },
        &Item {
            item_type: menu::ItemType::Callback {
                function: boot,
                parameters: &[
                    Parameter::Mandatory {
                        parameter_name: "idx",
                        help: Some("The block index to boot from"),
                    },
                ],
            },
            command: "boot",
            help: Some("Boot from a given block"),
        },
        &Item {
            item_type: menu::ItemType::Callback {
                function: upl_stat,
                parameters: &[],
            },
            command: "ustat",
            help: Some("Uploading information"),
        },
        &Item {
            item_type: menu::ItemType::Callback {
                function: upl_complete,
                parameters: &[
                    Parameter::Mandatory {
                        parameter_name: "kind",
                        help: Some("The block kind (after upload). 'program' or 'storage'")
                    },
                    Parameter::Mandatory {
                        parameter_name: "name",
                        help: Some("The block kind (after upload). 'program' or 'storage'")
                    },
                ],
            },
            command: "ucomplete",
            help: Some("Complete Upload"),
        }
    ],
    entry: None,
    exit: None,
};

struct Context {
    buf: String<1024>,
    uploading: Option<Uploader>,
}

impl core::fmt::Write for Context {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.buf.write_str(s)
    }
}

fn boot<'a>(_menu: &Menu<Context>, item: &Item<Context>, args: &[&str], context: &mut Context) {
    if let Some(upl) = context.uploading.as_ref() {
        writeln!(&mut context.buf, "Error: Already uploading to {}!", upl.block).unwrap();
        return;
    }

    let idx = if let Ok(Some(parm)) = menu::argument_finder(item, args, "idx") {
        if let Ok(idx) = str::parse::<u32>(parm) {
            idx
        } else {
            writeln!(context, "Error: Failed to parse {} as an index!", parm).unwrap();
            return;
        }
    } else {
        writeln!(context, "Error: Missing argument!").unwrap();
        return;
    };

    let store_info = block_storage::store_info().unwrap();

    if idx >= store_info.blocks {
        writeln!(context, "Error: Invalid block index!").unwrap();
        return;
    }

    // Sure, whatever
    system::set_boot_block(idx).unwrap();
    system::reset().unwrap();
}

fn upl_complete<'a>(_menu: &Menu<Context>, item: &Item<Context>, args: &[&str], context: &mut Context) {
    if let Some(upl) = context.uploading.as_ref() {
        if !upl.is_done {
            writeln!(context.buf, "Not done!").unwrap();
            return;
        }

        let kind = match menu::argument_finder(item, args, "kind") {
            Ok(Some("storage")) => BlockKind::Storage,
            Ok(Some("program")) => BlockKind::Program,
            _ => {
                writeln!(context, "Error: Invalid kind!").unwrap();
                return;
            }
        };

        let name = match menu::argument_finder(item, args, "name") {
            Ok(Some(name)) => {
                name
            },
            _ => {
                writeln!(context, "Error: Invalid name!").unwrap();
                return;
            }
        };

        block_storage::block_close(upl.block, name, upl.ttl_offset as u32, kind).unwrap();
        writeln!(context, "Closed successfully!").unwrap();
        context.uploading = None;
    } else {
        writeln!(context, "Not uploading!").unwrap();
    }
}

fn upl_stat<'a>(_menu: &Menu<Context>, _item: &Item<Context>, _args: &[&str], context: &mut Context) {
    if let Some(upl) = context.uploading.as_ref() {
        writeln!(context.buf, "Uploading:    Block {}", upl.block).unwrap();
        writeln!(context.buf, "Disk Written: {}", upl.ttl_offset).unwrap();
        writeln!(context.buf, "Done:         {}", upl.is_done).unwrap();
    } else {
        writeln!(context, "Not uploading!").unwrap();
    }
}

fn store_info<'a>(_menu: &Menu<Context>, _item: &Item<Context>, _args: &[&str], context: &mut Context) {
    let store_info = block_storage::store_info().unwrap();
    writeln!(context, "Block Storage Device Information:").unwrap();
    writeln!(context, "blocks: {}, capacity: {}", store_info.blocks, store_info.capacity).unwrap();
}

fn block_info<'a>(_menu: &Menu<Context>, item: &Item<Context>, args: &[&str], context: &mut Context) {
    let idx = if let Ok(Some(parm)) = menu::argument_finder(item, args, "idx") {
        if let Ok(idx) = str::parse::<u32>(parm) {
            idx
        } else {
            writeln!(context, "Error: Failed to parse {} as an index!", parm).unwrap();
            return;
        }
    } else {
        writeln!(context, "Error: Missing argument!").unwrap();
        return;
    };

    let store_info = block_storage::store_info().unwrap();

    if idx >= store_info.blocks {
        writeln!(context, "Error: Invalid block index!").unwrap();
        return;
    }

    let mut name_buf = [0u8; 128];
    match block_storage::block_info(idx, &mut name_buf) {
        Ok(block_info) => {
            writeln!(
                context,
                "{:02}: name: {:?}, kind: {:?}, status: {:?}, size: {}/{}",
                idx,
                block_info.name,
                block_info.kind,
                block_info.status,
                block_info.length,
                store_info.capacity
            ).ok();
        }
        Err(()) => {
            writeln!(context, "Error: Command failed!").unwrap();
        }
    }
}

fn upload<'a>(_menu: &Menu<Context>, item: &Item<Context>, args: &[&str], context: &mut Context) {
    if let Some(upl) = context.uploading.as_ref() {
        writeln!(&mut context.buf, "Error: Already uploading to {}!", upl.block).unwrap();
        return;
    }

    let idx = if let Ok(Some(parm)) = menu::argument_finder(item, args, "idx") {
        if let Ok(idx) = str::parse::<u32>(parm) {
            idx
        } else {
            writeln!(context, "Error: Failed to parse {} as an index!", parm).unwrap();
            return;
        }
    } else {
        writeln!(context, "Error: Missing argument!").unwrap();
        return;
    };

    let store_info = block_storage::store_info().unwrap();

    if idx >= store_info.blocks {
        writeln!(context, "Error: Invalid block index!").unwrap();
        return;
    }

    match block_storage::block_open(idx) {
        Ok(()) => {
            writeln!(context, "Opened block {}.", idx).unwrap();
        }
        Err(()) => {
            writeln!(context, "Failed to open block!").unwrap();
            return;
        }
    }

    // Drain anything from port 1 already there
    let mut buf = [0u8; 32];
    let mut timeouts = 0;
    loop {
        if timeouts >= 3 {
            break;
        }
        match serial::read_port(1, &mut buf) {
            Ok(data) if data.len() > 0 => {
                timeouts = 0;
            },
            Ok(_) => {
                timeouts += 1;
                time::sleep_micros(10_000).ok();
            }
            Err(_) => {
                writeln!(context, "Error clearing port 1!").unwrap();
                return;
            },
        }
    }

    writeln!(context, "Listening to port 1 for data...").unwrap();

    let mut upl = Uploader {
        block: idx,
        abuf: AlignBuf { byte: [0u8; 256] },
        ttl_offset: 0,
        acc: CobsAccumulator::new(),
        is_done: false,
    };

    upl.send_request();

    context.uploading = Some(upl);
}

#[repr(align(4))]
struct AlignBuf {
    byte: [u8; 256],
}

struct Uploader {
    block: u32,
    abuf: AlignBuf,
    ttl_offset: usize,
    acc: CobsAccumulator<512>,
    is_done: bool,
}

#[derive(Serialize, Deserialize)]
enum Request {
    Send {
        offset: u32,
    },
    Done,
}

#[derive(Serialize, Deserialize)]
enum Response {
    Buffer {
        start: u32,
        data: Vec<u8, 256>,
    },
    Done(u32),
    Retry,
}

impl Uploader {
    fn process(&mut self) {
        let mut upl_buf = [0u8; 64];


        loop {
            match serial::read_port(1, &mut upl_buf) {
                Ok(buf) if buf.len() > 0 => {
                    let mut window = &buf[..];
                    while !window.is_empty() {
                        match self.acc.feed::<Response>(window) {
                            FeedResult::Consumed => {
                                window = &[];
                            },
                            FeedResult::OverFull(rem) => {
                                window = rem;
                                self.send_request();
                            }
                            FeedResult::DeserError(rem) => {
                                window = rem;
                                self.send_request();
                            },
                            FeedResult::Success { data, remaining } => {
                                self.handle_msg(&data);
                                window = remaining;
                            },
                        }
                    }
                }
                Ok(_) => return,
                Err(_) => return,
            }
        }
    }

    fn handle_msg(&mut self, resp: &Response) {
        match resp {
            Response::Buffer { start, data } => {
                if (*start != self.ttl_offset as u32) || (data.len() != 256) {
                    self.send_request();
                    return
                }
                self.abuf.byte.copy_from_slice(&data);
                block_storage::block_write(self.block, self.ttl_offset as u32, &self.abuf.byte).unwrap();
                self.ttl_offset += 256;
            },
            Response::Done(amt) => {
                if self.ttl_offset as u32 == *amt {
                    let mut out_buf = [0u8; 32];
                    let req = Request::Done;
                    let used = postcard::to_slice_cobs(&req, &mut out_buf).unwrap();
                    serial::write_port(1, &used[..]).unwrap();
                    self.is_done = true;
                }
            },
            Response::Retry => {
                self.send_request();
            },
        }
    }

    fn send_request(&mut self) {
        let mut out_buf = [0u8; 32];
        let req = Request::Send { offset: self.ttl_offset as u32 };
        let used = postcard::to_slice_cobs(&req, &mut out_buf).unwrap();
        serial::write_port(1, &used[..]).unwrap();
    }
}

#[no_mangle]
pub fn entry() -> ! {
    // First, open Port 1 (we will write to it)
    serial::open_port(1).unwrap();
    let mut buffer = [0u8; 64];
    let mut inp_buffer = [0u8; 64];


    let mut r = Runner::new(&ROOT_MENU, &mut buffer, Context {
        buf: String::new(),
        uploading: None
    });

    loop {
        if let Some(upl) = r.context.uploading.as_mut() {
            upl.process();
        }

        let inp = match serial::read_port(0, &mut inp_buffer) {
            Ok(insl) if insl.len() > 0 => {
                insl
            }
            Ok(_) => {
                if r.context.uploading.is_none() {
                    time::sleep_micros(10_000).ok();
                }
                continue;
            }
            Err(()) => {
                if r.context.uploading.is_none() {
                    time::sleep_micros(10_000).ok();
                }
                continue;
            }
        };

        for b in inp.iter() {
            if *b == b'\n' {
                r.input_byte(b'\r');
            } else {
                r.input_byte(*b);
            }
        }

        if r.context.buf.len() > 0 {
            serial::write_port(0, r.context.buf.as_bytes()).unwrap();
            r.context.buf.clear();
        }
    }
}
