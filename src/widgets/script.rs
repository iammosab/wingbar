//! Script widgets: a command run through bash. Line 1 of its output is
//! `level|label` or just a label (level "ok"); the level picks the colours and
//! may pick the icon (the `levels` option). Later lines may set the icon for
//! this run (`icon: <SF Symbol>`) and describe a menu (`menu: <title> | <command>`,
//! `menu: <title>`, `menu: ---`). With `raw`, the first line is the label as it
//! is, and nothing else is read. Everything particular about one widget (an
//! icon, icon-only, grey when ok, the prayer icon) is an option of its definition.

use super::{Button, Failure, Show, Widget, has_action, item, run_action};
use crate::config::import::shell_quote;
use crate::config::{Clicks, IconSet, LabelIcons, Level, MenuOn, Rgba, ScriptOptions, Theme};
use crate::sched::{self, Every, Limit, run_shell_limited};
use crate::state::Item;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

/// The most of a script's output that is read; a script printing more is stopped.
pub const OUTPUT_LIMIT: Limit = Limit { lines: 64, bytes: 16 * 1024 };

/// One `menu:` line of a script's output.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuLine {
    /// `menu: <title> | <command>`: runs the command (through bash) when chosen.
    Action { title: String, command: String },
    /// `menu: <title>`: shown, not clickable.
    Info(String),
    /// `menu: ---`
    Separator,
}

/// A script's output, read with the output protocol.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptOutput {
    /// Line 1, trimmed (what the cache file keeps).
    pub line: String,
    /// "ok" when line 1 has no `|` (or is empty).
    pub level: String,
    pub label: String,
    /// From an `icon:` line (the first one that names an icon).
    pub icon: Option<String>,
    pub menu: Vec<MenuLine>,
}

impl ScriptOutput {
    /// Read the output protocol: line 1 is `level|label` or a label; then
    /// `icon:` and `menu:` lines, in any order. Other lines are ignored, and so
    /// is anything past the 64th line.
    pub fn parse(out: &str) -> ScriptOutput {
        let mut lines = out.lines().take(OUTPUT_LIMIT.lines);
        let line = lines.next().unwrap_or("").trim().to_string();
        let (level, label) = parse_level_line(&line).unwrap_or_else(|| ("ok".into(), String::new()));
        let mut o = ScriptOutput { line, level, label, icon: None, menu: Vec::new() };
        for l in lines {
            let l = l.trim();
            if let Some(name) = l.strip_prefix("icon:") {
                let name = name.trim();
                if o.icon.is_none() && !name.is_empty() {
                    o.icon = Some(name.to_string());
                }
            } else if let Some(m) = l.strip_prefix("menu:") {
                o.menu.extend(parse_menu_line(m));
            }
        }
        o
    }

    /// A `raw` script's output: the first line of the trimmed output is the
    /// label, as it is (a `|` in it is text), level "ok"; nothing else is read.
    pub fn parse_raw(out: &str) -> ScriptOutput {
        let line = out.trim().lines().next().unwrap_or("").trim().to_string();
        ScriptOutput { label: line.clone(), line, level: "ok".into(), icon: None, menu: Vec::new() }
    }
}

/// What follows `menu:`; `None` for a line with no title.
fn parse_menu_line(m: &str) -> Option<MenuLine> {
    let m = m.trim();
    if m == "---" {
        return Some(MenuLine::Separator);
    }
    // the first `|` ends the title: the command may have more
    let (title, command) = match m.split_once('|') {
        Some((t, c)) => (t.trim(), c.trim()),
        None => (m, ""),
    };
    if title.is_empty() {
        return None;
    }
    Some(if command.is_empty() {
        MenuLine::Info(title.to_string())
    } else {
        MenuLine::Action { title: title.to_string(), command: command.to_string() }
    })
}

/// When a script widget's menu opens: as set, else on click unless it has a
/// click command of its own.
pub fn menu_on(set: Option<MenuOn>, has_click: bool) -> MenuOn {
    set.unwrap_or(if has_click { MenuOn::None } else { MenuOn::Click })
}

