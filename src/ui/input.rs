//! Clicks, scrolls and hovers, passed on by the wing views: find what's under
//! the pointer (the hit regions of the last layout, in bar coordinates) and
//! hand the event to the widget registry, open the widget's own menu (a
//! script's `menu:` lines), or ask the person's BarPopover app for a hover menu.

use super::bar_menu;
use super::layout::{Hit, Region, Side, hit_at};
use super::menu::{self, Menu};
use super::view::BarView;
use crate::config::{self, MenuOn, Wing};
use crate::sched;
use crate::state::{self, Panel};
use crate::widgets::{self, Button, MenuLine, Scroll};
use block2::RcBlock;
use objc2::rc::Weak;
use objc2::{DefinedClass, MainThreadMarker};
use objc2_app_kit::{NSEvent, NSEventModifierFlags};
use objc2_foundation::{NSPoint, NSTimer};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Counts hover changes, so a hover menu still waiting can tell the pointer moved on.
static HOVER_GEN: AtomicU64 = AtomicU64::new(0);

/// How long the pointer rests on a widget before its hover menu opens.
const HOVER_MENU_DELAY: Duration = Duration::from_millis(300);

impl BarView {
    fn hit_at(&self, e: &NSEvent) -> Option<Region> {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        hit_at(&self.ivars().hits.borrow(), p.x).cloned()
    }

    pub(super) fn click(&self, e: &NSEvent, b: Button) {
        let region = self.hit_at(e);
        // the wingbar menu: a right-click, unless the widget has a right-click
        // action of its own; ⌥-right-click and ⌃-click open it whatever is there
        if b != Button::Middle && bar_menu::forced(b == Button::Right, e.modifierFlags()) {
            return self.open_bar_menu(e, region.as_ref());
        }
        if b == Button::Right {
            let ran = match &region {
                Some(Region { hit: Hit::Part(key, part), .. }) => widgets::current().click(key, b, Some(part)),
                Some(Region { hit: Hit::Widget(key), .. }) => widgets::current().click(key, b, None),
                None => false,
            };
            if !ran {
                self.open_bar_menu(e, region.as_ref());
            }
            return;
        }
        let Some(Region { x0, x1, hit }) = region else { return };
        let reg = widgets::current();
        match hit {
            Hit::Part(key, part) => {
                reg.click(&key, b, Some(&part));
            }
            Hit::Widget(key) => {
                if b == Button::Left
                    && let Some(lines) = reg.menu(&key, MenuOn::Click)
                {
                    drop(reg);
                    self.open_menu(&key, lines, x0);
                } else if !reg.click(&key, b, None) && b == Button::Left {
                    self.show_popover(&key, x0, x1, 0);
                }
            }
        }
    }

    /// The wingbar menu, under the widget clicked (or the pointer), for the
    /// wing it is in. Returns when the menu closes.
    fn open_bar_menu(&self, e: &NSEvent, region: Option<&Region>) {
        let cfg = config::current();
        let key = region.map(|r| r.hit.key().to_string());
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let wing = key.as_deref().and_then(|k| cfg.wing_of(k)).unwrap_or_else(|| {
            let (left_end, _) = self.ivars().wings.get();
            if p.x <= left_end { Wing::Left } else { Wing::Right }
        });
        let x = region.map_or(p.x, |r| r.x0);
        let h = self.ivars().m.get().h;
        bar_menu::pop_up(&cfg, key.as_deref(), wing, self, NSPoint::new(x, h));
    }

    /// Show a widget's menu under it. Returns when the menu closes.
    fn open_menu(&self, key: &str, lines: Vec<MenuLine>, x0: f64) {
        let h = self.ivars().m.get().h;
        menu::pop_up(script_menu(key, lines), self, NSPoint::new(x0, h));
    }

