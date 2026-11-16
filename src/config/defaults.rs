//! The settings a fresh install starts with, and the pieces the import and the
//! edit operations share with it.

use super::model::WidgetType;
use super::parse::{DEFAULT_POPOVER_DAEMON, DEFAULT_PORT, THEME_DEFAULTS};
use serde_json::{Map, Value, json};

/// The `bar` section with every default written out.
pub fn bar() -> Value {
    json!({
        "fontSize": null,
        "titleMaxWidth": 420,
        "windowLevel": "above",
        "volumeStep": 5,
        "port": DEFAULT_PORT,
        "server": true,
        "hideEmptySpaces": true,
        "spacesExclusions": [],
        "popoverBinary": "",
        "popoverDaemon": DEFAULT_POPOVER_DAEMON,
    })
}

/// The wing colours with their defaults, and `colors`.
pub fn theme(colors: Map<String, Value>) -> Value {
    let mut t = Map::new();
    for (k, v) in THEME_DEFAULTS {
        t.insert(k.to_string(), json!(v));
    }
    t.insert("colors".into(), Value::Object(colors));
    Value::Object(t)
}

/// A new definition of a built-in type, with its usual options.
pub fn definition(t: WidgetType) -> Value {
    match t {
        WidgetType::Cpu => json!({ "type": "cpu", "every": "2s", "click": "open -a 'Activity Monitor'" }),
        WidgetType::Memory => json!({ "type": "memory", "every": "4s", "click": "open -a 'Activity Monitor'" }),
        WidgetType::Battery => json!({ "type": "battery", "click": "caffeinate" }),
        WidgetType::Date => json!({ "type": "date", "click": "open -a Calendar" }),
        WidgetType::Time => json!({ "type": "time", "hour12": false, "seconds": false }),
        WidgetType::Script => json!({ "type": "script", "command": "", "every": "10s" }),
        t => json!({ "type": t.name() }),
    }
}

/// A fresh install's settings: the workspaces (when a window manager was
/// found), the window title, cpu, memory, battery, sound, date and time.
pub fn fresh(wm_found: bool) -> Value {
    let mut left = vec![];
    if wm_found {
        left.push("spaces");
    }
    left.push("title");
    let right = ["cpu", "memory", "battery", "sound", "date", "time"];
    let mut widgets = Map::new();
    for k in left.iter().chain(right.iter()) {
        let t = WidgetType::parse(k).expect("built-in type");
        widgets.insert(k.to_string(), definition(t));
    }
    let mut colors = Map::new();
    for (n, c) in [("green", "#9ed072"), ("blue", "#76cce0"), ("red", "#fc5d7c")] {
        colors.insert(n.into(), json!(c));
    }
    json!({
        "version": 2,
        "windowManager": "auto",
        "bar": bar(),
        "theme": theme(colors),
        "left": left,
        "right": right,
        "widgets": widgets,
    })
}
