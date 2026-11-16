//! Import from simple-bar (`~/.simplebarrc`) and from wingbar 0.2's optional
//! `config.json` (version 1) into a version 2 settings document, so that the
//! bar looks and behaves as 0.2.0 did with the same files.
//!
//! The import is the one place that knows what 0.2.0 did by default (its wing
//! order, its grey-when-ok and popover lists, its per-name quirks): those were
//! one person's setup built into the code, and now become explicit options in
//! that person's imported file. Nothing else reads `.simplebarrc`, and the
//! import never changes it.

use super::defaults;
use super::expand;
use super::model::WidgetType;
use super::parse::{MIN_EVERY_MS, PALETTE, THEME_DEFAULTS, format_duration, valid_key};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 0.2.0's wing order when config.json had none.
const V020_LEFT: &[&str] = &["spaces", "user:1", "process", "clickup", "adb", "claude"];
const V020_RIGHT: &[&str] =
    &["cpu", "memory", "storage", "mic", "sound", "spotify", "music", "network", "battery", "salat", "date", "time"];
/// 0.2.0's level scripts whose "ok" icon was grey, when config.json didn't say.
const V020_GREY_OK: &[&str] = &["network", "storage", "memory", "adb", "claude"];
/// 0.2.0's widgets with a BarPopover hover menu, when config.json didn't say.
const V020_POPOVERS: &[&str] = &["clickup", "claude", "salat", "adb", "storage", "memory", "network"];
/// Where 0.2.0 looked for BarPopover's binary, when config.json didn't say.
const V020_POPOVER_BIN: &str = "~/.config/simple-bar/popover/barpopover";

/// The keys of a 0.2 config.json.
const V1_KEYS: &[&str] = &[
    "simplebarrc",
    "left",
    "right",
    "greyWhenOk",
    "popovers",
    "colors",
    "fontSize",
    "titleMaxWidth",
    "volumeStep",
    "windowLevel",
    "port",
    "server",
    "popoverDaemon",
    "popoverBinary",
];

/// simple-bar's colour settings → `theme.colors` names.
const SB_COLORS: &[(&str, &str)] = &[
    ("colorMain", "main"),
    ("colorMainAlt", "main-alt"),
    ("colorMinor", "minor"),
    ("colorAccent", "accent"),
    ("colorRed", "red"),
    ("colorGreen", "green"),
    ("colorYellow", "yellow"),
    ("colorOrange", "orange"),
    ("colorBlue", "blue"),
    ("colorMagenta", "magenta"),
    ("colorCyan", "cyan"),
    ("colorBlack", "black"),
    ("colorWhite", "white"),
    ("colorForeground", "foreground"),
    ("colorBackground", "background"),
];

/// simple-bar icon names → SF Symbols.
pub fn sf_symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "Keyboard" => "keyboard",
        "ClickUp" => "timer",
        "Claude" => "sparkle",
        "AndroidStudio" | "Android" => "iphone",
        "Moon" => "moon.stars",
        "Sun" => "sun.max",
        "MicOn" | "Mic" => "mic.fill",
        "MicOff" => "mic.slash.fill",
        "Desktop" | "Storage" => "internaldrive",
        "Memory" | "Ram" => "memorychip",
        "Wifi" => "wifi",
        "WifiOff" => "wifi.slash",
        "Signal" | "Ethernet" => "network",
        "Battery" => "battery.75",
        "Cpu" | "CPU" => "cpu",
        "Volume" | "Sound" => "speaker.wave.2",
        "Music" | "Spotify" => "music.note",
        "Calendar" | "Date" => "calendar",
        "Clock" | "Time" => "clock",
        "Github" | "GitHub" => "chevron.left.forwardslash.chevron.right",
        "Bell" | "Notification" => "bell",
        "Coffee" => "cup.and.saucer",
        "Git" => "arrow.triangle.branch",
        "Lock" => "lock",
        "Heart" => "heart",
        "Star" => "star",
        "Terminal" => "terminal",
        "Download" => "arrow.down.circle",
        "Upload" => "arrow.up.circle",
        "Rss" => "dot.radiowaves.up.forward",
        _ => return None,
    })
}

/// The icon simple-bar showed for a name it (or we) didn't know.
const FALLBACK_ICON: &str = "square.grid.2x2";

// ── reading simple-bar's file: silently, as 0.2 did ──

fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(k))
}
fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    at(v, path)?.as_str()
}
fn b(v: &Value, path: &[&str], default: bool) -> bool {
    at(v, path).and_then(Value::as_bool).unwrap_or(default)
}
/// Any number: fractions truncated, negatives 0 (numbers as strings don't count).
fn n(v: &Value, path: &[&str]) -> Option<u64> {
    let x = at(v, path)?;
    x.as_u64().or_else(|| x.as_f64().map(|f| f as u64))
}

/// A word for the shell: as it is when that's safe, else in single quotes.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_./~:@%+=,-".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn open_app(app: &str) -> String {
    format!("open -a {}", shell_quote(app))
}

fn has_action(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s != "true"
}

/// A simple-bar colour (`--name` or hex) as a v2 colour; `None` if it names nothing.
fn colour(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(name) = s.strip_prefix("--") {
        return PALETTE.iter().any(|(n, _)| *n == name).then(|| name.to_string());
    }
    super::model::Rgba::hex(s)?;
    Some(if s.starts_with('#') { s.to_string() } else { format!("#{s}") })
}

/// A user widget that runs one of the level scripts: `... <dir>/sbw.sh <name> <level> [ttl]`.
#[derive(Debug, PartialEq)]
pub(super) struct SbwCall {
    /// The scripts' directory, as written.
    pub dir: String,
    pub name: String,
    pub level: String,
}

/// Pure: the first word ending in `/sbw.sh` and the name and level after it.
/// `None` (a plain widget) if there is no such word or fewer than two words
/// follow it. Anything after the level (the ttl) is ignored.
pub(super) fn parse_sbw(output: &str) -> Option<SbwCall> {
    let words: Vec<&str> = output.split_whitespace().collect();
    let i = words.iter().position(|w| w.ends_with("/sbw.sh"))?;
    let [script, name, level, ..] = &words[i..] else { return None };
    Some(SbwCall {
        dir: script.trim_end_matches("/sbw.sh").to_string(),
        name: name.to_string(),
        level: level.to_string(),
    })
}

