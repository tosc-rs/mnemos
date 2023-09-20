//! DMAC [`Descriptor`]s configure DMA transfers initiated using the DMAC.
// Most of the code in this module still needs documentation...
#![allow(missing_docs)]
// Unusual groupings are used in binary literals in this file in order to
// separate the bits by which field they represent, rather than by their byte.
#![allow(clippy::unusual_byte_groupings)]

use core::{cmp, fmt, mem, ptr::NonNull};
use d1_pac::generic::{Reg, RegisterSpec};

#[derive(Clone, Debug)]
#[repr(C, align(4))]
pub struct Descriptor {
    configuration: Cfg,
    source_address: u32,
    destination_address: u32,
    byte_counter: u32,
    parameter: u32,
    link: u32,
}

/// A builder for constructing DMA [`Descriptor`]s.
#[derive(Copy, Clone, Debug)]
#[must_use = "a `DescriptorBuilder` does nothing unless `DescriptorBuilder::build()` is called"]
pub struct DescriptorBuilder<S = (), D = ()> {
    cfg: Cfg,
    wait_clock_cycles: u8,
    link: u32,
    source: S,
    dest: D,
}

/// Errors returned by [`DescriptorBuilder::build`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidDescriptor {
    ByteCounterTooLong(usize),
    LinkAddr(InvalidLink),
}

/// Errors returned by [`DescriptorBuilder::source_slice`],
/// [`DescriptorBuilder::dest_slice`], [`DescriptorBuilder::source_reg`], and [`DescriptorBuilder::dest_reg`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidOperand {
    reason: InvalidOperandReason,
    kind: Operand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidOperandReason {
    /// Indicates that the address of the provided operand was too high in
    /// memory. Operand addresses for DMA transfers may not exceed
    /// [`Descriptor::ADDR_MAX`] (34 bits).
    AddrTooHigh(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operand {
    Source,
    Destination,
}

/// Errors returned by [`Descriptor::set_link`] and [`DescriptorBuilder::build`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidLink {
    TooLong(usize),
    Misaligned(usize),
}

mycelium_bitfield::bitfield! {
    struct Cfg<u32> {
        /// DMA source DRQ type.
        const SRC_DRQ_TYPE: SrcDrqType;

        /// DMA source block size.
        const SRC_BLOCK_SIZE: BlockSize;

        /// DMA source address mode.
        const SRC_ADDR_MODE: AddressMode;

        /// DMA source data width.
        const SRC_DATA_WIDTH: DataWidth;

        const _RESERVED_0 = 5;

        /// DMA destination DRQ type
        const DEST_DRQ_TYPE: DestDrqType;

        /// DMA destination block size.
        const DEST_BLOCK_SIZE: BlockSize;

        /// DMA destination address mode.
        const DEST_ADDR_MODE: AddressMode;

        /// DMA destination data width.
        const DEST_DATA_WIDTH: DataWidth;

        const _RESERVED_1 = 3;

        /// BMODE select
        const BMODE_SEL: BModeSel;
    }
}

mycelium_bitfield::enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum SrcDrqType<u8> {
        Sram = 0,
        Dram = 1,
        OwaRx = 2,
        I2sPcm0Rx = 3,
        I2sPcm1Rx = 4,
        I2sPcm2Rx = 5,
        AudioCodec = 7,
        Dmic = 8,
        GpADC = 12,
        TpADC = 13,
        Uart0Rx = 14,
        Uart1Rx = 15,
        Uart2Rx = 16,
        Uart3Rx = 17,
        Uart4Rx = 18,
        Uart5Rx = 19,
        Spi0Rx = 22,
        Spi1Rx = 23,
        Usb0Ep1 = 30,
        Usb0Ep2 = 31,
        Usb0Ep3 = 32,
        Usb0Ep4 = 33,
        Usb0Ep5 = 34,
        Twi0 = 43,
        Twi1 = 44,
        Twi2 = 45,
        Twi3 = 46,
    }
}

mycelium_bitfield::enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum DestDrqType<u8> {
        Sram = 0,
        Dram = 1,
        OwaTx = 2,
        I2sPcm0Tx = 3,
        I2sPcm1Tx = 4,
        I2sPcm2Tx = 5,
        AudioCodec = 7,
        IrTx = 13,
        Uart0Tx = 14,
        Uart1Tx = 15,
        Uart2Tx = 16,
        Uart3Tx = 17,
        Uart4Tx = 18,
        Uart5Tx = 19,
        Spi0Tx = 22,
        Spi1Tx = 23,
        Usb0Ep1 = 30,
        Usb0Ep2 = 31,
        Usb0Ep3 = 32,
        Usb0Ep4 = 33,
        Usb0Ep5 = 34,
        Ledc = 42,
        Twi0 = 43,
        Twi1 = 44,
        Twi2 = 45,
        Twi3 = 46,
}
}

