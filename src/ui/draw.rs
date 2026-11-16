//! Painting a layout with AppKit: the wings, then each segment. Colours, fonts
//! and tinted SF Symbols are cached (main thread only).

use super::layout::{self, Env, Kind, Metrics, Seg, Side, Snapshot};
use super::view::BarView;
use crate::config::{self, Rgba, Theme};
use crate::state;
use crate::widgets::{self, Registry};
use objc2::AllocAnyThread;
use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBezierPath, NSColor, NSCompositingOperation, NSFont, NSFontAttributeName, NSFontWeightMedium,
    NSFontWeightSemibold, NSForegroundColorAttributeName, NSImage, NSImageSymbolConfiguration, NSLineBreakMode,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSStringDrawing,
};
use objc2_foundation::{NSAttributedStringKey, NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::collections::HashMap;

/// Tinted SF Symbols by (name, colour key, point size bits); `None` = no image could be made.
type IconCache = HashMap<(String, u32, u64), Option<Retained<NSImage>>>;

thread_local! {
    static ICONS: RefCell<IconCache> = RefCell::new(HashMap::new());
    static FONTS: RefCell<HashMap<(u64, bool), Retained<NSFont>>> = RefCell::new(HashMap::new());
    static COLORS: RefCell<HashMap<u32, Retained<NSColor>>> = RefCell::new(HashMap::new());
    /// The state copy each draw lays out from, kept to reuse its allocations.
    static SNAPSHOT: RefCell<Snapshot> = RefCell::new(Snapshot::default());
}

fn color_key(c: Rgba) -> u32 {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (b(c.0) << 24) | (b(c.1) << 16) | (b(c.2) << 8) | b(c.3)
}

fn nscolor(c: Rgba) -> Retained<NSColor> {
    COLORS.with(|m| {
        m.borrow_mut()
            .entry(color_key(c))
            .or_insert_with(|| NSColor::colorWithSRGBRed_green_blue_alpha(c.0, c.1, c.2, c.3))
            .clone()
    })
}

fn font(size: f64, bold: bool) -> Retained<NSFont> {
    FONTS.with(|m| {
        m.borrow_mut()
            .entry((size.to_bits(), bold))
            .or_insert_with(|| {
                // SAFETY: AppKit's font weight constants are immutable statics, initialised on load.
                let w = unsafe { if bold { NSFontWeightSemibold } else { NSFontWeightMedium } };
                NSFont::monospacedDigitSystemFontOfSize_weight(size, w)
            })
            .clone()
    })
}

fn icon(name: &str, color: Rgba, size: f64) -> Option<Retained<NSImage>> {
    ICONS.with(|m| {
        m.borrow_mut()
            .entry((name.to_string(), color_key(color), size.to_bits()))
            .or_insert_with(|| {
                if crate::icons::is_bundled(name) {
                    return bundled_icon(name, color, size);
                }
                let img = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
                    .or_else(|| {
                        NSImage::imageWithSystemSymbolName_accessibilityDescription(
                            &NSString::from_str("circle.fill"),
                            None,
                        )
                    })?;
                // SAFETY: an immutable AppKit constant, initialised on load.
                let w = unsafe { NSFontWeightMedium };
                let cfg = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, w);
                let tint = NSImageSymbolConfiguration::configurationWithHierarchicalColor(&nscolor(color));
                img.imageWithSymbolConfiguration(&cfg.configurationByApplyingConfiguration(&tint))
            })
            .clone()
    })
}

/// How tall a bundled icon is drawn, for SF Symbols of point size `size`: a
/// symbol's image is taller than its point size, and simple-bar's icons fill
/// their whole box.
const BUNDLED_SCALE: f64 = 1.1;

/// A bundled (simple-bar) icon, drawn from its SVG in `color`. An unknown
/// name falls back to the generic `Default` icon, as SF Symbols fall back to
/// a dot.
fn bundled_icon(name: &str, color: Rgba, size: f64) -> Option<Retained<NSImage>> {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let fill = format!("#{:02x}{:02x}{:02x}", b(color.0), b(color.1), b(color.2));
    let px = (size * BUNDLED_SCALE).round();
    let doc =
        crate::icons::svg(name, &fill, color.3, px).or_else(|| crate::icons::svg("Default", &fill, color.3, px))?;
    let data = NSData::with_bytes(doc.as_bytes());
    let img = NSImage::initWithData(NSImage::alloc(), &data)?;
    // the SVG's own size, in points
    let sz = img.size();
    (sz.width > 0.0 && sz.height > 0.0).then_some(img)
}

