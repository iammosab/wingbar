use super::*;

fn p(args: &str) -> Result<Command, String> {
    parse(&args.split_whitespace().map(String::from).collect::<Vec<_>>())
}

fn pv(args: &[&str]) -> Result<Command, String> {
    parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

fn widget(args: &str) -> WidgetCmd {
    match p(args) {
        Ok(Command::Widget { cmd, .. }) => cmd,
        other => panic!("{args}: {other:?}"),
    }
}

fn err(args: &str) -> String {
    p(args).unwrap_err()
}

#[test]
fn running_the_bar() {
    assert_eq!(p(""), Ok(Command::Run { port: None, no_server: false, quit_after: None, snapshot: None }));
    assert_eq!(
        p("--port 47998 --no-server --quit-after 5 --snapshot /tmp/s"),
        Ok(Command::Run { port: Some(47998), no_server: true, quit_after: Some(5), snapshot: Some("/tmp/s".into()) })
    );
    assert_eq!(
        p("--port=47998").unwrap(),
        Command::Run { port: Some(47998), no_server: false, quit_after: None, snapshot: None }
    );
    assert_eq!(err("--port 0"), "--port needs a number from 1 to 65535");
    assert_eq!(err("--port x"), "--port needs a number from 1 to 65535");
    assert_eq!(err("--port"), "--port needs a value");
    assert_eq!(err("--quit-after soon"), "--quit-after needs a number of seconds");
    assert_eq!(err("--bogus"), "wingbar has no option --bogus (see wingbar --help)");
    assert_eq!(err("--no-server=1"), "--no-server takes no value");
}

#[test]
fn help_and_version() {
    for (args, text) in [
        ("--help", HELP),
        ("-h", HELP),
        ("help", HELP),
        ("help widget", WIDGET_HELP),
        ("help config", CONFIG_HELP),
        ("widget", WIDGET_HELP),
        ("widget --help", WIDGET_HELP),
        ("widget add --help", WIDGET_HELP),
        ("widget help", WIDGET_HELP),
        ("config", CONFIG_HELP),
        ("config -h", CONFIG_HELP),
        ("status --help", HELP),
    ] {
        assert_eq!(p(args), Ok(Command::Help(text)), "{args}");
    }
    assert_eq!(p("--version"), Ok(Command::Version));
    assert_eq!(p("version"), Ok(Command::Version));
    // the help says --at counts from 1
    assert!(WIDGET_HELP.contains("--at N                  position N in the wing, counting from 1\n"));
    for line in HELP.lines().chain(WIDGET_HELP.lines()).chain(CONFIG_HELP.lines()) {
        assert!(line.chars().count() <= 80, "too long: {line}");
    }
}

#[test]
fn commands_for_the_running_bar() {
    let c = |path: &str, port| Ok(Command::Client { port, path: path.into() });
    assert_eq!(p("status"), c("/status", None));
    assert_eq!(p("reload --port 47998"), c("/reload", Some(47998)));
    assert_eq!(p("--port 47998 quit"), c("/quit", Some(47998)));
    assert_eq!(p("refresh"), c("/refresh", None));
    assert_eq!(p("refresh all"), c("/refresh", None));
    assert_eq!(p("refresh spaces"), c("/aerospace/spaces/refresh", None));
    assert_eq!(p("refresh cpu"), c("/widget/cpu/refresh", None));
    assert_eq!(err("status now"), "wingbar status takes no argument (got \"now\")");
    assert_eq!(err("refresh a b"), "unexpected \"b\" (try --help)");
    assert_eq!(err("status --json"), "wingbar status has no option --json (see wingbar --help)");
    assert_eq!(err("frobnicate"), "unknown command \"frobnicate\" (try wingbar --help)");
}

#[test]
fn widget_list_and_catalog() {
    assert_eq!(widget("widget list"), WidgetCmd::List { json: false });
    assert_eq!(widget("widget list --json"), WidgetCmd::List { json: true });
    assert_eq!(widget("widget catalog"), WidgetCmd::Catalog);
    assert_eq!(
        p("widget list --port 47998"),
        Ok(Command::Widget { port: Some(47998), cmd: WidgetCmd::List { json: false } })
    );
    assert_eq!(
        err("widget list cpu"),
        "unexpected \"cpu\"; usage: wingbar widget list [--json] (quote a value with spaces)"
    );
    assert_eq!(err("widget list --left"), "wingbar widget list has no option --left (see wingbar widget --help)");
    assert_eq!(
        err("widget frob"),
        "unknown widget command \"frob\"; use list, catalog, add, remove, show, hide, move or set"
    );
}

#[test]
fn widget_add_with_every_flag() {
    assert_eq!(widget("widget add storage"), WidgetCmd::Add(Add::new("storage")));
    let full = pv(&[
        "widget",
        "add",
        "time",
        "--as",
        "clock2",
        "--left",
        "--at",
        "1",
        "--every",
        "1s",
        "--command",
        "echo hi there",
    ]);
    assert_eq!(
        full,
        Ok(Command::Widget {
            port: None,
            cmd: WidgetCmd::Add(Add {
                what: "time".into(),
                key: Some("clock2".into()),
                wing: Some(Wing::Left),
                pos: Position::At(0),
                every: Some("1s".into()),
                command: Some("echo hi there".into()),
            })
        })
    );
    let before = widget("widget add cpu --right --before date");
    assert_eq!(
        before,
        WidgetCmd::Add(Add { wing: Some(Wing::Right), pos: Position::Before("date".into()), ..Add::new("cpu") })
    );
    assert_eq!(
        widget("widget add cpu --after=time"),
        WidgetCmd::Add(Add { pos: Position::After("time".into()), ..Add::new("cpu") })
    );
    assert_eq!(
        err("widget add"),
        "missing NAME (a built-in type or catalog script; see wingbar widget catalog); usage: wingbar widget add NAME [--as KEY] [--left|--right] [--before K|--after K|--at N] [--every DUR] [--command CMD]"
    );
    assert!(err("widget add a b").starts_with("unexpected \"b\""));
    assert_eq!(err("widget add cpu --left --right"), "use --left or --right, not both");
    assert_eq!(err("widget add cpu --at 0"), "--at needs a position counting from 1 (1 = first), not \"0\"");
    assert_eq!(err("widget add cpu --at x"), "--at needs a position counting from 1 (1 = first), not \"x\"");
    assert_eq!(
        err("widget add cpu --at 1 --before date"),
        "use only one of --before, --after and --at (got --before and --at)"
    );
    assert_eq!(err("widget add cpu --as"), "--as needs a value");
    assert_eq!(err("widget add cpu --as a --as b"), "--as is given twice");
    assert_eq!(err("widget add cpu --json"), "wingbar widget add has no option --json (see wingbar widget --help)");
}

#[test]
fn widget_remove_show_hide_move() {
    assert_eq!(widget("widget remove cpu"), WidgetCmd::Remove("cpu".into()));
    assert_eq!(widget("widget rm cpu"), WidgetCmd::Remove("cpu".into()));
    assert_eq!(widget("widget hide cpu"), WidgetCmd::Hide("cpu".into()));
    assert_eq!(widget("widget show cpu"), WidgetCmd::Show { key: "cpu".into(), wing: None, pos: Position::End });
    assert_eq!(
        widget("widget show cpu --left --at 2"),
        WidgetCmd::Show { key: "cpu".into(), wing: Some(Wing::Left), pos: Position::At(1) }
    );
    assert_eq!(
        widget("widget move cpu --after memory"),
        WidgetCmd::Move { key: "cpu".into(), wing: None, pos: Position::After("memory".into()) }
    );
    assert_eq!(
        widget("widget move cpu --left"),
        WidgetCmd::Move { key: "cpu".into(), wing: Some(Wing::Left), pos: Position::End }
    );
    assert_eq!(err("widget move cpu"), "say where to: --left, --right, --before KEY, --after KEY or --at N");
    assert_eq!(err("widget remove"), "missing KEY; usage: wingbar widget remove KEY");
    assert_eq!(err("widget hide cpu --left"), "wingbar widget hide has no option --left (see wingbar widget --help)");
    assert_eq!(
        err("widget show"),
        "missing KEY; usage: wingbar widget show KEY [--left|--right] [--before K|--after K|--at N]"
    );
}

#[test]
fn widget_set_values() {
    let set = |k: &str, o: &str, v: &str| WidgetCmd::Set { key: k.into(), option: o.into(), value: v.into() };
    assert_eq!(widget("widget set storage every 30s"), set("storage", "every", "30s"));
    assert_eq!(
        pv(&["widget", "set", "date", "click", "open -a Calendar"]).unwrap(),
        Command::Widget { port: None, cmd: set("date", "click", "open -a Calendar") }
    );
    assert_eq!(
        pv(&["widget", "set", "net", "levels", "{\"ok\": \"green\"}"]).unwrap(),
        Command::Widget { port: None, cmd: set("net", "levels", "{\"ok\": \"green\"}") }
    );
    // a value that looks like a flag, after --
    assert_eq!(
        pv(&["widget", "set", "x", "command", "--", "--weird"]).unwrap(),
        Command::Widget { port: None, cmd: set("x", "command", "--weird") }
    );
    // negative numbers are values
    assert_eq!(widget("widget set x every -5"), set("x", "every", "-5"));
    assert_eq!(err("widget set cpu every"), "missing VALUE; usage: wingbar widget set KEY OPTION VALUE");
    assert_eq!(
        err("widget set date click open Calendar"),
        "unexpected \"Calendar\"; usage: wingbar widget set KEY OPTION VALUE (quote a value with spaces)"
    );
    assert_eq!(
        err("widget set date click open -a Calendar"),
        "wingbar widget set has no option -a (see wingbar widget --help); \
         quote a value with spaces, and put -- before a value that starts with -"
    );
}

#[test]
fn config_commands() {
    let c = |cmd| Ok(Command::Config { port: None, cmd });
    assert_eq!(p("config path"), c(ConfigCmd::Path));
    assert_eq!(p("config check"), c(ConfigCmd::Check));
    assert_eq!(p("config edit"), c(ConfigCmd::Edit));
    assert_eq!(
        p("config import-simplebar"),
        c(ConfigCmd::ImportSimplebar { from: None, dry_run: false, force: false })
    );
    assert_eq!(
        p("config import-simplebar --from /x/sbrc --dry-run --force"),
        c(ConfigCmd::ImportSimplebar { from: Some("/x/sbrc".into()), dry_run: true, force: true })
    );
    let home = crate::config::home();
    assert_eq!(
        p("config import-simplebar --from ~/sb.json"),
        c(ConfigCmd::ImportSimplebar { from: Some(home.join("sb.json")), dry_run: false, force: false })
    );
    assert_eq!(err("config path now"), "unexpected \"now\"; usage: wingbar config path (quote a value with spaces)");
    assert_eq!(err("config check --force"), "wingbar config check has no option --force (see wingbar config --help)");
    assert_eq!(err("config frob"), "unknown config command \"frob\"; use path, check, edit or import-simplebar");
}

// --- the commands that touch files (temp files only) -----------------------------

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wingbar-cli-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn check_reports_errors_and_warnings() {
    let d = temp("check");
    let path = d.join("config.json");
    let (code, out) = check(&path);
    assert_eq!(code, 1);
    assert!(out.contains("doesn't exist yet"), "{out}");
    std::fs::write(&path, "{ oops").unwrap();
    let (code, out) = check(&path);
    assert_eq!(code, 1);
    assert!(out.contains("not valid JSON at line 1"), "{out}");
    std::fs::write(&path, "[]").unwrap();
    assert_eq!(check(&path).0, 1);
    std::fs::write(&path, "{ \"left\": [] }").unwrap();
    let (code, out) = check(&path);
    assert_eq!(code, 0);
    assert!(out.contains("wingbar 0.2 file"), "{out}");
    let mut doc = crate::config::defaults::fresh(false);
    std::fs::write(&path, crate::config::doc::to_text(&doc)).unwrap();
    let (code, out) = check(&path);
    assert_eq!((code, out), (0, format!("{}: ok, 7 widgets (7 shown, 0 hidden)\n", path.display())));
    doc["widgets"]["cpu"]["every"] = serde_json::json!("soon");
    doc["right"].as_array_mut().unwrap().push(serde_json::json!("ghost"));
    std::fs::write(&path, crate::config::doc::to_text(&doc)).unwrap();
    let (code, out) = check(&path);
    assert_eq!(code, 0, "warnings only");
    assert!(out.contains("warning: ") && out.contains("widgets.cpu.every") && out.contains("ghost"), "{out}");
    assert!(out.ends_with(", 2 warnings\n"), "{out}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn import_refuses_to_overwrite_without_force() {
    let d = temp("import");
    let path = d.join("config.json");
    let sb = d.join("simplebarrc.json");
    assert!(import(&path, &sb, false, false).unwrap_err().contains("doesn't exist; name simple-bar's settings"));
    std::fs::write(&sb, include_str!("../../tests/fixtures/simplebarrc.json")).unwrap();
    // dry run: prints, writes nothing
    let out = import(&path, &sb, true, false).unwrap();
    assert!(out.starts_with("{\n  \"version\": 2,"), "{out}");
    assert!(!path.exists());
    // the first import writes
    let out = import(&path, &sb, false, false).unwrap();
    assert!(out.contains(&format!("imported {} into {}", sb.display(), path.display())), "{out}");
    let first = std::fs::read_to_string(&path).unwrap();
    // then: refused without --force, a dry run is still fine
    let e = import(&path, &sb, false, false).unwrap_err();
    assert!(
        e.contains("already has wingbar settings") && e.contains("--force") && e.contains("config.json.bak"),
        "{e}"
    );
    assert!(import(&path, &sb, true, false).is_ok());
    // forced: the old file is kept as config.json.bak
    std::fs::write(&path, "{ \"version\": 2, \"mine\": true }").unwrap();
    let out = import(&path, &sb, false, true).unwrap();
    assert!(out.contains("kept the old settings as"), "{out}");
    assert_eq!(std::fs::read_to_string(d.join("config.json.bak")).unwrap(), "{ \"version\": 2, \"mine\": true }");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    // a 0.2 file is merged and kept as config.v1.json, without --force
    std::fs::write(&path, "{ \"windowLevel\": \"desktop\" }").unwrap();
    import(&path, &sb, false, false).unwrap();
    assert!(d.join("config.v1.json").exists());
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["bar"]["windowLevel"], "desktop");
    // a broken settings file is not replaced
    std::fs::write(&path, "{ oops").unwrap();
    assert!(import(&path, &sb, false, true).unwrap_err().contains("not valid JSON"));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn forced_imports_never_overwrite_a_backup() {
    let d = temp("import-backups");
    let path = d.join("config.json");
    let sb = d.join("simplebarrc.json");
    std::fs::write(&sb, include_str!("../../tests/fixtures/simplebarrc.json")).unwrap();
    let mine = |n: u32| format!("{{ \"version\": 2, \"mine\": {n} }}");
    for (n, bak) in [(1, "config.json.bak"), (2, "config.json.bak-2"), (3, "config.json.bak-3")] {
        std::fs::write(&path, mine(n)).unwrap();
        let out = import(&path, &sb, false, true).unwrap();
        assert!(out.contains(&format!("kept the old settings as {}", d.join(bak).display())), "{out}");
    }
    for (n, bak) in [(1, "config.json.bak"), (2, "config.json.bak-2"), (3, "config.json.bak-3")] {
        assert_eq!(std::fs::read_to_string(d.join(bak)).unwrap(), mine(n), "{bak}");
    }
    // the refusal names the backup that --force would make
    let e = import(&path, &sb, false, false).unwrap_err();
    assert!(e.contains("config.json.bak-4"), "{e}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn import_refuses_a_source_that_is_not_a_json_object() {
    let d = temp("import-bad-source");
    let path = d.join("config.json");
    let sb = d.join("simplebarrc.json");
    std::fs::write(&path, "{ \"version\": 2, \"mine\": true }").unwrap();
    for (text, says) in [("{ oops", "not valid JSON"), ("[1, 2]", "JSON object"), ("", "not valid JSON")] {
        std::fs::write(&sb, text).unwrap();
        for (dry_run, force) in [(false, true), (true, false), (false, false)] {
            let e = import(&path, &sb, dry_run, force).unwrap_err();
            assert!(e.contains(says) && e.contains("simplebarrc.json") && e.contains("nothing was written"), "{e}");
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ \"version\": 2, \"mine\": true }");
        assert!(!d.join("config.json.bak").exists());
    }
    // nor without settings yet
    std::fs::remove_file(&path).unwrap();
    assert!(import(&path, &sb, false, false).is_err());
    assert!(!path.exists());
    // nor into the file it reads
    std::fs::write(&sb, include_str!("../../tests/fixtures/simplebarrc.json")).unwrap();
    assert!(import(&sb, &sb, false, true).unwrap_err().contains("simple-bar"));
    assert_eq!(std::fs::read_to_string(&sb).unwrap(), include_str!("../../tests/fixtures/simplebarrc.json"));
    assert!(!d.join("simplebarrc.json.bak").exists());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn widget_commands_write_the_settings_only_when_they_may() {
    use crate::config::Pending::{Missing, V1};
    let never = || -> RunningBar { panic!("the port need not be asked") };
    // a version 2 file: as it is, the bar isn't asked
    assert_eq!(before_widget_command(true, None, never, 7999), Before::AsItIs);
    assert_eq!(before_widget_command(false, None, never, 7999), Before::AsItIs);
    // list and catalog never write: they show what the first start will write
    for p in [Missing, V1] {
        assert_eq!(before_widget_command(false, Some(p), never, 7999), Before::Preview(p));
    }
    // an edit creates the file, unless an older wingbar runs on these settings
    for p in [Missing, V1] {
        assert_eq!(before_widget_command(true, Some(p), || RunningBar::None, 7999), Before::Create);
        assert_eq!(
            before_widget_command(true, Some(p), || RunningBar::Uses("/x/config.json".into()), 7999),
            Before::Create
        );
        assert_eq!(
            before_widget_command(true, Some(p), || RunningBar::Other, 7999),
            Before::Refuse(
                "an older wingbar is running on port 7999 and uses these settings; \
                 install this version first (./install.sh), then run the command again"
                    .into()
            )
        );
    }
}

#[test]
fn reload_answers_name_the_file() {
    assert_eq!(reloaded_path("reloaded /tmp/x/config.json\nadded: cpu\n"), Some("/tmp/x/config.json"));
    assert_eq!(reloaded_path("nope"), None);
    assert_eq!(backup_path(Path::new("/a/config.json"), &|_: &Path| false), PathBuf::from("/a/config.json.bak"));
    let taken = |p: &Path| p.ends_with("config.json.bak") || p.ends_with("config.json.bak-2");
    assert_eq!(backup_path(Path::new("/a/config.json"), &taken), PathBuf::from("/a/config.json.bak-3"));
}

#[test]
fn request_talks_to_the_server() {
    // the bar's server on an ephemeral loopback port (never the bar's own)
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        crate::server::handle(listener.accept().unwrap().0, &crate::widgets::fixture_registry())
    });
    let (code, body) = request(port, "/widgets", Duration::from_secs(2)).unwrap();
    server.join().unwrap();
    assert_eq!(code, 200);
    assert!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["widgets"].is_object());
}