// TODO: Verify bits or bytes?
mycelium_bitfield::enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum BlockSize<u8> {
        Byte1 = 0b00,
        Byte4 = 0b01,
        Byte8 = 0b10,
        Byte16 = 0b11,
    }
}

mycelium_bitfield::enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum AddressMode<u8> {
        LinearMode = 0,
        IoMode = 1,
    }
}

mycelium_bitfield::enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum DataWidth<u8> {
        Bit8 = 0b00,
        Bit16 = 0b01,
        Bit32 = 0b10,
        Bit64 = 0b11,
    }
}

mycelium_bitfield::enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum BModeSel<u8> {
        Normal = 0,
        BMode = 1,
    }
}

impl DescriptorBuilder {
    pub const fn new() -> Self {
        Self {
            cfg: Cfg::new(),
            wait_clock_cycles: 0,
            link: Descriptor::END_LINK,
            source: (),
            dest: (),
        }
    }
}

type DestBuf<'dest> = &'dest mut [mem::MaybeUninit<u8>];

impl<S, D> DescriptorBuilder<S, D> {
    pub fn src_block_size(self, val: BlockSize) -> Self {
        Self {
            cfg: self.cfg.with(Cfg::SRC_BLOCK_SIZE, val),
            ..self
        }
    }

    pub fn src_data_width(self, val: DataWidth) -> Self {
        Self {
            cfg: self.cfg.with(Cfg::SRC_DATA_WIDTH, val),
            ..self
        }
    }

    pub fn dest_block_size(self, val: BlockSize) -> Self {
        Self {
            cfg: self.cfg.with(Cfg::DEST_BLOCK_SIZE, val),
            ..self
        }
    }

    pub fn dest_data_width(self, val: DataWidth) -> Self {
        Self {
            cfg: self.cfg.with(Cfg::DEST_DATA_WIDTH, val),
            ..self
        }
    }

    pub fn bmode_sel(self, val: BModeSel) -> Self {
        Self {
            cfg: self.cfg.with(Cfg::BMODE_SEL, val),
            ..self
        }
    }

    pub fn link(self, link: impl Into<Option<NonNull<Descriptor>>>) -> Result<Self, InvalidLink> {
        let link = link
            .into()
            .map(Descriptor::addr_to_link)
            .transpose()?
            .unwrap_or(Descriptor::END_LINK);
        Ok(Self { link, ..self })
    }

    pub fn wait_clock_cycles(self, wait_clock_cycles: u8) -> Self {
        Self {
            wait_clock_cycles,
            ..self
        }
    }

    /// Sets the provided slice as the source for the DMA transfer. Bytes will
    /// be copied out of this slice to the destination operand of the transfer.
    ///
    /// Since the slice is in memory, this automatically sets the source address
    /// mode to [`AddressMode::LinearMode`] and the source DRQ type to
    /// [`SrcDrqType::Dram`].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`DescriptorBuilder`]`)` with `source` as the source operand,
    ///   if the provided slice's address is a valid DMA source.
    /// - [`Err`]`(`[`InvalidOperand`]`)` if `source` is not a valid DMA
    ///   source address.
    pub fn source_slice(
        self,
        source: &'_ [u8],
    ) -> Result<DescriptorBuilder<&'_ [u8], D>, InvalidOperand> {
        Self::check_addr(source as *const _ as *const (), Operand::Source)?;