fn attrs(f: &NSFont, c: &NSColor, truncate: bool) -> Retained<NSDictionary<NSAttributedStringKey, AnyObject>> {
    // SAFETY: the attribute names are immutable AppKit constants, and each value
    // has the type its key expects (NSFont, NSColor, NSParagraphStyle).
    unsafe {
        if truncate {
            let p = NSMutableParagraphStyle::new();
            p.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            NSDictionary::from_slices(
                &[NSFontAttributeName, NSForegroundColorAttributeName, NSParagraphStyleAttributeName],
                &[f as &AnyObject, c as &AnyObject, &p as &AnyObject],
            )
        } else {
            NSDictionary::from_slices(
                &[NSFontAttributeName, NSForegroundColorAttributeName],
                &[f as &AnyObject, c as &AnyObject],
            )
        }
    }
}

fn measure(s: &str, f: &NSFont) -> NSSize {
    let a = attrs(f, &nscolor(Rgba(1.0, 1.0, 1.0, 1.0)), false);
    // SAFETY: `a` holds only string-drawing attributes with values of the right types.
    unsafe { NSString::from_str(s).sizeWithAttributes(Some(&a)) }
}

pub(super) fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w.max(0.0), h.max(0.0)))
}

fn draw_icon(img: &NSImage, x: f64, h: f64) {
    let sz = img.size();
    // SAFETY: called from a wing's drawRect:, with its graphics context current;
    // a zero source rect means the whole image and there are no hints.
    unsafe {
        img.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
            rect(x, ((h - sz.height) / 2.0).round(), sz.width, sz.height),
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.0,
            true,
            None,
        );
    }
}

impl BarView {
    /// Lay the bar out again from the state, and have only the wings that look
    /// different drawn again (and moved or resized, when their span changed).
    pub(super) fn update(&self) {
        let cfg = config::current();
        let reg = widgets::current();
        let iv = self.ivars();
        let m = iv.m.get();
        let width = self.frame().size.width;

        let lay = SNAPSHOT.with(|cell| {
            let mut fresh = None;
            let mut kept = cell.try_borrow_mut().ok();
            let snap: &mut Snapshot = match kept.as_deref_mut() {
                Some(s) => s,
                None => fresh.insert(Snapshot::default()),
            };
            // copy, then let go of the lock before measuring anything
            snap.fill(&state::lock());
            let text_w = |s: &str, bold: bool| measure(s, &font(m.font, bold)).width;
            let icon_w = |n: &str, c: Rgba| icon(n, c, m.icon).map(|i| i.size().width);
            let app_icon = |app: &str| crate::icons::for_app(app).map(String::from);
            let simple_bar = cfg.theme.icons == config::IconSet::SimpleBar;
            let env = Env {
                m,
                width,
                screen: iv.screen.get(),
                screens: super::bar_count(),
                title_max: cfg.bar.title_max,
                text_w: &text_w,
                icon_w: &icon_w,
                app_icon: if simple_bar { Some(&app_icon) } else { None },
            };
            layout::layout(&cfg.left, &cfg.right, snap, &reg, &env)
        });
        iv.wings.set((lay.left_end, lay.right_start));
        if iv.screen.get() == 1
            && let Some(g) = lay.geom(m.h)
        {
            super::write_geom(g);
        }
        // BarPopover's tab hangs under one widget, on the main screen
        if iv.screen.get() == 1
            && let Some(key) = reg.popover_tab()
            && let Some(seg) = lay.left.iter().chain(&lay.right).find(|s| s.key == key)
        {
            super::input::report_popover_tab(seg.x, seg.w, m.h, width);
        }
        *iv.hits.borrow_mut() = lay.regions();
        for side in [Side::Left, Side::Right] {
            let changed = lay.changed(iv.layout.borrow().as_ref(), side);
            let v = self.wing_view(side);
            v.place(lay.span(side, width), m.h);
            if changed {
                v.setNeedsDisplay(true);
            }
        }
        *iv.layout.borrow_mut() = Some(lay);
    }

