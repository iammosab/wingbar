//! Reading a version 2 settings document into a `Config`, leniently: one bad
//! value warns and falls back to its default; nothing drops the whole file.
//! Keys starting with `//` are comments, anywhere. Unknown keys warn.
//!
//! Pure apart from `probe` (does this path exist?), which finds the window
//! manager and BarPopover's binary.

use super::model::*;
use super::{expand, expand_path};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

/// Durations below this are raised to it.
pub const MIN_EVERY_MS: u64 = 500;

pub const DEFAULT_PORT: u16 = 7776;
pub const DEFAULT_POPOVER_DAEMON: &str = "127.0.0.1:7779";
pub const DEFAULT_SCRIPT_EVERY_MS: u64 = 10_000;
pub const DEFAULT_SCRIPT_TIMEOUT_MS: u64 = 30_000;
pub const ACTIVITY_MONITOR: &str = "open -a 'Activity Monitor'";
pub const CALENDAR: &str = "open -a Calendar";
/// The battery's built-in click action.
pub const CAFFEINATE: &str = "caffeinate";

/// simple-bar's palette, the defaults for `theme.colors`.
pub const PALETTE: &[(&str, &str)] = &[
    ("main", "#2c2e34"),
    ("main-alt", "#a7aab4"),
    ("minor", "#414550"),
    ("accent", "#fc5d7c"),
    ("red", "#fc5d7c"),
    ("green", "#9ed072"),
    ("yellow", "#e7c664"),
    ("orange", "#f39660"),
    ("blue", "#76cce0"),
    ("magenta", "#b39df3"),
    ("cyan", "#76cce0"),
    ("black", "#181819"),
    ("white", "#e2e2e3"),
    ("foreground", "#e2e2e3"),
    ("background", "#2c2e34"),
];

/// The wing colours' defaults, by theme key.
pub const THEME_DEFAULTS: &[(&str, &str)] = &[
    ("wing", "#000000"),
    ("wingHover", "#161618"),
    ("text", "#f2f2f2"),
    ("dim", "#8e8e93"),
    ("faint", "#7c7c82"),
    ("warn", "#ff9f45"),
    ("alert", "#ff6b7a"),
];

const TOP_KEYS: &[&str] =
    &["version", "windowManager", "aerospacePath", "yabaiPath", "bar", "theme", "left", "right", "widgets"];
const BAR_KEYS: &[&str] = &[
    "fontSize",
    "titleMaxWidth",
    "windowLevel",
    "volumeStep",
    "port",
    "server",
    "hideEmptySpaces",
    "spacesExclusions",
    "popoverBinary",
    "popoverDaemon",
];

/// A widget key: `[a-z0-9_-]+`.
pub fn valid_key(k: &str) -> bool {
    !k.is_empty() && k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn is_comment(k: &str) -> bool {
    k.starts_with("//")
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() { key.to_string() } else { format!("{path}.{key}") }
}

/// Collects warnings, with the dotted path of the value they are about.
struct Reader<'w> {
    w: &'w mut Vec<String>,
}

