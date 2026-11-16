use super::parse::{duration_ms, format_duration, parse};
use super::*;
use serde_json::json;

fn no_fs(_: &Path) -> bool {
    false
}

fn read(doc: Value) -> (Config, Vec<String>) {
    parse(&doc, &no_fs)
}

fn rgb(hex: &str) -> Rgba {
    Rgba::hex(hex).unwrap()
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn has(warnings: &[String], parts: &[&str]) -> bool {
    warnings.iter().any(|w| parts.iter().all(|p| w.contains(p)))
}

#[test]
fn the_settings_path_is_always_absolute() {
    let cwd = Path::new("/work/here");
    let from = |v: Option<&str>| settings_path_from(v.map(std::ffi::OsString::from), cwd);
    // a relative WINGBAR_CONFIG is taken from the folder the command runs in
    assert_eq!(from(Some("config.json")), PathBuf::from("/work/here/config.json"));
    assert_eq!(from(Some("./t/../t/c.json")), PathBuf::from("/work/here/t/c.json"));
    assert_eq!(from(Some("/abs/c.json")), PathBuf::from("/abs/c.json"));
    assert_eq!(from(Some("~/c.json")), home().join("c.json"));
    assert_eq!(from(None), home().join(".config/wingbar/config.json"));
    assert_eq!(from(Some("")), home().join(".config/wingbar/config.json"));
    assert!(settings_path().is_absolute());
}

#[test]
fn simple_bars_file_is_never_the_settings_file() {
    let t = Temp::new("not-simplebarrc");
    let sb = t.0.join(".simplebarrc");
    std::fs::write(&sb, "{}").unwrap();
    let refused = |p: &Path| check_not_simplebars(p, &sb).unwrap_err();
    let e = refused(&sb);
    assert!(e.contains("WINGBAR_CONFIG") && e.contains("simple-bar") && e.contains(".simplebarrc"), "{e}");
    // any file of that name, existing or not
    refused(&t.0.join("elsewhere/.simplebarrc"));
    // a link to it
    let link = t.0.join("config.json");
    std::os::unix::fs::symlink(&sb, &link).unwrap();
    refused(&link);
    // the simple-bar file in use, whatever it is called
    let other = t.0.join("sb.json");
    std::fs::write(&other, "{}").unwrap();
    assert!(check_not_simplebars(&other, &other).is_err());
    // wingbar's own file is fine
    assert_eq!(check_not_simplebars(&t.0.join("wingbar/config.json"), &sb), Ok(()));
    assert_eq!(check_not_simplebars(&other, &sb), Ok(()));
}

#[test]
fn home_is_expanded_in_every_spelling() {
    let h = home().display().to_string();
    assert_eq!(expand("~"), h);
    assert_eq!(expand("~/a b"), format!("{h}/a b"));
    assert_eq!(expand("$HOME/x"), format!("{h}/x"));
    assert_eq!(expand("${HOME}/x"), format!("{h}/x"));
    assert_eq!(expand("a ${HOME} $HOME"), format!("a {h} {h}"));
    assert_eq!(expand("/abs/~x"), "/abs/~x");
    assert_eq!(expand("~other/x"), "~other/x");
}

#[test]
fn hex_six_and_eight_digits() {
    assert_eq!(Rgba::hex("#ff0000"), Some(Rgba(1.0, 0.0, 0.0, 1.0)));
    assert_eq!(Rgba::hex("00ff00"), Some(Rgba(0.0, 1.0, 0.0, 1.0)));
    assert_eq!(Rgba::hex("  #0000FF  "), Some(Rgba(0.0, 0.0, 1.0, 1.0)));
    assert_eq!(Rgba::hex("#ffffff00"), Some(Rgba(1.0, 1.0, 1.0, 0.0)));
    let c = Rgba::hex("80808080").unwrap();
    assert!((c.0 - 128.0 / 255.0).abs() < 1e-9 && (c.3 - 128.0 / 255.0).abs() < 1e-9);
}

#[test]
fn hex_three_digits_expand() {
    assert_eq!(Rgba::hex("#fff"), Rgba::hex("#ffffff"));
    assert_eq!(Rgba::hex("a1c"), Rgba::hex("aa11cc"));
}

#[test]
fn hex_rejects_invalid() {
    for bad in ["", "#", "black", "#12", "#1234", "#12345", "#1234567", "#zzzzzz", "#ff00zz", "ééé", "#ff0000ff00"] {
        assert_eq!(Rgba::hex(bad), None, "{bad:?}");
    }
}

#[test]
fn colours_by_hex_theme_key_or_palette_name() {
    let (cfg, w) = read(
        json!({ "version": 2, "theme": { "warn": "#123", "colors": { "brand": "#00ff00", "--odd": "#0000ff" } } }),
    );
    assert!(w.is_empty(), "{w:?}");
    let t = &cfg.theme;
    assert_eq!(t.color("#ff0000"), Some(rgb("#ff0000")));
    assert_eq!(t.color("#ff000080"), Some(Rgba(1.0, 0.0, 0.0, 128.0 / 255.0)));
    assert_eq!(t.color("warn"), Some(rgb("#112233")));
    assert_eq!(t.color("--warn"), Some(rgb("#112233")));
    assert_eq!(t.color("dim"), Some(rgb("#8e8e93")));
    assert_eq!(t.color("wingHover"), Some(rgb("#161618")));
    assert_eq!(t.color("brand"), Some(rgb("#00ff00")));
    assert_eq!(t.color(" --brand "), Some(rgb("#00ff00")));
    assert_eq!(t.color("odd"), Some(rgb("#0000ff")), "a leading -- on a colour's name is dropped");
    assert_eq!(t.color("red"), Some(rgb("#fc5d7c")), "simple-bar's palette is the default");
    assert_eq!(t.color("--main-alt"), Some(rgb("#a7aab4")));
    assert_eq!(t.color("nope"), None);
    assert_eq!(t.color("rgb(1,2,3)"), None);
}

#[test]
fn durations() {
    for (v, ms) in [
        (json!("500ms"), 500),
        (json!("15s"), 15_000),
        (json!("2m"), 120_000),
        (json!("1h"), 3_600_000),
        (json!("1.5s"), 1500),
        (json!(" 3 s "), 3000),
        (json!(750), 750),
        (json!(2500.7), 2500),
    ] {
        assert_eq!(duration_ms(&v), Some(ms), "{v}");
    }
    for bad in
        [json!("fast"), json!("10"), json!("5x"), json!("s"), json!(-1), json!(true), json!(null), json!("1.2.3s")]
    {
        assert_eq!(duration_ms(&bad), None, "{bad}");
    }
    assert_eq!(format_duration(500), "500ms");
    assert_eq!(format_duration(1500), "1500ms");
    assert_eq!(format_duration(15_000), "15s");
    assert_eq!(format_duration(150_000), "150s");
    assert_eq!(format_duration(120_000), "2m");
    for ms in [500, 1500, 15_000, 120_000, 600_000] {
        assert_eq!(duration_ms(&json!(format_duration(ms))), Some(ms));
    }
}

#[test]
fn an_empty_document_is_all_defaults() {
    let (cfg, w) = read(Value::Null);
    assert!(w.is_empty(), "{w:?}");
    assert!(cfg.widgets.is_empty() && cfg.left.is_empty() && cfg.right.is_empty());
    let b = &cfg.bar;
    assert_eq!((b.port, b.server, b.level_above_windows, b.hide_empty_spaces), (7776, true, true, true));
    assert_eq!((b.font_size, b.title_max, b.volume_step), (None, 420.0, 5.0));
    assert_eq!((b.popover_bin.as_deref(), b.popover_daemon.as_str()), (None, "127.0.0.1:7779"));
    assert_eq!(cfg.theme.wing, rgb("#000000"));
    assert_eq!(cfg.theme.text, rgb("#f2f2f2"));
    assert_eq!(cfg.theme.named.len(), 15);
}

#[test]
fn fresh_default_reads_without_warnings() {
    let (cfg, w) = read(defaults::fresh(true));
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(cfg.left, strings(&["spaces", "title"]));
    assert_eq!(cfg.right, strings(&["cpu", "memory", "battery", "sound", "date", "time"]));
    assert_eq!(cfg.hidden().count(), 0);
    assert_eq!(
        cfg.def("cpu").unwrap().options,
        Options::Cpu(Sampled { every_ms: 2000, click: parse::ACTIVITY_MONITOR.into() })
    );
    let (cfg, w) = read(defaults::fresh(false));
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(cfg.left, strings(&["title"]), "no window manager: no workspaces");
    // nothing personal in a fresh install
    let text = doc::to_text(&defaults::fresh(true)).to_lowercase();
    for word in ["clickup", "claude", "salat", "adb", "barpopover", "simple-bar"] {
        assert!(!text.contains(word), "{word}");
    }
}

#[test]
fn config_example_reads_without_warnings() {
    let ex: Value = serde_json::from_str(include_str!("../../config.example.json")).unwrap();
    let (cfg, w) = read(ex.clone());
    assert!(w.is_empty(), "{w:?}");
    let fresh = defaults::fresh(true);
    assert_eq!(ex["left"], fresh["left"]);
    assert_eq!(ex["right"], fresh["right"]);
    assert_eq!(cfg.wing_of("network"), None, "the script example is defined, hidden");
    let Options::Script(s) = &cfg.def("network").unwrap().options else { panic!("a script") };
    assert_eq!(s.levels[1].icon.as_deref(), Some("cable.connector"));
}

#[test]
fn every_option_reads() {
    let doc = json!({
        "version": 2,
        "windowManager": "yabai",
        "yabaiPath": "/nonexistent/yabai",
        "aerospacePath": "",
        "bar": {
            "fontSize": 13, "titleMaxWidth": 300, "windowLevel": "desktop", "volumeStep": 2,
            "port": 7001, "server": false, "hideEmptySpaces": false, "spacesExclusions": ["9"],
            "popoverBinary": "/nonexistent/pop", "popoverDaemon": "127.0.0.1:9",
        },
        "theme": { "wing": "#111", "wingHover": "black", "text": "#eeeeee", "dim": "#010101", "faint": "#020202",
                   "warn": "#030303", "alert": "red", "colors": { "red": "#ff0000" } },
        "left": ["s", "t"],
        "right": ["c", "m", "b", "snd", "d", "clock", "clock2", "sp", "mu", "x"],
        "widgets": {
            "s": { "type": "spaces" },
            "t": { "type": "title" },
            "c": { "type": "cpu", "every": "1s", "click": "" },
            "m": { "type": "memory", "every": 1500, "click": "open -a htop" },
            "b": { "type": "battery", "click": "pmset sleepnow", "caffeinateOption": "-i" },
            "snd": { "type": "sound" },
            "d": { "type": "date", "click": "" },
            "clock": { "type": "time", "hour12": true, "seconds": true, "click": "open -a Clock" },
            "clock2": { "type": "time" },
            "sp": { "type": "spotify" },
            "mu": { "type": "music" },
            "x": {
                "type": "script", "command": "echo hi", "every": "2m", "timeout": "5s", "icon": "star",
                "levels": { "ok": "green", "busy": { "color": "#00f", "icon": "hourglass" }, "quiet": { "icon": "moon" } },
                "click": "a", "rightClick": "b", "middleClick": "c", "menuOn": "hover", "popover": true,
                "hideWhenEmpty": false, "iconOnly": true, "greyWhenOk": true, "iconFromLabel": "prayer",
                "prelude": "~/env.sh", "cacheFile": "$TMPDIR/x.out", "simplebarIds": [3, 4], "raw": true,
            },
        },
    });
    let (cfg, w) = read(doc);
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(cfg.wm, Wm::Yabai("yabai".into()), "not found: the bare name");
    let b = &cfg.bar;
    assert_eq!(
        b,
        &Bar {
            font_size: Some(13.0),
            title_max: 300.0,
            level_above_windows: false,
            volume_step: 2.0,
            port: 7001,
            server: false,
            hide_empty_spaces: false,
            space_exclusions: strings(&["9"]),
            popover_bin: None,
            popover_daemon: "127.0.0.1:9".into(),
        }
    );
    let t = &cfg.theme;
    assert_eq!((t.wing, t.wing_hover, t.alert), (rgb("#111111"), rgb("#181819"), rgb("#ff0000")));
    let o = |k: &str| cfg.def(k).unwrap().options.clone();
    assert_eq!(o("c"), Options::Cpu(Sampled { every_ms: 1000, click: String::new() }));
    assert_eq!(o("m"), Options::Memory(Sampled { every_ms: 1500, click: "open -a htop".into() }));
    assert_eq!(
        o("b"),
        Options::Battery(BatteryOptions { click: "pmset sleepnow".into(), caffeinate_option: "-i".into() })
    );
    assert_eq!(o("d"), Options::Date(String::new()));
    assert_eq!(o("clock"), Options::Time(TimeOptions { hour12: true, seconds: true, click: "open -a Clock".into() }));
    assert_eq!(o("clock2"), Options::Time(TimeOptions { hour12: false, seconds: false, click: String::new() }));
    let Options::Script(s) = o("x") else { panic!() };
    assert_eq!(
        (s.command.as_str(), s.every_ms, s.timeout_ms, s.icon.as_deref()),
        ("echo hi", 120_000, 5000, Some("star"))
    );
    assert_eq!(
        s.levels,
        [
            Level { name: "ok".into(), color: Some(rgb("#9ed072")), icon: None },
            Level { name: "busy".into(), color: Some(rgb("#0000ff")), icon: Some("hourglass".into()) },
            Level { name: "quiet".into(), color: None, icon: Some("moon".into()) },
        ]
    );
    assert_eq!(s.clicks, Clicks { left: "a".into(), right: "b".into(), middle: "c".into() });
    assert_eq!(s.menu_on, Some(MenuOn::Hover));
    assert!(s.popover && !s.hide_when_empty && s.icon_only && s.grey_when_ok);
    assert_eq!(s.icon_from_label, Some(LabelIcons::Prayer));
    assert_eq!(s.prelude, Some(format!("{}/env.sh", home().display())));
    assert_eq!(s.cache_file, Some(format!("{}x.out", crate::sys::user_temp_dir())));
    assert_eq!(s.simplebar_ids, [3, 4]);
    assert!(s.raw);
    // the order of the placed widgets
    let placed: Vec<(Wing, &str)> = cfg.placed().map(|(w, d)| (w, d.key.as_str())).collect();
    assert_eq!(placed[..3], [(Wing::Left, "s"), (Wing::Left, "t"), (Wing::Right, "c")]);
}

#[test]
fn script_defaults() {
    let (cfg, w) = read(json!({ "version": 2, "widgets": { "x": { "type": "script", "command": "date" } } }));
    assert!(w.is_empty(), "{w:?}");
    let Options::Script(s) = &cfg.def("x").unwrap().options else { panic!() };
    assert_eq!((s.every_ms, s.timeout_ms, s.icon.clone(), s.levels.len()), (10_000, 30_000, None, 0));
    assert!(s.hide_when_empty && !s.icon_only && !s.grey_when_ok && !s.popover);
    assert_eq!((s.menu_on, s.icon_from_label, s.prelude.clone(), s.cache_file.clone()), (None, None, None, None));
    assert!(s.simplebar_ids.is_empty() && !s.raw);
    assert_eq!(cfg.hidden().count(), 1, "defined but placed nowhere: hidden");
}

#[test]
fn wrong_values_warn_and_keep_the_rest() {
    let doc = json!({
        "version": 2,
        "windowManager": "i3",
        "bar": { "fontSize": "big", "titleMaxWidth": 300, "server": "yes", "port": 70000, "windowLevel": "top",
                 "spacesExclusions": ["a", 3] },
        "theme": "dark",
        "left": "spaces",
        "right": ["c", 3, "s"],
        "widgets": {
            "c": { "type": "cpu", "every": "soon" },
            "s": { "type": "script", "command": "x", "every": "100ms", "levels": { "ok": "no-such", "warn": 5 },
                   "simplebarIds": [1, "2", -3], "menuOn": "always", "iconFromLabel": "weather", "hideWhenEmpty": 0 },
        },
    });
    let (cfg, w) = read(doc);
    assert_eq!(cfg.bar.title_max, 300.0, "good values still apply");
    assert!(cfg.bar.server && cfg.bar.level_above_windows);
    assert_eq!((cfg.bar.font_size, cfg.bar.port), (None, 7776));
    assert_eq!(cfg.bar.space_exclusions, strings(&["a"]));
    assert_eq!(cfg.theme.wing, rgb("#000000"));
    assert!(cfg.left.is_empty());
    assert_eq!(cfg.right, strings(&["c", "s"]));
    assert_eq!(
        cfg.def("c").unwrap().options,
        Options::Cpu(Sampled { every_ms: 2000, click: parse::ACTIVITY_MONITOR.into() })
    );
    let Options::Script(s) = &cfg.def("s").unwrap().options else { panic!() };
    assert_eq!(s.every_ms, 500);
    assert_eq!(s.levels[0].color, Some(cfg.theme.dim), "an unknown colour is dim, as before");
    assert_eq!(s.levels[1].color, Some(cfg.theme.dim));
    assert_eq!(s.simplebar_ids, [1]);
    assert!(s.hide_when_empty && s.menu_on.is_none() && s.icon_from_label.is_none());
    for parts in [
        &["windowManager", "\"i3\""][..],
        &["bar.fontSize", "\"big\""],
        &["bar.server", "\"yes\""],
        &["bar.port", "70000"],
        &["bar.windowLevel", "\"top\""],
        &["bar.spacesExclusions[1]", "3"],
        &["invalid theme = \"dark\""],
        &["invalid left = \"spaces\""],
        &["right[1]"],
        &["widgets.c.every", "\"soon\""],
        &["widgets.s.every", "500 ms minimum"],
        &["widgets.s.levels.ok", "\"no-such\""],
        &["widgets.s.levels.warn", "5"],
        &["widgets.s.simplebarIds", "\"2\""],
        &["widgets.s.simplebarIds", "-3"],
        &["widgets.s.menuOn", "\"always\""],
        &["widgets.s.iconFromLabel", "\"weather\""],
        &["widgets.s.hideWhenEmpty", "0"],
    ] {
        assert!(has(&w, parts), "{parts:?}: {w:#?}");
    }
    assert_eq!(w.len(), 18, "{w:#?}");
}

#[test]
fn unknown_keys_and_types_warn_once_each() {
    let doc = json!({
        "version": 2,
        "//": "a comment",
        "// more": { "anything": 1 },
        "colours": {},
        "Left": [],
        "bar": { "height": 30, "// note": 1 },
        "theme": { "shadow": "#000", "colors": { "// c": "x" } },
        "left": ["w", "nt", "nope", "bad"],
        "widgets": {
            "// off": { "type": "cpu" },
            "w": { "type": "weather" },
            "nt": { "every": "1s" },
            "bad": "cpu",
            "c": { "type": "cpu", "colour": "red", "// why": "comments are fine" },
            "s": { "type": "script", "command": "x", "levels": { "ok": { "color": "green", "size": 2 } } },
        },
    });
    let (cfg, w) = read(doc);
    assert_eq!(cfg.widgets.iter().map(|d| d.key.as_str()).collect::<Vec<_>>(), ["c", "s"]);
    assert!(cfg.left.is_empty());
    for parts in [
        &["unknown key \"colours\""][..],
        &["unknown key \"Left\""],
        &["unknown key \"height\" in bar"],
        &["unknown key \"shadow\" in theme"],
        &["widgets.w", "unknown type \"weather\"", "script"],
        &["widgets.nt has no \"type\""],
        &["invalid widgets.bad = \"cpu\""],
        &["unknown key \"colour\" in widgets.c"],
        &["unknown key \"size\" in widgets.s.levels.ok"],
        &["\"nope\" in \"left\" has no definition"],
    ] {
        assert!(has(&w, parts), "{parts:?}: {w:#?}");
    }
    assert_eq!(w.len(), 10, "skipped definitions don't warn again when placed: {w:#?}");
}

#[test]
fn placement_rules() {
    let doc = json!({
        "version": 2,
        "left": ["a", "b", "a"],
        "right": ["b", "c", "Up"],
        "widgets": { "a": { "type": "sound" }, "b": { "type": "sound" }, "c": { "type": "sound" },
                     "d": { "type": "sound" }, "Up": { "type": "sound" } },
    });
    let (cfg, w) = read(doc);
    assert_eq!(cfg.left, strings(&["a", "b"]));
    assert_eq!(cfg.right, strings(&["c", "Up"]), "an odd key still works, with a warning");
    assert_eq!(cfg.hidden().map(|d| d.key.as_str()).collect::<Vec<_>>(), ["d"]);
    assert!(has(&w, &["\"a\" is placed twice", "\"left\" and \"left\""]), "{w:?}");
    assert!(has(&w, &["\"b\" is placed twice", "\"left\" and \"right\"", "keeping the first"]), "{w:?}");
    assert!(has(&w, &["\"Up\"", "a-z, 0-9"]), "{w:?}");
    assert_eq!(w.len(), 3, "{w:?}");
    let (_, w) = read(json!({ "left": [] }));
    assert!(has(&w, &["no \"version\"", "add \"version\": 2"]), "{w:?}");
    let (_, w) = read(json!({ "version": 3 }));
    assert!(has(&w, &["version = 3"]), "{w:?}");
}

#[test]
fn window_manager_and_binaries() {
    let fs = |p: &Path| p == Path::new("/usr/local/bin/yabai") || p == Path::new("/custom/aerospace");
    let wm = |doc: Value| parse(&doc, &fs).0.wm;
    assert_eq!(wm(json!({ "version": 2 })), Wm::Yabai("/usr/local/bin/yabai".into()), "auto: what is installed");
    assert_eq!(wm(json!({ "version": 2, "windowManager": "none" })), Wm::None);
    assert_eq!(
        wm(json!({ "version": 2, "windowManager": "aerospace", "aerospacePath": "/custom/aerospace" })),
        Wm::Aerospace("/custom/aerospace".into())
    );
    assert_eq!(
        wm(json!({ "version": 2, "windowManager": "aerospace", "aerospacePath": "$(which aerospace)" })),
        Wm::Aerospace("aerospace".into())
    );
    assert_eq!(parse(&json!({ "version": 2 }), &no_fs).0.wm, Wm::None);
    let pop = |p: &Path| p == Path::new("/x/pop");
    let doc = json!({ "version": 2, "bar": { "popoverBinary": "/x/pop" } });
    assert_eq!(parse(&doc, &pop).0.bar.popover_bin.as_deref(), Some("/x/pop"));
    assert_eq!(parse(&doc, &no_fs).0.bar.popover_bin, None, "only when it exists");
}

#[test]
fn fixture_is_the_imported_simplebarrc_and_v1_config() {
    let cfg = fixture();
    assert_eq!(cfg.left, strings(&["spaces", "title", "clickup"]));
    assert_eq!(cfg.right, strings(&["salat", "user-11", "date", "battery", "sound", "music", "time", "user-3"]));
    assert_eq!(cfg.theme.wing, rgb("#111111"));
    assert!(!cfg.bar.level_above_windows);
}

// ── the startup flow, in temp folders ──

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        let d = std::env::temp_dir().join(format!("wingbar-config-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Temp(d)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_text() -> &'static str {
    include_str!("../../tests/fixtures/simplebarrc.json")
}

#[test]
fn missing_settings_are_imported_from_simplebarrc() {
    let t = Temp::new("import");
    let sb = t.0.join("simplebarrc.json");
    std::fs::write(&sb, fixture_text()).unwrap();
    let path = t.0.join("wingbar/config.json");
    let p = prepare(&path, &sb, &exists);
    assert!(p.messages[0].starts_with("imported ") && p.messages[0].contains("shown"), "{:?}", p.messages);
    let written = doc::Document::load(&path).unwrap().value;
    assert_eq!(written, p.doc);
    assert_eq!(written["version"], 2);
    assert!(written["widgets"]["clickup"].is_object());
    assert_eq!(std::fs::read_to_string(&sb).unwrap(), fixture_text(), ".simplebarrc is never changed");
    // the next start reads the file as it is
    let again = prepare(&path, &sb, &exists);
    assert!(again.messages.is_empty(), "{:?}", again.messages);
    assert_eq!(again.doc, written);
}

#[test]
fn a_preview_is_what_the_first_start_writes_and_writes_nothing() {
    let t = Temp::new("preview");
    let sb = t.0.join("simplebarrc.json");
    std::fs::write(&sb, fixture_text()).unwrap();
    let path = t.0.join("wingbar/config.json");
    assert_eq!(pending(&path), Some(Pending::Missing));
    let seen = preview_with(&path, &sb, &exists);
    assert!(!path.exists() && !t.0.join("wingbar").exists(), "nothing written");
    assert_eq!(seen, prepare(&path, &sb, &exists).doc, "what the first start writes");
    assert_eq!(pending(&path), None, "a version 2 file");
    // a 0.2 file: merged, in memory only
    let v1 = json!({ "left": ["clickup"], "right": ["time"] });
    std::fs::write(&path, v1.to_string()).unwrap();
    assert_eq!(pending(&path), Some(Pending::V1));
    let seen = preview_with(&path, &sb, &exists);
    assert_eq!(seen["left"], json!(["clickup"]));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), v1.to_string(), "not moved or changed");
    assert!(!t.0.join("wingbar/config.v1.json").exists());
    // a broken file is no pending one
    std::fs::write(&path, "{ oops").unwrap();
    assert_eq!(pending(&path), None);
}