    /// Open the widget's hover menu once the pointer has rested on it for
    /// `HOVER_MENU_DELAY`, unless the hover `hgen` is over by then.
    ///
    /// A run-loop timer, not a main-queue block: the menu runs its own loop
    /// until it closes, and from inside a main-queue block that loop would hold
    /// up every other block on the queue (the bar's redraws).
    fn menu_after_rest(&self, key: &str, x0: f64, hgen: u64) {
        let (bar, key) = (Weak::from(self), key.to_string());
        let block = RcBlock::new(move |_: NonNull<NSTimer>| {
            if HOVER_GEN.load(Ordering::SeqCst) != hgen {
                return; // the pointer has moved on
            }
            // asked again: the menu may have changed, or gone, meanwhile
            if let (Some(bar), Some(lines)) = (bar.load(), widgets::current().menu(&key, MenuOn::Hover)) {
                bar.open_menu(&key, lines, x0);
            }
        });
        // SAFETY: a one-shot timer on this (the main) thread's run loop, so the
        // block, which holds main-thread-only objects, also runs on this thread.
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(HOVER_MENU_DELAY.as_secs_f64(), false, &block) };
    }

    pub(super) fn scroll(&self, e: &NSEvent) {
        let Some(dy) = scroll_dy(e.scrollingDeltaY(), e.isDirectionInvertedFromDevice()) else { return };
        let Some(Region { hit, .. }) = self.hit_at(e) else { return };
        let s = Scroll {
            dy,
            precise: e.hasPreciseScrollingDeltas(),
            fine: e.modifierFlags().contains(NSEventModifierFlags::Option),
        };
        widgets::current().scroll(hit.key(), s);
    }

    pub(super) fn hover(&self, e: &NSEvent) {
        let hit = self.hit_at(e);
        let key = match &hit {
            Some(Region { hit: Hit::Widget(k), .. }) => Some(k.clone()),
            _ => None,
        };
        if *self.ivars().hovered.borrow() == key {
            return;
        }
        let hgen = HOVER_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let old = self.ivars().hovered.replace(key.clone());
        self.redraw_hovered(old.as_deref(), key.as_deref());
        if let (Some(k), Some(r)) = (key, hit) {
            self.show_popover(&k, r.x0, r.x1, hgen);
            if widgets::current().menu(&k, MenuOn::Hover).is_some() {
                self.menu_after_rest(&k, r.x0, hgen);
            }
        }
    }

    pub(super) fn unhover(&self) {
        let old = self.ivars().hovered.borrow_mut().take();
        if old.is_some() {
            HOVER_GEN.fetch_add(1, Ordering::SeqCst);
            self.redraw_hovered(old.as_deref(), None);
        }
    }

    /// Draw again the wings of the widgets the pointer left and is now over
    /// (their hover highlight).
    fn redraw_hovered(&self, old: Option<&str>, new: Option<&str>) {
        let sides: Vec<Side> = {
            let lay = self.ivars().layout.borrow();
            let Some(lay) = lay.as_ref() else { return };
            [old, new].into_iter().flatten().filter_map(|k| lay.side_of(k)).collect()
        };
        for side in [Side::Left, Side::Right] {
            if sides.contains(&side) {
                self.redraw_wing(side);
            }
        }
    }

    /// Ask the person's native hover-menu app (BarPopover) to open under a widget.
    fn show_popover(&self, key: &str, x0: f64, x1: f64, hgen: u64) {
        if !widgets::current().popover(key) {
            return;
        }
        let (min_x, max_x) = popover_bounds(x0, x1, self.ivars().wings.get(), self.frame().size.width);
        let h = self.ivars().m.get().h;
        let nums = [x0, x1 - x0, min_x, max_x, h].map(|v| v.round().to_string());
        let args = std::iter::once(key.to_string()).chain(nums).collect();
        request_popover(Popover { args, hgen, path: None, screen: self.ivars().screen.get() });
    }
}

/// A script's menu lines as a menu: an item runs its command like a click
/// action, then refreshes the widget.
fn script_menu(key: &str, lines: Vec<MenuLine>) -> Menu {
    let mut m = Menu::new();
    for l in lines {
        match l {
            MenuLine::Separator => m.separator(),
            MenuLine::Info(title) => m.info(title),
            MenuLine::Action { title, command } => {
                let key = key.to_string();
                m.item(title, move || widgets::run_menu_command(&key, &command))
            }
        };
    }
    m
}

/// A check without the screen (`WINGBAR_DEBUG_MENUS` with `--snapshot`): print
/// every widget's menu as AppKit got it, and with `fire`, choose each menu's
/// first item that has an action, as a click on it would.
pub fn debug_menus(mtm: MainThreadMarker, fire: bool) {
    let reg = widgets::current();
    for key in reg.keys() {
        let Some((on, lines)) = reg.get(key).and_then(|w| w.menu()) else { continue };
        let built = menu::build(script_menu(key, lines), mtm);
        println!("menu {key} (opens on {on:?}):");
        let mut first = None;
        for i in 0..built.menu.numberOfItems() {
            let Some(it) = built.menu.itemAtIndex(i) else { continue };
            if it.isSeparatorItem() {
                println!("  ---");
                continue;
            }
            let action = it.action().map(|s| s.name().to_string_lossy().into_owned());
            println!("  {:?} enabled={} tag={} action={action:?}", it.title().to_string(), it.isEnabled(), it.tag());
            if first.is_none() && action.is_some() {
                first = Some(i);
            }
        }
        if fire && let Some(i) = first {
            println!("  firing item {i}");
            built.menu.performActionForItemAtIndex(i);
        }
    }
}

