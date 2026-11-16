//! The look of a widget's menu, read from its lines (pure, tested). A plain
//! `menu:` line is what it always was: a reading, or an item when it has a
//! command. A line may also say how it looks:
//!
//! ```text
//! menu: # Memory<TAB>Pressure normal       header: bold title, subtitle right
//! menu: ## Today<TAB>8 runs                section: a bold row (an item with a command)
//! menu: Used<TAB>40.9 GB                   label left, value right
//! menu: ~85 green                          a meter under the row before (0–100, a colour)
//! menu: > Refresh | <command>              a button; buttons in a row share a line
//! menu: _ updated 1m ago                   a caption: small and dim
//! ```
//!
//! A tab is the column break, so text a script copies from elsewhere (stripped
//! of control characters, as the protocol asks) can't make one. In a macOS menu
//! (`theme.menus: "native"`) the same lines become plain items: see `plain`.

use super::menu::{Action, Entry, Menu};

/// How one line wants to look (from its title).
#[derive(Clone, Debug, PartialEq)]
pub enum Style<'a> {
    Header(&'a str, Option<&'a str>),
    Section(&'a str, Option<&'a str>),
    Caption(&'a str),
    Meter(f64, Option<&'a str>),
    Button(&'a str),
    Plain(&'a str, Option<&'a str>),
}

fn columns(s: &str) -> (&str, Option<&str>) {
    match s.split_once('\t') {
        Some((l, r)) => (l.trim(), Some(r.trim()).filter(|r| !r.is_empty())),
        None => (s.trim(), None),
    }
}

/// What a line's title asks for.
pub fn style(title: &str) -> Style<'_> {
    if let Some(r) = title.strip_prefix("## ") {
        let (l, v) = columns(r);
        return Style::Section(l, v);
    }
    if let Some(r) = title.strip_prefix("# ") {
        let (l, v) = columns(r);
        return Style::Header(l, v);
    }
    if let Some(r) = title.strip_prefix("_ ") {
        return Style::Caption(r.trim());
    }
    if let Some(r) = title.strip_prefix("> ") {
        return Style::Button(r.trim());
    }
    if let Some(r) = title.strip_prefix('~') {
        let mut parts = r.split_whitespace();
        if let Some(Ok(pct)) = parts.next().map(str::parse::<f64>)
            && pct.is_finite()
        {
            return Style::Meter(pct.clamp(0.0, 100.0), parts.next());
        }
    }
    let (l, v) = columns(title);
    Style::Plain(l, v)
}

/// A line for a macOS menu, which has no columns, meters or buttons: the
/// title as text (columns joined), `None` for a meter.
pub fn plain(title: &str) -> Option<String> {
    let join = |l: &str, v: Option<&str>| match v {
        Some(v) => format!("{l}   {v}"),
        None => l.to_string(),
    };
    match style(title) {
        Style::Header(l, v) | Style::Section(l, v) | Style::Plain(l, v) => Some(join(l, v)),
        Style::Caption(t) | Style::Button(t) => Some(t.to_string()),
        Style::Meter(..) => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub title: String,
    /// Its action, in the menu's action list.
    pub slot: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Something to choose.
    Item,
    /// A reading.
    Info,
    Section,
    Header,
    Caption,
    /// A meter (0–100) and its colour, as written.
    Meter(f64, Option<String>),
    Buttons(Vec<Button>),
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: Kind,
    pub title: String,
    /// The right-hand column (a header's subtitle).
    pub value: Option<String>,
    /// Its action (items and sections with a command), in the action list.
    pub slot: Option<usize>,
}

impl Row {
    fn new(kind: Kind, title: &str, value: Option<&str>, slot: Option<usize>) -> Row {
        Row { kind, title: title.to_string(), value: value.map(String::from), slot }
    }
}

/// A menu's entries as rows, and their actions (by slot). Buttons next to
/// each other share a row; a meter goes under a reading, item or section; an item without an
/// action (or with a submenu, which isn't shown) is a reading.
pub fn rows_of(menu: Menu) -> (Vec<Row>, Vec<Option<Action>>) {
    let mut rows: Vec<Row> = Vec::new();
    let mut actions: Vec<Option<Action>> = Vec::new();
    for e in menu.entries {
        let it = match e {
            Entry::Separator => {
                rows.push(Row::new(Kind::Separator, "", None, None));
                continue;
            }
            Entry::Item(it) => it,
        };
        let act = it.action.filter(|_| it.enabled && it.submenu.is_none());
        let mut slot = |a: Option<Action>| {
            a.map(|f| {
                actions.push(Some(f));
                actions.len() - 1
            })
        };
        match style(&it.title) {
            Style::Header(l, v) => rows.push(Row::new(Kind::Header, l, v, None)),
            Style::Caption(t) => rows.push(Row::new(Kind::Caption, t, None, None)),
            Style::Meter(pct, color) => {
                // under the reading, item or section it measures
                if rows.last().is_some_and(|r| matches!(r.kind, Kind::Item | Kind::Info | Kind::Section)) {
                    rows.push(Row::new(Kind::Meter(pct, color.map(String::from)), "", None, None));
                }
            }
            Style::Button(t) => {
                let b = Button { title: t.to_string(), slot: slot(act) };
                match rows.last_mut() {
                    Some(Row { kind: Kind::Buttons(bs), .. }) => bs.push(b),
                    _ => rows.push(Row::new(Kind::Buttons(vec![b]), "", None, None)),
                }
            }
            Style::Section(l, v) => {
                let s = slot(act);
                rows.push(Row::new(Kind::Section, l, v, s));
            }
            Style::Plain(l, v) => {
                let s = slot(act);
                let kind = if s.is_some() { Kind::Item } else { Kind::Info };
                rows.push(Row::new(kind, l, v, s));
            }
        }
    }
    (rows, actions)
}

/// A typeface a row is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Regular,
    Bold,
    Small,
}

/// Sizes for a bar's metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dims {
    pub row_h: f64,
    pub header_h: f64,
    pub caption_h: f64,
    pub meter_h: f64,
    pub button_h: f64,
    pub sep_h: f64,
    /// Above the first row and below the last.
    pub pad_y: f64,
    /// Left and right of a row's text.
    pub pad_x: f64,
    /// Between a label and its value.
    pub gap: f64,
    /// The `›` after a clickable row's value.
    pub chevron: f64,
    /// Left and right of a button's title.
    pub button_pad: f64,
    pub min_w: f64,
    pub max_w: f64,
    /// The radius of the panel's bottom corners.
    pub corner: f64,
    /// The concave fillets joining panel and wing.
    pub fillet: f64,
}

