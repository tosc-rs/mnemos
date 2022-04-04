// TODO: For now, assume all syscalls are blocking, non-reentrant, and all
// that other good stuff

use core::{
    sync::atomic::{AtomicUsize, AtomicPtr, Ordering},
    ptr::null_mut, arch::asm,
};

// TODO: This is a little "woo" because I'm using usize and ptr, instead of a fixed
// u32 size. That being said... It's fine for now.

#[link_section=".bridge.syscall_in.ptr"]
pub static SYSCALL_IN_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());
#[link_section=".bridge.syscall_in.len"]
pub static SYSCALL_IN_LEN: AtomicUsize = AtomicUsize::new(0);

#[link_section=".bridge.syscall_out.ptr"]
pub static SYSCALL_OUT_PTR: AtomicPtr<u8> = AtomicPtr::new(null_mut());
#[link_section=".bridge.syscall_out.len"]
pub static SYSCALL_OUT_LEN: AtomicUsize = AtomicUsize::new(0);


// TODO: This is really only a "kernel" thing...
// DON'T call this in the svc handler! Userspace should clean up after
// itself, not the kernel, because it needs to "catch" the modified
// output len, and can't reset the in ptr before then.
pub fn syscall_clear() {
    SYSCALL_OUT_PTR.store(core::ptr::null_mut(), Ordering::SeqCst);
    SYSCALL_IN_LEN.store(0, Ordering::SeqCst);
    SYSCALL_OUT_LEN.store(0, Ordering::SeqCst);

    // TODO: Always do this last, for ABI reasons?
    SYSCALL_IN_PTR.store(core::ptr::null_mut(), Ordering::SeqCst);
}

// TODO: This is a userspace (and idle?) thing...
pub fn try_syscall<'i, 'o>(input: &'i [u8], output: &'o mut [u8]) -> Result<&'o mut [u8], ()> {
    let in_ptr = input.as_ptr() as *mut u8;

    // Try to atomically swap the in ptr for our input parameter. If this fails,
    // it means another syscall is in progress, and we should try again later.
    //
    // An "idle" syscall state is represented as a null pointer in the input
    // field.
    //
    // TODO: Should we just spin on this? Probably doesn't matter until we have
    // pre-emption, if ever...
    SYSCALL_IN_PTR
        .compare_exchange(
            null_mut(),
            in_ptr,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .map_err(drop)?;

    // We've made it past the hurdle! Fill the rest of the buffers, then trigger
    // the svc call
    SYSCALL_IN_LEN.store(input.len(), Ordering::SeqCst);
    SYSCALL_OUT_PTR.store(output.as_ptr() as *mut u8, Ordering::SeqCst);
    SYSCALL_OUT_LEN.store(output.len(), Ordering::SeqCst);

    unsafe {
        asm!("svc 0");
    }

    // Now we need to grab the output length, then reset all fields.
    let new_out_len = SYSCALL_OUT_LEN.swap(0, Ordering::SeqCst);
    SYSCALL_OUT_PTR.store(null_mut(), Ordering::SeqCst);
    SYSCALL_IN_LEN.store(0, Ordering::SeqCst);
    SYSCALL_IN_PTR.store(null_mut(), Ordering::SeqCst);

    if new_out_len == 0 {
        // This is bad. Just report it as an error for now
        Err(())
    } else {
        Ok(&mut output[..new_out_len])
    }
}

// TODO: This is really only a "kernel" thing...
pub fn try_recv_syscall<F>(hdlr: F) -> Result<(), ()>
where
    // Note: We only need one lifetime here, which is the handling duration
    // of the syscall. Userspace has two, since it has a different view of
    // the data. We need to be rid of BOTH before we are done handling the
    // syscall.
    F: FnOnce(&[u8], &mut [u8]) -> usize
{
    let inp_ptr = SYSCALL_IN_PTR.load(Ordering::SeqCst) as *const u8;
    let inp_len = SYSCALL_IN_LEN.load(Ordering::SeqCst);
    let out_ptr = SYSCALL_OUT_PTR.load(Ordering::SeqCst);
    let out_len = SYSCALL_OUT_LEN.load(Ordering::SeqCst);

    let any_zeros = [inp_ptr as usize, inp_len, out_ptr as usize, out_len].iter().any(|v| *v == 0);

    if any_zeros {
        // ANGERY
        SYSCALL_OUT_LEN.store(0, Ordering::SeqCst);
        return Err(());
    }

    // Okay, seems good, let's call the handler
    let inp_slice = unsafe { core::slice::from_raw_parts(inp_ptr, inp_len) };
    let out_slice = unsafe { core::slice::from_raw_parts_mut(out_ptr, out_len) };

    let out_len = hdlr(inp_slice, out_slice);

    // Happy!
    SYSCALL_OUT_LEN.store(out_len, Ordering::SeqCst);

    Ok(())
}

