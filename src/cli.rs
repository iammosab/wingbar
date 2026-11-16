//! The command line: arguments → a `Command` (pure, tested), and running the
//! commands that don't start the bar (`widget`, `config`, and the ones that
//! talk to a running bar). Arguments are parsed by hand: flags may come
//! anywhere after the command; `--` ends the flags (for a value that starts
//! with `--`).

use crate::config::edit::Position;
use crate::config::{self, Wing};
use crate::manage::{self, Add, Files};
use std::collections::HashMap;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const HELP: &str = "\
wingbar: a native macOS status bar

Usage:
  wingbar [--port N] [--no-server] [--quit-after SECS] [--snapshot DIR]
                                 run the bar
  wingbar status                 what the running bar shows, and its settings
  wingbar reload                 apply the settings file now (saving does too)
  wingbar refresh [WIDGET]       run one widget again now, or all of them
  wingbar quit                   stop the running bar
  wingbar widget COMMAND ...     list, add, remove, show, hide, move and set
                                 widgets (wingbar widget --help)
  wingbar config COMMAND ...     the settings file: path, check, edit and
                                 import-simplebar (wingbar config --help)
  wingbar --version

Options:
  --port N          the port the bar listens on (default: bar.port in the
                    settings, 7776); with a command, the running bar's port
  --no-server       run without the local HTTP server (commands can't reach it)
  --quit-after N    quit after N seconds
  --snapshot DIR    save a picture of each bar into DIR after 5 s, then quit
  -h, --help        this text

The settings file is $WINGBAR_CONFIG, else ~/.config/wingbar/config.json.
Right-click a wing for the wingbar menu: move, hide, remove and add widgets.
On a widget with its own right-click action, use ⌥-right-click or ⌃-click.
";

pub const WIDGET_HELP: &str = "\
Usage: wingbar widget COMMAND [ARGS]

  list [--json]           every widget: key, type, wing, position, shown or
                          hidden, and its last error when the bar is running
  catalog                 the built-in widgets and catalog scripts you can add
  add NAME [--as KEY] [--left|--right] [--before KEY|--after KEY|--at N]
           [--every DUR] [--command CMD]
                          add a built-in widget (cpu, time, ...) or a catalog
                          script (storage, network, ...; its script is written
                          to the widgets folder, never over an existing file).
                          Your own script: add script --as KEY --command CMD
  remove KEY              delete a widget's settings (script files are kept)
  show KEY [--left|--right] [--before KEY|--after KEY|--at N]
                          put a hidden widget back on the bar
  hide KEY                take a widget off the bar; its settings are kept
  move KEY [--left|--right] [--before KEY|--after KEY|--at N]
                          move a widget within its wing, or to the other one
  set KEY OPTION VALUE    set one option. VALUE is JSON if it reads as JSON
                          (5, true, null, {\"ok\": \"green\"}), else plain text
                          (2s, open -a Calendar); null removes the option

Where a widget goes:
  --left, --right         the wing (default: where it is, or its usual wing)
  --before KEY            just before another shown widget (in that one's wing)
  --after KEY             just after another shown widget
  --at N                  position N in the wing, counting from 1
                          (1 = first; default: the end)

Names: --as KEY picks the new widget's key (a-z, 0-9, _ and -). By default it
is NAME, or NAME-2, NAME-3, ... when that is taken.
Durations (--every): 500ms, 15s, 2m, or a number of milliseconds.

Every command edits the settings file, then asks a running bar to reload it
(--port N: the bar's port; default bar.port in the settings). With no bar
running, the change applies when it starts. Put -- before a value that starts
with --.
";

pub const CONFIG_HELP: &str = "\
Usage: wingbar config COMMAND

  path                    print where the settings file is
  check                   read the settings file and print every warning;
                          exits 1 when it can't be used
  edit                    open it in $EDITOR (in a terminal), else in the
                          default text editor
  import-simplebar [--from PATH] [--dry-run] [--force]
                          make the settings from simple-bar's ~/.simplebarrc
                          (only read, never changed). --from: another file.
                          --dry-run: print the result, write nothing.
                          --force: replace existing settings (the old file is
                          kept as config.json.bak, or .bak-2, ...; a backup
                          is never overwritten)

The settings file is $WINGBAR_CONFIG, else ~/.config/wingbar/config.json.
";

/// What the command line asks for.
#[derive(Debug, PartialEq)]
pub enum Command {
    Run {
        port: Option<u16>,
        no_server: bool,
        quit_after: Option<u64>,
        snapshot: Option<String>,
    },
    Help(&'static str),
    Version,
    /// A request to the running bar (`/status`, `/reload`, ...).
    Client {
        port: Option<u16>,
        path: String,
    },
    Widget {
        port: Option<u16>,
        cmd: WidgetCmd,
    },
    Config {
        port: Option<u16>,
        cmd: ConfigCmd,
    },
}

#[derive(Debug, PartialEq)]
pub enum WidgetCmd {
    List { json: bool },
    Catalog,
    Add(Add),
    Remove(String),
    Show { key: String, wing: Option<Wing>, pos: Position },
    Hide(String),
    Move { key: String, wing: Option<Wing>, pos: Position },
    Set { key: String, option: String, value: String },
}

#[derive(Debug, PartialEq)]
pub enum ConfigCmd {
    Path,
    Check,
    Edit,
    ImportSimplebar { from: Option<PathBuf>, dry_run: bool, force: bool },
}

/// The arguments after the command: flags (with their values) and words.
struct Args {
    words: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

/// Flags that take a value.
const VALUED: &[&str] =
    &["--port", "--quit-after", "--snapshot", "--as", "--before", "--after", "--at", "--every", "--command", "--from"];

fn split(args: &[String]) -> Result<Args, String> {
    let mut out = Args { words: Vec::new(), flags: Vec::new() };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            out.words.extend(args[i + 1..].iter().cloned());
            break;
        }
        if a.starts_with('-') && a.len() > 1 && !a.chars().nth(1).is_some_and(|c| c.is_ascii_digit()) {
            // --flag=value works too
            let (name, inline) = match a.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
                _ => (a.clone(), None),
            };
            if VALUED.contains(&name.as_str()) {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i).cloned().ok_or_else(|| format!("{name} needs a value"))?
                    }
                };
                out.flags.push((name, Some(v)));
            } else {
                if inline.is_some() {
                    return Err(format!("{name} takes no value"));
                }
                out.flags.push((name, None));
            }
        } else {
            out.words.push(a.clone());
        }
        i += 1;
    }
    Ok(out)
}

impl Args {
    fn wants_help(&self) -> bool {
        self.flags.iter().any(|(f, _)| f == "-h" || f == "--help")
    }

    /// The port, from `--port` (checked).
    fn port(&self) -> Result<Option<u16>, String> {
        match self.value("--port") {
            None => Ok(None),
            Some(p) => p
                .parse::<u16>()
                .ok()
                .filter(|p| *p > 0)
                .map(Some)
                .ok_or_else(|| "--port needs a number from 1 to 65535".to_string()),
        }
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(f, _)| f == name).and_then(|(_, v)| v.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(f, _)| f == name)
    }

    /// Every flag is one of `allowed` (plus --port and --help), given once.
    fn only(&self, allowed: &[&str], what: &str) -> Result<(), String> {
        for (i, (f, _)) in self.flags.iter().enumerate() {
            if !allowed.contains(&f.as_str()) && !["--port", "-h", "--help"].contains(&f.as_str()) {
                let group = what.split(' ').next().filter(|g| ["widget", "config"].contains(g));
                let help = group.map_or("wingbar --help".to_string(), |g| format!("wingbar {g} --help"));
                let cmd = if what.is_empty() { "wingbar".to_string() } else { format!("wingbar {what}") };
                return Err(format!("{cmd} has no option {f} (see {help})"));
            }
            if self.flags[..i].iter().any(|(g, _)| g == f) {
                return Err(format!("{f} is given twice"));
            }
        }
        Ok(())
    }

    /// `--left` / `--right`.
    fn wing(&self) -> Result<Option<Wing>, String> {
        match (self.has("--left"), self.has("--right")) {
            (true, true) => Err("use --left or --right, not both".into()),
            (true, false) => Ok(Some(Wing::Left)),
            (false, true) => Ok(Some(Wing::Right)),
            (false, false) => Ok(None),
        }
    }

    /// `--before K` / `--after K` / `--at N` (1-based), else the end.
    fn position(&self) -> Result<Position, String> {
        let given: Vec<&str> = ["--before", "--after", "--at"].into_iter().filter(|f| self.has(f)).collect();
        if given.len() > 1 {
            return Err(format!("use only one of --before, --after and --at (got {})", given.join(" and ")));
        }
        if let Some(k) = self.value("--before") {
            return Ok(Position::Before(k.to_string()));
        }
        if let Some(k) = self.value("--after") {
            return Ok(Position::After(k.to_string()));
        }
        if let Some(n) = self.value("--at") {
            return match n.parse::<usize>() {
                Ok(n) if n >= 1 => Ok(Position::At(n - 1)),
                _ => Err(format!("--at needs a position counting from 1 (1 = first), not {n:?}")),
            };
        }
        Ok(Position::End)
    }

    /// Exactly `n` words after the subcommand, named for the error.
    fn words_exactly(&self, names: &[&str], usage: &str) -> Result<Vec<String>, String> {
        let got = &self.words[2..];
        if got.len() < names.len() {
            return Err(format!("missing {}; usage: {usage}", names[got.len()]));
        }
        if got.len() > names.len() {
            return Err(format!("unexpected {:?}; usage: {usage} (quote a value with spaces)", got[names.len()]));
        }
        Ok(got.to_vec())
    }
}

