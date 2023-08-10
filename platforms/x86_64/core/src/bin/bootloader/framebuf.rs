// TODO(eliza): eventually, turn this into a nice mnemOS-style driver task...
use bootloader_api::{info, BootInfo};
use core::{
    mem,
    ops::{Deref, DerefMut},
};
use hal_x86_64::framebuffer::{self, Framebuffer};
use mycelium_util::sync::{spin, InitOnce};

#[derive(Debug)]
pub struct FramebufGuard(spin::MutexGuard<'static, info::FrameBuffer>);
pub type FramebufWriter = Framebuffer<'static, FramebufGuard>;

/// Locks the framebuffer and returns a [`FramebufWriter`].
///
/// # Safety
///
/// In release mode, this function *assumes* the frame buffer has been
/// initialized by [`init`]. If this is ever called before [`init`] has been
/// called and returned `true`, this may read uninitialized memory!
pub(super) unsafe fn mk_framebuf() -> FramebufWriter {
    let (cfg, buf) = unsafe {
        // Safety: we can reasonably assume this will only be called
        // after `arch_entry`, so if we've failed to initialize the
        // framebuffer...things have gone horribly wrong...
        FRAMEBUFFER.get_unchecked()
    };
    Framebuffer::new(cfg, FramebufGuard(buf.lock()))
}

/// Forcibly unlock the framebuffer mutex.
///
/// # Safety
///
/// This forcibly unlocks a potentially-locked mutex, violating mutual
/// exclusion! This should only be called in conditions where no other CPU core
/// will *ever* attempt to access the framebuffer again (such as while oopsing).
#[allow(dead_code)]
pub(super) unsafe fn force_unlock() {
    if let Some((_, fb)) = FRAMEBUFFER.try_get() {
        fb.force_unlock();
    }
}

/// Try to initialize the framebuffer based on the provided [`BootInfo`].
///
/// Returns `true` if the framebuffer is available, or `false` if there is no
/// framebuffer enabled.
///
/// If the framebuffer has already been initialized, this does nothing.
pub(super) fn init(bootinfo: &mut BootInfo) -> bool {
    use info::Optional;
    // Has the framebuffer already been initialized?
    if FRAMEBUFFER.try_get().is_some() {
        return true;
    }

    // Okay, try to initialize the framebuffer
    let Optional::Some(framebuffer) = mem::replace(&mut bootinfo.framebuffer, Optional::None)
    else {
        // The boot info does not contain a framebuffer configuration. Nothing
        // for us to do!
        return false;
    };

    let info = framebuffer.info();
    let cfg = framebuffer::Config {
        height: info.height,
        width: info.width,
        px_bytes: info.bytes_per_pixel,
        line_len: info.stride,
        px_kind: match info.pixel_format {
            info::PixelFormat::Rgb => framebuffer::PixelKind::Rgb,
            info::PixelFormat::Bgr => framebuffer::PixelKind::Bgr,
            info::PixelFormat::U8 => framebuffer::PixelKind::Gray,
            x => unimplemented!("hahaha wtf, found a weird pixel format: {:?}", x),
        },
    };
    FRAMEBUFFER.init((cfg, spin::Mutex::new(framebuffer)));
    true
}

static FRAMEBUFFER: InitOnce<(framebuffer::Config, spin::Mutex<info::FrameBuffer>)> =
    InitOnce::uninitialized();

impl Deref for FramebufGuard {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.0.buffer()
    }
}

impl DerefMut for FramebufGuard {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.buffer_mut()
    }
}
