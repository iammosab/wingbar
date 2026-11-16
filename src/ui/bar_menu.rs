//! The wingbar menu: right-click on a wing (docs/design/widgets-v2.md, "Right-click
//! menu on the bar"). What it holds is a pure function of the settings, the
//! widget it was opened on and the wing (`model`, tested); choosing an item
//! runs one `manage` operation and then reloads the settings, through the same
//! path as `wingbar reload`, so the bar redraws as for any other change.
//!
//! ```text
//! CPU                                (header, when over a widget)
//! Move Left / Move Right / Move to Other Wing   (disabled when impossible)
//! Hide CPU
//! Remove CPU…                        (asks first; script files are kept)
//! ───────────
//! Widgets ▸        every defined widget, ✓ = shown; choosing one shows or hides it
//! Add Widget ▸     Built-in ▸ / Catalog ▸ / Custom Script…
//! ───────────
//! Open Settings File
//! Reload Settings
//! About wingbar 0.x.y                (disabled)
//! Quit wingbar
//! ```

use super::menu::{self, Item, Menu};
use crate::catalog;
use crate::config::edit::Position;
use crate::config::{Config, WidgetType, Wing};
use crate::manage::{self, Add, Files, Step};
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem,
    NSTextField, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::time::Duration;

/// What an item does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Step(String, Step),
    Hide(String),
    /// The key and its display name (for the confirmation).
    Remove(String, String),
    /// Show a hidden widget, or hide a shown one.
    Toggle {
        key: String,
        shown: bool,
    },
    AddBuiltin(WidgetType, Wing),
    AddCatalog(&'static str, Wing),
    AddCustom(Wing),
    OpenSettings,
    Reload,
    Quit,
}

/// The menu as data.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item(MItem),
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MItem {
    pub title: String,
    pub enabled: bool,
    pub checked: bool,
    pub tooltip: Option<String>,
    pub act: Option<Act>,
    pub submenu: Option<Vec<Entry>>,
}

impl MItem {
    fn new(title: impl Into<String>, act: Option<Act>) -> MItem {
        MItem { title: title.into(), enabled: true, checked: false, tooltip: None, act, submenu: None }
    }
    fn info(title: impl Into<String>) -> MItem {
        MItem { enabled: false, ..MItem::new(title, None) }
    }
    fn sub(title: impl Into<String>, entries: Vec<Entry>) -> MItem {
        MItem { submenu: Some(entries), ..MItem::new(title, None) }
    }
}

/// A widget's display name in these settings.
fn name_of(cfg: &Config, key: &str) -> String {
    match cfg.def(key) {
        Some(d) => manage::display_name(key, d.options.widget_type()),
        None => manage::title_case(key),
    }
}

