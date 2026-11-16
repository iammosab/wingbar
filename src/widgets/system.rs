//! Built-ins read in-process (no shell): CPU, memory, battery, sound.

use super::{Button, Failure, Scroll, Show, Widget, has_action, item, run_action};
use crate::config::parse::CAFFEINATE;
use crate::config::{BatteryOptions, IconSet, Sampled, Theme};
use crate::sched::{self, Every};
use crate::state::Item;
use crate::sys;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ── cpu ──
pub struct Cpu {
    key: String,
    every_ms: u64,
    click: String,
    theme: Theme,
}

impl Cpu {
    pub fn new(key: &str, o: &Sampled, theme: &Theme) -> Cpu {
        Cpu { key: key.into(), every_ms: o.every_ms, click: o.click.clone(), theme: theme.clone() }
    }
}

fn cpu_item(percent: f64, t: &Theme) -> Item {
    let tc = if percent >= 90.0 {
        t.alert
    } else if percent >= 70.0 {
        t.warn
    } else {
        t.text
    };
    item(Some(t.icons.pick("cpu", "CPU")), format!("{percent:.0}%"), t.dim, tc)
}

impl Widget for Cpu {
    fn key(&self) -> &str {
        &self.key
    }
    fn schedule(&self) -> Every {
        Every::Ms(self.every_ms)
    }
    fn poll(&self) -> Result<Show, Failure> {
        Ok(sys::cpu_percent().map_or(Show::Keep, |p| Show::Item(cpu_item(p, &self.theme))))
    }
    fn clickable(&self) -> bool {
        has_action(&self.click)
    }
    fn click(&self, b: Button, _: Option<&str>) -> bool {
        b == Button::Left && run_action(&self.click)
    }
}

// ── memory ──
pub struct Memory {
    key: String,
    every_ms: u64,
    click: String,
    theme: Theme,
}

impl Memory {
    pub fn new(key: &str, o: &Sampled, theme: &Theme) -> Memory {
        Memory { key: key.into(), every_ms: o.every_ms, click: o.click.clone(), theme: theme.clone() }
    }
}

impl Widget for Memory {
    fn key(&self) -> &str {
        &self.key
    }
    fn schedule(&self) -> Every {
        Every::Ms(self.every_ms)
    }
    fn poll(&self) -> Result<Show, Failure> {
        let t = &self.theme;
        Ok(sys::memory_percent()
            .map_or(Show::Keep, |p| Show::Item(item(Some("memorychip"), format!("{p:.0}%"), t.dim, t.text))))
    }
    fn clickable(&self) -> bool {
        has_action(&self.click)
    }
    fn click(&self, b: Button, _: Option<&str>) -> bool {
        b == Button::Left && run_action(&self.click)
    }
}

// ── battery (macOS tells us when it changes; see `sys::watch_battery`) ──
pub struct Battery {
    key: String,
    /// `"caffeinate"` (toggle it), a shell command, or empty.
    click: String,
    caffeinate_option: String,
    theme: Theme,
}

/// The running `caffeinate`, while the cup is on. One for the bar, not per
/// widget, so a reload that makes the battery widget anew keeps it.
static CAFFEINATED: Mutex<Option<std::process::Child>> = Mutex::new(None);

impl Battery {
    pub fn new(key: &str, o: &BatteryOptions, theme: &Theme) -> Battery {
        Battery {
            key: key.into(),
            click: o.click.trim().to_string(),
            caffeinate_option: o.caffeinate_option.trim().to_string(),
            theme: theme.clone(),
        }
    }

    fn toggle_caffeinate(&self) {
        {
            let mut c = CAFFEINATED.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(mut child) = c.take() {
                let _ = child.kill();
                let _ = child.wait();
            } else {
                let mut cmd = std::process::Command::new("/usr/bin/caffeinate");
                if self.caffeinate_option.is_empty() {
                    cmd.arg("-d").arg("-i");
                } else {
                    cmd.args(self.caffeinate_option.split_whitespace());
                }
                *c = cmd.spawn().ok();
            }
        }
        sched::refresh(&self.key);
    }
}

