// TODO: add docs to these methods...
#![allow(clippy::missing_safety_doc)]

use core::{
    ptr::NonNull,
    sync::atomic::{fence, Ordering},
};

use d1_pac::{
    dmac::{dmac_desc_addr::DMAC_DESC_ADDR_SPEC, dmac_en::DMAC_EN_SPEC, dmac_mode::DMAC_MODE_SPEC},
    generic::Reg,
    DMAC,
};

use crate::ccu::Ccu;
use kernel::maitake::sync::{WaitCell, WaitQueue};
use mnemos_bitslab::index::IndexAlloc16;

use self::descriptor::Descriptor;

pub mod descriptor;

#[derive(Copy, Clone)]
pub struct Dmac {
    // this struct is essentially used as a "yes, the DMAC is initialized now" token...
    _p: (),
}

pub struct Channel {
    idx: u8,
    channel: &'static ChannelState,
}

struct DmacState {
    channels: [ChannelState; Dmac::CHANNEL_COUNT as usize],
    claims: IndexAlloc16,
    claim_wait: WaitQueue,
}

struct ChannelState {
    waker: WaitCell,
}

static DMAC_STATE: DmacState = {
    // This `const` is used as a static initializer, so clippy is wrong here...
    #[allow(clippy::declare_interior_mutable_const)]
    const NEW_CHANNEL: ChannelState = ChannelState {
        waker: WaitCell::new(),
    };

    DmacState {
        channels: [NEW_CHANNEL; Dmac::CHANNEL_COUNT as usize],
        claims: IndexAlloc16::new(),
        claim_wait: WaitQueue::new(),
    }
};

impl Dmac {
    pub const CHANNEL_COUNT: u8 = 16;

    /// Initializes the DMAC, enabling the queue IRQ for all channels.
    pub fn new(mut dmac: DMAC, ccu: &mut Ccu) -> Self {
        /// Sets the `DMA_QUEUE_IRQ_EN` bit for the given channel index.
        fn set_queue_irq_en(idx: u8, bits: u32) -> u32 {
            bits | (1 << queue_irq_en_offset(idx))
        }

        ccu.enable_module(&mut dmac);

        // enable the queue IRQ for all the channels, because use the other
        // channel IRQs (and i don't really understand what they do, because i
        // didn't read the manual).
        critical_section::with(|_cs| unsafe {
            for idx in 0..16 {
                if idx < 8 {
                    // if the channel number is 0-7, it's in the `DMAC_IRQ_EN0` register.
                    dmac.dmac_irq_en0
                        .modify(|r, w| w.bits(set_queue_irq_en(idx, r.bits())));
                } else {
                    // otherwise, if the channel number is 8-15, it's in the
                    // `DMAC_IRQ_EN1` register, instead.
                    dmac.dmac_irq_en1
                        .modify(|r, w| w.bits(set_queue_irq_en(idx - 8, r.bits())));
                }
            }
        });
        Self { _p: () }
    }

    /// Performs a DMA transfer described by the provided [`Descriptor`] on any
    /// available free DMA [`Channel`].
    ///
    /// This function will first use [`Dmac::claim_channel`] to claim an
    /// available DMA channel, waiting for one to become available if all
    /// channels are currently in use. Once a channel has been acquired, it sets
    /// the `src` and `dst` [`ChannelMode`]s using
    /// [`Channel::set_channel_modes`]. Then, it performs the transfer using
    /// [`Channel::transfer`].
    ///
    /// If multiple transfers will be made in sequence, it may be more efficient to
    /// call [`Dmac::claim_channel`] once and perform multiple transfers on the same
    /// channel, to avoid releasing and re-claiming a channel in between transfers.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the descriptor pointed to by `desc`, and the
    /// associated memory region used by the transfer, is valid for as long as
    /// the DMA transfer is active. When this function returns, the transfer has
    /// completed, and it is safe to drop the descriptor. If this future is
    /// cancelled, the transfer is cancelled and the descriptor
    /// and its associated buffer may be dropped safely. However, **it is super
    /// ultra not okay to [`core::mem::forget`] this future**. If you
    /// `mem::forget` a DMA transfer future inside your driver, you deserve
    /// whatever happens next.
    ///
    /// # Cancel Safety
    ///
    /// Dropping this future cancels the DMA transfer. If this future is
    /// dropped, the descriptor and its associated memory region may also be
    /// dropped safely.
    ///
    /// Of course, the transfer may still have completed partially, and if we
    /// were writing to a device, the device may be unhappy to have only gotten
    /// some of the data it wanted. Cancelling an incomplete transfer may result
    /// in, for example, writing out half of a string to the UART, or only part
    /// of a structured message over SPI, and so on. But, at least we don't have
    /// abandoned DMA transfers running around in random parts of the heap you
    /// probably wanted to use for normal stuff like having strings, or whatever
    /// it is that people do on the computer.
    ///
    /// If the DMA transfer was a read rather than a write, cancelling a partial
    /// transfer will have no ill effects whatsoever.[^1]
    ///
    /// [^1]: Unless you actually wanted to have the data you were reading, but
    ///     then you probably wouldn't have cancelled it.
    pub async unsafe fn transfer(
        &self,
        src: ChannelMode,
        dst: ChannelMode,
        desc: NonNull<Descriptor>,
    ) {
        let mut channel = self.claim_channel().await;
        channel.set_channel_modes(src, dst);
        channel.transfer(desc).await
    }

