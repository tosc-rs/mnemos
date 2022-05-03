use crate::{
    alloc::HeapGuard,
    future_box::{FutureBoxExHdl, FutureBoxPendHdl, Source},
    traits::{OutputPin, SpiTransactionKind, SpiHandle, SpiTransaction, Spi, SpimNode},
};
use core::{
    iter::repeat_with,
    sync::atomic::{compiler_fence, Ordering},
};

use embedded_hal::spi::{Mode, MODE_0, MODE_1, MODE_2};
use heapless::{Deque, LinearMap};
use nrf52840_hal::{
    gpio::{Floating, Input, Output, Pin, PushPull},
    pac::SPIM0,
    spim::Frequency,
    target_constants::{EASY_DMA_SIZE, SRAM_LOWER, SRAM_UPPER},
};

enum State {
    Idle,

    // The data is at the front of vdq.
    Transferring { hdl: SpiHandle },
}

struct SpimInner {
    periph: SPIM0,
}

struct InProgress {
    data: FutureBoxExHdl<SpiTransaction>,
    start_offset: usize,
}

pub struct Node {
    node: &'static mut dyn SpimNode,
    vdq: Deque<InProgress, 8>,
    waiting: Deque<FutureBoxPendHdl<SpiTransaction>, 8>,
}

pub struct Spim {
    spi: SpimInner,
    nodes: LinearMap<SpiHandle, Node, 4>,
    state: State,
    cur_idx: u8,
}

impl Spim {
    pub fn new(spim: SPIM0, pins: Pins, mode: Mode) -> Self {
        // Enable certain interrupts
        spim.intenset.modify(|_r, w| {
            w.stopped().set_bit();
            w.end().set_bit();
            w
        });

        Self {
            spi: SpimInner::new(spim, pins, Frequency::M1, mode, 0x00),
            nodes: LinearMap::new(),
            state: State::Idle,
            cur_idx: 0,
        }
    }
}

pub fn new_send_fut(
    heap: &mut HeapGuard,
    hdl: SpiHandle,
    speed_khz: u32,
    count: usize,
) -> Result<FutureBoxExHdl<SpiTransaction>, ()> {
    let data = heap.alloc_box_array(0u8, count)?;
    FutureBoxExHdl::new_exclusive(
        heap,
        SpiTransaction {
            kind: SpiTransactionKind::Send,
            data,
            hdl,
            speed_khz,
        },
        Source::Kernel,
    )
    .map_err(drop)
}

impl Spi for Spim {
    fn register_handle(
        &mut self,
        node: &'static mut dyn SpimNode,
    ) -> Result<SpiHandle, &'static mut dyn SpimNode> {
        if self.nodes.len() == self.nodes.capacity() {
            return Err(node);
        }

        Ok(loop {
            self.cur_idx = self.cur_idx.wrapping_add(1);
            let new_hdl = SpiHandle { idx: self.cur_idx };
            if !self.nodes.contains_key(&new_hdl) {
                let node = Node {
                    node,
                    vdq: Deque::new(),
                    waiting: Deque::new(),
                };
                // We already checked if there was size
                self.nodes.insert(new_hdl, node).ok();
                break new_hdl;
            }
        })
    }

    fn alloc_transaction(
        &mut self,
        heap: &mut HeapGuard,
        kind: SpiTransactionKind,
        hdl: SpiHandle,
        speed_khz: u32,
        count: usize,
    ) -> Option<FutureBoxExHdl<SpiTransaction>> {
        self.alloc_transaction(heap, kind, hdl, speed_khz, count)
    }

    fn start_send(&mut self) {
        self.start_send()
    }

    fn end_send(&mut self) {
        self.end_send()
    }
}

impl Spim {
    pub fn alloc_transaction(
        &mut self,
        heap: &mut HeapGuard,
        kind: SpiTransactionKind,
        hdl: SpiHandle,
        speed_khz: u32,
        count: usize,
    ) -> Option<FutureBoxExHdl<SpiTransaction>> {
        // TODO
        let SpiTransactionKind::Send = kind;

        let handle = match self.nodes.get_mut(&hdl) {
            Some(h) => h,
            None => return None,
        };

        if handle.waiting.is_full() {
            return None;
        }
        let data = heap.alloc_box_array(0u8, count).ok()?;
        let fut = FutureBoxExHdl::new_exclusive(
            heap,
            SpiTransaction {
                kind: SpiTransactionKind::Send,
                data,
                hdl,
                speed_khz,
            },
            Source::Userspace,
        )
        .ok()?;

        let our_hdl = fut.kernel_waiter();
        handle.waiting.push_back(our_hdl).ok()?;

        Some(fut)
    }

