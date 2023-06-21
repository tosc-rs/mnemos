use mycelium_bitfield::bitfield;

const TWI0_BASE: u32 = 0x02502000;
const TWI1_BASE: u32 = 0x02502400;
const TWI2_BASE: u32 = 0x02502800;
const TWI3_BASE: u32 = 0x02502C00;

// bitfield! {
//     /// Register `TWI_ADDR`
//     struct AddrReg<u32> {
//         /// `GCE`: General Call Address Enable
//         const GENERAL_CALL_ENABLE: bool;
//         /// `SLA`: Device Address
//         ///
//         /// The D1 supports both the 7-bit and 10-bit I<sup>2</sup>C addressing
//         /// modes.
//         /// - When using the 7-bit addressing mode, this is the 7-bit
//         ///   address of the I2C device to communicate with.
//         /// - When using the 10-bit addressing mode, this field must be
//         ///   `0b11110, 
//         ///
//         /// > For 7-bit addressing, the bit[7:1] indicates:
//         /// > SLA6, SLA5, SLA4, SLA3, SLA2, SLA1, SLA0
//         /// > For 10-bit addressing, the bit[7:1] indicates:
//         /// > 1, 1, 1, 1, 0, SLAX[9:8]
//         const ADDR = 7;
//     }
// }

bitfield! {
    /// Register `TWI_CNTR` (offset `0x000c`)
    struct ControlReg<u32> {
        ///
        const CLOCK_COUNT_MODE: bool;
        const _UNUSED_1 = 1;
        const ACK: bool;
        /// `INT_FLAG`: Interrupt Flag
        ///
        /// > The INT_FLAG is automatically set to ‘1’ when any of the 28 (out
        /// > of the possible 29) states is entered (see ‘STAT Register’ below).
        /// > The state that does not set INT_FLAG is state F8h. If the INT_EN bit
        /// > is set, the interrupt line goes high when INT_FLAG is set to ‘1’. If
        /// > the TWI is operating in slave mode, the data transfer is suspended
        /// > when INT_FLAG is set and the low period of the TWI bus clock line
        /// > (SCL) is stretched until ‘1’ is written to INT_FLAG. The TWI clock
        /// > line is then released and the interrupt line goes low.
        const INTERRUPT_FLAG: bool;
        /// `M_STP`: Controller Mode Stop
        ///
        /// If this bit is set, the TWI will transmit a STOP condition to
        /// indicate that it is no longer the bus controller, and then clear
        /// this bit.
        ///
        /// From the datasheet:
        ///
        /// > If the M_STP is set to ‘1’ in master mode, a STOP condition is
        /// > transmitted on the TWI bus. If the M_STP bit is set to ‘1’ in slave
        /// > mode, the TWI will indicate if a STOP condition has been received,
        /// > but no STOP condition will be transmitted on the TWI bus. If both
        /// > M_STA and M_STP bits are set, the TWI will first transmit the STOP
        /// > condition (if in master mode), then transmit the START condition.
        /// >
        /// > The M_STP bit is cleared automatically. Writing a ‘0’ to this bit has
        /// > no effect.
        const CONTROLLER_STOP: bool;
        /// `M_STA`: Controller Mode Start
        ///
        /// If this is set, the TWI will enter controller mode and clear this
        /// bit.
        ///
        /// From the data sheet:
        ///
        /// > When the M_STA is set to ‘1’, the TWI controller enters master
        /// > mode and will transmit a START condition on the bus when the bus
        /// > is free. If the M_STA bit is set to ‘1’ when the TWI controller is
        /// > already in master mode and one or more bytes have been
        /// > transmitted, then a repeated START condition will be sent. If the
        /// > M_STA bit is set to ‘1’ when the TWI is accessed in slave mode, the
        /// > TWI will complete the data transfer in slave mode then enter
        /// > master mode when the bus has been released
        const CONTROLLER_START: bool;
        /// `BUS_EN`: TWI Bus Enable
        ///
        /// This must be `true` in order to use the I2C bus on this TWI.
        const BUS_ENABLE: bool;
                /// `INT_EN`: Interrupt Enable
        /// - `false`: the interrupt line is always held low
        /// - `true`: the interrupt line will be asserted high when `INT_FLAG` is set.
        const INTERRUPT_ENABLE: bool;



    }
}