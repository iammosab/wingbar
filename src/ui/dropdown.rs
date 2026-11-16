//! A widget's menu drawn by wingbar (`theme.menus: "wings"`, the default): a
//! panel in the bar's colours hanging under the widget, joined to its wing.
//! Where it is flush with the wing's inner end the wing's corner squares
//! (through `state.panel`, as for BarPopover); elsewhere a small concave
//! fillet joins panel and wing. Rows: items (highlighted under the pointer),
//! readings (dimmed) and separators.
//!
//! Opened by a hover, it closes once the pointer has left the widget and the
//! panel for a moment. Opened by a click, it stays until a click elsewhere
//! (the widget again included) or a choice. Only while one is open does a
//! timer (100 ms) check on the pointer and a monitor watch for clicks elsewhere.
//! No keyboard: wingbar never becomes the active app, so keys stay with the
//! app in front (a macOS menu is the way to get them: `theme.menus: "native"`).
//!
//! The model (rows, sizes, placement, hit testing) is pure and tested; the
//! AppKit part only draws it and forwards events.

use super::draw::{attrs, font, measure, nscolor, rect};
use super::input::popover_bounds;
use super::layout::Metrics;
use super::menu::{Action, Entry, Menu};
use super::view::BarView;
use crate::config::{self, Rgba};
use crate::state::{self, Panel};
use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSEvent, NSEventMask, NSPanel, NSPopUpMenuWindowLevel, NSStringDrawing,
    NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSTimer};
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

// ── the model (pure) ────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// Something to choose.
    Item,
    /// A reading: shown dimmed, not chosen.
    Info,
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    pub title: String,
}

/// Sizes for a bar's metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dims {
    pub row_h: f64,
    pub sep_h: f64,
    /// Above the first row and below the last.
    pub pad_y: f64,
    /// Left and right of a row's text.
    pub pad_x: f64,
    pub min_w: f64,
    pub max_w: f64,
    /// The radius of the panel's bottom corners.
    pub corner: f64,
    /// The concave fillets joining panel and wing.
    pub fillet: f64,
}

impl Dims {
    pub fn for_metrics(m: &Metrics) -> Dims {
        Dims {
            row_h: (m.font * 2.0).round(),
            sep_h: 9.0,
            pad_y: 5.0,
            pad_x: m.pad + 5.0,
            min_w: 150.0,
            max_w: 420.0,
            corner: (m.corner * 0.8).round(),
            fillet: (m.corner * 0.6).round(),
        }
    }
}

/// The panel body's width and height, and each row's top and height.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub w: f64,
    pub h: f64,
    pub rows: Vec<(f64, f64)>,
}

/// Lay rows out: as wide as the widest title (between `min_w`, or the widget's
/// own width when wider, and `max_w`), rows stacked between the paddings.
pub fn geometry(rows: &[Row], text_w: &dyn Fn(&str) -> f64, d: &Dims, anchor_w: f64) -> Geometry {
    let widest = rows.iter().filter(|r| r.kind != RowKind::Separator).map(|r| text_w(&r.title)).fold(0.0, f64::max);
    let w = (widest + d.pad_x * 2.0).max(d.min_w).max(anchor_w).min(d.max_w).ceil();
    let mut y = d.pad_y;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let h = if r.kind == RowKind::Separator { d.sep_h } else { d.row_h };
        out.push((y, h));
        y += h;
    }
    Geometry { w, h: (y + d.pad_y).ceil(), rows: out }
}

/// Where the panel body's left edge goes: centred under the widget
/// (`anchor`, its left and right edge), kept inside `bounds` (the wing), or
/// inside the screen (`width`) when the wing is narrower than the panel.
pub fn place(anchor: (f64, f64), w: f64, bounds: (f64, f64), width: f64) -> f64 {
    let x = ((anchor.0 + anchor.1) / 2.0 - w / 2.0).round();
    let (lo, hi) = if bounds.1 - bounds.0 >= w { bounds } else { (8.0, width - 8.0) };
    // whole points, so the window sits on whole pixels
    x.min(hi - w).max(lo).round()
}

/// Which top corners get a fillet: a side flush with the end of the wing has
/// none (the wing's corner squares there, or it is the screen's edge).
pub fn fillets(x: f64, w: f64, bounds: (f64, f64)) -> (bool, bool) {
    (x > bounds.0 + 1.0, x + w < bounds.1 - 1.0)
}

