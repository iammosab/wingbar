//! Where everything goes, with no AppKit in sight: from the wing lists, a copy
//! of the state and a way to measure text and icons, work out each segment's
//! width and x, how far each wing reaches (around a notch), and which widget
//! or workspace sits under any x. `draw` paints the result; `input` routes
//! events through its hit regions.

use crate::config::Rgba;
use crate::state::{Item, Panel, Space, State};
use crate::widgets::{self, Registry};
use std::collections::{HashMap, HashSet};

/// Breathing room at the screen edges.
pub const EDGE: f64 = 6.0;
/// Extra room beside the rounded inner corners.
pub const INNER: f64 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub h: f64,
    pub font: f64,
    pub icon: f64,
    pub pad: f64,
    pub corner: f64,
    /// (end of the left free area, start of the right free area) beside a notch.
    pub notch: Option<(f64, f64)>,
}

impl Metrics {
    /// Space between an icon and its text (and before a trailing icon).
    pub fn gap(&self) -> f64 {
        (self.font * 0.45).round()
    }

    /// Padding on each side of a workspace name.
    pub fn space_pad(&self) -> f64 {
        (self.pad * 0.8).round()
    }
}

/// What drawing needs from the state, copied out under the lock so workers
/// never wait on AppKit. `fill` reuses the previous copy's allocations.
#[derive(Default)]
pub struct Snapshot {
    pub items: HashMap<String, Item>,
    pub spaces: Vec<Space>,
    pub title: String,
    pub disabled: HashSet<String>,
    pub panel: Option<Panel>,
}

impl Snapshot {
    pub fn fill(&mut self, st: &State) {
        self.items.clone_from(&st.items);
        self.spaces.clone_from(&st.spaces);
        self.title.clone_from(&st.title);
        self.disabled.clone_from(&st.disabled);
        self.panel = st.panel;
    }
}

/// The icon of an app's windows, `None` for an app without one of its own.
pub type AppIcon = dyn Fn(&str) -> Option<String>;

/// How to measure, and the bar being laid out.
pub struct Env<'a> {
    pub m: Metrics,
    /// The bar's width.
    pub width: f64,
    /// This bar's screen, 1-based.
    pub screen: usize,
    /// How many bars there are (with one, every workspace shows on it).
    pub screens: usize,
    pub title_max: f64,
    /// Width of a text in the bar's font, regular or bold.
    pub text_w: &'a dyn Fn(&str, bool) -> f64,
    /// Width of a tinted SF Symbol at the icon size; `None` = no image.
    pub icon_w: &'a dyn Fn(&str, Rgba) -> Option<f64>,
    /// The icon for an app's windows in the workspaces (simple-bar's look):
    /// `None` for no app icons; the function gives `None` for an app without
    /// an icon of its own.
    pub app_icon: Option<&'a AppIcon>,
}

/// One workspace in the spaces segment.
#[derive(Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub focused: bool,
    /// After the name: an icon for each app with a window there, and its width.
    pub icons: Vec<(String, f64)>,
    pub w: f64,
}

/// What a segment shows, copied out of the snapshot: a layout is kept from
/// one update to the next, to draw from and to tell which wing changed.
#[derive(Debug, PartialEq)]
pub enum Kind {
    Spaces(Vec<Part>),
    /// The title and the width it may take (truncated beyond).
    Title(String, f64),
    /// The item, its icon width and its text width.
    Item(Item, f64, f64),
}

#[derive(Debug, PartialEq)]
pub struct Seg {
    pub key: String,
    pub kind: Kind,
    pub x: f64,
    pub w: f64,
}

/// One of the two wings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    /// One part of a widget: (widget key, workspace name).
    Part(String, String),
    Widget(String),
}

impl Hit {
    pub fn key(&self) -> &str {
        match self {
            Hit::Part(k, _) | Hit::Widget(k) => k,
        }
    }
}