impl Reader<'_> {
    fn invalid(&mut self, path: &str, v: &Value, expected: &str) {
        self.w.push(format!("invalid {path} = {v} (expected {expected}), using the default"));
    }

    /// Warn about every key of `m` that isn't in `known` (comments aside).
    fn unknown(&mut self, m: &Map<String, Value>, known: &[&str], path: &str) {
        for k in m.keys().filter(|k| !is_comment(k) && !known.contains(&k.as_str())) {
            let at = if path.is_empty() { String::new() } else { format!(" in {path}") };
            self.w.push(format!("unknown key {k:?}{at} (ignored)"));
        }
    }

    /// `m[key]` read with `f`; a value `f` can't read warns (as `expected`) and reads as `None`.
    fn get<T>(
        &mut self,
        m: &Map<String, Value>,
        key: &str,
        path: &str,
        expected: &str,
        f: impl Fn(&Value) -> Option<T>,
    ) -> Option<T> {
        let v = m.get(key)?;
        let r = f(v);
        if r.is_none() {
            self.invalid(&join(path, key), v, expected);
        }
        r
    }

    fn string(&mut self, m: &Map<String, Value>, key: &str, path: &str) -> Option<String> {
        self.get(m, key, path, "a string", |v| v.as_str().map(String::from))
    }

    fn bool(&mut self, m: &Map<String, Value>, key: &str, path: &str) -> Option<bool> {
        self.get(m, key, path, "true or false", Value::as_bool)
    }

    fn number(&mut self, m: &Map<String, Value>, key: &str, path: &str) -> Option<f64> {
        self.get(m, key, path, "a number", Value::as_f64)
    }

    fn strings(&mut self, m: &Map<String, Value>, key: &str, path: &str) -> Option<Vec<String>> {
        let list = self.get(m, key, path, "a list of strings", |v| v.as_array().cloned())?;
        let mut out = Vec::new();
        for (i, x) in list.iter().enumerate() {
            match x.as_str() {
                Some(s) => out.push(s.to_string()),
                None => self.invalid(&format!("{}[{i}]", join(path, key)), x, "a string"),
            }
        }
        Some(out)
    }

    /// A duration, raised to the 500 ms minimum (with a warning).
    fn duration(&mut self, m: &Map<String, Value>, key: &str, path: &str) -> Option<u64> {
        let ms = self.get(m, key, path, "a duration like \"500ms\", \"15s\", \"2m\" or a number of ms", duration_ms)?;
        if ms < MIN_EVERY_MS {
            self.w.push(format!("{} = {ms} ms is under the 500 ms minimum, using 500ms", join(path, key)));
            return Some(MIN_EVERY_MS);
        }
        Some(ms)
    }

    /// One of `choices`.
    fn choice(
        &mut self,
        m: &Map<String, Value>,
        key: &str,
        path: &str,
        choices: &[&'static str],
    ) -> Option<&'static str> {
        let expected = format!("one of {}", choices.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", "));
        self.get(m, key, path, &expected, |v| choices.iter().copied().find(|c| v.as_str() == Some(c)))
    }

    /// An object section; anything else warns and reads as empty.
    fn section<'a>(&mut self, m: &'a Map<String, Value>, key: &str, path: &str) -> Option<&'a Map<String, Value>> {
        let v = m.get(key)?;
        if v.as_object().is_none() {
            self.invalid(&join(path, key), v, "an object");
        }
        v.as_object()
    }
}

/// `"500ms"`, `"15s"`, `"2m"`, `"1h"`, `"1.5s"`, or a number of milliseconds.
pub fn duration_ms(v: &Value) -> Option<u64> {
    if let Some(n) = v.as_f64() {
        return (n >= 0.0).then_some(n as u64);
    }
    let s = v.as_str()?.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let scale = match unit.trim() {
        "ms" => 1.0,
        "s" => 1000.0,
        "m" => 60_000.0,
        "h" => 3_600_000.0,
        _ => return None,
    };
    Some((n * scale).round() as u64)
}

/// The shortest way to write `ms` as a duration string.
pub fn format_duration(ms: u64) -> String {
    if ms >= 60_000 && ms.is_multiple_of(60_000) {
        format!("{}m", ms / 60_000)
    } else if ms >= 1000 && ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

fn hex(s: &str) -> Rgba {
    Rgba::hex(s).expect("valid default colour")
}

/// The theme with every default.
pub fn default_theme() -> Theme {
    let d = |k: &str| hex(THEME_DEFAULTS.iter().find(|(n, _)| *n == k).expect("theme key").1);
    Theme {
        wing: d("wing"),
        wing_hover: d("wingHover"),
        text: d("text"),
        dim: d("dim"),
        faint: d("faint"),
        warn: d("warn"),
        alert: d("alert"),
        named: PALETTE.iter().map(|(n, c)| (n.to_string(), hex(c))).collect::<HashMap<_, _>>(),
        icons: IconSet::Sf,
        menus: MenuStyle::Wings,
    }
}

/// Filesystem (through `probe`): `given` if it is an existing path (and not a
/// `$(...)` command), else `bin` in the usual install directories, else the bare name.
pub fn find_binary(bin: &str, given: &str, probe: &dyn Fn(&Path) -> bool) -> String {
    let given = expand(given);
    if !given.is_empty() && !given.contains("$(") && probe(Path::new(&given)) {
        return given;
    }
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/run/current-system/sw/bin"] {
        let p = format!("{d}/{bin}");
        if probe(Path::new(&p)) {
            return p;
        }
    }
    bin.to_string()
}

/// AeroSpace, else yabai, else none, by what is installed.
pub fn detect_wm(probe: &dyn Fn(&Path) -> bool) -> Wm {
    let a = find_binary("aerospace", "", probe);
    if a.starts_with('/') {
        return Wm::Aerospace(a);
    }
    let y = find_binary("yabai", "", probe);
    if y.starts_with('/') { Wm::Yabai(y) } else { Wm::None }
}