/// The row at height `y` (panel coordinates, top down), if it can be chosen.
pub fn item_at(rows: &[Row], g: &Geometry, y: f64) -> Option<usize> {
    let i = g.rows.iter().position(|&(top, h)| y >= top && y < top + h)?;
    (rows[i].kind == RowKind::Item).then_some(i)
}

/// A menu's entries as rows and their actions (an item without one, or with
/// a submenu, which this menu doesn't show, is a reading).
fn rows_of(menu: Menu) -> (Vec<Row>, Vec<Option<Action>>) {
    let mut rows = Vec::new();
    let mut actions = Vec::new();
    for e in menu.entries {
        match e {
            Entry::Separator => {
                rows.push(Row { kind: RowKind::Separator, title: String::new() });
                actions.push(None);
            }
            Entry::Item(it) => {
                let act = it.action.filter(|_| it.enabled && it.submenu.is_none());
                let kind = if act.is_some() { RowKind::Item } else { RowKind::Info };
                rows.push(Row { kind, title: it.title });
                actions.push(act);
            }
        }
    }
    (rows, actions)
}

// ── AppKit ──────────────────────────────────────────────────────────────────

/// How long the pointer may be away before a hover-opened menu closes.
const LEAVE_GRACE: Duration = Duration::from_millis(250);

pub struct ViewIvars {
    rows: Vec<Row>,
    actions: RefCell<Vec<Option<Action>>>,
    g: Geometry,
    d: Dims,
    m: Metrics,
    /// Fillets at the top left and right.
    fillets: (bool, bool),
    hovered: Cell<Option<usize>>,
}

define_class!(
    /// The menu's content: draws the panel and its rows.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "WingbarDropdownView"]
    #[ivars = ViewIvars]
    struct DropdownView;

    impl DropdownView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            false
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _e: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            self.hover_at(e);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, e: &NSEvent) {
            self.hover_at(e);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _e: &NSEvent) {
            self.set_hovered(None);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _e: &NSEvent) {}

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, e: &NSEvent) {
            if let Some(i) = self.row_under(e) {
                self.choose(i);
            }
        }

    }
);

impl DropdownView {
    fn row_under(&self, e: &NSEvent) -> Option<usize> {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let iv = self.ivars();
        let x0 = if iv.fillets.0 { iv.d.fillet } else { 0.0 };
        if p.x < x0 || p.x > x0 + iv.g.w {
            return None;
        }
        item_at(&iv.rows, &iv.g, p.y)
    }

    fn hover_at(&self, e: &NSEvent) {
        self.set_hovered(self.row_under(e));
    }

    fn set_hovered(&self, i: Option<usize>) {
        if self.ivars().hovered.replace(i) != i {
            self.setNeedsDisplay(true);
        }
    }

    /// Close, then run the item's action.
    fn choose(&self, i: usize) {
        let act = self.ivars().actions.borrow_mut().get_mut(i).and_then(Option::take);
        close();
        if let Some(f) = act {
            f();
        }
    }

    fn draw(&self) {
        let cfg = config::current();
        let t = &cfg.theme;
        let iv = self.ivars();
        let (d, g) = (&iv.d, &iv.g);
        let f = d.fillet;
        let bx = if iv.fillets.0 { f } else { 0.0 };

        nscolor(t.wing).setFill();
        // the body: square top (against the wing), rounded bottom
        let r = d.corner;
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(bx, -r, g.w, g.h + r), r, r).fill();
        // concave fillets where the body meets the wing's lower edge
        let k = 0.5523 * f;
        if iv.fillets.0 {
            let p = NSBezierPath::bezierPath();
            p.moveToPoint(NSPoint::new(0.0, 0.0));
            p.lineToPoint(NSPoint::new(f, 0.0));
            p.lineToPoint(NSPoint::new(f, f));
            p.curveToPoint_controlPoint1_controlPoint2(
                NSPoint::new(0.0, 0.0),
                NSPoint::new(f, f - k),
                NSPoint::new(k, 0.0),
            );
            p.closePath();
            p.fill();
        }
        if iv.fillets.1 {
            // from the body's edge, not the window's width (AppKit may round that up)
            let w = bx + g.w + f;
            let p = NSBezierPath::bezierPath();
            p.moveToPoint(NSPoint::new(w, 0.0));
            p.lineToPoint(NSPoint::new(w - f, 0.0));
            p.lineToPoint(NSPoint::new(w - f, f));
            p.curveToPoint_controlPoint1_controlPoint2(
                NSPoint::new(w, 0.0),
                NSPoint::new(w - f, f - k),
                NSPoint::new(w - k, 0.0),
            );
            p.closePath();
            p.fill();
        }