#[test]
fn nothing_to_import_writes_the_fresh_default() {
    let t = Temp::new("fresh");
    let path = t.0.join("config.json");
    let p = prepare(&path, &t.0.join("no-simplebarrc"), &|p: &Path| p.exists() && !p.starts_with("/opt"));
    assert_eq!(p.messages, [format!("wrote the default settings to {}", path.display())]);
    let written = doc::Document::load(&path).unwrap().value;
    assert_eq!(written, p.doc);
    assert_eq!(written["right"], json!(["cpu", "memory", "battery", "sound", "date", "time"]));
    let (_, w) = parse(&written, &no_fs);
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn a_v1_file_is_merged_and_kept() {
    let t = Temp::new("v1");
    let sb = t.0.join("other-simplebarrc.json");
    std::fs::write(&sb, fixture_text()).unwrap();
    let path = t.0.join("config.json");
    let v1 = json!({ "simplebarrc": sb.display().to_string(), "left": ["clickup"], "right": ["time"], "port": 7001 });
    std::fs::write(&path, v1.to_string()).unwrap();
    std::fs::write(t.0.join("config.v1.json"), "older").unwrap();
    let p = prepare(&path, &t.0.join("unused"), &exists);
    assert!(p.messages[0].contains("0.2 file") && p.messages[0].contains("config.v1-2.json"), "{:?}", p.messages);
    let doc = doc::Document::load(&path).unwrap().value;
    assert_eq!(doc["left"], json!(["clickup"]), "the wing order of the 0.2 file");
    assert_eq!(doc["bar"]["port"], 7001);
    assert!(doc["widgets"]["salat"].is_object(), "and the widgets of the simplebarrc it named");
    let kept: Value = serde_json::from_str(&std::fs::read_to_string(t.0.join("config.v1-2.json")).unwrap()).unwrap();
    assert_eq!(kept, v1);
    assert_eq!(std::fs::read_to_string(t.0.join("config.v1.json")).unwrap(), "older", "never overwritten");
}

#[test]
fn a_version_2_file_without_its_version_is_read_as_version_2() {
    let t = Temp::new("no-version");
    let path = t.0.join("config.json");
    let text = r#"{ "right": ["t", "x"], "widgets": { "t": { "type": "time" }, "x": { "type": "script", "command": "echo x" } } }"#;
    std::fs::write(&path, text).unwrap();
    let p = prepare(&path, &t.0.join("none"), &exists);
    assert!(p.messages.is_empty(), "nothing migrated: {:?}", p.messages);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "left as it is");
    assert!(!t.0.join("config.v1.json").exists());
    let (cfg, warnings) = parse(&p.doc, &no_fs);
    assert_eq!(cfg.right, strings(&["t", "x"]), "its widgets are kept");
    assert!(has(&warnings, &["add \"version\": 2"]), "{warnings:?}");
    // any one of the keys only version 2 has is enough
    for key in ["widgets", "theme", "bar", "windowManager"] {
        assert!(!is_v1(&json!({ key: {}, "left": [] })), "{key}");
    }
    // a 0.2 file has none of them
    assert!(is_v1(&json!({ "left": ["clickup"], "port": 7001, "colors": {} })));
    assert!(!is_v1(&json!({ "version": 2 })) && !is_v1(&json!([])));
}