/// Read a version 2 document. Anything that isn't an object reads as all defaults.
pub fn parse(doc: &Value, probe: &dyn Fn(&Path) -> bool) -> (Config, Vec<String>) {
    let mut w = Vec::new();
    let empty = Map::new();
    let top = doc.as_object().unwrap_or(&empty);
    let mut r = Reader { w: &mut w };
    r.unknown(top, TOP_KEYS, "");
    match top.get("version") {
        Some(v) if v.as_u64() == Some(2) => {}
        Some(v) => r.w.push(format!("version = {v} is not 2; reading the file as version 2")),
        None if top.is_empty() => {}
        None => r.w.push("no \"version\" in the file; reading it as version 2 (add \"version\": 2)".into()),
    }

    let wm = {
        let which = r.choice(top, "windowManager", "", &["auto", "aerospace", "yabai", "none"]).unwrap_or("auto");
        let a = r.string(top, "aerospacePath", "").unwrap_or_default();
        let y = r.string(top, "yabaiPath", "").unwrap_or_default();
        match which {
            "aerospace" => Wm::Aerospace(find_binary("aerospace", &a, probe)),
            "yabai" => Wm::Yabai(find_binary("yabai", &y, probe)),
            "none" => Wm::None,
            _ => detect_wm(probe),
        }
    };

    let theme = read_theme(&mut r, top);
    let bar = read_bar(&mut r, top, probe);
    let widgets = read_widgets(&mut r, top, &theme);
    let (left, right) = read_wings(&mut r, top, &widgets);
    (Config { wm, theme, bar, left, right, widgets }, w)
}

fn read_theme(r: &mut Reader, top: &Map<String, Value>) -> Theme {
    let mut theme = default_theme();
    let empty = Map::new();
    let t = r.section(top, "theme", "").unwrap_or(&empty);
    let mut known = THEME_KEYS.to_vec();
    known.extend(["colors", "icons", "menus"]);
    r.unknown(t, &known, "theme");
    theme.icons = match r.choice(t, "icons", "theme", &["sf", "simple-bar"]) {
        Some("simple-bar") => IconSet::SimpleBar,
        _ => IconSet::Sf,
    };
    theme.menus = match r.choice(t, "menus", "theme", &["wings", "native"]) {
        Some("native") => MenuStyle::Native,
        _ => MenuStyle::Wings,
    };
    if let Some(colors) = r.section(t, "colors", "theme") {
        for (name, v) in colors.iter().filter(|(k, _)| !is_comment(k)) {
            let path = format!("theme.colors.{name}");
            match v.as_str().and_then(Rgba::hex) {
                Some(c) => {
                    theme.named.insert(name.trim_start_matches("--").to_string(), c);
                }
                None => r.invalid(&path, v, "a hex colour like \"#rrggbb\""),
            }
        }
    }
    for key in THEME_KEYS {
        let Some(v) = t.get(*key) else { continue };
        let Some(c) = v.as_str().and_then(|s| theme.color(s)) else {
            r.invalid(&format!("theme.{key}"), v, "a colour: \"#rrggbb\" or a theme.colors name");
            continue;
        };
        let slot = match *key {
            "wing" => &mut theme.wing,
            "wingHover" => &mut theme.wing_hover,
            "text" => &mut theme.text,
            "dim" => &mut theme.dim,
            "faint" => &mut theme.faint,
            "warn" => &mut theme.warn,
            _ => &mut theme.alert,
        };
        *slot = c;
    }
    theme
}

fn read_bar(r: &mut Reader, top: &Map<String, Value>, probe: &dyn Fn(&Path) -> bool) -> Bar {
    let empty = Map::new();
    let b = r.section(top, "bar", "").unwrap_or(&empty);
    r.unknown(b, BAR_KEYS, "bar");
    let p = "bar";
    let font_size = match b.get("fontSize") {
        None | Some(Value::Null) => None,
        Some(_) => r.get(b, "fontSize", p, "a positive number or null", |v| v.as_f64().filter(|f| *f > 0.0)),
    };
    let port = r
        .get(b, "port", p, "a whole number 1-65535", |v| {
            let n = v.as_u64().or_else(|| v.as_f64().filter(|f| *f >= 0.0 && f.fract() == 0.0).map(|f| f as u64))?;
            u16::try_from(n).ok().filter(|&p| p != 0)
        })
        .unwrap_or(DEFAULT_PORT);
    let popover_bin = r
        .string(b, "popoverBinary", p)
        .filter(|s| !s.trim().is_empty())
        .map(|s| expand(&s))
        .filter(|s| probe(Path::new(s)));
    Bar {
        font_size,
        title_max: r.number(b, "titleMaxWidth", p).unwrap_or(420.0),
        level_above_windows: r.choice(b, "windowLevel", p, &["above", "desktop"]).unwrap_or("above") != "desktop",
        volume_step: r.number(b, "volumeStep", p).unwrap_or(5.0),
        port,
        server: r.bool(b, "server", p).unwrap_or(true),
        hide_empty_spaces: r.bool(b, "hideEmptySpaces", p).unwrap_or(true),
        space_exclusions: r.strings(b, "spacesExclusions", p).unwrap_or_default(),
        popover_bin,
        popover_daemon: r.string(b, "popoverDaemon", p).unwrap_or_else(|| DEFAULT_POPOVER_DAEMON.to_string()),
    }
}

