#![no_std]

use core::ptr::{null, null_mut};

use userspace as _;
use userspace::common::porcelain::{time, serial};

#[repr(i32)]
#[allow(non_camel_case_types)]
pub enum StatusCode {
    STATUS_BAD = -1,
    STATUS_GOOD = 0,
}

#[repr(C)]
pub struct Slice {
    ptr: *const u8,
    len: usize,
}

#[repr(C)]
pub struct SliceMut {
    ptr: *mut u8,
    len: usize,
}

impl Slice {
    fn is_nonnull_nonzero(&self) -> bool {
        self.ptr != null() && self.len != 0
    }
}

impl SliceMut {
    fn is_nonnull_nonzero(&self) -> bool {
        self.ptr != null_mut() && self.len != 0
    }
}

/// Open a virtual serial port. This must be called (and return
/// successfully before using `serial_read_port()` or
/// `serial_write_port()` on any port OTHER than port 0, which
/// is opened automatically.
#[no_mangle]
pub extern "C" fn serial_open_port(port: u16) -> StatusCode {
    match serial::open_port(port) {
        Ok(()) => StatusCode::STATUS_GOOD,
        Err(()) => StatusCode::STATUS_BAD,
    }
}

/// Sleep for (at least) the given number of microseconds
#[no_mangle]
pub extern "C" fn time_sleep_us(us: u32) -> StatusCode {
    match time::sleep_micros(us) {
        Ok(_) => StatusCode::STATUS_GOOD,
        Err(()) => StatusCode::STATUS_BAD,
    }
}

/// Attempt to fill the `slice` with data from the requested port.
///
/// On success, the slice will be updated with the length of data read
/// to the associated pointer. On failure, the length will be truncated
/// to zero.
#[no_mangle]
pub extern "C" fn serial_read_port(port: u16, slice: *mut SliceMut) -> StatusCode {
    if slice.is_null() {
        return StatusCode::STATUS_BAD;
    }
    let slice_mut = unsafe { &mut *slice };

    if !slice_mut.is_nonnull_nonzero() {
        return StatusCode::STATUS_BAD;
    }

    let contents = unsafe { core::slice::from_raw_parts_mut(slice_mut.ptr, slice_mut.len) };
    match serial::read_port(port, contents) {
        Ok(read) if read.len() <= slice_mut.len => {
            slice_mut.len = read.len();
            StatusCode::STATUS_GOOD
        },
        _ => {
            slice_mut.len = 0;
            StatusCode::STATUS_BAD
        },
    }
}

/// Attempt to send the contents of `slice` to the requested port.
///
/// On success, the slice (pointer and len) will be updated with the
/// start point and length of data NOT sent to the associated port.
/// If the length is set to zero, then all data was sent.
///
/// On failure, the slice will not be modified.
#[no_mangle]
pub extern "C" fn serial_read_write(port: u16, slice: *mut Slice) -> StatusCode {
    if slice.is_null() {
        return StatusCode::STATUS_BAD;
    }
    let slice = unsafe { &mut *slice };

    if !slice.is_nonnull_nonzero() {
        return StatusCode::STATUS_BAD;
    }

    let contents = unsafe { core::slice::from_raw_parts(slice.ptr, slice.len) };
    match serial::write_port(port, contents) {
        Ok(None) => {
            slice.len = 0;
            StatusCode::STATUS_GOOD
        }
        Ok(Some(remainder)) if remainder.len() <= slice.len => {
            slice.ptr = remainder.as_ptr();
            slice.len = remainder.len();

            StatusCode::STATUS_GOOD
        },
        _ => {
            slice.len = 0;
            StatusCode::STATUS_BAD
        },
    }
}
