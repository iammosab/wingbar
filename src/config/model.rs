//! What the settings file means, once read: the types the rest of the bar uses.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub f64, pub f64, pub f64, pub f64);

impl Rgba {
    /// `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` may be left out).
    pub fn hex(s: &str) -> Option<Rgba> {
        let s = s.trim().trim_start_matches('#');
        let p = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|v| v as f64 / 255.0);
        match s.len() {
            6 => Some(Rgba(p(0)?, p(2)?, p(4)?, 1.0)),
            8 => Some(Rgba(p(0)?, p(2)?, p(4)?, p(6)?)),
            3 => {
                let e: String = s.chars().flat_map(|c| [c, c]).collect();
                Rgba::hex(&e)
            }
            _ => None,
        }
    }
}

/// The bar's colours: the wing colours and the named palette (`theme.colors`).
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub wing: Rgba,
    pub wing_hover: Rgba,
    pub text: Rgba,
    pub dim: Rgba,
    pub faint: Rgba,
    pub warn: Rgba,
    pub alert: Rgba,
    /// `theme.colors` by name, without a leading `--` ("red", "main-alt", ...).
    pub named: HashMap<String, Rgba>,
    /// The icons built-in widgets draw with (`theme.icons`).
    pub icons: IconSet,
    /// How widget menus are drawn (`theme.menus`).
    pub menus: MenuStyle,
}

/// `theme.menus`: how a widget's menu (a script's `menu:` lines) is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuStyle {
    /// `"wings"`: drawn by wingbar in the bar's colours, hanging from the wing.
    #[default]
    Wings,
    /// `"native"`: a standard macOS menu (VoiceOver reads it like any menu).
    Native,
}

/// `theme.icons`: the icons built-in widgets draw with. Any icon option can
/// still name either kind (see `crate::icons`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconSet {
    /// `"sf"`: SF Symbols.
    #[default]
    Sf,
    /// `"simple-bar"`: the bundled simple-bar icons.
    SimpleBar,
}

impl IconSet {
    /// Pick the icon for this set.
    pub fn pick<'a>(self, sf: &'a str, simple_bar: &'a str) -> &'a str {
        match self {
            IconSet::Sf => sf,
            IconSet::SimpleBar => simple_bar,
        }
    }
}

/// The theme keys a colour may name, as written in the file.
pub const THEME_KEYS: &[&str] = &["wing", "wingHover", "text", "dim", "faint", "warn", "alert"];

impl Theme {
    /// A colour as written anywhere in the settings: a theme key (`warn`,
    /// `dim`, ...), a `theme.colors` name, or `#rgb` / `#rrggbb` / `#rrggbbaa`;
    /// names may start with `--`. `None` if it is none of those.
    pub fn color(&self, s: &str) -> Option<Rgba> {
        let s = s.trim();
        let name = s.strip_prefix("--").unwrap_or(s);
        let key = match name {
            "wing" => Some(self.wing),
            "wingHover" => Some(self.wing_hover),
            "text" => Some(self.text),
            "dim" => Some(self.dim),
            "faint" => Some(self.faint),
            "warn" => Some(self.warn),
            "alert" => Some(self.alert),
            _ => None,
        };
        key.or_else(|| self.named.get(name).copied()).or_else(|| Rgba::hex(s))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wm {
    Aerospace(String),
    Yabai(String),
    None,
}

/// The `bar` section.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    /// `None`: follow the screen.
    pub font_size: Option<f64>,
    pub title_max: f64,
    pub level_above_windows: bool,
    pub volume_step: f64,
    pub port: u16,
    pub server: bool,
    pub hide_empty_spaces: bool,
    pub space_exclusions: Vec<String>,
    /// BarPopover's binary, when set and present.
    pub popover_bin: Option<String>,
    pub popover_daemon: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wing {
    Left,
    Right,
}

impl Wing {
    pub fn name(self) -> &'static str {
        match self {
            Wing::Left => "left",
            Wing::Right => "right",
        }
    }
    pub fn other(self) -> Wing {
        match self {
            Wing::Left => Wing::Right,
            Wing::Right => Wing::Left,
        }
    }
}

/// The built-in widget types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WidgetType {
    Spaces,
    Title,
    Cpu,
    Memory,
    Battery,
    Sound,
    Date,
    Time,
    Spotify,
    Music,
    Script,
}

impl WidgetType {
    pub const ALL: &[WidgetType] = &[
        WidgetType::Spaces,
        WidgetType::Title,
        WidgetType::Cpu,
        WidgetType::Memory,
        WidgetType::Battery,
        WidgetType::Sound,
        WidgetType::Date,
        WidgetType::Time,
        WidgetType::Spotify,
        WidgetType::Music,
        WidgetType::Script,
    ];

    pub fn name(self) -> &'static str {
        match self {
            WidgetType::Spaces => "spaces",
            WidgetType::Title => "title",
            WidgetType::Cpu => "cpu",
            WidgetType::Memory => "memory",
            WidgetType::Battery => "battery",
            WidgetType::Sound => "sound",
            WidgetType::Date => "date",
            WidgetType::Time => "time",
            WidgetType::Spotify => "spotify",
            WidgetType::Music => "music",
            WidgetType::Script => "script",
        }
    }

    pub fn parse(s: &str) -> Option<WidgetType> {
        WidgetType::ALL.iter().copied().find(|t| t.name() == s)
    }

    /// The options a definition of this type may have (besides `type`).
    pub fn options(self) -> &'static [&'static str] {
        match self {
            WidgetType::Spaces | WidgetType::Title | WidgetType::Sound | WidgetType::Spotify | WidgetType::Music => &[],
            WidgetType::Cpu | WidgetType::Memory => &["every", "click"],
            WidgetType::Battery => &["click", "caffeinateOption"],
            WidgetType::Date => &["click"],
            WidgetType::Time => &["hour12", "seconds", "click"],
            WidgetType::Script => &[
                "command",
                "every",
                "timeout",
                "icon",
                "levels",
                "click",
                "rightClick",
                "middleClick",
                "menuOn",
                "popover",
                "popoverTab",
                "hideWhenEmpty",
                "iconOnly",
                "greyWhenOk",
                "iconFromLabel",
                "prelude",
                "cacheFile",
                "simplebarIds",
                "raw",
            ],
        }
    }

    /// The wing a widget of this type goes to when none is named.
    pub fn default_wing(self) -> Wing {
        match self {
            WidgetType::Spaces | WidgetType::Title => Wing::Left,
            _ => Wing::Right,
        }
    }
}