/// A clickable span of the bar, `x0 <= x < x1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub x0: f64,
    pub x1: f64,
    pub hit: Hit,
}

#[derive(Debug)]
pub struct Layout {
    pub left: Vec<Seg>,
    pub right: Vec<Seg>,
    /// Where the left wing ends (0 = no left wing).
    pub left_end: f64,
    /// Where the right wing starts (the bar's width = no right wing).
    pub right_start: f64,
    /// The left wing's inner corner is square: a hover panel hangs flush under it.
    pub square_left: bool,
    /// The right wing's inner corner is square, likewise.
    pub square_right: bool,
}

impl Layout {
    /// What can be clicked, hovered or scrolled: each workspace and each item
    /// (the title isn't).
    pub fn regions(&self) -> Vec<Region> {
        let mut out = Vec::new();
        for s in self.left.iter().chain(&self.right) {
            match &s.kind {
                Kind::Spaces(parts) => {
                    let mut x = s.x;
                    for p in parts {
                        out.push(Region { x0: x, x1: x + p.w, hit: Hit::Part(s.key.clone(), p.name.clone()) });
                        x += p.w;
                    }
                }
                Kind::Title(..) => {}
                Kind::Item(..) => out.push(Region { x0: s.x, x1: s.x + s.w, hit: Hit::Widget(s.key.clone()) }),
            }
        }
        out
    }

    /// The `simple-bar-geom` line: the workspaces' x and width, then the left
    /// wing (0 and its end) and the bar height. `None` without workspaces on the left.
    pub fn geom(&self, h: f64) -> Option<String> {
        let s = self.left.iter().find(|s| matches!(s.kind, Kind::Spaces(_)))?;
        Some(format!("{} {} 0 {} {}", s.x.round(), s.w.round(), self.left_end.round(), h.round()))
    }

