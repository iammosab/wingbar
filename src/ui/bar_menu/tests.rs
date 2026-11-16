use super::*;
use crate::config::{defaults, parse};
use serde_json::{Value, json};
use std::path::Path;

/// A fresh install's settings (spaces, title | cpu, memory, battery, sound,
/// date, time), changed by `f`.
fn cfg(f: impl FnOnce(&mut Value)) -> Config {
    let mut doc = defaults::fresh(true);
    f(&mut doc);
    let (cfg, warnings) = parse::parse(&doc, &|_: &Path| false);
    assert!(warnings.is_empty(), "{warnings:?}");
    cfg
}

/// (title, enabled, checked, has an act, has a submenu), "---" for separators.
fn shape(entries: &[Entry]) -> Vec<(String, bool, bool, bool, bool)> {
    entries
        .iter()
        .map(|e| match e {
            Entry::Item(it) => (it.title.clone(), it.enabled, it.checked, it.act.is_some(), it.submenu.is_some()),
            Entry::Separator => ("---".into(), false, false, false, false),
        })
        .collect()
}

fn titles(entries: &[Entry]) -> Vec<String> {
    shape(entries).into_iter().map(|s| s.0).collect()
}

fn find<'a>(entries: &'a [Entry], title: &str) -> &'a MItem {
    entries
        .iter()
        .find_map(|e| match e {
            Entry::Item(it) if it.title == title => Some(it),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {title:?} in {:?}", titles(entries)))
}

fn sub<'a>(entries: &'a [Entry], title: &str) -> &'a [Entry] {
    find(entries, title).submenu.as_deref().unwrap()
}

fn s(t: &str, enabled: bool, checked: bool, act: bool, submenu: bool) -> (String, bool, bool, bool, bool) {
    (t.into(), enabled, checked, act, submenu)
}

fn about() -> String {
    format!("About wingbar {}", env!("CARGO_PKG_VERSION"))
}

#[test]
fn over_a_widget() {
    let c = cfg(|_| {});
    let m = model(&c, Some("cpu"), Wing::Right);
    assert_eq!(
        shape(&m),
        [
            s("CPU", false, false, false, false),
            s("Move Left", false, false, true, false),
            s("Move Right", true, false, true, false),
            s("Move to Other Wing", true, false, true, false),
            s("Hide CPU", true, false, true, false),
            s("Remove CPU…", true, false, true, false),
            s("---", false, false, false, false),
            s("Widgets", true, false, false, true),
            s("Add Widget", true, false, false, true),
            s("---", false, false, false, false),
            s("Open Settings File", true, false, true, false),
            s("Reload Settings", true, false, true, false),
            s(&about(), false, false, false, false),
            s("Quit wingbar", true, false, true, false),
        ]
    );
    assert_eq!(find(&m, "Move Left").act, Some(Act::Step("cpu".into(), Step::Left)));
    assert_eq!(find(&m, "Move to Other Wing").act, Some(Act::Step("cpu".into(), Step::OtherWing)));
    assert_eq!(find(&m, "Hide CPU").act, Some(Act::Hide("cpu".into())));
    assert_eq!(find(&m, "Remove CPU…").act, Some(Act::Remove("cpu".into(), "CPU".into())));
    assert_eq!(find(&m, "Quit wingbar").act, Some(Act::Quit));
    assert_eq!(find(&m, "Reload Settings").act, Some(Act::Reload));
    assert_eq!(find(&m, "Open Settings File").act, Some(Act::OpenSettings));
}

#[test]
fn moves_are_disabled_at_the_ends() {
    let c = cfg(|_| {});
    let enabled = |key: &str, wing| -> Vec<bool> {
        let m = model(&c, Some(key), wing);
        ["Move Left", "Move Right", "Move to Other Wing"].iter().map(|t| find(&m, t).enabled).collect()
    };
    assert_eq!(enabled("time", Wing::Right), [true, false, true], "last in its wing");
    assert_eq!(enabled("memory", Wing::Right), [true, true, true]);
    assert_eq!(enabled("spaces", Wing::Left), [false, true, true]);
    assert_eq!(enabled("title", Wing::Left), [true, false, true]);
    let alone = cfg(|d| d["left"] = json!(["title"]));
    let m = model(&alone, Some("title"), Wing::Left);
    let got: Vec<bool> =
        ["Move Left", "Move Right", "Move to Other Wing"].iter().map(|t| find(&m, t).enabled).collect();
    assert_eq!(got, [false, false, true], "alone in its wing");
    assert_eq!(find(&m, "Hide Window Title").act, Some(Act::Hide("title".into())));
}

#[test]
fn over_no_widget() {
    let c = cfg(|_| {});
    let m = model(&c, None, Wing::Left);
    assert_eq!(
        titles(&m),
        ["Widgets", "Add Widget", "---", "Open Settings File", "Reload Settings", &about(), "Quit wingbar"]
    );
    // a key the settings don't define counts as no widget
    assert_eq!(titles(&model(&c, Some("ghost"), Wing::Left)), titles(&m));
}

