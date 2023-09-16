// TODO: add docs to these methods...
#![allow(clippy::missing_safety_doc)]

use core::{
    ptr::NonNull,
    sync::atomic::{fence, AtomicU8, Ordering},
};

use d1_pac::{
    dmac::{dmac_desc_addr::DMAC_DESC_ADDR_SPEC, dmac_en::DMAC_EN_SPEC, dmac_mode::DMAC_MODE_SPEC},
    generic::Reg,
    DMAC,
};

use crate::ccu::Ccu;
use kernel::maitake::sync::WaitCell;

use self::descriptor::Descriptor;

pub mod descriptor;

pub struct Channel {
    idx: u8,
    channel: &'static ChannelState,
}

pub struct Dmac {
    used_channels: AtomicU8,
    channels: [ChannelState; Self::CHANNEL_COUNT as usize],
}

struct ChannelState {
    waker: WaitCell,
    state: AtomicU8,
}

static DMAC_STATE: Dmac = {
    // This `const` is used as a static initializer, so clippy is wrong here...
    #[allow(clippy::declare_interior_mutable_const)]
    const NEW_CHANNEL: ChannelState = ChannelState {
        waker: WaitCell::new(),
        state: AtomicU8::new(0),
    };

    Dmac {
        used_channels: AtomicU8::new(u8::MAX),
        channels: [NEW_CHANNEL; Dmac::CHANNEL_COUNT as usize],
    }
};

impl Dmac {
    pub const CHANNEL_COUNT: u8 = 16;
    const UNINITIALIZED: u8 = u8::MAX;

    pub fn initialize(mut dmac: DMAC, ccu: &mut Ccu) {
        DMAC_STATE
            .used_channels
            .compare_exchange(Self::UNINITIALIZED, 0, Ordering::AcqRel, Ordering::Acquire)
            .expect("DMAC cannot be initialized twice!");
        ccu.enable_module(&mut dmac);
    }

    pub fn allocate_channel() -> Option<Channel> {
        DMAC_STATE
            .channels
            .iter()
            .enumerate()
            .find_map(|(idx, channel)| {
                // can we claim this channel?
                channel
                    .state
                    .compare_exchange(
                        ChannelState::UNCLAIMED,
                        ChannelState::CLAIMED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .ok()?;
                DMAC_STATE.used_channels.fetch_add(1, Ordering::AcqRel);
                Some(Channel {
                    idx: idx as u8,
                    channel,
                })
            })
    }

    pub(crate) fn handle_interrupt() {
        let dmac = unsafe { &*DMAC::PTR };
        dmac.dmac_irq_pend0.modify(|r, w| {
            tracing::trace!(dmac_irq_pend0 = ?format_args!("{:#b}", r.bits()), "DMAC interrupt");
            for (i, chan) in DMAC_STATE.channels[..8].iter().enumerate() {
                if unsafe { r.dma_queue_irq_pend(i as u8) }.bit_is_set() {
                    chan.waker.wake();
                }
            }

            // Will write-back and high bits
            w
        });

        dmac.dmac_irq_pend1.modify(|r, w| {
            tracing::trace!(dmac_irq_pend1 = ?format_args!("{:#b}", r.bits()), "DMAC interrupt");
            for (i, chan) in DMAC_STATE.channels[8..].iter().enumerate() {
                if unsafe { r.dma_queue_irq_pend(i as u8) }.bit_is_set() {
                    chan.waker.wake();
                }
            }

            // Will write-back and high bits
            w
        });
    }

    pub(crate) unsafe fn cancel_all() {
        for (i, channel) in DMAC_STATE.channels.iter().enumerate() {
            channel.waker.close();
            Channel {
                idx: i as u8,
                channel,
            }
            .stop_dma();
        }
    }
}

impl ChannelState {
    const UNCLAIMED: u8 = 0;
    const CLAIMED: u8 = 1;
    const IN_FLIGHT: u8 = 2;
}

impl Channel {
    // pub unsafe fn summon_channel(idx: u8) -> Channel {
    //     assert!(idx < Dmac::CHANNEL_COUNT);
    //     Self { idx }
    // }

    pub fn channel_index(&self) -> u8 {
        self.idx
    }

