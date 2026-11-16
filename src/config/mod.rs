//! Settings: wingbar's own file, `$WINGBAR_CONFIG` or
//! `~/.config/wingbar/config.json` (format version 2, see `config.example.json`
//! and docs/design/widgets-v2.md).
//!
//! - `model`: what the settings mean (`Config`, `WidgetDef`, `Theme`, ...)
//! - `parse`: reading a version 2 document leniently, with warnings
//! - `doc`: the file as an editable JSON document, written back atomically
//! - `edit`: add / remove / show / hide / move / set on that document
//! - `import`: simple-bar's `.simplebarrc` and 0.2's config.json → version 2
//! - `defaults`: a fresh install's settings
//!
//! At startup a missing file is imported from `~/.simplebarrc` (or written
//! with the defaults); a 0.2 file (no `"version"`) is imported and merged, and
//! kept as `config.v1.json`. The settings in use are swappable (`current`,
//! `set_current`), so they can be reloaded without a restart.

pub mod defaults;
pub mod doc;
pub mod edit;
pub mod import;
mod model;
pub mod parse;

pub use model::*;

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// `~`, `~/x`, `$HOME` and `${HOME}` → the home folder.
pub fn expand(p: &str) -> String {
    let home = home().display().to_string();
    if p == "~" {
        home
    } else if let Some(rest) = p.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        p.replace("${HOME}", &home).replace("$HOME", &home)
    }
}

/// `expand`, and `$TMPDIR` → the per-user temp folder (`DARWIN_USER_TEMP_DIR`).
pub fn expand_path(p: &str) -> String {
    let p = expand(p);
    if p.contains("$TMPDIR") {
        let tmp = crate::sys::user_temp_dir();
        p.replace("$TMPDIR/", &tmp).replace("$TMPDIR", tmp.trim_end_matches('/'))
    } else {
        p
    }
}

/// Where the settings live: `$WINGBAR_CONFIG`, else `~/.config/wingbar/config.json`.
pub fn settings_path() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    settings_path_from(std::env::var_os("WINGBAR_CONFIG"), &cwd)
}

/// Simple-bar's settings file, which wingbar imports once and only ever reads.
pub fn simplebarrc() -> PathBuf {
    home().join(doc::SIMPLEBARRC)
}

/// `settings_path`, unless that is simple-bar's file (for the bar's start and
/// every command that uses the settings): then why it can't be used.
pub fn usable_settings_path() -> Result<PathBuf, String> {
    let path = settings_path();
    check_not_simplebars(&path, &simplebarrc())?;
    Ok(path)
}

/// Pure but for following links: `path` must not be simple-bar's settings
/// (a file named `.simplebarrc`, or `simplebarrc` itself), which wingbar
/// would write, rename or migrate as its own.
pub fn check_not_simplebars(path: &Path, simplebarrc: &Path) -> Result<(), String> {
    let same = |a: &Path, b: &Path| {
        a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
    };
    let named = path.file_name().is_some_and(|n| n == doc::SIMPLEBARRC)
        || std::fs::canonicalize(path).is_ok_and(|p| p.file_name().is_some_and(|n| n == doc::SIMPLEBARRC));
    if named || same(path, simplebarrc) {
        return Err(format!(
            "WINGBAR_CONFIG points at simple-bar's settings file ({}), which wingbar never writes; \
             set it to a file of wingbar's own (or unset it for ~/.config/wingbar/config.json), \
             and simple-bar's settings are imported from it",
            path.display()
        ));
    }
    Ok(())
}

/// `settings_path` for this `WINGBAR_CONFIG`, from `cwd`: always absolute
/// (a relative one is taken from `cwd`, with `.` and `..` worked out), so a
/// bar and a command started in different folders name the file the same way.
fn settings_path_from(var: Option<std::ffi::OsString>, cwd: &Path) -> PathBuf {
    use std::path::Component;
    let p = match var.filter(|p| !p.is_empty()) {
        Some(p) => cwd.join(expand(&p.to_string_lossy())),
        None => home().join(".config/wingbar/config.json"),
    };
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

fn exists(p: &Path) -> bool {
    p.exists()
}

static CURRENT: RwLock<Option<Arc<Config>>> = RwLock::new(None);

/// The settings in use. (All defaults before `load`.)
pub fn current() -> Arc<Config> {
    if let Some(c) = CURRENT.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
        return c.clone();
    }
    Arc::new(parse::parse(&Value::Null, &|_: &Path| false).0)
}