pub struct ScriptWidget {
    key: String,
    /// The command as run, with its prelude.
    command: String,
    every_ms: u64,
    timeout: Duration,
    clicks: Clicks,
    /// The simple-bar user widget numbers it answers to.
    ids: Vec<u32>,
    icon: Option<String>,
    levels: Vec<Level>,
    hide_when_empty: bool,
    icon_only: bool,
    grey_when_ok: bool,
    icon_from_label: Option<LabelIcons>,
    /// Where line 1 is copied after each run (other tools read it).
    cache_file: Option<PathBuf>,
    theme: Theme,
    /// `menuOn` as set (`None`: decided by the click command).
    menu_on: Option<MenuOn>,
    /// The output is read as it is, not with the protocol (`raw`).
    raw: bool,
    /// The menu the last run printed. Kept here, not in the state: a new menu
    /// changes nothing on screen, and it goes away with the widget.
    menu: Mutex<Vec<MenuLine>>,
}

impl ScriptWidget {
    pub fn new(key: &str, o: &ScriptOptions, theme: &Theme) -> ScriptWidget {
        let command = match &o.prelude {
            Some(p) => format!("source {} >/dev/null 2>&1; {}", shell_quote(p), o.command),
            None => o.command.clone(),
        };
        ScriptWidget {
            key: key.to_string(),
            command,
            every_ms: o.every_ms,
            timeout: Duration::from_millis(o.timeout_ms),
            clicks: o.clicks.clone(),
            ids: o.simplebar_ids.clone(),
            icon: o.icon.clone(),
            levels: o.levels.clone(),
            hide_when_empty: o.hide_when_empty,
            icon_only: o.icon_only,
            grey_when_ok: o.grey_when_ok,
            icon_from_label: o.icon_from_label,
            cache_file: o.cache_file.as_ref().map(PathBuf::from),
            theme: theme.clone(),
            menu_on: o.menu_on,
            raw: o.raw,
            menu: Mutex::new(Vec::new()),
        }
    }

    /// What the bar shows for the script's output (without running anything).
    fn present(&self, out: &ScriptOutput) -> Show {
        let t = &self.theme;
        let (level, label) = (out.level.clone(), out.label.clone());
        // icon-only widgets show for any output; the others need a label
        let empty = if self.icon_only { out.line.is_empty() } else { label.is_empty() };
        if empty && self.hide_when_empty {
            return Show::Hide;
        }
        let this = self.levels.iter().find(|l| l.name == level);
        let from_label = self.icon_from_label.and_then(|rule| label_icon(rule, &label, t.icons));
        // this run's `icon:` line, the label's rule, the level's icon, the widget's
        let icon = out
            .icon
            .clone()
            .or_else(|| from_label.map(|(icon, _)| icon.to_string()))
            .or_else(|| this.and_then(|l| l.icon.clone()))
            .or_else(|| self.icon.clone());
        let (ic, tc) = match level.as_str() {
            "warn" => (t.warn, t.warn),
            "alert" => (t.alert, t.alert),
            "dim" => (t.faint, t.faint),
            _ if self.grey_when_ok => (t.dim, t.text),
            // an unknown level looks like the first one
            _ => {
                let c = this.and_then(|l| l.color).or_else(|| self.levels.first().and_then(|l| l.color));
                (c.unwrap_or(t.text), t.text)
            }
        };
        // a colour of the label's rule is the icon's; the level keeps the text's
        let ic = match from_label {
            Some((_, Some(c))) if out.icon.is_none() => {
                if level == "dim" {
                    Rgba(c.0, c.1, c.2, c.3 * 0.5)
                } else {
                    c
                }
            }
            _ => ic,
        };
        let text = if self.icon_only { String::new() } else { label };
        let mut it: Item = item(icon.as_deref(), text, ic, tc);
        it.level = level;
        Show::Item(it)
    }

    /// Keep the copy of line 1 that other tools read.
    fn write_cache(&self, line: &str) {
        let Some(cache) = &self.cache_file else { return };
        if let Some(dir) = cache.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = PathBuf::from(format!("{}.tmp", cache.display()));
        if std::fs::write(&tmp, format!("{line}\n")).is_ok() {
            let _ = std::fs::rename(&tmp, cache);
        }
    }
}

