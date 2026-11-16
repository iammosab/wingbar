//! kqueue(2) for file events: a queue, vnode watches on open file
//! descriptors, and a wait with a timeout. Used to watch the settings folder.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Duration;

/// `NOTE_*` flags for a vnode watch. On a folder, `WRITE` fires when an entry
/// in it is created, removed or renamed.
pub const WRITE: u32 = libc::NOTE_WRITE;
pub const EXTEND: u32 = libc::NOTE_EXTEND;
pub const ATTRIB: u32 = libc::NOTE_ATTRIB;
pub const DELETE: u32 = libc::NOTE_DELETE;
pub const RENAME: u32 = libc::NOTE_RENAME;
pub const REVOKE: u32 = libc::NOTE_REVOKE;

/// A kqueue, closed when dropped (which also drops every watch on it).
pub struct Kqueue(OwnedFd);

/// Open `path` only to watch it (`O_EVTONLY`: it doesn't keep its volume
/// from being unmounted). Follows a symbolic link.
pub fn open_for_events(path: &Path) -> io::Result<OwnedFd> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
    // SAFETY: `c` is a NUL-terminated path that lives across the call; open(2)
    // only reads it, and the flags take no mode argument.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_EVTONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: open(2) just returned this descriptor, and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Kqueue {
    pub fn new() -> io::Result<Kqueue> {
        // SAFETY: kqueue(2) takes no arguments.
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: kqueue(2) just returned this descriptor, and nothing else owns it.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // the queue isn't inherited by the scripts the bar starts
        // SAFETY: fcntl(2) on a descriptor we own, with an integer argument.
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        Ok(Kqueue(fd))
    }

    /// Report `flags` (`WRITE`, `DELETE`, ...) on `fd`, once per change
    /// (`EV_CLEAR`). The watch lasts until `fd` is closed.
    pub fn watch(&self, fd: BorrowedFd<'_>, flags: u32) -> io::Result<()> {
        let ev = libc::kevent {
            ident: fd.as_raw_fd() as usize,
            filter: libc::EVFILT_VNODE,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_CLEAR,
            fflags: flags,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: one valid change record in, no event list out (count 0, NULL);
        // the kqueue and `fd` are open descriptors.
        let r = unsafe { libc::kevent(self.0.as_raw_fd(), &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// Wait up to `timeout` (`None`: for ever) for events, and append each
    /// one's (descriptor, flags) to `out`. Returns how many came: 0 on a
    /// timeout or an interrupted wait.
    pub fn wait(&self, timeout: Option<Duration>, out: &mut Vec<(RawFd, u32)>) -> io::Result<usize> {
        const N: usize = 8;
        // SAFETY: `kevent` is plain integers and a pointer, for which all zeroes is valid.
        let mut evs: [libc::kevent; N] = unsafe { std::mem::zeroed() };
        let ts = timeout.map(|t| libc::timespec {
            tv_sec: t.as_secs().min(i64::MAX as u64) as libc::time_t,
            tv_nsec: t.subsec_nanos() as libc::c_long,
        });
        let tsp = ts.as_ref().map_or(std::ptr::null(), |t| t as *const libc::timespec);
        // SAFETY: no changes in (NULL, 0); `evs` is writable for N records; `tsp`
        // is NULL or points to `ts`, which lives across the call.
        let n = unsafe { libc::kevent(self.0.as_raw_fd(), std::ptr::null(), 0, evs.as_mut_ptr(), N as i32, tsp) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted { Ok(0) } else { Err(e) };
        }
        let n = n as usize;
        out.extend(evs[..n].iter().map(|e| (e.ident as RawFd, e.fflags)));
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    #[test]
    fn a_write_to_a_watched_file_is_reported() {
        let dir = std::env::temp_dir().join(format!("wingbar-kqueue-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f");
        std::fs::write(&file, "a").unwrap();
        let kq = Kqueue::new().unwrap();
        let fd = open_for_events(&file).unwrap();
        kq.watch(fd.as_fd(), WRITE | EXTEND | DELETE | RENAME).unwrap();
        let mut got = Vec::new();
        assert_eq!(kq.wait(Some(Duration::from_millis(50)), &mut got).unwrap(), 0, "nothing yet");
        std::fs::write(&file, "bb").unwrap();
        assert_eq!(kq.wait(Some(Duration::from_secs(2)), &mut got).unwrap(), 1);
        assert_eq!(got[0].0, fd.as_raw_fd());
        assert!(got[0].1 & (WRITE | EXTEND) != 0, "{:#x}", got[0].1);
        assert!(open_for_events(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