#[test]
fn a_broken_file_is_left_alone() {
    let t = Temp::new("broken");
    let path = t.0.join("config.json");
    std::fs::write(&path, "{ \"version\": 2,").unwrap();
    let p = prepare(&path, &t.0.join("none"), &no_fs);
    assert!(p.messages[0].contains("not valid JSON") && p.messages[0].contains("line 1"), "{:?}", p.messages);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ \"version\": 2,");
    assert_eq!(p.doc["left"], json!(["title"]), "the defaults, in memory");
}

#[test]
fn reading_for_a_reload() {
    let path = Path::new("/nonexistent/wingbar/config.json");
    let read = |text: &str| from_text(path, Ok(text.to_string()), &no_fs);
    let e = from_text(path, Err(std::io::ErrorKind::NotFound.into()), &no_fs).unwrap_err();
    assert!(e.starts_with("can't read /nonexistent/wingbar/config.json: "), "{e}");
    let e = read("{ \"version\": 2, ").unwrap_err();
    assert!(e.contains("not valid JSON") && e.contains("line 1"), "{e}");
    assert!(read("[]").unwrap_err().contains("must be a JSON object"));
    let (cfg, warnings) =
        read(r#"{ "version": 2, "right": ["t"], "widgets": { "t": { "type": "time" } }, "bogus": 1 }"#).unwrap();
    assert_eq!(cfg.right, strings(&["t"]));
    assert!(has(&warnings, &["bogus"]), "{warnings:?}");
}

#[test]
fn port_and_server_changes_wait_for_a_restart() {
    let running = fixture();
    let mut cfg = running.clone();
    assert!(keep_restart_only(&mut cfg, &running).is_empty());
    cfg.bar.port = running.bar.port + 1;
    cfg.bar.server = !running.bar.server;
    cfg.bar.title_max = 99.0;
    let notes = keep_restart_only(&mut cfg, &running);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(notes[0].starts_with("bar.port is now ") && notes[1].starts_with("bar.server is now "), "{notes:?}");
    assert_eq!((cfg.bar.port, cfg.bar.server), (running.bar.port, running.bar.server), "kept as running");
    assert_eq!(cfg.bar.title_max, 99.0, "everything else applies");
}

#[test]
fn current_settings_can_be_replaced() {
    // the only test that sets the current settings
    let mut cfg = fixture();
    cfg.bar.port = 7123;
    let a = set_current(cfg.clone());
    assert_eq!(current().bar.port, 7123);
    cfg.bar.port = 7124;
    set_current(cfg);
    assert_eq!(current().bar.port, 7124);
    assert_eq!(a.bar.port, 7123, "a reader keeps the settings it took");
}

#[test]
fn menus_are_drawn_by_wingbar_unless_native_is_asked_for() {
    assert_eq!(read(json!({ "version": 2 })).0.theme.menus, MenuStyle::Wings);
    let (c, w) = read(json!({ "version": 2, "theme": { "menus": "native" } }));
    assert_eq!((c.theme.menus, w.len()), (MenuStyle::Native, 0));
    let (c, w) = read(json!({ "version": 2, "theme": { "menus": "fancy" } }));
    assert_eq!(c.theme.menus, MenuStyle::Wings, "a bad value keeps the default");
    assert!(has(&w, &["theme.menus"]), "{w:?}");
}