/// One entry of `userWidgets.userWidgetsList`.
struct SbUser<'a> {
    id: u32,
    w: &'a Value,
}

impl SbUser<'_> {
    fn active(&self) -> bool {
        b(self.w, &["active"], false)
    }
    fn str(&self, k: &str) -> &str {
        s(self.w, &[k]).unwrap_or("")
    }
    fn every_ms(&self) -> u64 {
        n(self.w, &["refreshFrequency"]).unwrap_or(10_000).max(MIN_EVERY_MS)
    }
    /// simple-bar's icon itself (wingbar bundles them), else an SF Symbol
    /// like it.
    fn icon(&self, notes: &mut Vec<String>) -> String {
        let name = s(self.w, &["icon"]).unwrap_or("Widget");
        if crate::icons::exists(name) {
            return name.to_string();
        }
        sf_symbol(name).map(String::from).unwrap_or_else(|| {
            if name != "Widget" {
                notes.push(format!("no SF Symbol for simple-bar's icon {name:?}; using {FALLBACK_ICON}"));
            }
            FALLBACK_ICON.to_string()
        })
    }
    fn colour(&self, notes: &mut Vec<String>) -> String {
        let c = s(self.w, &["backgroundColor"]).unwrap_or("--minor");
        colour(c).unwrap_or_else(|| {
            notes.push(format!("user widget {}: colour {c:?} names no colour; using dim (as simple-bar did)", self.id));
            "dim".into()
        })
    }
    fn clicks(&self, def: &mut Map<String, Value>) {
        for (sb, ours) in
            [("onClickAction", "click"), ("onRightClickAction", "rightClick"), ("onMiddleClickAction", "middleClick")]
        {
            let a = self.str(sb);
            if has_action(a) {
                def.insert(ours.into(), json!(a));
            }
        }
    }
}

/// The user widgets, sorted by number (keys that aren't numbers and entries
/// that aren't objects are skipped, as before).
fn user_list(sb: &Value) -> Vec<SbUser<'_>> {
    let mut list: Vec<SbUser> = at(sb, &["userWidgets", "userWidgetsList"])
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, w)| w.is_object())
        .filter_map(|(k, w)| Some(SbUser { id: k.parse().ok()?, w }))
        .collect();
    list.sort_by_key(|u| u.id);
    list
}

/// A level script's variants, gathered under its name.
struct Group<'a> {
    name: String,
    dir: String,
    variants: Vec<(&'a SbUser<'a>, String)>,
}

/// A key from a script's name: lowercase, with anything else made a `-`.
fn key_from(name: &str) -> String {
    let k: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect();
    if k.is_empty() { "script".into() } else { k }
}

/// What the import knows about one widget it defines.
struct Def {
    key: String,
    def: Map<String, Value>,
    /// Shown in 0.2 (a built-in switched on, an active user widget).
    on: bool,
    /// The names 0.2's wing lists used for it.
    names: Vec<String>,
    /// A user widget (shown on the right when 0.2's wings didn't place it).
    user: bool,
}

