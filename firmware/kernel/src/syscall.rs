// TODO: For now, assume all syscalls are blocking, non-reentrant, and all
// that other good stuff

use core::sync::atomic::Ordering;
use common::{SYSCALL_IN_PTR, SYSCALL_IN_LEN, SYSCALL_OUT_PTR, SYSCALL_OUT_LEN};
use common::{SysCallRequest, SysCallSuccess};

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

// This is really only a "kernel" thing...
pub fn try_recv_syscall<'a, F>(hdlr: F) -> Result<(), ()>
where
    // Note: We only need one lifetime here, which is the handling duration
    // of the syscall. Userspace has two, since it has a different view of
    // the data. We need to be rid of BOTH before we are done handling the
    // syscall.
    F: FnOnce(SysCallRequest<'a>) -> Result<SysCallSuccess<'a>, ()>
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
    let request = match postcard::from_bytes(inp_slice) {
        Ok(req) => req,
        Err(_) => {
            // ANGERY
            SYSCALL_OUT_LEN.store(0, Ordering::SeqCst);
            return Err(());
        },
    };

    let response = match hdlr(request) {
        Ok(resp) => resp,
        Err(_) => {
            // ANGERY
            SYSCALL_OUT_LEN.store(0, Ordering::SeqCst);
            return Err(());
        },
    };

    let out_slice = unsafe { core::slice::from_raw_parts_mut(out_ptr, out_len) };

    let used = match postcard::to_slice(&response, out_slice) {
        Ok(ser) => ser.len(),
        Err(_) => {
            // ANGERY
            SYSCALL_OUT_LEN.store(0, Ordering::SeqCst);
            return Err(());
        },
    };

    // Happy!
    SYSCALL_OUT_LEN.store(used, Ordering::SeqCst);

    Ok(())
}