    pub fn send(
        &mut self,
        st: FutureBoxExHdl<SpiTransaction>,
    ) -> Result<FutureBoxPendHdl<SpiTransaction>, FutureBoxExHdl<SpiTransaction>> {
        // Does this node exist?
        let handle = match self.nodes.get_mut(&st.hdl) {
            Some(h) => h,
            None => return Err(st),
        };

        let mon = st.create_monitor();

        handle
            .vdq
            .push_back(InProgress {
                data: st,
                start_offset: 0,
            })
            .map_err(|ip| ip.data)?;

        match self.state {
            State::Idle => self.start_send(),
            State::Transferring { .. } => {}
        }

        Ok(mon)
    }

    pub fn flush_waiting(&mut self) {
        for node in self.nodes.values_mut() {
            while !node.vdq.is_full() {
                match node.waiting.pop_front() {
                    Some(pend) => match pend.try_upgrade() {
                        Ok(Some(ready)) => {
                            node.vdq
                                .push_back(InProgress {
                                    data: ready,
                                    start_offset: 0,
                                })
                                .ok();
                        }
                        Ok(None) => {
                            node.waiting.push_front(pend).ok();
                            break;
                        }
                        Err(_) => {
                            defmt::println!("Dropped error");
                        }
                    },
                    None => break,
                }
            }
        }
    }

    pub fn start_send(&mut self) {
        self.flush_waiting();

        match self.state {
            State::Idle => {}
            State::Transferring { .. } => return,
        }

        let mut hdl_node = None;
        for (hdl, node) in self.nodes.iter_mut() {
            let data_ready = !node.vdq.is_empty();
            let node_ready = node.node.is_ready();

            if data_ready && node_ready {
                hdl_node = Some((hdl, node));
                break;
            }
        }

        let (sh, handle) = match hdl_node {
            Some(h) => h,
            None => return,
        };

        let data = match handle.vdq.pop_front() {
            Some(d) => d,
            None => return,
        };

        // TODO: This only handles Send transactions! This is a static assert basically, for now
        let SpiTransactionKind::Send = data.data.kind;

        let rx_data = DmaSlice::null();
        let tx_data = if data.data.data.len() > data.start_offset {
            let sl = &data.data.data[data.start_offset..];
            DmaSlice::from_slice(sl)
        } else {
            return;
        };

        // defmt::println!("[SPI] START");

        self.spi.change_speed(data.data.speed_khz).unwrap();
        handle.node.set_active();

        compiler_fence(Ordering::SeqCst);

        unsafe {
            self.spi.do_spi_dma_transfer_start(tx_data, rx_data);
        }

        // NOTE: We keep the data in the queue, so that the space is reserved, and the
        // consumer can't re-fill it between the start of send and end of send.
        //
        // This should be impossible, since we just freed at least one space here.
        handle.vdq.push_front(data).map_err(drop).unwrap();
        self.state = State::Transferring { hdl: *sh };
    }

    // This should probably be called any time a "stopped" or "end" event occurs. Could be the
    // natural end, or some kind of trigger.
    pub fn end_send(&mut self) {
        self.flush_waiting();

        let mut state = State::Idle;
        core::mem::swap(&mut self.state, &mut state);

        let (mut wip, handle) = match state {
            State::Idle => {
                self.spi.clear_events();
                return;
            }
            State::Transferring { hdl } => {
                if let Some(handle) = self.nodes.get_mut(&hdl) {
                    match handle.vdq.pop_front() {
                        Some(wip) => (wip, handle),
                        None => {
                            self.spi.clear_events();
                            return;
                        }
                    }
                } else {
                    self.spi.clear_events();
                    return;
                }
            }
        };

        // TODO: This only handles Send transactions! This is a static assert basically, for now
        let SpiTransactionKind::Send = wip.data.kind;

        match self.spi.do_spi_dma_transfer_end() {
            Ok((tx_len, _rx_len)) => {
                handle.node.set_inactive();

                compiler_fence(Ordering::SeqCst);

                let txul = tx_len as usize;
                if (txul + wip.start_offset) == wip.data.data.len() {
                    // We are done! Yay! Start the next item and mark the previous as complete
                    wip.data.release_to_complete();
                    // defmt::println!("[SPI] STOP");
                    self.start_send();
                } else {
                    // defmt::println!("[SPI] PAUSE {=usize}", txul);
                    // Uh oh! We stopped early. Assume that was for a reason, and don't autostart.
                    wip.start_offset += txul;

                    // This should be unpossible
                    // TODO: A vecdeque is probably the wrong structure here. We probably ACTUALLY
                    // want a vecdeque for EACH chip select, and do some sort of priority or round
                    // robining of this resource. For now... don't.
                    handle.vdq.push_front(wip).map_err(drop).unwrap();
                }
            }
            Err(e) => panic!("{:?}", e),
        }
    }
}

