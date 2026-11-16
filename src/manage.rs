//! Managing widgets: the one set of operations behind both the bar's
//! right-click menu and the `wingbar widget …` commands, so the two can never
//! disagree (docs/design/widgets-v2.md, "Managing widgets").
//!
//! Each operation reads the settings file, applies one pure edit
//! (`config::edit`), writes the file back atomically and returns a message for
//! people ("Added “storage” to the right wing (position 7)."). An error names
//! the problem and the fix; the file is then left as it was. Telling a running
//! bar to reload is the caller's part (`reload::reload` in the bar, `GET /reload`
//! from the command line).

use crate::catalog;
use crate::config::doc::Document;
use crate::config::edit::{self, EditError, Position};
use crate::config::{self, WidgetType, Wing, defaults};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The files the operations work on.
#[derive(Clone, Debug)]
pub struct Files {
    /// The settings file.
    pub settings: PathBuf,
    /// Where catalog scripts are installed.
    pub widgets_dir: PathBuf,
}

impl Files {
    /// The settings in use (`$WINGBAR_CONFIG` or `~/.config/wingbar/config.json`).
    pub fn current() -> Files {
        Files { settings: config::settings_path(), widgets_dir: catalog::widgets_dir() }
    }
}

/// What to add, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct Add {
    /// A built-in type (`cpu`), a catalog name (`storage`) or `script`.
    pub what: String,
    /// The key to use; by default `what`, or `what-2`, `what-3`, ... when taken.
    pub key: Option<String>,
    pub wing: Option<Wing>,
    pub pos: Position,
    pub every: Option<String>,
    pub command: Option<String>,
}

impl Add {
    pub fn new(what: impl Into<String>) -> Add {
        Add { what: what.into(), key: None, wing: None, pos: Position::End, every: None, command: None }
    }
}

/// One step for the menu's Move items.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Left,
    Right,
    /// Across the middle of the bar: to the start of the right wing, or the
    /// end of the left one (the nearest end).
    OtherWing,
}

type Result<T> = std::result::Result<T, String>;

fn msg(e: EditError) -> String {
    e.0
}

fn quoted(key: &str) -> String {
    format!("“{key}”")
}

/// The settings document, ready to edit: it must exist, be a JSON object, and
/// be a version 2 file.
fn load(f: &Files) -> Result<Document> {
    let shown = f.settings.display();
    if !f.settings.exists() {
        return Err(format!(
            "there is no settings file at {shown} yet; start wingbar once (it writes one), \
             or run `wingbar config import-simplebar`"
        ));
    }
    let d = Document::load(&f.settings)?;
    if !d.value.is_object() {
        return Err(format!("{shown} is not a JSON object; fix it by hand (`wingbar config edit`)"));
    }
    if config::is_v1(&d.value) {
        return Err(format!(
            "{shown} is a wingbar 0.2 file; start the bar once to convert it, or run `wingbar config import-simplebar --force`"
        ));
    }
    Ok(d)
}

