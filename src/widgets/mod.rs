//! Every widget in one place. Each one says, together, what its key is, how
//! often it runs, how it computes what the bar shows, and what clicking or
//! scrolling it does. The registry, built from the settings (one widget per
//! placed definition), is what the scheduler, the bar and the server ask about
//! widgets. It is swappable (`current`, `set_current`) so the settings can be
//! reloaded without a restart.
//!
//! - `system`: cpu, memory, battery, sound (in-process readings)
//! - `clock`: date, time
//! - `media`: Spotify / Music now playing
//! - `script`: script widgets (`level|label` output)
//! - `window`: the focused window's title
//! - `spaces`: the workspaces (AeroSpace / yabai)

mod clock;
mod media;
mod script;
mod spaces;
mod system;
mod window;

pub use media::{PLAYERS, now_playing_changed};
pub use script::MenuLine;
pub use window::AX_OK;

use crate::config::{Config, MenuOn, Options, Rgba, WidgetDef, WidgetType, Wm};
use crate::sched::{self, Every, Task, spawn_shell};
use crate::state::{self, Item, State};
use std::sync::{Arc, Mutex, RwLock};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// One scroll event over a widget.
#[derive(Clone, Copy, Debug)]
pub struct Scroll {
    /// Positive = fingers / wheel up, whatever the natural-scrolling setting.
    pub dy: f64,
    /// A trackpad or Magic Mouse (pixel deltas), not wheel notches.
    pub precise: bool,
    /// ⌥ is held: take small steps.
    pub fine: bool,
}

/// How the bar draws a widget. Only workspaces and the window title have
/// segments of their own; everything else is an `Item` in the state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Item,
    Spaces,
    Title,
}

/// Whether the widget `key`, drawn as `kind`, has something on the bar now
/// (a disabled or empty widget takes no room, so it can't be seen).
pub fn showing(st: &State, key: &str, kind: Kind) -> bool {
    match kind {
        Kind::Spaces => !st.spaces.is_empty(),
        Kind::Title => !st.title.is_empty(),
        Kind::Item => st.items.contains_key(key) && !st.disabled.contains(key),
    }
}

/// What a poll means for the bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Show {
    Item(Item),
    /// Show nothing for this widget.
    Hide,
    /// Leave the bar as it is (nothing new, or the widget wrote its own state).
    Keep,
}

/// A poll that went wrong. `show` is what to do with the bar anyway: a
/// script's output is still shown after it fails, as it always was.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub error: String,
    pub show: Box<Show>,
}

impl Failure {
    pub fn new(error: String, show: Show) -> Failure {
        Failure { error, show: Box::new(show) }
    }
}

pub trait Widget: Send + Sync {
    /// The key in the wing lists, the state and the server routes.
    fn key(&self) -> &str;
    fn schedule(&self) -> Every;
    /// Compute what the bar shows. Runs on its own short-lived thread; never
    /// twice at once. The caller records the error (or clears it) and applies `Show`.
    fn poll(&self) -> Result<Show, Failure>;
    fn kind(&self) -> Kind {
        Kind::Item
    }
    /// The simple-bar user widget numbers it answers to.
    fn user_widgets(&self) -> &[u32] {
        &[]
    }
    /// Whether hovering it shows the "you can click this" highlight.
    fn clickable(&self) -> bool {
        false
    }
    /// Run what clicking does. `part` is the workspace name for the spaces
    /// widget, else `None`. Returns false when it has nothing to run.
    fn click(&self, _b: Button, _part: Option<&str>) -> bool {
        false
    }
    /// Returns false when scrolling over it does nothing.
    fn scroll(&self, _s: Scroll) -> bool {
        false
    }
    /// The menu it has now (a script's `menu:` lines) and when it opens; `None`
    /// for no menu.
    fn menu(&self) -> Option<(MenuOn, Vec<MenuLine>)> {
        None
    }
}

/// Names simple-bar used, for the key a type has by default: they reach that
/// widget (server routes, wing lists) when it is in the registry.
const ALIASES: &[(&str, &str)] = &[
    ("process", "title"),
    ("window", "title"),
    ("date-display", "date"),
    ("aerospace", "spaces"),
    ("yabai", "spaces"),
];

struct Entry {
    widget: Arc<dyn Widget>,
    ty: WidgetType,
    /// BarPopover has a hover menu for it.
    popover: bool,
    /// BarPopover's tab hangs under it.
    popover_tab: bool,
}