fn battery_item(b: &sys::Battery, caffeinated: bool, t: &Theme) -> Item {
    let icon = if b.charging || b.on_ac && b.percent >= 99 {
        "battery.100.bolt"
    } else {
        match b.percent {
            0..=12 => "battery.0",
            13..=37 => "battery.25",
            38..=62 => "battery.50",
            63..=87 => "battery.75",
            _ => "battery.100",
        }
    };
    let low = b.percent <= 15 && !b.on_ac;
    let mut it = item(
        Some(icon),
        format!("{}%", b.percent),
        if low { t.alert } else { t.dim },
        if low { t.alert } else { t.text },
    );
    if caffeinated {
        it.trail = Some(t.icons.pick("cup.and.saucer.fill", "Coffee").into());
    }
    it
}

impl Widget for Battery {
    fn key(&self) -> &str {
        &self.key
    }
    fn schedule(&self) -> Every {
        Every::Ms(60_000)
    }
    fn poll(&self) -> Result<Show, Failure> {
        let Some(b) = sys::battery() else { return Ok(Show::Hide) };
        let caffeinated = CAFFEINATED.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        Ok(Show::Item(battery_item(&b, caffeinated, &self.theme)))
    }
    fn clickable(&self) -> bool {
        has_action(&self.click)
    }
    fn click(&self, b: Button, _: Option<&str>) -> bool {
        if b != Button::Left {
            return false;
        }
        if self.click == CAFFEINATE {
            self.toggle_caffeinate();
            return true;
        }
        run_action(&self.click)
    }
}

// ── sound (CoreAudio tells us when it changes; see `sys::watch_audio`) ──
pub struct Sound {
    key: String,
    step: f64,
    theme: Theme,
    /// Trackpad scroll not yet turned into a volume step.
    acc: Mutex<f64>,
}

impl Sound {
    pub fn new(key: &str, step: f64, theme: &Theme) -> Sound {
        Sound { key: key.into(), step, theme: theme.clone(), acc: Mutex::new(0.0) }
    }
}

fn sound_item(v: &sys::Volume, t: &Theme) -> Item {
    let (sf, text) = match v.level {
        None => ("speaker.wave.2", "–".to_string()),
        Some(_) if v.muted => ("speaker.slash", "0%".to_string()),
        Some(l) if l <= 0.0 => ("speaker.slash", "0%".to_string()),
        Some(l) if l < 34.0 => ("speaker.wave.1", format!("{l:.0}%")),
        Some(l) if l < 67.0 => ("speaker.wave.2", format!("{l:.0}%")),
        Some(l) => ("speaker.wave.3", format!("{l:.0}%")),
    };
    if t.icons == IconSet::SimpleBar {
        // as the simple-bar theme showed it: 1–3 waves (to 33%, 66%, above),
        // and a red speaker with "Muted" when muted or at 0%
        // (as shown: 33.4 reads 33%)
        return match v.level.map(f64::round) {
            Some(l) if v.muted || l <= 0.0 => item(Some("SpeakerMuted"), "Muted".into(), t.alert, t.alert),
            Some(l) if l <= 33.0 => item(Some("Speaker1"), text, t.dim, t.text),
            Some(l) if l <= 66.0 => item(Some("Speaker2"), text, t.dim, t.text),
            _ => item(Some("Speaker3"), text, t.dim, t.text),
        };
    }
    item(Some(sf), text, t.dim, t.text)
}