/// The command the arguments (without the program name) ask for.
pub fn parse(args: &[String]) -> Result<Command, String> {
    let a = split(args)?;
    let port = a.port()?;
    let first = a.words.first().map(String::as_str);
    if a.has("--version") || a.has("-V") || first == Some("version") {
        return Ok(Command::Version);
    }
    match first {
        None => {
            if a.wants_help() {
                return Ok(Command::Help(HELP));
            }
            a.only(&["--no-server", "--quit-after", "--snapshot"], "")?;
            let quit_after = match a.value("--quit-after") {
                None => None,
                Some(s) => Some(s.parse::<u64>().map_err(|_| "--quit-after needs a number of seconds".to_string())?),
            };
            Ok(Command::Run {
                port,
                no_server: a.has("--no-server"),
                quit_after,
                snapshot: a.value("--snapshot").map(String::from),
            })
        }
        Some("help") => Ok(Command::Help(match a.words.get(1).map(String::as_str) {
            Some("widget") => WIDGET_HELP,
            Some("config") => CONFIG_HELP,
            _ => HELP,
        })),
        Some(cmd @ ("status" | "reload" | "quit" | "refresh")) => {
            if a.wants_help() {
                return Ok(Command::Help(HELP));
            }
            a.only(&[], cmd)?;
            let path = match (cmd, a.words.get(1).map(String::as_str)) {
                ("refresh", None | Some("all")) => "/refresh".to_string(),
                ("refresh", Some("spaces")) => "/aerospace/spaces/refresh".to_string(),
                ("refresh", Some(w)) => format!("/widget/{w}/refresh"),
                (_, None) => format!("/{cmd}"),
                (_, Some(w)) => return Err(format!("wingbar {cmd} takes no argument (got {w:?})")),
            };
            if a.words.len() > 2 {
                return Err(format!("unexpected {:?} (try --help)", a.words[2]));
            }
            Ok(Command::Client { port, path })
        }
        Some("widget") => {
            if a.wants_help() || a.words.len() < 2 || a.words[1] == "help" {
                return Ok(Command::Help(WIDGET_HELP));
            }
            Ok(Command::Widget { port, cmd: parse_widget(&a)? })
        }
        Some("config") => {
            if a.wants_help() || a.words.len() < 2 || a.words[1] == "help" {
                return Ok(Command::Help(CONFIG_HELP));
            }
            Ok(Command::Config { port, cmd: parse_config(&a)? })
        }
        Some(other) => Err(format!("unknown command {other:?} (try wingbar --help)")),
    }
}

