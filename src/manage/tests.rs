use super::*;
use crate::config::doc::to_text;
use std::os::unix::fs::PermissionsExt;

/// A temp folder with a fresh install's settings (spaces, title | cpu, memory,
/// battery, sound, date, time). Removed when dropped.
struct Temp {
    dir: PathBuf,
    f: Files,
}

impl Temp {
    fn new(name: &str) -> Temp {
        let dir = std::env::temp_dir().join(format!("wingbar-manage-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = Files { settings: dir.join("config.json"), widgets_dir: dir.join("widgets") };
        std::fs::write(&f.settings, to_text(&defaults::fresh(true))).unwrap();
        Temp { dir, f }
    }
    fn doc(&self) -> Value {
        Document::load(&self.f.settings).unwrap().value
    }
    fn text(&self) -> String {
        std::fs::read_to_string(&self.f.settings).unwrap()
    }
    fn wing(&self, w: Wing) -> Vec<String> {
        wing_keys(&self.doc(), w)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn add_req(what: &str) -> Add {
    Add::new(what)
}

#[test]
fn add_a_built_in_type_then_again_as_name_2() {
    let t = Temp::new("builtin");
    assert_eq!(add(&t.f, &add_req("spotify")).unwrap(), "Added “spotify” to the right wing (position 7).");
    assert_eq!(t.doc()["widgets"]["spotify"], json!({ "type": "spotify" }));
    assert_eq!(add(&t.f, &add_req("cpu")).unwrap(), "Added “cpu-2” (cpu) to the right wing (position 8).");
    assert_eq!(add(&t.f, &add_req("cpu")).unwrap(), "Added “cpu-3” (cpu) to the right wing (position 9).");
    assert_eq!(t.doc()["widgets"]["cpu-2"], defaults::definition(WidgetType::Cpu));
    // its usual wing
    assert!(add(&t.f, &add_req("title")).unwrap().contains("“title-2” (title) to the left wing (position 3)"));
    // the file reads without warnings
    let (_, warnings) = config::parse::parse(&t.doc(), &|_: &Path| false);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn add_a_catalog_script_installs_it_once() {
    let t = Temp::new("catalog");
    let m = add(&t.f, &add_req("storage")).unwrap();
    let script = t.f.widgets_dir.join("storage.sh");
    assert_eq!(m, format!("Added “storage” to the right wing (position 7).\nWrote {}.", script.display()));
    assert_eq!(std::fs::read_to_string(&script).unwrap(), catalog::get("storage").unwrap().script);
    assert_eq!(std::fs::metadata(&script).unwrap().permissions().mode() & 0o111, 0o111, "executable");
    let def = &t.doc()["widgets"]["storage"];
    assert_eq!(def["type"], "script");
    assert!(def["command"].as_str().unwrap().ends_with("storage.sh"), "{def}");
    // a second one: storage-2, and the script someone changed is kept
    std::fs::write(&script, "echo mine").unwrap();
    let m = add(&t.f, &add_req("storage")).unwrap();
    assert!(m.starts_with("Added “storage-2” (storage) to the right wing (position 8).\nKept the existing"), "{m}");
    assert_eq!(std::fs::read_to_string(&script).unwrap(), "echo mine");
    // a catalog script goes to its own wing; a --command replaces the catalog's
    let mut r = add_req("aerospace-mode");
    r.command = Some("bash ~/x.sh".into());
    assert!(add(&t.f, &r).unwrap().contains("to the left wing (position 3)"));
    assert_eq!(t.doc()["widgets"]["aerospace-mode"]["command"], "bash ~/x.sh");
    assert!(t.f.widgets_dir.join("aerospace-mode.sh").exists(), "installed anyway");
    // ... unless a neighbour decides
    let mut r = add_req("network");
    r.pos = Position::After("spaces".into());
    assert!(add(&t.f, &r).unwrap().contains("to the left wing (position 2)"));
}

#[test]
fn add_a_custom_script() {
    let t = Temp::new("custom");
    let mut r = add_req("script");
    r.command = Some("echo hi".into());
    assert!(add(&t.f, &r).unwrap_err().contains("needs a name"));
    r.key = Some("hello".into());
    r.command = None;
    assert!(add(&t.f, &r).unwrap_err().contains("needs a command"));
    r.command = Some("  ".into());
    assert!(add(&t.f, &r).unwrap_err().contains("command is empty"));
    r.command = Some("echo hi".into());
    assert_eq!(add(&t.f, &r).unwrap(), "Added “hello” to the right wing (position 7).");
    assert_eq!(t.doc()["widgets"]["hello"], json!({ "type": "script", "command": "echo hi", "every": "10s" }));
    // the menu's dialog
    assert!(
        add_custom(&t.f, " weather ", " curl wttr.in ", Wing::Left).unwrap().contains("the left wing (position 3)")
    );
    assert_eq!(t.doc()["widgets"]["weather"]["command"], "curl wttr.in");
    let bad = add_custom(&t.f, "My Weather", "x", Wing::Left).unwrap_err();
    assert!(bad.contains("a-z, 0-9") && bad.contains("\"my-weather\""), "{bad}");
    assert!(add_custom(&t.f, "weather", "x", Wing::Left).unwrap_err().contains("already exists"));
    assert!(add_custom(&t.f, "", "x", Wing::Left).unwrap_err().contains("can't be a widget name"));
}

#[test]
fn add_with_a_key_a_wing_and_a_position() {
    let t = Temp::new("place");
    let r = Add { key: Some("clock2".into()), wing: Some(Wing::Left), pos: Position::At(0), ..add_req("time") };
    assert_eq!(add(&t.f, &r).unwrap(), "Added “clock2” (time) to the left wing (position 1).");
    assert_eq!(t.wing(Wing::Left), ["clock2", "spaces", "title"]);
    let taken = Add { key: Some("clock2".into()), ..add_req("time") };
    assert!(add(&t.f, &taken).unwrap_err().contains("already exists"));
    // --at counts from 1 in messages, and the end is one past the last
    let past = Add { pos: Position::At(4), wing: Some(Wing::Left), ..add_req("sound") };
    assert_eq!(
        add(&t.f, &past).unwrap_err(),
        "position 5 is past the end of the left wing; use 1 to 4 (or leave --at out for the end)"
    );
    let end = Add { pos: Position::At(3), wing: Some(Wing::Left), ..add_req("sound") };
    assert!(add(&t.f, &end).unwrap().contains("left wing (position 4)"));
    let before = Add { pos: Position::Before("date".into()), ..add_req("music") };
    assert!(add(&t.f, &before).unwrap().contains("right wing (position 5)"));
    let wrong = Add { wing: Some(Wing::Left), pos: Position::After("cpu".into()), ..add_req("music") };
    assert!(add(&t.f, &wrong).unwrap_err().contains("in the right wing, not the left"));
}

#[test]
fn add_with_every_and_command() {
    let t = Temp::new("every");
    let r = Add { every: Some("5s".into()), ..add_req("cpu") };
    add(&t.f, &r).unwrap();
    assert_eq!(t.doc()["widgets"]["cpu-2"]["every"], "5s");
    let r = Add { every: Some("1500".into()), ..add_req("storage") };
    add(&t.f, &r).unwrap();
    assert_eq!(t.doc()["widgets"]["storage"]["every"], 1500, "a number is milliseconds");
    let before = t.text();
    let e = |r: Add| add(&t.f, &r).unwrap_err();
    assert!(e(Add { every: Some("5s".into()), ..add_req("battery") }).contains("battery widget has no --every"));
    assert!(e(Add { every: Some("soon".into()), ..add_req("cpu") }).contains("every"));
    assert!(e(Add { command: Some("x".into()), ..add_req("cpu") }).contains("--command is for script widgets"));
    let unknown = e(add_req("weather"));
    assert!(
        unknown.contains("\"weather\"") && unknown.contains("spaces") && unknown.contains("git-branch"),
        "{unknown}"
    );
    assert_eq!(t.text(), before, "nothing written after an error");
}

#[test]
fn remove_keeps_script_files() {
    let t = Temp::new("remove");
    assert_eq!(remove(&t.f, "cpu").unwrap(), "Removed “cpu”.");
    assert!(t.doc()["widgets"].get("cpu").is_none());
    assert!(!t.wing(Wing::Right).contains(&"cpu".to_string()));
    add(&t.f, &add_req("storage")).unwrap();
    assert_eq!(remove(&t.f, "storage").unwrap(), "Removed “storage”. Its script files are kept.");
    assert!(t.f.widgets_dir.join("storage.sh").exists());
    assert!(remove(&t.f, "storage").unwrap_err().contains("no widget named \"storage\""));
}

#[test]
fn hide_and_show() {
    let t = Temp::new("hide");
    assert_eq!(
        hide(&t.f, "cpu").unwrap(),
        "Hid “cpu”; its settings are kept (`wingbar widget show cpu` brings it back)."
    );
    assert!(hide(&t.f, "cpu").unwrap_err().contains("already hidden"));
    assert_eq!(show(&t.f, "cpu", None, &Position::End).unwrap(), "Showing “cpu” in the right wing (position 6).");
    assert!(show(&t.f, "cpu", None, &Position::End).unwrap_err().contains("already shown"));
    // given a place, a shown widget moves there
    assert_eq!(
        show(&t.f, "cpu", Some(Wing::Left), &Position::At(0)).unwrap(),
        "Showing “cpu” in the left wing (position 1)."
    );
    hide(&t.f, "cpu").unwrap();
    assert!(show(&t.f, "cpu", None, &Position::At(9)).unwrap_err().contains("use 1 to 6"));
    assert!(show(&t.f, "nope", None, &Position::End).unwrap_err().contains("no widget named"));
}

#[test]
fn move_within_and_across_wings() {
    let t = Temp::new("move");
    // right: cpu memory battery sound date time
    assert_eq!(
        move_to(&t.f, "cpu", None, &Position::After("memory".into())).unwrap(),
        "Moved “cpu” to the right wing (position 2)."
    );
    assert_eq!(t.wing(Wing::Right), ["memory", "cpu", "battery", "sound", "date", "time"]);
    // --at counts the wing without the widget being moved
    assert!(move_to(&t.f, "cpu", None, &Position::At(5)).unwrap().contains("position 6"));
    assert!(move_to(&t.f, "cpu", None, &Position::At(6)).unwrap_err().contains("use 1 to 6"));
    assert!(move_to(&t.f, "cpu", Some(Wing::Left), &Position::At(2)).unwrap().contains("left wing (position 3)"));
    assert!(move_to(&t.f, "cpu", None, &Position::Before("cpu".into())).unwrap_err().contains("next to itself"));
    hide(&t.f, "cpu").unwrap();
    assert!(move_to(&t.f, "cpu", None, &Position::End).unwrap_err().contains("hidden; use show"));
}

#[test]
fn steps_for_the_menu() {
    use Step::*;
    assert_eq!(step_target(Wing::Right, 0, 3, Left), None);
    assert_eq!(step_target(Wing::Right, 1, 3, Left), Some((Wing::Right, Position::At(0))));
    assert_eq!(step_target(Wing::Right, 1, 3, Right), Some((Wing::Right, Position::At(2))));
    assert_eq!(step_target(Wing::Right, 2, 3, Right), None);
    assert_eq!(step_target(Wing::Left, 0, 1, Left), None);
    assert_eq!(step_target(Wing::Left, 0, 1, Right), None);
    assert_eq!(step_target(Wing::Left, 0, 1, OtherWing), Some((Wing::Right, Position::At(0))));
    assert_eq!(step_target(Wing::Right, 5, 6, OtherWing), Some((Wing::Left, Position::End)));

    let t = Temp::new("step");
    move_step(&t.f, "memory", Left).unwrap();
    assert_eq!(t.wing(Wing::Right), ["memory", "cpu", "battery", "sound", "date", "time"]);
    move_step(&t.f, "memory", Right).unwrap();
    assert_eq!(t.wing(Wing::Right), ["cpu", "memory", "battery", "sound", "date", "time"]);
    assert_eq!(move_step(&t.f, "cpu", Left).unwrap_err(), "“cpu” is already first in the right wing");
    assert_eq!(move_step(&t.f, "time", Right).unwrap_err(), "“time” is already last in the right wing");
    // across the middle: to the nearest end of the other wing
    assert_eq!(move_step(&t.f, "cpu", OtherWing).unwrap(), "Moved “cpu” to the left wing (position 3).");
    assert_eq!(t.wing(Wing::Left), ["spaces", "title", "cpu"]);
    move_step(&t.f, "title", OtherWing).unwrap();
    assert_eq!(t.wing(Wing::Right), ["title", "memory", "battery", "sound", "date", "time"]);
    hide(&t.f, "memory").unwrap();
    assert!(move_step(&t.f, "memory", Left).unwrap_err().contains("not on the bar"));
}

#[test]
fn set_json_and_plain_strings() {
    let t = Temp::new("set");
    assert_eq!(set(&t.f, "cpu", "every", "5s").unwrap(), "Set cpu.every to \"5s\".");
    assert_eq!(set(&t.f, "time", "hour12", "true").unwrap(), "Set time.hour12 to true.");
    assert_eq!(t.doc()["widgets"]["time"]["hour12"], true);
    set(&t.f, "cpu", "click", "open -a 'Activity Monitor'").unwrap();
    assert_eq!(t.doc()["widgets"]["cpu"]["click"], "open -a 'Activity Monitor'");
    set(&t.f, "cpu", "every", "3000").unwrap();
    assert_eq!(t.doc()["widgets"]["cpu"]["every"], 3000);
    assert_eq!(set(&t.f, "cpu", "every", "null").unwrap(), "Removed cpu.every (back to its default).");
    assert!(t.doc()["widgets"]["cpu"].get("every").is_none());
    add_custom(&t.f, "net", "echo ok", Wing::Right).unwrap();
    set(&t.f, "net", "levels", r#"{"ok": "green", "eth": {"color": "blue", "icon": "cable.connector"}}"#).unwrap();
    assert_eq!(t.doc()["widgets"]["net"]["levels"]["eth"]["icon"], "cable.connector");
    let before = t.text();
    assert!(set(&t.f, "cpu", "colour", "red").unwrap_err().contains("no option \"colour\""));
    assert!(set(&t.f, "time", "hour12", "maybe").unwrap_err().contains("hour12"));
    assert!(set(&t.f, "nope", "every", "5s").unwrap_err().contains("no widget named"));
    assert!(set(&t.f, "cpu", "type", "memory").unwrap_err().contains("can't be changed"));
    assert_eq!(t.text(), before);
}

#[test]
fn files_that_cant_be_edited() {
    let t = Temp::new("files");
    let missing = Files { settings: t.dir.join("nope.json"), widgets_dir: t.dir.join("w") };
    let e = hide(&missing, "cpu").unwrap_err();
    assert!(e.contains("no settings file") && e.contains("import-simplebar"), "{e}");
    std::fs::write(&t.f.settings, "{ \"left\": [] }").unwrap();
    assert!(hide(&t.f, "cpu").unwrap_err().contains("wingbar 0.2 file"));
    std::fs::write(&t.f.settings, "{ oops").unwrap();
    assert!(hide(&t.f, "cpu").unwrap_err().contains("not valid JSON"));
    std::fs::write(&t.f.settings, "[1]").unwrap();
    assert!(hide(&t.f, "cpu").unwrap_err().contains("not a JSON object"));
    assert_eq!(t.text(), "[1]");
    // the catalog of a missing file offers everything
    let o = catalog(&missing).unwrap();
    assert!(o.iter().any(|o| o.name == "cpu"));
}

#[test]
fn list_rows_in_wing_order_then_hidden() {
    let t = Temp::new("list");
    hide(&t.f, "memory").unwrap();
    add_custom(&t.f, "net", "echo ok", Wing::Left).unwrap();
    let mut errors = HashMap::new();
    errors.insert("net".to_string(), "exited with status 1".to_string());
    let rows = list(&t.f, &errors).unwrap();
    let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["spaces", "title", "net", "cpu", "battery", "sound", "date", "time", "memory"]);
    assert_eq!(
        rows[2],
        Row {
            key: "net".into(),
            ty: "script".into(),
            wing: Some(Wing::Left),
            position: Some(3),
            error: Some("exited with status 1".into())
        }
    );
    assert_eq!(rows[8], Row { key: "memory".into(), ty: "memory".into(), wing: None, position: None, error: None });
    let text = rows_text(&rows);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "KEY      TYPE     WING   POS  STATE   ERROR");
    assert_eq!(lines[3], "net      script   left   3    shown   exited with status 1");
    assert_eq!(lines[4], "cpu      cpu      right  1    shown");
    assert_eq!(lines[9], "memory   memory   -      -    hidden");
    let j = rows_json(&rows);
    assert_eq!(
        j[8],
        json!({ "key": "memory", "type": "memory", "wing": null, "position": null, "shown": false, "error": null })
    );
    assert_eq!(j[2]["error"], "exited with status 1");
    // a placed key without a definition, and a key in both wings
    let doc = json!({ "version": 2, "left": ["ghost", "a"], "right": ["a"], "widgets": { "a": { "type": "cpu" } } });
    let rows = super::rows(&doc, &HashMap::new());
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].ty.as_str(), rows[1].wing), ("?", Some(Wing::Left)));
    assert!(rows_text(&[]).contains("no widgets"));
}

#[test]
fn catalog_offers_unused_types_and_every_script() {
    let t = Temp::new("offers");
    add(&t.f, &add_req("storage")).unwrap();
    let o = catalog(&t.f).unwrap();
    let names: Vec<&str> = o.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(
        names,
        ["spotify", "music", "script", "storage", "network", "mic", "adb", "aerospace-mode", "git-branch"]
    );
    assert!(o[3].added && !o[4].added);
    assert_eq!(o[2].kind, OfferKind::Custom);
    let text = offers_text(&o, Path::new("/w"));
    assert!(text.contains("  spotify         What Spotify is playing\n"), "{text}");
    assert!(text.contains("Catalog scripts (written to /w when added):"), "{text}");
    assert!(
        text.contains("storage         How full the startup disk is (warns at 85%, alerts at 95%) (added)\n"),
        "{text}"
    );
}

#[test]
fn display_names() {
    use WidgetType::*;
    assert_eq!(display_name("cpu", Cpu), "CPU");
    assert_eq!(display_name("spaces", Spaces), "Workspaces");
    assert_eq!(display_name("title", Title), "Window Title");
    assert_eq!(display_name("cpu-2", Cpu), "CPU 2");
    assert_eq!(display_name("clock2", Time), "Clock2");
    assert_eq!(display_name("big_clock", Time), "Big Clock");
    assert_eq!(display_name("aerospace-mode", Script), "Aerospace Mode");
    assert_eq!(display_name("script", Script), "Script");
    assert_eq!(display_name("time-x", Time), "Time X");
    assert_eq!(title_case("user-3"), "User 3");
    for t in WidgetType::ALL {
        assert!(!type_description(*t).is_empty());
    }
}

#[test]
fn an_empty_widget_says_when_it_appears() {
    let (title, text) = not_showing("spotify-2", WidgetType::Spotify, Wing::Right, None);
    assert_eq!(title, "“Spotify 2” has nothing to show yet");
    assert_eq!(
        text,
        "It is on the right wing, but empty for now, so the bar doesn't show it. It appears while Spotify is playing."
    );
    // a catalog script is recognised by its key, also when numbered
    assert_eq!(appears_when("aerospace-mode", WidgetType::Script), "while AeroSpace is in a mode other than main");
    assert_eq!(appears_when("adb-2", WidgetType::Script), "while an Android device is connected");
    // a catalog script that always shows something, and one of the person's own
    let own = "once its command prints something on its first line";
    assert_eq!(appears_when("storage", WidgetType::Script), own);
    assert_eq!(appears_when("adbx", WidgetType::Script), own);
    assert_eq!(appears_when("vpn", WidgetType::Script), own);
    assert_eq!(appears_when("cpu", WidgetType::Cpu), "after its first reading");
}

#[test]
fn a_failing_widget_says_why() {
    let (title, text) = not_showing("vpn", WidgetType::Script, Wing::Left, Some("exited with status 127"));
    assert_eq!(title, "“Vpn” isn't working");
    assert_eq!(text, "It is on the left wing, but it failed: exited with status 127");
}

#[test]
fn newly_shown_is_what_an_edit_put_on_the_bar() {
    let before = vec!["cpu".to_string(), "time".to_string()];
    let after = ["cpu", "spotify", "time", "adb"].map(String::from);
    assert_eq!(newly_shown(&before, after), ["spotify", "adb"]);
    assert!(newly_shown(&before, before.clone()).is_empty(), "a move puts nothing new on the bar");
}