    /// Claims an idle DMA channel, waiting for one to become available if none
    /// are currently idle.
    ///
    /// For a version of this method which does not wait, see [`Dmac::try_claim_channel`].
    pub async fn claim_channel(&self) -> Channel {
        // first, try to claim a channel without registering with the WaitQueue,
        // so that we don't need to lock to remove our wait future from the
        // WaitQueue's linked list. this is a silly eliza optimization and
        // everything will still work fine without it.
        if let Some(channel) = self.try_claim_channel() {
            return channel;
        }

        loop {
            // if no channel was available, register our waker and try again.
            let wait = DMAC_STATE.claim_wait.wait();
            futures::pin_mut!(wait);
            // ensure the `WaitQueue` entry is registered before we actually
            // check the claim state.
            let _ = wait.as_mut().subscribe();

            // try to claim a channel again, returning it if we got one.
            if let Some(channel) = self.try_claim_channel() {
                return channel;
            }

            // welp, someone else got it. oh well. wait for the next one.
            wait.await
                .expect("DMAC channel allocation WaitQueue is never closed.");
        }
    }

    /// Claims an idle DMA channel, if one is available.
    ///
    /// If no DMA channels are currently free, this function returns [`None`].
    /// To instead wait for an active DMA channel to become free, use
    /// [`Dmac::claim_channel`] instead.
    ///
    /// # Returns
    ///
    /// - [`Some`]`(`[`Channel`]`)` containing a free DMA channel ready to use
    ///   for transfers, if one was available.
    /// - [`None`] if all DMA channels are currently in use.
    pub fn try_claim_channel(&self) -> Option<Channel> {
        let idx = DMAC_STATE.claims.allocate()?;
        Some(Channel {
            idx,
            channel: &DMAC_STATE.channels[idx as usize],
        })
    }

