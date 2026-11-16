//! The bar: one borderless, non-activating, transparent panel per screen, as
//! wide as the screen. Its content view draws nothing; two child views, each
//! only as wide as its wing, draw everything (no web view, no layout engine).
//! A change in the state lays the bar out again, and only a wing that looks
//! different is drawn again, so a redraw uploads a wing, not the screen width.
//!
//! - `layout`: where every segment goes, each wing's span, what's under a point (pure, tested)
//! - `draw`: laying out and painting with AppKit (colour, font and icon caches)
//! - `view`: the `BarView` (content view) and `WingView` classes, forwarding events
//! - `input`: clicks, scrolls and hovers to the widget registry; hover menus
//! - `bar_menu`: the wingbar menu (right-click): manage widgets
//! - `observers`: the NSWorkspace / screen / wake / now-playing notifications

mod bar_menu;
mod draw;
mod input;
mod layout;
pub mod menu;
mod observers;
mod view;

pub use bar_menu::debug as debug_bar_menus;
pub use input::debug_menus;
pub use observers::install_observers;

use crate::config;
use draw::rect;
use layout::Metrics;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSBitmapImageFileType, NSColor, NSPanel, NSScreen, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};
use std::cell::{Cell, RefCell};
use view::BarView;

struct Bar {
    window: Retained<NSPanel>,
    view: Retained<BarView>,
}

thread_local! {
    static BARS: RefCell<Vec<Bar>> = const { RefCell::new(Vec::new()) };
    static BARS_COUNT: Cell<usize> = const { Cell::new(1) };
}

/// How many screens (and so bars) there are.
fn bar_count() -> usize {
    BARS_COUNT.with(|c| c.get())
}

/// Lay every bar out again; only the wings that changed are drawn again.
pub fn redraw_all() {
    BARS.with(|b| {
        for bar in b.borrow().iter() {
            bar.view.update();
        }
    });
}

fn metrics_for(screen: &NSScreen) -> Metrics {
    let f = screen.frame();
    let sh = f.size.height;
    let (mut h, mut font, mut icon, pad, corner) = if sh >= 1600.0 {
        (42.0, 14.0, 14.0, 11.0, 18.0)
    } else if sh >= 1200.0 {
        (38.0, 13.0, 13.0, 10.0, 16.0)
    } else {
        (34.0, 12.5, 12.0, 9.0, 14.0)
    };
    let top = screen.safeAreaInsets().top;
    let notch = if top > 0.0 {
        h = top.max(24.0);
        let l = screen.auxiliaryTopLeftArea();
        let r = screen.auxiliaryTopRightArea();
        Some((l.size.width, r.origin.x - f.origin.x))
    } else {
        None
    };
    if let Some(fs) = config::current().bar.font_size {
        font = fs;
        icon = fs;
    }
    Metrics { h, font, icon, pad, corner, notch }
}

/// The bars' window level, from the settings in use.
fn window_level() -> isize {
    if config::current().bar.level_above_windows {
        21 // above app windows and the Dock, below the menu bar when it slides in
    } else {
        // just above the desktop icons
        crate::ffi::cg::window_level_for_key(crate::ffi::cg::DESKTOP_ICON_WINDOW_LEVEL_KEY) as isize + 1
    }
}

/// The settings were reloaded: take up their window level, font size, colours
/// and wing lists, and lay every bar out and draw every wing again. The
/// windows stay (nothing blinks); if the screens changed meanwhile, the bars
/// are made again.
pub fn apply_settings(mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm);
    if screens.count() != BARS.with(|b| b.borrow().len()) {
        build_bars(mtm);
        return;
    }
    let level = window_level();
    let rebuild = BARS.with(|b| {
        for (bar, screen) in b.borrow().iter().zip(screens.iter()) {
            let m = metrics_for(&screen);
            let iv = bar.view.ivars();
            if m.h != iv.m.get().h {
                return true; // the window's size changes too
            }
            if bar.window.level() != level {
                bar.window.setLevel(level);
            }
            iv.m.set(m);
            // no layout to compare with: every wing is drawn again (the colours may have changed)
            *iv.layout.borrow_mut() = None;
            bar.view.update();
        }
        false
    });
    if rebuild {
        build_bars(mtm);
    }
}

