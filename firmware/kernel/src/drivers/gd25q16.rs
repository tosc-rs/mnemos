use core::str::FromStr;

use byte_slab::ManagedArcSlab;
use cassette::{Cassette, pin_mut};
use common::syscall::{BlockKind, success::{BlockInfo, BlockStatus}};
use heapless::String;
use postcard::{from_bytes_cobs, to_slice_cobs};
use serde::{Serialize, Deserialize};

use crate::{traits::BlockStorage, qspi::{Qspi, FlashChunk, EraseLength}, alloc::HEAP};

pub struct Gd25q16 {
    table: BlockTable,
    qspi: Qspi,
    status: [BlockStatus; 15],
}

#[repr(align(4))]
#[derive(Clone, Copy)]
struct WordAlign<const N: usize> {
    data: [u8; N]
}

impl Gd25q16 {
    pub fn new(mut qspi: Qspi) -> Result<Self, ()> {
        let mut data = HEAP.try_lock().ok_or(())?.alloc_box(WordAlign { data: [0u8; 4096] })?;
        {
            // Note: do this manually so we don't have to build the block table twice
            let fut = qspi.read(15 * 64 * 1024, &mut data.data);
            pin_mut!(fut);
            let cas = Cassette::new(fut);
            cas.block_on().map_err(drop)?;
        }

        let mut was_bad = false;


        let bt = if let Some(pos) = data.data.iter().position(|b| *b == 0) {
            let bt: BlockTable = from_bytes_cobs(&mut data.data[..pos]).unwrap_or_else(|_| {
                defmt::println!("Failed deserialization!");
                was_bad = true;
                const NEW_BLOCK: Block = Block::new();
                BlockTable {
                    magic: 0xB10C_0000,
                    blocks: [NEW_BLOCK; 15],
                }
            });
            bt
        } else {
            was_bad = true;
            const NEW_BLOCK: Block = Block::new();
            BlockTable {
                magic: 0xB10C_0000,
                blocks: [NEW_BLOCK; 15],
            }
        };

        let mut bd = Self {
            status: [BlockStatus::Idle; 15],
            table: bt,
            qspi,
        };

        if bd.table.magic != 0xB10C_0000 {
            defmt::println!("Failed magic check!");
            was_bad = true;
            bd.table.magic = 0xB10C_0000;
            bd.table.blocks.iter_mut().for_each(|b| *b = Block::new());
        }

        if was_bad {
            defmt::println!("Invalid block table! Writing blank table...");
            let used = to_slice_cobs(&bd.table, data.data.as_mut_slice()).map_err(drop)?.len();
            // Round up to the next word
            let used = ((used + 3) / 4) * 4;
            defmt::println!("Writing: {=[u8]}", &data.data[..used]);
            bd.write(15, 0, &data.data[..used])?;
        };

        defmt::println!("Gd25q16::{:#?}", bd.table);

        Ok(bd)
    }

    // Annoying, because borrow checker.
    fn erase_block(qspi: &mut Qspi, block: u32) -> Result<(), ()> {
        defmt::println!("Erasing block {=u32}...", block);
        let addr = block_offset_to_aligned_addr(block, 0)? as usize;
        let fut = qspi.erase(addr, EraseLength::_64KB);
        pin_mut!(fut);
        let cas_fut = Cassette::new(fut);
        cas_fut.block_on().map_err(drop)
    }

    fn read<'a>(&mut self, block: u32, offset: u32, data: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        match block {
            0..=14 => {
                let stat = self.status.get_mut(block as usize).ok_or(())?;
                match stat {
                    // Must be opened before reading
                    BlockStatus::Idle => {
                        defmt::println!("Tried to read without opening!");
                        return Err(())
                    }
                    _ => {},
                }
            }
            // This is the table block, just let it happen
            15 => {},
            _ => {
                defmt::println!("Invalid block ID for read!");
                return Err(())
            }
        }

        slice_is_aligned(data)?;
        fits_in_dest(offset, data)?;
        let src_addr = block_offset_to_aligned_addr(block, offset)?;

        defmt::println!("Reading {=usize} bytes from QSPI 0x{=u32:08X}", data.len(), src_addr);

        {
            let fut = self.qspi.read(src_addr as usize, data);
            pin_mut!(fut);
            let cas = Cassette::new(fut);
            cas.block_on().map_err(drop)?;
        }

        Ok(data)
    }

    fn write(&mut self, block: u32, offset: u32, data: &[u8]) -> Result<(), ()> {
        match block {
            0..=14 => {
                let stat = self.status.get_mut(block as usize).ok_or(())?;
                let bloc = self.table.blocks.get_mut(block as usize).ok_or(())?;
                match stat {
                    // Must be opened before writing
                    BlockStatus::Idle => {
                        defmt::println!("Tried to write without opening!");
                        return Err(())
                    }

                    // Mark as writes pending
                    BlockStatus::OpenNoWrites => {
                        Self::erase_block(&mut self.qspi, block)?;
                        // TODO: For now, just erase the whole block on first write.
                        // Eventually I should track this... for now, just don't screw
                        // it up.
                        *stat = BlockStatus::OpenWritten;
                        *bloc = Block {
                            name: String::new(),
                            len: 0,
                            kind: BlockKind::Unused,
                        };
                    },

                    // Already pending writes
                    BlockStatus::OpenWritten => {},
                }
            }
            // This is the table block, just let it happen
            15 => {
                Self::erase_block(&mut self.qspi, block)?;
            },
            _ => {
                defmt::println!("Invalid block ID for write!");
                return Err(())
            }
        }

        slice_is_aligned(data)?;
        fits_in_dest(offset, data)?;
        let dest_addr = block_offset_to_aligned_addr(block, offset)?;

        defmt::println!("Writing {=usize} bytes to QSPI 0x{=u32:08X}", data.len(), dest_addr);

        let fut = self.qspi.write(FlashChunk {
            addr: dest_addr as usize,
            data: ManagedArcSlab::<2, 0>::Borrowed(data),
        });
        pin_mut!(fut);
        let cas_fut = Cassette::new(fut);
        cas_fut.block_on().map_err(drop)?;

        Ok(())
    }

    fn close(&mut self, block: u32, name: &str, len: u32, kind: BlockKind) -> Result<(), ()> {
        let status = self.status.get_mut(block as usize).ok_or(())?;
        let bloc = self.table.blocks.get_mut(block as usize).ok_or(())?;
        if len > (64 * 1024) {
            return Err(());
        }
        let name: String<128> = String::from_str(name).map_err(drop)?;
        let mut data = HEAP.try_lock().ok_or(())?.alloc_box(WordAlign { data: [0u8; 4096] })?;

        *status = BlockStatus::Idle;
        *bloc = Block { name, len, kind };

        let used = to_slice_cobs(&self.table, data.data.as_mut_slice()).map_err(drop)?.len();
        // Round up to the next word
        let used = ((used + 3) / 4) * 4;
        self.write(15, 0, &data.data[..used])?;

        Ok(())
    }
}