impl Dims {
    pub fn new(font: f64, pad: f64, corner: f64) -> Dims {
        let row_h = (font * 2.0).round();
        Dims {
            row_h,
            header_h: (row_h * 1.2).round(),
            caption_h: (row_h * 0.85).round(),
            meter_h: 8.0,
            button_h: (row_h * 1.15).round(),
            sep_h: 9.0,
            pad_y: 5.0,
            pad_x: pad + 5.0,
            gap: 24.0,
            chevron: (font * 1.1).round(),
            button_pad: 9.0,
            min_w: 150.0,
            max_w: 440.0,
            corner: (corner * 0.8).round(),
            fillet: (corner * 0.6).round(),
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

/// The face a row's title is drawn in.
pub fn title_face(k: &Kind) -> Face {
    match k {
        Kind::Header | Kind::Section => Face::Bold,
        Kind::Caption => Face::Small,
        _ => Face::Regular,
    }
}

/// The face a row's value is drawn in.
pub fn value_face(k: &Kind) -> Face {
    match k {
        Kind::Header => Face::Small,
        Kind::Section | Kind::Info | Kind::Item => Face::Bold,
        _ => Face::Regular,
    }
}

/// Lay rows out: as wide as the widest row (between `min_w`, or the widget's
/// own width when wider, and `max_w`), rows stacked between the paddings.
pub fn geometry(rows: &[Row], text_w: &dyn Fn(&str, Face) -> f64, d: &Dims, anchor_w: f64) -> Geometry {
    let need = |r: &Row| -> f64 {
        match &r.kind {
            Kind::Separator | Kind::Meter(..) => 0.0,
            Kind::Buttons(bs) => bs.iter().map(|b| text_w(&b.title, Face::Regular) + d.button_pad * 2.0).sum::<f64>(),
            k => {
                let v = r.value.as_deref().map_or(0.0, |v| d.gap + text_w(v, value_face(k)));
                let chev = if *k == Kind::Item && r.value.is_some() { d.chevron } else { 0.0 };
                text_w(&r.title, title_face(k)) + v + chev + d.pad_x * 2.0
            }
        }
    };
    let widest = rows.iter().map(need).fold(0.0, f64::max);
    let w = widest.max(d.min_w).max(anchor_w).min(d.max_w).ceil();
    let mut y = d.pad_y;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let h = match r.kind {
            Kind::Separator => d.sep_h,
            Kind::Header => d.header_h,
            Kind::Caption => d.caption_h,
            Kind::Meter(..) => d.meter_h,
            Kind::Buttons(_) => d.button_h,
            _ => d.row_h,
        };
        out.push((y, h));
        y += h;
    }
    Geometry { w, h: (y + d.pad_y).ceil(), rows: out }
}

/// Something under the pointer that can be chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    /// A row of buttons, and which button.
    Button(usize, usize),
}

/// What can be chosen at (`x`, `y`) of the panel body (top down), if anything.
pub fn hit(rows: &[Row], g: &Geometry, x: f64, y: f64) -> Option<Hit> {
    if x < 0.0 || x > g.w {
        return None;
    }
    let i = g.rows.iter().position(|&(top, h)| y >= top && y < top + h)?;
    match &rows[i].kind {
        Kind::Item | Kind::Section if rows[i].slot.is_some() => Some(Hit::Row(i)),
        Kind::Buttons(bs) if !bs.is_empty() => {
            let j = ((x / (g.w / bs.len() as f64)) as usize).min(bs.len() - 1);
            bs[j].slot.is_some().then_some(Hit::Button(i, j))
        }
        _ => None,
    }
}

/// The action slot a hit chooses.
pub fn slot_of(rows: &[Row], h: Hit) -> Option<usize> {
    match h {
        Hit::Row(i) => rows.get(i)?.slot,
        Hit::Button(i, j) => match &rows.get(i)?.kind {
            Kind::Buttons(bs) => bs.get(j)?.slot,
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_says_how_it_looks() {
        assert_eq!(style("Used\t40.9 GB"), Style::Plain("Used", Some("40.9 GB")));
        assert_eq!(style("Open Actions"), Style::Plain("Open Actions", None));
        assert_eq!(style("# Memory\tPressure normal"), Style::Header("Memory", Some("Pressure normal")));
        assert_eq!(style("## Today\t8 runs"), Style::Section("Today", Some("8 runs")));
        assert_eq!(style("_ updated 1m ago"), Style::Caption("updated 1m ago"));
        assert_eq!(style("> Refresh"), Style::Button("Refresh"));
        assert_eq!(style("~85 green"), Style::Meter(85.0, Some("green")));
        assert_eq!(style("~140"), Style::Meter(100.0, None), "clamped");
        // what plain menus already had keeps its look
        assert_eq!(style("#15065 Fix the build"), Style::Plain("#15065 Fix the build", None));
        assert_eq!(style("~ approx"), Style::Plain("~ approx", None));
        assert_eq!(style("Used\t"), Style::Plain("Used", None), "an empty value is none");
    }

    #[test]
    fn a_macos_menu_gets_plain_text() {
        assert_eq!(plain("Used\t40.9 GB").as_deref(), Some("Used   40.9 GB"));
        assert_eq!(plain("# Memory\tPressure normal").as_deref(), Some("Memory   Pressure normal"));
        assert_eq!(plain("> Refresh").as_deref(), Some("Refresh"));
        assert_eq!(plain("_ note").as_deref(), Some("note"));
        assert_eq!(plain("~40 red"), None);
    }

    fn menu() -> Menu {
        let mut m = Menu::new();
        m.info("# Memory\tPressure normal")
            .info("~30 green")
            .info("Used\t40.9 GB")
            .info("~85 green")
            .item("## Top apps\t6", || {})
            .item("Safari\t2.2 GB", || {})
            .separator()
            .info("_ helpers included")
            .item("> Activity Monitor", || {})
            .info("> Disabled")
            .item("> Refresh", || {});
        m
    }

    #[test]
    fn lines_become_rows_and_buttons_share_one() {
        let (rows, actions) = rows_of(menu());
        let kinds: Vec<&Kind> = rows.iter().map(|r| &r.kind).collect();
        assert_eq!(kinds[0], &Kind::Header);
        assert_eq!(kinds[1], &Kind::Info, "a meter right after a header is dropped");
        assert_eq!(kinds[2], &Kind::Meter(85.0, Some("green".into())));
        assert_eq!((kinds[3], rows[3].value.as_deref(), rows[3].slot), (&Kind::Section, Some("6"), Some(0)));
        assert_eq!((kinds[4], rows[4].slot), (&Kind::Item, Some(1)));
        assert_eq!(kinds[5], &Kind::Separator);
        assert_eq!(kinds[6], &Kind::Caption);
        let Kind::Buttons(bs) = kinds[7] else { panic!("buttons") };
        assert_eq!(
            bs.iter().map(|b| (b.title.as_str(), b.slot)).collect::<Vec<_>>(),
            [("Activity Monitor", Some(2)), ("Disabled", None), ("Refresh", Some(3))]
        );
        assert_eq!(rows.len(), 8);
        assert_eq!(actions.len(), 4);
    }

    const D: Dims = Dims {
        row_h: 28.0,
        header_h: 34.0,
        caption_h: 24.0,
        meter_h: 8.0,
        button_h: 32.0,
        sep_h: 9.0,
        pad_y: 5.0,
        pad_x: 16.0,
        gap: 24.0,
        chevron: 15.0,
        button_pad: 9.0,
        min_w: 150.0,
        max_w: 440.0,
        corner: 14.0,
        fillet: 11.0,
    };
    fn w(s: &str, f: Face) -> f64 {
        s.chars().count() as f64 * if f == Face::Small { 6.0 } else { 7.0 }
    }

    #[test]
    fn each_kind_has_its_height_and_columns_add_up() {
        let (rows, _) = rows_of(menu());
        let g = geometry(&rows, &w, &D, 80.0);
        let hs: Vec<f64> = g.rows.iter().map(|r| r.1).collect();
        assert_eq!(hs, [34.0, 28.0, 8.0, 28.0, 28.0, 9.0, 24.0, 32.0]);
        assert_eq!(g.h, 5.0 + hs.iter().sum::<f64>() + 5.0);
        // widest: the buttons, each its title and padding: (112 + 18) + (56 + 18) + (49 + 18)
        assert_eq!(g.w, 271.0);
        // a header: "Memory" + gap + "Pressure normal" (small) + padding
        let mut m = Menu::new();
        m.info("# Memory\tPressure normal");
        let (rows, _) = rows_of(m);
        assert_eq!(geometry(&rows, &w, &D, 0.0).w, 42.0 + 24.0 + 90.0 + 32.0);
        // an item with a value leaves room for its chevron
        let mut m = Menu::new();
        m.item("Runner\tonline · idle", || {});
        let (rows, _) = rows_of(m);
        assert_eq!(geometry(&rows, &w, &D, 0.0).w, 42.0 + 24.0 + 91.0 + 15.0 + 32.0);
    }

    #[test]
    fn rows_and_buttons_can_be_hit() {
        let (rows, _) = rows_of(menu());
        let g = geometry(&rows, &w, &D, 300.0);
        assert_eq!(g.w, 300.0);
        let y = |i: usize| g.rows[i].0 + 2.0;
        assert_eq!(hit(&rows, &g, 50.0, y(0)), None, "the header");
        assert_eq!(hit(&rows, &g, 50.0, y(3)), Some(Hit::Row(3)), "a section with a command");
        assert_eq!(hit(&rows, &g, 50.0, y(4)), Some(Hit::Row(4)));
        assert_eq!(hit(&rows, &g, 50.0, y(2)), None, "a meter");
        assert_eq!(hit(&rows, &g, 10.0, y(7)), Some(Hit::Button(7, 0)));
        assert_eq!(hit(&rows, &g, 150.0, y(7)), None, "the button without a command");
        assert_eq!(hit(&rows, &g, 299.0, y(7)), Some(Hit::Button(7, 2)));
        assert_eq!(hit(&rows, &g, 301.0, y(7)), None, "outside");
        assert_eq!(slot_of(&rows, Hit::Button(7, 2)), Some(3));
        assert_eq!(slot_of(&rows, Hit::Row(4)), Some(1));
    }
}