        let fnt = font(iv.m.font, false);
        let th = measure("0", &fnt).height;
        for (i, (row, &(y, h))) in iv.rows.iter().zip(&g.rows).enumerate() {
            match row.kind {
                RowKind::Separator => {
                    let c = t.faint;
                    nscolor(Rgba(c.0, c.1, c.2, c.3 * 0.45)).setFill();
                    NSBezierPath::fillRect(rect(bx + d.pad_x, (y + h / 2.0).floor(), g.w - d.pad_x * 2.0, 1.0));
                }
                RowKind::Item | RowKind::Info => {
                    let item = row.kind == RowKind::Item;
                    if item && iv.hovered.get() == Some(i) {
                        nscolor(t.wing_hover).setFill();
                        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                            rect(bx + 5.0, y + 1.0, g.w - 10.0, h - 2.0),
                            6.0,
                            6.0,
                        )
                        .fill();
                    }
                    let color = nscolor(if item { t.text } else { t.dim });
                    let a = attrs(&fnt, &color, true);
                    let tr = rect(bx + d.pad_x, y + ((h - th) / 2.0).round(), g.w - d.pad_x * 2.0, th);
                    // SAFETY: inside drawRect:; `a` holds string-drawing attributes of the right types.
                    unsafe { NSString::from_str(&row.title).drawInRect_withAttributes(tr, Some(&a)) };
                }
            }
        }
    }
}

/// The menu open now.
struct Open {
    panel: Retained<NSPanel>,
    /// Kept with the panel (its content view).
    _view: Retained<DropdownView>,
    key: String,
    bar: Weak<BarView>,
    /// The widget, in screen coordinates.
    anchor: NSRect,
    /// Opened by a click: stays until a click elsewhere or a choice.
    pinned: bool,
    timer: Retained<NSTimer>,
    monitor: Option<Retained<AnyObject>>,
    outside_since: Cell<Option<Instant>>,
}

thread_local! {
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    /// The menu closed last: kept until the next one opens, since it may close
    /// from inside one of its own event handlers.
    static CLOSED: RefCell<Option<Open>> = const { RefCell::new(None) };
}

/// Whether the menu open now belongs to `key` (its widget stays highlighted).
pub fn is_open_for(key: &str) -> bool {
    OPEN.with(|o| o.borrow().as_ref().is_some_and(|o| o.key == key))
}

/// Whether a menu opened by a click is open for `key` (a second click closes it).
pub(super) fn pinned_for(key: &str) -> bool {
    OPEN.with(|o| o.borrow().as_ref().is_some_and(|o| o.key == key && o.pinned))
}