pub struct Registry {
    /// The placed widgets, left wing then right wing, in order.
    entries: Vec<Entry>,
}

static CURRENT: RwLock<Option<Arc<Registry>>> = RwLock::new(None);

/// The registry in use (empty before `set_current`).
pub fn current() -> Arc<Registry> {
    CURRENT.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_else(|| Arc::new(Registry::empty()))
}

/// Replace the registry in use.
pub fn set_current(reg: Registry) -> Arc<Registry> {
    let reg = Arc::new(reg);
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(reg.clone());
    reg
}

/// Re-run every widget of a type now (the workspaces after a Space change, ...).
pub fn refresh_type(t: WidgetType) {
    refresh_type_in(t, 0);
}

pub fn refresh_type_in(t: WidgetType, delay_ms: u64) {
    for k in current().keys_of_type(t) {
        sched::refresh_in(&k, delay_ms);
    }
}

/// The widget for one definition; `None` for the workspaces without a window manager.
fn make(def: &WidgetDef, cfg: &Config) -> Option<Arc<dyn Widget>> {
    let k = def.key.as_str();
    let t = &cfg.theme;
    Some(match &def.options {
        Options::Spaces if cfg.wm == Wm::None => return None,
        Options::Spaces => Arc::new(spaces::Spaces::new(k)),
        Options::Title => Arc::new(window::Title::new(k)),
        Options::Cpu(o) => Arc::new(system::Cpu::new(k, o, t)),
        Options::Memory(o) => Arc::new(system::Memory::new(k, o, t)),
        Options::Battery(o) => Arc::new(system::Battery::new(k, o, t)),
        Options::Sound => Arc::new(system::Sound::new(k, cfg.bar.volume_step, t)),
        Options::Date(click) => Arc::new(clock::Clock::date(k, click, t)),
        Options::Time(o) => Arc::new(clock::Clock::time(k, o, t)),
        Options::Spotify => Arc::new(media::NowPlaying::new(k, &PLAYERS[0], t)),
        Options::Music => Arc::new(media::NowPlaying::new(k, &PLAYERS[1], t)),
        Options::Script(o) => Arc::new(script::ScriptWidget::new(k, o, t)),
    })
}

impl Registry {
    pub fn empty() -> Registry {
        Registry { entries: Vec::new() }
    }

    /// One widget per placed definition, in wing order. Hidden ones don't run.
    pub fn build(cfg: &Config) -> Registry {
        Registry::rebuild(cfg, None)
    }

    /// `build`, for settings that replace `prev`'s: a widget whose definition is
    /// the same, while the settings every widget reads (theme, `bar`, window
    /// manager) are too, is kept as it is (with what it holds, such as a
    /// half-done scroll); every other one is made anew.
    pub fn rebuild(cfg: &Config, prev: Option<(&Config, &Registry)>) -> Registry {
        let prev = prev.filter(|(old, _)| old.theme == cfg.theme && old.bar == cfg.bar && old.wm == cfg.wm);
        let entries = cfg
            .placed()
            .filter_map(|(_, def)| {
                let popover = matches!(&def.options, Options::Script(s) if s.popover);
                let popover_tab = matches!(&def.options, Options::Script(s) if s.popover_tab);
                let kept = prev
                    .filter(|(old, _)| old.def(&def.key) == Some(def))
                    .and_then(|(_, reg)| reg.entry(&def.key))
                    .map(|e| e.widget.clone());
                let widget = match kept {
                    Some(w) => w,
                    None => make(def, cfg)?,
                };
                Some(Entry { widget, ty: def.options.widget_type(), popover, popover_tab })
            })
            .collect();
        Registry { entries }
    }

    /// Whether `key` is the very same widget in both registries (kept by `rebuild`).
    pub fn same_widget(&self, other: &Registry, key: &str) -> bool {
        match (self.entry(key), other.entry(key)) {
            (Some(a), Some(b)) => Arc::ptr_eq(&a.widget, &b.widget),
            _ => false,
        }
    }

    fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.widget.key() == key)
    }

    pub fn get(&self, key: &str) -> Option<&dyn Widget> {
        self.entry(key).map(|e| &*e.widget)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.widget.key())
    }

    /// The keys of the widgets of one type.
    pub fn keys_of_type(&self, t: WidgetType) -> Vec<String> {
        self.entries.iter().filter(|e| e.ty == t).map(|e| e.widget.key().to_string()).collect()
    }

    /// One scheduler task per widget, writing to `st` (the bar's state, or a
    /// test's). Each run records the widget's error (or clears it) and applies
    /// what it shows, unless a reload has since removed the widget or made it
    /// anew (its key moved on to another generation).
    pub fn tasks(&self, st: &'static Mutex<State>) -> Vec<Task> {
        let state = st.lock().unwrap_or_else(|e| e.into_inner());
        self.entries
            .iter()
            .map(|e| {
                let w = e.widget.clone();
                let generation = state.generation_of(w.key());
                Task {
                    key: w.key().to_string(),
                    every: w.schedule(),
                    job: Arc::new(move || {
                        run(&*w, generation, st);
                    }),
                }
            })
            .collect()
    }

    /// Translate a name to a key: a key is itself; simple-bar's names for the
    /// title, the date and the workspaces (`process`, `date-display`, `yabai`,
    /// ...) reach the widget with that type's default key. Any case will do
    /// (keys are lowercase; 0.2 took `/widget/ClickUp/...` for its "ClickUp"
    /// scripts). Anything else is returned lowercased.
    pub fn key_for_name(&self, name: &str) -> String {
        let name = name.to_lowercase();
        if self.entry(&name).is_none()
            && let Some((_, k)) = ALIASES.iter().find(|(a, _)| *a == name)
            && self.entry(k).is_some()
        {
            return k.to_string();
        }
        name
    }

    /// Which widget answers to a simple-bar user widget number.
    pub fn key_for_user_widget(&self, id: u32) -> Option<String> {
        self.entries.iter().find(|e| e.widget.user_widgets().contains(&id)).map(|e| e.widget.key().to_string())
    }

    /// Whether the person's BarPopover app has a menu for it.
    pub fn popover(&self, key: &str) -> bool {
        self.entry(key).is_some_and(|e| e.popover)
    }

    /// The widget BarPopover's tab hangs under (the first one shown with
    /// `popoverTab`).
    pub fn popover_tab(&self) -> Option<&str> {
        self.entries.iter().find(|e| e.popover_tab).map(|e| e.widget.key())
    }

    /// Shows the "you can click this" highlight: it has a click action, a
    /// BarPopover menu or a menu of its own.
    pub fn clickable(&self, key: &str) -> bool {
        self.popover(key)
            || self.get(key).is_some_and(|w| w.clickable())
            || self.menu(key, MenuOn::Click).is_some()
            || self.menu(key, MenuOn::Hover).is_some()
    }

    /// The widget's menu, if it has one that opens `on` this (a left click or a
    /// hover). BarPopover's hover menu wins over a hover menu of its own.
    pub fn menu(&self, key: &str, on: MenuOn) -> Option<Vec<MenuLine>> {
        let (opens, lines) = self.get(key)?.menu()?;
        let popover_instead = on == MenuOn::Hover && self.popover(key);
        (opens == on && on != MenuOn::None && !popover_instead && !lines.is_empty()).then_some(lines)
    }

    pub fn click(&self, key: &str, b: Button, part: Option<&str>) -> bool {
        self.get(key).is_some_and(|w| w.click(b, part))
    }

    pub fn scroll(&self, key: &str, s: Scroll) -> bool {
        self.get(key).is_some_and(|w| w.scroll(s))
    }
}

/// The scheduler's job for a widget of `generation`: its result is written
/// to `st` only while that is still the key's generation (no reload removed
/// the widget or made it anew since). False when it was dropped.
fn run(w: &dyn Widget, generation: u64, st: &Mutex<State>) -> bool {
    let (show, error) = match w.poll() {
        Ok(show) => (show, None),
        Err(f) => (*f.show, Some(f.error)),
    };
    let item = match show {
        Show::Item(it) => Some(Some(it)),
        Show::Hide => Some(None),
        Show::Keep => None,
    };
    state::apply(st, generation, w.key(), error, item)
}

fn item(icon: Option<&str>, text: String, icon_color: Rgba, text_color: Rgba) -> Item {
    Item { icon: icon.map(String::from), text, icon_color, text_color, bold: false, level: String::new(), trail: None }
}

