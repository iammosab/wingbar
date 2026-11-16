//! Applying changed settings without a restart (docs/design/widgets-v2.md,
//! "Applying changes"). One `reload` for everything: `GET /reload`,
//! `wingbar reload`, the bar's menu, the `wingbar widget` commands, and the
//! watcher on the settings folder, which sees the file saved by hand.
//!
//! A reload, on the caller's thread, one at a time:
//!
//! 1. read and parse the file; if it can't be used, keep the running settings
//!    and record why (stderr and `wingbar status`), and stop here;
//! 2. keep `bar.port` / `bar.server` as they run (a restart applies them; it says so);
//! 3. build the new widget registry, keeping the widgets that didn't change;
//! 4. move the widgets that were removed, added or made anew on to a new
//!    generation, so a job of theirs started before now writes nothing (a kept
//!    widget's run under way still shows what it brings), and forget the
//!    widgets that are gone (items, errors, hidden mark);
//! 5. put the settings and the registry in use (`config` / `widgets::set_current`);
//! 6. give the scheduler the new tasks (new keys run at once, removed ones stop,
//!    the others keep their timing, and one that is running runs again when it
//!    ends) and re-run the widgets that were made anew;
//! 7. on the main thread, take up the window level, font size and colours, and
//!    lay out and draw every bar.

use crate::config::{self, Config, LoadStatus, WidgetType};
use crate::ffi::kqueue::{self, Kqueue};
use crate::sched::{self, Task};
use crate::state::{self, State};
use crate::widgets::{self, Registry};
use std::collections::HashSet;
use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the settings folder must be quiet after a change before reloading.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// A stream of changes reloads at least this often.
const MOST: Duration = Duration::from_secs(2);

/// What a reload changed.
#[derive(Debug)]
pub struct Outcome {
    pub path: PathBuf,
    /// Widgets that weren't there before (they run at once).
    pub added: Vec<String>,
    /// Widgets that are gone (they stopped, and their state was cleared).
    pub removed: Vec<String>,
    /// Widgets made anew because they or the settings they read changed (run again).
    pub remade: Vec<String>,
    pub warnings: Vec<String>,
    /// Changes that wait for a restart.
    pub restart: Vec<String>,
}

impl Outcome {
    /// For `wingbar reload` and stderr, one line each.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!("reloaded {}", self.path.display())];
        for (what, keys) in [("added", &self.added), ("removed", &self.removed), ("changed", &self.remade)] {
            if !keys.is_empty() {
                out.push(format!("{what}: {}", keys.join(", ")));
            }
        }
        out.extend(self.restart.iter().map(|r| format!("restart wingbar to apply: {r}")));
        out.extend(self.warnings.iter().map(|w| format!("warning: {w}")));
        out
    }
}

/// What a reload changes: the running bar (`Live`), or a test's stand-in.
trait Target {
    /// The settings and the registry in use.
    fn running(&self) -> (Arc<Config>, Arc<Registry>);
    /// The state the widget jobs write to.
    fn state(&self) -> &'static Mutex<State>;
    fn install(&self, cfg: Config, reg: Registry);
    fn replace_tasks(&self, tasks: Vec<Task>);
    fn refresh(&self, key: &str);
    /// Take up the new settings on screen.
    fn relayout(&self);
    fn status(&self) -> LoadStatus;
    fn set_status(&self, s: LoadStatus);
}