fn fits_in_dest(offset: u32, data: &[u8]) -> Result<(), ()> {
    let data_len = data.len() as u32;
    if (offset + data_len) <= (64 * 1024) {
        Ok(())
    } else {
        defmt::println!("Data won't fit in block!");
        Err(())
    }
}

fn block_offset_to_aligned_addr(block: u32, offset: u32) -> Result<u32, ()> {
    // 0..=14 are user blocks. Block 15 is for table info
    if block > 15 {
        defmt::println!("Invalid block!");
        return Err(());
    }
    if offset > ((64 * 1024) - 4) {
        defmt::println!("Invalid offset!");
        return Err(());
    }
    if offset % 4 != 0 {
        defmt::println!("Offset improperly aligned!");
        return Err(());
    }

    Ok((block * 64 * 1024) + offset)
}

fn slice_is_aligned(sli: &[u8]) -> Result<(), ()> {
    let addr = sli.as_ptr() as usize;
    let len = sli.len();

    let addr_al = addr % 4 == 0;
    let len_ali = len % 4 == 0;

    if addr_al && len_ali {
        Ok(())
    } else {
        defmt::println!("Data improperly aligned!");
        Err(())
    }
}

#[derive(Serialize, Deserialize, defmt::Format)]
pub struct BlockTable {
    magic: u32,
    blocks: [Block; 15],
}

#[derive(Serialize, Deserialize, defmt::Format)]
pub struct Block {
    name: String<128>,
    len: u32,
    kind: BlockKind,
}

impl Block {
    const fn new() -> Self {
        Self {
            name: String::new(),
            len: 0,
            kind: BlockKind::Unused,
        }
    }
}

impl Default for Block {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockStorage for Gd25q16 {
    fn block_count(&self) -> u32 {
        // For now, we have a fixed size of 2MiB, and a fixed block size of
        // 64KiB per block. At the moment, we also reserve the last block
        // to contain storage info. This means we have 15 blocks available.
        15
    }

    fn block_size(&self) -> u32 {
        // We currently have a fixed block size of 64KiB.
        64 * 1024
    }

    fn block_info<'a>(&self, block: u32, name_buf: &'a mut [u8]) -> Result<BlockInfo<'a>, ()> {
        let block = block as usize;
        let binfo = self.table.blocks.get(block).ok_or(())?;
        let status = *self.status.get(block).ok_or(())?;

        let name = if binfo.kind != BlockKind::Unused {
            let name_bytes = binfo.name.as_bytes();
            let name_len = name_bytes.len();

            if name_buf.len() < name_len {
                return Err(());
            }
            name_buf[..name_len].copy_from_slice(name_bytes);
            Some(name_buf[..name_len].into())
        } else {
            None
        };

        Ok(BlockInfo {
            length: binfo.len,
            capacity: self.block_size(),
            kind: binfo.kind,
            status,
            name,
        })
    }

    fn block_open(&mut self, block: u32) -> Result<(), ()> {
        let block = block as usize;
        let status = self.status.get_mut(block).ok_or(())?;

        match status {
            BlockStatus::Idle => {
                *status = BlockStatus::OpenNoWrites;
            },
            BlockStatus::OpenNoWrites => return Err(()),
            BlockStatus::OpenWritten => return Err(()),
        }

        Ok(())
    }

    fn block_write(&mut self, block: u32, offset: u32, data: &[u8]) -> Result<(), ()> {
        // Don't let the user write the internal table block
        if block < 15 {
            self.write(block, offset, data)
        } else {
            Err(())
        }
    }

    fn block_read<'a>(&mut self, block: u32, offset: u32, data: &'a mut [u8]) -> Result<&'a mut [u8], ()> {
        // Don't let the user read the internal table block
        if block < 15 {
            self.read(block, offset, data)
        } else {
            Err(())
        }
    }

    fn block_close(&mut self, block: u32, name: &str, len: u32, kind: BlockKind) -> Result<(), ()> {
        self.close(block, name, len, kind)
    }
}