    /// One wing's segments.
    pub fn segs(&self, side: Side) -> &[Seg] {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    /// The span of the bar a wing's view covers, as (x, width) in whole points
    /// so the view sits on whole pixels and draws exactly what the bar drew
    /// there; `None` when there's no such wing. The left wing is `0..left_end`,
    /// the right one `right_start..width` (its rounded corner reaches no further).
    pub fn span(&self, side: Side, width: f64) -> Option<(f64, f64)> {
        match side {
            Side::Left if self.left_end > 0.0 => Some((0.0, self.left_end.ceil().min(width))),
            Side::Right if self.right_start < width => {
                let x = self.right_start.floor().max(0.0);
                Some((x, width - x))
            }
            _ => None,
        }
    }

    /// Whether a wing looks different from `old`'s (a different shape or
    /// different segments), so its view needs drawing again.
    pub fn changed(&self, old: Option<&Layout>, side: Side) -> bool {
        let Some(old) = old else { return true };
        match side {
            Side::Left => self.left_end != old.left_end || self.square_left != old.square_left || self.left != old.left,
            Side::Right => {
                self.right_start != old.right_start || self.square_right != old.square_right || self.right != old.right
            }
        }
    }

    /// The wing a widget shows in, if it shows.
    pub fn side_of(&self, key: &str) -> Option<Side> {
        if self.left.iter().any(|s| s.key == key) {
            Some(Side::Left)
        } else if self.right.iter().any(|s| s.key == key) {
            Some(Side::Right)
        } else {
            None
        }
    }
}

/// The region under `x`, if any.
pub fn hit_at(regions: &[Region], x: f64) -> Option<&Region> {
    regions.iter().find(|r| x >= r.x0 && x < r.x1)
}

/// Measure the segments of one wing, in order. Disabled widgets, unknown
/// names, items with nothing to show, an empty title and a screen without
/// workspaces are left out.
fn segments(names: &[String], snap: &Snapshot, reg: &Registry, env: &Env) -> Vec<Seg> {
    let m = &env.m;
    let gap = m.gap();
    let mut out = Vec::new();
    for name in names {
        let key = reg.key_for_name(name);
        if snap.disabled.contains(&key) {
            continue;
        }
        let Some(widget) = reg.get(&key) else { continue };
        let key = widget.key().to_string();
        match widget.kind() {
            widgets::Kind::Spaces => {
                let sp_pad = m.space_pad();
                let parts: Vec<Part> = snap
                    .spaces
                    .iter()
                    .filter(|s| s.screen == 0 || s.screen == env.screen || env.screens <= 1)
                    .map(|s| {
                        let icons: Vec<(String, f64)> = match env.app_icon {
                            Some(f) => s
                                .apps
                                .iter()
                                .filter_map(|a| {
                                    let name = f(a)?;
                                    let w = (env.icon_w)(&name, Rgba(1.0, 1.0, 1.0, 1.0))?;
                                    Some((name, w))
                                })
                                .take(MAX_APP_ICONS)
                                .collect(),
                            None => Vec::new(),
                        };
                        let iw: f64 = icons.iter().map(|(_, w)| w + app_gap(m)).sum();
                        Part {
                            name: s.name.clone(),
                            focused: s.focused,
                            w: (env.text_w)(&s.name, false) + iw + sp_pad * 2.0,
                            icons,
                        }
                    })
                    .collect();
                if parts.is_empty() {
                    continue;
                }
                let w = parts.iter().map(|p| p.w).sum::<f64>() + m.pad;
                out.push(Seg { key, kind: Kind::Spaces(parts), x: 0.0, w });
            }
            widgets::Kind::Title => {
                if snap.title.is_empty() {
                    continue;
                }
                let tw = (env.text_w)(&snap.title, false).min(env.title_max);
                out.push(Seg { key, kind: Kind::Title(snap.title.clone(), tw), x: 0.0, w: tw + m.pad * 2.0 });
            }
            widgets::Kind::Item => {
                let Some(it) = snap.items.get(&key) else { continue };
                let tw = if it.text.is_empty() { 0.0 } else { (env.text_w)(&it.text, it.bold) };
                let iw = it.icon.as_deref().and_then(|n| (env.icon_w)(n, it.icon_color)).unwrap_or(0.0);
                let trail = it.trail.as_deref().and_then(|n| (env.icon_w)(n, it.icon_color)).map_or(0.0, |w| w + gap);
                let inner = iw + if iw > 0.0 && tw > 0.0 { gap } else { 0.0 } + tw + trail;
                out.push(Seg { key, kind: Kind::Item(it.clone(), iw, tw), x: 0.0, w: inner + m.pad * 2.0 });
            }
        }
    }
    out
}

/// At most this many app icons per workspace.
pub const MAX_APP_ICONS: usize = 3;

/// Space before each app icon in a workspace.
pub fn app_gap(m: &Metrics) -> f64 {
    (m.font * 0.35).round()
}

fn total(segs: &[Seg]) -> f64 {
    segs.iter().map(|s| s.w).sum()
}

/// Lay out both wings. The title gives way first: it shrinks to what the left
/// wing has room for (the notch, or what the right wing leaves) and goes when
/// under 40 pt. Beside a notch, right items that would slide under it are
/// dropped from the start (the least important end).
pub fn layout(left: &[String], right: &[String], snap: &Snapshot, reg: &Registry, env: &Env) -> Layout {
    let m = &env.m;
    let width = env.width;
    let mut left = segments(left, snap, reg, env);
    let mut right = segments(right, snap, reg, env);

    let right_w = total(&right) + EDGE + INNER;
    // how far the left wing may reach: the notch, or what the right wing leaves
    let limit = match m.notch {
        Some((l, _)) => l,
        None => width - right_w - 40.0,
    };
    let fixed: f64 =
        EDGE + INNER + left.iter().filter(|s| !matches!(s.kind, Kind::Title(..))).map(|s| s.w).sum::<f64>();
    let room = (limit - fixed - m.pad * 2.0).max(0.0);
    // A title squeezed under 40 pt says nothing, so it goes; a title that is
    // simply short ("cmux") and fits stays.
    let mut squeezed_away = false;
    for s in left.iter_mut() {
        if let Kind::Title(_, tw) = &mut s.kind
            && *tw > room
        {
            *tw = room;
            s.w = *tw + m.pad * 2.0;
            squeezed_away = *tw < 40.0;
        }
    }
    if squeezed_away {
        left.retain(|s| !matches!(s.kind, Kind::Title(..)));
    }
    if let Some((_, r)) = m.notch {
        while !right.is_empty() && width - (total(&right) + EDGE + INNER) < r {
            right.remove(0);
        }
    }

    let left_end = if left.is_empty() { 0.0 } else { EDGE + INNER + total(&left) };
    let right_start = if right.is_empty() { width } else { width - (EDGE + INNER + total(&right)) };
    let mut x = EDGE;
    for s in left.iter_mut() {
        s.x = x;
        x += s.w;
    }
    let mut x = right_start + INNER;
    for s in right.iter_mut() {
        s.x = x;
        x += s.w;
    }
    let panel = snap.panel.filter(|p| p.screen == env.screen).map(|p| (p.x0, p.x1));
    let (square_left, square_right) = square_corners(left_end, right_start, width, panel);
    Layout { left, right, left_end, right_start, square_left, square_right }
}

/// Which wings' inner (rounded) corners go square because a hover panel
/// hangs flush under them, so wing and panel read as one shape. A panel stays
/// inside its widget's wing, so one that reaches a wing's inner end (within a
/// point: BarPopover reports whole points) joins that wing there.
pub fn square_corners(left_end: f64, right_start: f64, width: f64, panel: Option<(f64, f64)>) -> (bool, bool) {
    let Some((x0, x1)) = panel else { return (false, false) };
    let left = left_end > 0.0 && x0 < left_end && x1 >= left_end - 1.0;
    let right = right_start < width && x1 > right_start && x0 <= right_start + 1.0;
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture_registry;

    const M: Metrics = Metrics { h: 34.0, font: 12.0, icon: 12.0, pad: 10.0, corner: 14.0, notch: None };
    const WHITE: Rgba = Rgba(1.0, 1.0, 1.0, 1.0);
    // gap = round(12 * 0.45) = 5, space pad = 8

    /// 10 pt a character (12 bold), so widths are easy to work out.
    fn text_w(s: &str, bold: bool) -> f64 {
        s.chars().count() as f64 * if bold { 12.0 } else { 10.0 }
    }

    /// Icons are 14 pt, except "missing", which has no image.
    fn icon_w(name: &str, _c: Rgba) -> Option<f64> {
        (name != "missing").then_some(14.0)
    }

    fn env(m: Metrics, width: f64) -> Env<'static> {
        Env { m, width, screen: 1, screens: 1, title_max: 420.0, text_w: &text_w, icon_w: &icon_w, app_icon: None }
    }

