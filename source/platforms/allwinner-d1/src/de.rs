#![allow(dead_code)]

const DE_BASE: u32 = 0x0500_0000;
const DE_SCLK_GATE: u32 = DE_BASE + 0x000;
const DE_HCLK_GATE: u32 = DE_BASE + 0x004;
const DE_AHB_RESET: u32 = DE_BASE + 0x008;
const DE_SCLK_DIV: u32 = DE_BASE + 0x00C;

const DE_MIXER0: u32 = DE_BASE + 0x0010_0000;
const DE_M0_GLB: u32 = DE_MIXER0 + 0x0_0000;
const DE_M0_BLD: u32 = DE_MIXER0 + 0x0_1000;
const DE_M0_OVL_V: u32 = DE_MIXER0 + 0x0_2000;
const DE_M0_OVL_UI1: u32 = DE_MIXER0 + 0x0_3000;
const DE_M0_VIDEO_SCALAR: u32 = DE_MIXER0 + 0x2_0000;
const DE_M0_UI_SCALAR1: u32 = DE_MIXER0 + 0x4_0000;
const DE_M0_POST_PROC1: u32 = DE_MIXER0 + 0xA_0000;
const DE_M0_POST_PROC2: u32 = DE_MIXER0 + 0xB_0000;
const DE_M0_DMA: u32 = DE_MIXER0 + 0xC_0000;

const DE_M0_GLB_CTL: u32 = DE_M0_GLB + 0x000;
const DE_M0_GLB_STS: u32 = DE_M0_GLB + 0x004;
const DE_M0_GLB_DBUFFER: u32 = DE_M0_GLB + 0x008;
const DE_M0_GLB_SIZE: u32 = DE_M0_GLB + 0x00C;
const DE_M0_OVL_V_ATTCTL: u32 = DE_M0_OVL_V + 0x000;
const DE_M0_OVL_V_MBSIZE: u32 = DE_M0_OVL_V + 0x004;
const DE_M0_OVL_V_COOR: u32 = DE_M0_OVL_V + 0x008;
const DE_M0_OVL_V_PITCH0: u32 = DE_M0_OVL_V + 0x00C;
const DE_M0_OVL_V_PITCH1: u32 = DE_M0_OVL_V + 0x010;
const DE_M0_OVL_V_PITCH2: u32 = DE_M0_OVL_V + 0x014;
const DE_M0_OVL_V_TOP_LADD0: u32 = DE_M0_OVL_V + 0x018;
const DE_M0_OVL_V_TOP_LADD1: u32 = DE_M0_OVL_V + 0x01C;
const DE_M0_OVL_V_TOP_LADD2: u32 = DE_M0_OVL_V + 0x020;
const DE_M0_OVL_V_BOT_LADD0: u32 = DE_M0_OVL_V + 0x024;
const DE_M0_OVL_V_BOT_LADD1: u32 = DE_M0_OVL_V + 0x028;
const DE_M0_OVL_V_BOT_LADD2: u32 = DE_M0_OVL_V + 0x02C;
const DE_M0_OVL_V_FILL_COLOR: u32 = DE_M0_OVL_V + 0x0C0;
const DE_M0_OVL_V_TOP_HADD0: u32 = DE_M0_OVL_V + 0x0D0;
const DE_M0_OVL_V_TOP_HADD1: u32 = DE_M0_OVL_V + 0x0D4;
const DE_M0_OVL_V_TOP_HADD2: u32 = DE_M0_OVL_V + 0x0D8;
const DE_M0_OVL_V_BOT_HADD0: u32 = DE_M0_OVL_V + 0x0DC;
const DE_M0_OVL_V_BOT_HADD1: u32 = DE_M0_OVL_V + 0x0E0;
const DE_M0_OVL_V_BOT_HADD2: u32 = DE_M0_OVL_V + 0x0E4;
const DE_M0_OVL_V_SIZE: u32 = DE_M0_OVL_V + 0x0E8;
const DE_M0_OVL_V_HDS_CTL0: u32 = DE_M0_OVL_V + 0x0F0;
const DE_M0_OVL_V_HDS_CTL1: u32 = DE_M0_OVL_V + 0x0F4;
const DE_M0_OVL_V_VDS_CTL0: u32 = DE_M0_OVL_V + 0x0F8;
const DE_M0_OVL_V_VDS_CTL1: u32 = DE_M0_OVL_V + 0x0FC;
const DE_M0_UI1_ATTCTL_L0: u32 = DE_M0_OVL_UI1 + 0x000;
const DE_M0_UI1_MBSIZE_L0: u32 = DE_M0_OVL_UI1 + 0x004;
const DE_M0_UI1_COOR_L0: u32 = DE_M0_OVL_UI1 + 0x008;
const DE_M0_UI1_PITCH_L0: u32 = DE_M0_OVL_UI1 + 0x00C;
const DE_M0_UI1_TOP_LADD_L0: u32 = DE_M0_OVL_UI1 + 0x010;
const DE_M0_UI1_BOT_LADD_L0: u32 = DE_M0_OVL_UI1 + 0x014;
const DE_M0_UI1_FILL_COLOR_L0: u32 = DE_M0_OVL_UI1 + 0x018;
const DE_M0_UI1_TOP_HADD: u32 = DE_M0_OVL_UI1 + 0x080;
const DE_M0_UI1_BOT_HADD: u32 = DE_M0_OVL_UI1 + 0x084;
const DE_M0_UI1_SIZE: u32 = DE_M0_OVL_UI1 + 0x088;
const DE_M0_BLD_FILL_COLOR_CTL: u32 = DE_M0_BLD + 0x000;
const DE_M0_BLD_FILL_COLOR_P0: u32 = DE_M0_BLD + 0x004 + 0 * 0x14;
const DE_M0_BLD_CH_ISIZE_P0: u32 = DE_M0_BLD + 0x008 + 0 * 0x14;
const DE_M0_BLD_CH_OFFSET_P0: u32 = DE_M0_BLD + 0x008 + 0 * 0x14;
const DE_M0_BLD_CH_RTCTL: u32 = DE_M0_BLD + 0x080;
const DE_M0_BLD_PREMUL_CTL: u32 = DE_M0_BLD + 0x084;
const DE_M0_BLD_BK_COLOR: u32 = DE_M0_BLD + 0x088;
const DE_M0_BLD_SIZE: u32 = DE_M0_BLD + 0x08C;
const DE_M0_BLD_CTL: u32 = DE_M0_BLD + 0x090;
const DE_M0_BLD_KEY_CTL: u32 = DE_M0_BLD + 0x0B0;
const DE_M0_BLD_KEY_CON: u32 = DE_M0_BLD + 0x0B4;
const DE_M0_BLD_KEY_MAX: u32 = DE_M0_BLD + 0x0C0;
const DE_M0_BLD_KEY_MIN: u32 = DE_M0_BLD + 0x0E0;
const DE_M0_BLD_OUT_COLOR: u32 = DE_M0_BLD + 0x0FC;

