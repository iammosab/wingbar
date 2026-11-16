//! Native system readings: no shell, no polling where macOS can tell us.
//! CPU and memory from the kernel, battery from IOKit (with a change callback),
//! volume from CoreAudio (with change listeners), window title from Accessibility.

use crate::ffi::ax::Element;
use crate::ffi::cf::{self, CFString};
use crate::ffi::coreaudio::{self as ca, Address, Api};
use crate::ffi::{iokit, mach, posix, proc};
use std::ffi::{CString, c_void};
use std::sync::{Mutex, Once, OnceLock};

// ── CPU and memory (Mach host statistics) ──────────────────────────────────
static PREV_TICKS: Mutex<Option<[u32; 4]>> = Mutex::new(None);

/// Whole-machine CPU use since the last call, in percent.
pub fn cpu_percent() -> Option<f64> {
    let t = mach::cpu_load_ticks()?; // user, system, idle, nice
    let mut prev = PREV_TICKS.lock().unwrap_or_else(|e| e.into_inner());
    let p = prev.replace(t).unwrap_or([0; 4]);
    let d: [f64; 4] = std::array::from_fn(|i| t[i].wrapping_sub(p[i]) as f64);
    let busy = d[0] + d[1] + d[3];
    let total = busy + d[2];
    (total > 0.0).then(|| busy / total * 100.0)
}

/// "Memory used" as Activity Monitor counts it (app + wired + compressed), in percent.
pub fn memory_percent() -> Option<f64> {
    static TOTAL: OnceLock<Option<u64>> = OnceLock::new();
    let total = (*TOTAL.get_or_init(|| posix::sysctl_u64(c"hw.memsize"))).filter(|&t| t > 0)?;
    let v = mach::vm_stats()?;
    let page = posix::page_size() as f64;
    let app = (v.internal_page_count as f64 - v.purgeable_count as f64).max(0.0);
    let used = (app + v.wire_count as f64 + v.compressor_page_count as f64) * page;
    Some((used / total as f64 * 100.0).clamp(0.0, 100.0))
}

// ── Battery (IOKit power sources) ──────────────────────────────────────────
pub struct Battery {
    pub percent: i64,
    pub charging: bool,
    pub on_ac: bool,
}

pub fn battery() -> Option<Battery> {
    let sources = iokit::PowerSources::copy()?;
    let d = sources
        .descriptions()
        .find(|d| cf::get(d, "Type").and_then(|v| cf::string(&v)).as_deref() == Some("InternalBattery"))?;
    let num = |k| cf::get(d, k).and_then(|v| cf::i64(&v));
    let cur = num("Current Capacity").unwrap_or(0);
    let max = num("Max Capacity").unwrap_or(100).max(1);
    Some(Battery {
        percent: (cur * 100 / max).clamp(0, 100),
        charging: cf::get(d, "Is Charging").and_then(|v| cf::bool(&v)).unwrap_or(false),
        on_ac: cf::get(d, "Power Source State").and_then(|v| cf::string(&v)).as_deref() == Some("AC Power"),
    })
}

extern "C" fn power_changed(_: *mut c_void) {
    crate::widgets::refresh_type(crate::config::WidgetType::Battery);
}

/// Ask macOS to tell us when the battery changes (call on the main thread).
/// Only the first call registers; later calls would add duplicate sources.
pub fn watch_battery() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        iokit::notify_on_main_run_loop(power_changed);
    });
}

// ── Volume (CoreAudio) ─────────────────────────────────────────────────────
const DEFAULT_OUT: Address = Address::new(ca::DEFAULT_OUTPUT_DEVICE, ca::SCOPE_GLOBAL, 0);
const MAIN_VOLUME: Address = Address::new(ca::VIRTUAL_MAIN_VOLUME, ca::SCOPE_OUTPUT, 0);
const MUTE: Address = Address::new(ca::MUTE, ca::SCOPE_OUTPUT, 0);
/// Per-device properties whose changes mean "redraw the volume".
const DEVICE_WATCH: [Address; 4] = [
    Address::new(ca::VOLUME_SCALAR, ca::SCOPE_OUTPUT, 0),
    Address::new(ca::VOLUME_SCALAR, ca::SCOPE_OUTPUT, 1),
    Address::new(ca::VOLUME_SCALAR, ca::SCOPE_OUTPUT, 2),
    MUTE,
];

fn output_device() -> Option<ca::ObjectId> {
    Api::Object.get::<u32>(ca::SYSTEM_OBJECT, &DEFAULT_OUT).filter(|&d| d != 0)
}

extern "C" fn audio_changed(_: u32, _: u32, _: *const Address, _: *mut c_void) -> i32 {
    crate::widgets::refresh_type(crate::config::WidgetType::Sound);
    0
}

/// The device whose volume/mute we listen to, and the addresses we registered on it.
struct Watched {
    dev: ca::ObjectId,
    addrs: Vec<Address>,
}
static WATCHED: Mutex<Option<Watched>> = Mutex::new(None);

/// Listen to `dev` and stop listening to the previous output device, so
/// exactly one device holds our listeners however often the output changes.
fn watch_device(dev: ca::ObjectId) {
    let mut w = WATCHED.lock().unwrap_or_else(|e| e.into_inner());
    if w.as_ref().is_some_and(|w| w.dev == dev) {
        return;
    }
    if let Some(old) = w.take() {
        for a in &old.addrs {
            ca::remove_listener(old.dev, a, audio_changed);
        }
    }
    let addrs = DEVICE_WATCH
        .into_iter()
        .filter(|a| Api::Object.has(dev, a) && ca::add_listener(dev, a, audio_changed))
        .collect();
    *w = Some(Watched { dev, addrs });
}