    fn item(icon: Option<&str>, text: &str) -> Item {
        Item {
            icon: icon.map(String::from),
            text: text.into(),
            icon_color: WHITE,
            text_color: WHITE,
            bold: false,
            level: String::new(),
            trail: None,
        }
    }

    fn space(name: &str, focused: bool, screen: usize) -> Space {
        Space { name: name.into(), focused, screen, apps: vec![] }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn snap() -> Snapshot {
        let mut s = Snapshot::default();
        s.items.insert("time".into(), item(Some("clock"), "20:15"));
        s.items.insert("date".into(), item(None, "Thu 12 Nov"));
        s.items.insert("clickup".into(), item(Some("checkmark"), "3"));
        s.spaces = vec![space("1", false, 0), space("2", true, 0)];
        s.title = "Terminal / zsh".into();
        s
    }

    fn keys(segs: &[Seg]) -> Vec<&str> {
        segs.iter().map(|s| s.key.as_str()).collect()
    }

    #[test]
    fn segments_follow_the_wing_order_and_names_map_to_keys() {
        let reg = fixture_registry();
        let s = snap();
        let l = layout(
            &names(&["spaces", "process", "clickup"]),
            &names(&["date-display", "time"]),
            &s,
            &reg,
            &env(M, 2000.0),
        );
        assert_eq!(keys(&l.left), ["spaces", "title", "clickup"]);
        assert_eq!(keys(&l.right), ["date", "time"]);
    }

    #[test]
    fn widths_and_positions() {
        let reg = fixture_registry();
        let s = snap();
        let l = layout(&names(&["spaces", "clickup"]), &names(&["date", "time"]), &s, &reg, &env(M, 2000.0));
        // spaces: two names of 10 + 2 * 8 padding, plus the pad
        assert_eq!(
            l.left[0].kind,
            Kind::Spaces(vec![
                Part { name: "1".into(), focused: false, icons: vec![], w: 26.0 },
                Part { name: "2".into(), focused: true, icons: vec![], w: 26.0 }
            ])
        );
        assert_eq!((l.left[0].x, l.left[0].w), (EDGE, 62.0));
        // icon 14 + gap 5 + text 10 + pads 20
        assert_eq!((l.left[1].x, l.left[1].w), (EDGE + 62.0, 49.0));
        assert_eq!(l.left_end, EDGE + INNER + 62.0 + 49.0);
        // text only: 100 + 20; icon + text: 14 + 5 + 50 + 20
        assert_eq!(l.right.iter().map(|s| s.w).collect::<Vec<_>>(), [120.0, 89.0]);
        assert_eq!(l.right_start, 2000.0 - (EDGE + INNER + 209.0));
        assert_eq!(l.right[0].x, l.right_start + INNER);
        assert_eq!(l.right[1].x, l.right_start + INNER + 120.0);
    }

    #[test]
    fn item_widths_with_missing_icons_and_trails() {
        let reg = fixture_registry();
        let mut s = Snapshot::default();
        let mut it = item(Some("missing"), "ab");
        it.trail = Some("cup".into());
        it.bold = true;
        s.items.insert("time".into(), it);
        s.items.insert("date".into(), item(Some("calendar"), ""));
        s.items.insert("battery".into(), {
            let mut b = item(None, "");
            b.trail = Some("missing".into());
            b
        });
        let l = layout(&[], &names(&["time", "date", "battery"]), &s, &reg, &env(M, 2000.0));
        // no icon: bold text 24 + trail 14 + gap 5, plus pads
        assert_eq!(l.right[0].kind, Kind::Item(s.items["time"].clone(), 0.0, 24.0));
        assert_eq!(l.right[0].w, 24.0 + 19.0 + 20.0);
        // icon only: no gap
        assert_eq!(l.right[1].w, 14.0 + 20.0);
        // a trail without an image takes no room, not even its gap
        assert_eq!(l.right[2].w, 20.0);
    }

    #[test]
    fn disabled_unknown_and_empty_widgets_are_skipped() {
        let reg = fixture_registry();
        let mut s = snap();
        s.disabled.insert("date".into());
        let l = layout(
            &names(&["nothing-by-this-name", "user-3", "clickup"]),
            &names(&["date-display", "battery", "time"]),
            &s,
            &reg,
            &env(M, 2000.0),
        );
        // user:3 and battery have shown nothing yet; date is disabled under its alias
        assert_eq!(keys(&l.left), ["clickup"]);
        assert_eq!(keys(&l.right), ["time"]);

        s.title.clear();
        s.spaces.clear();
        let l = layout(&names(&["spaces", "title"]), &[], &s, &reg, &env(M, 2000.0));
        assert!(l.left.is_empty());
        assert_eq!((l.left_end, l.right_start), (0.0, 2000.0), "no wings at all");
    }

    #[test]
    fn spaces_are_filtered_per_screen() {
        let reg = fixture_registry();
        let mut s = snap();
        s.spaces = vec![space("1", true, 1), space("2", false, 2), space("all", false, 0)];
        let parts = |s: &Snapshot, screen, screens| {
            let e = Env { screen, screens, ..env(M, 2000.0) };
            let l = layout(&names(&["spaces"]), &[], s, &reg, &e);
            match l.left.first().map(|s| &s.kind) {
                Some(Kind::Spaces(p)) => p.iter().map(|p| p.name.to_string()).collect::<Vec<_>>(),
                _ => vec![],
            }
        };
        assert_eq!(parts(&s, 1, 2), ["1", "all"]);
        assert_eq!(parts(&s, 2, 2), ["2", "all"]);
        assert_eq!(parts(&s, 1, 1), ["1", "2", "all"], "one bar shows every workspace");
        s.spaces = vec![space("2", false, 2)];
        assert!(parts(&s, 1, 2).is_empty(), "no segment when the screen has no workspace");
    }

    #[test]
    fn the_title_is_capped_then_squeezed_then_dropped() {
        let reg = fixture_registry();
        let mut s = snap();
        s.title = "x".repeat(100); // 1000 pt
        let title_w = |width: f64| {
            let l = layout(&names(&["title"]), &names(&["date"]), &s, &reg, &env(M, width));
            l.left.iter().find_map(|s| match s.kind {
                Kind::Title(_, tw) => Some((tw, s.w)),
                _ => None,
            })
        };
        assert_eq!(title_w(3000.0), Some((420.0, 440.0)), "title_max");
        // 600 - right wing (12 + 120) - 40 - fixed (12) - pads (20)
        assert_eq!(title_w(600.0), Some((396.0, 416.0)));
        assert_eq!(title_w(204.0 + 39.0), None, "39 pt of room: gone");
        assert_eq!(title_w(204.0 + 40.0), Some((40.0, 60.0)));
    }

    #[test]
    fn a_short_title_that_fits_is_kept() {
        let reg = fixture_registry();
        let mut s = snap();
        s.title = "abc".into(); // 30 pt: shorter than the 40 pt floor, but not squeezed
        let title_w = |width: f64| {
            let l = layout(&names(&["title"]), &names(&["date"]), &s, &reg, &env(M, width));
            l.left.iter().find_map(|s| match s.kind {
                Kind::Title(_, tw) => Some(tw),
                _ => None,
            })
        };
        assert_eq!(title_w(3000.0), Some(30.0));
        // 204 + 30 pt of room: it just fits, untouched
        assert_eq!(title_w(204.0 + 30.0), Some(30.0));
        // 25 pt of room: squeezed under 40 pt, so it goes
        assert_eq!(title_w(204.0 + 25.0), None);
    }

    #[test]
    fn a_notch_limits_the_left_wing_and_drops_right_items_from_the_start() {
        let reg = fixture_registry();
        let mut s = snap();
        s.title = "x".repeat(100);
        let m = Metrics { notch: Some((300.0, 700.0)), ..M };
        let l = layout(&names(&["title"]), &names(&["date", "clickup", "time"]), &s, &reg, &env(m, 1000.0));
        // the title reaches the notch: 300 - 12 - 20
        assert_eq!(l.left[0].kind, Kind::Title(s.title.clone(), 268.0));
        assert_eq!(l.left_end, 300.0);
        // date (120) + clickup (49) + time (89) + 12 = 270, and 1000 - 270 >= 700: all fit
        assert_eq!(keys(&l.right), ["date", "clickup", "time"]);
        let l = layout(&names(&["title"]), &names(&["date", "clickup", "time"]), &s, &reg, &env(m, 950.0));
        // 950 - 270 = 680 < 700: date goes, then 950 - 150 = 800 fits
        assert_eq!(keys(&l.right), ["clickup", "time"]);
        assert_eq!(l.right_start, 950.0 - (EDGE + INNER + 138.0));
        assert!(l.right_start >= 700.0);
    }

    #[test]
    fn hit_testing() {
        let reg = fixture_registry();
        let s = snap();
        let l = layout(&names(&["spaces", "title", "clickup"]), &names(&["time"]), &s, &reg, &env(M, 2000.0));
        let regions = l.regions();
        let at = |x: f64| hit_at(&regions, x).map(|r| r.hit.clone());
        let part = |n: &str| Some(Hit::Part("spaces".into(), n.into()));
        assert_eq!(at(EDGE), part("1"));
        assert_eq!(at(EDGE + 25.9), part("1"));
        assert_eq!(at(EDGE + 26.0), part("2"), "a region ends where the next starts");
        assert_eq!(at(EDGE + 52.0), None, "the pad after the workspaces");
        assert_eq!(at(EDGE + 62.0 + 10.0), None, "the title can't be clicked");
        let clickup = l.left[2].x;
        assert_eq!(at(clickup + 1.0), Some(Hit::Widget("clickup".into())));
        assert_eq!(at(EDGE - 1.0), None, "the edge");
        assert_eq!(at(l.left_end + 1.0), None, "between the wings");
        assert_eq!(at(l.right[0].x), Some(Hit::Widget("time".into())));
        assert_eq!(at(2000.0 - 1.0), None, "the right edge");
        assert_eq!(regions.iter().map(|r| r.hit.key()).collect::<Vec<_>>(), ["spaces", "spaces", "clickup", "time"]);
    }

    #[test]
    fn geom_line() {
        let reg = fixture_registry();
        let s = snap();
        let l = layout(&names(&["clickup", "spaces"]), &[], &s, &reg, &env(M, 2000.0));
        assert_eq!(l.geom(34.0).as_deref(), Some("55 62 0 123 34"));
        let l = layout(&names(&["clickup"]), &names(&["spaces"]), &s, &reg, &env(M, 2000.0));
        assert_eq!(l.geom(34.0), None, "only workspaces on the left count");
    }

    #[test]
    fn each_wing_gets_a_span_in_whole_points() {
        let reg = fixture_registry();
        let s = snap();
        let l = layout(&names(&["spaces", "clickup"]), &names(&["date", "time"]), &s, &reg, &env(M, 2000.0));
        // left: 6 + 6 + 62 + 49 = 123; right starts at 2000 - 221 = 1779
        assert_eq!(l.span(Side::Left, 2000.0), Some((0.0, 123.0)));
        assert_eq!(l.span(Side::Right, 2000.0), Some((1779.0, 221.0)));
        // fractional ends round outwards, so the views cover all the wing does
        let l = Layout {
            left: vec![],
            right: vec![],
            left_end: 123.4,
            right_start: 1779.6,
            square_left: false,
            square_right: false,
        };
        assert_eq!(l.span(Side::Left, 2000.0), Some((0.0, 124.0)));
        assert_eq!(l.span(Side::Right, 2000.0), Some((1779.0, 221.0)));
        let l = layout(&[], &[], &s, &reg, &env(M, 2000.0));
        assert_eq!((l.span(Side::Left, 2000.0), l.span(Side::Right, 2000.0)), (None, None), "no wings");
    }

    #[test]
    fn only_a_wing_that_looks_different_counts_as_changed() {
        let reg = fixture_registry();
        let mut s = snap();
        let lay =
            |s: &Snapshot| layout(&names(&["spaces", "title"]), &names(&["clickup", "time"]), s, &reg, &env(M, 2000.0));
        let old = lay(&s);
        let both = |new: &Layout, old: Option<&Layout>| (new.changed(old, Side::Left), new.changed(old, Side::Right));
        assert_eq!(both(&old, None), (true, true), "the first layout");
        assert_eq!(both(&lay(&s), Some(&old)), (false, false));
        s.items.get_mut("time").unwrap().text = "20:16".into(); // same width
        assert_eq!(both(&lay(&s), Some(&old)), (false, true));
        s.title = "Terminal / vim".into(); // same width, different text
        assert_eq!(both(&lay(&s), Some(&old)), (true, true));
        let mut s = snap();
        s.title = "Terminal".into(); // narrower: only the left wing changes shape
        let new = lay(&s);
        assert!(new.left_end < old.left_end);
        assert_eq!(both(&new, Some(&old)), (true, false));
    }

    #[test]
    fn which_wing_a_widget_is_in() {
        let reg = fixture_registry();
        let s = snap();
        let l = layout(&names(&["spaces", "clickup"]), &names(&["time"]), &s, &reg, &env(M, 2000.0));
        assert_eq!(l.side_of("clickup"), Some(Side::Left));
        assert_eq!(l.side_of("time"), Some(Side::Right));
        assert_eq!(l.side_of("date"), None, "not in either wing");
        assert_eq!(l.segs(Side::Right).len(), 1);
    }

    #[test]
    fn snapshot_copies_what_drawing_needs() {
        let mut st = State::default();
        st.items.insert("time".into(), item(None, "20:15"));
        st.spaces = vec![space("1", true, 0)];
        st.title = "t".into();
        st.disabled.insert("date".into());
        st.panel = Some(Panel { screen: 1, x0: 10.0, x1: 90.0 });
        let mut s = Snapshot::default();
        s.items.insert("stale".into(), item(None, "x"));
        s.fill(&st);
        assert_eq!(s.items, st.items);
        assert_eq!(s.panel, st.panel);
        assert_eq!((s.spaces, s.title, s.disabled), (st.spaces, st.title, st.disabled));
    }

    #[test]
    fn a_panel_flush_with_a_wing_squares_its_inner_corner() {
        // left wing 0..300, right wing 1500..2000
        let sq = |p| square_corners(300.0, 1500.0, 2000.0, p);
        assert_eq!(sq(None), (false, false), "no panel");
        assert_eq!(sq(Some((1500.0, 1780.0))), (false, true), "under the right wing's first widget");
        assert_eq!(sq(Some((1501.0, 1781.0))), (false, true), "whole points: within a point counts");
        assert_eq!(sq(Some((1600.0, 1880.0))), (false, false), "in the middle of the wing");
        assert_eq!(sq(Some((1720.0, 2000.0))), (false, false), "at the screen edge: that corner is square anyway");
        assert_eq!(sq(Some((20.0, 300.0))), (true, false), "under the left wing's last widget");
        assert_eq!(sq(Some((0.0, 280.0))), (false, false), "at the left screen edge");
        assert_eq!(sq(Some((700.0, 980.0))), (false, false), "between the wings");
        // no wing, no corner
        assert_eq!(square_corners(0.0, 2000.0, 2000.0, Some((0.0, 2000.0))), (false, false));
    }

    #[test]
    fn only_the_wing_a_panel_hangs_from_on_its_own_screen_changes() {
        let reg = fixture_registry();
        let mut s = snap();
        let lay = |s: &Snapshot| layout(&names(&["spaces"]), &names(&["clickup", "time"]), s, &reg, &env(M, 2000.0));
        let old = lay(&s);
        assert!(!old.square_left && !old.square_right);
        s.panel = Some(Panel { screen: 1, x0: old.right_start.round(), x1: old.right_start.round() + 280.0 });
        let new = lay(&s);
        assert!(new.square_right && !new.square_left);
        assert_eq!((new.changed(Some(&old), Side::Left), new.changed(Some(&old), Side::Right)), (false, true));
        s.panel = Some(Panel { screen: 2, ..s.panel.unwrap() });
        assert!(!lay(&s).square_right, "a panel on another screen leaves this bar alone");
    }
}