/// Does this slice reside entirely within RAM?
#[allow(dead_code)]
pub(crate) fn slice_in_ram(slice: &[u8]) -> bool {
    let ptr = slice.as_ptr() as usize;
    ptr >= SRAM_LOWER && (ptr + slice.len()) < SRAM_UPPER
}

/// Return an error if slice is not in RAM.
#[allow(dead_code)]
pub(crate) fn slice_in_ram_or<T>(slice: &[u8], err: T) -> Result<(), T> {
    if slice_in_ram(slice) {
        Ok(())
    } else {
        Err(err)
    }
}

/// A handy structure for converting rust slices into ptr and len pairs
/// for use with EasyDMA. Care must be taken to make sure mutability
/// guarantees are respected
pub(crate) struct DmaSlice {
    ptr: u32,
    len: u32,
}

impl DmaSlice {
    pub fn null() -> Self {
        Self { ptr: 0, len: 0 }
    }

    #[allow(dead_code)]
    pub fn from_slice(slice: &[u8]) -> Self {
        Self {
            ptr: slice.as_ptr() as u32,
            len: slice.len() as u32,
        }
    }
}

impl SpimInner {
    pub fn new(spim: SPIM0, pins: Pins, frequency: Frequency, mode: Mode, orc: u8) -> Self {
        // Select pins.
        spim.psel.sck.write(|w| {
            unsafe { w.bits(pins.sck.psel_bits()) };
            w.connect().connected()
        });

        match pins.mosi {
            Some(mosi) => spim.psel.mosi.write(|w| {
                unsafe { w.bits(mosi.psel_bits()) };
                w.connect().connected()
            }),
            None => spim.psel.mosi.write(|w| w.connect().disconnected()),
        }
        match pins.miso {
            Some(miso) => spim.psel.miso.write(|w| {
                unsafe { w.bits(miso.psel_bits()) };
                w.connect().connected()
            }),
            None => spim.psel.miso.write(|w| w.connect().disconnected()),
        }

        // Enable SPIM instance.
        spim.enable.write(|w| w.enable().enabled());

        // Configure mode.
        spim.config.write(|w| {
            // Can't match on `mode` due to embedded-hal, see https://github.com/rust-embedded/embedded-hal/pull/126
            if mode == MODE_0 {
                w.order().msb_first();
                w.cpol().active_high();
                w.cpha().leading();
            } else if mode == MODE_1 {
                w.order().msb_first();
                w.cpol().active_high();
                w.cpha().trailing();
            } else if mode == MODE_2 {
                w.order().msb_first();
                w.cpol().active_low();
                w.cpha().leading();
            } else {
                w.order().msb_first();
                w.cpol().active_low();
                w.cpha().trailing();
            }
            w
        });

        // Configure frequency.
        spim.frequency.write(|w| w.frequency().variant(frequency));

        // Set over-read character to `0`.
        spim.orc.write(|w|
            // The ORC field is 8 bits long, so `0` is a valid value to write
            // there.
            unsafe { w.orc().bits(orc) });

        SpimInner { periph: spim }
    }

    #[allow(dead_code)]
    fn do_spi_dma_transfer(&mut self, tx: DmaSlice, rx: DmaSlice) -> Result<(), Error> {
        let tx_len = tx.len;
        let rx_len = rx.len;

        unsafe { self.do_spi_dma_transfer_start(tx, rx) };

        loop {
            match self.do_spi_dma_transfer_end() {
                Ok((tx_done, rx_done)) => {
                    break if tx_done != tx_len {
                        Err(Error::Transmit)
                    } else if rx_done != rx_len {
                        Err(Error::Receive)
                    } else {
                        Ok(())
                    }
                }
                Err(Error::NotDone) => continue,
                Err(e) => break Err(e),
            }
        }
    }