        Ok(DescriptorBuilder {
            cfg: self
                .cfg
                .with(Cfg::SRC_ADDR_MODE, AddressMode::LinearMode)
                .with(Cfg::SRC_DRQ_TYPE, SrcDrqType::Dram),
            wait_clock_cycles: self.wait_clock_cycles,
            link: self.link,
            source,
            dest: self.dest,
        })
    }

    /// Sets the provided slice as the destination of the DMA transfer. Bytes will
    /// be copied from the source operand of the transfer into this slice.
    ///
    /// Since the slice is in memory, this automatically sets the destination address
    /// mode to [`AddressMode::LinearMode`].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`DescriptorBuilder`]`)` with `dest` as the destination
    ///   operand, if the provided slice's address is a valid DMA destination.
    /// - [`Err`]`(`[`InvalidOperand`]`)` if `dest` is not a valid DMA
    ///   destination address.
    pub fn dest_slice(
        self,
        dest: DestBuf<'_>,
    ) -> Result<DescriptorBuilder<S, DestBuf<'_>>, InvalidOperand> {
        Self::check_addr(dest as *const _ as *const (), Operand::Destination)?;

        Ok(DescriptorBuilder {
            cfg: self
                .cfg
                .with(Cfg::DEST_ADDR_MODE, AddressMode::LinearMode)
                .with(Cfg::DEST_DRQ_TYPE, DestDrqType::Dram),
            wait_clock_cycles: self.wait_clock_cycles,
            link: self.link,
            source: self.source,
            dest,
        })
    }

    /// Sets the provided pointer to a memory-mapped IO register as the source
    /// for the DMA transfer. Bytes will be copied from this register to the
    /// destination operand of the transfer.
    ///
    /// Since the source is a memory-mapped IO register, this automatically sets
    /// the source address  mode to [`AddressMode::IoMode`]. The provided
    /// [`SrcDrqType`] describes the type of DRQ signal that should be used
    /// when transferring from this register. Note that if this is not the correct
    /// DRQ for this register, the DMA transfer may never complete.
    ///
    /// # Safety
    ///
    /// `source` MUST point to a memory-mapped IO register which is a valid
    /// source for a DMA transfer. Otherwise, you will have a bad time.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`DescriptorBuilder`]`)` with `source` as the source operand,
    ///   if the provided register's address is a valid DMA source.
    /// - [`Err`]`(`[`InvalidOperand`]`)` if `source` is not a valid DMA source.
    pub unsafe fn source_reg<R: RegisterSpec>(
        self,
        source: &Reg<R>,
        drq_type: SrcDrqType,
    ) -> Result<DescriptorBuilder<*const (), D>, InvalidOperand> {
        let source = source.as_ptr().cast() as *const _;
        Self::check_addr(source, Operand::Source)?;

        Ok(DescriptorBuilder {
            cfg: self
                .cfg
                .with(Cfg::SRC_ADDR_MODE, AddressMode::IoMode)
                .with(Cfg::SRC_DRQ_TYPE, drq_type),
            wait_clock_cycles: self.wait_clock_cycles,
            link: self.link,
            source,
            dest: self.dest,
        })
    }

    /// Sets the provided memory-mapped IO register as the destination for the
    /// DMA transfer. Bytes will be copied from the source operand to the
    /// pointed MMIO register.
    ///
    /// Since the destination is a memory-mapped IO register, this automatically sets
    /// the destination address mode to [`AddressMode::IoMode`]. The provided
    /// [`DestDrqType`] describes the type of DRQ signal that should be used
    /// when transferring to this register. Note that if this is not the correct
    /// DRQ for this register, the DMA transfer may never complete.
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`DescriptorBuilder`]`)` with `dest` as the destination operand,
    ///   if the provided register's address is a valid DMA destination.
    /// - [`Err`]`(`[`InvalidOperand`]`)` if `dest` is not a valid DMA source.
    pub fn dest_reg<R: RegisterSpec>(
        self,
        dest: &Reg<R>,
        drq_type: DestDrqType,
    ) -> Result<DescriptorBuilder<S, *mut ()>, InvalidOperand> {
        let dest = dest.as_ptr().cast();
        Self::check_addr(dest as *const _, Operand::Destination)?;

        Ok(DescriptorBuilder {
            cfg: self
                .cfg
                .with(Cfg::DEST_ADDR_MODE, AddressMode::IoMode)
                .with(Cfg::DEST_DRQ_TYPE, drq_type),
            wait_clock_cycles: self.wait_clock_cycles,
            link: self.link,
            dest,
            source: self.source,
        })
    }

    #[inline]
    fn check_addr(addr: *const (), kind: Operand) -> Result<(), InvalidOperand> {
        let addr = addr as usize;
        if addr > Descriptor::ADDR_MAX as usize {
            return Err(InvalidOperand {
                reason: InvalidOperandReason::AddrTooHigh(addr),
                kind,
            });
        }

        Ok(())
    }
}

