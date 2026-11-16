//! The widget catalog: ready-made script widgets anyone can add (see
//! docs/design/widgets-v2.md, "Catalog"). The scripts live in `catalog/` and
//! are built into the binary; adding one writes it to the widgets folder
//! (never over an existing file) and puts its definition in the settings.
//!
//! Each script is plain bash for macOS's /bin/bash 3.2, needs no secrets,
//! follows the script widget output protocol (`level|label`, `icon:` and
//! `menu:` lines) and says in its header what it shows and needs.

use crate::config::Wing;
use crate::config::import::shell_quote;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Where a definition's command looks for the script, unless the script was
/// installed somewhere else (`Entry::definition_for`).
#[cfg_attr(not(test), allow(dead_code))] // the tests' reference definitions
pub const DEFAULT_DIR: &str = "~/.config/wingbar/widgets";

/// One catalog widget.
pub struct Entry {
    /// Its name: the script's file name (without `.sh`) and the widget key
    /// it is added under unless another is asked for.
    pub name: &'static str,
    /// One line for lists and menus.
    pub description: &'static str,
    /// When it has something to show, to finish "It appears …" ("" when it
    /// always does): said when one is added while it shows nothing.
    pub appears: &'static str,
    /// The script itself.
    pub script: &'static str,
    /// The wing it goes to when none is named.
    pub wing: Wing,
    /// The definition, given the script's path as written in a command.
    def: fn(&str) -> Value,
}

impl Entry {
    /// `<name>.sh`
    pub fn file_name(&self) -> String {
        format!("{}.sh", self.name)
    }

    /// The definition for the script in the usual place,
    /// `~/.config/wingbar/widgets/<name>.sh`.
    #[cfg_attr(not(test), allow(dead_code))] // adding uses `definition_for`
    pub fn definition(&self) -> Value {
        (self.def)(&format!("{DEFAULT_DIR}/{}", self.file_name()))
    }

    /// The definition for the script installed at `path` (as `install`
    /// returns it): the home folder is written `~`, and the path is quoted
    /// for the shell when it needs to be.
    pub fn definition_for(&self, path: &Path) -> Value {
        (self.def)(&command_path(path))
    }
}

/// A script path as a command names it: `~/...` under the home folder when
/// that needs no quoting, else the whole path, quoted when needed.
fn command_path(path: &Path) -> String {
    let full = path.display().to_string();
    let home = crate::config::home().display().to_string();
    let home = home.trim_end_matches('/');
    if !home.is_empty()
        && let Some(rest) = full.strip_prefix(&format!("{home}/"))
    {
        let short = format!("~/{rest}");
        if shell_quote(&short) == short {
            return short;
        }
    }
    shell_quote(&full)
}

fn script(command: &str) -> String {
    format!("bash {command}")
}