    /// Internal helper function to setup and execute SPIM DMA transfer.
    unsafe fn do_spi_dma_transfer_start(&mut self, tx: DmaSlice, rx: DmaSlice) {
        // Conservative compiler fence to prevent optimizations that do not
        // take in to account actions by DMA. The fence has been placed here,
        // before any DMA action has started.
        compiler_fence(Ordering::SeqCst);

        // Set up the DMA write.
        self.periph
            .txd
            .ptr
            .write(|w| unsafe { w.ptr().bits(tx.ptr) });

        self.periph.txd.maxcnt.write(|w|
            // Note that that nrf52840 maxcnt is a wider.
            // type than a u8, so we use a `_` cast rather than a `u8` cast.
            // The MAXCNT field is thus at least 8 bits wide and accepts the full
            // range of values that fit in a `u8`.
            unsafe { w.maxcnt().bits(tx.len as _ ) });

        // Set up the DMA read.
        self.periph.rxd.ptr.write(|w|
            // This is safe for the same reasons that writing to TXD.PTR is
            // safe. Please refer to the explanation there.
            unsafe { w.ptr().bits(rx.ptr) });
        self.periph.rxd.maxcnt.write(|w|
            // This is safe for the same reasons that writing to TXD.MAXCNT is
            // safe. Please refer to the explanation there.
            unsafe { w.maxcnt().bits(rx.len as _) });

        // Start SPI transaction.
        self.periph.tasks_start.write(|w|
            // `1` is a valid value to write to task registers.
            unsafe { w.bits(1) });

        // Conservative compiler fence to prevent optimizations that do not
        // take in to account actions by DMA. The fence has been placed here,
        // after all possible DMA actions have completed.
        compiler_fence(Ordering::SeqCst);
    }

    fn clear_events(&mut self) -> (bool, bool) {
        let is_ended = self.periph.events_end.read().bits() != 0;
        let is_stopped = self.periph.events_stopped.read().bits() != 0;

        // Reset the events, otherwise it will always read `1` from now on.
        if is_ended {
            self.periph.events_end.write(|w| w);
        }
        if is_stopped {
            self.periph.events_stopped.write(|w| w);
        }

        (is_ended, is_stopped)
    }

    fn do_spi_dma_transfer_end(&mut self) -> Result<(u32, u32), Error> {
        // Wait for END event.
        //
        // This event is triggered once both transmitting and receiving are
        // done.
        let (is_ended, is_stopped) = self.clear_events();
        if !(is_ended || is_stopped) {
            return Err(Error::NotDone);
        }

        // Conservative compiler fence to prevent optimizations that do not
        // take in to account actions by DMA. The fence has been placed here,
        // after all possible DMA actions have completed.
        compiler_fence(Ordering::SeqCst);

        let tx_done = self.periph.txd.amount.read().bits();
        let rx_done = self.periph.rxd.amount.read().bits();

        Ok((tx_done, rx_done))
    }

    /// Read and write from a SPI slave, using separate read and write buffers.
    ///
    /// This method implements a complete read transaction, which consists of
    /// the master transmitting what it wishes to read, and the slave responding
    /// with the requested data.
    ///
    /// Uses the provided chip select pin to initiate the transaction. Transmits
    /// all bytes in `tx_buffer`, then receives bytes until `rx_buffer` is full.
    ///
    /// If `tx_buffer.len() != rx_buffer.len()`, the transaction will stop at the
    /// smaller of either buffer.
    #[allow(dead_code)]
    pub fn transfer_split_even(
        &mut self,
        chip_select: &mut dyn OutputPin,
        tx_buffer: &[u8],
        rx_buffer: &mut [u8],
    ) -> Result<(), Error> {
        // NOTE: RAM slice check for `rx_buffer` is not necessary, as a mutable
        // slice can only be built from data located in RAM.
        slice_in_ram_or(tx_buffer, Error::DMABufferNotInDataMemory)?;

        let txi = tx_buffer.chunks(EASY_DMA_SIZE);
        let rxi = rx_buffer.chunks_mut(EASY_DMA_SIZE);

        chip_select.set_pin(false);

        // Don't return early, as we must reset the CS pin
        let res = txi.zip(rxi).try_for_each(|(t, r)| {
            self.do_spi_dma_transfer(DmaSlice::from_slice(t), DmaSlice::from_slice(r))
        });

        chip_select.set_pin(true);

        res
    }

