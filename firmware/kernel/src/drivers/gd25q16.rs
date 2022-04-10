use cassette::{Cassette, pin_mut};
use common::syscall::{BlockKind, success::BlockInfo};
use heapless::String;
use postcard::from_bytes;
use serde::{Serialize, Deserialize};

use crate::{traits::BlockStorage, qspi::Qspi, alloc::HEAP};

pub struct Gd25q16 {
    table: BlockTable,
    qspi: Qspi,
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
            let fut = qspi.read(0, &mut data.data);
            pin_mut!(fut);
            let cas = Cassette::new(fut);
            cas.block_on().map_err(drop)?;
        }
        let mut bt: BlockTable = from_bytes(&data.data).map_err(drop)?;
        drop(data);

        if bt.magic != 0xB10C_0000 {
            defmt::println!("Invalid block table! Using blank table...");
            bt.magic = 0xB10C_0000;
            bt.blocks.iter_mut().for_each(|b| *b = Block::default());
            bt.open = [false; 15];

            // TODO: Immediately write it back?
        };

        Ok(Self {
            table: bt,
            qspi,
        })
    }
}

#[derive(Serialize, Deserialize)]
pub struct BlockTable {
    magic: u32,
    blocks: [Block; 15],
    open: [bool; 15],
}

#[derive(Serialize, Deserialize)]
pub struct Block {
    name: String<128>,
    len: u32,
    kind: BlockKind,
}

impl Default for Block {
    fn default() -> Self {
        Self {
            name: String::new(),
            len: 0,
            kind: BlockKind::Unused,
        }
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

    fn block_info<'a>(&'a self, block: u32) -> Result<BlockInfo<'a>, ()> {
        let block = block as usize;
        let binfo = self.table.blocks.get(block).ok_or(())?;
        Ok(BlockInfo {
            length: binfo.len,
            capacity: self.block_size(),
            kind: binfo.kind,
            name: if binfo.kind != BlockKind::Unused {
                Some(binfo.name.as_bytes().into())
            } else {
                None
            },
        })
    }
}
