//! A widget's menu drawn by wingbar (`theme.menus: "wings"`, the default): a
//! panel in the bar's colours hanging under the widget, joined to its wing.
//! Where it is flush with the wing's inner end the wing's corner squares
//! (through `state.panel`, as for BarPopover); elsewhere a small concave
//! fillet joins panel and wing. Its rows are read from the menu's lines by
//! `rich`: items (highlighted under the pointer), readings, headers, sections,
//! label/value columns, meters, captions, a row of buttons, separators.
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
use super::menu::{Action, Menu};
use super::rich::{self, Dims, Face, Geometry, Hit, Kind, Row};
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

// ── placement (pure) ─────────────────────────────────────────────────────────
// The rows themselves (what each line looks like, sizes, hit testing) are in `rich`.

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

// ── AppKit ──────────────────────────────────────────────────────────────────

/// How long the pointer may be away before a hover-opened menu closes.
const LEAVE_GRACE: Duration = Duration::from_millis(250);

pub struct ViewIvars {
    rows: Vec<Row>,
    /// By slot (see `rich::rows_of`).
    actions: RefCell<Vec<Option<Action>>>,
    g: Geometry,
    d: Dims,
    m: Metrics,
    /// Fillets at the top left and right.
    fillets: (bool, bool),
    hovered: Cell<Option<Hit>>,
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
            if let Some(h) = self.hit_under(e) {
                self.choose(h);
            }
        }

    }
);

impl DropdownView {
    fn hit_under(&self, e: &NSEvent) -> Option<Hit> {
        let p = self.convertPoint_fromView(e.locationInWindow(), None);
        let iv = self.ivars();
        let x0 = if iv.fillets.0 { iv.d.fillet } else { 0.0 };
        rich::hit(&iv.rows, &iv.g, p.x - x0, p.y)
    }

    fn hover_at(&self, e: &NSEvent) {
        self.set_hovered(self.hit_under(e));
    }

    fn set_hovered(&self, h: Option<Hit>) {
        if self.ivars().hovered.replace(h) != h {
            self.setNeedsDisplay(true);
        }
    }

    /// Close, then run the chosen action.
    fn choose(&self, h: Hit) {
        let iv = self.ivars();
        let act = rich::slot_of(&iv.rows, h).and_then(|i| iv.actions.borrow_mut().get_mut(i).and_then(Option::take));
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

        let hovered = iv.hovered.get();
        let faint = |a: f64| nscolor(Rgba(t.faint.0, t.faint.1, t.faint.2, t.faint.3 * a));
        let highlight = |x: f64, y: f64, w: f64, h: f64| {
            nscolor(t.wing_hover).setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(x, y, w, h), 6.0, 6.0).fill();
        };
        let (left, right) = (bx + d.pad_x, bx + g.w - d.pad_x);
        for (i, (row, &(y, h))) in iv.rows.iter().zip(&g.rows).enumerate() {
            match &row.kind {
                Kind::Separator => {
                    faint(0.45).setFill();
                    NSBezierPath::fillRect(rect(left, (y + h / 2.0).floor(), right - left, 1.0));
                }
                Kind::Meter(pct, color) => {
                    let c = color.as_deref().and_then(|c| t.color(c)).unwrap_or(t.text);
                    let (my, mh) = ((y + 1.0).round(), 3.0);
                    faint(0.3).setFill();
                    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(left, my, right - left, mh), 1.5, 1.5)
                        .fill();
                    nscolor(c).setFill();
                    let fw = ((right - left) * pct / 100.0).max(if *pct > 0.0 { mh } else { 0.0 });
                    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(left, my, fw, mh), 1.5, 1.5).fill();
                }
                Kind::Buttons(bs) => {
                    let bw = g.w / bs.len() as f64;
                    for (j, b) in bs.iter().enumerate() {
                        let x = bx + bw * j as f64;
                        if hovered == Some(Hit::Button(i, j)) {
                            highlight(x + 4.0, y + 3.0, bw - 8.0, h - 6.0);
                        }
                        if j > 0 {
                            faint(0.45).setFill();
                            NSBezierPath::fillRect(rect(x.floor(), y + h * 0.28, 1.0, h * 0.44));
                        }
                        let c = if b.slot.is_some() { t.text } else { t.dim };
                        let f = self.face(Face::Bold);
                        let tw = measure(&b.title, &f).width.min(bw - 8.0);
                        self.text(&b.title, Face::Bold, c, x + ((bw - tw) / 2.0).max(4.0), y, tw, h);
                    }
                }
                k => {
                    let clickable = row.slot.is_some();
                    if clickable && hovered == Some(Hit::Row(i)) {
                        highlight(bx + 5.0, y + 1.0, g.w - 10.0, h - 2.0);
                    }
                    let label_c = match k {
                        Kind::Info | Kind::Caption => t.dim,
                        _ => t.text,
                    };
                    let mut r = right;
                    if *k == Kind::Item && row.value.is_some() {
                        let cw = d.chevron;
                        self.text("›", Face::Regular, t.dim, r - cw + 4.0, y, cw, h);
                        r -= cw;
                    }
                    let mut vw = 0.0;
                    if let Some(v) = &row.value {
                        let vf = rich::value_face(k);
                        let value_c = if *k == Kind::Header { t.dim } else { t.text };
                        // the value gets what the label leaves; both shorten only when they must
                        let room = r - left - d.gap * 0.5;
                        let lw = measure(&row.title, &self.face(rich::title_face(k))).width;
                        vw = measure(v, &self.face(vf)).width.min((room - lw).max(room * 0.45));
                        self.text(v, vf, value_c, r - vw, y, vw, h);
                        vw += d.gap * 0.5;
                    }
                    self.text(&row.title, rich::title_face(k), label_c, left, y, (r - left - vw).max(0.0), h);
                }
            }
        }
    }

    /// A font for `face` at this bar's size.
    fn face(&self, f: Face) -> Retained<objc2_app_kit::NSFont> {
        let size = self.ivars().m.font;
        match f {
            Face::Regular => font(size, false),
            Face::Bold => font(size, true),
            Face::Small => font((size * 0.86).round(), false),
        }
    }

    /// Draw `s` in `face` and `color`, in the box at `x` of width `w`, centred
    /// in the row from `y` of height `h`; cut short with … when it doesn't fit.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, s: &str, face: Face, color: Rgba, x: f64, y: f64, w: f64, h: f64) {
        let f = self.face(face);
        let th = measure("0", &f).height;
        let a = attrs(&f, &nscolor(color), true);
        // SAFETY: inside drawRect:; `a` holds string-drawing attributes of the right types.
        unsafe {
            NSString::from_str(s).drawInRect_withAttributes(rect(x, y + ((h - th) / 2.0).round(), w, th), Some(&a))
        };
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
    let (rows, actions) = rich::rows_of(menu);
    if rows.iter().all(|r| r.kind == Kind::Separator) {
        return;
    }
    let mtm = bar.mtm();
    let Some(win) = bar.window() else { return };
    let iv = bar.ivars();
    let m = iv.m.get();
    let d = Dims::new(m.font, m.pad, m.corner);
    let fonts = [font(m.font, false), font(m.font, true), font((m.font * 0.86).round(), false)];
    let text_w = |s: &str, f: Face| measure(s, &fonts[f as usize]).width;
    let g: Geometry = rich::geometry(&rows, &text_w, &d, x1 - x0);
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
}