impl DescriptorBuilder<&'_ [u8], DestBuf<'_>> {
    pub fn build(self) -> Result<Descriptor, InvalidDescriptor> {
        let len: u32 = {
            // if the source buffer is shorter than the dest, we will be copying
            // only `source.len()` bytes into `dest`. if the dest buffer is
            // shorter than `source`, we will be copying only enough bytes to
            // fill the dest.
            let min = cmp::min(self.source.len(), self.dest.len());
            min.try_into()
                .map_err(|_| InvalidDescriptor::ByteCounterTooLong(min))?
        };
        DescriptorBuilder {
            source: self.source.as_ptr().cast(),
            dest: self.dest.as_mut_ptr().cast(),
            wait_clock_cycles: self.wait_clock_cycles,
            link: self.link,
            cfg: self.cfg,
        }
        .build(len)
    }
}

impl DescriptorBuilder<*const (), DestBuf<'_>> {
    pub fn build(self) -> Result<Descriptor, InvalidDescriptor> {
        let len: u32 = self
            .dest
            .len()
            .try_into()
            .map_err(|_| InvalidDescriptor::ByteCounterTooLong(self.dest.len()))?;
        DescriptorBuilder {
            source: self.source,
            dest: self.dest.as_mut_ptr().cast(),
            wait_clock_cycles: self.wait_clock_cycles,
            link: self.link,
            cfg: self.cfg,
        }
        .build(len)
    }
}

impl DescriptorBuilder<&'_ [u8], *mut ()> {
    pub fn build(self) -> Result<Descriptor, InvalidDescriptor> {
        let len: u32 = self
            .source
            .len()
            .try_into()
            .map_err(|_| InvalidDescriptor::ByteCounterTooLong(self.source.len()))?;
        DescriptorBuilder {
            source: self.source.as_ptr().cast(),
            dest: self.dest,
            wait_clock_cycles: self.wait_clock_cycles,
            link: self.link,
            cfg: self.cfg,
        }
        .build(len)
    }
}

impl DescriptorBuilder<*const (), *mut ()> {
    pub fn build(self, len: u32) -> Result<Descriptor, InvalidDescriptor> {
        let source = self.source as usize;
        let destination = self.dest as usize;
        debug_assert!(
            source <= Descriptor::ADDR_MAX as usize,
            "source address should already have been validated"
        );

        debug_assert!(
            destination <= Descriptor::ADDR_MAX as usize,
            "destination address should already have been validated"
        );

        if len > Descriptor::MAX_LEN {
            return Err(InvalidDescriptor::ByteCounterTooLong(len as usize));
        }

        let mut parameter = self.wait_clock_cycles as u32;

        // Set source
        let source_address = source as u32;
        //             332222222222 11 11 11111100 00000000
        //             109876543210 98 76 54321098 76543210
        parameter &= 0b111111111111_11_00_11111111_11111111;
        parameter |= (((source >> 32) & 0b11) << 16) as u32;

        // Set dest
        let destination_address = destination as u32;
        //             332222222222 11 11 11111100 00000000
        //             109876543210 98 76 54321098 76543210
        parameter &= 0b111111111111_00_11_11111111_11111111;
        parameter |= (((destination >> 32) & 0b11) << 18) as u32;

        Ok(Descriptor {
            configuration: self.cfg,
            source_address,
            destination_address,
            byte_counter: len,
            parameter,
            // link address field was already validated by
            // `DescriptorBuilder::link`.
            link: self.link,
        })
    }
}

// Descriptor

impl Descriptor {
    const END_LINK: u32 = 0xFFFF_F800;

    /// Maximum value for the `byte_counter` argument to
    /// [`DescriptorBuilder::build`] --- byte counters must be 25 bits wide or
    /// less.
    pub const MAX_LEN: u32 = (1 << 25) - 1;

    /// Maximum value for the `source` and `destination` arguments to
    /// [`DescriptorBuilder::build`] --- addresses must be 34 bits wide or less.
    pub const ADDR_MAX: u64 = (1 << 34) - 1;

    /// Maximum value for the `link` address passed to
    /// [`DescriptorBuilder::link`] -- link addresses must be 32 bits wide or
    /// less.
    pub const LINK_ADDR_MAX: usize = u32::MAX as usize;

    pub const fn builder() -> DescriptorBuilder {
        DescriptorBuilder::new()
    }

    pub fn set_link(&mut self, link: impl Into<Option<NonNull<Self>>>) -> Result<(), InvalidLink> {
        self.link = link
            .into()
            .map(Self::addr_to_link)
            .transpose()?
            .unwrap_or(Self::END_LINK);
        Ok(())
    }

