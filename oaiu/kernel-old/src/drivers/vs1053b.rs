use crate::{
    alloc::HeapGuard,
    traits::{PcmSink, Spi, SpiHandle, SpiTransactionKind, SpiTransaction}, future_box::FutureBoxExHdl,
};

const CMD_SPEED_KHZ: u32 = 1_000;
const DATA_SPEED_KHZ: u32 = 8_000;

pub struct Vs1053b {
    xcs: SpiHandle,
    xdcs: SpiHandle,
    enabled: bool,
}

impl Vs1053b {
    pub fn from_handles(xcs: SpiHandle, xdcs: SpiHandle) -> Self {
        Self { xcs, xdcs, enabled: false }
    }

    fn send_cmd(
        &mut self,
        heap: &mut HeapGuard,
        spi: &mut dyn Spi,
        msg: &[u8],
    ) -> Result<(), ()> {
        let mut tx = spi
            .alloc_transaction(
                heap,
                SpiTransactionKind::Send,
                self.xcs,
                CMD_SPEED_KHZ,
                msg.len(),
            )
            .ok_or(())?;

        tx.data.copy_from_slice(msg);
        tx.release_to_kernel();

        // TODO! Add a "NOP" SPI command for delays...

        Ok(())
    }

    fn send_data(
        &mut self,
        heap: &mut HeapGuard,
        spi: &mut dyn Spi,
        msg: &[u8],
    ) -> Result<(), ()> {
        let mut tx = spi
            .alloc_transaction(
                heap,
                SpiTransactionKind::Send,
                self.xdcs,
                DATA_SPEED_KHZ,
                msg.len(),
            )
            .ok_or(())?;

        tx.data.copy_from_slice(msg);

        tx.release_to_kernel();
        Ok(())
    }
}

impl PcmSink for Vs1053b {
    fn enable(&mut self, heap: &mut HeapGuard, spi: &mut dyn Spi) -> Result<(), ()> {
        // SCI command goes:
        // Operation: 1 byte
        //     * Read:  0x03
        //     * Write: 0x02
        // Address: 1 byte
        // Data: 2 bytes

        // SOFT RESET
        self.send_cmd(
            heap,
            spi,
            &[
                0x02, // Write
                0x00, // MODE
                0x48, // ?
                0x04, // ?
            ],
        )?;

        // Set CLOCKF register (0x03)
        // 10.2 recommend a value of 9800, meaning
        // 100 - 11 - 00000000000
        //   XTALIx3.5 (Mult)
        //   XTALIx1.5 (Max boost)
        //   Freq = 0 (12.288MHz)
        self.send_cmd(
            heap,
            spi,
            &[
                0x02, // Write
                0x03, // CLOCKF
                0x98, // ??
                0x00, // ??
            ],
        )?;

        // One bit every 4 CLKI pulses.
        // Since we've increased the clock rate to
        // 3.5xXTALI (~43MHz), that gives us a max SPI
        // clock rate of ~10.75MHz. Use 8MHz.

        // Before decoding, set
        // * SCI_MODE
        // * SCI_BASS
        // * SCI_CLOCKF (done)
        // * SCI_VOL

        // Probably skip the others, but probably set volume to like 0x2424,
        // which means -18.0dB in each ear.
        self.send_cmd(
            heap,
            spi,
            &[
                0x02, // Write
                0x0B, // VOLUME
                0x24, // -18.0dB
                0x24, // -18.0dB
            ],
        )?;

        // Example: A 44100 Hz 16-bit stereo PCM header would read as follows:
        // 0000 52 49 46 46 ff ff ff ff 57 41 56 45 66 6d 74 20 |RIFF....WAVEfmt |
        // 0100 10 00 00 00 01 00 02 00 44 ac 00 00 10 b1 02 00 |........D.......|
        // 0200 04 00 10 00 64 61 74 61 ff ff ff ff             |....data....|
        self.send_data(
            heap,
            spi,
            &[
                0x52, 0x49, 0x46, 0x46, 0xff, 0xff, 0xff, 0xff, 0x57, 0x41, 0x56, 0x45, 0x66, 0x6d,
                0x74, 0x20, 0x10, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x44, 0xac, 0x00, 0x00,
                0x10, 0xb1, 0x02, 0x00, 0x04, 0x00, 0x10, 0x00, 0x64, 0x61, 0x74, 0x61, 0xff, 0xff,
                0xff, 0xff,
            ],
        )?;

        self.enabled = true;
        spi.start_send();

        Ok(())
    }

    fn disable(&mut self, _heap: &mut HeapGuard, _spi: &mut dyn Spi) -> Result<(), ()> {
        self.enabled = false;
        defmt::panic!("lol")
    }

    fn allocate_stereo_samples(
        &mut self,
        heap: &mut HeapGuard,
        spi: &mut dyn Spi,
        count: usize,
    ) -> Option<FutureBoxExHdl<SpiTransaction>> {
        if !self.enabled {
            return None;
        }
        // Kick the spi, just in case we paused for some reason. This means that if samples
        // N were sent (but are waiting), at least they go out at sample N + 1.
        // TODO: this should probably be done with some kind of "waker" instead...
        spi.start_send();

        spi.alloc_transaction(
            heap,
            SpiTransactionKind::Send,
            self.xdcs,
            DATA_SPEED_KHZ,
            count * 4,
        )
    }
}
