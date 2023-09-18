//! DMAC [`Descriptor`]s configure DMA transfers initiated using the DMAC.
// Most of the code in this module still needs documentation...
#![allow(missing_docs)]
// Unusual groupings are used in binary literals in this file in order to
// separate the bits by which field they represent, rather than by their byte.
#![allow(clippy::unusual_byte_groupings)]

use core::fmt;

#[derive(Clone, Debug)]
#[repr(C, align(4))]
pub struct Descriptor {
    configuration: Configuration,
    source_address: u32,
    destination_address: u32,
    byte_counter: u32,
    parameter: u32,
    link: u32,
}

#[derive(Copy, Clone, Debug)]
pub struct DescriptorBuilder {
    cfg: Configuration,
    wait_clock_cycles: u8,
    link: Option<*const ()>,
}

/// Errors returned by [`DescriptorBuilder::build`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidDescriptor {
    SrcAddrTooLong(usize),
    DestAddrTooLong(usize),
    ByteCounterTooLong(u32),
    LinkAddrMisaligned(usize),
}

mycelium_bitfield::bitfield! {
    struct Configuration<u32> {
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
            cfg: Configuration::new(),
            wait_clock_cycles: 0,
            link: None,
        }
    }

    pub fn src_drq_type(self, val: SrcDrqType) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::SRC_DRQ_TYPE, val),
            ..self
        }
    }

    pub fn src_block_size(self, val: BlockSize) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::SRC_BLOCK_SIZE, val),
            ..self
        }
    }

    pub fn src_addr_mode(self, val: AddressMode) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::SRC_ADDR_MODE, val),
            ..self
        }
    }

    pub fn src_data_width(self, val: DataWidth) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::SRC_DATA_WIDTH, val),
            ..self
        }
    }

    pub fn dest_drq_type(self, val: DestDrqType) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::DEST_DRQ_TYPE, val),
            ..self
        }
    }

    pub fn dest_block_size(self, val: BlockSize) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::DEST_BLOCK_SIZE, val),
            ..self
        }
    }

    pub fn dest_addr_mode(self, val: AddressMode) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::DEST_ADDR_MODE, val),
            ..self
        }
    }

    pub fn dest_data_width(self, val: DataWidth) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::DEST_DATA_WIDTH, val),
            ..self
        }
    }

    pub fn bmode_sel(self, val: BModeSel) -> Self {
        Self {
            cfg: self.cfg.with(Configuration::BMODE_SEL, val),
            ..self
        }
    }

    pub const fn link(self, link: Option<*const ()>) -> Self {
        Self { link, ..self }
    }

    pub const fn wait_clock_cycles(self, wait_clock_cycles: u8) -> Self {
        Self {
            wait_clock_cycles,
            ..self
        }
    }

    pub fn build(
        self,
        source: *const (),
        destination: *mut (),
        byte_counter: u32,
    ) -> Result<Descriptor, InvalidDescriptor> {
        let source = source as usize;
        let destination = destination as usize;

        if source > Descriptor::ADDR_MAX as usize {
            return Err(InvalidDescriptor::SrcAddrTooLong(source));
        }
        if destination > Descriptor::ADDR_MAX as usize {
            return Err(InvalidDescriptor::DestAddrTooLong(destination));
        }
        if byte_counter > Descriptor::BYTE_COUNTER_MAX {
            return Err(InvalidDescriptor::ByteCounterTooLong(byte_counter));
        }
        if let Some(link) = self.link {
            if (link as usize & 0b11) != 0 {
                return Err(InvalidDescriptor::LinkAddrMisaligned(link as usize));
            }
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

        let link = self
            .link
            .map(|link| {
                // We already verified above the low bits of `value.link` are clear,
                // no need to re-mask the current state of `descriptor.link`.
                link as u32 | ((link as usize >> 32) as u32) & 0b11
            })
            .unwrap_or(Descriptor::END_LINK);

        Ok(Descriptor {
            configuration: self.cfg,
            source_address,
            destination_address,
            byte_counter,
            parameter,
            link,
        })
    }
}

// Descriptor

impl Descriptor {
    const END_LINK: u32 = 0xFFFF_F800;

    /// Maximum value for the `byte_counter` argument to
    /// [`DescriptorBuilder::build`] --- byte counters must be 25 bits wide or
    /// less.
    pub const BYTE_COUNTER_MAX: u32 = (1 << 25) - 1;

    /// Maximum value for the `source` and `destination` arguments to
    /// [`DescriptorBuilder::build`] --- addresses must be 34 bits wide or less.
    pub const ADDR_MAX: u64 = (1 << 34) - 1;

    pub const fn builder() -> DescriptorBuilder {
        DescriptorBuilder::new()
    }
}

// InvalidDescriptor

impl fmt::Display for InvalidDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidDescriptor::SrcAddrTooLong(addr) => write!(
                f,
                "source address {addr:#x} is greater than `Descriptor::ADDR_MAX` ({:#x})",
                Descriptor::ADDR_MAX
            ),
            InvalidDescriptor::DestAddrTooLong(addr) => write!(
                f,
                "destination address {addr:#x} is greater than `Descriptor::ADDR_MAX` ({:#x})",
                Descriptor::ADDR_MAX
            ),
            InvalidDescriptor::ByteCounterTooLong(counter) => write!(
                f,
                "byte counter {counter} is greater than `Descriptor::BYTE_COUNTER_MAX` ({})",
                Descriptor::BYTE_COUNTER_MAX
            ),
            InvalidDescriptor::LinkAddrMisaligned(addr) => write!(
                f,
                "linked descriptor address {addr:#x} was not at least 4-byte aligned"
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
        Configuration::assert_valid();
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
            let config = DescriptorBuilder::new()
                .src_drq_type(cfg.src_drq_type)
                .src_block_size(cfg.src_block_size)
                .src_addr_mode(cfg.src_addr_mode)
                .src_data_width(cfg.src_data_width)
                .dest_drq_type(cfg.dest_drq_type)
                .dest_block_size(cfg.dest_block_size)
                .dest_addr_mode(cfg.dest_addr_mode)
                .dest_data_width(cfg.dest_data_width)
                .bmode_sel(cfg.bmode_sel).cfg;

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