/// (Re)create one bar per screen.
pub fn build_bars(mtm: MainThreadMarker) {
    BARS.with(|b| {
        for bar in b.borrow_mut().drain(..) {
            bar.window.orderOut(None);
            bar.window.close();
        }
    });
    let screens = NSScreen::screens(mtm);
    BARS_COUNT.with(|c| c.set(screens.count()));
    let level = window_level();
    for (i, screen) in screens.iter().enumerate() {
        let f = screen.frame();
        let m = metrics_for(&screen);
        let win_h = m.h;
        let frame =
            NSRect::new(NSPoint::new(f.origin.x, f.origin.y + f.size.height - win_h), NSSize::new(f.size.width, win_h));
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        // SAFETY: NSPanel's designated initialiser with arguments of the declared
        // types, called once on a fresh allocation.
        let window: Retained<NSPanel> = unsafe {
            msg_send![NSPanel::alloc(mtm), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
        };
        // SAFETY: we keep our own `Retained` to the window, so AppKit must not
        // also release it on close (that would be a double release).
        unsafe { window.setReleasedWhenClosed(false) };
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(false);
        window.setLevel(level);
        window.setHidesOnDeactivate(false);
        window.setFloatingPanel(false);
        window.setBecomesKeyOnlyIfNeeded(true);
        window.setAcceptsMouseMovedEvents(true);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let view = BarView::new(mtm, rect(0.0, 0.0, f.size.width, win_h), i + 1, m);
        window.setContentView(Some(&view));
        view.update();
        window.orderFrontRegardless();
        BARS.with(|b| b.borrow_mut().push(Bar { window, view }));
    }
}

/// Save a picture of each bar as drawn into `dir` (created if needed), for
/// checking the look without screen recording. Problems go to stderr; returns
/// how many pictures were saved.
pub fn snapshot(dir: &str) -> usize {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("wingbar: can't create {dir}: {e}");
        return 0;
    }
    BARS.with(|b| {
        let mut saved = 0;
        for (i, bar) in b.borrow().iter().enumerate() {
            let p = format!("{dir}/bar-{}.png", i + 1);
            let v = &bar.view;
            // from the state as it is now; the full width, wings drawn by their views
            v.update();
            let r = v.bounds();
            let Some(rep) = v.bitmapImageRepForCachingDisplayInRect(r) else {
                eprintln!("wingbar: can't make a bitmap for {p}");
                continue;
            };
            v.cacheDisplayInRect_toBitmapImageRep(r, &rep);
            // SAFETY: PNG with no properties is a valid file type / properties pair.
            let png =
                unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) };
            let Some(data) = png else {
                eprintln!("wingbar: can't encode {p} as PNG");
                continue;
            };
            match std::fs::write(&p, data.to_vec()) {
                Ok(()) => {
                    println!("saved {p}");
                    saved += 1;
                }
                Err(e) => eprintln!("wingbar: can't write {p}: {e}"),
            }
        }
        saved
    })
}

/// Write where the workspace numbers and the left wing sit (`Layout::geom`)
/// for the person's `keys.sh` cheat sheet (the same file the Übersicht hover
/// widget wrote), when it changed.
fn write_geom(g: String) {
    thread_local! { static LAST: RefCell<String> = const { RefCell::new(String::new()) }; }
    LAST.with(|l| {
        if *l.borrow() != g {
            let _ = std::fs::write(format!("{}simple-bar-geom", crate::sys::user_temp_dir()), format!("{g}\n"));
            *l.borrow_mut() = g;
        }
    });
}