/// The menu for a right-click over `key` (`None`: over no widget) in `wing`.
pub fn model(cfg: &Config, key: Option<&str>, wing: Wing) -> Vec<Entry> {
    let mut out = Vec::new();
    let item = |title: &str, act: Act| Entry::Item(MItem::new(title, Some(act)));
    if let Some(key) = key.filter(|k| cfg.def(k).is_some()) {
        let name = name_of(cfg, key);
        out.push(Entry::Item(MItem::info(&name)));
        let list = |w: Wing| if w == Wing::Left { &cfg.left } else { &cfg.right };
        let placed = cfg.wing_of(key).and_then(|w| Some((w, list(w).iter().position(|k| k == key)?)));
        for (title, step) in
            [("Move Left", Step::Left), ("Move Right", Step::Right), ("Move to Other Wing", Step::OtherWing)]
        {
            let possible = placed.is_some_and(|(w, i)| manage::step_target(w, i, list(w).len(), step).is_some());
            out.push(Entry::Item(MItem { enabled: possible, ..MItem::new(title, Some(Act::Step(key.into(), step))) }));
        }
        out.push(Entry::Item(MItem {
            enabled: placed.is_some(),
            ..MItem::new(format!("Hide {name}"), Some(Act::Hide(key.into())))
        }));
        out.push(item(&format!("Remove {name}…"), Act::Remove(key.into(), name)));
        out.push(Entry::Separator);
    }

    // every defined widget: the shown ones in wing order, then the hidden ones by name
    let mut widgets: Vec<Entry> = cfg
        .placed()
        .map(|(_, d)| {
            let act = Act::Toggle { key: d.key.clone(), shown: true };
            Entry::Item(MItem { checked: true, ..MItem::new(name_of(cfg, &d.key), Some(act)) })
        })
        .collect();
    let mut hidden: Vec<(String, String)> = cfg
        .widgets
        .iter()
        .filter(|d| cfg.wing_of(&d.key).is_none())
        .map(|d| (name_of(cfg, &d.key), d.key.clone()))
        .collect();
    hidden.sort_by_key(|(name, key)| (name.to_lowercase(), key.clone()));
    widgets.extend(hidden.into_iter().map(|(name, key)| item(&name, Act::Toggle { key, shown: false })));
    if widgets.is_empty() {
        widgets.push(Entry::Item(MItem::info("No widgets")));
    }
    out.push(Entry::Item(MItem::sub("Widgets", widgets)));

    let builtin = WidgetType::ALL
        .iter()
        .filter(|t| **t != WidgetType::Script)
        .map(|t| {
            Entry::Item(MItem {
                tooltip: Some(manage::type_description(*t).into()),
                ..MItem::new(manage::type_title(*t), Some(Act::AddBuiltin(*t, wing)))
            })
        })
        .collect();
    let scripts = catalog::entries()
        .iter()
        .map(|e| {
            Entry::Item(MItem {
                tooltip: Some(e.description.into()),
                ..MItem::new(manage::title_case(e.name), Some(Act::AddCatalog(e.name, wing)))
            })
        })
        .collect();
    out.push(Entry::Item(MItem::sub(
        "Add Widget",
        vec![
            Entry::Item(MItem::sub("Built-in", builtin)),
            Entry::Item(MItem::sub("Catalog", scripts)),
            item("Custom Script…", Act::AddCustom(wing)),
        ],
    )));
    out.push(Entry::Separator);
    out.push(item("Open Settings File", Act::OpenSettings));
    out.push(item("Reload Settings", Act::Reload));
    out.push(Entry::Item(MItem::info(format!("About wingbar {}", env!("CARGO_PKG_VERSION")))));
    out.push(item("Quit wingbar", Act::Quit));
    out
}

/// The model as a menu whose items run their `Act`.
pub fn to_menu(entries: Vec<Entry>) -> Menu {
    let mut m = Menu::new();
    for e in entries {
        let Entry::Item(it) = e else {
            m.separator();
            continue;
        };
        let mut item = Item::new(it.title).enabled(it.enabled).checked(it.checked);
        if let Some(t) = it.tooltip {
            item = item.tooltip(t);
        }
        if let Some(act) = it.act {
            item = item.action(move || perform(act));
        }
        if let Some(sub) = it.submenu {
            item = item.submenu(to_menu(sub));
        }
        m.push(item);
    }
    m
}

/// Whether a click opens the wingbar menu whatever is under it: ⌥-right-click
/// or ⌃-click.
pub fn forced(right: bool, mods: NSEventModifierFlags) -> bool {
    if right { mods.contains(NSEventModifierFlags::Option) } else { mods.contains(NSEventModifierFlags::Control) }
}

/// Show the menu for `key` in `wing` with its top-left corner at `at`.
pub fn pop_up(cfg: &Config, key: Option<&str>, wing: Wing, view: &NSView, at: NSPoint) {
    menu::pop_up(to_menu(model(cfg, key, wing)), view, at);
}

/// Run what an item stands for (on the main thread, from the menu).
fn perform(act: Act) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let f = Files::current();
    let before = shown_keys();
    let done = match act {
        Act::Step(k, s) => manage::move_step(&f, &k, s),
        Act::Hide(k) | Act::Toggle { key: k, shown: true } => manage::hide(&f, &k),
        Act::Toggle { key, shown: false } => manage::show(&f, &key, None, &Position::End),
        Act::Remove(k, name) => {
            if !confirm_remove(mtm, &name) {
                return;
            }
            manage::remove(&f, &k)
        }
        Act::AddBuiltin(t, w) => manage::add(&f, &Add { wing: Some(w), ..Add::new(t.name()) }),
        Act::AddCatalog(n, w) => manage::add(&f, &Add { wing: Some(w), ..Add::new(n) }),
        Act::AddCustom(w) => return custom_script(mtm, w, &before),
        Act::OpenSettings => {
            crate::sched::spawn_program("/usr/bin/open", &["-t", &f.settings.display().to_string()]);
            return;
        }
        Act::Reload => {
            if let Err(e) = crate::reload::reload() {
                alert(mtm, "The settings can't be used", &format!("{e}\n\nThe bar keeps the settings it had."));
            }
            return;
        }
        Act::Quit => std::process::exit(0),
    };
    match done {
        Ok(_) => apply(mtm, &before),
        Err(e) => alert(mtm, "That didn't work", &e),
    }
}

