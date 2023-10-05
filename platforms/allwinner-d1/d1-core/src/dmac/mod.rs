//! Higher-level APIs for the Allwinner D1's DMA Controller (DMAC).
#![warn(missing_docs)]
use core::{
    fmt,
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

/// A handle to the DMA controller (DMAC) peripheral.
///
/// A `Dmac` can be used used to claim DMA [`Channel`]s from the DMAC's shared
/// pool of 16 channels, using the [`claim_channel`](Self::claim_channel) and
/// [`try_claim_channel`](Self::try_claim_channel) methods. Once a [`Channel`] has
/// been claimed, it may be used to perform DMA transfers using the
/// [`Channel::transfer`] method. Dropping a [`Channel`] releases it back to the
/// DMAC's channel pool, allowing it to be claimed by other drivers.
///
/// The `Dmac` also provides a [`Dmac::transfer`] convenience method, which
/// claims a channel to perform a single transfer and immediately releases it
/// back to the pool once the transfer has completed.
///
/// This struct is constructed using [`Dmac::new`], which initializes the DMA
/// controller and returns a `Dmac`. Since this struct is essentially a token
/// representing that the DMAC has been initialized, it may be freely copied
/// into any driver that wishes to perform DMA oeprations.
#[derive(Copy, Clone)]
pub struct Dmac {
    // this struct is essentially used as a "yes, the DMAC is initialized now" token...
    _p: (),
}

/// A DMA channel.
///
/// Channels are used to perform DMA transfers using the [`Channel::transfer`]
/// method. Before performing a transfer, a channel must be configured with the
/// desired [`ChannelMode`]s using [`Channel::set_channel_modes`].
///
/// The DMA controller owns a shared pool of 16 DMA channels, which may be used
/// by drivers to initiate DMA transfers. Channels can be acquired from the
/// shared pool using the [`Dmac::claim_channel`] and
/// [`Dmac::try_claim_channel`] methods. Dropping a `Channel` releases it back
/// to the shared pool, allowing it to be claimed by other drivers.
pub struct Channel {
    idx: u8,
    xfer_done: &'static WaitCell,
}

/// DMA channel modes.
///
/// These configure the behavior of a DMA channel when a transfer completes. The
/// source and destination modes of a [`Channel`] may be configured using
/// [`Channel::set_channel_modes`].
#[derive(Copy, Clone, Debug)]
pub enum ChannelMode {
    /// DMA transfer wait mode.
    ///
    /// In this mode, the DMAC will wait for a configurable number of clock
    /// cycles before automatically starting the next transfer.
    ///
    /// The Allwinner documentation for the D1 describes this mode as follows:
    ///
    /// > * When the DMAC detects a valid external request signal, the DMAC
    /// >   starts to operate the peripheral device. The internal DRQ always
    /// >   holds high before the transferred data amount reaches the
    /// >   transferred block length.
    /// > * When the transferred data amount reaches the transferred block
    /// >   length, the internal DRQ pulls low automatically.
    /// > * The internal DRQ holds low for certain clock cycles (W`AIT_CYC`),
    /// >   and then the DMAC restarts to detect the external requests. If the
    /// >   external request signal is valid, then the next transfer starts.
    Wait,
    /// DMA transfer handshake mode.
    ///
    /// In this mode, the DMAC sends the peripheral a DMA Ack signal when the
    /// transfer completes, and waits for the peripheral to pull the DMA
    /// Active signal low before starting the next transfer.
    ///
    /// The Allwinner documentationh for the D1 describes this mode as follows:
    ///
    /// > * When the DMAC detects a valid external request signal, the DMAC
    /// >   starts to operate the peripheral device. The internal DRQ always
    /// >   holds high before the transferred data amount reaches the
    /// >   transferred block length.
    /// > * When the transferred data amount reaches the transferred block
    /// >   length, the internal DRQ will be pulled down automatically. For the
    /// >   last data transfer of the block, the DMAC sends a DMA Last signal
    /// >   with the DMA commands to the peripheral device. The DMA Last signal
    /// >   will be packed as part of the DMA commands and transmitted on the
    /// >   bus. It is used to inform the peripheral device that it is the end
    /// >   of the data transfer for the current DRQ.
    /// > * When the peripheral device receives the DMA Last signal, it can
    /// >   judge that the data transfer for the current DRQ is finished. To
    /// >   continue the data transfer, it sends a DMA Active signal to the
    /// >   DMAC.
    /// >   **Note**: One DMA Active signal will be converted to one DRQ signal
    /// >   in the DMA module. To generate multiple DRQs, the peripheral device
    /// >   needs to send out multiple DMA Active signals via the bus protocol.
    /// > * When the DMAC received the DMA Active signal, it sends back a DMA
    /// >   ACK signal to the peripheral device.
    /// > * When the peripheral device receives the DMA ACK signal, it waits for
    /// >   all the operations on the local device completed, and both the FIFO
    /// >   and DRQ status refreshed. Then it invalidates the DMA Active signal.
    /// > * When the DMAC detects the falling edge of the DMA Active signal, it
    /// >   invalidates the corresponding DMA ACK signal, and restarts to detect
    /// >   the external request signals. If a valid request signal is detected,
    /// >   the next data transfer starts.
    Handshake,
}

/// Internal shared state for the DMAC driver.
struct DmacState {
    /// WaitCells for DMA channel IRQs, one per channel.
    channel_wait: [WaitCell; Dmac::CHANNEL_COUNT as usize],
    /// Index allocator tracking channel claimedness.
    claims: IndexAlloc16,
    /// WaitQueue for notifing tasks waiting for a free channel when one is
    /// freed.
    claim_wait: WaitQueue,
}

static STATE: DmacState = {
    // This `const` is used as a static initializer, so clippy is wrong here...
    #[allow(clippy::declare_interior_mutable_const)]
    const NEW_WAITCELL: WaitCell = WaitCell::new();

    DmacState {
        channel_wait: [NEW_WAITCELL; Dmac::CHANNEL_COUNT as usize],
        claims: IndexAlloc16::new(),
        claim_wait: WaitQueue::new(),
    }
};

// === impl Dmac ==

impl Dmac {
    /// The total number of DMA channels available on the DMAC.
    pub const CHANNEL_COUNT: u8 = 16;

    /// Initializes the DMAC, enabling the queue IRQ for all channels.
    #[must_use]
    pub fn new(mut dmac: DMAC, ccu: &mut Ccu) -> Self {
        /// Sets the `DMA_QUEUE_IRQ_EN` bit for the given channel index.
        fn set_queue_irq_en(idx: u8, bits: u32) -> u32 {
            bits | (1 << queue_irq_en_offset(idx))
        }

        ccu.enable_module(&mut dmac);

        // enable the queue IRQ (`DMA_QUEUE_IRQ_EN`) for all the channels. we
        // can get away with doing that for all channels in initialization
        // (rather than doing it channel-by-channel as channels are actually
        // allocated), because the DMAC will only fire an IRQ for a channel if
        // we've actually started a DMA xfer on that channel. and, we currently
        // don't ever use the other two channel IRQs (`DMA_PKG_IRQ_EN` and
        // `DMA_HLAF_IRQ_EN`) --- i don't really understand what those actually
        // do, because i didn't read the manual.
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
    /// Of course, the transfer may still have completed partially. If we
    /// were writing to a device, the device may be unhappy to have only gotten
    /// some of the data it wanted. If we were reading from a device, reads may
    /// have side effects and incomplete reads may leave the device in a weird
    /// state. Cancelling an incomplete transfer may result in, for example,
    /// writing out half of a string to the UART, or only part  of a structured
    /// message over SPI, and so on. But, at least we don't have abandoned DMA
    /// transfers running around in random parts of the heap you probably wanted
    /// to use for normal stuff like having strings, or whatever it is that
    /// people do on the computer.
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

    /// Claims an idle DMA [`Channel`] from the DMAC's channel pool, waiting for
    /// one to become available if none are currently idle.
    ///
    /// For a version of this method which does not wait, see
    /// [`Dmac::try_claim_channel`].
    ///
    /// # Cancel Safety
    ///
    /// This future can be cancelled freely with no potential negative
    /// consequences. Dropping this future cancels the attempt to claim a
    /// [`Channel`] from the DMAC pool. If a channel has been acquired, it will
    /// be released back to the pool, and may be acquired by other tasks.
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
            let wait = STATE.claim_wait.wait();
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
        let idx = STATE.claims.allocate()?;
        Some(Channel {
            idx,
            xfer_done: &STATE.channel_wait[idx as usize],
        })
    }

    /// Handle a DMAC interrupt.
    pub fn handle_interrupt() {
        let dmac = unsafe { &*DMAC::PTR };
        // there are two registers that contain DMA channel IRQ status bits,
        // `DMAC_IRQ_PEND0` and `DMAC_IRQ_PEND1`. the first 8 channels (0-7) set
        // bits in `DMA_IRQ_PEND0` when their IRQs fire...
        dmac.dmac_irq_pend0.modify(|r, w| {
            tracing::trace!(dmac_irq_pend0 = ?format_args!("{:#b}", r.bits()), "DMAC interrupt");
            for i in 0..8 {
                if unsafe { r.dma_queue_irq_pend(i) }.bit_is_set() {
                    STATE.wake_channel(i);
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
                    STATE.wake_channel(i);
                }
            }

            // Will write-back any high bits, clearing the interrupt.
            w
        });
    }

    /// Cancel *all* currently active DMA transfers.
    ///
    /// This is generally used when shutting down the system, such as in panic
    /// and exception handlers.
    ///
    /// # Safety
    ///
    /// Cancelling DMA transfers abruptly might put peripherals in a weird state
    /// i guess?
    pub unsafe fn cancel_all() {
        for (i, channel) in STATE.channel_wait.iter().enumerate() {
            channel.close();
            Channel {
                idx: i as u8,
                xfer_done: channel,
            }
            .stop_dma();
        }
    }
}

impl fmt::Debug for Dmac {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dmac").finish()
    }
}

