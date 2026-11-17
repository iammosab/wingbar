//! The focused window's title ("App / Title"), drawn as its own segment.

use super::{Failure, Kind, Show, Widget};
use crate::sched::Every;
use crate::state;
use crate::sys;
use std::sync::atomic::{AtomicBool, Ordering};

/// Accessibility is granted: read titles in-process instead of asking the WM.
pub static AX_OK: AtomicBool = AtomicBool::new(false);

pub struct Title {
    key: String,
}

impl Title {
    pub fn new(key: &str) -> Title {
        Title { key: key.into() }
    }
}

impl Widget for Title {
    fn key(&self) -> &str {
        &self.key
    }
    fn kind(&self) -> Kind {
        Kind::Title
    }
    fn schedule(&self) -> Every {
        Every::Ms(if AX_OK.load(Ordering::Relaxed) { 1000 } else { 3000 })
    }
    /// Writes the title into the state itself. (The WM fallback has no error
    /// of its own to report: an empty workspace simply has no focused window.)
    fn poll(&self) -> Result<Show, Failure> {
        let (pid, app) = {
            let s = state::lock();
            (s.front_pid, s.front_name.clone())
        };
        // Accessibility granted after start (in System Settings): take it up now,
        // without a restart. AXIsProcessTrusted is an in-process check.
        if !AX_OK.load(Ordering::Relaxed) && sys::ax_trusted(false) {
            AX_OK.store(true, Ordering::Relaxed);
        }
        let t = if AX_OK.load(Ordering::Relaxed) && pid > 0 {
            let w = sys::window_title(pid).unwrap_or_default();
            if w.is_empty() || w == app { app } else { format!("{app} / {w}") }
        } else {
            match crate::wm::focused_title_from_wm() {
                Some((a, w)) if !w.is_empty() && w != a => format!("{a} / {w}"),
                Some((a, _)) => a,
                None => app,
            }
        };
        state::set_title(t);
        Ok(Show::Keep)
    }
}