/// The keys on the bar now.
fn shown_keys() -> Vec<String> {
    crate::config::current().placed().map(|(_, d)| d.key.clone()).collect()
}

/// How long a widget just put on the bar gets to show something before the
/// menu says it shows nothing (its first run starts at once; most scripts
/// take well under a second).
const FIRST_RUN: Duration = Duration::from_secs(2);

/// The file changed: reload it now. (A reload reads the file, builds the
/// registry and hands the scheduler its tasks: milliseconds, so it runs here;
/// the bars are redrawn from the main queue afterwards, as for any reload.)
/// `before`: the keys shown before the change. A widget the change put on the
/// bar that still shows nothing after its first run is explained, because
/// an empty widget takes no room and adding it would look like nothing
/// happened.
fn apply(mtm: MainThreadMarker, before: &[String]) {
    if let Err(e) = crate::reload::reload() {
        alert(mtm, "The settings were saved, but can't be used", &e);
        return;
    }
    let new = manage::newly_shown(before, shown_keys());
    if new.is_empty() {
        return;
    }
    let when = DispatchTime::try_from(FIRST_RUN).unwrap_or(DispatchTime::NOW);
    let _ = DispatchQueue::main().after(when, move || explain_empty(&new));
}

/// Say which of `keys` (just put on the bar) show nothing, and why.
fn explain_empty(keys: &[String]) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let cfg = crate::config::current();
    let reg = crate::widgets::current();
    let notes: Vec<(String, String)> = {
        let st = crate::state::lock();
        keys.iter()
            .filter_map(|k| {
                let (wing, def, w) = (cfg.wing_of(k)?, cfg.def(k)?, reg.get(k)?);
                if crate::widgets::showing(&st, k, w.kind()) {
                    return None;
                }
                Some(manage::not_showing(k, def.options.widget_type(), wing, st.errors.get(k).map(String::as_str)))
            })
            .collect()
    };
    for (title, text) in notes {
        let a = new_alert(mtm, &title, &text);
        a.setAlertStyle(NSAlertStyle::Informational);
        a.runModal();
    }
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// An alert in front of every app: wingbar has no windows of its own, so it
/// must come forward for its alert to get the keyboard.
fn new_alert(mtm: MainThreadMarker, title: &str, text: &str) -> Retained<NSAlert> {
    #[allow(deprecated)] // `activate` needs macOS 14
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    let a = NSAlert::new(mtm);
    a.setMessageText(&ns(title));
    a.setInformativeText(&ns(text));
    a
}

fn alert(mtm: MainThreadMarker, title: &str, text: &str) {
    let a = new_alert(mtm, title, text);
    a.setAlertStyle(NSAlertStyle::Warning);
    a.runModal();
}

fn confirm_remove(mtm: MainThreadMarker, name: &str) -> bool {
    let a = new_alert(mtm, &format!("Remove “{name}”?"), "Its settings are deleted; script files are kept.");
    a.addButtonWithTitle(&ns("Remove"));
    a.addButtonWithTitle(&ns("Cancel"));
    a.runModal() == NSAlertFirstButtonReturn
}

/// Custom Script…: ask for a name and a command until they make a widget, or
/// the person cancels.
fn custom_script(mtm: MainThreadMarker, wing: Wing, before: &[String]) {
    ensure_edit_menu(mtm);
    let (mut name, mut command) = (String::new(), String::new());
    loop {
        let Some((n, c)) = ask_custom(mtm, &name, &command) else { return };
        match manage::add_custom(&Files::current(), &n, &c, wing) {
            Ok(_) => return apply(mtm, before),
            Err(e) => alert(mtm, "The script widget can't be added", &e.replace(" (--as NAME)", "")),
        }
        (name, command) = (n, c);
    }
}