/// The import. `sb` is the parsed `.simplebarrc` (`Null` if there is none),
/// `v1` a 0.2 config.json to merge. `probe` says whether a path exists (for
/// `_env.sh` and BarPopover's binary). Returns the version 2 document and
/// notes on what was done and what couldn't be converted.
pub fn import_simplebar(sb: &Value, v1: Option<&Value>, probe: &dyn Fn(&Path) -> bool) -> (Value, Vec<String>) {
    let mut notes = Vec::new();
    let empty = Map::new();
    let v1o = v1.and_then(Value::as_object).unwrap_or(&empty);
    for k in v1o.keys().filter(|k| !k.starts_with("//") && !V1_KEYS.contains(&k.as_str())) {
        notes.push(format!("config.json: unknown key {k:?} not imported"));
    }
    let v1_list = |k: &str, notes: &mut Vec<String>| -> Option<Vec<String>> {
        let v = v1o.get(k)?;
        match v.as_array() {
            Some(a) => Some(a.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
            None => {
                notes.push(format!("config.json: {k} = {v} is not a list; using 0.2's default"));
                None
            }
        }
    };

    // ── built-ins ──
    let mut defs: Vec<Def> = Vec::new();
    let builtin = |t: WidgetType, on: bool, opts: Value, names: &[&str]| {
        let mut def = Map::new();
        def.insert("type".into(), json!(t.name()));
        if let Value::Object(o) = opts {
            def.extend(o);
        }
        let mut all = vec![t.name().to_string()];
        all.extend(names.iter().map(|s| s.to_string()));
        Def { key: t.name().into(), def, on, names: all, user: false }
    };
    let w = |k: &str, d: bool| b(sb, &["widgets", k], d);
    let every =
        |section: &str, d: u64| format_duration(n(sb, &[section, "refreshFrequency"]).unwrap_or(d).max(MIN_EVERY_MS));
    defs.push(builtin(WidgetType::Spaces, true, json!({}), &["aerospace", "yabai"]));
    defs.push(builtin(WidgetType::Title, w("processWidget", true), json!({}), &["process", "window"]));
    defs.push(builtin(
        WidgetType::Cpu,
        w("cpuWidget", true),
        json!({
            "every": every("cpuWidgetOptions", 2000),
            "click": open_app(s(sb, &["cpuWidgetOptions", "cpuMonitorApp"]).unwrap_or("Activity Monitor")),
        }),
        &[],
    ));
    defs.push(builtin(
        WidgetType::Memory,
        w("memoryWidget", false),
        json!({
            "every": every("memoryWidgetOptions", 4000),
            "click": open_app(s(sb, &["memoryWidgetOptions", "memoryMonitorApp"]).unwrap_or("Activity Monitor")),
        }),
        &[],
    ));
    let mut battery = json!({
        "click": if b(sb, &["batteryWidgetOptions", "toggleCaffeinateOnClick"], true) { "caffeinate" } else { "" },
    });
    if let Some(o) = s(sb, &["batteryWidgetOptions", "caffeinateOption"]).filter(|o| !o.trim().is_empty()) {
        battery["caffeinateOption"] = json!(o.trim());
    }
    defs.push(builtin(WidgetType::Battery, w("batteryWidget", true), battery, &[]));
    defs.push(builtin(WidgetType::Sound, w("soundWidget", true), json!({}), &[]));
    defs.push(builtin(
        WidgetType::Date,
        w("dateWidget", true),
        json!({ "click": open_app(s(sb, &["dateWidgetOptions", "calendarApp"]).unwrap_or("Calendar")) }),
        &["date-display"],
    ));
    defs.push(builtin(
        WidgetType::Time,
        w("timeWidget", true),
        json!({
            "hour12": b(sb, &["timeWidgetOptions", "hour12"], false),
            "seconds": b(sb, &["timeWidgetOptions", "showSeconds"], false),
        }),
        &[],
    ));
    defs.push(builtin(WidgetType::Spotify, w("spotifyWidget", false), json!({}), &[]));
    defs.push(builtin(WidgetType::Music, w("musicWidget", false), json!({}), &[]));

    // ── user widgets ──
    let users = user_list(sb);
    let mut groups: Vec<Group> = Vec::new();
    let mut plains: Vec<&SbUser> = Vec::new();
    for u in &users {
        match parse_sbw(u.str("output")) {
            Some(call) => match groups.iter_mut().find(|g| g.name == call.name) {
                Some(g) => g.variants.push((u, call.level)),
                None => groups.push(Group { name: call.name, dir: call.dir, variants: vec![(u, call.level)] }),
            },
            None => plains.push(u),
        }
    }
    let mut taken: HashSet<String> = HashSet::new();
    for g in &groups {
        // an active group uses its active variants only (as 0.2); a switched-off one keeps them all
        let on = g.variants.iter().any(|(u, _)| u.active());
        let variants: Vec<&(&SbUser, String)> = g.variants.iter().filter(|(u, _)| u.active() || !on).collect();
        let first = variants[0].0;
        let mut key = key_from(&g.name);
        if let Some(i) = defs.iter().position(|d| d.key == key && !d.user) {
            let t = WidgetType::parse(&key).expect("built-in");
            if on && !matches!(t, WidgetType::Spaces | WidgetType::Title) {
                if defs[i].on {
                    notes.push(format!("your {key:?} script replaces the built-in {key} widget (as in 0.2)"));
                }
                defs.remove(i);
            } else {
                key = format!("{key}-script");
                if on {
                    notes.push(format!("a script can't be named {:?}; imported it as {key:?}, hidden", g.name));
                }
            }
        }
        while taken.contains(&key) {
            key.push_str("-2");
        }
        taken.insert(key.clone());

        let mut def = Map::new();
        def.insert("type".into(), json!("script"));
        // `~` and `$HOME` expanded first (as 0.2 did): quoted, the shell wouldn't
        let dir = expand(&g.dir);
        def.insert("command".into(), json!(format!("bash {}", shell_quote(&format!("{dir}/{}.sh", g.name)))));
        let env = format!("{dir}/_env.sh");
        if probe(Path::new(&env)) {
            def.insert("prelude".into(), json!(env));
        }
        let every = variants.iter().map(|(u, _)| u.every_ms()).min().unwrap_or(10_000);
        def.insert("every".into(), json!(format_duration(every)));
        let icon = first.icon(&mut notes);
        def.insert("icon".into(), json!(icon));
        let mut levels = Map::new();
        for (u, level) in &variants {
            if levels.contains_key(level) {
                notes.push(format!("{key}: user widget {} repeats level {level:?}; the first one is kept", u.id));
                continue;
            }
            let c = u.colour(&mut notes);
            let i = u.icon(&mut notes);
            levels.insert(level.clone(), if i == icon { json!(c) } else { json!({ "color": c, "icon": i }) });
        }
        def.insert("levels".into(), Value::Object(levels));
        first.clicks(&mut def);
        def.insert("cacheFile".into(), json!(format!("$TMPDIR/simple-bar-widgets/{}.out", g.name.replace('/', "-"))));
        def.insert("simplebarIds".into(), json!(variants.iter().map(|(u, _)| u.id).collect::<Vec<_>>()));
        defs.push(Def { key, def, on, names: vec![g.name.clone()], user: true });
    }
    let mut placeholders = Vec::new();
    for u in plains {
        let output = u.str("output");
        let key = format!("user-{}", u.id);
        if output.trim().is_empty() {
            notes.push(format!("user widget {}: no command; not imported", u.id));
            continue;
        }
        if !u.active() && output.trim() == "true" {
            placeholders.push(u.id.to_string());
            continue;
        }
        let mut def = Map::new();
        def.insert("type".into(), json!("script"));
        def.insert("command".into(), json!(output));
        def.insert("every".into(), json!(format_duration(u.every_ms())));
        if !b(u.w, &["noIcon"], false) {
            def.insert("icon".into(), json!(u.icon(&mut notes)));
        }
        def.insert("levels".into(), json!({ "ok": u.colour(&mut notes) }));
        u.clicks(&mut def);
        if !b(u.w, &["hideWhenNoOutput"], true) {
            def.insert("hideWhenEmpty".into(), json!(false));
        }
        def.insert("simplebarIds".into(), json!([u.id]));
        // shown as 0.2 did: the first line as it is, no `level|label`, icon or menu lines
        def.insert("raw".into(), json!(true));
        taken.insert(key.clone());
        defs.push(Def { key, def, on: u.active(), names: vec![format!("user:{}", u.id)], user: true });
    }

    if !placeholders.is_empty() {
        notes.push(format!(
            "user widgets {}: switched-off placeholders (\"true\"); not imported",
            placeholders.join(", ")
        ));
    }

    // ── 0.2's per-name settings, now options ──
    let grey =
        v1_list("greyWhenOk", &mut notes).unwrap_or_else(|| V020_GREY_OK.iter().map(|s| s.to_string()).collect());
    let popover_bin_given = v1o.get("popoverBinary").and_then(Value::as_str);
    let popover_bin = popover_bin_given.unwrap_or(V020_POPOVER_BIN);
    let popover_bin = (!popover_bin.trim().is_empty() && probe(Path::new(&expand(popover_bin)))).then_some(popover_bin);
    if let (Some(given), None) = (popover_bin_given, popover_bin)
        && !given.trim().is_empty()
    {
        notes.push(format!("config.json: popoverBinary {given:?} doesn't exist; not imported"));
    }
    let popovers = match v1_list("popovers", &mut notes) {
        Some(l) => l,
        None if popover_bin.is_some() => V020_POPOVERS.iter().map(|s| s.to_string()).collect(),
        None => vec![],
    };
    for d in defs.iter_mut() {
        let script = d.user;
        let named = |list: &[String]| d.names.iter().any(|n| list.contains(n)) || list.contains(&d.key);
        if script && named(&grey) {
            d.def.insert("greyWhenOk".into(), json!(true));
        }
        if named(&popovers) {
            if script {
                d.def.insert("popover".into(), json!(true));
            } else if d.on {
                notes.push(format!("{}: a BarPopover menu on a built-in widget is not imported", d.key));
            }
        }
        // the quirks 0.2 had for these level scripts, by name
        if script && d.def.contains_key("cacheFile") {
            match d.names[0].as_str() {
                // simple-bar has no icon for these (they used `Desktop`, a
                // monitor): the theme's own, as it drew them
                "storage" => {
                    d.def.insert("icon".into(), json!("Storage"));
                }
                // the theme drew Ethernet as a port, not simple-bar's `Signal`
                // (the messenger's logo)
                "network" => {
                    if let Some(eth) = d.def.get_mut("levels").and_then(|l| l.get_mut("eth"))
                        && eth.get("icon").and_then(Value::as_str) == Some("Signal")
                    {
                        eth["icon"] = json!("Ethernet");
                    }
                }
                "salat" => {
                    d.def.insert("iconFromLabel".into(), json!("prayer"));
                    // BarPopover's "still to tick" tab hangs under it
                    if d.def.get("popover") == Some(&json!(true)) {
                        d.def.insert("popoverTab".into(), json!(true));
                    }
                }
                "memory" => {
                    d.def.insert("icon".into(), json!("Memory"));
                    if let Some(Value::Object(levels)) = d.def.get_mut("levels") {
                        for v in levels.values_mut() {
                            if let Some(c) = v.get("color").cloned() {
                                *v = c;
                            }
                        }
                    }
                }
                "mic" => {
                    d.def.insert("iconOnly".into(), json!(true));
                }
                _ => {}
            }
        }
    }

    // ── the wings ──
    let mut placed: HashSet<String> = HashSet::new();
    let mut place = |names: Vec<String>, wing: &str, explicit: bool, notes: &mut Vec<String>| -> Vec<String> {
        let mut out = Vec::new();
        for name in names {
            let Some(d) = defs.iter().find(|d| d.names.contains(&name) || (d.key == name && !d.user)) else {
                if explicit {
                    notes.push(format!("config.json: {name:?} in {wing} is no widget; skipped"));
                }
                continue;
            };
            if !d.on {
                continue;
            }
            if !placed.insert(d.key.clone()) {
                notes.push(format!("{:?} was placed twice; kept the first", d.key));
                continue;
            }
            out.push(d.key.clone());
        }
        out
    };
    let to_vec = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let (left_names, lx) = v1_list("left", &mut notes).map_or_else(|| (to_vec(V020_LEFT), false), |l| (l, true));
    let (right_names, rx) = v1_list("right", &mut notes).map_or_else(|| (to_vec(V020_RIGHT), false), |l| (l, true));
    let left = place(left_names, "left", lx, &mut notes);
    let right_listed = place(right_names, "right", rx, &mut notes);
    // as 0.2: a user widget the wings don't place goes to the front of the right wing
    let mut right: Vec<String> =
        defs.iter().filter(|d| d.user && d.on && !placed.contains(&d.key)).map(|d| d.key.clone()).collect();
    right.extend(right_listed);

    // ── bar and theme ──
    let mut bar = defaults::bar();
    bar["hideEmptySpaces"] = json!(b(sb, &["spacesDisplay", "hideEmptySpaces"], true));
    let exclusions: Vec<&str> = s(sb, &["spacesDisplay", "spacesExclusions"])
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    bar["spacesExclusions"] = json!(exclusions);
    if let Some(p) = popover_bin {
        bar["popoverBinary"] = json!(p);
    }
    type Check = fn(&Value) -> bool;
    let v1_checks: &[(&str, Check)] = &[
        ("fontSize", |v| v.is_null() || v.as_f64().is_some_and(|f| f > 0.0)),
        ("titleMaxWidth", |v| v.is_number()),
        ("volumeStep", |v| v.is_number()),
        ("windowLevel", |v| matches!(v.as_str(), Some("above" | "desktop"))),
        ("port", |v| {
            v.as_u64().is_some_and(|p| (1..=65535).contains(&p))
                || v.as_f64().is_some_and(|f| f.fract() == 0.0 && (1.0..=65535.0).contains(&f))
        }),
        ("server", |v| v.is_boolean()),
        ("popoverDaemon", |v| v.is_string()),
    ];
    for (k, ok) in v1_checks {
        if let Some(v) = v1o.get(*k) {
            if ok(v) {
                bar[*k] = v.clone();
            } else {
                notes.push(format!("config.json: {k} = {v} is not valid; using the default"));
            }
        }
    }
    let mut colors = Map::new();
    for (sbk, name) in SB_COLORS {
        if let Some(c) = s(sb, &["themes", sbk]).filter(|c| super::model::Rgba::hex(c).is_some()) {
            colors.insert(name.to_string(), json!(c));
        }
    }
    let mut theme = defaults::theme(colors);
    if let Some(c) = v1o.get("colors") {
        match c.as_object() {
            Some(c) => {
                for (k, v) in c {
                    match (THEME_DEFAULTS.iter().any(|(t, _)| t == k), v.as_str().and_then(super::model::Rgba::hex)) {
                        (true, Some(_)) => theme[k] = v.clone(),
                        (true, None) => {
                            notes.push(format!("config.json: colors.{k} = {v} is not a colour; using the default"))
                        }
                        (false, _) => notes.push(format!("config.json: colors.{k} is not a wing colour; not imported")),
                    }
                }
            }
            None => notes.push(format!("config.json: colors = {c} is not an object; using the defaults")),
        }
    }
    // simple-bar's own icons, for the built-in widgets too (and app icons in
    // the workspaces and before the title, as simple-bar showed them)
    theme["icons"] = json!("simple-bar");

    // ── the document ──
    let mut doc = Map::new();
    doc.insert("version".into(), json!(2));
    let wm = s(sb, &["global", "windowManager"]).filter(|w| matches!(*w, "aerospace" | "yabai")).unwrap_or("auto");
    doc.insert("windowManager".into(), json!(wm));
    for k in ["aerospacePath", "yabaiPath"] {
        if let Some(p) = s(sb, &["global", k]).filter(|p| !p.trim().is_empty() && !p.contains("$(")) {
            doc.insert(k.into(), json!(p));
        }
    }
    doc.insert("bar".into(), bar);
    doc.insert("theme".into(), theme);
    doc.insert("left".into(), json!(left));
    doc.insert("right".into(), json!(right));
    let mut widgets = Map::new();
    let shown = left.len() + right.len();
    for k in left.iter().chain(right.iter()) {
        let d = defs.iter().find(|d| &d.key == k).expect("placed key is defined");
        widgets.insert(k.clone(), Value::Object(d.def.clone()));
    }
    for d in &defs {
        if widgets.contains_key(&d.key) {
            continue;
        }
        if !valid_key(&d.key) {
            notes.push(format!("{:?} is not a valid widget name; rename it", d.key));
        }
        widgets.insert(d.key.clone(), Value::Object(d.def.clone()));
    }
    let mut seen = HashSet::new();
    notes.retain(|n| seen.insert(n.clone()));
    notes.insert(0, format!("{} widgets: {} shown, {} hidden", widgets.len(), shown, widgets.len() - shown));
    doc.insert("widgets".into(), Value::Object(widgets));
    (Value::Object(doc), notes)
}

/// Where simple-bar's file is: a 0.2 config.json's `simplebarrc`, else `~/.simplebarrc`.
pub fn simplebarrc_path(v1: Option<&Value>, default_sb: &Path) -> PathBuf {
    v1.and_then(|v| v.get("simplebarrc"))
        .and_then(Value::as_str)
        .map(|p| PathBuf::from(expand(p)))
        .unwrap_or_else(|| default_sb.to_path_buf())
}

/// Filesystem: read `.simplebarrc` (read only) and import it, merging `v1`.
/// A missing or broken `.simplebarrc` imports as empty, with a note.
pub fn import_files(v1: Option<&Value>, default_sb: &Path) -> (Value, Vec<String>) {
    let path = simplebarrc_path(v1, default_sb);
    let mut pre = Vec::new();
    let sb = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            pre.push(format!("{} is not valid JSON ({e}); imported nothing from it", path.display()));
            Value::Null
        }),
        Err(_) => Value::Null,
    };
    let (doc, mut notes) = import_simplebar(&sb, v1, &|p: &Path| p.exists());
    notes.extend(pre);
    (doc, notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::*;
    use crate::config::parse::parse;

    fn no_fs(_: &Path) -> bool {
        false
    }

    fn fixture_sb() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/simplebarrc.json")).unwrap()
    }

    fn variants_sb() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/simplebarrc-variants.json")).unwrap()
    }

    /// Import, then read the result: it must read without a warning.
    fn imported(sb: &Value, v1: Option<&Value>, probe: &dyn Fn(&Path) -> bool) -> (Value, Config, Vec<String>) {
        let (doc, notes) = import_simplebar(sb, v1, probe);
        let (cfg, warnings) = parse(&doc, &no_fs);
        assert!(warnings.is_empty(), "{warnings:?}\n{doc:#}");
        (doc, cfg, notes)
    }

    fn script<'a>(cfg: &'a Config, key: &str) -> &'a ScriptOptions {
        match &cfg.def(key).unwrap_or_else(|| panic!("{key} defined")).options {
            Options::Script(s) => s,
            o => panic!("{key} is {o:?}"),
        }
    }

    fn rgb(hex: &str) -> Rgba {
        Rgba::hex(hex).unwrap()
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn nothing_to_import_gives_simple_bars_defaults_in_0_2s_order() {
        let (doc, cfg, _) = imported(&Value::Null, None, &no_fs);
        // simple-bar's switches: cpu, battery, sound, date, time, title on; memory, players off
        assert_eq!(cfg.left, strings(&["spaces", "title"]));
        assert_eq!(cfg.right, strings(&["cpu", "sound", "battery", "date", "time"]));
        let hidden: Vec<&str> = cfg.hidden().map(|d| d.key.as_str()).collect();
        assert_eq!(hidden, ["memory", "spotify", "music"]);
        assert_eq!(cfg.bar.port, 7776);
        assert!(cfg.bar.server && cfg.bar.level_above_windows && cfg.bar.hide_empty_spaces);
        assert_eq!(cfg.bar.popover_bin, None);
        assert_eq!(cfg.theme.named["red"], rgb("#fc5d7c"));
        assert_eq!(cfg.theme.named.len(), 15);
        assert_eq!(doc["windowManager"], "auto");
        assert_eq!(
            cfg.def("cpu").unwrap().options,
            Options::Cpu(Sampled { every_ms: 2000, click: "open -a 'Activity Monitor'".into() })
        );
        assert_eq!(
            cfg.def("memory").unwrap().options,
            Options::Memory(Sampled { every_ms: 4000, click: "open -a 'Activity Monitor'".into() })
        );
        assert_eq!(
            cfg.def("battery").unwrap().options,
            Options::Battery(BatteryOptions { click: "caffeinate".into(), caffeinate_option: String::new() })
        );
        assert_eq!(cfg.def("date").unwrap().options, Options::Date("open -a Calendar".into()));
        assert_eq!(
            cfg.def("time").unwrap().options,
            Options::Time(TimeOptions { hour12: false, seconds: false, click: String::new() })
        );
    }

    #[test]
    fn fixture_level_widgets_merge_into_one_script_each() {
        let (doc, cfg, _) = imported(&fixture_sb(), None, &no_fs);
        let c = script(&cfg, "clickup");
        assert_eq!(c.command, "bash /nonexistent/wingbar-fixture/clickup.sh");
        assert_eq!(c.prelude, None, "no _env.sh in a directory that doesn't exist");
        assert_eq!(c.every_ms, 10_000, "the shortest interval of its variants");
        assert_eq!(c.icon.as_deref(), Some("ClickUp"), "simple-bar's own icon (bundled)");
        let levels: Vec<(&str, Option<Rgba>, Option<&str>)> =
            c.levels.iter().map(|l| (l.name.as_str(), l.color, l.icon.as_deref())).collect();
        assert_eq!(levels, [("ok", Some(rgb("#00ff00")), None), ("warn", Some(rgb("#ff0000")), None)]);
        assert_eq!(doc["widgets"]["clickup"]["levels"], json!({ "ok": "green", "warn": "red" }));
        assert_eq!(c.clicks.left, "open -a ClickUp", "the first variant's clicks");
        assert_eq!(c.simplebar_ids, [1, 2]);
        assert!(c.cache_file.as_ref().unwrap().ends_with("/simple-bar-widgets/clickup.out"));
        assert!(!c.cache_file.as_ref().unwrap().contains("$TMPDIR"), "expanded when read");
        assert_eq!(doc["widgets"]["clickup"]["cacheFile"], "$TMPDIR/simple-bar-widgets/clickup.out");
        assert!(!c.popover, "no BarPopover: no hover menus");
        assert!(c.hide_when_empty && !c.grey_when_ok && !c.icon_only);

        let salat = script(&cfg, "salat");
        assert_eq!(salat.levels.len(), 1);
        assert_eq!(salat.levels[0].color, Some(rgb("#76cce0")), "--blue keeps its default");
        assert_eq!(salat.every_ms, 500, "intervals are at least 500 ms");
        assert_eq!(salat.icon_from_label, Some(LabelIcons::Prayer), "each prayer's icon, as the theme showed");
    }

    #[test]
    fn fixture_plain_and_switched_off_widgets() {
        let (doc, cfg, notes) = imported(&fixture_sb(), None, &no_fs);
        let p = script(&cfg, "user-3");
        assert_eq!(p.command, "echo hello");
        // shown as 0.2 did: line 1 as it is, no level|label, icon: or menu: lines
        assert_eq!(doc["widgets"]["user-3"]["raw"], true);
        assert!(["user-4", "user-11"].iter().all(|k| doc["widgets"][k]["raw"] == true));
        assert!(doc["widgets"]["clickup"].get("raw").is_none(), "a level script keeps the protocol");
        assert_eq!(p.icon.as_deref(), Some("Terminal"));
        assert_eq!(p.levels[0].color, Some(rgb("#123456")));
        assert_eq!(p.every_ms, 5000);
        assert!(!p.hide_when_empty);
        assert_eq!(p.simplebar_ids, [3]);
        let p = script(&cfg, "user-11");
        assert_eq!(p.icon, None, "noIcon");
        assert!(p.hide_when_empty);
        assert_eq!(p.every_ms, 10_000);
        // --minor in the fixture is not a colour, so it keeps its default
        assert_eq!(p.levels[0].color, Some(rgb("#414550")));
        assert_eq!(cfg.theme.named["minor"], rgb("#414550"));
        // a switched-off user widget is defined, hidden
        assert_eq!(script(&cfg, "user-4").command, "echo inactive");
        assert_eq!(cfg.wing_of("user-4"), None);
        assert!(notes[0].contains("shown"), "{notes:?}");
        assert!(doc["widgets"]["user-4"].is_object());
    }

    #[test]
    fn fixture_wings_follow_0_2_defaults() {
        let (_, cfg, _) = imported(&fixture_sb(), None, &no_fs);
        // 0.2's default order; user widgets it doesn't place go to the front of the right wing
        assert_eq!(cfg.left, strings(&["spaces", "title", "clickup"]));
        assert_eq!(cfg.right, strings(&["user-3", "user-11", "sound", "music", "battery", "salat", "date", "time"]));
        assert!(matches!(cfg.wm, Wm::Aerospace(_)));
        assert_eq!(cfg.theme.named["red"], rgb("#ff0000"));
        assert!(cfg.def("cpu").is_some() && cfg.wing_of("cpu").is_none(), "switched off: hidden");
        assert_eq!(
            cfg.def("cpu").unwrap().options,
            Options::Cpu(Sampled { every_ms: 500, click: "open -a 'Activity Monitor'".into() })
        );
        assert_eq!(
            cfg.def("time").unwrap().options,
            Options::Time(TimeOptions { hour12: true, seconds: false, click: String::new() })
        );
        assert_eq!(cfg.bar.space_exclusions, strings(&["9", "scratch"]));
    }

    #[test]
    fn v1_config_is_merged() {
        let v1 = json!({
            "//": "a comment",
            "left": ["spaces", "clickup", "nothing-here"],
            "right": ["time", "user:3", "date-display", "time"],
            "colors": { "wing": "#111111", "text": "white", "shadow": "#000" },
            "popoverBinary": "/nonexistent/wingbar-fixture/barpopover",
            "windowLevel": "desktop",
            "fontSize": 13,
            "port": 70000,
            "server": false,
            "greyWhenOk": ["salat"],
            "colours": {},
        });
        let (doc, cfg, notes) = imported(&fixture_sb(), Some(&v1), &no_fs);
        assert_eq!(cfg.left, strings(&["spaces", "clickup"]));
        assert_eq!(cfg.right, strings(&["salat", "user-11", "time", "user-3", "date"]));
        assert_eq!(cfg.theme.wing, rgb("#111111"));
        assert_eq!(cfg.theme.text, rgb("#f2f2f2"));
        assert!(!cfg.bar.level_above_windows && !cfg.bar.server);
        assert_eq!(cfg.bar.font_size, Some(13.0));
        assert_eq!(cfg.bar.port, 7776);
        assert_eq!(doc["bar"]["popoverBinary"], "");
        assert!(script(&cfg, "salat").grey_when_ok);
        assert!(!script(&cfg, "clickup").popover, "no BarPopover: no popovers by default");
        for want in [
            "\"nothing-here\" in left",
            "placed twice",
            "colors.text",
            "colors.shadow",
            "port = 70000",
            "\"colours\"",
            "popoverBinary",
        ] {
            assert!(notes.iter().any(|n| n.contains(want)), "{want}: {notes:?}");
        }
        assert!(!notes.iter().any(|n| n.contains("\"//\"")), "{notes:?}");
    }

    #[test]
    fn popovers_and_the_popover_binary() {
        let probe = |p: &Path| p.to_string_lossy().ends_with("/popover/barpopover");
        let (doc, cfg, _) = imported(&fixture_sb(), None, &probe);
        assert_eq!(doc["bar"]["popoverBinary"], "~/.config/simple-bar/popover/barpopover");
        assert!(cfg.bar.popover_bin.is_none(), "the test reads it back without a filesystem");
        assert!(script(&cfg, "clickup").popover && script(&cfg, "salat").popover);
        assert!(script(&cfg, "salat").popover_tab && !script(&cfg, "clickup").popover_tab, "the prayer tab");
        assert!(!script(&cfg, "user-3").popover);
        // an explicit list wins, binary or not
        let v1 = json!({ "popovers": ["user:3"] });
        let (_, cfg, _) = imported(&fixture_sb(), Some(&v1), &no_fs);
        assert!(script(&cfg, "user-3").popover && !script(&cfg, "clickup").popover);
    }

    #[test]
    fn variants_with_custom_levels_icons_and_true_clicks() {
        let probe = |p: &Path| p.to_string_lossy() == "/nonexistent/wb/_env.sh";
        let (doc, cfg, notes) = imported(&variants_sb(), None, &probe);
        let w = &doc["widgets"];
        // the person's shapes: a custom "done" level, per-variant icons, `true` as "no action"
        assert_eq!(
            w["timer"],
            json!({
                "type": "script",
                "command": "bash /nonexistent/wb/timer.sh",
                "prelude": "/nonexistent/wb/_env.sh",
                "every": "15s",
                "icon": "ClickUp",
                "levels": { "ok": "magenta", "done": "green", "dim": "#7f8490" },
                "click": "bash /nonexistent/wb/timer_toggle.sh",
                "rightClick": "bash /nonexistent/wb/timer_open.sh",
                "cacheFile": "$TMPDIR/simple-bar-widgets/timer.out",
                "simplebarIds": [10, 11, 12],
            })
        );
        assert_eq!(
            w["network"]["levels"],
            json!({
                "ok": "#f28fad",
                "dim": { "color": "#7f8490", "icon": "WifiOff" },
                "eth": { "color": "#9ab8f8", "icon": "Ethernet" },
            })
        );
        assert_eq!(
            w["network"]["click"], "open 'x-apple.systempreferences:com.apple.wifi-settings-extension'",
            "first variant's"
        );
        assert!(w["usage"].get("click").is_none(), "`true` is no action");
        assert_eq!(w["usage"]["every"], "150s");
        // 0.2's quirks by name
        assert_eq!(w["memory"]["icon"], "Memory", "wingbar's own icon: simple-bar has none for memory");
        assert_eq!(w["memory"]["levels"], json!({ "ok": "green", "warn": "orange" }), "level icons dropped");
        assert_eq!(w["memory"]["greyWhenOk"], true);
        assert_eq!(w["mic"]["iconOnly"], true);
        assert_eq!(w["mic"]["levels"]["alert"], json!({ "color": "red", "icon": "MicOff" }));
        assert_eq!(w["network"]["greyWhenOk"], true);
        // the memory script replaces the switched-off built-in
        assert_eq!(cfg.def("memory").unwrap().options.widget_type(), WidgetType::Script);
        // retired placeholders are skipped; a switched-off group stays, hidden, with all its levels
        assert!(w.get("user-2").is_none() && w.get("user-3").is_none());
        assert!(notes.iter().any(|n| n.contains("user widgets 2, 3: switched-off placeholders")), "{notes:?}");
        assert!(!notes.iter().any(|n| n.contains("\"Gear\"")), "Gear is bundled: {notes:?}");
        assert_eq!(w["build"]["icon"], "Gear");
        assert_eq!(cfg.wing_of("build"), None);
        assert_eq!(script(&cfg, "build").levels.len(), 2);
        assert_eq!(script(&cfg, "build").simplebar_ids, [90, 91]);
        // a plain one
        assert_eq!(w["user-1"]["command"], "bash /nonexistent/wb/mode.sh");
        assert_eq!(w["user-1"]["levels"], json!({ "ok": "red" }));
        assert_eq!(w["user-1"]["click"], "/opt/homebrew/bin/aerospace mode main");
        assert_eq!(w["user-1"]["icon"], "Keyboard");
        assert_eq!(doc["theme"]["icons"], "simple-bar");
        assert_eq!(cfg.left, strings(&["spaces", "user-1", "title"]));
        assert_eq!(
            cfg.right,
            strings(&["timer", "usage", "cpu", "memory", "mic", "sound", "network", "battery", "date", "time"])
        );
    }

    #[test]
    fn simplebarrc_wrong_types_fall_back_per_field() {
        let sb = json!({
            "themes": "dark",
            "global": ["yabai"],
            "widgets": { "cpuWidget": "no", "memoryWidget": true, "processWidget": 0 },
            "cpuWidgetOptions": { "refreshFrequency": "5000", "cpuMonitorApp": "htop" },
            "memoryWidgetOptions": { "refreshFrequency": 2500.7 },
            "timeWidgetOptions": { "hour12": true, "showSeconds": "yes" },
            "spacesDisplay": { "spacesExclusions": 9, "hideEmptySpaces": false },
            "userWidgets": { "userWidgetsList": {
                "1": { "active": true, "output": "echo a", "icon": 5, "refreshFrequency": -100, "noIcon": "true" },
                "2": "not an object",
                "x": { "active": true, "output": "echo not-a-number-key" },
                "3": { "active": "true", "output": "echo active-must-be-a-bool" },
                "4": { "active": true, "output": ["echo"], "hideWhenNoOutput": null },
            } },
        });
        let (_, cfg, _) = imported(&sb, None, &no_fs);
        assert_eq!(cfg.theme.named["red"], rgb("#fc5d7c"));
        for k in ["cpu", "memory", "title"] {
            assert!(cfg.wing_of(k).is_some(), "{k} on");
        }
        assert_eq!(
            cfg.def("cpu").unwrap().options,
            Options::Cpu(Sampled { every_ms: 2000, click: "open -a htop".into() }),
            "numbers as strings are not numbers"
        );
        assert_eq!(
            cfg.def("memory").unwrap().options,
            Options::Memory(Sampled { every_ms: 2500, click: "open -a 'Activity Monitor'".into() }),
            "fractions are truncated"
        );
        assert_eq!(
            cfg.def("time").unwrap().options,
            Options::Time(TimeOptions { hour12: true, seconds: false, click: String::new() })
        );
        assert!(cfg.bar.space_exclusions.is_empty() && !cfg.bar.hide_empty_spaces);
        let users: Vec<&str> =
            cfg.widgets.iter().filter(|d| d.key.starts_with("user-")).map(|d| d.key.as_str()).collect();
        assert_eq!(users, ["user-1", "user-3"], "placed first, then hidden; no command: not imported");
        let p = script(&cfg, "user-1");
        assert_eq!((p.command.as_str(), p.icon.as_deref(), p.every_ms), ("echo a", Some("Widget"), 500));
        assert_eq!(cfg.wing_of("user-3"), None, "\"true\" as a string is not active");
    }

    #[test]
    fn short_sbw_call_is_a_plain_widget() {
        let sb = json!({ "userWidgets": { "userWidgetsList": {
            "1": { "active": true, "output": "bash /nonexistent/sbw.sh clickup" },
        } } });
        let (_, cfg, _) = imported(&sb, None, &no_fs);
        assert_eq!(script(&cfg, "user-1").command, "bash /nonexistent/sbw.sh clickup");
    }

    #[test]
    fn scripts_cant_take_the_workspaces_or_the_title() {
        let sb = json!({ "userWidgets": { "userWidgetsList": {
            "1": { "active": true, "output": "bash /w/sbw.sh title ok" },
            "2": { "active": false, "output": "bash /w/sbw.sh cpu ok" },
        } } });
        let (_, cfg, notes) = imported(&sb, None, &no_fs);
        assert_eq!(cfg.def("title").unwrap().options, Options::Title);
        assert_eq!(cfg.wing_of("title-script"), Some(Wing::Right), "an active user widget, placed as 0.2 did");
        assert!(notes.iter().any(|n| n.contains("can't be named")), "{notes:?}");
        assert_eq!(cfg.def("cpu").unwrap().options.widget_type(), WidgetType::Cpu, "a switched-off script yields");
        assert!(cfg.def("cpu-script").is_some());
    }

    #[test]
    fn sbw_calls_parse() {
        let call = |dir: &str, name: &str, level: &str| {
            Some(SbwCall { dir: dir.to_string(), name: name.to_string(), level: level.to_string() })
        };
        assert_eq!(parse_sbw("bash /w/sbw.sh clickup ok"), call("/w", "clickup", "ok"));
        assert_eq!(parse_sbw("bash /w/sbw.sh clickup warn 60"), call("/w", "clickup", "warn"));
        assert_eq!(parse_sbw("  X=1   bash\t/w/sbw.sh  salat  ok  "), call("/w", "salat", "ok"));
        assert_eq!(parse_sbw("/w/sbw.sh n l"), call("/w", "n", "l"));
        assert_eq!(parse_sbw("bash ~/w/sbw.sh n l 5"), call("~/w", "n", "l"), "kept as written");
        assert_eq!(parse_sbw("bash $HOME/w/sbw.sh n l"), call("$HOME/w", "n", "l"));
        for plain in [
            "",
            "echo hello",
            "bash /w/sbw.sh",
            "bash /w/sbw.sh clickup",
            "bash sbw.sh clickup ok",
            "bash /w/sbw.sh.bak clickup ok",
        ] {
            assert_eq!(parse_sbw(plain), None, "{plain:?}");
        }
        assert_eq!(parse_sbw("bash /w/sbw.sh x && bash /v/sbw.sh y z").map(|c| c.level), Some("&&".to_string()));
    }

    #[test]
    fn sbw_folders_written_with_home_are_expanded_before_quoting() {
        let home = super::super::home().display().to_string();
        for dir in ["$HOME/x", "${HOME}/x", "~/x"] {
            let sb = json!({ "userWidgets": { "userWidgetsList": {
                "1": { "active": true, "output": format!("bash {dir}/sbw.sh clickup ok") },
            } } });
            let env = format!("{home}/x/_env.sh");
            let (doc, cfg, _) = imported(&sb, None, &|p: &Path| p == Path::new(&env));
            let c = script(&cfg, "clickup");
            assert_eq!(c.command, format!("bash {}", shell_quote(&format!("{home}/x/clickup.sh"))), "{dir}");
            assert!(!c.command.contains('$') && !c.command.contains('~'), "{dir}: {}", c.command);
            assert_eq!(doc["widgets"]["clickup"]["prelude"], json!(env), "{dir}");
        }
    }

    #[test]
    fn icons_and_quoting() {
        assert_eq!(sf_symbol("ClickUp"), Some("timer"));
        assert_eq!(sf_symbol("CPU"), Some("cpu"));
        assert_eq!(sf_symbol("MicOff"), Some("mic.slash.fill"));
        assert_eq!(sf_symbol("NoSuchIcon"), None);
        assert_eq!(shell_quote("Calendar"), "Calendar");
        assert_eq!(shell_quote("Activity Monitor"), "'Activity Monitor'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("/a b/c.sh"), "'/a b/c.sh'");
        assert_eq!(colour("--red").as_deref(), Some("red"));
        assert_eq!(colour("--nope"), None);
        assert_eq!(colour("123456").as_deref(), Some("#123456"));
        assert_eq!(colour("red"), None, "simple-bar needed the --");
    }

    #[test]
    fn importing_twice_gives_the_same_document() {
        let (a, _) = import_simplebar(&variants_sb(), None, &no_fs);
        let (b, _) = import_simplebar(&variants_sb(), None, &no_fs);
        assert_eq!(super::super::doc::to_text(&a), super::super::doc::to_text(&b));
    }
}