/// Steps 1 to 7 (see the top), for `text`, the file at `path` as just read.
fn reload_with(
    t: &dyn Target,
    path: &Path,
    text: io::Result<String>,
    probe: &dyn Fn(&Path) -> bool,
) -> Result<Outcome, String> {
    let (mut cfg, warnings) = match config::from_text(path, text, probe) {
        Ok(read) => read,
        Err(e) => {
            // the warnings and notes of the settings still in use stay
            t.set_status(LoadStatus { path: path.to_path_buf(), error: Some(e.clone()), ..t.status() });
            return Err(e);
        }
    };
    let (old_cfg, old_reg) = t.running();
    let restart = config::keep_restart_only(&mut cfg, &old_cfg);
    let reg = Registry::rebuild(&cfg, Some((&old_cfg, &old_reg)));
    let keys: HashSet<String> = reg.keys().map(String::from).collect();
    let added: Vec<String> = reg.keys().filter(|k| old_reg.get(k).is_none()).map(String::from).collect();
    let removed: Vec<String> = old_reg.keys().filter(|k| !keys.contains(*k)).map(String::from).collect();
    let remade: Vec<String> =
        reg.keys().filter(|k| old_reg.get(k).is_some() && !reg.same_widget(&old_reg, k)).map(String::from).collect();

    {
        let mut st = t.state().lock().unwrap_or_else(|e| e.into_inner());
        for k in removed.iter().chain(&added).chain(&remade) {
            st.invalidate(k);
        }
        let gone: Vec<String> = (st.items.keys().chain(st.errors.keys()).chain(st.disabled.iter()))
            .filter(|k| !keys.contains(*k))
            .cloned()
            .collect();
        for k in &gone {
            st.forget(k);
        }
        if reg.keys_of_type(WidgetType::Spaces).is_empty() {
            st.spaces.clear();
        }
        if reg.keys_of_type(WidgetType::Title).is_empty() {
            st.title.clear();
        }
    }
    let tasks = reg.tasks(t.state());
    t.install(cfg, reg);
    t.replace_tasks(tasks);
    for k in &remade {
        t.refresh(k);
    }
    t.relayout();
    t.set_status(LoadStatus {
        path: path.to_path_buf(),
        error: None,
        warnings: warnings.clone(),
        restart: restart.clone(),
    });
    Ok(Outcome { path: path.to_path_buf(), added, removed, remade, warnings, restart })
}

/// The running bar.
struct Live;

impl Target for Live {
    fn running(&self) -> (Arc<Config>, Arc<Registry>) {
        (config::current(), widgets::current())
    }
    fn state(&self) -> &'static Mutex<State> {
        state::cell()
    }
    fn install(&self, cfg: Config, reg: Registry) {
        config::set_current(cfg);
        widgets::set_current(reg);
    }
    fn replace_tasks(&self, tasks: Vec<Task>) {
        sched::replace(tasks);
    }
    fn refresh(&self, key: &str) {
        sched::refresh(key);
    }
    fn relayout(&self) {
        state::on_main_thread(|| {
            if let Some(mtm) = objc2::MainThreadMarker::new() {
                crate::ui::apply_settings(mtm);
            }
        });
    }
    fn status(&self) -> LoadStatus {
        config::status()
    }
    fn set_status(&self, s: LoadStatus) {
        config::set_status(s);
    }
}

/// The settings file's text as the last reload read it (`None`: it couldn't
/// be read). Its lock also makes reloads happen one at a time.
static LAST: Mutex<Option<String>> = Mutex::new(None);

/// Reload the settings now (see the top). Also reports to stderr.
pub fn reload() -> Result<Outcome, String> {
    reload_from_disk(false).unwrap_or_else(|| Err("nothing to reload".into()))
}

/// The watcher's reload: only when the file's text changed since the last
/// reload read it (so a save that wingbar itself made and already reloaded,
/// or a change to another file in the folder, costs one read).
fn reload_if_changed() {
    reload_from_disk(true);
}

fn reload_from_disk(only_if_changed: bool) -> Option<Result<Outcome, String>> {
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let path = config::settings_path();
    let text = std::fs::read_to_string(&path);
    if only_if_changed && text.as_ref().ok() == last.as_ref() {
        return None;
    }
    *last = text.as_ref().ok().cloned();
    let r = reload_with(&Live, &path, text, &|p: &Path| p.exists());
    match &r {
        Ok(o) => o.lines().iter().for_each(|l| eprintln!("wingbar: {l}")),
        Err(e) => eprintln!("wingbar: {e}; still using the previous settings"),
    }
    Some(r)
}

/// Watches a settings file and its folder with kqueue: the file written in
/// place, and any file created, removed or renamed in the folder (editors and
/// `write_atomic` save by writing another file and renaming it over).
pub struct Watcher {
    kq: Kqueue,
    dir: PathBuf,
    file: PathBuf,
    dir_fd: Option<OwnedFd>,
    file_fd: Option<OwnedFd>,
}

const DIR_FLAGS: u32 = kqueue::WRITE | kqueue::DELETE | kqueue::RENAME | kqueue::REVOKE;
const FILE_FLAGS: u32 =
    kqueue::WRITE | kqueue::EXTEND | kqueue::ATTRIB | kqueue::DELETE | kqueue::RENAME | kqueue::REVOKE;