    fn addr_to_link(link: NonNull<Self>) -> Result<u32, InvalidLink> {
        let addr = link.as_ptr() as usize;
        if addr > Self::LINK_ADDR_MAX {
            return Err(InvalidLink::TooLong(addr));
        }

        if addr & (mem::align_of::<Self>() - 1) > 0 {
            return Err(InvalidLink::Misaligned(addr));
        }

        // We already verified above the low bits of `link` are clear,
        // no need to re-mask them.
        Ok(addr as u32 | ((addr >> 32) as u32) & 0b11)
    }
}

// InvalidDescriptor

impl fmt::Display for InvalidDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidDescriptor::ByteCounterTooLong(counter) => write!(
                f,
                "byte counter {counter} is greater than `Descriptor::BYTE_COUNTER_MAX` ({})",
                Descriptor::MAX_LEN
            ),
            InvalidDescriptor::LinkAddr(error) => fmt::Display::fmt(error, f),
        }
    }
}

// InvalidLink

impl fmt::Display for InvalidLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidLink::TooLong(addr) => write!(
                f,
                "link address {addr:#x} is greater than `Descriptor::LINK_ADDR_MAX` ({:#x})",
                Descriptor::LINK_ADDR_MAX
            ),
            InvalidLink::Misaligned(addr) => {
                write!(f, "link address {addr:#x} is not at least 4-byte aligned!",)
            }
        }
    }
}

// InvalidOperand

impl fmt::Display for InvalidOperand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { reason, kind } = self;
        match kind {
            Operand::Source => f.write_str("invalid source ")?,
            Operand::Destination => f.write_str("invalid destination ")?,
        }
        fmt::Display::fmt(reason, f)
    }
}

impl fmt::Display for InvalidOperandReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AddrTooHigh(addr) => write!(
                f,
                "address {addr:#x} must be less than `Descriptor::ADDR_MAX` ({:#x})",
                Descriptor::ADDR_MAX
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::{prop_assert_eq, proptest};

    use super::*;

    #[test]
    fn configuration_is_valid() {
        Cfg::assert_valid();
    }

    #[derive(proptest_derive::Arbitrary, Debug)]
    struct ArbitraryConfig {
        src_drq_type: SrcDrqType,
        src_block_size: BlockSize,
        src_addr_mode: AddressMode,
        src_data_width: DataWidth,
        dest_drq_type: DestDrqType,
        dest_block_size: BlockSize,
        dest_addr_mode: AddressMode,
        dest_data_width: DataWidth,
        bmode_sel: BModeSel,
    }

    impl ArbitraryConfig {
        fn manual_pack(&self) -> u32 {
            // 6 bits, no shift
            let src_drq_type = ((self.src_drq_type as u8) & 0b11_1111) as u32;
            let src_block_size = (((self.src_block_size as u8) & 0b11) as u32) << 6;
            let src_addr_mode = (((self.src_addr_mode as u8) & 0b1) as u32) << 8;
            let src_data_width = (((self.src_data_width as u8) & 0b11) as u32) << 9;

            let dest_drq_type = (((self.dest_drq_type as u8) & 0b11_1111) as u32) << 16;
            let dest_block_size = (((self.dest_block_size as u8) & 0b11) as u32) << 22;
            let dest_addr_mode = (((self.dest_addr_mode as u8) & 0b1) as u32) << 24;
            let dest_data_width = (((self.dest_data_width as u8) & 0b11) as u32) << 25;

            let bmode_sel = (((self.bmode_sel as u8) & 0b1) as u32) << 30;

            src_drq_type
                | src_block_size
                | src_addr_mode
                | src_data_width
                | dest_drq_type
                | dest_block_size
                | dest_addr_mode
                | dest_data_width
                | bmode_sel
        }
    }

    proptest! {
        #[test]
        fn pack_configuration(cfg: ArbitraryConfig) {
            let mut config = DescriptorBuilder::new()
                .src_block_size(cfg.src_block_size)
                .src_data_width(cfg.src_data_width)
                .dest_block_size(cfg.dest_block_size)
                .dest_data_width(cfg.dest_data_width)
                .bmode_sel(cfg.bmode_sel).cfg;
            config
                .set(Cfg::SRC_ADDR_MODE, cfg.src_addr_mode)
                .set(Cfg::DEST_ADDR_MODE, cfg.dest_addr_mode)
                .set(Cfg::SRC_DRQ_TYPE, cfg.src_drq_type)
                .set(Cfg::DEST_DRQ_TYPE, cfg.dest_drq_type);

            prop_assert_eq!(
                cfg.manual_pack(),
                config.bits(),
                "\n{:032b} (expected), vs:\n{}",
                cfg.manual_pack(),
                config
            );
        }
    }
}