// === impl DmacState ===

impl DmacState {
    #[inline]
    fn wake_channel(&self, idx: u8) {
        self.channel_wait[idx as usize].wake();
    }
}

// === impl Channel ===

impl Channel {
    /// Returns the channel index of this channel, from 0 to 15.
    #[inline]
    #[must_use]
    pub fn channel_index(&self) -> u8 {
        self.idx
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
    /// were writing to a device, the device may be unhappy to have only gotten
    /// some of the data it wanted. If we were reading from a device, reads may
    /// have side effects and incomplete reads may leave the device in a weird
    /// state. Cancelling an incomplete transfer may result in, for example,
    /// writing out half of a string to the UART, or only part  of a structured
    /// message over SPI, and so on. But, at least we don't have abandoned DMA
    /// transfers running around in random parts of the heap you probably wanted
    /// to use for normal stuff like having strings, or whatever it is that
    /// people do on the computer.
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
        let xfer_done = self.xfer_done.subscribe().await;

        // actually start the DMA transfer.
        self.start_descriptor(desc);

        // ensure the transfer is stopped if this future is cancelled. if we're
        // cancelled at the next await point, it is necessary to ensure the
        // transfer is terminated, which this drop guard ensures.
        let _cancel = DefinitelyStop(self);

        // wait for the DMA transfer to complete.
        let _wait = xfer_done.await;
        debug_assert!(
            _wait.is_ok(),
            "DMA channel WaitCells should never be closed"
        );
    }