const PLACE: &[&str] = &["--left", "--right", "--before", "--after", "--at"];

fn parse_widget(a: &Args) -> Result<WidgetCmd, String> {
    let sub = a.words[1].as_str();
    let place = |usage: &str| -> Result<(String, Option<Wing>, Position), String> {
        a.only(PLACE, &format!("widget {sub}"))?;
        let w = a.words_exactly(&["KEY"], usage)?;
        Ok((w[0].clone(), a.wing()?, a.position()?))
    };
    Ok(match sub {
        "list" => {
            a.only(&["--json"], "widget list")?;
            a.words_exactly(&[], "wingbar widget list [--json]")?;
            WidgetCmd::List { json: a.has("--json") }
        }
        "catalog" => {
            a.only(&[], "widget catalog")?;
            a.words_exactly(&[], "wingbar widget catalog")?;
            WidgetCmd::Catalog
        }
        "add" => {
            let mut allowed = PLACE.to_vec();
            allowed.extend(["--as", "--every", "--command"]);
            a.only(&allowed, "widget add")?;
            let w = a.words_exactly(&["NAME (a built-in type or catalog script; see wingbar widget catalog)"], "wingbar widget add NAME [--as KEY] [--left|--right] [--before K|--after K|--at N] [--every DUR] [--command CMD]")?;
            WidgetCmd::Add(Add {
                what: w[0].clone(),
                key: a.value("--as").map(String::from),
                wing: a.wing()?,
                pos: a.position()?,
                every: a.value("--every").map(String::from),
                command: a.value("--command").map(String::from),
            })
        }
        "remove" | "rm" => {
            a.only(&[], "widget remove")?;
            WidgetCmd::Remove(a.words_exactly(&["KEY"], "wingbar widget remove KEY")?.remove(0))
        }
        "hide" => {
            a.only(&[], "widget hide")?;
            WidgetCmd::Hide(a.words_exactly(&["KEY"], "wingbar widget hide KEY")?.remove(0))
        }
        "show" => {
            let (key, wing, pos) = place("wingbar widget show KEY [--left|--right] [--before K|--after K|--at N]")?;
            WidgetCmd::Show { key, wing, pos }
        }
        "move" => {
            let (key, wing, pos) = place("wingbar widget move KEY [--left|--right] [--before K|--after K|--at N]")?;
            if wing.is_none() && pos == Position::End {
                return Err("say where to: --left, --right, --before KEY, --after KEY or --at N".into());
            }
            WidgetCmd::Move { key, wing, pos }
        }
        "set" => {
            a.only(&[], "widget set")
                .map_err(|e| format!("{e}; quote a value with spaces, and put -- before a value that starts with -"))?;
            let w = a.words_exactly(&["KEY", "OPTION", "VALUE"], "wingbar widget set KEY OPTION VALUE")?;
            WidgetCmd::Set { key: w[0].clone(), option: w[1].clone(), value: w[2].clone() }
        }
        other => {
            return Err(format!(
                "unknown widget command {other:?}; use list, catalog, add, remove, show, hide, move or set"
            ));
        }
    })
}