impl Watcher {
    /// Watch `file` (which need not exist yet) and its folder (which must).
    pub fn new(file: &Path) -> io::Result<Watcher> {
        let dir = file.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        let mut w = Watcher { kq: Kqueue::new()?, dir, file: file.to_path_buf(), dir_fd: None, file_fd: None };
        let fd = kqueue::open_for_events(&w.dir)?;
        w.kq.watch(fd.as_fd(), DIR_FLAGS)?;
        w.dir_fd = Some(fd);
        w.arm_file();
        Ok(w)
    }

    /// Watch the file at its path again: after a save by rename, the old
    /// descriptor follows the file that was replaced. (Closing it drops its watch.)
    fn arm_file(&mut self) {
        self.file_fd =
            kqueue::open_for_events(&self.file).ok().filter(|fd| self.kq.watch(fd.as_fd(), FILE_FLAGS).is_ok());
    }

    /// Watch the folder again after it went away; true when it is back.
    fn arm_dir(&mut self) -> bool {
        self.dir_fd = kqueue::open_for_events(&self.dir).ok().filter(|fd| self.kq.watch(fd.as_fd(), DIR_FLAGS).is_ok());
        self.dir_fd.is_some()
    }

    /// Wait up to `timeout` (`None`: for ever) for a change. True when one came.
    pub fn wait(&mut self, timeout: Option<Duration>) -> io::Result<bool> {
        // while the folder is gone, look for it again every second
        let lost = self.dir_fd.is_none();
        let step = if lost {
            Some(timeout.map_or(Duration::from_secs(1), |t| t.min(Duration::from_secs(1))))
        } else {
            timeout
        };
        let mut evs = Vec::new();
        if self.kq.wait(step, &mut evs)? == 0 {
            if lost && self.arm_dir() {
                self.arm_file();
                return Ok(true);
            }
            return Ok(false);
        }
        let dir_fd = self.dir_fd.as_ref().map(|fd| fd.as_raw_fd());
        if evs
            .iter()
            .any(|&(fd, flags)| Some(fd) == dir_fd && flags & (kqueue::DELETE | kqueue::RENAME | kqueue::REVOKE) != 0)
        {
            self.arm_dir();
        }
        self.arm_file();
        Ok(true)
    }

    /// Wait (up to `timeout`) for a change, then until the changes stop for
    /// `quiet` (or `most` has gone by), so a burst counts once. False when
    /// nothing changed within `timeout`.
    pub fn next_burst(&mut self, timeout: Option<Duration>, quiet: Duration, most: Duration) -> io::Result<bool> {
        if !self.wait(timeout)? {
            return Ok(false);
        }
        let start = Instant::now();
        while start.elapsed() < most && self.wait(Some(quiet))? {}
        Ok(true)
    }
}

