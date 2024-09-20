// copied from std::sys::pal::common::small_c_string

use std::ffi::{CStr, CString};
use std::io::Error;
use std::mem::MaybeUninit;
use std::path::Path;
use std::slice;
use std::{io, ptr};

const MAX_STACK_ALLOCATION: usize = 384;

#[inline]
pub fn run_path_with_cstr<T>(path: &Path, f: &dyn Fn(&CStr) -> io::Result<T>) -> io::Result<T> {
    run_with_cstr(path.as_os_str().as_encoded_bytes(), f)
}

#[inline]
pub fn run_with_cstr<T>(bytes: &[u8], f: &dyn Fn(&CStr) -> io::Result<T>) -> io::Result<T> {
    // Dispatch and dyn erase the closure type to prevent mono bloat.
    // See https://github.com/rust-lang/rust/pull/121101.
    if bytes.len() >= MAX_STACK_ALLOCATION {
        run_with_cstr_allocating(bytes, f)
    } else {
        unsafe { run_with_cstr_stack(bytes, f) }
    }
}

/// # Safety
///
/// `bytes` must have a length less than `MAX_STACK_ALLOCATION`.
unsafe fn run_with_cstr_stack<T>(
    bytes: &[u8],
    f: &dyn Fn(&CStr) -> io::Result<T>,
) -> io::Result<T> {
    let mut buf = MaybeUninit::<[u8; MAX_STACK_ALLOCATION]>::uninit();
    let buf_ptr = buf.as_mut_ptr() as *mut u8;

    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), buf_ptr, bytes.len());
        buf_ptr.add(bytes.len()).write(0);
    }

    match CStr::from_bytes_with_nul(unsafe { slice::from_raw_parts(buf_ptr, bytes.len() + 1) }) {
        Ok(s) => f(s),
        Err(_) => Err(null_err()),
    }
}

#[cold]
#[inline(never)]
fn run_with_cstr_allocating<T>(bytes: &[u8], f: &dyn Fn(&CStr) -> io::Result<T>) -> io::Result<T> {
    match CString::new(bytes) {
        Ok(s) => f(&s),
        Err(_) => Err(null_err()),
    }
}

fn null_err() -> Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "file name contained an unexpected NUL byte",
    )
}