    /// Handle a DMAC interrupt.
    pub(crate) fn handle_interrupt() {
        let dmac = unsafe { &*DMAC::PTR };
        // there are two registers that contain DMA channel IRQ status bits,
        // `DMAC_IRQ_PEND0` and `DMAC_IRQ_PEND1`. the first 8 channels (0-7) set
        // bits in `DMA_IRQ_PEND0` when their IRQs fire...
        dmac.dmac_irq_pend0.modify(|r, w| {
            tracing::trace!(dmac_irq_pend0 = ?format_args!("{:#b}", r.bits()), "DMAC interrupt");
            for i in 0..8 {
                if unsafe { r.dma_queue_irq_pend(i) }.bit_is_set() {
                    DMAC_STATE.channels[i as usize].waker.wake();
                }
            }

            // Will write-back any high bits, clearing the interrupt.
            w
        });

        // ...and the second 8 channels (8-15) set their status bits in
        // `DMAC_IRQ_PEND1` instead
        dmac.dmac_irq_pend1.modify(|r, w| {
            tracing::trace!(dmac_irq_pend1 = ?format_args!("{:#b}", r.bits()), "DMAC interrupt");
            for i in 8..16 {
                if unsafe { r.dma_queue_irq_pend(i) }.bit_is_set() {
                    DMAC_STATE.channels[i as usize].waker.wake();
                }
            }

            // Will write-back any high bits, clearing the interrupt.
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

/// Returns the offset of the DMA_QUEUE_IRQ_EN bit for a given channel index.
fn queue_irq_en_offset(idx: u8) -> u8 {
    // Each channel uses 4 bits in the DMAC_IRQ_EN0/DMAC_IRQ_EN1 registers, and
    // the DMA_QUEUE_IRQ_EN bit is the third bit of that four-bit group.
    (idx * 4) + 2
}

impl Channel {
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

    /// Performs a DMA transfer described by the provided [`Descriptor`] on this
    /// channel.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the descriptor pointed to by `desc`, and the
    /// associated memory region used by the transfer, is valid for as long as
    /// the DMA transfer is active. When this function returns, the transfer has
    /// completed, and it is safe to drop the descriptor. If this future is
    /// cancelled, the transfer is cancelled and the descriptor
    /// and its associated buffer may be dropped safely. However, **it is super
    /// ultra not okay to [`core::mem::forget`] this future**. If you
    /// `mem::forget` a DMA transfer future inside your driver, you deserve
    /// whatever happens next.
    ///
    /// # Cancel Safety
    ///
    /// Dropping this future cancels the DMA transfer. If this future is
    /// dropped, the descriptor and its associated memory region may also be
    /// dropped safely.
    ///
    /// Of course, the transfer may still have completed partially, and if we
    /// were writing to a device, the device may be unhappy to have only gotten
    /// some of the data it wanted. Cancelling an incomplete transfer may result
    /// in, for example, writing out half of a string to the UART, or only part
    /// of a structured message over SPI, and so on. But, at least we don't have
    /// abandoned DMA transfers running around in random parts of the heap you
    /// probably wanted to use for normal stuff like having strings, or whatever
    /// it is that people do on the computer.
    ///
    /// If the DMA transfer was a read rather than a write, cancelling a partial
    /// transfer will have no ill effects whatsoever.[^1]
    ///
    /// [^1]: Unless you actually wanted to have the data you were reading, but
    ///     then you probably wouldn't have cancelled it.
    pub async unsafe fn transfer(&mut self, desc: NonNull<Descriptor>) {
        /// Drop guard ensuring that if a `transfer` future is cancelled
        /// before it completes, the in-flight DMA transfer on that channel is dropped.
        struct DefinitelyStop<'chan>(&'chan mut Channel);
        impl Drop for DefinitelyStop<'_> {
            fn drop(&mut self) {
                unsafe { self.0.stop_dma() }
            }
        }

        // pre-subscribe to the waitcell to ensure our waker is registered
        // before starting the DMA transfer.
        // if we're cancelled at this await point, that's fine, because we
        // haven't actually begun a transfer.
        let wait = self.channel.waker.subscribe().await;

        // actually start the DMA transfer.
        self.start_descriptor(desc);

        // ensure the transfer is stopped if this future is cancelled. if we're
        // cancelled at the next await point, it is necessary to ensure the
        // transfer is terminated, which this drop guard ensures.
        let _cancel = DefinitelyStop(self);

        // wait for the DMA transfer to complete.
        let _wait = wait.await;
        debug_assert!(
            _wait.is_ok(),
            "DMA channel WaitCells should never be closed"
        );
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

    /// Begins a DMA transfer *without* waiting for it to complete.
    ///
    /// This is a lower-level API, and you should probably use
    /// [`Channel::transfer`] instead.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the descriptor pointed to by `desc` is valid
    /// for as long as the DMA transfer is active.
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
        fence(Ordering::SeqCst); //////
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        unsafe {
            // tear down any currently active xfer so that the channel can be reused.
            self.stop_dma();
        }
        // free the channel index.
        DMAC_STATE.claims.free(self.idx);
        // if anyone else is waiting for a channel to become available, let them
        // know we're done with ours.
        DMAC_STATE.claim_wait.wake();
    }
}

pub enum ChannelMode {
    Wait,
    Handshake,
}

// Unfortunately, we can't define tests in this crate and have them run on the
// development host machine, because the `mnemos-d1` crate has a `forced-target`
// in its `Cargo.toml`, and will therefore not compile at all for host
// architectures, even just to run tests. In the future, we should look into
// whether it's possible to change our build configurations to allow host tests
// in this crate.
// TODO(eliza): if we can run tests for this crate on the build host, we should
// uncomment these tests.
/*
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dma_queue_irq_en_offset() {
        assert_eq!(dbg!(queue_irq_en_offset(0)), 2);
        assert_eq!(dbg!(queue_irq_en_offset(1)), 6);
        assert_eq!(dbg!(queue_irq_en_offset(2)), 10);
        assert_eq!(dbg!(queue_irq_en_offset(3)), 14);
        assert_eq!(dbg!(queue_irq_en_offset(4)), 18);
        assert_eq!(dbg!(queue_irq_en_offset(5)), 22);
        assert_eq!(dbg!(queue_irq_en_offset(6)), 26);
        assert_eq!(dbg!(queue_irq_en_offset(7)), 30);
    }
}
*/