fn parse_config(a: &Args) -> Result<ConfigCmd, String> {
    let sub = a.words[1].as_str();
    Ok(match sub {
        "path" | "check" | "edit" => {
            a.only(&[], &format!("config {sub}"))?;
            a.words_exactly(&[], &format!("wingbar config {sub}"))?;
            match sub {
                "path" => ConfigCmd::Path,
                "check" => ConfigCmd::Check,
                _ => ConfigCmd::Edit,
            }
        }
        "import-simplebar" => {
            a.only(&["--from", "--dry-run", "--force"], "config import-simplebar")?;
            a.words_exactly(&[], "wingbar config import-simplebar [--from PATH] [--dry-run] [--force]")?;
            ConfigCmd::ImportSimplebar {
                from: a.value("--from").map(|p| PathBuf::from(config::expand(p))),
                dry_run: a.has("--dry-run"),
                force: a.has("--force"),
            }
        }
        other => return Err(format!("unknown config command {other:?}; use path, check, edit or import-simplebar")),
    })
}

// --- talking to the running bar ------------------------------------------------

/// `GET path` from the bar on `port`: the status code and the body. `Err`
/// when nothing answers.
pub fn request(port: u16, path: &str, timeout: Duration) -> Result<(u16, String), String> {
    let mut s = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500))
        .map_err(|e| format!("wingbar isn't running on port {port} ({e})"))?;
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.write_all(format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let code = out.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let body = out.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("").to_string();
    Ok((code, body))
}

/// The settings file the bar's `/reload` answer says it read.
fn reloaded_path(body: &str) -> Option<&str> {
    body.lines().next()?.strip_prefix("reloaded ")
}

/// Which settings file the bar on `port` uses, as it reports on `/widgets`.
enum RunningBar {
    None,
    /// A wingbar before 0.3 (or something else) answers there: it has no
    /// `/widgets`, and a `/reload` would restart it on its own settings.
    Other,
    Uses(String),
}