impl Widget for ScriptWidget {
    fn key(&self) -> &str {
        &self.key
    }
    fn schedule(&self) -> Every {
        Every::Ms(self.every_ms)
    }
    fn user_widgets(&self) -> &[u32] {
        &self.ids
    }
    /// Runs the script. A failed script's output is still shown, as before.
    fn poll(&self) -> Result<Show, Failure> {
        let (out, error) = match run_shell_limited(&self.command, self.timeout, OUTPUT_LIMIT) {
            Ok(out) => (out, None),
            Err(e) => {
                let error = e.to_string();
                (e.stdout().unwrap_or_default(), Some(error))
            }
        };
        let out = if self.raw { ScriptOutput::parse_raw(&out) } else { ScriptOutput::parse(&out) };
        self.write_cache(&out.line);
        let show = self.present(&out);
        *self.menu.lock().unwrap_or_else(|e| e.into_inner()) = out.menu;
        match error {
            None => Ok(show),
            Some(error) => Err(Failure::new(error, show)),
        }
    }
    fn clickable(&self) -> bool {
        has_action(&self.clicks.left)
    }
    fn menu(&self) -> Option<(MenuOn, Vec<MenuLine>)> {
        let menu = self.menu.lock().unwrap_or_else(|e| e.into_inner());
        (!menu.is_empty()).then(|| (menu_on(self.menu_on, has_action(&self.clicks.left)), menu.clone()))
    }
    fn click(&self, b: Button, _: Option<&str>) -> bool {
        let cmd = match b {
            Button::Left => &self.clicks.left,
            Button::Right => &self.clicks.right,
            Button::Middle => &self.clicks.middle,
        };
        if !run_action(cmd) {
            return false;
        }
        sched::refresh_in(&self.key, 500);
        true
    }
}

/// Split line 1 into `(level, label)`. A line without `|` is an "ok" label;
/// an empty line is nothing at all.
pub fn parse_level_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(match line.split_once('|') {
        Some((l, s)) => (l.trim().to_string(), s.trim().to_string()),
        None => ("ok".to_string(), line.to_string()),
    })
}

/// The icon a label's rule picks, and the colour it gives that icon (if any).
fn label_icon(rule: LabelIcons, label: &str, set: IconSet) -> Option<(&'static str, Option<Rgba>)> {
    match rule {
        LabelIcons::Prayer => prayer_icon(label, set),
    }
}

