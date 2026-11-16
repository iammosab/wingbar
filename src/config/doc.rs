//! The settings file as a JSON document that can be changed and written back:
//! key order and keys wingbar doesn't know are kept (serde_json's
//! `preserve_order`), the text is pretty-printed with two spaces, and a write
//! is atomic (a temporary file in the same folder, then a rename).

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Document {
    pub path: PathBuf,
    pub value: Value,
}

impl Document {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(path: impl Into<PathBuf>, value: Value) -> Document {
        Document { path: path.into(), value }
    }

    /// Read and parse the file. The error says which file and what is wrong.
    pub fn load(path: impl Into<PathBuf>) -> Result<Document, String> {
        let path = path.into();
        let text = std::fs::read_to_string(&path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let value = parse_text(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Document { path, value })
    }

    /// Write the document back, atomically.
    pub fn save(&self) -> Result<(), String> {
        write_atomic(&self.path, &to_text(&self.value)).map_err(|e| format!("can't write {}: {e}", self.path.display()))
    }
}

/// Parse settings text, with the line and column of a syntax error.
pub fn parse_text(text: &str) -> Result<Value, String> {
    serde_json::from_str(text)
        .map_err(|e| format!("not valid JSON at line {}, column {} ({e}); fix it and save again", e.line(), e.column()))
}

/// The file's text: pretty-printed with two spaces, ending in a newline.
pub fn to_text(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into());
    s.push('\n');
    s
}

/// The name of simple-bar's settings file, which wingbar only ever reads.
pub const SIMPLEBARRC: &str = ".simplebarrc";

/// Write `text` to `path` through a temporary file in the same folder and a
/// rename, so a reader never sees half a file. Creates the folder if needed.
/// When `path` is a symbolic link (settings kept in a dotfiles folder), the
/// file it points to is written, and the link stays. The file keeps its
/// permissions (a new one gets 0644). Simple-bar's `.simplebarrc` is never
/// written (an error).
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let path = &resolve_links(path);
    if path.file_name().is_some_and(|n| n == SIMPLEBARRC) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is simple-bar's settings file; wingbar never writes it", path.display()),
        ));
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "config.json".into());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let mode = std::fs::metadata(path).map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o644);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.set_permissions(std::fs::Permissions::from_mode(mode))?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The file `path` names after following symbolic links (also one that points
/// to a file not there yet). `path` itself when it is no link.
fn resolve_links(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    // as many as the system follows (MAXSYMLINKS); a loop ends here too
    for _ in 0..32 {
        match std::fs::read_link(&p) {
            Ok(to) if to.is_absolute() => p = to,
            Ok(to) => p = p.parent().unwrap_or(Path::new(".")).join(to),
            Err(_) => break,
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wingbar-doc-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn round_trip_keeps_key_order_and_unknown_keys() {
        let text = r#"{
  "version": 2,
  "zeta": 1,
  "// note": "kept",
  "widgets": {
    "time": { "type": "time", "custom": [1, 2] },
    "cpu": { "type": "cpu" }
  },
  "alpha": true
}"#;
        let d = temp_dir("order");
        let path = d.join("sub/config.json");
        let doc = Document::new(&path, parse_text(text).unwrap());
        doc.save().unwrap();
        let back = std::fs::read_to_string(&path).unwrap();
        let keys: Vec<&str> = back.lines().filter_map(|l| l.trim().split('"').nth(1)).collect();
        assert_eq!(keys, ["version", "zeta", "// note", "widgets", "time", "type", "custom", "cpu", "type", "alpha"]);
        assert!(back.contains("\n  \"version\": 2,\n"), "two-space indent: {back}");
        assert!(back.ends_with("}\n"));
        assert_eq!(Document::load(&path).unwrap().value, doc.value);
        // no temporary file is left behind
        assert_eq!(std::fs::read_dir(d.join("sub")).unwrap().count(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_replaces_the_file() {
        let d = temp_dir("replace");
        let path = d.join("config.json");
        Document::new(&path, json!({ "a": 1 })).save().unwrap();
        Document::new(&path, json!({ "b": 2 })).save().unwrap();
        assert_eq!(Document::load(&path).unwrap().value, json!({ "b": 2 }));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_linked_settings_file_is_written_through_and_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let d = temp_dir("link");
        // config.json is a link to a file in a dotfiles folder, readable only by its owner
        let target = d.join("dotfiles/wingbar.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "{}").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = d.join("conf/config.json");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("../dotfiles/wingbar.json", &link).unwrap();
        write_atomic(&link, "{ \"a\": 1 }\n").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "still a link");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{ \"a\": 1 }\n", "written through to the file");
        assert_eq!(mode(&target), 0o600, "its permissions kept");
        // a link to a link
        let link2 = d.join("conf/other.json");
        std::os::unix::fs::symlink(&link, &link2).unwrap();
        write_atomic(&link2, "{ \"b\": 2 }\n").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{ \"b\": 2 }\n");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        // no temporary files are left in either folder
        assert_eq!(std::fs::read_dir(d.join("dotfiles")).unwrap().count(), 1);
        assert_eq!(std::fs::read_dir(d.join("conf")).unwrap().count(), 2);
        // a plain file keeps its mode; a new one is 0644
        let plain = d.join("plain.json");
        std::fs::write(&plain, "{}").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&plain, "{}\n").unwrap();
        assert_eq!(mode(&plain), 0o640);
        let new = d.join("new.json");
        write_atomic(&new, "{}\n").unwrap();
        assert_eq!(mode(&new), 0o644);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn simple_bars_file_is_never_written() {
        let d = temp_dir("simplebarrc");
        std::fs::create_dir_all(&d).unwrap();
        let sb = d.join(".simplebarrc");
        std::fs::write(&sb, "{ \"mine\": true }").unwrap();
        let e = write_atomic(&sb, "{}").unwrap_err();
        assert!(e.to_string().contains("simple-bar"), "{e}");
        // nor through a link to it
        let link = d.join("config.json");
        std::os::unix::fs::symlink(&sb, &link).unwrap();
        assert!(write_atomic(&link, "{}").is_err());
        assert_eq!(std::fs::read_to_string(&sb).unwrap(), "{ \"mine\": true }");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2, "no temporary file left");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn load_errors_name_the_file_and_the_place() {
        let d = temp_dir("errors");
        let missing = Document::load(d.join("nope.json")).err().unwrap();
        assert!(missing.contains("can't read") && missing.contains("nope.json"), "{missing}");
        std::fs::create_dir_all(&d).unwrap();
        let bad = d.join("bad.json");
        std::fs::write(&bad, "{\n  \"version\": 2,\n  oops\n}").unwrap();
        let e = Document::load(&bad).err().unwrap();
        assert!(e.contains("bad.json") && e.contains("line 3"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
