use core::{
    ptr,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

use hal_core::{interrupt, VAddr};
pub use hal_x86_64::interrupt::*;
use hal_x86_64::{
    cpu::{intrinsics, Ring},
    segment::{self, Gdt},
    task,
};
use kernel::maitake::time;
use mycelium_util::{fmt, sync};

#[tracing::instrument]
pub fn enable_exceptions() {
    init_gdt();
    tracing::info!("GDT initialized!");

    Controller::init::<InterruptHandlers>();
    tracing::info!("IDT initialized!");
}

#[tracing::instrument(skip(acpi, timer))]
pub fn enable_hardware_interrupts(
    acpi: Option<&acpi::InterruptModel>,
    timer: &'static time::Timer,
) {
    // no way to have an atomic `*const` ptr lol :|
    let timer = timer as *const _ as *mut _;
    let _timer = TIMER.swap(timer, Ordering::Release);
    debug_assert_eq!(_timer, ptr::null_mut());

    let controller = Controller::enable_hardware_interrupts(acpi, &crate::allocator::HEAP);
    controller
        .start_periodic_timer(TIMER_INTERVAL)
        .expect("10ms should be a reasonable interval for the PIT or local APIC timer...");
    tracing::info!(granularity = ?TIMER_INTERVAL, "global timer initialized")
}

/// Wait for an interrupt in a spin loop.
///
/// This is distinct from `core::hint::spin_loop`, as it is intended
/// specifically for waiting for an interrupt, rather than progress from another
/// thread. This should be called on each iteration of a loop that waits on a condition
/// set by an interrupt handler.
///
/// This function will execute one [`intrinsics::sti`] instruction to enable interrupts
/// followed by one [`intrinsics::hlt`] instruction to halt the CPU.
#[inline(always)]
pub(crate) fn wait_for_interrupt() {
    unsafe {
        intrinsics::sti();
        intrinsics::hlt();
    }
}

// TODO(eliza): put this somewhere good.
type StackFrame = [u8; 4096];

// chosen by fair dice roll, guaranteed to be random
const DOUBLE_FAULT_STACK_SIZE: usize = 8;

/// Stack used by ISRs during a double fault.
///
/// /!\ EXTREMELY SERIOUS WARNING: this has to be `static mut` or else it
///     will go in `.bss` and we'll all die or something.
static mut DOUBLE_FAULT_STACK: [StackFrame; DOUBLE_FAULT_STACK_SIZE] =
    [[0; 4096]; DOUBLE_FAULT_STACK_SIZE];

static TSS: sync::Lazy<task::StateSegment> = sync::Lazy::new(|| {
    tracing::trace!("initializing TSS..");
    let mut tss = task::StateSegment::empty();
    tss.interrupt_stacks[Idt::DOUBLE_FAULT_IST_OFFSET] = unsafe {
        // safety: asdf
        VAddr::from_usize_unchecked(core::ptr::addr_of!(DOUBLE_FAULT_STACK) as usize)
            .offset(DOUBLE_FAULT_STACK_SIZE as i32)
    };
    tracing::debug!(?tss, "TSS initialized");
    tss
});

pub(crate) static GDT: sync::InitOnce<Gdt> = sync::InitOnce::uninitialized();

pub const TIMER_INTERVAL: time::Duration = time::Duration::from_millis(10);
static TIMER: AtomicPtr<time::Timer> = AtomicPtr::new(ptr::null_mut());

static TEST_INTERRUPT_WAS_FIRED: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct InterruptHandlers;

/// Forcibly unlock the IOs we write to in an oops (VGA buffer and COM1 serial
/// port) to prevent deadlocks if the oops occured while either was locked.
///
/// # Safety
///
///  /!\ only call this when oopsing!!! /!\
impl hal_core::interrupt::Handlers<Registers> for InterruptHandlers {
    fn page_fault<C>(cx: C)
    where
        C: interrupt::Context<Registers = Registers> + hal_core::interrupt::ctx::PageFault,
    {
        let fault_vaddr = cx.fault_vaddr();
        let code = cx.display_error_code();

        // TODO: add a nice fault handler
        panic!("page fault at {fault_vaddr:?}\n{code}");
    }

    fn code_fault<C>(cx: C)
    where
        C: interrupt::Context<Registers = Registers> + interrupt::ctx::CodeFault,
    {
        // TODO: add a nice fault handler
        match cx.details() {
            Some(deets) => panic!("code fault {}: \n{deets}", cx.fault_kind()),
            None => panic!("code fault {}!", cx.fault_kind()),
        };
    }

    fn double_fault<C>(_cx: C)
    where
        C: hal_core::interrupt::Context<Registers = Registers>,
    {
        // TODO: add a nice fault handler
        panic!("double fault");
    }

    fn timer_tick() {
        if let Some(timer) = ptr::NonNull::new(TIMER.load(Ordering::Acquire)) {
            unsafe { timer.as_ref() }.advance_ticks(1);
        }
    }

    fn ps2_keyboard(scancode: u8) {
        // TODO(eliza): add a keyboard driver
        tracing::info!(scancode, "keyoard interrupt!!!");
    }

    fn test_interrupt<C>(cx: C)
    where
        C: hal_core::interrupt::ctx::Context<Registers = Registers>,
    {
        let fired = TEST_INTERRUPT_WAS_FIRED.fetch_add(1, Ordering::Release) + 1;
        tracing::info!(registers = ?cx.registers(), fired, "lol im in ur test interrupt");
    }
}

#[inline]
#[tracing::instrument(level = tracing::Level::DEBUG)]
pub(super) fn init_gdt() {
    tracing::trace!("initializing GDT...");
    let mut gdt = Gdt::new();

    // add one kernel code segment
    let code_segment = segment::Descriptor::code().with_ring(Ring::Ring0);
    let code_selector = gdt.add_segment(code_segment);
    tracing::debug!(
        descriptor = ?fmt::alt(code_segment),
        selector = ?fmt::alt(code_selector),
        "added code segment"
    );

    // add the TSS.

    let tss = segment::SystemDescriptor::tss(&TSS);
    let tss_selector = gdt.add_sys_segment(tss);
    tracing::debug!(
        tss.descriptor = ?fmt::alt(tss),
        tss.selector = ?fmt::alt(tss_selector),
        "added TSS"
    );

    // all done! long mode barely uses this thing lol.
    GDT.init(gdt);

    // load the GDT
    let gdt = GDT.get();
    tracing::debug!(GDT = ?gdt, "GDT initialized");
    gdt.load();

    tracing::trace!("GDT loaded");

    // set new segment selectors
    let code_selector = segment::Selector::current_cs();
    tracing::trace!(code_selector = ?fmt::alt(code_selector));
    unsafe {
        // set the code segment selector
        code_selector.set_cs();

        // in protected mode and long mode, the code segment, stack segment,
        // data segment, and extra segment must all have base address 0 and
        // limit `2^64`, since actual segmentation is not used in those modes.
        // therefore, we must zero the SS, DS, and ES registers.
        segment::Selector::null().set_ss();
        segment::Selector::null().set_ds();
        segment::Selector::null().set_es();

        task::StateSegment::load_tss(tss_selector);
    }

    tracing::debug!("segment selectors set");
}
