// Unusual groupings are used in binary literals in this file in order to
// separate the bits by which field they represent, rather than by their byte.
#![allow(clippy::unusual_byte_groupings)]

#[derive(Clone, Debug)]
#[repr(C, align(4))]
pub struct Descriptor {
    configuration: u32,
    source_address: u32,
    destination_address: u32,
    byte_counter: u32,
    parameter: u32,
    link: u32,
}

// TODO: THIS COULD PROBABLY BE A BITFIELD LIBRARY
pub struct DescriptorConfig {
    pub source: *const (),
    pub destination: *mut (),

    // NOTE: Max is < 2^25, or < 32MiB
    pub byte_counter: usize,
    pub link: Option<*const ()>,
    pub wait_clock_cycles: u8,

    pub bmode: BModeSel,

    pub dest_width: DataWidth,
    pub dest_addr_mode: AddressMode,
    pub dest_block_size: BlockSize,
    pub dest_drq_type: DestDrqType,

    pub src_data_width: DataWidth,
    pub src_addr_mode: AddressMode,
    pub src_block_size: BlockSize,
    pub src_drq_type: SrcDrqType,
}

#[derive(Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum SrcDrqType {
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

#[derive(Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum DestDrqType {
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

// TODO: Verify bits or bytes?
#[derive(Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum BlockSize {
    Byte1 = 0b00,
    Byte4 = 0b01,
    Byte8 = 0b10,
    Byte16 = 0b11,
}

#[derive(Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum AddressMode {
    LinearMode = 0,
    IoMode = 1,
}

#[derive(Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum DataWidth {
    Bit8 = 0b00,
    Bit16 = 0b01,
    Bit32 = 0b10,
    Bit64 = 0b11,
}

#[derive(Eq, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum BModeSel {
    Normal,
    BMode,
}

// Descriptor

impl Descriptor {
    pub fn set_source(&mut self, source: u64) {
        assert!(source < (1 << 34));
        self.source_address = source as u32;
        //                  332222222222 11 11 11111100 00000000
        //                  109876543210 98 76 54321098 76543210
        self.parameter &= 0b111111111111_11_00_11111111_11111111;
        self.parameter |= (((source >> 32) & 0b11) << 16) as u32;
    }

    pub fn set_dest(&mut self, dest: u64) {
        assert!(dest < (1 << 34));
        self.destination_address = dest as u32;
        //                  332222222222 11 11 11111100 00000000
        //                  109876543210 98 76 54321098 76543210
        self.parameter &= 0b111111111111_00_11_11111111_11111111;
        self.parameter |= (((dest >> 32) & 0b11) << 18) as u32;
    }

    pub fn end_link(&mut self) {
        self.link = 0xFFFF_F800;
    }
}

impl TryFrom<DescriptorConfig> for Descriptor {
    type Error = ();

    fn try_from(value: DescriptorConfig) -> Result<Self, Self::Error> {
        let source = value.source as usize;
        let destination = value.destination as usize;

        if source >= (1 << 34) {
            return Err(());
        }
        if destination >= (1 << 34) {
            return Err(());
        }
        if value.byte_counter >= (1 << 25) {
            return Err(());
        }
        if let Some(link) = value.link {
            let link = link as usize;
            if (link & 0b11) != 0 {
                return Err(());
            }
        }

        let mut descriptor = Descriptor {
            configuration: 0,
            source_address: 0,
            destination_address: 0,
            byte_counter: 0,
            parameter: 0,
            link: 0,
        };

        // Set source
        descriptor.source_address = source as u32;
        //                        332222222222 11 11 11111100 00000000
        //                        109876543210 98 76 54321098 76543210
        descriptor.parameter &= 0b111111111111_11_00_11111111_11111111;
        descriptor.parameter |= (((source >> 32) & 0b11) << 16) as u32;

        // Set dest
        descriptor.destination_address = destination as u32;
        //                        332222222222 11 11 11111100 00000000
        //                        109876543210 98 76 54321098 76543210
        descriptor.parameter &= 0b111111111111_00_11_11111111_11111111;
        descriptor.parameter |= (((destination >> 32) & 0b11) << 18) as u32;

        descriptor.byte_counter = value.byte_counter as u32;

        // Set configuration
        descriptor.configuration |= value.bmode.to_desc_bits();
        descriptor.configuration |= value.dest_width.to_desc_bits_dest();
        descriptor.configuration |= value.dest_addr_mode.to_desc_bits_dest();
        descriptor.configuration |= value.dest_block_size.to_desc_bits_dest();
        descriptor.configuration |= value.dest_drq_type.to_desc_bits();
        descriptor.configuration |= value.src_data_width.to_desc_bits_src();
        descriptor.configuration |= value.src_addr_mode.to_desc_bits_src();
        descriptor.configuration |= value.src_block_size.to_desc_bits_src();
        descriptor.configuration |= value.src_drq_type.to_desc_bits();

        if let Some(link) = value.link {
            descriptor.link = link as u32;
            // We already verified above the low bits of `value.link` are clear,
            // no need to re-mask the current state of `descriptor.link`.
            descriptor.link |= ((link as usize >> 32) as u32) & 0b11
        } else {
            descriptor.end_link();
        }

        Ok(descriptor)
    }
}

// DescriptorConfig

// SrcDrqType

impl SrcDrqType {
    #[inline(always)]
    fn to_desc_bits(self) -> u32 {
        // 6 bits, no shift
        ((self as u8) & 0b11_1111) as u32
    }
}

// DestDrqType

impl DestDrqType {
    #[inline(always)]
    fn to_desc_bits(self) -> u32 {
        (((self as u8) & 0b11_1111) as u32) << 16
    }
}

// BlockSize

impl BlockSize {
    #[inline(always)]
    fn to_desc_bits_dest(self) -> u32 {
        (((self as u8) & 0b11) as u32) << 22
    }

    #[inline(always)]
    fn to_desc_bits_src(self) -> u32 {
        (((self as u8) & 0b11) as u32) << 6
    }
}

// AddressMode

impl AddressMode {
    #[inline(always)]
    fn to_desc_bits_src(self) -> u32 {
        (((self as u8) & 0b1) as u32) << 8
    }

    #[inline(always)]
    fn to_desc_bits_dest(self) -> u32 {
        (((self as u8) & 0b1) as u32) << 24
    }
}

// DataWidth

impl DataWidth {
    #[inline(always)]
    fn to_desc_bits_dest(self) -> u32 {
        (((self as u8) & 0b11) as u32) << 25
    }

    #[inline(always)]
    fn to_desc_bits_src(self) -> u32 {
        (((self as u8) & 0b11) as u32) << 9
    }
}

// BModeSel

impl BModeSel {
    #[inline(always)]
    fn to_desc_bits(self) -> u32 {
        (((self as u8) & 0b1) as u32) << 30
    }
}