/// A click command does something (empty and `true` mean nothing).
fn has_action(cmd: &str) -> bool {
    let s = cmd.trim();
    !s.is_empty() && s != "true"
}

/// Run a click command through the shell; false when there is none.
fn run_action(cmd: &str) -> bool {
    if !has_action(cmd) {
        return false;
    }
    spawn_shell(cmd);
    true
}

/// Run a menu item's command like a click action (detached, through bash),
/// then refresh the widget after 500 ms.
pub fn run_menu_command(key: &str, cmd: &str) {
    if run_action(cmd) {
        sched::refresh_in(key, 500);
    }
}

/// The registry for `config::fixture()`.
#[cfg(test)]
pub fn fixture_registry() -> Registry {
    Registry::build(&crate::config::fixture())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse::parse;
    use serde_json::json;

    #[test]
    fn showing_is_what_takes_room_on_the_bar() {
        let mut st = State::default();
        assert!(!showing(&st, "spaces", Kind::Spaces), "no workspaces yet");
        assert!(!showing(&st, "title", Kind::Title));
        assert!(!showing(&st, "spotify", Kind::Item), "nothing playing: no item");
        st.spaces.push(state::Space { name: "1".into(), focused: true, screen: 0, apps: vec![] });
        st.title = "Ghostty".into();
        let c = crate::config::Rgba(1.0, 1.0, 1.0, 1.0);
        let item = Item {
            icon: None,
            text: "Song".into(),
            icon_color: c,
            text_color: c,
            bold: false,
            level: String::new(),
            trail: None,
        };
        st.items.insert("spotify".into(), item);
        assert!(showing(&st, "spaces", Kind::Spaces));
        assert!(showing(&st, "title", Kind::Title));
        assert!(showing(&st, "spotify", Kind::Item));
        st.disabled.insert("spotify".into());
        assert!(!showing(&st, "spotify", Kind::Item), "a disabled widget takes no room");
    }

    fn registry(doc: serde_json::Value) -> Registry {
        // a window manager "found" for the workspaces
        let fs = |p: &std::path::Path| p == std::path::Path::new("/opt/homebrew/bin/aerospace");
        let (cfg, w) = parse(&doc, &fs);
        assert!(w.is_empty(), "{w:?}");
        Registry::build(&cfg)
    }

    #[test]
    fn fixture_registry_keys_and_schedules() {
        let reg = fixture_registry();
        let got: Vec<(&str, Every)> = reg.keys().map(|k| (k, reg.get(k).unwrap().schedule())).collect();
        // the placed widgets, in wing order; switched-off ones (cpu) aren't placed
        assert_eq!(
            got,
            [
                ("spaces", Every::Ms(5000)),
                ("title", Every::Ms(3000)), // no Accessibility in tests: asks the WM, less often
                ("clickup", Every::Ms(10_000)),
                ("salat", Every::Ms(500)),
                ("user-11", Every::Ms(10_000)),
                ("date", Every::Minute),
                ("battery", Every::Ms(60_000)),
                ("sound", Every::Ms(60_000)),
                ("music", Every::Ms(300_000)),
                ("time", Every::Minute),
                ("user-3", Every::Ms(5000)),
            ]
        );
        let tasks = reg.tasks(state::cell());
        let task_keys: Vec<&str> = tasks.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(task_keys, reg.keys().collect::<Vec<_>>(), "one task per widget, same order");
        assert!(tasks.iter().zip(reg.keys()).all(|(t, k)| t.every == reg.get(k).unwrap().schedule()));
    }

    #[test]
    fn hidden_widgets_dont_run_and_order_follows_the_wings() {
        let reg = registry(json!({
            "version": 2,
            "left": ["t"],
            "right": ["c", "s"],
            "widgets": {
                "s": { "type": "sound" }, "c": { "type": "cpu", "every": "1s" }, "t": { "type": "title" },
                "m": { "type": "memory" }, "x": { "type": "script", "command": "echo hidden" },
            },
        }));
        assert_eq!(reg.keys().collect::<Vec<_>>(), ["t", "c", "s"]);
        assert!(reg.get("m").is_none() && reg.get("x").is_none());
        assert_eq!(reg.tasks(state::cell()).len(), 3);
        assert_eq!(reg.get("c").unwrap().schedule(), Every::Ms(1000));
    }

    #[test]
    fn two_widgets_of_one_type() {
        let reg = registry(json!({
            "version": 2,
            "right": ["clock", "utc"],
            "widgets": { "clock": { "type": "time" }, "utc": { "type": "time", "seconds": true, "hour12": true } },
        }));
        assert_eq!(reg.get("clock").unwrap().schedule(), Every::Minute);
        assert_eq!(reg.get("utc").unwrap().schedule(), Every::Second);
        assert_eq!(reg.keys_of_type(WidgetType::Time), ["clock", "utc"]);
        assert_eq!(reg.tasks(state::cell()).iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), ["clock", "utc"]);
    }

    #[test]
    fn no_window_manager_no_workspaces() {
        let doc = json!({ "version": 2, "windowManager": "none", "left": ["spaces"], "widgets": { "spaces": { "type": "spaces" } } });
        let (cfg, _) = parse(&doc, &|_: &std::path::Path| false);
        assert_eq!(Registry::build(&cfg).keys().count(), 0);
    }

    #[test]
    fn names_map_to_keys() {
        let reg = fixture_registry();
        assert_eq!(reg.key_for_name("date-display"), "date");
        assert_eq!(reg.key_for_name("process"), "title");
        assert_eq!(reg.key_for_name("window"), "title");
        for n in ["spaces", "aerospace", "yabai"] {
            assert_eq!(reg.key_for_name(n), "spaces");
        }
        assert_eq!(reg.key_for_name("date"), "date");
        assert_eq!(reg.key_for_name("clickup"), "clickup");
        assert_eq!(reg.key_for_name("user-3"), "user-3");
        assert_eq!(reg.key_for_name("nothing-by-this-name"), "nothing-by-this-name");
        // aliases only reach the type's default key, when it is there
        let reg = registry(json!({
            "version": 2,
            "right": ["day", "process"],
            "widgets": { "day": { "type": "date" }, "process": { "type": "script", "command": "ps" } },
        }));
        assert_eq!(reg.key_for_name("date-display"), "date-display", "no widget keyed \"date\"");
        assert_eq!(reg.key_for_name("process"), "process", "a key wins over an alias");
    }

    #[test]
    fn user_widget_numbers_map_to_keys() {
        let reg = fixture_registry();
        // both levels of the clickup script answer to it
        assert_eq!(reg.key_for_user_widget(1).as_deref(), Some("clickup"));
        assert_eq!(reg.key_for_user_widget(2).as_deref(), Some("clickup"));
        assert_eq!(reg.key_for_user_widget(10).as_deref(), Some("salat"));
        assert_eq!(reg.key_for_user_widget(3).as_deref(), Some("user-3"));
        assert_eq!(reg.key_for_user_widget(11).as_deref(), Some("user-11"));
        assert_eq!(reg.key_for_user_widget(4), None, "switched off: hidden");
        assert_eq!(reg.key_for_user_widget(99), None);
    }

    fn memory_script(click: &str) -> serde_json::Value {
        json!({
            "widgets": { "memoryWidget": true },
            "userWidgets": { "userWidgetsList": {
                "5": { "active": true, "output": "bash /nonexistent/wingbar-fixture/sbw.sh memory ok",
                       "icon": "Desktop", "onClickAction": click },
            } },
        })
    }

    /// The fixture with `extra`, imported with BarPopover installed and 0.2's default wings.
    fn imported(extra: serde_json::Value) -> Registry {
        let mut sb: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/simplebarrc.json")).unwrap();
        sb.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let probe = |p: &std::path::Path| p.to_string_lossy().ends_with("/popover/barpopover");
        let (doc, _) = crate::config::import::import_simplebar(&sb, None, &probe);
        let (cfg, w) = parse(&doc, &|_: &std::path::Path| false);
        assert!(w.is_empty(), "{w:?}");
        Registry::build(&cfg)
    }

    #[test]
    fn a_memory_script_replaces_the_built_in_memory_widget() {
        let builtin = imported(json!({ "widgets": { "memoryWidget": true } }));
        let m = builtin.get("memory").unwrap();
        assert!(m.clickable() && m.user_widgets().is_empty(), "the built-in opens the memory app");

        let reg = imported(memory_script(""));
        assert_eq!(reg.keys().filter(|k| *k == "memory").count(), 1, "one memory widget, one task");
        let m = reg.get("memory").unwrap();
        assert_eq!(m.user_widgets(), [5]);
        assert_eq!(m.schedule(), Every::Ms(10_000));
        // its click is the script's (none here), not the built-in's
        assert!(!m.clickable());
        assert!(!reg.click("memory", Button::Left, None));
        assert!(reg.popover("memory") && reg.clickable("memory"), "still has its hover menu");

        let reg = imported(memory_script("true"));
        assert!(!reg.get("memory").unwrap().clickable(), "`true` is no action");
    }

    #[test]
    fn click_routing_without_actions() {
        let reg = fixture_registry();
        // nothing to run: false, so the bar may open a hover menu instead
        assert!(!reg.click("time", Button::Left, None));
        assert!(!reg.click("user-3", Button::Left, None));
        assert!(!reg.click("user-11", Button::Middle, None));
        assert!(!reg.click("clickup", Button::Right, None), "only a left action is set");
        assert!(!reg.click("date", Button::Right, None));
        assert!(!reg.click("battery", Button::Middle, None));
        assert!(!reg.click("music", Button::Right, None));
        assert!(!reg.click("spaces", Button::Right, Some("1")));
        assert!(!reg.click("no-such-widget", Button::Left, None));
        assert!(!reg.scroll("time", Scroll { dy: 3.0, precise: true, fine: false }));
        assert!(!reg.scroll("no-such-widget", Scroll { dy: 3.0, precise: true, fine: false }));

        assert!(!reg.clickable("time") && !reg.clickable("user-3") && !reg.clickable("title"));
        for k in ["date", "battery", "sound", "music", "clickup"] {
            assert!(reg.clickable(k), "{k}");
        }
        assert!(reg.clickable("salat"), "no click action, but a hover menu");
        assert!(reg.popover("clickup") && !reg.popover("user-3"));
    }

    #[test]
    fn menu_routing() {
        let menu = "printf 'ok|x\\nmenu: Hello | echo hi\\n'";
        let reg = registry(json!({
            "version": 2,
            "right": ["m", "c", "cm", "h", "p", "n", "plain", "date"],
            "widgets": {
                "m": { "type": "script", "command": menu },
                "c": { "type": "script", "command": menu, "click": ":" },
                "cm": { "type": "script", "command": menu, "click": ":", "menuOn": "click" },
                "h": { "type": "script", "command": menu, "menuOn": "hover" },
                "p": { "type": "script", "command": menu, "menuOn": "hover", "popover": true },
                "n": { "type": "script", "command": menu, "menuOn": "none" },
                "plain": { "type": "script", "command": "echo 'ok|x'" },
                "date": { "type": "date" },
            },
        }));
        let keys: Vec<String> = reg.keys().map(String::from).collect();
        for k in &keys {
            let _ = reg.get(k).unwrap().poll();
        }
        let hello = || Some(vec![MenuLine::Action { title: "Hello".into(), command: "echo hi".into() }]);
        let on = |k: &str| (reg.menu(k, MenuOn::Click).is_some(), reg.menu(k, MenuOn::Hover).is_some());
        // a left click opens the menu: menu lines and no click command, or asked to
        assert_eq!(reg.menu("m", MenuOn::Click), hello());
        assert_eq!(on("m"), (true, false));
        assert_eq!(on("cm"), (true, false), "menuOn click wins over the click command");
        // the click command keeps the click
        assert_eq!(on("c"), (false, false));
        assert!(reg.click("c", Button::Left, None), "runs `:`");
        // hover menus, unless BarPopover has the hover
        assert_eq!(reg.menu("h", MenuOn::Hover), hello());
        assert_eq!(on("h"), (false, true));
        assert_eq!(on("p"), (false, false));
        assert!(reg.popover("p"));
        // never; no menu lines; not a script
        for k in ["n", "plain", "date", "no-such-widget"] {
            assert_eq!(on(k), (false, false), "{k}");
            assert_eq!(reg.menu(k, MenuOn::None), None, "{k}");
        }
        // the hover highlight: a menu that opens on click or hover counts
        for k in ["m", "c", "cm", "h", "p", "date"] {
            assert!(reg.clickable(k), "{k}");
        }
        assert!(!reg.clickable("n") && !reg.clickable("plain"));
    }

    #[test]
    fn a_menu_item_runs_its_command() {
        let dir = std::env::temp_dir().join(format!("wingbar-menu-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ran");
        run_menu_command("w", &format!("echo yes > '{}'", file.display()));
        run_menu_command("w", "true"); // no action: nothing runs
        let start = std::time::Instant::now();
        while std::fs::read_to_string(&file).ok().as_deref() != Some("yes\n") {
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "the command never ran");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn segment_kinds() {
        let reg = fixture_registry();
        assert_eq!(reg.get("spaces").unwrap().kind(), Kind::Spaces);
        assert_eq!(reg.get("title").unwrap().kind(), Kind::Title);
        assert!(
            reg.keys().filter(|k| !["spaces", "title"].contains(k)).all(|k| reg.get(k).unwrap().kind() == Kind::Item)
        );
    }

    #[test]
    fn the_current_registry_is_empty_until_set() {
        // other tests never set it; refreshing by type then reaches nothing
        refresh_type(WidgetType::Spaces);
        assert!(Registry::empty().keys_of_type(WidgetType::Spaces).is_empty());
    }

    struct Fake(&'static str, std::sync::Mutex<Vec<Result<Show, Failure>>>);
    impl Widget for Fake {
        fn key(&self) -> &str {
            self.0
        }
        fn schedule(&self) -> Every {
            Every::Minute
        }
        fn poll(&self) -> Result<Show, Failure> {
            self.1.lock().unwrap().remove(0)
        }
    }

    #[test]
    fn the_job_records_and_clears_errors() {
        let key = "widgets-test-job";
        let shown = |t: &str| Show::Item(item(None, t.into(), Rgba(1.0, 1.0, 1.0, 1.0), Rgba(1.0, 1.0, 1.0, 1.0)));
        let w = Fake(
            key,
            std::sync::Mutex::new(vec![
                Ok(shown("a")),
                Err(Failure::new("exited with status 1".into(), shown("partial"))),
                Err(Failure::new("timed out after 30s".into(), Show::Keep)),
                Ok(Show::Keep),
                Ok(Show::Hide),
                // from a job made before a reload (another generation): dropped
                Err(Failure::new("late".into(), shown("late"))),
            ]),
        );
        let now = || {
            let st = state::lock();
            (st.items.get(key).map(|i| i.text.clone()), st.errors.get(key).cloned())
        };
        // the bar's state: no test moves its generation on (reload tests use their own)
        let g = state::lock().generation_of(key);
        assert!(run(&w, g, state::cell()));
        assert_eq!(now(), (Some("a".into()), None));
        run(&w, g, state::cell());
        assert_eq!(now(), (Some("partial".into()), Some("exited with status 1".into())), "output kept");
        run(&w, g, state::cell());
        assert_eq!(now(), (Some("partial".into()), Some("timed out after 30s".into())));
        run(&w, g, state::cell());
        assert_eq!(now(), (Some("partial".into()), None), "a success clears the error");
        run(&w, g, state::cell());
        assert_eq!(now(), (None, None));
        assert!(!run(&w, g.wrapping_sub(1), state::cell()), "a late result is dropped");
        assert_eq!(now(), (None, None), "nothing of it written");
    }

    #[test]
    fn rebuilding_keeps_unchanged_widgets() {
        let doc = |every: &str, wing: &str| {
            json!({
                "version": 2,
                "windowManager": "none",
                "right": ["c", "m", "t"],
                "widgets": {
                    "c": { "type": "cpu", "every": every }, "m": { "type": "memory" }, "t": { "type": "time" },
                },
                "theme": { "wing": wing },
            })
        };
        let no_fs = |_: &std::path::Path| false;
        let (a, _) = parse(&doc("2s", "#000000"), &no_fs);
        let ra = Registry::build(&a);
        let (b, _) = parse(&doc("5s", "#000000"), &no_fs);
        let rb = Registry::rebuild(&b, Some((&a, &ra)));
        assert!(!rb.same_widget(&ra, "c"), "its definition changed");
        assert!(rb.same_widget(&ra, "m") && rb.same_widget(&ra, "t"));
        assert_eq!(rb.get("c").unwrap().schedule(), Every::Ms(5000));
        // the theme changed: every widget is made anew (they colour what they show)
        let (c, _) = parse(&doc("5s", "#111111"), &no_fs);
        let rc = Registry::rebuild(&c, Some((&b, &rb)));
        assert!(rc.keys().all(|k| !rc.same_widget(&rb, k)));
        assert!(!rc.same_widget(&rb, "nope"));
    }
}
