//! One bar per settings file. A bar starting while another uses the same
//! settings takes over: it asks the other to quit, waits for it to go (and
//! makes it, if it won't), then runs. Two bars on top of each other, left by
//! an upgrade or a second launch, sort themselves out with nobody closing one
//! by hand. A bar on other settings (`WINGBAR_CONFIG`, as in development) is
//! left alone.
//!
//! - A bar holds an exclusive lock on a file in the user's temporary folder,
//!   named after its settings path, for as long as it runs; the file holds its
//!   pid. The lock goes when the process does, however it ends.
//! - Bars from before 0.3.3 hold no lock: one answering on the port and
//!   saying it uses the same settings is asked to `/quit`.
//!
//! A bar asked to quit exits with status 0 (SIGTERM is handled so), so
//! launchd, which restarts the bar only after a failure, leaves it down.

use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The lock file's name for a settings path: stable across runs (FNV-1a).
pub fn lock_name(settings: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in settings.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("wingbar-{h:016x}.lock")
}

fn temp_dir() -> PathBuf {
    objc2::rc::autoreleasepool(|_| PathBuf::from(objc2_foundation::NSTemporaryDirectory().to_string()))
}

/// The lock at `path` if it is free now (`None` while another process holds it).
fn try_lock(path: &Path) -> std::io::Result<Option<File>> {
    let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
    // SAFETY: flock(2) on a descriptor we own, with plain integer flags.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(f));
    }
    let e = std::io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::EWOULDBLOCK) { Ok(None) } else { Err(e) }
}

/// The pid written in a lock file, if it is someone else's.
fn holder(path: &Path) -> Option<i32> {
    let mut s = String::new();
    File::open(path).ok()?.read_to_string(&mut s).ok()?;
    let pid: i32 = s.trim().parse().ok()?;
    // SAFETY: getpid(2) has no arguments and can't fail.
    (pid > 1 && pid != unsafe { libc::getpid() }).then_some(pid)
}

/// Try for the lock until `limit`, every 100 ms.
fn wait_for_lock(path: &Path, limit: Duration) -> Option<File> {
    let start = Instant::now();
    loop {
        if let Ok(Some(f)) = try_lock(path) {
            return Some(f);
        }
        if start.elapsed() >= limit {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Become the only bar on `settings`: take over from one holding the lock,
/// and from an older bar answering on `port` (when this bar runs its server).
/// Returns a note for the log when it replaced one.
pub fn take_over(settings: &Path, port: Option<u16>) -> Option<String> {
    let path = temp_dir().join(lock_name(settings));
    let mut note = None;
    let lock = match try_lock(&path) {
        Ok(Some(f)) => Some(f),
        Ok(None) => {
            let pid = holder(&path);
            if let Some(pid) = pid {
                // SAFETY: kill(2) with plain integers, to the process that holds the lock.
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
            let got = wait_for_lock(&path, Duration::from_secs(3)).or_else(|| {
                if let Some(pid) = pid {
                    // SAFETY: as above; it ignored the polite request.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
                wait_for_lock(&path, Duration::from_secs(2))
            });
            note = Some(match pid {
                Some(pid) => format!("replaced the bar already running on these settings (pid {pid})"),
                None => "replaced the bar already running on these settings".into(),
            });
            got
        }
        Err(e) => {
            eprintln!("wingbar: can't lock {}: {e}", path.display());
            None
        }
    };
    if let Some(mut f) = lock {
        let _ = f.set_len(0);
        let _ = f.rewind();
        // SAFETY: getpid(2) has no arguments and can't fail.
        let _ = write!(f, "{}", unsafe { libc::getpid() });
        // held until the process ends
        std::mem::forget(f);
    }
    if let Some(port) = port
        && older_bar_on(port, settings)
    {
        let _ = crate::cli::request(port, "/quit", Duration::from_secs(2));
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3)
            && crate::cli::request(port, "/status", Duration::from_millis(300)).is_ok()
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        note = Some(format!("replaced the older bar on port {port} (same settings)"));
    }
    note
}

/// A bar answers on `port` and says it uses `settings`.
fn older_bar_on(port: u16, settings: &Path) -> bool {
    let Ok((200, body)) = crate::cli::request(port, "/widgets", Duration::from_secs(2)) else { return false };
    serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("settings")?.as_str().map(PathBuf::from))
        .is_some_and(|p| p == settings)
}

extern "C" fn on_sigterm(_: libc::c_int) {
    // SAFETY: _exit(2) is async-signal-safe; nothing else is done here.
    unsafe { libc::_exit(0) }
}

/// Quit with status 0 on SIGTERM (a takeover, `launchctl bootout`, logout),
/// so launchd doesn't start a replaced bar again.
pub fn exit_cleanly_on_sigterm() {
    let h: extern "C" fn(libc::c_int) = on_sigterm;
    // SAFETY: installs a handler that only calls an async-signal-safe function.
    unsafe { libc::signal(libc::SIGTERM, h as libc::sighandler_t) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_lock_file_per_settings_path() {
        let a = lock_name(Path::new("/Users/x/.config/wingbar/config.json"));
        assert_eq!(a, lock_name(Path::new("/Users/x/.config/wingbar/config.json")), "stable");
        assert_ne!(a, lock_name(Path::new("/tmp/dev/config.json")), "other settings, other lock");
        assert!(a.starts_with("wingbar-") && a.ends_with(".lock") && a.len() == "wingbar-.lock".len() + 16);
    }

    #[test]
    fn the_lock_is_held_until_dropped() {
        let dir = std::env::temp_dir().join(format!("wingbar-lock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.lock");
        let first = try_lock(&path).unwrap().expect("free at first");
        assert!(try_lock(&path).unwrap().is_none(), "held");
        drop(first);
        assert!(wait_for_lock(&path, Duration::from_millis(300)).is_some(), "free again");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_lock_files_pid_counts_only_when_it_is_someone_elses() {
        let dir = std::env::temp_dir().join(format!("wingbar-holder-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.lock");
        std::fs::write(&path, "4242\n").unwrap();
        assert_eq!(holder(&path), Some(4242));
        std::fs::write(&path, std::process::id().to_string()).unwrap();
        assert_eq!(holder(&path), None, "our own");
        std::fs::write(&path, "x").unwrap();
        assert_eq!(holder(&path), None);
        std::fs::write(&path, "1").unwrap();
        assert_eq!(holder(&path), None, "never launchd");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
