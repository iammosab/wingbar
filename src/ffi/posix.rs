//! Small libc wrappers: local time formatting, `confstr`, `sysctlbyname`, page size.

use std::ffi::{CStr, c_char};

/// Format the current local time with strftime(3). Empty if the result does
/// not fit in 128 bytes.
pub fn strftime_now(fmt: &CStr) -> String {
    // SAFETY: a NULL argument is allowed; the time is only returned.
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    // SAFETY: `tm` is plain integers plus a `tm_zone` pointer, for which NULL is valid.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid; `localtime_r` is the thread-safe variant.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return String::new();
    }
    let mut buf = [0u8; 128];
    // SAFETY: `buf` is writable for `buf.len()` bytes, `fmt` is NUL-terminated,
    // and `tm` was filled by `localtime_r`. The return value is at most `buf.len() - 1`.
    let n = unsafe { libc::strftime(buf.as_mut_ptr().cast::<c_char>(), buf.len(), fmt.as_ptr(), &tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// A configuration string from confstr(3), e.g. `_CS_DARWIN_USER_TEMP_DIR`.
pub fn confstr(name: libc::c_int) -> Option<String> {
    let mut buf = [0 as c_char; 1024];
    // SAFETY: `buf` is writable for `buf.len()` bytes; confstr NUL-terminates
    // what it writes (truncating if needed).
    let n = unsafe { libc::confstr(name, buf.as_mut_ptr(), buf.len()) };
    if n == 0 || n > buf.len() {
        return None; // unknown name, or the value didn't fit
    }
    // SAFETY: confstr wrote a NUL-terminated string into `buf`.
    Some(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
}

/// A 64-bit integer from sysctlbyname(3), e.g. `hw.memsize`.
pub fn sysctl_u64(name: &CStr) -> Option<u64> {
    let mut v: u64 = 0;
    let mut len = size_of::<u64>();
    // SAFETY: `v` is writable for `len` bytes; no new value is set.
    let r = unsafe { libc::sysctlbyname(name.as_ptr(), (&raw mut v).cast(), &mut len, std::ptr::null_mut(), 0) };
    (r == 0 && len == size_of::<u64>()).then_some(v)
}

/// The VM page size in bytes.
pub fn page_size() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if n > 0 { n as u64 } else { 4096 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strftime_fixed_formats() {
        assert_eq!(strftime_now(c"literal %% text"), "literal % text");
        let year: i32 = strftime_now(c"%Y").parse().unwrap();
        assert!(year >= 2024);
        assert_eq!(strftime_now(c"%H:%M").len(), 5);
        assert_eq!(strftime_now(c""), "");
    }

    #[test]
    fn strftime_overflow_is_empty() {
        let long = std::ffi::CString::new("x".repeat(200)).unwrap();
        assert_eq!(strftime_now(&long), "");
    }

    #[test]
    fn sysctl_reads_memsize() {
        assert!(sysctl_u64(c"hw.memsize").unwrap() > 1 << 30);
        assert_eq!(sysctl_u64(c"no.such.sysctl"), None);
    }

    #[test]
    fn page_size_is_a_power_of_two() {
        assert!(page_size().is_power_of_two());
    }
}