static ENTRIES: &[Entry] = &[
    Entry {
        name: "storage",
        description: "How full the startup disk is (warns at 85%, alerts at 95%)",
        appears: "",
        script: include_str!("../catalog/storage.sh"),
        wing: Wing::Right,
        def: |p| {
            json!({
                "type": "script",
                "command": script(p),
                "every": "1m",
                "icon": "internaldrive",
                "levels": { "ok": "green" },
            })
        },
    },
    Entry {
        name: "network",
        description: "The Wi-Fi network's name, Ethernet, or offline",
        appears: "",
        script: include_str!("../catalog/network.sh"),
        wing: Wing::Right,
        def: |p| {
            json!({
                "type": "script",
                "command": script(p),
                "every": "10s",
                "icon": "wifi",
                "levels": {
                    "ok": "green",
                    "eth": { "color": "blue", "icon": "cable.connector" },
                    "dim": { "icon": "wifi.slash" },
                },
            })
        },
    },
    Entry {
        name: "mic",
        description: "Whether the microphone is muted, as an icon; click to mute or unmute",
        appears: "",
        script: include_str!("../catalog/mic.sh"),
        wing: Wing::Right,
        def: |p| {
            json!({
                "type": "script",
                "command": script(p),
                "every": "5s",
                "icon": "mic",
                "iconOnly": true,
                "levels": {
                    "ok": { "color": "green", "icon": "mic" },
                    "alert": { "icon": "mic.slash" },
                },
                "click": format!("{} toggle", script(p)),
            })
        },
    },
    Entry {
        name: "adb",
        description: "Connected Android devices (needs adb); hidden when there are none",
        appears: "while an Android device is connected",
        script: include_str!("../catalog/adb.sh"),
        wing: Wing::Right,
        def: |p| {
            json!({
                "type": "script",
                "command": script(p),
                "every": "10s",
                "icon": "smartphone",
                "levels": { "ok": "green" },
            })
        },
    },
    Entry {
        name: "aerospace-mode",
        description: "AeroSpace's binding mode, while it is not main",
        appears: "while AeroSpace is in a mode other than main",
        script: include_str!("../catalog/aerospace-mode.sh"),
        wing: Wing::Left,
        def: |p| {
            json!({
                "type": "script",
                "command": script(p),
                "every": "2s",
                "icon": "keyboard",
                "levels": { "ok": "yellow" },
            })
        },
    },
    Entry {
        name: "git-branch",
        description: "A git repository's branch, orange with uncommitted changes (name the repository in its command)",
        appears: "once its command names a repository: put the repository's folder after the script in its command",
        script: include_str!("../catalog/git-branch.sh"),
        wing: Wing::Right,
        def: |p| {
            json!({
                "type": "script",
                "command": script(p),
                "every": "5s",
                "icon": "arrow.triangle.branch",
                "levels": { "ok": "green" },
            })
        },
    },
];

/// Every catalog widget, in the order to list them.
pub fn entries() -> &'static [Entry] {
    ENTRIES
}

/// The catalog widget called `name`.
pub fn get(name: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|e| e.name == name)
}

/// Where catalog scripts are installed: `widgets/` next to the settings file
/// (`~/.config/wingbar/widgets`, or beside `$WINGBAR_CONFIG`).
pub fn widgets_dir() -> PathBuf {
    let settings = crate::config::settings_path();
    settings.parent().unwrap_or(Path::new(".")).join("widgets")
}

/// What `install` did.
#[derive(Debug, PartialEq)]
pub struct Installed {
    /// The script's path.
    pub path: PathBuf,
    /// False when a file was there already and was kept as it is.
    pub written: bool,
}

impl Installed {
    /// For the user: what happened to the file.
    pub fn message(&self) -> String {
        if self.written {
            format!("wrote {}", self.path.display())
        } else {
            format!("kept the existing {} (delete it to get the catalog's version)", self.path.display())
        }
    }
}

/// Write the catalog widget `name`'s script to `<widgets_dir>/<name>.sh`
/// (executable), creating the folder when needed. An existing file is never
/// overwritten: it is kept, and `written` is false.
pub fn install(name: &str, widgets_dir: &Path) -> Result<Installed, String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let entry = get(name).ok_or_else(|| {
        let known: Vec<&str> = ENTRIES.iter().map(|e| e.name).collect();
        format!("no catalog widget is called {name:?} (there are: {})", known.join(", "))
    })?;
    std::fs::create_dir_all(widgets_dir).map_err(|e| format!("can't create {}: {e}", widgets_dir.display()))?;
    let path = widgets_dir.join(entry.file_name());
    // create_new: never replace a file (or a link) that is there, even one
    // that appears between a check and the write
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o755).open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(Installed { path, written: false }),
        Err(e) => return Err(format!("can't write {}: {e}", path.display())),
    };
    // the mode passed to open is narrowed by the umask; the script must run
    let written = file
        .write_all(entry.script.as_bytes())
        .and_then(|_| file.set_permissions(std::fs::Permissions::from_mode(0o755)))
        .and_then(|_| file.sync_all());
    if let Err(e) = written {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(format!("can't write {}: {e}", path.display()));
    }
    Ok(Installed { path, written: true })
}

#[cfg(test)]
mod tests;