/// A sampled reading (cpu, memory).
#[derive(Clone, Debug, PartialEq)]
pub struct Sampled {
    pub every_ms: u64,
    /// A shell command; empty for none.
    pub click: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatteryOptions {
    /// `"caffeinate"` toggles caffeinate; any other text is a shell command; empty for none.
    pub click: String,
    /// Arguments for caffeinate(8); empty means `-d -i`.
    pub caffeinate_option: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimeOptions {
    pub hour12: bool,
    pub seconds: bool,
    pub click: String,
}

/// One state a script can report (`level|label`).
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub name: String,
    /// `None`: the first level's colour, else `theme.text`.
    pub color: Option<Rgba>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clicks {
    pub left: String,
    pub right: String,
    pub middle: String,
}

/// When a script widget's menu opens (`menuOn`; menus come in a later version).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuOn {
    Click,
    Hover,
    None,
}

/// A rule that picks the icon from the label (`iconFromLabel`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LabelIcons {
    /// The next prayer's name → a sun / moon symbol.
    Prayer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptOptions {
    pub command: String,
    pub every_ms: u64,
    pub timeout_ms: u64,
    pub icon: Option<String>,
    /// In the file's order: the first is the fallback for unknown levels.
    pub levels: Vec<Level>,
    pub clicks: Clicks,
    /// `None`: decided by the output (see the spec).
    pub menu_on: Option<MenuOn>,
    pub popover: bool,
    /// BarPopover's tab (its "still to tick" prayer tab) hangs under this
    /// widget: the bar tells BarPopover where the widget is.
    pub popover_tab: bool,
    pub hide_when_empty: bool,
    pub icon_only: bool,
    pub grey_when_ok: bool,
    pub icon_from_label: Option<LabelIcons>,
    /// A file sourced before the command (`~` and `$HOME` expanded).
    pub prelude: Option<String>,
    /// Where line 1 is copied after each run (`~`, `$HOME`, `$TMPDIR` expanded).
    pub cache_file: Option<String>,
    pub simplebar_ids: Vec<u32>,
    /// The output is no protocol: the label is the first line of the trimmed
    /// output as it is, level "ok", no `icon:` or `menu:` lines (0.2's plain
    /// user widgets).
    pub raw: bool,
}

/// A definition's type and options.
#[derive(Clone, Debug, PartialEq)]
pub enum Options {
    Spaces,
    Title,
    Cpu(Sampled),
    Memory(Sampled),
    Battery(BatteryOptions),
    Sound,
    /// The date's click command.
    Date(String),
    Time(TimeOptions),
    Spotify,
    Music,
    Script(Box<ScriptOptions>),
}

impl Options {
    pub fn widget_type(&self) -> WidgetType {
        match self {
            Options::Spaces => WidgetType::Spaces,
            Options::Title => WidgetType::Title,
            Options::Cpu(_) => WidgetType::Cpu,
            Options::Memory(_) => WidgetType::Memory,
            Options::Battery(_) => WidgetType::Battery,
            Options::Sound => WidgetType::Sound,
            Options::Date(_) => WidgetType::Date,
            Options::Time(_) => WidgetType::Time,
            Options::Spotify => WidgetType::Spotify,
            Options::Music => WidgetType::Music,
            Options::Script(_) => WidgetType::Script,
        }
    }
}

/// One entry of `widgets`.
#[derive(Clone, Debug, PartialEq)]
pub struct WidgetDef {
    pub key: String,
    pub options: Options,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub wm: Wm,
    pub theme: Theme,
    pub bar: Bar,
    /// Placed keys, in order: each one defined, and placed once.
    pub left: Vec<String>,
    pub right: Vec<String>,
    /// Every definition with a known type, in the file's order (placed or hidden).
    pub widgets: Vec<WidgetDef>,
}

impl Config {
    pub fn def(&self, key: &str) -> Option<&WidgetDef> {
        self.widgets.iter().find(|d| d.key == key)
    }

    pub fn wing_of(&self, key: &str) -> Option<Wing> {
        if self.left.iter().any(|k| k == key) {
            Some(Wing::Left)
        } else if self.right.iter().any(|k| k == key) {
            Some(Wing::Right)
        } else {
            None
        }
    }

    /// The placed definitions, left wing then right wing, in order.
    pub fn placed(&self) -> impl Iterator<Item = (Wing, &WidgetDef)> {
        let left = self.left.iter().filter_map(|k| Some((Wing::Left, self.def(k)?)));
        left.chain(self.right.iter().filter_map(|k| Some((Wing::Right, self.def(k)?))))
    }

    /// Defined, but in neither wing.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn hidden(&self) -> impl Iterator<Item = &WidgetDef> {
        self.widgets.iter().filter(|d| self.wing_of(&d.key).is_none())
    }
}