/// Replace the settings in use.
pub fn set_current(cfg: Config) -> Arc<Config> {
    let cfg = Arc::new(cfg);
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(cfg.clone());
    cfg
}

/// What the last reading of the settings file came to, for `wingbar status`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadStatus {
    pub path: PathBuf,
    /// Why the file couldn't be used (the bar runs on the settings it had).
    pub error: Option<String>,
    /// What `parse` warned about in the settings in use.
    pub warnings: Vec<String>,
    /// Changes that only a restart applies (`bar.port`, `bar.server`).
    pub restart: Vec<String>,
}

static STATUS: Mutex<Option<LoadStatus>> = Mutex::new(None);

pub fn status() -> LoadStatus {
    STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| LoadStatus { path: settings_path(), ..LoadStatus::default() })
}

pub fn set_status(s: LoadStatus) {
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
}

/// `--port` and `--no-server`, which hold over every reload.
static OVERRIDES: OnceLock<(Option<u16>, bool)> = OnceLock::new();

fn apply_overrides(cfg: &mut Config) {
    let (port, no_server) = OVERRIDES.get().copied().unwrap_or((None, false));
    if let Some(port) = port {
        cfg.bar.port = port;
    }
    cfg.bar.server &= !no_server;
}

/// The settings file read again, for a reload (nothing is imported or
/// written): what `text`, read from `path`, means, with `--port` /
/// `--no-server` applied, and what `parse` warned about; or why it can't be used.
pub fn from_text(
    path: &Path,
    text: std::io::Result<String>,
    probe: &dyn Fn(&Path) -> bool,
) -> Result<(Config, Vec<String>), String> {
    let shown = path.display();
    let text = text.map_err(|e| format!("can't read {shown}: {e}"))?;
    let doc = doc::parse_text(&text).map_err(|e| format!("{shown}: {e}"))?;
    if !doc.is_object() {
        return Err(format!("{shown}: the settings must be a JSON object ({{ \"version\": 2, ... }})"));
    }
    let (mut cfg, warnings) = parse::parse(&doc, probe);
    apply_overrides(&mut cfg);
    Ok((cfg, warnings))
}

/// What only a restart applies: where `cfg` differs from the `running`
/// settings in those, it is set back to them, and the change is described.
pub fn keep_restart_only(cfg: &mut Config, running: &Config) -> Vec<String> {
    let mut notes = Vec::new();
    if cfg.bar.port != running.bar.port {
        notes.push(format!("bar.port is now {} (the bar still listens on {})", cfg.bar.port, running.bar.port));
        cfg.bar.port = running.bar.port;
    }
    if cfg.bar.server != running.bar.server {
        let state = |on: bool| if on { "on" } else { "off" };
        notes.push(format!("bar.server is now {} (still {})", state(cfg.bar.server), state(running.bar.server)));
        cfg.bar.server = running.bar.server;
    }
    notes
}

/// Keys that only a version 2 file has (0.2's config.json had none of them).
const V2_ONLY_KEYS: &[&str] = &["widgets", "theme", "bar", "windowManager"];

/// Whether `v` is a wingbar 0.2 config.json, which is imported (and kept as
/// `config.v1.json`) rather than read: an object with no `"version"` and none
/// of the keys only version 2 has. (A version 2 file someone wrote without
/// `"version"` is read as version 2, with a warning.) The one test for that,
/// everywhere.
pub fn is_v1(v: &Value) -> bool {
    v.as_object().is_some_and(|o| !o.contains_key("version") && !V2_ONLY_KEYS.iter().any(|k| o.contains_key(*k)))
}