    pub unsafe fn desc_addr_reg(&self) -> &Reg<DMAC_DESC_ADDR_SPEC> {
        let dmac = &*DMAC::PTR;
        match self.idx {
            0 => &dmac.dmac_desc_addr0,
            1 => &dmac.dmac_desc_addr1,
            2 => &dmac.dmac_desc_addr2,
            3 => &dmac.dmac_desc_addr3,
            4 => &dmac.dmac_desc_addr4,
            5 => &dmac.dmac_desc_addr5,
            6 => &dmac.dmac_desc_addr6,
            7 => &dmac.dmac_desc_addr7,
            8 => &dmac.dmac_desc_addr8,
            9 => &dmac.dmac_desc_addr9,
            10 => &dmac.dmac_desc_addr10,
            11 => &dmac.dmac_desc_addr11,
            12 => &dmac.dmac_desc_addr12,
            13 => &dmac.dmac_desc_addr13,
            14 => &dmac.dmac_desc_addr14,
            15 => &dmac.dmac_desc_addr15,
            _ => panic!(),
        }
    }

    pub unsafe fn en_reg(&self) -> &Reg<DMAC_EN_SPEC> {
        let dmac = &*DMAC::PTR;
        match self.idx {
            0 => &dmac.dmac_en0,
            1 => &dmac.dmac_en1,
            2 => &dmac.dmac_en2,
            3 => &dmac.dmac_en3,
            4 => &dmac.dmac_en4,
            5 => &dmac.dmac_en5,
            6 => &dmac.dmac_en6,
            7 => &dmac.dmac_en7,
            8 => &dmac.dmac_en8,
            9 => &dmac.dmac_en9,
            10 => &dmac.dmac_en10,
            11 => &dmac.dmac_en11,
            12 => &dmac.dmac_en12,
            13 => &dmac.dmac_en13,
            14 => &dmac.dmac_en14,
            15 => &dmac.dmac_en15,
            _ => panic!(),
        }
    }

    pub unsafe fn mode_reg(&self) -> &Reg<DMAC_MODE_SPEC> {
        let dmac = &*DMAC::PTR;
        match self.idx {
            0 => &dmac.dmac_mode0,
            1 => &dmac.dmac_mode1,
            2 => &dmac.dmac_mode2,
            3 => &dmac.dmac_mode3,
            4 => &dmac.dmac_mode4,
            5 => &dmac.dmac_mode5,
            6 => &dmac.dmac_mode6,
            7 => &dmac.dmac_mode7,
            8 => &dmac.dmac_mode8,
            9 => &dmac.dmac_mode9,
            10 => &dmac.dmac_mode10,
            11 => &dmac.dmac_mode11,
            12 => &dmac.dmac_mode12,
            13 => &dmac.dmac_mode13,
            14 => &dmac.dmac_mode14,
            15 => &dmac.dmac_mode15,
            _ => panic!(),
        }
    }

    pub async unsafe fn run_descriptor(&mut self, desc: NonNull<Descriptor>) {
        // mark the channel as in-flight.
        let prev_state = self
            .channel
            .state
            .fetch_or(ChannelState::IN_FLIGHT, Ordering::AcqRel);
        assert_eq!(
            prev_state & ChannelState::IN_FLIGHT,
            0,
            "cannot start DMA transfer on a channel that already has an in-flight transfer",
        );

        // pre-subscribe to the waitcell to ensure our waker is registered
        // before starting the DMA transfer.
        let wait = self.channel.waker.subscribe().await;

        // actually start the DMA transfer.
        self.start_descriptor(desc);

        // wait for the DMA transfer to complete.
        let _wait = wait.await;
        debug_assert!(
            _wait.is_ok(),
            "DMA channel WaitCells should never be closed"
        );

        // stop the DMA transfer.
        self.stop_dma();
        self.channel
            .state
            .fetch_and(!ChannelState::IN_FLIGHT, Ordering::Release);
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
        self.channel
            .state
            .fetch_and(!ChannelState::IN_FLIGHT, Ordering::Release);

        fence(Ordering::SeqCst); //////
    }
}

pub enum ChannelMode {
    Wait,
    Handshake,
}
