use core::{
    ptr::NonNull,
    sync::atomic::{fence, Ordering},
};

use d1_pac::{
    dmac::{dmac_desc_addr_reg::DMAC_DESC_ADDR_REG_SPEC, dmac_en_reg::DMAC_EN_REG_SPEC, dmac_mode_reg::DMAC_MODE_REG_SPEC},
    generic::Reg,
    CCU, DMAC,
};

use self::descriptor::Descriptor;

pub mod descriptor;

pub struct Dmac {
    pub dmac: DMAC, // TODO: not this
    pub channels: [Channel; 16],
}

impl Dmac {
    pub fn new(dmac: DMAC, ccu: &mut CCU) -> Self {
        ccu.dma_bgr.write(|w| w.gating().pass().rst().deassert());
        Self {
            dmac,
            channels: [
                Channel { idx: 0 },
                Channel { idx: 1 },
                Channel { idx: 2 },
                Channel { idx: 3 },
                Channel { idx: 4 },
                Channel { idx: 5 },
                Channel { idx: 6 },
                Channel { idx: 7 },
                Channel { idx: 8 },
                Channel { idx: 9 },
                Channel { idx: 10 },
                Channel { idx: 11 },
                Channel { idx: 12 },
                Channel { idx: 13 },
                Channel { idx: 14 },
                Channel { idx: 15 },
            ],
        }
    }
}

pub struct Channel {
    idx: u8,
}

impl Channel {
    pub unsafe fn summon_channel(idx: u8) -> Channel {
        assert!(idx < 16);
        Self { idx }
    }

    pub unsafe fn desc_addr_reg(&self) -> &Reg<DMAC_DESC_ADDR_REG_SPEC> {
        let dmac = &*DMAC::PTR;
        match self.idx {
            0 => &dmac.dmac_desc_addr_reg0,
            1 => &dmac.dmac_desc_addr_reg1,
            2 => &dmac.dmac_desc_addr_reg2,
            3 => &dmac.dmac_desc_addr_reg3,
            4 => &dmac.dmac_desc_addr_reg4,
            5 => &dmac.dmac_desc_addr_reg5,
            6 => &dmac.dmac_desc_addr_reg6,
            7 => &dmac.dmac_desc_addr_reg7,
            8 => &dmac.dmac_desc_addr_reg8,
            9 => &dmac.dmac_desc_addr_reg9,
            10 => &dmac.dmac_desc_addr_reg10,
            11 => &dmac.dmac_desc_addr_reg11,
            12 => &dmac.dmac_desc_addr_reg12,
            13 => &dmac.dmac_desc_addr_reg13,
            14 => &dmac.dmac_desc_addr_reg14,
            15 => &dmac.dmac_desc_addr_reg15,
            _ => panic!(),
        }
    }

    pub unsafe fn en_reg(&self) -> &Reg<DMAC_EN_REG_SPEC> {
        let dmac = &*DMAC::PTR;
        match self.idx {
            0 => &dmac.dmac_en_reg0,
            1 => &dmac.dmac_en_reg1,
            2 => &dmac.dmac_en_reg2,
            3 => &dmac.dmac_en_reg3,
            4 => &dmac.dmac_en_reg4,
            5 => &dmac.dmac_en_reg5,
            6 => &dmac.dmac_en_reg6,
            7 => &dmac.dmac_en_reg7,
            8 => &dmac.dmac_en_reg8,
            9 => &dmac.dmac_en_reg9,
            10 => &dmac.dmac_en_reg10,
            11 => &dmac.dmac_en_reg11,
            12 => &dmac.dmac_en_reg12,
            13 => &dmac.dmac_en_reg13,
            14 => &dmac.dmac_en_reg14,
            15 => &dmac.dmac_en_reg15,
            _ => panic!(),
        }
    }

    pub unsafe fn mode_reg(&self) -> &Reg<DMAC_MODE_REG_SPEC> {
        let dmac = &*DMAC::PTR;
        match self.idx {
            0 => &dmac.dmac_mode_reg0,
            1 => &dmac.dmac_mode_reg1,
            2 => &dmac.dmac_mode_reg2,
            3 => &dmac.dmac_mode_reg3,
            4 => &dmac.dmac_mode_reg4,
            5 => &dmac.dmac_mode_reg5,
            6 => &dmac.dmac_mode_reg6,
            7 => &dmac.dmac_mode_reg7,
            8 => &dmac.dmac_mode_reg8,
            9 => &dmac.dmac_mode_reg9,
            10 => &dmac.dmac_mode_reg10,
            11 => &dmac.dmac_mode_reg11,
            12 => &dmac.dmac_mode_reg12,
            13 => &dmac.dmac_mode_reg13,
            14 => &dmac.dmac_mode_reg14,
            15 => &dmac.dmac_mode_reg15,
            _ => panic!(),
        }
    }

    pub unsafe fn set_channel_modes(&mut self, src: ChannelMode, dst: ChannelMode) {
        self.mode_reg().write(|w| {
            match src {
                ChannelMode::Wait => w.dma_src_mode().waiting(),
                ChannelMode::Handshake => w.dma_src_mode().handshake(),
            };
            match dst {
                ChannelMode::Wait => w.dma_dst_mode().waiting(),
                ChannelMode::Handshake => w.dma_dst_mode().handshake(),
            };
            w
        })
    }

    pub unsafe fn start_descriptor(&mut self, desc: NonNull<Descriptor>) {
        // TODO: Check if channel is idle?

        fence(Ordering::SeqCst); //////

        let desc_addr = desc.as_ptr() as usize;
        self.desc_addr_reg().write(|w| {
            w.dma_desc_addr().variant((desc_addr >> 2) as u32);
            w.dma_desc_high_addr()
                .variant(((desc_addr >> 32) as u8) & 0b11);
            w
        });
        self.en_reg().write(|w| w.dma_en().enabled());

        fence(Ordering::SeqCst); //////
    }

    pub unsafe fn stop_dma(&mut self) {
        self.en_reg().write(|w| w.dma_en().disabled());

        fence(Ordering::SeqCst); //////
    }
}

pub enum ChannelMode {
    Wait,
    Handshake,
}
