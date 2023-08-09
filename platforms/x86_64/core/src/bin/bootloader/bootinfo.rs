use crate::framebuf;
use bootloader_api::info;
use hal_core::{boot::BootInfo, mem, PAddr, VAddr};
use hal_x86_64::{mm, vga};

#[derive(Debug)]
pub struct BootloaderApiBootInfo {
    inner: &'static info::BootInfo,
    has_framebuffer: bool,
}

type MemRegionIter = core::slice::Iter<'static, info::MemoryRegion>;

impl BootInfo for BootloaderApiBootInfo {
    type MemoryMap = core::iter::Map<MemRegionIter, fn(&info::MemoryRegion) -> mem::Region>;

    type Writer = vga::Writer;

    type Framebuffer = framebuf::FramebufWriter;

    /// Returns the boot info's memory map.
    fn memory_map(&self) -> Self::MemoryMap {
        fn convert_region_kind(kind: info::MemoryRegionKind) -> mem::RegionKind {
            match kind {
                info::MemoryRegionKind::Usable => mem::RegionKind::FREE,
                // TODO(eliza): make known
                info::MemoryRegionKind::UnknownUefi(_) => mem::RegionKind::UNKNOWN,
                info::MemoryRegionKind::UnknownBios(_) => mem::RegionKind::UNKNOWN,
                info::MemoryRegionKind::Bootloader => mem::RegionKind::BOOT,
                _ => mem::RegionKind::UNKNOWN,
            }
        }

        fn convert_region(region: &info::MemoryRegion) -> mem::Region {
            let start = PAddr::from_u64(region.start);
            let size = {
                let end = PAddr::from_u64(region.end).offset(1);
                assert!(start < end, "bad memory range from boot_info!");
                let size = start.difference(end);
                assert!(size >= 0);
                size as usize + 1
            };
            let kind = convert_region_kind(region.kind);
            mem::Region::new(start, size, kind)
        }
        self.inner.memory_regions[..].iter().map(convert_region)
    }

    fn writer(&self) -> Self::Writer {
        unimplemented!()
    }

    fn framebuffer(&self) -> Option<Self::Framebuffer> {
        if !self.has_framebuffer {
            return None;
        }

        Some(unsafe { framebuf::mk_framebuf() })
    }

    fn bootloader_name(&self) -> &str {
        "rust-osdev/bootloader"
    }

    fn init_paging(&self) {
        mm::init_paging(self.vm_offset())
    }
}

impl BootloaderApiBootInfo {
    fn vm_offset(&self) -> VAddr {
        VAddr::from_u64(
            self.inner
                .physical_memory_offset
                .into_option()
                .expect("haha wtf"),
        )
    }

    pub(super) fn from_bootloader(inner: &'static mut info::BootInfo) -> Self {
        let has_framebuffer = framebuf::init(inner);
        Self {
            inner,
            has_framebuffer,
        }
    }
}