/// Watch the settings folder on a small thread of its own and reload after a
/// change (debounced by `DEBOUNCE`). Creates the folder if needed.
pub fn watch() {
    let path = config::settings_path();
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("wingbar: can't create {}: {e}", dir.display());
    }
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = std::fs::read_to_string(&path).ok();
    let mut w = match Watcher::new(&path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!(
                "wingbar: can't watch {} ({e}); run `wingbar reload` after changing the settings",
                path.display()
            );
            return;
        }
    };
    let started = std::thread::Builder::new().name("settings-watch".into()).stack_size(256 * 1024).spawn(move || {
        loop {
            match w.next_burst(None, DEBOUNCE, MOST) {
                Ok(true) => reload_if_changed(),
                Ok(false) => {}
                Err(e) => {
                    eprintln!("wingbar: stopped watching the settings ({e}); run `wingbar reload` after changing them");
                    return;
                }
            }
        }
    });
    if let Err(e) = started {
        eprintln!("wingbar: can't start the settings watcher ({e})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Item;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Temp {
            let d = std::env::temp_dir().join(format!("wingbar-reload-test-{name}-{}", std::process::id()));
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

    /// A bar of its own: none of the globals the running bar uses.
    struct TestBar {
        running: Mutex<(Arc<Config>, Arc<Registry>)>,
        /// Its own state (the widget jobs write there), leaked to live as long as they.
        state: &'static Mutex<State>,
        tasks: Mutex<Option<Vec<(String, u64)>>>,
        refreshed: Mutex<Vec<String>>,
        relayouts: AtomicUsize,
        status: Mutex<LoadStatus>,
        /// A scheduler that runs the tasks, when the test wants them run.
        sched: Option<sched::TestLoop>,
    }

    impl TestBar {
        fn new(cfg: Config) -> TestBar {
            let reg = Registry::build(&cfg);
            TestBar {
                running: Mutex::new((Arc::new(cfg), Arc::new(reg))),
                state: Box::leak(Box::default()),
                tasks: Mutex::new(None),
                refreshed: Mutex::default(),
                relayouts: AtomicUsize::new(0),
                status: Mutex::default(),
                sched: None,
            }
        }
        /// `new`, with a scheduler of its own running the widgets.
        fn running_widgets(cfg: Config) -> TestBar {
            let mut bar = TestBar::new(cfg);
            bar.sched = Some(sched::TestLoop::start(bar.running().1.tasks(bar.state)));
            bar
        }
        fn cfg(&self) -> Arc<Config> {
            self.running.lock().unwrap().0.clone()
        }
        fn task_keys(&self) -> Option<Vec<String>> {
            self.tasks.lock().unwrap().as_ref().map(|t| t.iter().map(|(k, _)| k.clone()).collect())
        }
    }

    impl Target for TestBar {
        fn running(&self) -> (Arc<Config>, Arc<Registry>) {
            self.running.lock().unwrap().clone()
        }
        fn state(&self) -> &'static Mutex<State> {
            self.state
        }
        fn install(&self, cfg: Config, reg: Registry) {
            *self.running.lock().unwrap() = (Arc::new(cfg), Arc::new(reg));
        }
        fn replace_tasks(&self, tasks: Vec<Task>) {
            let st = self.state.lock().unwrap();
            *self.tasks.lock().unwrap() =
                Some(tasks.iter().map(|t| (t.key.clone(), st.generation_of(&t.key))).collect());
            drop(st);
            if let Some(l) = &self.sched {
                l.replace(tasks);
            }
        }
        fn refresh(&self, key: &str) {
            self.refreshed.lock().unwrap().push(key.into());
            if let Some(l) = &self.sched {
                l.refresh(key);
            }
        }
        fn relayout(&self) {
            self.relayouts.fetch_add(1, Ordering::SeqCst);
        }
        fn status(&self) -> LoadStatus {
            self.status.lock().unwrap().clone()
        }
        fn set_status(&self, s: LoadStatus) {
            *self.status.lock().unwrap() = s;
        }
    }

    fn no_fs(_: &Path) -> bool {
        false
    }

    fn settings(right: &[&str], cpu_every: &str) -> serde_json::Value {
        json!({
            "version": 2,
            "windowManager": "none",
            "right": right,
            "widgets": {
                "cpu": { "type": "cpu", "every": cpu_every },
                "memory": { "type": "memory" },
                "time": { "type": "time" },
                "x": { "type": "script", "command": "echo x" },
            },
        })
    }

    fn parsed(doc: &serde_json::Value) -> Config {
        let (cfg, w) = config::parse::parse(doc, &no_fs);
        assert!(w.is_empty(), "{w:?}");
        cfg
    }

    fn item(text: &str) -> Item {
        let c = config::Rgba(1.0, 1.0, 1.0, 1.0);
        Item {
            icon: None,
            text: text.into(),
            icon_color: c,
            text_color: c,
            bold: false,
            level: String::new(),
            trail: None,
        }
    }

    fn reload_file(bar: &TestBar, path: &Path) -> Result<Outcome, String> {
        reload_with(bar, path, std::fs::read_to_string(path), &no_fs)
    }

    #[test]
    fn a_changed_file_replaces_the_tasks_and_clears_what_went_away() {
        let t = Temp::new("changed");
        let path = t.0.join("config.json");
        let bar = TestBar::new(parsed(&settings(&["cpu", "x", "time"], "2s")));
        {
            let mut st = bar.state.lock().unwrap();
            for k in ["cpu", "x", "time"] {
                st.items.insert(k.into(), item(k));
            }
            st.errors.insert("x".into(), "exited with status 1".into());
            st.disabled.insert("cpu".into());
        }
        let old_reg = bar.running().1;

        // cpu and the script go, memory comes, time stays
        std::fs::write(&path, settings(&["time", "memory"], "2s").to_string()).unwrap();
        let o = reload_file(&bar, &path).unwrap();
        assert_eq!(
            (o.added, o.removed, o.remade),
            (vec!["memory".to_string()], vec!["cpu".into(), "x".into()], vec![])
        );
        assert_eq!(bar.task_keys(), Some(vec!["time".into(), "memory".into()]), "the scheduler's tasks");
        assert_eq!(bar.cfg().right, ["time", "memory"]);
        {
            let st = bar.state.lock().unwrap();
            assert_eq!(st.generation_of("time"), 0, "a kept widget stays in its generation");
            assert!(["cpu", "x", "memory"].iter().all(|k| st.generation_of(k) != 0), "the others move on");
            assert_eq!(st.items.keys().collect::<Vec<_>>(), ["time"], "what is gone is cleared");
            assert!(st.errors.is_empty() && st.disabled.is_empty());
        }
        assert!(bar.running().1.same_widget(&old_reg, "time"), "time is unchanged: kept, not re-run");
        assert!(bar.refreshed.lock().unwrap().is_empty());
        assert_eq!(bar.relayouts.load(Ordering::SeqCst), 1);
        assert_eq!(bar.status().error, None);

        // a job of a removed widget started before the reload finishes now: it writes nothing
        let late = bar.state.lock().unwrap().apply(0, "cpu", None, Some(Some(item("late"))));
        assert_eq!(late, None);
        assert!(!bar.state.lock().unwrap().items.contains_key("cpu"));
        // one of the kept widget is shown
        let kept = bar.state.lock().unwrap().apply(0, "time", None, Some(Some(item("12:00"))));
        assert_eq!(kept, Some(true));

        // cpu comes back with a new period, and time's definition changes
        let mut doc = settings(&["time", "memory", "cpu"], "5s");
        doc["widgets"]["time"]["seconds"] = json!(true);
        std::fs::write(&path, doc.to_string()).unwrap();
        let o = reload_file(&bar, &path).unwrap();
        assert_eq!((o.added, o.remade), (vec!["cpu".to_string()], vec!["time".to_string()]));
        assert_eq!(*bar.refreshed.lock().unwrap(), ["time"], "a changed widget runs again");
        let (key, generation) = bar.tasks.lock().unwrap().as_ref().unwrap()[0].clone();
        assert_eq!(key, "time");
        assert_eq!(generation, bar.state.lock().unwrap().generation_of("time"), "the new widget's generation");
        assert_ne!(generation, 0);
        // a job of the replaced widget writes nothing
        assert_eq!(bar.state.lock().unwrap().apply(0, "time", None, Some(Some(item("old")))), None);
        assert_eq!(bar.running().1.get("cpu").unwrap().schedule(), sched::Every::Ms(5000));
    }

    #[test]
    fn a_kept_widget_still_shows_what_its_run_brings_across_a_reload() {
        // a slow script every 10 minutes, and the settings reloaded while its
        // first run is under way (another widget changed): what that run
        // brings is shown, not dropped until the next run in 10 minutes
        let t = Temp::new("kept-run");
        let path = t.0.join("config.json");
        // each run prints its number: "run 1", "run 2", ...
        let runs = t.0.join("runs");
        let command = format!(
            "n=$(($(cat '{0}' 2>/dev/null || echo 0) + 1)); echo $n > '{0}'; sleep 0.6; echo run $n",
            runs.display()
        );
        let doc = |seconds: bool| {
            json!({
                "version": 2,
                "windowManager": "none",
                "right": ["slow", "time"],
                "widgets": {
                    "slow": { "type": "script", "command": command, "every": "10m" },
                    "time": { "type": "time", "seconds": seconds },
                },
            })
        };
        let bar = TestBar::running_widgets(parsed(&doc(false)));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!bar.state.lock().unwrap().items.contains_key("slow"), "still running");
        std::fs::write(&path, doc(true).to_string()).unwrap();
        let o = reload_file(&bar, &path).unwrap();
        assert_eq!(o.remade, ["time"], "the script is kept");
        let start = Instant::now();
        let shown = loop {
            if let Some(it) = bar.state.lock().unwrap().items.get("slow") {
                break it.text.clone();
            }
            assert!(start.elapsed() < Duration::from_secs(5), "nothing shown");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(shown, "run 1", "the run under way at the reload is shown, not dropped");
    }

    #[test]
    fn a_file_that_cant_be_used_keeps_the_running_settings_and_says_why() {
        let t = Temp::new("broken");
        let path = t.0.join("config.json");
        let bar = TestBar::new(parsed(&settings(&["cpu"], "2s")));
        bar.state.lock().unwrap().items.insert("cpu".into(), item("12%"));
        std::fs::write(&path, "{ \"version\": 2, \"right\": [").unwrap();
        let e = reload_file(&bar, &path).unwrap_err();
        assert!(e.contains("config.json") && e.contains("not valid JSON"), "{e}");
        assert_eq!(bar.status().error.as_deref(), Some(e.as_str()), "for `wingbar status`");
        assert_eq!(bar.cfg().right, ["cpu"], "the running settings stay");
        assert_eq!(bar.task_keys(), None, "the scheduler is left alone");
        let st = bar.state.lock().unwrap();
        assert_eq!((st.generation, st.items.len()), (0, 1), "and so is the state");
        drop(st);
        assert_eq!(bar.relayouts.load(Ordering::SeqCst), 0);
        // gone altogether: the same
        std::fs::remove_file(&path).unwrap();
        assert!(reload_file(&bar, &path).unwrap_err().starts_with("can't read "));
        // fixed: applied, and the error is cleared
        std::fs::write(&path, settings(&["cpu", "time"], "2s").to_string()).unwrap();
        reload_file(&bar, &path).unwrap();
        assert_eq!((bar.cfg().right.len(), bar.status().error), (2, None));
    }

    #[test]
    fn port_and_server_changes_say_restart() {
        let t = Temp::new("port");
        let path = t.0.join("config.json");
        let bar = TestBar::new(parsed(&settings(&["cpu"], "2s")));
        let mut doc = settings(&["cpu"], "2s");
        doc["bar"] = json!({ "port": 7999, "server": false });
        doc["bogus"] = json!(1);
        std::fs::write(&path, doc.to_string()).unwrap();
        let o = reload_file(&bar, &path).unwrap();
        assert_eq!(o.restart.len(), 2, "{o:?}");
        assert_eq!((bar.cfg().bar.port, bar.cfg().bar.server), (config::parse::DEFAULT_PORT, true), "still as running");
        assert_eq!(bar.status().restart, o.restart);
        assert!(bar.status().warnings.iter().any(|w| w.contains("bogus")));
        let text = o.lines().join("\n");
        assert!(text.contains("restart wingbar to apply: bar.port") && text.contains("warning: "), "{text}");
    }

    #[test]
    fn the_watcher_sees_a_save_by_rename_and_a_write_in_place() {
        let t = Temp::new("watch");
        let path = t.0.join("config.json");
        std::fs::write(&path, "{}").unwrap();
        let mut w = Watcher::new(&path).unwrap();
        let soon = Some(Duration::from_millis(100));
        let a_while = Some(Duration::from_secs(3));
        assert!(!w.wait(soon).unwrap(), "nothing yet");
        // saved by rename, as wingbar and most editors do
        config::doc::write_atomic(&path, "{ \"version\": 2 }").unwrap();
        assert!(w.next_burst(a_while, Duration::from_millis(100), MOST).unwrap());
        assert!(!w.wait(soon).unwrap(), "the burst is over");
        // written in place (after the rename: the watch follows the new file)
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"\n").unwrap();
        }
        assert!(w.wait(a_while).unwrap(), "an in-place write");
        assert!(!w.wait(soon).unwrap());
        // removed, then created again
        std::fs::remove_file(&path).unwrap();
        assert!(w.next_burst(a_while, Duration::from_millis(100), MOST).unwrap());
        std::fs::write(&path, "{}").unwrap();
        assert!(w.next_burst(a_while, Duration::from_millis(100), MOST).unwrap());
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"\n").unwrap();
        }
        assert!(w.wait(a_while).unwrap(), "the new file is watched");
    }

    #[test]
    fn a_burst_of_changes_counts_once() {
        let t = Temp::new("burst");
        let path = t.0.join("config.json");
        std::fs::write(&path, "{}").unwrap();
        let mut w = Watcher::new(&path).unwrap();
        let p = path.clone();
        let writer = std::thread::spawn(move || {
            for i in 0..6 {
                config::doc::write_atomic(&p, &format!("{{ \"n\": {i} }}")).unwrap();
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let start = Instant::now();
        assert!(w.next_burst(Some(Duration::from_secs(3)), DEBOUNCE, MOST).unwrap());
        writer.join().unwrap();
        assert!(start.elapsed() >= DEBOUNCE, "waited for the burst to end: {:?}", start.elapsed());
        assert!(!w.next_burst(Some(Duration::from_millis(400)), DEBOUNCE, MOST).unwrap(), "one burst, one reload");
    }
}
