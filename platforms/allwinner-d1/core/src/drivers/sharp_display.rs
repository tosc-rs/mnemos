use crate::drivers::spim::SpiSenderClient;
use core::time::Duration;
use kernel::{mnemos_alloc::containers::FixedVec, Kernel};

pub async fn sharp_memory_display(k: &'static Kernel) {
    k.sleep(Duration::from_millis(100)).await;
    let mut spim = SpiSenderClient::from_registry(k).await.unwrap();

    loop {
        k.sleep(Duration::from_millis(100)).await;
        let mut msg_1 = FixedVec::new(2).await;
        msg_1.try_extend_from_slice(&[0x04, 0x00]).unwrap();
        if spim.send_wait(msg_1).await.is_ok() {
            break;
        }
    }

    k.sleep(Duration::from_millis(100)).await;

    // Loop, toggling the VCOM
    let mut vcom = true;
    let mut ctr = 0u32;
    let mut linebuf = FixedVec::new((52 * 240) + 2).await;
    for _ in 0..(52 * 240) + 2 {
        let _ = linebuf.try_push(0);
    }

    loop {
        // It takes ~50ms to send a full frame, and 20fps is every
        // 66.6ms. TODO: once we have "intervals", use that here.
        k.sleep(Duration::from_millis(17)).await;

        // Send a pattern
        //
        // A note on this format:
        //
        // * Every FRAME gets a 1 byte command.
        //     * It is zero, unless we are toggling VCOM.
        //     * VCOM must be toggled once per second...ish.
        // * Foreach LINE (240x) - 52 bytes total:
        //     * 1 byte for line number
        //     * (400bits / 8 = 50bytes) of data (one bit per pixel)
        //     * 1 "dummy" byte
        // * At the END, we need a total of two dummy bytes, so one from the last line, + 1 more
        //
        // This is where the 52x240 + 1 + 1 buffer size comes from.
        //
        // Reference: https://www.sharpsde.com/fileadmin/products/Displays/2016_SDE_App_Note_for_Memory_LCD_programming_V1.3.pdf
        let vc = if vcom { 0x02 } else { 0x00 };
        linebuf.as_slice_mut()[0] = 0x01 | vc;

        for (line, chunk) in &mut linebuf.as_slice_mut()[1..].chunks_exact_mut(52).enumerate() {
            chunk[0] = (line as u8) + 1;

            for b in &mut chunk[1..] {
                if vcom {
                    *b = 0x00;
                } else {
                    *b = 0xFF;
                }
            }
        }

        // This awaits until the send is complete. At 2MHz and ((52x240 + 2) * 8) = 99856 bits
        // to send, that is 49.9ms until this will be complete.
        linebuf = spim.send_wait(linebuf).await.map_err(drop).unwrap();

        if (ctr % 16) == 0 {
            vcom = !vcom;
        }
        ctr = ctr.wrapping_add(1);
    }
}