/// The dialog: a name and a command. `None` when cancelled.
fn ask_custom(mtm: MainThreadMarker, name: &str, command: &str) -> Option<(String, String)> {
    let a = new_alert(
        mtm,
        "Add a Script Widget",
        "The command runs with /bin/bash every 10 seconds; its first line is what the widget shows \
         (\"level|label\", or just a label).",
    );
    a.addButtonWithTitle(&ns("Add"));
    a.addButtonWithTitle(&ns("Cancel"));
    let w = 320.0;
    let view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, 104.0)));
    let label = |text: &str, y: f64| {
        let l = NSTextField::labelWithString(&ns(text), mtm);
        l.setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(w, 17.0)));
        view.addSubview(&l);
    };
    let field = |value: &str, placeholder: &str, y: f64| {
        let f = NSTextField::textFieldWithString(&ns(value), mtm);
        f.setPlaceholderString(Some(&ns(placeholder)));
        f.setFrame(NSRect::new(NSPoint::new(0.0, y), NSSize::new(w, 22.0)));
        view.addSubview(&f);
        f
    };
    label("Name (a-z, 0-9, _ and -)", 86.0);
    let name_field = field(name, "weather", 62.0);
    label("Command", 36.0);
    let command_field = field(command, "bash ~/scripts/weather.sh", 12.0);
    // SAFETY: both fields are subviews of the same view, alive while the alert runs.
    unsafe {
        name_field.setNextKeyView(Some(&command_field));
        command_field.setNextKeyView(Some(&name_field));
    }
    a.setAccessoryView(Some(&view));
    a.layout();
    a.window().setInitialFirstResponder(Some(&name_field));
    if a.runModal() != NSAlertFirstButtonReturn {
        return None;
    }
    Some((name_field.stringValue().to_string(), command_field.stringValue().to_string()))
}

/// wingbar has no menu bar of its own, so the text fields' ⌘X / ⌘C / ⌘V /
/// ⌘A / ⌘Z would do nothing: give the app a main menu (never shown, as wingbar
/// is an accessory app) with those items, sent to whatever has the keyboard.
fn ensure_edit_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    if app.mainMenu().is_some() {
        return;
    }
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns("Edit"));
    for (title, action, key) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        // SAFETY: NSMenuItem's designated initialiser on a fresh allocation; the
        // actions are standard responder-chain messages (no target: the first
        // responder gets them), which AppKit checks before sending.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &ns(title), Some(action), &ns(key))
        };
        edit.addItem(&item);
    }
    let edit_item = NSMenuItem::new(mtm);
    edit_item.setSubmenu(Some(&edit));
    let main = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns(""));
    main.addItem(&edit_item);
    app.setMainMenu(Some(&main));
}

/// `WINGBAR_DEBUG_MENUS=manage` with `--snapshot`: print the wingbar menu as
/// AppKit got it, over every shown widget and over no widget (submenus in
/// full the first time), without showing it.
pub fn debug(mtm: MainThreadMarker) {
    let cfg = crate::config::current();
    let mut targets: Vec<(Option<String>, Wing)> = cfg.placed().map(|(w, d)| (Some(d.key.clone()), w)).collect();
    targets.push((None, Wing::Right));
    for (i, (key, wing)) in targets.into_iter().enumerate() {
        let built = menu::build(to_menu(model(&cfg, key.as_deref(), wing)), mtm);
        println!("wingbar menu over {} ({} wing):", key.as_deref().unwrap_or("no widget"), wing.name());
        print_menu(&built.menu, 1, i == 0);
    }
}

fn print_menu(m: &NSMenu, depth: usize, submenus: bool) {
    let pad = "  ".repeat(depth);
    for i in 0..m.numberOfItems() {
        let Some(it) = m.itemAtIndex(i) else { continue };
        if it.isSeparatorItem() {
            println!("{pad}---");
            continue;
        }
        let mut flags = Vec::new();
        if !it.isEnabled() {
            flags.push("disabled".to_string());
        }
        if it.state() == objc2_app_kit::NSControlStateValueOn {
            flags.push("checked".into());
        }
        let sub = it.submenu();
        // (AppKit gives an item with a submenu an action of its own)
        if it.action().is_some() && sub.is_none() {
            flags.push("action".into());
        }
        if let Some(t) = it.toolTip() {
            flags.push(format!("tooltip={:?}", t.to_string()));
        }
        let arrow = if sub.is_some() { " ▸" } else { "" };
        println!("{pad}{:?}{arrow} [{}]", it.title().to_string(), flags.join(", "));
        if let Some(sub) = sub
            && submenus
        {
            print_menu(&sub, depth + 1, submenus);
        }
    }
}

#[cfg(test)]
mod tests;