/// Listen for output device, volume and mute changes. Only the first call
/// registers the system-wide listener.
pub fn watch_audio() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        ca::add_listener(ca::SYSTEM_OBJECT, &DEFAULT_OUT, audio_changed);
    });
    if let Some(d) = output_device() {
        watch_device(d);
    }
}

pub struct Volume {
    /// 0..=100, None when the device has no volume control (HDMI, DisplayPort).
    pub level: Option<f64>,
    pub muted: bool,
}

pub fn volume() -> Option<Volume> {
    let dev = output_device()?;
    watch_device(dev);
    let level = Api::Service
        .has(dev, &MAIN_VOLUME)
        .then(|| Api::Service.get::<f32>(dev, &MAIN_VOLUME))
        .flatten()
        .map(|v| (v as f64 * 100.0).round());
    let muted = Api::Object.has(dev, &MUTE) && Api::Object.get::<u32>(dev, &MUTE).unwrap_or(0) != 0;
    Some(Volume { level, muted })
}

pub fn set_volume(percent: f64) {
    let Some(dev) = output_device() else { return };
    let v = (percent.clamp(0.0, 100.0) / 100.0) as f32;
    Api::Service.set(dev, &MAIN_VOLUME, v);
    if percent > 0.0 {
        set_mute(false);
    }
}

pub fn set_mute(mute: bool) {
    let Some(dev) = output_device() else { return };
    if Api::Object.has(dev, &MUTE) {
        Api::Object.set(dev, &MUTE, mute as u32);
    }
}

// ── Focused window title (Accessibility) ───────────────────────────────────
pub fn ax_trusted(prompt: bool) -> bool {
    crate::ffi::ax::is_trusted(prompt)
}

/// Title of the focused window of an app, read in-process (about a tenth of a millisecond).
pub fn window_title(pid: i32) -> Option<String> {
    let app = Element::application(pid)?;
    app.set_timeout(0.25);
    let win = app.element_attribute(&CFString::from_static_str("AXFocusedWindow"))?;
    let title = win.attribute(&CFString::from_static_str("AXTitle"))?;
    cf::string(&title)
}

// ── Is an app running? (libproc, no process spawned) ───────────────────────
pub fn process_running(name: &str) -> bool {
    let mut buf = [0u8; 256];
    proc::all_pids().into_iter().any(|pid| proc::name(pid, &mut buf) == Some(name.as_bytes()))
}

// ── Clock ──────────────────────────────────────────────────────────────────
/// The local time formatted with strftime(3). Empty for a format containing NUL.
pub fn strftime(fmt: &str) -> String {
    CString::new(fmt).map(|f| posix::strftime_now(&f)).unwrap_or_default()
}

/// The per-user temp folder (`getconf DARWIN_USER_TEMP_DIR`), ending in '/'.
pub fn user_temp_dir() -> String {
    static D: OnceLock<String> = OnceLock::new();
    D.get_or_init(|| {
        let s = posix::confstr(libc::_CS_DARWIN_USER_TEMP_DIR)
            .or_else(|| std::env::var("TMPDIR").ok())
            .unwrap_or_else(|| "/tmp/".into());
        if s.ends_with('/') { s } else { format!("{s}/") }
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_in_range_on_second_call() {
        cpu_percent();
        // the kernel's tick counts can lag on a busy machine: give them up to a second
        let p = (0..20)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(50));
                cpu_percent()
            })
            .expect("ticks advanced");
        assert!((0.0..=100.0).contains(&p), "{p}");
    }

    #[test]
    fn memory_percent_in_range() {
        let p = memory_percent().unwrap();
        assert!(p > 0.0 && p <= 100.0, "{p}");
    }

    #[test]
    fn strftime_formats_and_survives_nul() {
        assert_eq!(strftime("%%"), "%");
        assert_eq!(strftime("%Y").len(), 4);
        assert_eq!(strftime("bad\0fmt"), "");
    }

    #[test]
    fn user_temp_dir_ends_with_slash() {
        let d = user_temp_dir();
        assert!(d.ends_with('/'), "{d}");
        assert!(std::path::Path::new(&d).is_dir(), "{d}");
    }

    #[test]
    fn process_running_sees_a_child() {
        // libproc only names processes of our own user on current macOS (not
        // launchd, which is root's), which is all the bar needs for apps.
        let mut child = std::process::Command::new("/bin/sleep").arg("10").spawn().unwrap();
        let seen = process_running("sleep");
        let _ = child.kill();
        let _ = child.wait();
        assert!(seen);
        assert!(!process_running("wingbar-no-such-process-name"));
    }

    #[test]
    fn battery_reading_is_consistent() {
        // desktops have no internal battery; a laptop's reading must be in range
        if let Some(b) = battery() {
            assert!((0..=100).contains(&b.percent));
        }
    }

    #[test]
    fn window_title_of_nonexistent_pid_is_none() {
        // never reaches an app (no Accessibility permission needed for a failed read)
        assert_eq!(window_title(-1), None);
    }
}