#[test]
fn widgets_shown_in_wing_order_then_hidden_by_name() {
    let c = cfg(|d| {
        d["right"] = json!(["memory", "cpu", "time"]);
        d["widgets"]["zeta"] = json!({ "type": "script", "command": "echo z" });
        d["widgets"]["aerospace-mode"] = json!({ "type": "script", "command": "echo a" });
        d["widgets"]["cpu-2"] = json!({ "type": "cpu" });
    });
    let m = model(&c, Some("cpu"), Wing::Right);
    let w = sub(&m, "Widgets");
    assert_eq!(
        shape(w),
        [
            s("Workspaces", true, true, true, false),
            s("Window Title", true, true, true, false),
            s("Memory", true, true, true, false),
            s("CPU", true, true, true, false),
            s("Time", true, true, true, false),
            s("Aerospace Mode", true, false, true, false),
            s("Battery", true, false, true, false),
            s("CPU 2", true, false, true, false),
            s("Date", true, false, true, false),
            s("Sound", true, false, true, false),
            s("Zeta", true, false, true, false),
        ]
    );
    assert_eq!(find(w, "CPU").act, Some(Act::Toggle { key: "cpu".into(), shown: true }));
    assert_eq!(find(w, "CPU 2").act, Some(Act::Toggle { key: "cpu-2".into(), shown: false }));
    // nothing defined at all
    let empty = cfg(|d| {
        d["left"] = json!([]);
        d["right"] = json!([]);
        d["widgets"] = json!({});
    });
    assert_eq!(shape(sub(&model(&empty, None, Wing::Right), "Widgets")), [s("No widgets", false, false, false, false)]);
}

#[test]
fn add_widget_offers_every_type_the_catalog_and_a_custom_script() {
    let c = cfg(|_| {});
    let m = model(&c, Some("spaces"), Wing::Left);
    let add = sub(&m, "Add Widget");
    assert_eq!(titles(add), ["Built-in", "Catalog", "Custom Script…"]);
    assert_eq!(find(add, "Custom Script…").act, Some(Act::AddCustom(Wing::Left)));
    let builtin = sub(add, "Built-in");
    assert_eq!(
        titles(builtin),
        ["Workspaces", "Window Title", "CPU", "Memory", "Battery", "Sound", "Date", "Time", "Spotify", "Music"],
        "a type already defined is still offered (it adds name-2)"
    );
    let cpu = find(builtin, "CPU");
    assert_eq!(cpu.act, Some(Act::AddBuiltin(WidgetType::Cpu, Wing::Left)), "to the wing the menu opened on");
    assert_eq!(cpu.tooltip.as_deref(), Some("How busy the processor is"));
    let cat = sub(add, "Catalog");
    assert_eq!(titles(cat), ["Storage", "Network", "Mic", "Adb", "Aerospace Mode", "Git Branch"]);
    let storage = find(cat, "Storage");
    assert_eq!(storage.act, Some(Act::AddCatalog("storage", Wing::Left)));
    assert_eq!(storage.tooltip.as_deref(), Some(catalog::get("storage").unwrap().description));
    // opened on the right wing, adds go there
    let m = model(&c, None, Wing::Right);
    let add = sub(&m, "Add Widget");
    assert_eq!(find(sub(add, "Built-in"), "Time").act, Some(Act::AddBuiltin(WidgetType::Time, Wing::Right)));
}

#[test]
fn the_model_becomes_a_menu_with_actions() {
    use super::menu::Entry as E;
    let c = cfg(|_| {});
    let m = to_menu(model(&c, Some("cpu"), Wing::Right));
    let items: Vec<&Item> = m
        .entries
        .iter()
        .filter_map(|e| match e {
            E::Item(it) => Some(it),
            E::Separator => None,
        })
        .collect();
    assert_eq!(items[0].title, "CPU");
    assert!(!items[0].enabled && items[0].action.is_none());
    assert!(items[2].action.is_some() && items[2].title == "Move Right");
    assert!(!items[1].enabled && items[1].action.is_some(), "disabled, still described");
    let widgets = items.iter().find(|i| i.title == "Widgets").unwrap().submenu.as_ref().unwrap();
    let E::Item(first) = &widgets.entries[0] else { panic!() };
    assert_eq!((first.title.as_str(), first.check), ("Workspaces", menu::Check::On));
    let add = items.iter().find(|i| i.title == "Add Widget").unwrap().submenu.as_ref().unwrap();
    let E::Item(builtin) = &add.entries[0] else { panic!() };
    let E::Item(spaces) = &builtin.submenu.as_ref().unwrap().entries[0] else { panic!() };
    assert_eq!(spaces.tooltip.as_deref(), Some("The window manager's workspaces (AeroSpace or yabai)"));
}

#[test]
fn forced_by_modifiers() {
    use objc2_app_kit::NSEventModifierFlags as M;
    assert!(forced(true, M::Option));
    assert!(forced(true, M::Option | M::Shift));
    assert!(!forced(true, M::empty()));
    assert!(!forced(true, M::Control), "⌃-right-click is a right-click");
    assert!(forced(false, M::Control));
    assert!(!forced(false, M::Option));
    assert!(!forced(false, M::empty()));
}