    /// Sets the source and destination [`ChannelMode`] for this channel.
    ///
    /// This configures the behavior of the two sides of the channel.
    ///
    /// # Safety
    ///
    /// This method should only be used when a DMA transfer is not currently in
    /// flight on this channel. This is ensured when using the
    /// [`Channel::transfer`] method, which mutably borrows the channel while
    /// the transfer is in progress, preventing the channel modes from being
    /// changed.
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

    /// Returns the raw `DMAC_DESC_ADDR` register corresponding to this channel.
    ///
    /// # Safety
    ///
    /// Manipulation of raw MMIO registers is generally unsafe. This method
    /// aliases the DMAC MMIO register block, and therefore should only be
    /// called within a critical section or while DMAC interrupts are disabled.
    ///
    /// Manipulating a channel's MMIO register block while a transfer is in
    /// progress on that channel is probably a bad idea. Using the
    /// [`Channel::transfer`] method, which mutably borrows the channel while
    /// the transfer is in progress, will prevent this method from being called
    /// until the transfer completes. However, if a transfer is started with
    /// [`Channel::start_descriptor`], it is possible to manipulate the channel
    /// register block while a transfer is in progress. I don't know what
    /// happens if you do this, but it's probably bad.
    unsafe fn desc_addr_reg(&self) -> &Reg<DMAC_DESC_ADDR_SPEC> {
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
            idx => unreachable!(
                "the DMAC only has 16 channels, but we somehow attempted to access
                nonexistent channel {idx}",
            ),
        }
    }

    /// Returns the raw `DMAC_EN` register corresponding to this channel.
    ///
    /// # Safety
    ///
    /// Manipulation of raw MMIO registers is generally unsafe. This method
    /// aliases the DMAC MMIO register block, and therefore should only be
    /// called within a critical section or while DMAC interrupts are disabled.
    ///
    /// Manipulating a channel's MMIO register block while a transfer is in
    /// progress on that channel is probably a bad idea. Using the
    /// [`Channel::transfer`] method, which mutably borrows the channel while
    /// the transfer is in progress, will prevent this method from being called
    /// until the transfer completes. However, if a transfer is started with
    /// [`Channel::start_descriptor`], it is possible to manipulate the channel
    /// register block while a transfer is in progress. I don't know what
    /// happens if you do this, but it's probably bad.
    unsafe fn en_reg(&self) -> &Reg<DMAC_EN_SPEC> {
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
            idx => unreachable!(
                "the DMAC only has 16 channels, but we somehow attempted to access
                nonexistent channel {idx}",
            ),
        }
    }

    /// Returns the raw `DMAC_MODE` register corresponding to this channel.
    ///
    /// # Safety
    ///
    /// Manipulation of raw MMIO registers is generally unsafe. This method
    /// aliases the DMAC MMIO register block, and therefore should only be
    /// called within a critical section or while DMAC interrupts are disabled.
    ///
    /// Manipulating a channel's MMIO register block while a transfer is in
    /// progress on that channel is probably a bad idea. Using the
    /// [`Channel::transfer`] method, which mutably borrows the channel while
    /// the transfer is in progress, will prevent this method from being called
    /// until the transfer completes. However, if a transfer is started with
    /// [`Channel::start_descriptor`], it is possible to manipulate the channel
    /// register block while a transfer is in progress. I don't know what
    /// happens if you do this, but it's probably bad.
    unsafe fn mode_reg(&self) -> &Reg<DMAC_MODE_SPEC> {
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
            idx => unreachable!(
                "the DMAC only has 16 channels, but we somehow attempted to access
                nonexistent channel {idx}",
            ),
        }
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
    ///
    /// The caller must not initiate another transfer on this channel until the
    /// transfer started using `start_descriptor` completes. I don't know what
    /// happens if you do this, but I'm sure it's bad.
    unsafe fn start_descriptor(&mut self, desc: NonNull<Descriptor>) {
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

    /// Cancel any DMA transfer currently in progress on this channel.
    ///
    /// This is a lower-level API, and you should probably use
    /// [`Channel::transfer`] instead, as it stops the transfer automatically
    /// once it has completed or when the future is dropped.
    ///
    /// # Safety
    ///
    /// This is actually pretty safe. AFAICT, calling `stop_dma` on a channel
    /// with no transfer currently in flight seems fine, actually. But, this
    /// does a raw MMIO register write, so.
    unsafe fn stop_dma(&mut self) {
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
        STATE.claims.free(self.idx);
        // if anyone else is waiting for a channel to become available, let them
        // know we're done with ours.
        STATE.claim_wait.wake();
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Channel").field(&self.idx).finish()
    }
}

// === helpers ===

/// Returns the offset of the DMA_QUEUE_IRQ_EN bit for a given channel index.
fn queue_irq_en_offset(idx: u8) -> u8 {
    // Each channel uses 4 bits in the DMAC_IRQ_EN0/DMAC_IRQ_EN1 registers, and
    // the DMA_QUEUE_IRQ_EN bit is the third bit of that four-bit group.
    (idx * 4) + 2
}

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