/// `iconFromLabel: "prayer"`: the icon follows the next prayer's name. With
/// simple-bar's icons, it is the prayer's own icon, in the colour of its time
/// of day (dawn blue, sunrise amber, noon yellow, afternoon orange, sunset
/// red, night violet).
fn prayer_icon(label: &str, set: IconSet) -> Option<(&'static str, Option<Rgba>)> {
    let first = label.split(|c: char| c.is_whitespace() || c == '·').next()?.to_lowercase();
    let (sf, own, hex) = match first.as_str() {
        "fajr" => ("sunrise", "Fajr", "#9db4ff"),
        "chorouq" | "shuruq" | "sunrise" => ("sun.horizon", "Chorouq", "#ffc56b"),
        "dohr" | "dhuhr" | "zuhr" => ("sun.max", "Dohr", "#ffd60a"),
        "asr" => ("sun.haze", "Asr", "#ffab5e"),
        "maghrib" => ("sunset", "Maghrib", "#ff7a6b"),
        "ishae" | "isha" => ("moon.stars", "Ishae", "#b39df3"),
        _ => return None,
    };
    Some(match set {
        IconSet::Sf => (sf, None),
        IconSet::SimpleBar => (own, Rgba::hex(hex)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rgba;

    fn lv(level: &str, label: &str) -> Option<(String, String)> {
        Some((level.to_string(), label.to_string()))
    }

    #[test]
    fn level_lines() {
        assert_eq!(parse_level_line("warn|3 tasks"), lv("warn", "3 tasks"));
        assert_eq!(parse_level_line("  alert |  late  "), lv("alert", "late"));
        assert_eq!(parse_level_line("a|b|c"), lv("a", "b|c"));
        assert_eq!(parse_level_line("warn|"), lv("warn", ""));
        assert_eq!(parse_level_line("|label"), lv("", "label"));
    }

    #[test]
    fn level_line_without_pipe_is_ok() {
        assert_eq!(parse_level_line("just text"), lv("ok", "just text"));
        assert_eq!(parse_level_line("  padded  "), lv("ok", "padded"));
    }

    #[test]
    fn empty_level_line_is_nothing() {
        assert_eq!(parse_level_line(""), None);
        assert_eq!(parse_level_line("   \t "), None);
    }

    #[test]
    fn prayer_icons() {
        assert_eq!(prayer_icon("Fajr 05:12", IconSet::Sf), Some(("sunrise", None)));
        assert_eq!(prayer_icon("Asr·15:40", IconSet::Sf), Some(("sun.haze", None)));
        assert_eq!(prayer_icon("DHUHR in 2h", IconSet::Sf), Some(("sun.max", None)));
        assert_eq!(prayer_icon("isha", IconSet::Sf), Some(("moon.stars", None)));
        assert_eq!(prayer_icon("Maghrib", IconSet::Sf), Some(("sunset", None)));
        assert_eq!(prayer_icon("Dohr · 39m", IconSet::SimpleBar), Some(("Dohr", Rgba::hex("#ffd60a"))));
        assert_eq!(prayer_icon("Ishae 20:10", IconSet::SimpleBar), Some(("Ishae", Rgba::hex("#b39df3"))));
        assert_eq!(prayer_icon("Lunch 12:00", IconSet::SimpleBar), None);
        assert_eq!(prayer_icon("", IconSet::Sf), None);
    }

    /// The fixture's theme with SF Symbols (the imported fixture uses simple-bar's icons).
    fn theme() -> Theme {
        Theme { icons: IconSet::Sf, ..crate::config::fixture().theme }
    }

    const GREEN: Rgba = Rgba(0.0, 1.0, 0.0, 1.0);
    const RED: Rgba = Rgba(1.0, 0.0, 0.0, 1.0);

    fn level(name: &str, color: Option<Rgba>, icon: Option<&str>) -> Level {
        Level { name: name.into(), color, icon: icon.map(String::from) }
    }

    /// A script with an "ok" (green) and a "warn" (red, its own icon) level.
    fn opts(icon: &str, warn_icon: Option<&str>) -> ScriptOptions {
        ScriptOptions {
            command: "true".into(),
            every_ms: 1000,
            timeout_ms: 30_000,
            icon: Some(icon.into()),
            levels: vec![level("ok", Some(GREEN), None), level("warn", Some(RED), warn_icon)],
            clicks: Clicks::default(),
            menu_on: None,
            popover: false,
            popover_tab: false,
            hide_when_empty: true,
            icon_only: false,
            grey_when_ok: false,
            icon_from_label: None,
            prelude: None,
            cache_file: None,
            simplebar_ids: vec![1, 2],
            raw: false,
        }
    }

    fn widget(o: &ScriptOptions) -> ScriptWidget {
        ScriptWidget::new("w", o, &theme())
    }

    fn shown(w: &ScriptWidget, out: &str) -> Item {
        match w.present(&ScriptOutput::parse(out)) {
            Show::Item(it) => it,
            other => panic!("{out:?} should show, got {other:?}"),
        }
    }

    #[test]
    fn level_colours() {
        let t = theme();
        let w = widget(&opts("timer", Some("bell")));
        let it = shown(&w, "ok|3 tasks\nsecond line is ignored");
        assert_eq!((it.icon.as_deref(), it.text.as_str(), it.level.as_str()), (Some("timer"), "3 tasks", "ok"));
        assert_eq!((it.icon_color, it.text_color), (GREEN, t.text), "the level's colour on the icon");

        let it = shown(&w, "warn|late");
        assert_eq!((it.icon.as_deref(), it.icon_color, it.text_color), (Some("bell"), t.warn, t.warn));
        let it = shown(&w, "alert|very late");
        assert_eq!((it.icon.as_deref(), it.icon_color, it.text_color), (Some("timer"), t.alert, t.alert));
        let it = shown(&w, "dim|idle");
        assert_eq!((it.icon_color, it.text_color), (t.faint, t.faint));
        let it = shown(&w, "just a label");
        assert_eq!((it.level.as_str(), it.icon_color), ("ok", GREEN));
    }

    #[test]
    fn unknown_level_looks_like_the_first() {
        let t = theme();
        let w = widget(&opts("timer", Some("bell")));
        let it = shown(&w, "weird|x");
        assert_eq!((it.icon.as_deref(), it.icon_color, it.text_color), (Some("timer"), GREEN, t.text));
        assert_eq!(it.level, "weird", "the level is kept for /status");
    }

    #[test]
    fn custom_levels_with_icons_and_without_colours() {
        let t = theme();
        let mut o = opts("wifi", None);
        o.levels.push(level("eth", Some(RED), Some("network")));
        o.levels.push(level("busy", None, Some("hourglass")));
        let w = widget(&o);
        let it = shown(&w, "eth|cable");
        assert_eq!((it.icon.as_deref(), it.icon_color, it.text_color), (Some("network"), RED, t.text));
        let it = shown(&w, "busy|x");
        assert_eq!((it.icon.as_deref(), it.icon_color), (Some("hourglass"), GREEN), "no colour: the first level's");
        o.levels.clear();
        let it = shown(&widget(&o), "eth|cable");
        assert_eq!((it.icon.as_deref(), it.icon_color), (Some("wifi"), t.text), "no levels at all: the text colour");
        o.icon = None;
        assert_eq!(shown(&widget(&o), "x").icon, None, "no icon option: no icon");
    }

    #[test]
    fn grey_when_ok() {
        let t = theme();
        let mut o = opts("internaldrive", None);
        o.grey_when_ok = true;
        let w = widget(&o);
        let it = shown(&w, "ok|120 GB");
        assert_eq!((it.icon_color, it.text_color), (t.dim, t.text));
        let it = shown(&w, "other|120 GB");
        assert_eq!((it.icon_color, it.text_color), (t.dim, t.text), "any level that isn't a warning");
        let it = shown(&w, "warn|5 GB");
        assert_eq!((it.icon_color, it.text_color), (t.warn, t.warn), "warnings keep their colour");
    }

    #[test]
    fn empty_output_hides() {
        let w = widget(&opts("timer", None));
        for out in ["", "\n", "   ", "ok|", "warn|  ", "\nok|second line"] {
            assert_eq!(w.present(&ScriptOutput::parse(out)), Show::Hide, "{out:?}");
        }
    }

    #[test]
    fn empty_output_shows_when_asked_to() {
        let t = theme();
        let mut o = opts("terminal", None);
        o.hide_when_empty = false;
        let it = shown(&widget(&o), "");
        assert_eq!((it.icon.as_deref(), it.text.as_str(), it.icon_color), (Some("terminal"), "", GREEN));
        assert_eq!(it.text_color, t.text);
    }

    #[test]
    fn icon_from_the_label() {
        let mut o = opts("moon.stars", Some("sun.max"));
        o.icon_from_label = Some(LabelIcons::Prayer);
        let w = widget(&o);
        assert_eq!(shown(&w, "ok|Fajr 05:12").icon.as_deref(), Some("sunrise"));
        assert_eq!(shown(&w, "warn|Asr·15:40").icon.as_deref(), Some("sun.haze"), "it wins over the level's icon");
        assert_eq!(shown(&w, "ok|Lunch").icon.as_deref(), Some("moon.stars"), "else the usual icon");
        assert_eq!(shown(&w, "warn|Lunch").icon.as_deref(), Some("sun.max"));
        // only with the option
        let other = widget(&opts("moon.stars", None));
        assert_eq!(shown(&other, "ok|Fajr 05:12").icon.as_deref(), Some("moon.stars"));
    }

    #[test]
    fn icon_only() {
        let t = theme();
        let mut o = opts("mic.fill", Some("mic.slash.fill"));
        o.icon_only = true;
        let w = widget(&o);
        let it = shown(&w, "ok|on");
        assert_eq!((it.icon.as_deref(), it.text.as_str()), (Some("mic.fill"), ""));
        let it = shown(&w, "warn|");
        assert_eq!((it.icon.as_deref(), it.text.as_str(), it.icon_color), (Some("mic.slash.fill"), "", t.warn));
        assert_eq!(w.present(&ScriptOutput::parse("")), Show::Hide, "no output still hides");
    }

    #[test]
    fn clicks_and_numbers() {
        let mut o = opts("terminal", None);
        o.clicks = Clicks { left: String::new(), right: "true".into(), middle: " ".into() };
        let w = widget(&o);
        assert_eq!(w.user_widgets(), [1, 2]);
        assert!(!w.clickable(), "no left action");
        for b in [Button::Left, Button::Right, Button::Middle] {
            assert!(!w.click(b, None), "{b:?}: nothing to run (`true` is no action)");
        }
    }

    #[test]
    fn failed_script_output_is_still_shown() {
        let mut o = opts("terminal", None);
        o.command = "echo partial; exit 3".into();
        let f = widget(&o).poll().unwrap_err();
        assert_eq!(f.error, "exited with status 3");
        assert!(matches!(&*f.show, Show::Item(it) if it.text == "partial"), "{f:?}");
        o.command = "echo fine".into();
        assert!(matches!(widget(&o).poll(), Ok(Show::Item(it)) if it.text == "fine"));
    }

    #[test]
    fn prelude_cache_and_timeout() {
        let dir = std::env::temp_dir().join(format!("wingbar-script-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let env = dir.join("env with space.sh");
        std::fs::write(&env, "GREETING=hello\n").unwrap();
        let mut o = opts("terminal", None);
        o.prelude = Some(env.display().to_string());
        o.command = "echo \"warn|$GREETING\"; echo more".into();
        o.cache_file = Some(dir.join("cache/w.out").display().to_string());
        let w = widget(&o);
        assert!(matches!(w.poll(), Ok(Show::Item(it)) if it.text == "hello" && it.level == "warn"));
        assert_eq!(std::fs::read_to_string(dir.join("cache/w.out")).unwrap(), "warn|hello\n");
        o.prelude = Some(dir.join("missing.sh").display().to_string());
        o.command = "echo still runs".into();
        assert!(matches!(widget(&o).poll(), Ok(Show::Item(it)) if it.text == "still runs"));
        o.prelude = None;
        o.command = "sleep 5".into();
        o.timeout_ms = 300;
        assert!(widget(&o).poll().unwrap_err().error.starts_with("timed out"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn action(title: &str, command: &str) -> MenuLine {
        MenuLine::Action { title: title.into(), command: command.into() }
    }

    #[test]
    fn output_protocol_lines() {
        let out = ScriptOutput::parse(
            "warn|3 tasks\n\
             icon: timer\n\
             menu: Stop timer | bash ~/scripts/clickup_toggle.sh\n\
             menu: Today: 2h 10m\n\
             menu: ---\n\
             some other line\n\
             menu: Open | open 'https://example.com/a|b' | cat\n",
        );
        assert_eq!(out.line, "warn|3 tasks");
        assert_eq!((out.level.as_str(), out.label.as_str()), ("warn", "3 tasks"));
        assert_eq!(out.icon.as_deref(), Some("timer"));
        assert_eq!(
            out.menu,
            [
                action("Stop timer", "bash ~/scripts/clickup_toggle.sh"),
                MenuLine::Info("Today: 2h 10m".into()),
                MenuLine::Separator,
                action("Open", "open 'https://example.com/a|b' | cat"),
            ],
            "the first `|` ends the title; the command keeps the rest"
        );
    }

    #[test]
    fn a_menu_command_is_only_ever_what_follows_the_first_bar_of_a_menu_line() {
        // a title can't hold a "|": text a script didn't strip ends the title
        // there (which is why scripts must strip it from names they print)
        let out = ScriptOutput::parse(
            "ok|x\n\
             menu: Pixel a|b · serial\n\
             menu:   | rm -rf ~\n\
             note menu: a | rm -rf ~\n\
             icon: bolt | rm -rf ~\n\
             a | menu: b | rm -rf ~\n",
        );
        assert_eq!(out.menu, [action("Pixel a", "b · serial")], "a line without a title is skipped");
        assert_eq!(out.icon.as_deref(), Some("bolt | rm -rf ~"), "icon lines run nothing");
    }

    #[test]
    fn output_protocol_line_one_as_before() {
        // only line 1, as every 0.2 script prints
        let out = ScriptOutput::parse("just a label\n");
        assert_eq!((out.level.as_str(), out.label.as_str(), out.icon, out.menu), ("ok", "just a label", None, vec![]));
        // line 1 is never read as an icon or menu line
        let out = ScriptOutput::parse("menu: x | y\nicon: bell");
        assert_eq!((out.level.as_str(), out.label.as_str()), ("menu: x", "y"));
        assert_eq!((out.icon.as_deref(), out.menu.len()), (Some("bell"), 0));
        let out = ScriptOutput::parse("icon: star");
        assert_eq!((out.label.as_str(), out.icon), ("icon: star", None));
        // empty line 1: nothing to show (the menu is still read)
        let out = ScriptOutput::parse("\nmenu: a | b\n");
        assert_eq!((out.line.as_str(), out.level.as_str(), out.label.as_str()), ("", "ok", ""));
        assert_eq!(out.menu, [action("a", "b")]);
        assert_eq!(ScriptOutput::parse(""), ScriptOutput { level: "ok".into(), ..ScriptOutput::default() });
    }

    #[test]
    fn output_protocol_whitespace_and_malformed_lines() {
        let out = ScriptOutput::parse(
            "  ok |  label  \r\n\
             \t icon:    \n\
             icon:   bell.fill  \r\n\
             icon: second\n\
             menu:\n\
             menu:    \n\
             menu: | no title\n\
             menu:   Spaced   |   cmd  arg  \n\
             menu:---\n\
             menu: title |   \n\
             MENU: upper | ignored\n\
             menus: x | y\n\
             icon\n\
             menu: --- | not a separator\n",
        );
        assert_eq!((out.level.as_str(), out.label.as_str()), ("ok", "label"));
        assert_eq!(out.icon.as_deref(), Some("bell.fill"), "the first icon line that names one");
        assert_eq!(
            out.menu,
            [
                action("Spaced", "cmd  arg"),
                MenuLine::Separator,
                MenuLine::Info("title".into()),
                action("---", "not a separator"),
            ]
        );
    }

    #[test]
    fn output_protocol_reads_64_lines_at_most() {
        let mut out = String::from("ok|x\n");
        for i in 1..100 {
            out += &format!("menu: item {i} | echo {i}\n");
        }
        let o = ScriptOutput::parse(&out);
        assert_eq!(o.menu.len(), 63, "line 1 and 63 menu lines");
        assert_eq!(o.menu.last(), Some(&action("item 63", "echo 63")));
    }

    #[test]
    fn prayer_icons_and_colours_with_simple_bars_icons() {
        let t = Theme { icons: IconSet::SimpleBar, ..theme() };
        let mut o = opts("Moon", None);
        o.icon_from_label = Some(LabelIcons::Prayer);
        let w = ScriptWidget::new("salat", &o, &t);
        let dohr = Rgba::hex("#ffd60a").unwrap();
        let it = shown(&w, "ok|Dohr · 39m");
        assert_eq!((it.icon.as_deref(), it.icon_color, it.text_color), (Some("Dohr"), dohr, t.text));
        let it = shown(&w, "warn|Dohr · 5m");
        assert_eq!((it.icon_color, it.text_color), (dohr, t.warn), "urgency stays on the text");
        let it = shown(&w, "dim|Dohr");
        assert_eq!(it.icon_color, Rgba(dohr.0, dohr.1, dohr.2, 0.5), "faded when dim");
        let it = shown(&w, "ok|Offline");
        assert_eq!(it.icon.as_deref(), Some("Moon"), "no prayer named: the widget's icon");
    }

    #[test]
    fn icon_precedence() {
        let mut o = opts("moon.stars", Some("sun.max"));
        o.icon_from_label = Some(LabelIcons::Prayer);
        let w = widget(&o);
        // the icon line wins over everything
        assert_eq!(shown(&w, "warn|Fajr 05:12\nicon: bolt").icon.as_deref(), Some("bolt"));
        // then the label's rule, when it matches
        assert_eq!(shown(&w, "warn|Fajr 05:12").icon.as_deref(), Some("sunrise"));
        // then the level's icon
        assert_eq!(shown(&w, "warn|Lunch").icon.as_deref(), Some("sun.max"));
        // then the widget's
        assert_eq!(shown(&w, "ok|Lunch").icon.as_deref(), Some("moon.stars"));
        // an icon line with no icon option at all
        let mut o = opts("x", None);
        o.icon = None;
        assert_eq!(shown(&widget(&o), "ok|a\nicon: star").icon.as_deref(), Some("star"));
        assert_eq!(shown(&widget(&o), "ok|a").icon, None);
    }

    #[test]
    fn colours_dont_change_with_an_icon_line() {
        let t = theme();
        let w = widget(&opts("timer", None));
        let it = shown(&w, "warn|late\nicon: bell\nmenu: a | b");
        assert_eq!((it.icon_color, it.text_color, it.text.as_str()), (t.warn, t.warn, "late"));
        let it = shown(&w, "ok|fine\nicon: bell");
        assert_eq!((it.icon_color, it.text_color), (GREEN, t.text));
    }

    #[test]
    fn menu_on_defaults() {
        assert_eq!(menu_on(None, false), MenuOn::Click, "menu lines and no click command");
        assert_eq!(menu_on(None, true), MenuOn::None, "the click command keeps the click");
        for set in [MenuOn::Click, MenuOn::Hover, MenuOn::None] {
            assert_eq!(menu_on(Some(set), true), set);
            assert_eq!(menu_on(Some(set), false), set);
        }
    }

    #[test]
    fn the_last_run_keeps_its_menu() {
        let mut o = opts("terminal", None);
        o.command = "printf 'ok|x\\nmenu: A | echo a\\nmenu: ---\\nmenu: B\\n'".into();
        let w = widget(&o);
        assert_eq!(w.menu(), None, "nothing before the first run");
        assert!(matches!(w.poll(), Ok(Show::Item(_))));
        let (on, lines) = w.menu().unwrap();
        assert_eq!(on, MenuOn::Click);
        assert_eq!(lines, [action("A", "echo a"), MenuLine::Separator, MenuLine::Info("B".into())]);
        // a click command: the menu doesn't open on click unless asked to
        o.clicks.left = "open -a Calendar".into();
        let w = widget(&o);
        w.poll().unwrap();
        assert_eq!(w.menu().map(|m| m.0), Some(MenuOn::None));
        o.menu_on = Some(MenuOn::Hover);
        let w = widget(&o);
        w.poll().unwrap();
        assert_eq!(w.menu().map(|m| m.0), Some(MenuOn::Hover));
        // a run without menu lines takes the menu away
        o.command = "echo ok".into();
        let w2 = widget(&o);
        w2.poll().unwrap();
        assert_eq!(w2.menu(), None);
    }

    /// A script widget as the settings file defines it.
    fn defined(def: serde_json::Value) -> ScriptWidget {
        let doc = serde_json::json!({ "version": 2, "right": ["w"], "widgets": { "w": def } });
        let (cfg, warnings) = crate::config::parse::parse(&doc, &|_: &std::path::Path| false);
        assert!(warnings.is_empty(), "{warnings:?}");
        match &cfg.def("w").unwrap().options {
            crate::config::Options::Script(o) => ScriptWidget::new("w", o, &cfg.theme),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn raw_output_is_the_first_line_as_it_is() {
        let raw = |command: &str| {
            defined(
                serde_json::json!({ "type": "script", "command": command, "raw": true, "levels": { "ok": "green" } }),
            )
        };
        // a "|" is part of the label, and later lines are not read as icon or menu lines
        let w = raw("printf 'CPU 12%% | MEM 40%%\\nicon: bolt\\nmenu: x | touch /tmp/never\\n'");
        let Ok(Show::Item(it)) = w.poll() else { panic!() };
        assert_eq!((it.text.as_str(), it.level.as_str(), it.icon.as_deref()), ("CPU 12% | MEM 40%", "ok", None));
        assert_eq!(w.menu(), None, "no menus");
        // leading blank lines don't hide it: the first line of the trimmed output
        let w = raw("printf '\\n\\n  warn|hello  \\nsecond\\n'");
        let Ok(Show::Item(it)) = w.poll() else { panic!() };
        assert_eq!((it.text.as_str(), it.level.as_str()), ("warn|hello", "ok"), "no level is read");
        // nothing at all still hides
        assert!(matches!(raw("printf '\\n  \\n'").poll(), Ok(Show::Hide)));
        // without raw, the protocol as before
        let w = defined(serde_json::json!({ "type": "script", "command": "echo 'CPU 12% | MEM 40%'" }));
        let Ok(Show::Item(it)) = w.poll() else { panic!() };
        assert_eq!((it.level.as_str(), it.text.as_str()), ("CPU 12%", "MEM 40%"));
    }

    #[test]
    fn a_script_flooding_output_is_cut_and_stopped() {
        let mut o = opts("terminal", None);
        o.command = "echo 'ok|flood'; echo 'menu: a | b'; yes 'menu: x | y'".into();
        let start = std::time::Instant::now();
        let w = widget(&o);
        assert!(matches!(w.poll(), Ok(Show::Item(it)) if it.text == "flood"));
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
        assert_eq!(w.menu().unwrap().1.len(), 63);
    }
}