/// Show `menu` for the widget `key` (spanning `x0..x1` of `bar`). An empty
/// menu isn't shown.
pub(super) fn show(bar: &BarView, key: &str, menu: Menu, x0: f64, x1: f64, pinned: bool) {
    close();
    let (rows, actions) = rows_of(menu);
    if rows.iter().all(|r| r.kind == RowKind::Separator) {
        return;
    }
    let mtm = bar.mtm();
    let Some(win) = bar.window() else { return };
    let iv = bar.ivars();
    let m = iv.m.get();
    let d = Dims::for_metrics(&m);
    let fnt = font(m.font, false);
    let g = geometry(&rows, &|s| measure(s, &fnt).width, &d, x1 - x0);
    let width = bar.frame().size.width;
    let bounds = popover_bounds(x0, x1, iv.wings.get(), width);
    let x = place((x0, x1), g.w, bounds, width);
    let fl = fillets(x, g.w, bounds);
    let left_pad = if fl.0 { d.fillet } else { 0.0 };
    let total_w = g.w + left_pad + if fl.1 { d.fillet } else { 0.0 };

    let wf = win.frame();
    let frame = NSRect::new(NSPoint::new(wf.origin.x + x - left_pad, wf.origin.y - g.h), NSSize::new(total_w, g.h));
    let anchor = NSRect::new(NSPoint::new(wf.origin.x + x0, wf.origin.y), NSSize::new(x1 - x0, m.h));

    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: NSPanel's designated initialiser with arguments of the declared
    // types, called once on a fresh allocation.
    let panel: Retained<NSPanel> = unsafe {
        msg_send![NSPanel::alloc(mtm), initWithContentRect: frame, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    // SAFETY: we keep our own `Retained` to the panel, so AppKit must not also
    // release it on close.
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    // no shadow: its rim would outline the join with the wing, which has none either
    panel.setHasShadow(false);
    panel.setLevel(NSPopUpMenuWindowLevel);
    panel.setHidesOnDeactivate(false);
    panel.setAcceptsMouseMovedEvents(true);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Transient
            | NSWindowCollectionBehavior::IgnoresCycle,
    );

    let body = (x, x + g.w);
    let ivars = ViewIvars { rows, actions: RefCell::new(actions), g, d, m, fillets: fl, hovered: Cell::new(None) };
    let this = DropdownView::alloc(mtm).set_ivars(ivars);
    // SAFETY: NSView's designated initialiser, called once on a freshly allocated view.
    let view: Retained<DropdownView> =
        unsafe { msg_send![super(this), initWithFrame: rect(0.0, 0.0, total_w, frame.size.height)] };
    let opts = NSTrackingAreaOptions::MouseEnteredAndExited
        | NSTrackingAreaOptions::MouseMoved
        | NSTrackingAreaOptions::ActiveAlways
        | NSTrackingAreaOptions::InVisibleRect;
    // SAFETY: a fresh allocation, initialised once; the owner is the view the area
    // is added to (AppKit doesn't retain it, and the view outlives its own areas).
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            NSRect::ZERO,
            opts,
            Some(&view as &AnyObject),
            None,
        )
    };
    view.addTrackingArea(&area);
    panel.setContentView(Some(&view));
    panel.orderFrontRegardless();

    let tick = RcBlock::new(|_: NonNull<NSTimer>| tick());
    // SAFETY: a repeating timer on this (the main) thread's run loop, so the block,
    // which touches main-thread-only state, also runs on this thread.
    let timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.1, true, &tick) };
    // a click in another app closes it (clicks on the bar close it in `input`)
    // (global monitors call back on the main thread)
    let outside = RcBlock::new(|_: NonNull<NSEvent>| close());
    let monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown,
        &outside,
    );

    state::set_panel(Some(Panel { screen: iv.screen.get(), x0: body.0, x1: body.1 }));
    OPEN.with(|o| {
        *o.borrow_mut() = Some(Open {
            panel,
            _view: view,
            key: key.to_string(),
            bar: Weak::from(bar),
            anchor,
            pinned,
            timer,
            monitor,
            outside_since: Cell::new(None),
        })
    });
    bar.redraw_hovered(None, Some(key));
}

/// Close the menu open now, if any.
pub fn close() {
    let Some(open) = OPEN.with(|o| o.borrow_mut().take()) else { return };
    open.timer.invalidate();
    if let Some(m) = &open.monitor {
        // SAFETY: `m` was returned by addGlobalMonitorForEventsMatchingMask and is removed once.
        unsafe { NSEvent::removeMonitor(m) };
    }
    open.panel.orderOut(None);
    state::set_panel(None);
    if let Some(bar) = open.bar.load() {
        bar.redraw_hovered(Some(&open.key), None);
    }
    CLOSED.with(|c| *c.borrow_mut() = Some(open));
}