/// What reading the settings file at startup came to.
struct Prepared {
    doc: Value,
    /// What happened, for stderr: what was imported or written, and problems.
    messages: Vec<String>,
    /// The file exists but couldn't be used (the defaults are used instead).
    error: Option<String>,
}

/// Filesystem: read the settings at `path`, first importing or writing them
/// when needed. `default_sb` is simple-bar's file when a 0.2 config.json
/// doesn't name another. Never writes to `.simplebarrc`.
fn prepare(path: &Path, default_sb: &Path, probe: &dyn Fn(&Path) -> bool) -> Prepared {
    let shown = path.display();
    let mut messages = Vec::new();
    let write = |doc: &Value, messages: &mut Vec<String>| {
        if let Err(e) = doc::write_atomic(path, &doc::to_text(doc)) {
            messages.push(format!("can't write {shown}: {e}; using these settings without saving them"));
        }
    };
    let import_notes = |notes: Vec<String>, messages: &mut Vec<String>| {
        messages.extend(notes.into_iter().skip(1).map(|n| format!("import: {n}")));
    };
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if probe(default_sb) {
                let (doc, notes) = import::import_files(None, default_sb);
                messages.push(format!("imported {} into {shown} ({})", default_sb.display(), notes[0]));
                write(&doc, &mut messages);
                import_notes(notes, &mut messages);
                Prepared { doc, messages, error: None }
            } else {
                let doc = defaults::fresh(parse::detect_wm(probe) != Wm::None);
                messages.push(format!("wrote the default settings to {shown}"));
                write(&doc, &mut messages);
                Prepared { doc, messages, error: None }
            }
        }
        Err(e) => {
            messages.push(format!("can't read {shown} ({e}); using the default settings"));
            let error = Some(format!("can't read {shown}: {e}"));
            Prepared { doc: defaults::fresh(parse::detect_wm(probe) != Wm::None), messages, error }
        }
        Ok(text) => match doc::parse_text(&text) {
            Err(e) => {
                messages.push(format!("{shown}: {e}; using the default settings until then"));
                let error = Some(format!("{shown}: {e}"));
                Prepared { doc: defaults::fresh(parse::detect_wm(probe) != Wm::None), messages, error }
            }
            Ok(v1) if is_v1(&v1) => {
                let (doc, notes) = import::import_files(Some(&v1), default_sb);
                let sb = import::simplebarrc_path(Some(&v1), default_sb);
                let kept = v1_backup_path(path, probe);
                match std::fs::rename(path, &kept) {
                    Ok(()) => {
                        messages.push(format!(
                            "{shown} was a wingbar 0.2 file: merged it with {} into version 2 settings ({}); the old file is now {}",
                            sb.display(),
                            notes[0],
                            kept.display()
                        ));
                        write(&doc, &mut messages);
                    }
                    Err(e) => messages.push(format!(
                        "{shown} is a wingbar 0.2 file, but it can't be moved to {} ({e}); using it converted, without saving",
                        kept.display()
                    )),
                }
                import_notes(notes, &mut messages);
                Prepared { doc, messages, error: None }
            }
            Ok(doc) => Prepared { doc, messages, error: None },
        },
    }
}

/// `config.v1.json` next to the settings, or `config.v1-2.json`, ... if taken.
pub(crate) fn v1_backup_path(path: &Path, probe: &dyn Fn(&Path) -> bool) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "config".into());
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}.v1.json") } else { format!("{stem}.v1-{n}.json") };
        let p = dir.join(name);
        if !probe(&p) {
            return p;
        }
        n += 1;
    }
}

/// Read (importing or writing first when needed) and use the settings.
/// Messages and warnings go to stderr. `port_override` and `no_server` (the
/// command line's) hold for the life of the process, over every reload.
pub fn load(port_override: Option<u16>, no_server: bool) -> Arc<Config> {
    let _ = OVERRIDES.set((port_override, no_server));
    let path = settings_path();
    let p = prepare(&path, &simplebarrc(), &exists);
    for m in &p.messages {
        eprintln!("wingbar: {m}");
    }
    let (mut cfg, warnings) = parse::parse(&p.doc, &exists);
    for w in &warnings {
        eprintln!("wingbar: {}: {w}", path.display());
    }
    apply_overrides(&mut cfg);
    set_status(LoadStatus { path, error: p.error, warnings, restart: Vec::new() });
    set_current(cfg)
}

