//! Mach host statistics (mach/host_info.h).
//!
//! Both calls used here fill a caller-provided, fixed-size struct, so the
//! kernel hands back no out-of-line memory to `vm_deallocate`. (That would
//! be needed for `host_processor_info`, which this module does not use.)

use std::sync::OnceLock;

unsafe extern "C" {
    // Declared here because libc marks its copy deprecated in favour of the mach2 crate.
    fn mach_host_self() -> libc::mach_port_t;
}

/// The host port. `mach_host_self()` adds a user reference to our send right
/// on every call, so it is called exactly once and the right is kept for the
/// life of the process (calling it per reading would leak a reference each time).
fn host() -> libc::mach_port_t {
    static H: OnceLock<libc::mach_port_t> = OnceLock::new();
    // SAFETY: no arguments, no preconditions.
    *H.get_or_init(|| unsafe { mach_host_self() })
}

/// Cumulative CPU ticks for the whole machine: [user, system, idle, nice].
pub fn cpu_load_ticks() -> Option<[u32; 4]> {
    let mut info = libc::host_cpu_load_info { cpu_ticks: [0; libc::CPU_STATE_MAX as usize] };
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    // SAFETY: `info` is a `host_cpu_load_info` and `count` is its size in
    // `integer_t`s, so the kernel writes only inside it.
    let r = unsafe { libc::host_statistics(host(), libc::HOST_CPU_LOAD_INFO, (&raw mut info).cast(), &mut count) };
    let t = info.cpu_ticks;
    (r == libc::KERN_SUCCESS).then_some([
        t[libc::CPU_STATE_USER as usize],
        t[libc::CPU_STATE_SYSTEM as usize],
        t[libc::CPU_STATE_IDLE as usize],
        t[libc::CPU_STATE_NICE as usize],
    ])
}

/// Virtual memory page counts (`vm_statistics64`).
pub fn vm_stats() -> Option<libc::vm_statistics64> {
    // SAFETY: `vm_statistics64` holds only integers, so all-zero is a valid value.
    let mut v: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: `v` is a `vm_statistics64` and `count` is its size in `integer_t`s;
    // an older kernel fills a prefix and lowers `count`.
    let r = unsafe { libc::host_statistics64(host(), libc::HOST_VM_INFO64, (&raw mut v).cast(), &mut count) };
    (r == libc::KERN_SUCCESS).then_some(v)
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_port_is_cached() {
        assert_eq!(super::host(), super::host());
        assert_ne!(super::host(), 0);
    }

    #[test]
    fn cpu_ticks_only_go_up() {
        let a = super::cpu_load_ticks().unwrap();
        let b = super::cpu_load_ticks().unwrap();
        assert!(a.iter().zip(&b).all(|(a, b)| b >= a));
        assert!(b.iter().any(|&t| t > 0));
    }

    #[test]
    fn vm_stats_are_plausible() {
        let v = super::vm_stats().unwrap();
        assert!(v.wire_count > 0);
    }
}
