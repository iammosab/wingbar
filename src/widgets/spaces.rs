//! The workspace numbers (AeroSpace or yabai, see `wm`), drawn as their own
//! segment: click one to focus it, scroll over them to go to the next one.

use super::{Button, Failure, Kind, Scroll, Show, Widget};
use crate::sched::Every;
use crate::wm;
use std::sync::Mutex;

/// How much scrolling makes one workspace step.
const STEP: f64 = 30.0;

pub struct Spaces {
    key: String,
    /// Scroll not yet turned into a step.
    acc: Mutex<f64>,
}

impl Spaces {
    pub fn new(key: &str) -> Spaces {
        Spaces { key: key.into(), acc: Mutex::new(0.0) }
    }
}

impl Widget for Spaces {
    fn key(&self) -> &str {
        &self.key
    }
    fn kind(&self) -> Kind {
        Kind::Spaces
    }
    /// A fallback: AeroSpace's workspace-change hook and macOS's space-change
    /// notification ask for a refresh the moment something changes.
    fn schedule(&self) -> Every {
        Every::Ms(5000)
    }
    /// Writes the workspaces into the state itself.
    fn poll(&self) -> Result<Show, Failure> {
        wm::refresh_spaces().map(|()| Show::Keep).map_err(|error| Failure::new(error, Show::Keep))
    }
    /// Left click on a workspace: focus it.
    fn click(&self, b: Button, part: Option<&str>) -> bool {
        match part {
            Some(name) if b == Button::Left => {
                wm::focus(name);
                true
            }
            _ => false,
        }
    }
    fn scroll(&self, s: Scroll) -> bool {
        let mut acc = self.acc.lock().unwrap_or_else(|e| e.into_inner());
        *acc += if s.precise { s.dy } else { s.dy.signum() * STEP };
        if acc.abs() >= STEP {
            wm::cycle(*acc < 0.0);
            *acc = 0.0;
        }
        true
    }
}