pub unsafe fn init(fb: &[u8]) {
    use core::ptr::write_volatile;
    // Enable ROT_SCLK_GATE, RT_WB_SCLK_GATE, CORE1_SCLK_GATE, CORE0_SCLK_GATE
    // (at least, as they exist in the H8).
    write_volatile(DE_SCLK_GATE as *mut u32, 0xF);

    // Enable ROT_HCLK_GATE, RT_WB_HCLK_GATE, CORE1_HCLK_GATE, CORE_HCLK_GATE0
    // (at least, as they exist in the H8).
    write_volatile(DE_HCLK_GATE as *mut u32, 0xF);

    // Bring ROT, RT_WB, CORE1, CORE0 out of reset.
    write_volatile(DE_AHB_RESET as *mut u32, 0xF);

    // Hopefully default div is OK.
    write_volatile(
        DE_SCLK_DIV as *mut u32,
        (0 << 12)     // ROT_SCLK_DIV
        | (0 <<  8)     // RT_WB_SCLK_DIV
        | (0 <<  4)     // CORE1_SCLK_DIV
        | (0 <<  0), // CORE0_SCLK_DIV
    );

    // Hopefully DE2TCON_MUX either doesn't exist or default is fine.
    // Hopefully CMD_CTL either doesn't exist or default is fine.
    // Hopefully DI_CTL either doesn't exist or default is fine.

    // Enable RT
    write_volatile(DE_M0_GLB_CTL as *mut u32, 1);
    // Set height=272 and width=480
    write_volatile(DE_M0_GLB_SIZE as *mut u32, (271 << 16) | (479 << 0));

    // Set OVL_UI1_L0 to alpha=FF, top-addr-only, no-premult, BGR888, no fill, global alpha, enable
    // NB not sure why BGR is required, since data is in RGB...??
    write_volatile(
        DE_M0_UI1_ATTCTL_L0 as *mut u32,
        (0xFF << 24) | (0 << 23) | (0 << 16) | (0x09 << 8) | (0 << 4) | (1 << 1) | (1 << 0),
    );
    // Set OVL_UI1_L0 to height=272 width=480
    write_volatile(DE_M0_UI1_MBSIZE_L0 as *mut u32, (271 << 16) | (479 << 0));
    // Set OVL_UI1_L0 coordinate to 0, 0
    write_volatile(DE_M0_UI1_COOR_L0 as *mut u32, (0 << 16) | (0 << 0));
    // Set OVL_UI1_L0 pitch to 480*3 bytes/line.
    write_volatile(DE_M0_UI1_PITCH_L0 as *mut u32, 480 * 3);
    // Set memory start address
    write_volatile(DE_M0_UI1_TOP_LADD_L0 as *mut u32, fb.as_ptr() as u32);
    write_volatile(DE_M0_UI1_TOP_HADD as *mut u32, 0);
    // Set overlay to 272x480
    write_volatile(DE_M0_UI1_SIZE as *mut u32, (271 << 16) | (479 << 0));

    // Enable Pipe0, no fill
    write_volatile(DE_M0_BLD_FILL_COLOR_CTL as *mut u32, 1 << 8);
    // Pipe0 Input size 272x480
    write_volatile(DE_M0_BLD_CH_ISIZE_P0 as *mut u32, (271 << 16) | (479 << 0));
    // Pipe 0 offset apparently needs to be 271,479? Not sure why.
    write_volatile(DE_M0_BLD_CH_OFFSET_P0 as *mut u32, (271 << 16) | (479 << 0));
    // Pipe 0 select from channel 1, pipe 1 from 0, pipe 2 from 2, pipe 3 from 3
    write_volatile(
        DE_M0_BLD_CH_RTCTL as *mut u32,
        (3 << 12) | (2 << 8) | (0 << 4) | (1 << 0),
    );
    // Output size 272x480
    write_volatile(DE_M0_BLD_SIZE as *mut u32, (271 << 16) | (479 << 0));
}