/// Every 100 ms while a menu opened by a hover is open: close it once the
/// pointer has been away from the widget and the menu for `LEAVE_GRACE`.
fn tick() {
    let leave = OPEN.with(|o| {
        let o = o.borrow();
        let Some(o) = o.as_ref() else { return false };
        if o.pinned {
            return false;
        }
        let p = NSEvent::mouseLocation();
        let inside = |r: NSRect, slack: f64| {
            p.x >= r.origin.x - slack
                && p.x <= r.origin.x + r.size.width + slack
                && p.y >= r.origin.y - slack
                && p.y <= r.origin.y + r.size.height + slack
        };
        if inside(o.panel.frame(), 4.0) || inside(o.anchor, 2.0) {
            o.outside_since.set(None);
            return false;
        }
        let since = o.outside_since.get().unwrap_or_else(|| {
            let now = Instant::now();
            o.outside_since.set(Some(now));
            now
        });
        since.elapsed() >= LEAVE_GRACE
    });
    if leave {
        close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: RowKind, t: &str) -> Row {
        Row { kind, title: t.into() }
    }
    const D: Dims = Dims {
        row_h: 28.0,
        sep_h: 9.0,
        pad_y: 5.0,
        pad_x: 16.0,
        min_w: 150.0,
        max_w: 420.0,
        corner: 14.0,
        fillet: 11.0,
    };

    fn sample() -> Vec<Row> {
        vec![
            row(RowKind::Info, "Wi-Fi (en0): 192.168.1.20"),
            row(RowKind::Separator, ""),
            row(RowKind::Item, "Open Wi-Fi Settings"),
            row(RowKind::Item, "Open Network Settings"),
        ]
    }

    #[test]
    fn rows_stack_and_the_panel_is_as_wide_as_its_widest_title() {
        let w = |s: &str| s.len() as f64 * 7.0;
        let g = geometry(&sample(), &w, &D, 80.0);
        assert_eq!(g.rows, vec![(5.0, 28.0), (33.0, 9.0), (42.0, 28.0), (70.0, 28.0)]);
        assert_eq!(g.h, 5.0 + 28.0 + 9.0 + 28.0 + 28.0 + 5.0);
        assert_eq!(g.w, 25.0 * 7.0 + 32.0, "the widest title and its padding");
        assert_eq!(geometry(&[row(RowKind::Item, "Hi")], &w, &D, 80.0).w, 150.0, "at least min_w");
        assert_eq!(geometry(&[row(RowKind::Item, "Hi")], &w, &D, 200.0).w, 200.0, "at least the widget");
        assert_eq!(geometry(&[row(RowKind::Item, &"x".repeat(200))], &w, &D, 80.0).w, 420.0, "at most max_w");
    }

    #[test]
    fn the_panel_is_centred_under_its_widget_and_stays_in_the_wing() {
        // right wing 1500..2000 of a 2000 pt screen
        let wing = (1500.0, 2000.0);
        assert_eq!(place((1700.0, 1760.0), 200.0, wing, 2000.0), 1630.0, "centred");
        assert_eq!(place((1505.0, 1560.0), 200.0, wing, 2000.0), 1500.0, "the wing's first widget: flush");
        assert_eq!(place((1900.0, 1990.0), 200.0, wing, 2000.0), 1800.0, "the last: flush with the screen edge");
        assert_eq!(place((1505.0, 1560.0), 600.0, wing, 2000.0), 1233.0, "wider than the wing: kept on screen");
        assert_eq!(place((2350.0, 2420.0), 200.0, (2344.6, 3360.0), 3360.0), 2345.0, "a fractional wing: whole points");
        assert_eq!(fillets(1630.0, 200.0, wing), (true, true));
        assert_eq!(fillets(1500.0, 200.0, wing), (false, true), "flush with the wing's start");
        assert_eq!(fillets(1800.0, 200.0, wing), (true, false), "flush with its end");
    }

    #[test]
    fn only_items_can_be_hit() {
        let rows = sample();
        let g = geometry(&rows, &|s| s.len() as f64 * 7.0, &D, 80.0);
        assert_eq!(item_at(&rows, &g, 10.0), None, "a reading");
        assert_eq!(item_at(&rows, &g, 36.0), None, "the separator");
        assert_eq!(item_at(&rows, &g, 50.0), Some(2));
        assert_eq!(item_at(&rows, &g, 97.0), Some(3));
        assert_eq!(item_at(&rows, &g, 99.0), None, "the bottom padding");
    }

    #[test]
    fn a_menu_becomes_rows_with_their_actions() {
        let mut sub = Menu::new();
        sub.item("deeper", || {});
        let mut m = Menu::new();
        m.info("Reading").separator().item("Do it", || {}).submenu("More", sub);
        let (rows, actions) = rows_of(m);
        assert_eq!(
            rows,
            vec![
                row(RowKind::Info, "Reading"),
                row(RowKind::Separator, ""),
                row(RowKind::Item, "Do it"),
                row(RowKind::Info, "More"),
            ]
        );
        assert_eq!(actions.iter().map(Option::is_some).collect::<Vec<_>>(), [false, false, true, false]);
    }
}