impl Widget for Sound {
    fn key(&self) -> &str {
        &self.key
    }
    fn schedule(&self) -> Every {
        Every::Ms(60_000)
    }
    fn poll(&self) -> Result<Show, Failure> {
        Ok(sys::volume().map_or(Show::Hide, |v| Show::Item(sound_item(&v, &self.theme))))
    }
    fn clickable(&self) -> bool {
        true
    }
    /// Left click: mute / unmute.
    fn click(&self, b: Button, _: Option<&str>) -> bool {
        if b != Button::Left {
            return false;
        }
        if let Some(v) = sys::volume() {
            sys::set_mute(!v.muted);
        }
        sched::refresh(&self.key);
        true
    }
    /// Up = louder; ⌥ for 1% steps. A trackpad moves one step per 12 points.
    fn scroll(&self, s: Scroll) -> bool {
        let step = if s.fine { 1.0 } else { self.step };
        if !s.precise {
            nudge_volume(&self.key, s.dy.signum() * step);
            return true;
        }
        let mut acc = self.acc.lock().unwrap_or_else(|e| e.into_inner());
        *acc += s.dy;
        let notches = (*acc / 12.0).trunc();
        *acc -= notches * 12.0;
        if notches != 0.0 {
            nudge_volume(&self.key, notches * step);
        }
        true
    }
}

/// The volume a scroll gesture last set, and when, so the next step of the
/// same gesture steers from it instead of re-reading CoreAudio (which may not
/// have caught up yet).
static VOL_TARGET: Mutex<Option<(f64, Instant)>> = Mutex::new(None);

/// Steps of one gesture come closer together than this; after it, read the volume again.
const GESTURE_GAP: Duration = Duration::from_millis(600);

/// Where a step of `steps` points from: the gesture's last target if it is
/// recent, else the volume `read` now.
fn next_volume(last: Option<(f64, Instant)>, now: Instant, steps: f64, read: impl FnOnce() -> f64) -> f64 {
    let cur = match last {
        Some((v, at)) if now.saturating_duration_since(at) < GESTURE_GAP => v,
        _ => read(),
    };
    (cur + steps).clamp(0.0, 100.0).round()
}