fn running_bar(port: u16) -> RunningBar {
    match request(port, "/widgets", Duration::from_secs(2)) {
        Err(_) => RunningBar::None,
        Ok((200, body)) => serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("settings")?.as_str().map(String::from))
            .map_or(RunningBar::Other, RunningBar::Uses),
        Ok(_) => RunningBar::Other,
    }
}

/// After an edit: ask the bar to reload, but only a bar that says it uses this
/// settings file; say what came of it. Exit code.
fn reload_bar(port: u16, settings: &Path) -> i32 {
    match running_bar(port) {
        RunningBar::None => {
            println!("wingbar isn't running (port {port}); the change applies when it starts.");
            return 0;
        }
        RunningBar::Other => {
            println!(
                "note: an older wingbar is running on port {port}; it doesn't read {}. \
                 The change applies when this version starts.",
                settings.display()
            );
            return 0;
        }
        RunningBar::Uses(p) if Path::new(&p) != settings => {
            println!("note: the bar on port {port} uses {p}, not {}; it won't show this change", settings.display());
            return 0;
        }
        RunningBar::Uses(_) => {}
    }
    match request(port, "/reload", Duration::from_secs(5)) {
        Err(_) => {
            println!("wingbar isn't running (port {port}); the change applies when it starts.");
            0
        }
        Ok((200, body)) => {
            if let Some(p) = reloaded_path(&body)
                && Path::new(p) != settings
            {
                println!(
                    "note: the bar on port {port} uses {p}, not {}; it won't show this change",
                    settings.display()
                );
            }
            for l in body.lines().skip(1) {
                println!("bar: {l}");
            }
            if body.lines().count() <= 1 {
                println!("bar: reloaded");
            }
            0
        }
        Ok((_, body)) => {
            eprint!("wingbar: the bar on port {port} couldn't use the settings: {body}");
            1
        }
    }
}

/// The running bar's last error per widget (empty when it isn't running, or
/// reads another settings file).
fn bar_errors(port: u16, settings: &Path) -> Option<HashMap<String, String>> {
    let widgets = bar_widgets(port, settings)?;
    Some(widgets.iter().filter_map(|(k, w)| Some((k.clone(), w.get("error")?.as_str()?.to_string()))).collect())
}

