//! Changes to the settings document, shared by the bar's menu and the
//! `wingbar widget …` commands: add, remove, show, hide, move, set. Each one
//! works on the parsed document (so the rest of the file stays as it was),
//! checks its input, and leaves the document unchanged when it fails. The
//! error says what is wrong and how to fix it.

use super::model::{WidgetType, Wing};
use super::parse::{parse, valid_key};
use serde_json::{Map, Value};

/// Where in a wing a widget goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Position {
    /// At the end of the wing.
    End,
    /// Just before this (placed) key.
    Before(String),
    /// Just after this (placed) key.
    After(String),
    /// At this index (0 = first); at most the wing's length.
    At(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditError(pub String);

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

type Result<T> = std::result::Result<T, EditError>;

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(EditError(msg.into()))
}

/// A value given on the command line: JSON if it reads as JSON, else the text itself.
pub fn parse_value(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

/// A key a new widget may take: valid, and not used yet.
pub fn check_new_key(doc: &Value, key: &str) -> Result<()> {
    if !valid_key(key) {
        let suggestion: String = key
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
            .collect();
        return err(format!("{key:?} can't be a widget name: use only a-z, 0-9, _ and - (for example {suggestion:?})"));
    }
    if defined(doc, key) || placement(doc, key).is_some() {
        return err(format!("a widget named {key:?} already exists; pick another name (--as NAME)"));
    }
    Ok(())
}

/// Which wing a key is placed in, and where.
pub fn placement(doc: &Value, key: &str) -> Option<(Wing, usize)> {
    [Wing::Left, Wing::Right].into_iter().find_map(|w| {
        let list = doc.get(w.name())?.as_array()?;
        list.iter().position(|v| v.as_str() == Some(key)).map(|i| (w, i))
    })
}

fn defined(doc: &Value, key: &str) -> bool {
    doc.get("widgets").and_then(|w| w.get(key)).is_some()
}

fn def_type(doc: &Value, key: &str) -> Option<WidgetType> {
    doc.get("widgets")?.get(key)?.get("type")?.as_str().and_then(WidgetType::parse)
}

fn root(doc: &mut Value) -> Result<&mut Map<String, Value>> {
    match doc {
        Value::Object(m) => Ok(m),
        _ => err("the settings file is not a JSON object; fix it by hand (it should start with '{')"),
    }
}

fn wing_mut(doc: &mut Value, w: Wing) -> Result<&mut Vec<Value>> {
    let m = root(doc)?;
    let v = m.entry(w.name()).or_insert_with(|| Value::Array(vec![]));
    match v {
        Value::Array(a) => Ok(a),
        _ => err(format!("\"{}\" in the settings file is not a list; fix it by hand", w.name())),
    }
}

fn widgets_mut(doc: &mut Value) -> Result<&mut Map<String, Value>> {
    let m = root(doc)?;
    let v = m.entry("widgets").or_insert_with(|| Value::Object(Map::new()));
    match v {
        Value::Object(o) => Ok(o),
        _ => err("\"widgets\" in the settings file is not an object; fix it by hand"),
    }
}

fn unplace(doc: &mut Value, key: &str) -> Result<()> {
    for w in [Wing::Left, Wing::Right] {
        if doc.get(w.name()).is_some() {
            wing_mut(doc, w)?.retain(|v| v.as_str() != Some(key));
        }
    }
    Ok(())
}

/// The wing a placement lands in: the one named, else the wing of the key in
/// `pos`, else `fallback`. Checks that the key in `pos` is in that wing.
fn target_wing(doc: &Value, key: &str, wing: Option<Wing>, pos: &Position, fallback: Wing) -> Result<Wing> {
    let reference = match pos {
        Position::Before(r) | Position::After(r) => r,
        _ => return Ok(wing.unwrap_or(fallback)),
    };
    if reference == key {
        return err(format!("{key:?} can't be placed next to itself; name another widget"));
    }
    match (placement(doc, reference), wing) {
        (None, _) => err(format!(
            "{reference:?} is not in either wing, so nothing can go next to it; name a shown widget (see `wingbar widget list`)"
        )),
        (Some((rw, _)), Some(w)) if rw != w => err(format!(
            "{reference:?} is in the {} wing, not the {}; leave out --{} or name a widget in the {} wing",
            rw.name(),
            w.name(),
            w.name(),
            w.name()
        )),
        (Some((rw, _)), _) => Ok(rw),
    }
}

/// Put `key` into wing `w` at `pos` (it must not be placed already).
fn insert(doc: &mut Value, key: &str, w: Wing, pos: &Position) -> Result<()> {
    let list = wing_mut(doc, w)?;
    let index_of = |list: &Vec<Value>, r: &str| list.iter().position(|v| v.as_str() == Some(r));
    let i = match pos {
        Position::End => list.len(),
        Position::At(n) if *n <= list.len() => *n,
        Position::At(n) => {
            return err(format!(
                "position {n} is past the end of the {} wing (it has {} widgets); use a number from 0 to {}, or leave it out for the end",
                w.name(),
                list.len(),
                list.len()
            ));
        }
        Position::Before(r) => {
            index_of(list, r).ok_or_else(|| EditError(format!("{r:?} is not in the {} wing", w.name())))?
        }
        Position::After(r) => {
            index_of(list, r).ok_or_else(|| EditError(format!("{r:?} is not in the {} wing", w.name())))? + 1
        }
    };
    list.insert(i, Value::String(key.to_string()));
    Ok(())
}

/// Run `f` on a copy of `doc`; keep the result only if it worked and the file
/// reads without any new warning (which then becomes the error).
fn checked<T>(doc: &mut Value, f: impl FnOnce(&mut Value) -> Result<T>) -> Result<T> {
    let mut new = doc.clone();
    let out = f(&mut new)?;
    let no_fs = |_: &std::path::Path| false;
    let (_, before) = parse(doc, &no_fs);
    let (_, after) = parse(&new, &no_fs);
    let fresh: Vec<String> = after.into_iter().filter(|w| !before.contains(w)).collect();
    if !fresh.is_empty() {
        return err(fresh.join("; "));
    }
    *doc = new;
    Ok(out)
}

/// Define `key` as `def` and place it in `wing` (default: the type's usual
/// wing) at `pos`. Returns the wing it went to.
pub fn add(doc: &mut Value, key: &str, def: Value, wing: Option<Wing>, pos: &Position) -> Result<Wing> {
    check_new_key(doc, key)?;
    let Some(t) = def.get("type").and_then(Value::as_str) else {
        return err("a new widget needs a \"type\"");
    };
    let Some(t) = WidgetType::parse(t) else {
        let known: Vec<&str> = WidgetType::ALL.iter().map(|t| t.name()).collect();
        return err(format!("unknown widget type {t:?}; use one of: {}", known.join(", ")));
    };
    checked(doc, |doc| {
        let w = target_wing(doc, key, wing, pos, t.default_wing())?;
        widgets_mut(doc)?.insert(key.to_string(), def);
        insert(doc, key, w, pos)?;
        Ok(w)
    })
}

/// Remove `key`'s definition and placement. (Script files are never touched.)
pub fn remove(doc: &mut Value, key: &str) -> Result<()> {
    if !defined(doc, key) && placement(doc, key).is_none() {
        return err(format!("there is no widget named {key:?} (see `wingbar widget list`)"));
    }
    checked(doc, |doc| {
        unplace(doc, key)?;
        if defined(doc, key) {
            widgets_mut(doc)?.shift_remove(key);
        }
        Ok(())
    })
}

/// Place a hidden widget (default: its type's usual wing, at the end). A shown
/// widget given a wing or position moves there.
pub fn show(doc: &mut Value, key: &str, wing: Option<Wing>, pos: &Position) -> Result<Wing> {
    let Some(t) = def_type(doc, key) else {
        return err(not_defined(doc, key));
    };
    if let Some((w, _)) = placement(doc, key) {
        if wing.is_none() && *pos == Position::End {
            return err(format!("{key:?} is already shown (in the {} wing); use move to move it", w.name()));
        }
        return move_to(doc, key, wing, pos);
    }
    checked(doc, |doc| {
        let w = target_wing(doc, key, wing, pos, t.default_wing())?;
        insert(doc, key, w, pos)?;
        Ok(w)
    })
}

/// Take a widget out of its wing; its definition stays.
pub fn hide(doc: &mut Value, key: &str) -> Result<()> {
    if !defined(doc, key) {
        return err(not_defined(doc, key));
    }
    if placement(doc, key).is_none() {
        return err(format!("{key:?} is already hidden"));
    }
    checked(doc, |doc| unplace(doc, key))
}

/// Move a shown widget within its wing or to the other one (`wing`); `pos` as for `add`.
pub fn move_to(doc: &mut Value, key: &str, wing: Option<Wing>, pos: &Position) -> Result<Wing> {
    let Some((from, _)) = placement(doc, key) else {
        return if defined(doc, key) {
            err(format!("{key:?} is hidden; use show to place it"))
        } else {
            err(not_defined(doc, key))
        };
    };
    checked(doc, |doc| {
        let w = target_wing(doc, key, wing, pos, from)?;
        unplace(doc, key)?;
        insert(doc, key, w, pos)?;
        Ok(w)
    })
}

/// Set one option of a definition (`null` removes it, back to its default).
pub fn set_option(doc: &mut Value, key: &str, option: &str, value: Value) -> Result<()> {
    let Some(t) = def_type(doc, key) else {
        return err(not_defined(doc, key));
    };
    if option == "type" {
        return err("a widget's type can't be changed; remove it and add a new one");
    }
    if !t.options().contains(&option) && !option.starts_with("//") {
        let opts = t.options();
        return err(if opts.is_empty() {
            format!("a {} widget has no options", t.name())
        } else {
            format!("a {} widget has no option {option:?}; its options are: {}", t.name(), opts.join(", "))
        });
    }
    checked(doc, |doc| {
        let Some(Value::Object(def)) = widgets_mut(doc)?.get_mut(key) else {
            return err(format!("widgets.{key} is not an object; fix it by hand"));
        };
        if value.is_null() {
            def.shift_remove(option);
        } else {
            def.insert(option.to_string(), value);
        }
        Ok(())
    })
}

fn not_defined(doc: &Value, key: &str) -> String {
    if placement(doc, key).is_some() {
        format!("{key:?} is placed but has no definition in \"widgets\"; add one, or remove it")
    } else {
        format!("there is no widget named {key:?} (see `wingbar widget list`)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({
            "version": 2,
            "// keep": "me",
            "left": ["spaces", "title"],
            "right": ["cpu", "date", "time"],
            "widgets": {
                "spaces": { "type": "spaces" },
                "title": { "type": "title" },
                "cpu": { "type": "cpu", "every": "2s" },
                "date": { "type": "date" },
                "time": { "type": "time" },
                "memory": { "type": "memory" },
            },
            "unknownButKept": 1,
        })
    }

    fn wings(d: &Value) -> (Vec<&str>, Vec<&str>) {
        let l = |w: &str| d[w].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        (l("left"), l("right"))
    }

    #[test]
    fn values_from_the_command_line() {
        assert_eq!(parse_value("5"), json!(5));
        assert_eq!(parse_value("true"), json!(true));
        assert_eq!(parse_value("{\"color\":\"red\"}"), json!({ "color": "red" }));
        assert_eq!(parse_value("\"x\""), json!("x"));
        assert_eq!(parse_value("open -a Calendar"), json!("open -a Calendar"));
        assert_eq!(parse_value("2s"), json!("2s"));
    }

    #[test]
    fn add_at_the_end_before_after_and_at() {
        let mut d = doc();
        let def = json!({ "type": "time", "seconds": true });
        assert_eq!(add(&mut d, "clock2", def.clone(), None, &Position::End), Ok(Wing::Right));
        assert_eq!(wings(&d).1, ["cpu", "date", "time", "clock2"]);
        assert_eq!(d["widgets"]["clock2"], def);
        assert_eq!(
            add(&mut d, "a", json!({ "type": "sound" }), None, &Position::Before("date".into())),
            Ok(Wing::Right)
        );
        assert_eq!(
            add(&mut d, "b", json!({ "type": "sound" }), None, &Position::After("spaces".into())),
            Ok(Wing::Left)
        );
        assert_eq!(add(&mut d, "c", json!({ "type": "sound" }), Some(Wing::Left), &Position::At(0)), Ok(Wing::Left));
        assert_eq!(
            add(&mut d, "d", json!({ "type": "spaces" }), None, &Position::End),
            Ok(Wing::Left),
            "its usual wing"
        );
        assert_eq!(wings(&d), (vec!["c", "spaces", "b", "title", "d"], vec!["cpu", "a", "date", "time", "clock2"]));
        // the rest of the document is untouched, in order
        let keys: Vec<&String> = d.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["version", "// keep", "left", "right", "widgets", "unknownButKept"]);
        let wkeys: Vec<&String> = d["widgets"].as_object().unwrap().keys().collect();
        assert_eq!(wkeys[..6], ["spaces", "title", "cpu", "date", "time", "memory"]);
    }

    #[test]
    fn add_refuses_bad_input_and_changes_nothing() {
        let mut d = doc();
        let before = d.clone();
        let e = |r: Result<Wing>| r.unwrap_err().0;
        let s = || json!({ "type": "sound" });
        assert!(e(add(&mut d, "cpu", s(), None, &Position::End)).contains("already exists"));
        let bad = e(add(&mut d, "My Widget", s(), None, &Position::End));
        assert!(bad.contains("a-z, 0-9") && bad.contains("\"my-widget\""), "{bad}");
        assert!(e(add(&mut d, "", s(), None, &Position::End)).contains("can't be a widget name"));
        assert!(e(add(&mut d, "x", json!({}), None, &Position::End)).contains("needs a \"type\""));
        let t = e(add(&mut d, "x", json!({ "type": "weather" }), None, &Position::End));
        assert!(t.contains("unknown widget type") && t.contains("script"), "{t}");
        let p = e(add(&mut d, "x", s(), None, &Position::At(9)));
        assert!(p.contains("past the end") && p.contains("0 to 3"), "{p}");
        assert!(e(add(&mut d, "x", s(), None, &Position::Before("memory".into()))).contains("not in either wing"));
        let w = e(add(&mut d, "x", s(), Some(Wing::Left), &Position::After("cpu".into())));
        assert!(w.contains("in the right wing, not the left"), "{w}");
        // an option of the wrong type is caught by reading the result
        let o = e(add(&mut d, "x", json!({ "type": "cpu", "every": "soon" }), None, &Position::End));
        assert!(o.contains("widgets.x.every"), "{o}");
        let u = e(add(&mut d, "x", json!({ "type": "cpu", "colour": "red" }), None, &Position::End));
        assert!(u.contains("unknown key \"colour\""), "{u}");
        assert_eq!(d, before);
    }

    #[test]
    fn remove_takes_the_definition_and_the_placement() {
        let mut d = doc();
        remove(&mut d, "date").unwrap();
        assert_eq!(wings(&d).1, ["cpu", "time"]);
        assert!(d["widgets"].get("date").is_none());
        remove(&mut d, "memory").unwrap();
        assert!(d["widgets"].get("memory").is_none(), "a hidden one too");
        assert!(remove(&mut d, "date").unwrap_err().0.contains("no widget named \"date\""));
        // a placed key without a definition can be removed (it only warned)
        d["right"].as_array_mut().unwrap().push(json!("ghost"));
        remove(&mut d, "ghost").unwrap();
        assert_eq!(wings(&d).1, ["cpu", "time"]);
        let keys: Vec<&String> = d["widgets"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["spaces", "title", "cpu", "time"], "order kept");
    }

    #[test]
    fn show_and_hide() {
        let mut d = doc();
        assert_eq!(show(&mut d, "memory", None, &Position::After("cpu".into())), Ok(Wing::Right));
        assert_eq!(wings(&d).1, ["cpu", "memory", "date", "time"]);
        hide(&mut d, "memory").unwrap();
        assert_eq!(wings(&d).1, ["cpu", "date", "time"]);
        assert!(d["widgets"].get("memory").is_some(), "hidden, still defined");
        assert_eq!(show(&mut d, "memory", None, &Position::End), Ok(Wing::Right));
        assert_eq!(wings(&d).1, ["cpu", "date", "time", "memory"]);
        assert!(show(&mut d, "memory", None, &Position::End).unwrap_err().0.contains("already shown"));
        // with a place, showing a shown widget moves it
        assert_eq!(show(&mut d, "memory", Some(Wing::Left), &Position::At(0)), Ok(Wing::Left));
        assert_eq!(wings(&d).0, ["memory", "spaces", "title"]);
        hide(&mut d, "memory").unwrap();
        assert!(hide(&mut d, "memory").unwrap_err().0.contains("already hidden"));
        assert!(hide(&mut d, "nope").unwrap_err().0.contains("no widget named"));
        assert!(show(&mut d, "nope", None, &Position::End).unwrap_err().0.contains("no widget named"));
    }

    #[test]
    fn move_within_and_between_wings() {
        let mut d = doc();
        assert_eq!(move_to(&mut d, "time", None, &Position::At(0)), Ok(Wing::Right));
        assert_eq!(wings(&d).1, ["time", "cpu", "date"]);
        assert_eq!(move_to(&mut d, "time", None, &Position::End), Ok(Wing::Right));
        assert_eq!(wings(&d).1, ["cpu", "date", "time"]);
        assert_eq!(move_to(&mut d, "cpu", None, &Position::After("date".into())), Ok(Wing::Right));
        assert_eq!(wings(&d).1, ["date", "cpu", "time"]);
        assert_eq!(move_to(&mut d, "cpu", Some(Wing::Left), &Position::End), Ok(Wing::Left));
        assert_eq!(wings(&d), (vec!["spaces", "title", "cpu"], vec!["date", "time"]));
        assert_eq!(
            move_to(&mut d, "cpu", None, &Position::Before("time".into())),
            Ok(Wing::Right),
            "follows its neighbour"
        );
        assert_eq!(wings(&d), (vec!["spaces", "title"], vec!["date", "cpu", "time"]));
        // At counts the wing without the widget being moved
        assert_eq!(move_to(&mut d, "date", None, &Position::At(2)), Ok(Wing::Right));
        assert_eq!(wings(&d).1, ["cpu", "time", "date"]);
        let e = |r: Result<Wing>| r.unwrap_err().0;
        assert!(e(move_to(&mut d, "memory", None, &Position::End)).contains("hidden; use show"));
        assert!(e(move_to(&mut d, "cpu", None, &Position::Before("cpu".into()))).contains("next to itself"));
        assert!(e(move_to(&mut d, "nope", None, &Position::End)).contains("no widget named"));
        assert!(e(move_to(&mut d, "cpu", None, &Position::At(3))).contains("past the end"));
    }

    #[test]
    fn set_options() {
        let mut d = doc();
        set_option(&mut d, "cpu", "every", json!("5s")).unwrap();
        assert_eq!(d["widgets"]["cpu"]["every"], json!("5s"));
        set_option(&mut d, "cpu", "click", json!("")).unwrap();
        set_option(&mut d, "cpu", "every", Value::Null).unwrap();
        assert_eq!(d["widgets"]["cpu"], json!({ "type": "cpu", "click": "" }), "null removes");
        let e = |r: Result<()>| r.unwrap_err().0;
        let o = e(set_option(&mut d, "cpu", "colour", json!("red")));
        assert!(o.contains("no option \"colour\"") && o.contains("every, click"), "{o}");
        assert!(e(set_option(&mut d, "title", "x", json!(1))).contains("has no options"));
        assert!(e(set_option(&mut d, "cpu", "type", json!("memory"))).contains("can't be changed"));
        assert!(e(set_option(&mut d, "nope", "every", json!("5s"))).contains("no widget named"));
        let bad = e(set_option(&mut d, "cpu", "every", json!(true)));
        assert!(bad.contains("invalid widgets.cpu.every = true"), "{bad}");
        assert_eq!(d["widgets"]["cpu"], json!({ "type": "cpu", "click": "" }), "unchanged after an error");
        // scripts: levels as an object
        add(&mut d, "net", json!({ "type": "script", "command": "echo ok" }), None, &Position::End).unwrap();
        set_option(
            &mut d,
            "net",
            "levels",
            json!({ "ok": "green", "eth": { "color": "blue", "icon": "cable.connector" } }),
        )
        .unwrap();
        let c = e(set_option(&mut d, "net", "levels", json!({ "ok": "no-such-colour" })));
        assert!(c.contains("levels.ok"), "{c}");
    }

    #[test]
    fn broken_documents_say_so() {
        let mut d = json!([1, 2]);
        assert!(
            add(&mut d, "x", json!({ "type": "cpu" }), None, &Position::End)
                .unwrap_err()
                .0
                .contains("not a JSON object")
        );
        let mut d = json!({ "version": 2, "right": "cpu", "widgets": {} });
        let e = add(&mut d, "x", json!({ "type": "cpu" }), None, &Position::End).unwrap_err().0;
        assert!(e.contains("\"right\"") && e.contains("not a list"), "{e}");
        // missing lists and widgets are created
        let mut d = json!({ "version": 2 });
        add(&mut d, "x", json!({ "type": "cpu" }), None, &Position::End).unwrap();
        assert_eq!(d["right"], json!(["x"]));
    }

    #[test]
    fn placed_without_definition() {
        let mut d = doc();
        d["left"].as_array_mut().unwrap().push(json!("ghost"));
        assert!(hide(&mut d, "ghost").unwrap_err().0.contains("placed but has no definition"));
        assert!(check_new_key(&d, "ghost").unwrap_err().0.contains("already exists"));
    }
}