    /// Paint one wing (from its view's `drawRect:`, in bar coordinates): its
    /// shape, then its segments.
    pub(super) fn draw_wing(&self, side: Side) {
        let cfg = config::current();
        let t = &cfg.theme;
        let iv = self.ivars();
        let m = iv.m.get();
        let width = self.frame().size.width;
        let h = m.h;
        let lay = iv.layout.borrow();
        let Some(lay) = lay.as_ref() else { return };

        nscolor(t.wing).setFill();
        let r = m.corner;
        match side {
            Side::Left if lay.left_end > 0.0 => {
                let left_end = lay.left_end;
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect(0.0, -r, left_end, h + r), r, r).fill();
                NSBezierPath::fillRect(rect(0.0, 0.0, left_end - r, h));
            }
            Side::Right if lay.right_start < width => {
                let right_start = lay.right_start;
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    rect(right_start, -r, width - right_start + r, h + r),
                    r,
                    r,
                )
                .fill();
                NSBezierPath::fillRect(rect(right_start + r, 0.0, width - right_start - r, h));
            }
            _ => return,
        }

        let hovered = iv.hovered.borrow();
        let reg = widgets::current();
        for s in lay.segs(side) {
            draw_seg(s, &m, t, hovered.as_deref(), &reg);
        }
    }
}

fn draw_seg(s: &Seg, m: &Metrics, t: &Theme, hovered: Option<&str>, reg: &Registry) {
    let h = m.h;
    let x = s.x;
    let f = font(m.font, false);
    let th = measure("0", &f).height;
    let ty = ((h - th) / 2.0).round();
    let gap = m.gap();
    match &s.kind {
        Kind::Spaces(parts) => {
            let sp_pad = m.space_pad();
            let mut cx = x;
            for p in parts {
                let c = if p.focused { t.text } else { t.dim };
                let a = attrs(&f, &nscolor(c), false);
                // SAFETY: inside drawRect:; `a` holds string-drawing attributes of the right types.
                unsafe {
                    NSString::from_str(&p.name).drawAtPoint_withAttributes(NSPoint::new(cx + sp_pad, ty), Some(&a))
                };
                let mut ix = cx + p.w - sp_pad - p.icons.iter().map(|(_, w)| w + layout::app_gap(m)).sum::<f64>();
                for (name, w) in &p.icons {
                    ix += layout::app_gap(m);
                    if let Some(img) = icon(name, c, m.icon) {
                        draw_icon(&img, ix, h);
                    }
                    ix += w;
                }
                if p.focused {
                    nscolor(t.text).setFill();
                    let uy = (ty + th + 2.0).min(h - 3.0);
                    NSBezierPath::fillRect(rect(cx + sp_pad - 1.0, uy, p.w - sp_pad * 2.0 + 2.0, 1.5));
                }
                cx += p.w;
            }
        }
        Kind::Title(text, tw) => {
            let a = attrs(&f, &nscolor(t.dim), true);
            // SAFETY: inside drawRect:; `a` holds string-drawing attributes of the right types.
            unsafe {
                NSString::from_str(text).drawInRect_withAttributes(rect(x + m.pad, ty, *tw, th), Some(&a));
            }
        }
        Kind::Item(it, iw, tw) => {
            if hovered == Some(&*s.key) && reg.clickable(&s.key) {
                nscolor(t.wing_hover).setFill();
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    rect(x + 2.0, 4.0, s.w - 4.0, h - 8.0),
                    6.0,
                    6.0,
                )
                .fill();
            }
            let mut cx = x + m.pad;
            if let Some(img) = it.icon.as_ref().and_then(|n| icon(n, it.icon_color, m.icon)) {
                draw_icon(&img, cx, h);
                cx += iw + if *tw > 0.0 { gap } else { 0.0 };
            }
            if *tw > 0.0 {
                let fb = font(m.font, it.bold);
                let a = attrs(&fb, &nscolor(it.text_color), false);
                // SAFETY: inside drawRect:; `a` holds string-drawing attributes of the right types.
                unsafe { NSString::from_str(&it.text).drawAtPoint_withAttributes(NSPoint::new(cx, ty), Some(&a)) };
                cx += tw + gap;
            }
            if let Some(img) = it.trail.as_ref().and_then(|n| icon(n, it.icon_color, m.icon)) {
                draw_icon(&img, cx, h);
            }
        }
    }
}