/// Fingers / wheel up = positive, whatever the natural-scrolling setting.
/// `None` for an event that doesn't move (momentum ends, phase changes), which
/// would otherwise count as a step up.
fn scroll_dy(delta: f64, inverted: bool) -> Option<f64> {
    (delta != 0.0).then(|| delta * if inverted { -1.0 } else { 1.0 })
}

/// How far a hover menu may stretch: within the wing the widget is in, or the
/// whole bar if it's in neither.
fn popover_bounds(x0: f64, x1: f64, (left_end, right_start): (f64, f64), width: f64) -> (f64, f64) {
    if x1 <= left_end + 1.0 {
        (0.0, left_end)
    } else if x0 >= right_start - 1.0 {
        (right_start, width)
    } else {
        (0.0, width)
    }
}

/// A hover-menu request: `key, x, width, min x, max x, bar height`, and the
/// hover it belongs to (0 = a click, shown at once).
struct Popover {
    args: Vec<String>,
    hgen: u64,
    /// Instead of showing a menu: this request for the daemon (no fallback).
    path: Option<String>,
    /// The screen of the bar asking (1-based).
    screen: usize,
}

/// The tab position last sent to BarPopover, and when.
static TAB_SENT: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// Sent again this often even unchanged: BarPopover forgets it when it restarts.
const TAB_RESEND: Duration = Duration::from_secs(60);

/// Tell BarPopover where the widget its tab hangs under is: `x` and `w` on
/// the main screen, the bar's height and the screen's width (as simple-bar's
/// hover script did). Sent when it changed, and once a minute while the bar
/// redraws (a request of a few bytes).
pub(super) fn report_popover_tab(x: f64, w: f64, h: f64, screen_w: f64) {
    let q = tab_query(x, w, h, screen_w);
    let mut sent = TAB_SENT.lock().unwrap_or_else(|e| e.into_inner());
    if sent.as_ref().is_some_and(|(last, at)| *last == q && at.elapsed() < TAB_RESEND) {
        return;
    }
    *sent = Some((q.clone(), Instant::now()));
    request_popover(Popover { args: Vec::new(), hgen: 0, path: Some(format!("/tab?{q}")), screen: 1 });
}

fn tab_query(x: f64, w: f64, h: f64, screen_w: f64) -> String {
    format!("x={}&w={}&h={}&sw={}", x.round(), w.round(), h.round(), screen_w.round())
}

/// Hand a request to the one popover thread (started on first use).
fn request_popover(p: Popover) {
    static TX: OnceLock<Sender<Popover>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("popover".into()).stack_size(128 * 1024).spawn(move || popovers(rx)).ok();
        tx
    });
    let _ = tx.send(p);
}

