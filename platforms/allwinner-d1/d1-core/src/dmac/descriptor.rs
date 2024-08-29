//! DMAC [`Descriptor`]s configure DMA transfers initiated using the DMAC.
// Most of the code in this module still needs documentation...
#![allow(missing_docs)]
// Unusual groupings are used in binary literals in this file in order to
// separate the bits by which field they represent, rather than by their byte.
#![allow(clippy::unusual_byte_groupings)]

use core::{cmp, mem, ptr::NonNull};

use d1_pac::generic::{Reg, RegisterSpec};
use mycelium_bitfield::{bitfield, enum_from_bits};

use self::errors::*;

#[derive(Clone, Debug)]
#[repr(C, align(4))]
pub struct Descriptor {
    configuration: Cfg,
    source_address: u32,
    destination_address: u32,
    byte_counter: u32,
    parameter: Param,
    link: u32,
}

/// A builder for constructing DMA [`Descriptor`]s.
#[derive(Copy, Clone, Debug)]
#[must_use = "a `DescriptorBuilder` does nothing unless `DescriptorBuilder::build()` is called"]
pub struct DescriptorBuilder<S = (), D = ()> {
    cfg: Cfg,
    param: Param,
    link: u32,
    source: S,
    dest: D,
}

enum_from_bits! {
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

enum_from_bits! {
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
enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum BlockSize<u8> {
        Byte1 = 0b00,
        Byte4 = 0b01,
        Byte8 = 0b10,
        Byte16 = 0b11,
    }
}

enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum AddressMode<u8> {
        LinearMode = 0,
        IoMode = 1,
    }
}

enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum DataWidth<u8> {
        Bit8 = 0b00,
        Bit16 = 0b01,
        Bit32 = 0b10,
        Bit64 = 0b11,
    }
}

enum_from_bits! {
    #[derive(Debug, Eq, PartialEq)]
    #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
    pub enum BModeSel<u8> {
        Normal = 0,
        BMode = 1,
    }
}

