use core::mem::size_of;

#[repr(C, align(4))]
#[derive(Debug, defmt::Format)]
pub struct RawHeader {
    // Bridge
    syscall_in_ptr: u32,
    syscall_in_len: u32,
    syscall_out_ptr: u32,
    syscall_out_len: u32,

    // Header
    etext: u32,
    srodata: u32,
    sdata: u32,
    edata: u32,
    sbss: u32,
    ebss: u32,
    stack_start: u32,
    entry_point: u32,
}

pub struct PartingWords {
    pub stack_start: u32,
    pub entry_point: u32,
}

impl RawHeader {
    // TODO: Get these from linker script?
    const START_ADDR: u32 = 0x2000_0000;
    const END_ADDR: u32 = Self::START_ADDR + (128 * 1024);

    pub fn oc_flash_setup(&self, app: &[u8]) -> PartingWords {
        // Copy text - not inclusive of rodata
        let txt_ptr = Self::START_ADDR as usize as *const u8 as *mut u8;
        unsafe {
            txt_ptr.copy_from_nonoverlapping(app.as_ptr(), app.len());
        }

        // Copy .rodata from the image to the actual .data range (if any)
        let data_size = (self.edata - self.sdata) as usize;
        if data_size > 0 {
            let ro_offset = (self.srodata - Self::START_ADDR) as usize;
            let data_ptr = self.sdata as usize as *const u8 as *mut u8;
            unsafe {
                data_ptr.copy_from_nonoverlapping(app.as_ptr().add(ro_offset), data_size);
            }
        }

        let bss_size = (self.ebss - self.sbss) as usize;
        if bss_size > 0 {
            let bss_ptr = self.sbss as usize as *const u8 as *mut u8;
            unsafe {
                bss_ptr.write_bytes(0, bss_size);
            }
        }

        PartingWords { stack_start: self.stack_start, entry_point: self.entry_point }
    }
}

#[repr(align(4))]
struct AlignHdrBuf {
    data: [u8; Self::SIZE],
}

impl AlignHdrBuf {
    const SIZE: usize = size_of::<RawHeader>();
}

fn addr_in_range(addr: u32) -> Result<(), ()> {
    let good = (addr >= RawHeader::START_ADDR) && (addr < RawHeader::END_ADDR);
    let good = good && ((addr % 4) == 0);

    if good {
        Ok(())
    } else {
        defmt::println!("Not in range: 0x{=u32:08X}", addr);
        Err(())
    }
}

impl From<AlignHdrBuf> for RawHeader {
    fn from(ahb: AlignHdrBuf) -> Self {
        unsafe {
            core::mem::transmute(ahb)
        }
    }
}

pub fn validate_header(bytes: &[u8]) -> Result<RawHeader, ()> {
    if bytes.len() < AlignHdrBuf::SIZE {
        defmt::println!("Too short!");
        return Err(());
    }

    let mut ahb = AlignHdrBuf {
        data: [0u8; AlignHdrBuf::SIZE],
    };
    ahb.data.copy_from_slice(&bytes[..AlignHdrBuf::SIZE]);
    let hdr: RawHeader = ahb.into();

    defmt::println!("{:08X}", hdr);

    // Make sure all of the bridge values are zero. If they are not,
    // it's a hint the data may be malformed.
    let bridge = &[
        hdr.syscall_in_ptr,
        hdr.syscall_in_len,
        hdr.syscall_out_ptr,
        hdr.syscall_out_len,
    ];

    let all_zero = bridge.iter().all(|w| *w == 0);
    if !all_zero {
        defmt::println!("Not all zero?");
        return Err(());
    }

    addr_in_range(hdr.etext)?;
    addr_in_range(hdr.srodata)?;
    addr_in_range(hdr.sdata)?;
    addr_in_range(hdr.edata)?;
    addr_in_range(hdr.sbss)?;
    addr_in_range(hdr.ebss)?;
    addr_in_range(hdr.stack_start)?;

    let good_entry = (hdr.entry_point >= RawHeader::START_ADDR) && (hdr.entry_point < RawHeader::END_ADDR);
    let good_entry = good_entry && ((hdr.entry_point % 2) == 1);
    if !good_entry {
        defmt::println!("Bad entry!");
        return Err(());
    }

    defmt::println!("Passed range check!");

    if hdr.edata < hdr.sdata {
        defmt::println!("Data check fail!");
        return Err(());
    }

    if hdr.ebss < hdr.sbss {
        defmt::println!("BSS check fail!");
        return Err(());
    }

    Ok(hdr)
}