fn nudge_volume(key: &str, steps: f64) {
    let now = Instant::now();
    let next = {
        let mut last = VOL_TARGET.lock().unwrap_or_else(|e| e.into_inner());
        let next = next_volume(*last, now, steps, || sys::volume().and_then(|v| v.level).unwrap_or(50.0));
        *last = Some((next, now));
        next
    };
    sys::set_volume(next);
    sched::refresh(key);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_colours_by_load() {
        // SF Symbols (the imported fixture uses simple-bar's icons)
        let t = crate::config::Theme { icons: crate::config::IconSet::Sf, ..crate::config::fixture().theme };
        let it = cpu_item(42.4, &t);
        assert_eq!((it.icon.as_deref(), it.text.as_str()), (Some("cpu"), "42%"));
        assert_eq!((it.icon_color, it.text_color), (t.dim, t.text));
        assert_eq!(cpu_item(70.0, &t).text_color, t.warn);
        assert_eq!(cpu_item(95.0, &t).text_color, t.alert);
    }

    #[test]
    fn battery_icons_and_low_warning() {
        // SF Symbols (the imported fixture uses simple-bar's icons)
        let t = crate::config::Theme { icons: crate::config::IconSet::Sf, ..crate::config::fixture().theme };
        let b = |percent, charging, on_ac| sys::Battery { percent, charging, on_ac };
        assert_eq!(battery_item(&b(50, false, false), false, &t).icon.as_deref(), Some("battery.50"));
        assert_eq!(battery_item(&b(50, true, true), false, &t).icon.as_deref(), Some("battery.100.bolt"));
        assert_eq!(battery_item(&b(100, false, true), false, &t).icon.as_deref(), Some("battery.100.bolt"));
        let low = battery_item(&b(10, false, false), false, &t);
        assert_eq!((low.icon.as_deref(), low.text_color, low.icon_color), (Some("battery.0"), t.alert, t.alert));
        assert_eq!(battery_item(&b(10, false, true), false, &t).text_color, t.text, "not low on AC power");
        assert_eq!(battery_item(&b(80, false, false), true, &t).trail.as_deref(), Some("cup.and.saucer.fill"));
    }

    #[test]
    fn volume_steps_follow_the_gesture_then_reread() {
        let t0 = Instant::now();
        let reads = std::cell::Cell::new(0);
        let read = |v: f64| {
            reads.set(reads.get() + 1);
            v
        };
        // the first step reads the volume
        assert_eq!(next_volume(None, t0, 5.0, || read(40.0)), 45.0);
        // later steps of the same gesture steer from the last target, not a fresh read
        let t1 = t0 + Duration::from_millis(100);
        assert_eq!(next_volume(Some((45.0, t0)), t1, 5.0, || read(40.0)), 50.0);
        assert_eq!(reads.get(), 1);
        // once the gesture is over, read again
        let t2 = t1 + GESTURE_GAP;
        assert_eq!(next_volume(Some((50.0, t1)), t2, -1.0, || read(30.0)), 29.0);
        assert_eq!(reads.get(), 2);
        // clamped and rounded
        assert_eq!(next_volume(Some((98.0, t2)), t2, 5.0, || read(0.0)), 100.0);
        assert_eq!(next_volume(None, t2, -5.0, || read(2.4)), 0.0);
        assert_eq!(next_volume(None, t2, 1.0, || read(33.6)), 35.0);
    }

    #[test]
    fn sound_icons() {
        // SF Symbols (the imported fixture uses simple-bar's icons)
        let t = crate::config::Theme { icons: crate::config::IconSet::Sf, ..crate::config::fixture().theme };
        let v = |level, muted| sys::Volume { level, muted };
        let show = |vol: sys::Volume| {
            let it = sound_item(&vol, &t);
            (it.icon.unwrap(), it.text)
        };
        assert_eq!(show(v(None, false)), ("speaker.wave.2".into(), "–".into()));
        assert_eq!(show(v(Some(40.0), true)), ("speaker.slash".into(), "0%".into()));
        assert_eq!(show(v(Some(0.0), false)), ("speaker.slash".into(), "0%".into()));
        assert_eq!(show(v(Some(20.0), false)), ("speaker.wave.1".into(), "20%".into()));
        assert_eq!(show(v(Some(50.0), false)), ("speaker.wave.2".into(), "50%".into()));
        assert_eq!(show(v(Some(90.0), false)), ("speaker.wave.3".into(), "90%".into()));
    }

    #[test]
    fn simple_bar_icons_for_built_ins() {
        let mut t = crate::config::fixture().theme;
        t.icons = crate::config::IconSet::SimpleBar;
        let v = |level, muted| sys::Volume { level, muted };
        let show = |vol: sys::Volume| {
            let it = sound_item(&vol, &t);
            (it.icon.unwrap(), it.text, it.icon_color)
        };
        assert_eq!(show(v(Some(40.0), true)), ("SpeakerMuted".into(), "Muted".into(), t.alert));
        assert_eq!(show(v(Some(0.0), false)).0, "SpeakerMuted");
        assert_eq!(show(v(Some(33.0), false)), ("Speaker1".into(), "33%".into(), t.dim));
        assert_eq!(show(v(Some(34.0), false)).0, "Speaker2");
        assert_eq!(show(v(Some(66.0), false)).0, "Speaker2");
        assert_eq!(show(v(Some(67.0), false)).0, "Speaker3");
        assert_eq!(cpu_item(42.0, &t).icon.as_deref(), Some("CPU"));
        let b = sys::Battery { percent: 50, charging: false, on_ac: false };
        assert_eq!(battery_item(&b, true, &t).trail.as_deref(), Some("Coffee"));
        for name in ["SpeakerMuted", "Speaker1", "Speaker2", "Speaker3", "CPU", "Coffee", "Playing"] {
            assert!(crate::icons::exists(name), "{name} is bundled");
        }
    }
}