fn read_widgets(r: &mut Reader, top: &Map<String, Value>, theme: &Theme) -> Vec<WidgetDef> {
    let Some(ws) = r.section(top, "widgets", "") else { return Vec::new() };
    let mut out = Vec::new();
    for (key, v) in ws.iter().filter(|(k, _)| !is_comment(k)) {
        if let Some(def) = read_widget(r, key, v, theme) {
            out.push(def);
        }
    }
    out
}

/// One definition, or `None` (with a warning) when it has no known type.
fn read_widget(r: &mut Reader, key: &str, v: &Value, theme: &Theme) -> Option<WidgetDef> {
    let path = format!("widgets.{key}");
    if !valid_key(key) {
        r.w.push(format!(
            "widget key {key:?} should use only a-z, 0-9, _ and - (rename it in \"widgets\", \"left\" and \"right\")"
        ));
    }
    let Some(m) = v.as_object() else {
        r.invalid(&path, v, "an object with a \"type\"");
        return None;
    };
    let types = || WidgetType::ALL.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ");
    let Some(ty) = m.get("type") else {
        r.w.push(format!("{path} has no \"type\" (one of: {}); skipped", types()));
        return None;
    };
    let Some(t) = ty.as_str().and_then(WidgetType::parse) else {
        r.w.push(format!("{path}: unknown type {ty} (known: {}); skipped", types()));
        return None;
    };
    let mut known = t.options().to_vec();
    known.push("type");
    r.unknown(m, &known, &path);
    let p = path.as_str();
    let click = |r: &mut Reader, default: &str| r.string(m, "click", p).unwrap_or_else(|| default.to_string());
    let options = match t {
        WidgetType::Spaces => Options::Spaces,
        WidgetType::Title => Options::Title,
        WidgetType::Sound => Options::Sound,
        WidgetType::Spotify => Options::Spotify,
        WidgetType::Music => Options::Music,
        WidgetType::Cpu => Options::Cpu(Sampled {
            every_ms: r.duration(m, "every", p).unwrap_or(2000),
            click: click(r, ACTIVITY_MONITOR),
        }),
        WidgetType::Memory => Options::Memory(Sampled {
            every_ms: r.duration(m, "every", p).unwrap_or(4000),
            click: click(r, ACTIVITY_MONITOR),
        }),
        WidgetType::Battery => Options::Battery(BatteryOptions {
            click: click(r, CAFFEINATE),
            caffeinate_option: r.string(m, "caffeinateOption", p).unwrap_or_default(),
        }),
        WidgetType::Date => Options::Date(click(r, CALENDAR)),
        WidgetType::Time => Options::Time(TimeOptions {
            hour12: r.bool(m, "hour12", p).unwrap_or(false),
            seconds: r.bool(m, "seconds", p).unwrap_or(false),
            click: click(r, ""),
        }),
        WidgetType::Script => Options::Script(Box::new(read_script(r, m, p, theme))),
    };
    Some(WidgetDef { key: key.to_string(), options })
}

