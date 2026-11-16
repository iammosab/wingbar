//! Process listing (libproc.h), without spawning `ps`/`pgrep`, and this
//! process's own resource usage (getrusage, proc_pid_rusage).

use std::ffi::c_void;

/// Every pid on the system.
pub fn all_pids() -> Vec<i32> {
    // SAFETY: a NULL buffer of size 0 only asks how many pids there are.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    // headroom for processes started between the two calls
    let mut pids = vec![0i32; count as usize + 64];
    let bytes = (pids.len() * size_of::<i32>()) as i32;
    // SAFETY: `pids` is writable for `bytes` bytes; the call writes at most that
    // and returns how many pids it wrote.
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast::<c_void>(), bytes) };
    pids.truncate(n.clamp(0, pids.len() as i32) as usize);
    pids
}

/// The short name of a process (as `ps -c` shows it), written into `buf`.
pub fn name(pid: i32, buf: &mut [u8; 256]) -> Option<&[u8]> {
    // SAFETY: `buf` is writable for its full length; proc_name writes at most that.
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast::<c_void>(), buf.len() as u32) };
    (n > 0).then(|| &buf[..(n as usize).min(buf.len())])
}

/// What this process has used so far (for `WINGBAR_MEASURE`, see `main.rs`).
/// CPU times are in seconds. The `child_*` times cover the child processes
/// that have been reaped (widget scripts, click actions), not this process.
#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub user: f64,
    pub system: f64,
    pub child_user: f64,
    pub child_system: f64,
    /// Voluntary context switches: how often a thread blocked and was woken again.
    pub voluntary_switches: i64,
    pub involuntary_switches: i64,
    /// Peak resident memory, in bytes.
    pub max_rss: u64,
    pub phys_footprint: u64,
    pub max_phys_footprint: u64,
    /// Wakeups that took a CPU package out of idle (Activity Monitor's "Idle Wake Ups").
    pub idle_wakeups: u64,
    /// Interrupt wakeups (mostly timers firing).
    pub interrupt_wakeups: u64,
    pub instructions: u64,
    pub cycles: u64,
}

fn rusage(who: libc::c_int) -> Option<libc::rusage> {
    // SAFETY: `rusage` is plain integers and timevals; all-zero is valid.
    let mut r: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `r` is a valid, writable `rusage`.
    (unsafe { libc::getrusage(who, &mut r) } == 0).then_some(r)
}

fn seconds(t: libc::timeval) -> f64 {
    t.tv_sec as f64 + t.tv_usec as f64 / 1e6
}

pub fn usage() -> Usage {
    let mut u = Usage::default();
    if let Some(r) = rusage(libc::RUSAGE_SELF) {
        u.user = seconds(r.ru_utime);
        u.system = seconds(r.ru_stime);
        u.voluntary_switches = r.ru_nvcsw;
        u.involuntary_switches = r.ru_nivcsw;
        u.max_rss = r.ru_maxrss as u64; // bytes on macOS
    }
    if let Some(r) = rusage(libc::RUSAGE_CHILDREN) {
        u.child_user = seconds(r.ru_utime);
        u.child_system = seconds(r.ru_stime);
    }
    // SAFETY: `rusage_info_v4` is plain integers; all-zero is valid.
    let mut ri: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let buf = (&mut ri as *mut libc::rusage_info_v4).cast::<libc::rusage_info_t>();
    // SAFETY: `buf` points to a writable `rusage_info_v4`, which is what the V4
    // flavor fills (libc types the pointer as `*mut rusage_info_t`).
    if unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, buf) } == 0 {
        u.phys_footprint = ri.ri_phys_footprint;
        u.max_phys_footprint = ri.ri_lifetime_max_phys_footprint;
        u.idle_wakeups = ri.ri_pkg_idle_wkups;
        u.interrupt_wakeups = ri.ri_interrupt_wkups;
        u.instructions = ri.ri_instructions;
        u.cycles = ri.ri_cycles;
    }
    u
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_our_own_process() {
        let me = std::process::id() as i32;
        assert!(super::all_pids().contains(&me));
        let mut buf = [0u8; 256];
        assert!(super::name(me, &mut buf).is_some_and(|n| !n.is_empty()));
    }

    #[test]
    fn usage_reads_this_process() {
        let mut x = 0u64;
        for i in 0..2_000_000u64 {
            x = std::hint::black_box(x.wrapping_add(i));
        }
        let u = super::usage();
        assert!(u.user + u.system > 0.0, "{u:?}");
        assert!(u.max_rss > 0 && u.phys_footprint > 0 && u.max_phys_footprint >= u.phys_footprint, "{u:?}");
    }
}