/// The keys placed in wing `w`, in order.
fn wing_keys(doc: &Value, w: Wing) -> Vec<String> {
    doc.get(w.name())
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn def_type(doc: &Value, key: &str) -> Option<String> {
    doc.get("widgets")?.get(key)?.get("type")?.as_str().map(String::from)
}

/// "the right wing (position 3)": where `key` is now.
fn where_now(doc: &Value, key: &str) -> String {
    match edit::placement(doc, key) {
        Some((w, i)) => format!("the {} wing (position {})", w.name(), i + 1),
        None => "neither wing".into(),
    }
}

/// `base`, or `base-2`, `base-3`, ... : the first one free for a new widget.
pub fn free_key(doc: &Value, base: &str) -> String {
    if edit::check_new_key(doc, base).is_ok() {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}-{n}")).find(|k| edit::check_new_key(doc, k).is_ok()).expect("a free key")
}

/// `--at` is past the end of wing `w` (counting without `moving`, which is
/// taken out first): say so in the 1-based terms people use.
fn check_at(doc: &Value, moving: Option<&str>, w: Wing, pos: &Position) -> Result<()> {
    let Position::At(n) = pos else { return Ok(()) };
    let len = wing_keys(doc, w).iter().filter(|k| Some(k.as_str()) != moving).count();
    if *n > len {
        return Err(format!(
            "position {} is past the end of the {} wing; use 1 to {} (or leave --at out for the end)",
            n + 1,
            w.name(),
            len + 1
        ));
    }
    Ok(())
}

/// What `Add::what` names.
enum Source {
    Builtin(WidgetType),
    Catalog(&'static catalog::Entry),
    Custom,
}

fn source(what: &str) -> Result<Source> {
    if what == WidgetType::Script.name() {
        return Ok(Source::Custom);
    }
    if let Some(t) = WidgetType::parse(what) {
        return Ok(Source::Builtin(t));
    }
    if let Some(e) = catalog::get(what) {
        return Ok(Source::Catalog(e));
    }
    let types: Vec<&str> = WidgetType::ALL.iter().map(|t| t.name()).collect();
    let scripts: Vec<&str> = catalog::entries().iter().map(|e| e.name).collect();
    Err(format!(
        "there is no widget type or catalog script called {what:?}; built-in types: {}; catalog: {} (see `wingbar widget catalog`)",
        types.join(", "),
        scripts.join(", ")
    ))
}

/// Add a widget: a built-in type, a catalog script (its script is written to
/// the widgets folder, never over an existing file) or a custom script.
pub fn add(f: &Files, req: &Add) -> Result<String> {
    let mut doc = load(f)?;
    let what = req.what.trim();
    let src = source(what)?;
    let ty = match src {
        Source::Builtin(t) => t,
        _ => WidgetType::Script,
    };
    let key = match (&req.key, &src) {
        (Some(k), _) => {
            edit::check_new_key(&doc.value, k).map_err(msg)?;
            k.clone()
        }
        (None, Source::Custom) => {
            return Err("a script widget needs a name: wingbar widget add script --as NAME --command CMD".into());
        }
        (None, Source::Builtin(t)) => free_key(&doc.value, t.name()),
        (None, Source::Catalog(e)) => free_key(&doc.value, e.name),
    };
    let script = match src {
        Source::Catalog(e) => Some((e, f.widgets_dir.join(e.file_name()))),
        _ => None,
    };
    let mut def = match (&src, &script) {
        (Source::Builtin(t), _) => defaults::definition(*t),
        (Source::Catalog(e), Some((_, path))) => e.definition_for(path),
        _ => defaults::definition(WidgetType::Script),
    };
    match &req.command {
        Some(_) if ty != WidgetType::Script => {
            return Err(format!("a {} widget runs no command; --command is for script widgets", ty.name()));
        }
        Some(c) if c.trim().is_empty() => return Err("the command is empty; give the shell command to run".into()),
        Some(c) => def["command"] = json!(c),
        None if matches!(src, Source::Custom) => {
            return Err("a script widget needs a command: --command CMD (run with /bin/bash -c)".into());
        }
        None => {}
    }
    if let Some(every) = &req.every {
        if !ty.options().contains(&"every") {
            return Err(format!("a {} widget has no --every (it updates by itself)", ty.name()));
        }
        def["every"] = edit::parse_value(every);
    }
    // a catalog script goes to its own usual wing, unless a neighbour decides
    let wing = req.wing.or(match (&src, &req.pos) {
        (Source::Catalog(e), Position::End | Position::At(_)) => Some(e.wing),
        _ => None,
    });
    check_at(&doc.value, None, wing.unwrap_or(ty.default_wing()), &req.pos)?;
    edit::add(&mut doc.value, &key, def, wing, &req.pos).map_err(msg)?;
    let installed = match &script {
        Some((e, _)) => Some(catalog::install(e.name, &f.widgets_dir)?),
        None => None,
    };
    doc.save()?;
    let named =
        if key == what || matches!(src, Source::Custom) { quoted(&key) } else { format!("{} ({what})", quoted(&key)) };
    let mut out = format!("Added {named} to {}.", where_now(&doc.value, &key));
    if let Some(i) = installed {
        out.push('\n');
        out.push_str(&capitalised(&i.message()));
        out.push('.');
    }
    Ok(out)
}

/// The menu's Custom Script…: `command` as a script widget called `key`, at
/// the end of `wing`.
pub fn add_custom(f: &Files, key: &str, command: &str, wing: Wing) -> Result<String> {
    add(
        f,
        &Add {
            key: Some(key.trim().to_string()),
            wing: Some(wing),
            command: Some(command.trim().to_string()),
            ..Add::new("script")
        },
    )
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// Delete a widget's definition and placement. Script files are never deleted.
pub fn remove(f: &Files, key: &str) -> Result<String> {
    let mut doc = load(f)?;
    let script = def_type(&doc.value, key).as_deref() == Some("script");
    edit::remove(&mut doc.value, key).map_err(msg)?;
    doc.save()?;
    Ok(if script {
        format!("Removed {}. Its script files are kept.", quoted(key))
    } else {
        format!("Removed {}.", quoted(key))
    })
}

/// Put a hidden widget on the bar (or, given a place, move a shown one there).
pub fn show(f: &Files, key: &str, wing: Option<Wing>, pos: &Position) -> Result<String> {
    let mut doc = load(f)?;
    let placed = edit::placement(&doc.value, key);
    let fallback = match placed {
        Some((w, _)) => w,
        None => def_type(&doc.value, key).and_then(|t| WidgetType::parse(&t)).map_or(Wing::Right, |t| t.default_wing()),
    };
    check_at(&doc.value, placed.map(|_| key), wing.unwrap_or(fallback), pos)?;
    edit::show(&mut doc.value, key, wing, pos).map_err(msg)?;
    doc.save()?;
    Ok(format!("Showing {} in {}.", quoted(key), where_now(&doc.value, key)))
}

/// Take a widget off the bar; its settings stay.
pub fn hide(f: &Files, key: &str) -> Result<String> {
    let mut doc = load(f)?;
    edit::hide(&mut doc.value, key).map_err(msg)?;
    doc.save()?;
    Ok(format!("Hid {}; its settings are kept (`wingbar widget show {key}` brings it back).", quoted(key)))
}

/// Move a shown widget within its wing or to the other one.
pub fn move_to(f: &Files, key: &str, wing: Option<Wing>, pos: &Position) -> Result<String> {
    let mut doc = load(f)?;
    if let Some((from, _)) = edit::placement(&doc.value, key) {
        check_at(&doc.value, Some(key), wing.unwrap_or(from), pos)?;
    }
    edit::move_to(&mut doc.value, key, wing, pos).map_err(msg)?;
    doc.save()?;
    Ok(format!("Moved {} to {}.", quoted(key), where_now(&doc.value, key)))
}

/// Where a step takes a widget placed at `index` of `wing` (which has `len`
/// widgets): `None` when it can't go that way.
pub fn step_target(wing: Wing, index: usize, len: usize, step: Step) -> Option<(Wing, Position)> {
    match step {
        Step::Left => (index > 0).then(|| (wing, Position::At(index - 1))),
        // At counts the wing without the widget being moved
        Step::Right => (index + 1 < len).then(|| (wing, Position::At(index + 1))),
        Step::OtherWing => Some((wing.other(), if wing == Wing::Left { Position::At(0) } else { Position::End })),
    }
}

/// The menu's Move Left / Move Right / Move to Other Wing.
pub fn move_step(f: &Files, key: &str, step: Step) -> Result<String> {
    let doc = load(f)?;
    let Some((w, i)) = edit::placement(&doc.value, key) else {
        return Err(format!("{} is not on the bar; show it first", quoted(key)));
    };
    let len = wing_keys(&doc.value, w).len();
    let Some((to, pos)) = step_target(w, i, len, step) else {
        let end = if step == Step::Left { "first" } else { "last" };
        return Err(format!("{} is already {end} in the {} wing", quoted(key), w.name()));
    };
    move_to(f, key, Some(to), &pos)
}

/// Set one option of a widget: `value` is JSON if it reads as JSON, else the
/// text itself; `null` removes the option (back to its default).
pub fn set(f: &Files, key: &str, option: &str, value: &str) -> Result<String> {
    let mut doc = load(f)?;
    let v = edit::parse_value(value);
    let removing = v.is_null();
    edit::set_option(&mut doc.value, key, option, v.clone()).map_err(msg)?;
    doc.save()?;
    Ok(if removing {
        format!("Removed {key}.{option} (back to its default).")
    } else {
        format!("Set {key}.{option} to {v}.")
    })
}

// --- listing -------------------------------------------------------------------

/// One widget, for `wingbar widget list`.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: String,
    /// Its `type` as written ("?" when it has no definition).
    pub ty: String,
    /// `None`: hidden.
    pub wing: Option<Wing>,
    /// 1-based, within its wing.
    pub position: Option<usize>,
    /// The last error of its script or reading, from the running bar.
    pub error: Option<String>,
}

/// Every widget: the left wing's in order, the right wing's, then the hidden
/// ones in the file's order. `errors` are the running bar's (by key).
pub fn rows(doc: &Value, errors: &HashMap<String, String>) -> Vec<Row> {
    let mut out = Vec::new();
    for w in [Wing::Left, Wing::Right] {
        for (i, k) in wing_keys(doc, w).into_iter().enumerate() {
            if out.iter().any(|r: &Row| r.key == k) {
                continue; // in both wings: the first wins, as when the bar reads it
            }
            let ty = def_type(doc, &k).unwrap_or_else(|| "?".into());
            let error = errors.get(&k).cloned();
            out.push(Row { key: k, ty, wing: Some(w), position: Some(i + 1), error });
        }
    }
    if let Some(defs) = doc.get("widgets").and_then(Value::as_object) {
        for k in defs.keys().filter(|k| !k.starts_with("//")) {
            if !out.iter().any(|r| &r.key == k) {
                let ty = def_type(doc, k).unwrap_or_else(|| "?".into());
                out.push(Row { key: k.clone(), ty, wing: None, position: None, error: errors.get(k).cloned() });
            }
        }
    }
    out
}

/// The settings file's widgets (see `rows`).
pub fn list(f: &Files, errors: &HashMap<String, String>) -> Result<Vec<Row>> {
    Ok(rows(&load(f)?.value, errors))
}

/// `rows` as a table.
pub fn rows_text(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "no widgets are defined (add one: wingbar widget add cpu)\n".into();
    }
    let kw = rows.iter().map(|r| r.key.chars().count()).max().unwrap_or(3).max(3);
    let tw = rows.iter().map(|r| r.ty.len()).max().unwrap_or(4).max(4);
    let mut out = format!("{:kw$}  {:tw$}  {:5}  {:3}  {:6}  ERROR\n", "KEY", "TYPE", "WING", "POS", "STATE");
    for r in rows {
        let wing = r.wing.map_or("-", Wing::name);
        let pos = r.position.map_or("-".to_string(), |p| p.to_string());
        let state = if r.wing.is_some() { "shown" } else { "hidden" };
        let line =
            format!("{:kw$}  {:tw$}  {wing:5}  {pos:3}  {state:6}  {}", r.key, r.ty, r.error.as_deref().unwrap_or(""));
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// `rows` as JSON, for `wingbar widget list --json`.
pub fn rows_json(rows: &[Row]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| {
                json!({
                    "key": r.key,
                    "type": r.ty,
                    "wing": r.wing.map(Wing::name),
                    "position": r.position,
                    "shown": r.wing.is_some(),
                    "error": r.error,
                })
            })
            .collect(),
    )
}

/// Something `wingbar widget catalog` offers.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// What to pass to `wingbar widget add`.
    pub name: String,
    pub description: String,
    pub kind: OfferKind,
    /// A widget with this key is defined already (adding makes `name-2`).
    pub added: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OfferKind {
    Builtin,
    Custom,
    Catalog,
}

/// The built-in types no widget uses yet, a custom script, and every catalog
/// script. A missing settings file offers everything.
pub fn offers(doc: &Value) -> Vec<Offer> {
    let used: Vec<String> = doc
        .get("widgets")
        .and_then(Value::as_object)
        .map(|w| w.values().filter_map(|d| d.get("type")?.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let defined = |k: &str| doc.get("widgets").and_then(|w| w.get(k)).is_some();
    let mut out: Vec<Offer> = WidgetType::ALL
        .iter()
        .filter(|t| **t != WidgetType::Script && !used.iter().any(|u| u == t.name()))
        .map(|t| Offer {
            name: t.name().into(),
            description: type_description(*t).into(),
            kind: OfferKind::Builtin,
            added: false,
        })
        .collect();
    out.push(Offer {
        name: "script".into(),
        description: "Your own command's output: add script --as NAME --command CMD".into(),
        kind: OfferKind::Custom,
        added: false,
    });
    out.extend(catalog::entries().iter().map(|e| Offer {
        name: e.name.into(),
        description: e.description.into(),
        kind: OfferKind::Catalog,
        added: defined(e.name),
    }));
    out
}

pub fn catalog(f: &Files) -> Result<Vec<Offer>> {
    let doc = if f.settings.exists() { load(f)?.value } else { Value::Null };
    Ok(offers(&doc))
}

/// `offers` as text.
pub fn offers_text(offers: &[Offer], widgets_dir: &Path) -> String {
    let w = offers.iter().map(|o| o.name.len()).max().unwrap_or(0);
    let mut out = String::from("Built-in widgets (wingbar widget add NAME):\n");
    for o in offers.iter().filter(|o| o.kind != OfferKind::Catalog) {
        out.push_str(&format!("  {:w$}  {}\n", o.name, o.description));
    }
    out.push_str(&format!("\nCatalog scripts (written to {} when added):\n", widgets_dir.display()));
    for o in offers.iter().filter(|o| o.kind == OfferKind::Catalog) {
        let added = if o.added { " (added)" } else { "" };
        out.push_str(&format!("  {:w$}  {}{added}\n", o.name, o.description));
    }
    out.push_str(
        "\nA type that is already used can be added again: it gets the next free name (time-2, ...), or --as NAME.\n",
    );
    out
}

// --- names -----------------------------------------------------------------------

/// A built-in type's name for people.
pub fn type_title(t: WidgetType) -> &'static str {
    match t {
        WidgetType::Spaces => "Workspaces",
        WidgetType::Title => "Window Title",
        WidgetType::Cpu => "CPU",
        WidgetType::Memory => "Memory",
        WidgetType::Battery => "Battery",
        WidgetType::Sound => "Sound",
        WidgetType::Date => "Date",
        WidgetType::Time => "Time",
        WidgetType::Spotify => "Spotify",
        WidgetType::Music => "Music",
        WidgetType::Script => "Script",
    }
}

/// One line about a built-in type.
pub fn type_description(t: WidgetType) -> &'static str {
    match t {
        WidgetType::Spaces => "The window manager's workspaces (AeroSpace or yabai)",
        WidgetType::Title => "The focused window's app and title",
        WidgetType::Cpu => "How busy the processor is",
        WidgetType::Memory => "How much memory is in use",
        WidgetType::Battery => "The battery's charge; click to keep the Mac awake",
        WidgetType::Sound => "The output volume; scroll to change it",
        WidgetType::Date => "Today's date",
        WidgetType::Time => "The time",
        WidgetType::Spotify => "What Spotify is playing",
        WidgetType::Music => "What Music is playing",
        WidgetType::Script => "Your own command's output",
    }
}

/// `aerospace-mode` → "Aerospace Mode".
pub fn title_case(key: &str) -> String {
    key.split(['-', '_']).filter(|w| !w.is_empty()).map(capitalised).collect::<Vec<_>>().join(" ")
}

/// A widget's name for people: its type's ("CPU") when the key is the type's
/// name, "CPU 2" for `cpu-2`, else the key title-cased ("Aerospace Mode").
pub fn display_name(key: &str, t: WidgetType) -> String {
    if t != WidgetType::Script {
        if key == t.name() {
            return type_title(t).into();
        }
        if let Some(n) = key.strip_prefix(t.name()).and_then(|r| r.strip_prefix('-'))
            && !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit())
        {
            return format!("{} {n}", type_title(t));
        }
    }
    title_case(key)
}