    /// Read and write from a SPI slave, using separate read and write buffers.
    ///
    /// This method implements a complete read transaction, which consists of
    /// the master transmitting what it wishes to read, and the slave responding
    /// with the requested data.
    ///
    /// Uses the provided chip select pin to initiate the transaction. Transmits
    /// all bytes in `tx_buffer`, then receives bytes until `rx_buffer` is full.
    ///
    /// This method is more complicated than the other `transfer` methods because
    /// it is allowed to perform transactions where `tx_buffer.len() != rx_buffer.len()`.
    /// If this occurs, extra incoming bytes will be discarded, OR extra outgoing bytes
    /// will be filled with the `orc` value.
    #[allow(dead_code)]
    pub fn transfer_split_uneven(
        &mut self,
        chip_select: &mut dyn OutputPin,
        tx_buffer: &[u8],
        rx_buffer: &mut [u8],
    ) -> Result<(), Error> {
        // NOTE: RAM slice check for `rx_buffer` is not necessary, as a mutable
        // slice can only be built from data located in RAM.
        if !tx_buffer.is_empty() {
            slice_in_ram_or(tx_buffer, Error::DMABufferNotInDataMemory)?;
        }

        // For the tx and rx, we want to return Some(chunk)
        // as long as there is data to send. We then chain a repeat to
        // the end so once all chunks have been exhausted, we will keep
        // getting Nones out of the iterators.
        let txi = tx_buffer
            .chunks(EASY_DMA_SIZE)
            .map(Some)
            .chain(repeat_with(|| None));

        let rxi = rx_buffer
            .chunks_mut(EASY_DMA_SIZE)
            .map(Some)
            .chain(repeat_with(|| None));

        chip_select.set_pin(false);

        // We then chain the iterators together, and once BOTH are feeding
        // back Nones, then we are done sending and receiving.
        //
        // Don't return early, as we must reset the CS pin.
        let res = txi
            .zip(rxi)
            .take_while(|(t, r)| t.is_some() || r.is_some())
            // We also turn the slices into either a DmaSlice (if there was data), or a null
            // DmaSlice (if there is no data).
            .map(|(t, r)| {
                (
                    t.map(|t| DmaSlice::from_slice(t))
                        .unwrap_or_else(DmaSlice::null),
                    r.map(|r| DmaSlice::from_slice(r))
                        .unwrap_or_else(DmaSlice::null),
                )
            })
            .try_for_each(|(t, r)| self.do_spi_dma_transfer(t, r));

        chip_select.set_pin(true);

        res
    }

    /// Write to an SPI slave.
    ///
    /// This method uses the provided chip select pin to initiate the
    /// transaction, then transmits all bytes in `tx_buffer`. All incoming
    /// bytes are discarded.
    #[allow(dead_code)]
    pub fn write(
        &mut self,
        chip_select: &mut dyn OutputPin,
        tx_buffer: &[u8],
    ) -> Result<(), Error> {
        slice_in_ram_or(tx_buffer, Error::DMABufferNotInDataMemory)?;
        self.transfer_split_uneven(chip_select, tx_buffer, &mut [0u8; 0])
    }

    #[allow(dead_code)]
    fn change_orc(&mut self, orc: u8) {
        self.periph.orc.write(|w| unsafe { w.orc().bits(orc) });
    }

    fn change_speed(&mut self, freq_khz: u32) -> Result<(), ()> {
        let speed = match freq_khz {
            0..=124 => return Err(()),
            125..=249 => Frequency::K125,
            250..=499 => Frequency::K250,
            500..=999 => Frequency::K500,
            1000..=1999 => Frequency::M1,
            2000..=3999 => Frequency::M2,
            4000..=7999 => Frequency::M4,
            8000..=15999 => Frequency::M8,
            16000..=31999 => Frequency::M16,
            _ => Frequency::M32,
        };

        self.periph
            .frequency
            .write(|w| w.frequency().variant(speed));
        Ok(())
    }
}

/// GPIO pins for SPIM interface
pub struct Pins {
    /// SPI clock
    pub sck: Pin<Output<PushPull>>,

    /// MOSI Master out, slave in
    /// None if unused
    pub mosi: Option<Pin<Output<PushPull>>>,

    /// MISO Master in, slave out
    /// None if unused
    pub miso: Option<Pin<Input<Floating>>>,
}

#[derive(Debug)]
pub enum Error {
    TxBufferTooLong,
    RxBufferTooLong,
    /// EasyDMA can only read from data memory, read only buffers in flash will fail.
    DMABufferNotInDataMemory,
    Transmit,
    Receive,
    NotDone,
}