bitfield! {
    /// A DMAC descriptor `Configuration` field.
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

bitfield! {
    /// A DMAC descriptor `Parameter` field.
    struct Param<u32> {
        /// Wait clock cycles.
        ///
        /// Sets the wait time in DRQ mode.
        const WAIT_CLOCK_CYCLES: u8;

        const _RESERVED_0 = 8;

        /// The highest two bits of the 34-bit source address.
        const SRC_HIGH = 2;

        /// The highest two bits of the 34-bit destination address.
        const DEST_HIGH = 2;

    }
}

impl Default for DescriptorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DescriptorBuilder {
    pub const fn new() -> Self {
        Self {
            cfg: Cfg::new(),
            param: Param::new(),
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
            param: self.param.with(Param::WAIT_CLOCK_CYCLES, wait_clock_cycles),
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
    /// - [`Ok`]`(`[`DescriptorBuilder`]`)` with `dest` as the destination
    ///   operand, if the provided slice's address is a valid DMA destination.
    /// - [`Err`]`(`[`InvalidOperand`]`)` with [`InvalidOperandReason::TooLong`]
    ///   if the provided slice is longer than [`Descriptor::MAX_LEN`].
    /// - [`Err`]`(`[`InvalidOperand`]`)` with
    ///   [`InvalidOperandReason::AddrTooHigh`] if the provided slice's address
    ///   is higher than [`Descriptor::ADDR_MAX`].
    pub fn source_slice(
        self,
        source: &'_ [u8],
    ) -> Result<DescriptorBuilder<&'_ [u8], D>, InvalidOperand> {
        let high_bits = Self::high_bits(source as *const _ as *const (), Operand::Source)?;

        if source.len() > Descriptor::MAX_LEN as usize {
            return Err(InvalidOperand::too_long(Operand::Source, source.len()));
        }

        Ok(DescriptorBuilder {
            cfg: self
                .cfg
                .with(Cfg::SRC_ADDR_MODE, AddressMode::LinearMode)
                .with(Cfg::SRC_DRQ_TYPE, SrcDrqType::Dram),
            param: self.param.with(Param::SRC_HIGH, high_bits),
            link: self.link,
            source,
            dest: self.dest,
        })
    }

    /// Sets the provided slice as the destination of the DMA transfer. Bytes will
    /// be copied from the source operand of the transfer into this slice.
    ///
    /// Since the slice is in memory, this automatically sets the destination address
    /// mode to [`AddressMode::LinearMode`], and the destination DRQ type to
    /// [`DestDrqType::Dram].
    ///
    /// # Returns
    ///
    /// - [`Ok`]`(`[`DescriptorBuilder`]`)` with `dest` as the destination
    ///   operand, if the provided slice's address is a valid DMA destination.
    /// - [`Err`]`(`[`InvalidOperand`]`)` with [`InvalidOperandReason::TooLong`]
    ///   if the provided slice is longer than [`Descriptor::MAX_LEN`].
    /// - [`Err`]`(`[`InvalidOperand`]`)` with
    ///   [`InvalidOperandReason::AddrTooHigh`] if the provided slice's address
    ///   is higher than [`Descriptor::ADDR_MAX`].
    pub fn dest_slice(
        self,
        dest: DestBuf<'_>,
    ) -> Result<DescriptorBuilder<S, DestBuf<'_>>, InvalidOperand> {
        let high_bits = Self::high_bits(dest as *const _ as *const (), Operand::Destination)?;

        if dest.len() > Descriptor::MAX_LEN as usize {
            return Err(InvalidOperand::too_long(Operand::Destination, dest.len()));
        }

        Ok(DescriptorBuilder {
            cfg: self
                .cfg
                .with(Cfg::DEST_ADDR_MODE, AddressMode::LinearMode)
                .with(Cfg::DEST_DRQ_TYPE, DestDrqType::Dram),
            param: self.param.with(Param::DEST_HIGH, high_bits),
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
    /// - [`Err`]`(`[`InvalidOperand`]`)` with
    ///   [`InvalidOperandReason::AddrTooHigh`] if the provided register's address
    ///   is higher than [`Descriptor::ADDR_MAX`].
    pub fn source_reg<R: RegisterSpec>(
        self,
        source: &Reg<R>,
        drq_type: SrcDrqType,
    ) -> Result<DescriptorBuilder<*const (), D>, InvalidOperand> {
        let source = source.as_ptr().cast() as *const _;
        let high_bits = Self::high_bits(source, Operand::Source)?;

        Ok(DescriptorBuilder {
            cfg: self
                .cfg
                .with(Cfg::SRC_ADDR_MODE, AddressMode::IoMode)
                .with(Cfg::SRC_DRQ_TYPE, drq_type),
            param: self.param.with(Param::SRC_HIGH, high_bits),
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
    /// - [`Err`]`(`[`InvalidOperand`]`)` with
    ///   [`InvalidOperandReason::AddrTooHigh`] if the provided register's address
    ///   is higher than [`Descriptor::ADDR_MAX`].
    pub fn dest_reg<R: RegisterSpec>(
        self,
        dest: &Reg<R>,
        drq_type: DestDrqType,
    ) -> Result<DescriptorBuilder<S, *mut ()>, InvalidOperand> {
        let dest = dest.as_ptr().cast();
        let high_bits = Self::high_bits(dest as *const _, Operand::Destination)?;

        Ok(DescriptorBuilder {
            cfg: self
                .cfg
                .with(Cfg::DEST_ADDR_MODE, AddressMode::IoMode)
                .with(Cfg::DEST_DRQ_TYPE, drq_type),
            param: self.param.with(Param::DEST_HIGH, high_bits),
            link: self.link,
            dest,
            source: self.source,
        })
    }

    #[inline]
    fn high_bits(addr: *const (), kind: Operand) -> Result<u32, InvalidOperand> {
        let addr = addr as usize;
        if addr > Descriptor::ADDR_MAX as usize {
            return Err(InvalidOperand::addr_too_high(kind, addr));
        }

        Ok((addr >> 32 & 0b11) as u32)
    }

    /// This method assumes that the value of `byte_counter`, as well as the
    /// source, destination, and link addresses, have already been validated.
    #[inline]
    fn build_inner(self, source_addr: usize, dest_addr: usize, byte_counter: u32) -> Descriptor {
        debug_assert!(
            source_addr <= Descriptor::ADDR_MAX as usize,
            "source address should already have been validated"
        );

        debug_assert!(
            dest_addr <= Descriptor::ADDR_MAX as usize,
            "destination address should already have been validated"
        );

        debug_assert!(
            byte_counter <= Descriptor::MAX_LEN,
            "byte counter length should already have been validated"
        );

        debug_assert!(
            self.link <= Descriptor::LINK_ADDR_MAX as u32,
            "link address should already have been validated"
        );

        Descriptor {
            configuration: self.cfg,
            source_address: source_addr as u32,
            destination_address: dest_addr as u32,
            byte_counter,
            parameter: self.param,
            // link address field was already validated by
            // `DescriptorBuilder::link`.
            link: self.link,
        }
    }
}

impl DescriptorBuilder<&'_ [u8], DestBuf<'_>> {
    pub fn build(self) -> Descriptor {
        // if the source buffer is shorter than the dest, we will be copying
        // only `source.len()` bytes into `dest`. if the dest buffer is
        // shorter than `source`, we will be copying only enough bytes to
        // fill the dest.
        let len = cmp::min(self.source.len(), self.dest.len()) as u32;
        let dest = self.dest.as_mut_ptr() as *mut _ as usize;
        let source = self.source.as_ptr() as *const _ as usize;
        self.build_inner(source, dest, len)
    }
}

impl DescriptorBuilder<*const (), DestBuf<'_>> {
    pub fn build(self) -> Descriptor {
        let len = self.dest.len() as u32;
        let source = self.source as usize;
        let dest = self.dest.as_mut_ptr() as *mut _ as usize;
        self.build_inner(source, dest, len)
    }
}

impl DescriptorBuilder<&'_ [u8], *mut ()> {
    pub fn build(self) -> Descriptor {
        let len = self.source.len() as u32;
        self.build_inner(
            self.source.as_ptr() as *const _ as usize,
            self.dest as usize,
            len,
        )
    }
}

impl DescriptorBuilder<*const (), *mut ()> {
    pub fn try_build(self, len: u32) -> Result<Descriptor, InvalidDescriptor> {
        if len > Descriptor::MAX_LEN {
            return Err(InvalidDescriptor::ByteCounterTooLong(len as usize));
        }
        Ok(self.build_inner(self.source as usize, self.dest as usize, len))
    }
}

// Descriptor

impl Descriptor {
    const END_LINK: u32 = 0xFFFF_F800;

    /// Maximum length for arguments to [`DescriptorBuilder::source_slice`] and
    /// [`DescriptorBuilder::dest_slice`], and to the `len` argument to
    /// [`DescriptorBuilder::try_build`] --- byte counters must be 25 bits wide
    /// or less.
    pub const MAX_LEN: u32 = (1 << 25) - 1;

    /// Highest allowable address for arguments to
    /// [`DescriptorBuilder::source_slice`], [`DescriptorBuilder::dest_slice`],
    /// [`DescriptorBuilder::source_reg`], and [`DescriptorBuilder::dest_reg`]
    /// --- addresses must be 34 bits wide or less.
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

pub mod errors {
    use core::fmt;

    use super::*;

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
        /// A slice was longer than the maximum supported DMA transfer size of
        /// [`Descriptor::MAX_LEN`] (25 bits).
        TooLong(usize),
    }

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
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

    // === InvalidDescriptor ===

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

    // === InvalidLink ===

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

    // === InvalidOperand ===

    impl InvalidOperand {
        /// Returns whether this error describes the source or destination operand of
        /// a DMA transfer.
        #[must_use]
        pub fn operand(&self) -> Operand {
            self.kind
        }

        /// Returns an [`InvalidOperandReason`] describing why the operand was
        /// invalid.
        #[must_use]
        pub fn reason(&self) -> &InvalidOperandReason {
            &self.reason
        }

        #[must_use]
        pub(super) fn too_long(kind: Operand, len: usize) -> Self {
            Self {
                reason: InvalidOperandReason::TooLong(len),
                kind,
            }
        }

        #[must_use]
        pub(super) fn addr_too_high(kind: Operand, addr: usize) -> Self {
            Self {
                reason: InvalidOperandReason::AddrTooHigh(addr),
                kind,
            }
        }
    }

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
                Self::TooLong(len) => write!(
                    f,
                    "length {len} is greater than `Descriptor::MAX_LEN` ({})",
                    Descriptor::MAX_LEN
                ),
            }
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

        #[test]
        fn pack_param(src_high in 0b00u32..0b11u32, dest_high in 0b00u32..0b11u32, wait_clock_cycles: u8) {
            let mut manual = wait_clock_cycles as u32;

            // Set source
            //          332222222222 11 11 11111100 00000000
            //          109876543210 98 76 54321098 76543210
            manual &= 0b111111111111_11_00_11111111_11111111;
            manual |= src_high << 16;

            // Set dest
            //          332222222222 11 11 11111100 00000000
            //          109876543210 98 76 54321098 76543210
            manual &= 0b111111111111_00_11_11111111_11111111;
            manual |= dest_high << 18;

            let param = Param::new()
                .with(Param::WAIT_CLOCK_CYCLES, wait_clock_cycles)
                .with(Param::SRC_HIGH, src_high)
                .with(Param::DEST_HIGH, dest_high);
            prop_assert_eq!(
                manual,
                param.bits(),
                "\n{:032b} (expected), vs:\n{}",
                manual,
                param,
            )
        }
    }
}