/// When a widget has something to show, to finish "It appears …", for a
/// widget of type `t` under `key` (a script added from the catalog is
/// recognised by its key: `adb`, `adb-2`).
pub fn appears_when(key: &str, t: WidgetType) -> &'static str {
    match t {
        WidgetType::Spaces => "while AeroSpace or yabai is running",
        WidgetType::Title => "while a window has the focus",
        WidgetType::Battery => "on a Mac with a battery",
        WidgetType::Spotify => "while Spotify is playing",
        WidgetType::Music => "while Music is playing",
        WidgetType::Script => {
            let from = |e: &&catalog::Entry| {
                key == e.name
                    || key
                        .strip_prefix(e.name)
                        .and_then(|r| r.strip_prefix('-'))
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            };
            match catalog::entries().iter().find(from) {
                Some(e) if !e.appears.is_empty() => e.appears,
                _ => "once its command prints something on its first line",
            }
        }
        _ => "after its first reading",
    }
}

/// What to tell someone who just added (or showed) the widget `key`, placed
/// on `wing`, when it shows nothing: a title and a sentence or two. `error`:
/// the last failure of its script or reading.
pub fn not_showing(key: &str, t: WidgetType, wing: Wing, error: Option<&str>) -> (String, String) {
    let name = display_name(key, t);
    match error {
        Some(e) => {
            (format!("“{name}” isn't working"), format!("It is on the {} wing, but it failed: {e}", wing.name()))
        }
        None => (
            format!("“{name}” has nothing to show yet"),
            format!(
                "It is on the {} wing, but empty for now, so the bar doesn't show it. It appears {}.",
                wing.name(),
                appears_when(key, t)
            ),
        ),
    }
}

/// The keys shown in `after` that `before` didn't show: what an edit put on
/// the bar.
pub fn newly_shown(before: &[String], after: impl IntoIterator<Item = String>) -> Vec<String> {
    after.into_iter().filter(|k| !before.contains(k)).collect()
}

#[cfg(test)]
mod tests;