/// The running bar's `/widgets` entries by key, when it reads `settings`.
fn bar_widgets(port: u16, settings: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    let (code, body) = request(port, "/widgets", Duration::from_secs(2)).ok()?;
    if code != 200 {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    if v.get("settings").and_then(|s| s.as_str()).is_some_and(|p| Path::new(p) != settings) {
        return None;
    }
    match v.get("widgets")? {
        serde_json::Value::Object(m) => Some(m.clone()),
        _ => None,
    }
}

/// The keys on the bar, as the settings file has them.
fn shown_keys(f: &Files) -> Vec<String> {
    manage::list(f, &HashMap::new())
        .map(|rows| rows.into_iter().filter(|r| r.wing.is_some()).map(|r| r.key).collect())
        .unwrap_or_default()
}

/// How long a widget just put on the bar gets to show something before the
/// CLI says it shows nothing (as the bar's menu does).
const FIRST_RUN: Duration = Duration::from_secs(2);

/// After an edit the bar reloaded: a widget it put on the bar that still
/// shows nothing after its first run takes no room, so say why it can't be
/// seen. `before`: the keys shown before the edit.
fn explain_empty(port: u16, f: &Files, before: &[String]) {
    let Ok(rows) = manage::list(f, &HashMap::new()) else { return };
    let new: Vec<_> = rows.iter().filter(|r| r.wing.is_some() && !before.contains(&r.key)).collect();
    if new.is_empty() {
        return;
    }
    let deadline = std::time::Instant::now() + FIRST_RUN;
    loop {
        // `showing` is missing from a bar that doesn't report it: say nothing
        let Some(widgets) = bar_widgets(port, &f.settings) else { return };
        let empty = |key: &str| widgets.get(key).and_then(|w| w.get("showing")?.as_bool()) == Some(false);
        let pending: Vec<_> = new.iter().filter(|r| empty(&r.key)).collect();
        if pending.is_empty() || std::time::Instant::now() >= deadline {
            for r in pending {
                let (Some(ty), Some(wing)) = (config::WidgetType::parse(&r.ty), r.wing) else { continue };
                let error = widgets.get(&r.key).and_then(|w| w.get("error")?.as_str());
                let (title, text) = manage::not_showing(&r.key, ty, wing, error);
                println!("note: {title}. {text}");
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// --- running the commands ------------------------------------------------------

/// What a `widget` command does about the settings file first.
#[derive(Debug, PartialEq)]
enum Before {
    /// Use the file as it is.
    AsItIs,
    /// The command only reads: show what the file will be after the bar's
    /// first start, worked out in memory; nothing is written.
    Preview(config::Pending),
    /// Write the version 2 file first, as the bar's first start does.
    Create,
    /// Do nothing; fail with this message.
    Refuse(String),
}

/// Pure: what to do before a `widget` command that `edits` (or only reads)
/// the settings, which are `pending` (missing or a 0.2 file) or not. `bar`
/// asks the port what runs there; it is only asked when that matters (an
/// older wingbar uses the file as it is, so it must not be converted under it).
fn before_widget_command(
    edits: bool,
    pending: Option<config::Pending>,
    bar: impl FnOnce() -> RunningBar,
    port: u16,
) -> Before {
    match pending {
        None => Before::AsItIs,
        Some(p) if !edits => Before::Preview(p),
        Some(_) => match bar() {
            RunningBar::Other => Before::Refuse(format!(
                "an older wingbar is running on port {port} and uses these settings; \
                 install this version first (./install.sh), then run the command again"
            )),
            RunningBar::None | RunningBar::Uses(_) => Before::Create,
        },
    }
}

/// Run a `widget` command; returns the exit code.
pub fn widget(port: Option<u16>, cmd: WidgetCmd) -> i32 {
    if let Err(e) = config::usable_settings_path() {
        return fail(&e);
    }
    let f = Files::current();
    let port = || port.unwrap_or_else(config::client_port);
    let edits = !matches!(cmd, WidgetCmd::List { .. } | WidgetCmd::Catalog);
    // On a fresh install the settings don't exist until the bar first starts;
    // an edit creates them here the same way (importing simple-bar's when
    // present), so `wingbar widget add ...` works as the very first command.
    let preview = match before_widget_command(edits, config::pending(&f.settings), || running_bar(port()), port()) {
        Before::AsItIs => None,
        Before::Create => {
            for m in config::ensure_settings(&f.settings) {
                eprintln!("wingbar: {m}");
            }
            None
        }
        Before::Preview(p) => {
            eprintln!("wingbar: {}", preview_note(&f.settings, p));
            Some(config::preview(&f.settings))
        }
        Before::Refuse(e) => return fail(&e),
    };
    let puts_on_bar = matches!(cmd, WidgetCmd::Add(_) | WidgetCmd::Show { .. });
    let before = if puts_on_bar { shown_keys(&f) } else { Vec::new() };
    let done = match cmd {
        WidgetCmd::List { json } => {
            let errors = bar_errors(port(), &f.settings);
            let rows = match &preview {
                Some(doc) => Ok(manage::rows(doc, errors.as_ref().unwrap_or(&HashMap::new()))),
                None => manage::list(&f, errors.as_ref().unwrap_or(&HashMap::new())),
            };
            return match rows {
                Ok(rows) if json => {
                    println!("{}", serde_json::to_string_pretty(&manage::rows_json(&rows)).unwrap_or_default());
                    0
                }
                Ok(rows) => {
                    print!("{}", manage::rows_text(&rows));
                    if errors.is_none() {
                        println!("(the bar isn't running on port {}, so no errors are shown)", port());
                    }
                    0
                }
                Err(e) => fail(&e),
            };
        }
        WidgetCmd::Catalog => {
            let offers = match &preview {
                Some(doc) => Ok(manage::offers(doc)),
                None => manage::catalog(&f),
            };
            return match offers {
                Ok(o) => {
                    print!("{}", manage::offers_text(&o, &f.widgets_dir));
                    0
                }
                Err(e) => fail(&e),
            };
        }
        WidgetCmd::Add(req) => manage::add(&f, &req),
        WidgetCmd::Remove(k) => manage::remove(&f, &k),
        WidgetCmd::Show { key, wing, pos } => manage::show(&f, &key, wing, &pos),
        WidgetCmd::Hide(k) => manage::hide(&f, &k),
        WidgetCmd::Move { key, wing, pos } => manage::move_to(&f, &key, wing, &pos),
        WidgetCmd::Set { key, option, value } => manage::set(&f, &key, &option, &value),
    };
    match done {
        Ok(m) => {
            println!("{m}");
            let code = reload_bar(port(), &f.settings);
            if code == 0 && puts_on_bar {
                explain_empty(port(), &f, &before);
            }
            code
        }
        Err(e) => fail(&e),
    }
}

/// Said before a preview (see `Before::Preview`).
fn preview_note(settings: &Path, p: config::Pending) -> String {
    match p {
        config::Pending::Missing => format!(
            "there is no settings file at {} yet; this is what wingbar will write there when it first starts \
             (nothing was written)",
            settings.display()
        ),
        config::Pending::V1 => format!(
            "{} is a wingbar 0.2 file; this is what wingbar will convert it to when it first starts \
             (nothing was written)",
            settings.display()
        ),
    }
}

fn fail(e: &str) -> i32 {
    eprintln!("wingbar: {e}");
    1
}

/// Run a `config` command; returns the exit code.
pub fn config_cmd(port: Option<u16>, cmd: ConfigCmd) -> i32 {
    let path = config::settings_path();
    if cmd != ConfigCmd::Path
        && let Err(e) = config::usable_settings_path()
    {
        return fail(&e);
    }
    match cmd {
        ConfigCmd::Path => {
            println!("{}", path.display());
            0
        }
        ConfigCmd::Check => {
            let (code, out) = check(&path);
            print!("{out}");
            code
        }
        ConfigCmd::Edit => edit(&path),
        ConfigCmd::ImportSimplebar { from, dry_run, force } => {
            let from = from.unwrap_or_else(config::simplebarrc);
            // Same guard as the widget commands: don't write version 2 settings
            // under an older wingbar that still reads this file.
            let port_n = port.unwrap_or_else(config::client_port);
            if !dry_run && matches!(running_bar(port_n), RunningBar::Other) {
                return fail(&format!(
                    "an older wingbar is running on port {port_n} and uses these settings; \
                     install this version first (./install.sh), then run the command again"
                ));
            }
            match import(&path, &from, dry_run, force) {
                Ok(out) => {
                    print!("{out}");
                    if dry_run { 0 } else { reload_bar(port.unwrap_or_else(config::client_port), &path) }
                }
                Err(e) => fail(&e),
            }
        }
    }
}

/// `config check`: the exit code and what to print.
pub fn check(path: &Path) -> (i32, String) {
    let shown = path.display();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (
                1,
                format!(
                    "{shown} doesn't exist yet; wingbar writes it when it starts (or run `wingbar config import-simplebar`)\n"
                ),
            );
        }
        Err(e) => return (1, format!("can't read {shown}: {e}\n")),
    };
    let doc = match config::doc::parse_text(&text) {
        Ok(d) => d,
        Err(e) => return (1, format!("{shown}: {e}\n")),
    };
    if !doc.is_object() {
        return (1, format!("{shown}: the settings must be a JSON object ({{ \"version\": 2, ... }})\n"));
    }
    if config::is_v1(&doc) {
        return (
            0,
            format!(
                "{shown} is a wingbar 0.2 file; the bar converts it when it starts (keeping it as config.v1.json)\n"
            ),
        );
    }
    let (cfg, warnings) = config::parse::parse(&doc, &|p: &Path| p.exists());
    let mut out = String::new();
    for w in &warnings {
        out.push_str(&format!("warning: {w}\n"));
    }
    let shown_n = cfg.left.len() + cfg.right.len();
    out.push_str(&format!(
        "{shown}: ok, {} widgets ({shown_n} shown, {} hidden){}\n",
        cfg.widgets.len(),
        cfg.widgets.len().saturating_sub(shown_n),
        match warnings.len() {
            0 => String::new(),
            1 => ", 1 warning".into(),
            n => format!(", {n} warnings"),
        }
    ));
    (0, out)
}

/// `config edit`: $EDITOR in a terminal, else the default text editor.
fn edit(path: &Path) -> i32 {
    if !path.exists() {
        return fail(&format!(
            "{} doesn't exist yet; start wingbar once (it writes one), or run `wingbar config import-simplebar`",
            path.display()
        ));
    }
    let editor = std::env::var("EDITOR").ok().filter(|e| !e.trim().is_empty());
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let status = match editor {
        // the editor may have arguments ("code -w"): through the shell, the path as $1
        Some(ed) if tty => {
            std::process::Command::new("/bin/sh").args(["-c", &format!("{ed} \"$1\""), "sh"]).arg(path).status()
        }
        _ => std::process::Command::new("/usr/bin/open").arg("-t").arg(path).status(),
    };
    match status {
        Ok(s) if s.success() => 0,
        Ok(s) => fail(&format!("the editor exited with {s}")),
        Err(e) => fail(&format!("can't open an editor: {e}")),
    }
}

/// `config import-simplebar`: what to print. Writes nothing with `dry_run`;
/// never replaces version 2 settings without `force` (then keeps them as
/// `config.json.bak`, or `config.json.bak-2`, ... : a backup is never
/// overwritten). A wingbar 0.2 file there is merged in and kept. Refuses, and
/// writes nothing, when `from` is not a JSON object.
pub fn import(path: &Path, from: &Path, dry_run: bool, force: bool) -> Result<String, String> {
    if !from.exists() {
        return Err(format!("{} doesn't exist; name simple-bar's settings file with --from PATH", from.display()));
    }
    // the file imported from is only read
    config::check_not_simplebars(path, from)?;
    let source = std::fs::read_to_string(from).map_err(|e| format!("can't read {}: {e}", from.display()))?;
    match config::doc::parse_text(&source) {
        Err(e) => return Err(format!("{}: {e}; nothing was written", from.display())),
        Ok(v) if !v.is_object() => {
            return Err(format!(
                "{} is not a JSON object, so it is no simple-bar settings file; nothing was written",
                from.display()
            ));
        }
        Ok(_) => {}
    }
    let existing = match std::fs::read_to_string(path) {
        Ok(t) => Some(
            config::doc::parse_text(&t)
                .map_err(|e| format!("{}: {e}; fix it, or move it away and import again", path.display()))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("can't read {}: {e}", path.display())),
    };
    let v1 = existing.as_ref().filter(|v| config::is_v1(v));
    if existing.is_some() && v1.is_none() && !force && !dry_run {
        return Err(format!(
            "{} already has wingbar settings; add --force to replace them (they are kept as {})",
            path.display(),
            backup_path(path, &|p: &Path| p.exists()).display()
        ));
    }
    // `from` is the file to import: a 0.2 file's own `simplebarrc` doesn't override it
    let v1_for_import = v1.map(|v| {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("simplebarrc");
        }
        v
    });
    let (doc, notes) = config::import::import_files(v1_for_import.as_ref(), from);
    let mut out = String::new();
    if dry_run {
        out.push_str(&config::doc::to_text(&doc));
        for n in &notes {
            eprintln!("import: {n}");
        }
        eprintln!("(dry run: nothing was written)");
        return Ok(out);
    }
    if existing.is_some() {
        let taken = |p: &Path| p.symlink_metadata().is_ok();
        let kept = if v1.is_some() { config::v1_backup_path(path, &taken) } else { backup_path(path, &taken) };
        keep_copy(path, &kept)
            .map_err(|e| format!("can't keep a copy at {}: {e}; nothing was changed", kept.display()))?;
        out.push_str(&format!("kept the old settings as {}\n", kept.display()));
    }
    config::doc::write_atomic(path, &config::doc::to_text(&doc))
        .map_err(|e| format!("can't write {}: {e}", path.display()))?;
    out.push_str(&format!("imported {} into {} ({})\n", from.display(), path.display(), notes[0]));
    for n in notes.iter().skip(1) {
        out.push_str(&format!("import: {n}\n"));
    }
    Ok(out)
}

/// `config.json.bak` next to the settings, or `config.json.bak-2`, ... when
/// `taken` says that one is.
fn backup_path(path: &Path, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "config.json".into());
    (1..)
        .map(|n| path.with_file_name(if n == 1 { format!("{name}.bak") } else { format!("{name}.bak-{n}") }))
        .find(|p| !taken(p))
        .expect("a free name")
}

/// Copy `from` to `to`, which must not exist yet (never overwritten).
fn keep_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    let bytes = std::fs::read(from)?;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    f.write_all(&bytes)?;
    f.sync_all()
}

#[cfg(test)]
mod tests;