/// Settings the bar's first start still has to write.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pending {
    /// There is no settings file.
    Missing,
    /// It is a wingbar 0.2 file (`is_v1`).
    V1,
}

/// Whether the settings at `path` are still to be written (missing, or a 0.2
/// file); `None` for a version 2 file, or one that can't be read or parsed.
pub fn pending(path: &Path) -> Option<Pending> {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Pending::Missing),
        Err(_) => None,
        Ok(text) => doc::parse_text(&text).is_ok_and(|v| is_v1(&v)).then_some(Pending::V1),
    }
}

/// For commands that edit the settings: when `path` is missing, or is a wingbar
/// 0.2 file, create the version 2 file exactly as the bar's first start does
/// (importing `~/.simplebarrc` when it exists). Returns what was done; an
/// existing version 2 file (or one that can't be read) is left alone.
pub fn ensure_settings(path: &Path) -> Vec<String> {
    if pending(path).is_none() {
        return Vec::new();
    }
    prepare(path, &simplebarrc(), &exists).messages
}

/// What the bar's first start would write at `path` (see `pending`), worked
/// out without writing or moving anything: for commands that only read.
pub fn preview(path: &Path) -> Value {
    preview_with(path, &simplebarrc(), &exists)
}

fn preview_with(path: &Path, default_sb: &Path, probe: &dyn Fn(&Path) -> bool) -> Value {
    let v1 = std::fs::read_to_string(path).ok().and_then(|t| doc::parse_text(&t).ok()).filter(is_v1);
    match v1 {
        Some(v1) => import::import_files(Some(&v1), default_sb).0,
        None if probe(default_sb) => import::import_files(None, default_sb).0,
        None => defaults::fresh(parse::detect_wm(probe) != Wm::None),
    }
}

/// The port of a running bar, for `wingbar status` and friends: read from the
/// settings without importing or writing anything.
pub fn client_port() -> u16 {
    let v = std::fs::read_to_string(settings_path()).ok().and_then(|t| doc::parse_text(&t).ok());
    let port = match &v {
        Some(v) if !is_v1(v) => v.get("bar").and_then(|b| b.get("port")),
        Some(v) => v.get("port"),
        None => None,
    };
    port.and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).filter(|p| *p > 0).unwrap_or(parse::DEFAULT_PORT)
}

/// The config the tests share: `tests/fixtures/simplebarrc.json` and a small
/// 0.2 config.json, imported. Touches no real file and nothing in the home folder.
#[cfg(test)]
pub(crate) fn fixture() -> Config {
    fixture_with(serde_json::json!({}))
}

/// `fixture`, with `extra` merged over the fixture's `.simplebarrc` (top-level keys replaced).
#[cfg(test)]
pub(crate) fn fixture_with(extra: Value) -> Config {
    let mut sb: Value = serde_json::from_str(include_str!("../../tests/fixtures/simplebarrc.json")).unwrap();
    if let (Some(sb), Some(extra)) = (sb.as_object_mut(), extra.as_object()) {
        sb.extend(extra.clone());
    }
    let v1 = serde_json::json!({
        "left": ["spaces", "process", "clickup"],
        "right": ["date-display", "battery", "sound", "music", "time", "user:3"],
        "colors": { "wing": "#111111" },
        "popovers": ["clickup", "salat"],
        "windowLevel": "desktop",
    });
    let no_fs = |_: &Path| false;
    let (doc, _) = import::import_simplebar(&sb, Some(&v1), &no_fs);
    let (cfg, warnings) = parse::parse(&doc, &no_fs);
    assert!(warnings.is_empty(), "{warnings:?}");
    cfg
}

#[cfg(test)]
mod tests;