fn read_script(r: &mut Reader, m: &Map<String, Value>, p: &str, theme: &Theme) -> ScriptOptions {
    let command = r.string(m, "command", p).unwrap_or_default();
    if command.trim().is_empty() {
        r.w.push(format!("{p} has no \"command\": it shows nothing"));
    }
    let non_empty = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
    let mut levels = Vec::new();
    if let Some(ls) = r.section(m, "levels", p) {
        for (name, v) in ls.iter().filter(|(k, _)| !is_comment(k)) {
            levels.push(read_level(r, name, v, &format!("{p}.levels.{name}"), theme));
        }
    }
    let menu_on = r.choice(m, "menuOn", p, &["click", "hover", "none"]).map(|c| match c {
        "click" => MenuOn::Click,
        "hover" => MenuOn::Hover,
        _ => MenuOn::None,
    });
    let simplebar_ids = r
        .get(m, "simplebarIds", p, "a list of numbers", |v| v.as_array().cloned())
        .map(|ids| {
            ids.iter()
                .filter_map(|x| {
                    let id = x.as_u64().and_then(|n| u32::try_from(n).ok());
                    if id.is_none() {
                        r.invalid(&format!("{p}.simplebarIds"), x, "a whole number");
                    }
                    id
                })
                .collect()
        })
        .unwrap_or_default();
    ScriptOptions {
        command,
        every_ms: r.duration(m, "every", p).unwrap_or(DEFAULT_SCRIPT_EVERY_MS),
        timeout_ms: r.duration(m, "timeout", p).unwrap_or(DEFAULT_SCRIPT_TIMEOUT_MS),
        icon: non_empty(r.string(m, "icon", p)),
        levels,
        clicks: Clicks {
            left: r.string(m, "click", p).unwrap_or_default(),
            right: r.string(m, "rightClick", p).unwrap_or_default(),
            middle: r.string(m, "middleClick", p).unwrap_or_default(),
        },
        menu_on,
        popover: r.bool(m, "popover", p).unwrap_or(false),
        popover_tab: r.bool(m, "popoverTab", p).unwrap_or(false),
        hide_when_empty: r.bool(m, "hideWhenEmpty", p).unwrap_or(true),
        icon_only: r.bool(m, "iconOnly", p).unwrap_or(false),
        grey_when_ok: r.bool(m, "greyWhenOk", p).unwrap_or(false),
        icon_from_label: r.choice(m, "iconFromLabel", p, &["prayer"]).map(|_| LabelIcons::Prayer),
        prelude: non_empty(r.string(m, "prelude", p)).map(|s| expand_path(&s)),
        cache_file: non_empty(r.string(m, "cacheFile", p)).map(|s| expand_path(&s)),
        simplebar_ids,
        raw: r.bool(m, "raw", p).unwrap_or(false),
    }
}

/// `"green"`, or `{ "color": "green", "icon": "wifi" }`. A colour that names
/// nothing warns and is `theme.dim` (as simple-bar's unknown colours were).
fn read_level(r: &mut Reader, name: &str, v: &Value, path: &str, theme: &Theme) -> Level {
    let expected = "a colour, or { \"color\", \"icon\" }";
    let color = |r: &mut Reader, v: &Value, path: &str| match v.as_str().map(|s| theme.color(s)) {
        Some(Some(c)) => Some(c),
        _ => {
            r.w.push(format!(
                "invalid {path} = {v} (expected a colour: \"#rrggbb\", a theme key or a theme.colors name), using dim"
            ));
            Some(theme.dim)
        }
    };
    let (color, icon) = match v {
        Value::String(_) => (color(r, v, path), None),
        Value::Object(o) => {
            r.unknown(o, &["color", "icon"], path);
            let c = o.get("color").and_then(|c| color(r, c, &format!("{path}.color")));
            let icon = r.string(o, "icon", path).filter(|s| !s.trim().is_empty());
            (c, icon)
        }
        _ => {
            r.invalid(path, v, expected);
            (Some(theme.dim), None)
        }
    };
    Level { name: name.to_string(), color, icon }
}

/// The wings: keys that are defined, each placed once. Warns about the rest.
fn read_wings(r: &mut Reader, top: &Map<String, Value>, defs: &[WidgetDef]) -> (Vec<String>, Vec<String>) {
    let defined_in_file: Vec<&str> = top
        .get("widgets")
        .and_then(Value::as_object)
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let mut placed: Vec<(String, Wing)> = Vec::new();
    let mut wings = (Vec::new(), Vec::new());
    for wing in [Wing::Left, Wing::Right] {
        let Some(keys) = r.strings(top, wing.name(), "") else { continue };
        for k in keys {
            if !defs.iter().any(|d| d.key == k) {
                // a definition that was skipped has had its warning already
                if !defined_in_file.contains(&k.as_str()) {
                    r.w.push(format!(
                        "{:?} in \"{}\" has no definition in \"widgets\"; skipped (add it to \"widgets\" or remove it from \"{}\")",
                        k,
                        wing.name(),
                        wing.name()
                    ));
                }
                continue;
            }
            if let Some((_, first)) = placed.iter().find(|(p, _)| *p == k) {
                r.w.push(format!(
                    "{:?} is placed twice (in \"{}\" and \"{}\"); keeping the first",
                    k,
                    first.name(),
                    wing.name()
                ));
                continue;
            }
            placed.push((k.clone(), wing));
            match wing {
                Wing::Left => wings.0.push(k),
                Wing::Right => wings.1.push(k),
            }
        }
    }
    wings
}