/// The popover thread. The latest request wins; a hover waits 150 ms first, to
/// ignore a pointer just passing over. After asking for a panel it follows the
/// panel (see `follow_panel`) until it closes, and then only waits.
fn popovers(rx: Receiver<Popover>) {
    let mut watch: Option<Watch> = None;
    loop {
        let next = match &watch {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(_) => rx.recv_timeout(PANEL_POLL),
        };
        let mut p = match next {
            Ok(p) => p,
            Err(RecvTimeoutError::Timeout) => {
                watch = watch.and_then(follow_panel);
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        loop {
            while let Ok(newer) = rx.try_recv() {
                p = newer;
            }
            if p.hgen == 0 {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(newer) => p = newer, // wait again, for the newer one
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        if p.hgen != 0 && HOVER_GEN.load(Ordering::SeqCst) != p.hgen {
            continue; // the pointer has moved on
        }
        let cfg = config::current();
        let path = p.path.clone().unwrap_or_else(|| format!("/show?a={}", p.args.join(",")));
        let sent = (|| -> std::io::Result<()> {
            let addr = cfg.bar.popover_daemon.parse().map_err(|_| std::io::Error::other("addr"))?;
            let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(150))?;
            s.set_read_timeout(Some(Duration::from_millis(500)))?;
            s.write_all(format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())?;
            let mut sink = [0u8; 256];
            let _ = s.read(&mut sink);
            Ok(())
        })();
        if sent.is_err() && p.path.is_some() {
            // not listening (yet): send it again on the next redraw
            *TAB_SENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let mut asked = sent.is_ok();
        if sent.is_err()
            && p.path.is_none()
            && let Some(bin) = &cfg.bar.popover_bin
        {
            let args: Vec<&str> = std::iter::once("show").chain(p.args.iter().map(String::as_str)).collect();
            sched::spawn_program(bin, &args);
            asked = true;
        }
        if asked && p.path.is_none() {
            watch = Some(Watch { screen: p.screen, since: Instant::now(), seen: false });
        }
    }
}

/// How often an open panel is checked on.
const PANEL_POLL: Duration = Duration::from_millis(100);

/// A panel asked of BarPopover, followed until it closes.
#[derive(Clone, Copy)]
struct Watch {
    screen: usize,
    since: Instant,
    /// The panel has been seen open.
    seen: bool,
}

/// Where BarPopover says which panel is open, and where: `pid kind left right`
/// (screen points) while one is, removed as it closes. Its temporary folder,
/// which for the same user is this one.
fn panel_file() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        objc2::rc::autoreleasepool(|_| PathBuf::from(objc2_foundation::NSTemporaryDirectory().to_string()))
            .join("barpopover.pid")
    })
}

/// One check on the panel being followed: tell the state where it is, or that
/// it closed. `None` stops following: the panel closed (or never came within
/// 3 s), its app is gone, or it has been followed for 10 minutes.
fn follow_panel(w: Watch) -> Option<Watch> {
    let text = std::fs::read_to_string(panel_file()).ok();
    let state = text.as_deref().map(parse_panel_file);
    let age = w.since.elapsed();
    let alive = |pid: i32| {
        // SAFETY: signal 0 only checks that the process exists; nothing is sent.
        pid > 0
            && (unsafe { libc::kill(pid, 0) } == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
    };
    let next = match state {
        Some(Some((pid, edges))) if alive(pid) && age < Duration::from_secs(600) => {
            if let Some((x0, x1)) = edges {
                state::set_panel(Some(Panel { screen: w.screen, x0, x1 }));
            }
            Some(Watch { seen: true, ..w })
        }
        // not up yet (its content may take a moment)
        None if !w.seen && age < Duration::from_secs(3) => Some(w),
        _ => None,
    };
    if next.is_none() {
        state::set_panel(None);
    }
    next
}

/// BarPopover's panel file: its pid, and the panel's left and right edges
/// once it is placed (`pid kind` before). `None` for anything else.
fn parse_panel_file(text: &str) -> Option<(i32, Option<(f64, f64)>)> {
    let mut f = text.split_whitespace();
    let pid = f.next()?.parse().ok()?;
    let _kind = f.next()?;
    let edges = match (f.next().map(str::parse::<f64>), f.next().map(str::parse::<f64>)) {
        (Some(Ok(x0)), Some(Ok(x1))) if x0 < x1 => Some((x0, x1)),
        _ => None,
    };
    Some((pid, edges))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_position_query_is_whole_points() {
        assert_eq!(tab_query(3083.4, 135.6, 42.0, 3360.0), "x=3083&w=136&h=42&sw=3360");
    }

    #[test]
    fn scroll_direction_and_zero_deltas() {
        assert_eq!(scroll_dy(3.0, false), Some(3.0));
        assert_eq!(scroll_dy(3.0, true), Some(-3.0), "natural scrolling: fingers up is still up");
        assert_eq!(scroll_dy(-0.5, false), Some(-0.5));
        assert_eq!(scroll_dy(0.0, false), None, "0.0.signum() is 1.0: would read as up");
        assert_eq!(scroll_dy(-0.0, true), None);
    }

    #[test]
    fn script_menu_lines_become_items() {
        use super::menu::Entry;
        let lines = vec![
            MenuLine::Info("Today: 2h".into()),
            MenuLine::Separator,
            MenuLine::Action { title: "Stop".into(), command: "true".into() },
        ];
        let m = script_menu("w", lines);
        let got: Vec<(String, bool, bool)> = m
            .entries
            .iter()
            .map(|e| match e {
                Entry::Item(it) => (it.title.clone(), it.enabled, it.action.is_some()),
                Entry::Separator => ("---".into(), false, false),
            })
            .collect();
        assert_eq!(
            got,
            [("Today: 2h".into(), false, false), ("---".into(), false, false), ("Stop".into(), true, true)]
        );
        assert!(script_menu("w", vec![]).is_empty());
    }

    #[test]
    fn panel_file_lines() {
        assert_eq!(parse_panel_file("123 ci 2419 2690\n"), Some((123, Some((2419.0, 2690.0)))));
        assert_eq!(parse_panel_file("123 ci"), Some((123, None)), "claimed, not placed yet");
        assert_eq!(parse_panel_file("123 ci 900 800"), Some((123, None)), "edges the wrong way round");
        assert_eq!(parse_panel_file(""), None);
        assert_eq!(parse_panel_file("abc ci 1 2"), None);
    }

    #[test]
    fn popovers_stay_in_their_wing() {
        let wings = (300.0, 1500.0);
        assert_eq!(popover_bounds(100.0, 200.0, wings, 2000.0), (0.0, 300.0));
        assert_eq!(popover_bounds(250.0, 301.0, wings, 2000.0), (0.0, 300.0), "1 pt of slack");
        assert_eq!(popover_bounds(1499.0, 1600.0, wings, 2000.0), (1500.0, 2000.0));
        assert_eq!(popover_bounds(800.0, 900.0, wings, 2000.0), (0.0, 2000.0), "in neither wing");
    }
}
