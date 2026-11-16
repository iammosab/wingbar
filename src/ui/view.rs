//! The views each bar has. The panel spans the whole screen width, but most of
//! it is empty, so its content view (`BarView`) draws nothing: it holds the
//! bar's layout and state, and one `WingView` per wing, each only as wide as
//! its wing, draws that wing. Only the wings have a backing store, so a redraw
//! uploads a wing's worth of pixels, not the screen's width.
//!
//! Everything is in bar coordinates: a wing view's bounds start where its
//! frame does, so drawing and hit testing need no offsets. Mouse events in a
//! wing go to `input` on the bar.

use super::layout::{Layout, Metrics, Region, Side};
use crate::widgets::Button;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSTrackingArea, NSTrackingAreaOptions, NSView};
use objc2_foundation::{NSPoint, NSRect};
use std::cell::{Cell, RefCell};

pub struct Ivars {
    /// This bar's screen, 1-based.
    pub(super) screen: Cell<usize>,
    pub(super) m: Cell<Metrics>,
    /// The last layout: what the wings draw, and what to compare the next one with.
    pub(super) layout: RefCell<Option<Layout>>,
    /// What can be clicked, as of the last layout.
    pub(super) hits: RefCell<Vec<Region>>,
    /// The key of the widget under the pointer.
    pub(super) hovered: RefCell<Option<String>>,
    /// (end of the left wing, start of the right wing), as of the last layout.
    pub(super) wings: Cell<(f64, f64)>,
    /// The left and the right wing's views.
    pub(super) wing_views: RefCell<Vec<Retained<WingView>>>,
}

define_class!(
    /// The bar's content view: the full width of the screen, drawing nothing.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "WingbarView"]
    #[ivars = Ivars]
    pub struct BarView;

    impl BarView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            false
        }
    }
);

pub struct WingIvars {
    side: Side,
    /// The bar it belongs to (its superview).
    bar: Weak<BarView>,
}

define_class!(
    /// One wing: only as wide as the wing, drawing its shape and segments.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "WingbarWingView"]
    #[ivars = WingIvars]
    pub struct WingView;

    impl WingView {
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
            if let Some(bar) = self.bar() {
                bar.draw_wing(self.ivars().side);
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, e: &NSEvent) {
            if let Some(bar) = self.bar() {
                bar.click(e, Button::Left);
            }
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, e: &NSEvent) {
            if let Some(bar) = self.bar() {
                bar.click(e, Button::Right);
            }
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, e: &NSEvent) {
            if let Some(bar) = self.bar() {
                bar.click(e, Button::Middle);
            }
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, e: &NSEvent) {
            if let Some(bar) = self.bar() {
                bar.scroll(e);
            }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, e: &NSEvent) {
            if let Some(bar) = self.bar() {
                bar.hover(e);
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, e: &NSEvent) {
            if let Some(bar) = self.bar() {
                bar.hover(e);
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _e: &NSEvent) {
            if let Some(bar) = self.bar() {
                bar.unhover();
            }
        }
    }
);

impl WingView {
    fn new(mtm: MainThreadMarker, bar: &BarView, side: Side) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WingIvars { side, bar: Weak::from(bar) });
        let h = bar.ivars().m.get().h;
        // SAFETY: NSView's designated initialiser, called once on a freshly allocated view.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: super::rect(0.0, 0.0, 0.0, h)] };
        view.setHidden(true);
        // follows the view's frame as the wing grows and shrinks
        let opts = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        // SAFETY: a fresh allocation, initialised once; the owner is the view the area is
        // added to (AppKit doesn't retain it, and the view outlives its own tracking areas).
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
        view
    }

    fn bar(&self) -> Option<Retained<BarView>> {
        self.ivars().bar.load()
    }

    /// Cover `span` of the bar (bar coordinates; `None` hides the view).
    /// Untouched when it already does, so an unchanged wing costs nothing.
    pub(super) fn place(&self, span: Option<(f64, f64)>, h: f64) {
        let Some((x, w)) = span else {
            self.setHidden(true);
            return;
        };
        let f = super::rect(x, 0.0, w, h);
        if self.frame() != f {
            self.setFrame(f);
        }
        if self.bounds().origin != NSPoint::new(x, 0.0) {
            self.setBoundsOrigin(NSPoint::new(x, 0.0));
        }
        if self.isHidden() {
            self.setHidden(false);
        }
    }
}

impl BarView {
    pub(super) fn new(mtm: MainThreadMarker, frame: NSRect, screen: usize, m: Metrics) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            screen: Cell::new(screen),
            m: Cell::new(m),
            layout: RefCell::new(None),
            hits: RefCell::new(Vec::new()),
            hovered: RefCell::new(None),
            wings: Cell::new((0.0, frame.size.width)),
            wing_views: RefCell::new(Vec::new()),
        });
        // SAFETY: NSView's designated initialiser, called once on a freshly allocated view.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        for side in [Side::Left, Side::Right] {
            let w = WingView::new(mtm, &view, side);
            view.addSubview(&w);
            view.ivars().wing_views.borrow_mut().push(w);
        }
        view
    }

    /// A wing's view.
    pub(super) fn wing_view(&self, side: Side) -> Retained<WingView> {
        self.ivars().wing_views.borrow()[side as usize].clone()
    }

    /// Ask a wing's view to draw again.
    pub(super) fn redraw_wing(&self, side: Side) {
        self.wing_view(side).setNeedsDisplay(true);
    }
}
