//! One scheduler thread for every widget. It sleeps until the next widget is
//! due (or until something asks for a refresh), so an idle bar costs nothing.
//! A widget never runs twice at once, and a slow script never blocks others.
//! A widget asked to run while it is still running runs once more when done.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const PATH: &str = "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Every {
    Ms(u64),
    /// Just after each minute (or second) boundary: the clock.
    Minute,
    Second,
}

pub struct Task {
    pub key: String,
    pub every: Every,
    pub job: Arc<dyn Fn() + Send + Sync>,
}

enum Msg {
    Refresh(String, u64),
    All,
    /// New settings: run these tasks from now on (see `replace`).
    Replace(Vec<Task>),
    /// Stop the loop (tests only: the bar's scheduler runs as long as the app).
    #[cfg(test)]
    Quit,
}

/// The scheduler's inbox. `Sender` is `Sync`, so it needs no lock (and no lock can be poisoned).
static TX: OnceLock<Sender<Msg>> = OnceLock::new();

/// Counters for `WINGBAR_MEASURE` (see `main.rs`): widget runs started, runs
/// put off because the widget was still running, and threads this module started.
static RUNS: AtomicU64 = AtomicU64::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);
static THREADS: AtomicU64 = AtomicU64::new(0);

pub struct Stats {
    pub runs: u64,
    pub skipped: u64,
    pub threads: u64,
}

pub fn stats() -> Stats {
    Stats {
        runs: RUNS.load(Ordering::Relaxed),
        skipped: SKIPPED.load(Ordering::Relaxed),
        threads: THREADS.load(Ordering::Relaxed),
    }
}

fn counted() {
    THREADS.fetch_add(1, Ordering::Relaxed);
}

/// Re-run one widget now (after `delay_ms`).
pub fn refresh_in(key: &str, delay_ms: u64) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Msg::Refresh(key.to_string(), delay_ms));
    }
}
pub fn refresh(key: &str) {
    refresh_in(key, 0);
}
pub fn refresh_all() {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Msg::All);
    }
}

/// Run these tasks instead of the current ones (the settings were reloaded).
/// A new key runs at once; a key that is gone stops (a run in progress
/// finishes); a key that stays keeps its timing, unless its period changed
/// (then it runs at once), and is still never run twice at once; one that is
/// running runs once more when that run ends.
pub fn replace(tasks: Vec<Task>) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(Msg::Replace(tasks));
    }
}

fn until_boundary(unit_ms: u64) -> Duration {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    Duration::from_millis(unit_ms - now % unit_ms + 20)
}

// A slot's run state.
const IDLE: u8 = 0;
const RUNNING: u8 = 1;
/// Running, and asked to run again meanwhile: what it is computing may already
/// be stale (the workspace changed after the run started), so it runs once more.
const AGAIN: u8 = 2;

struct Slot {
    task: Task,
    next: Instant,
    /// The next run was asked for (a refresh), not just due on the timer.
    asked: bool,
    state: Arc<AtomicU8>,
}

impl Slot {
    fn schedule_next(&mut self, now: Instant) {
        self.next = match self.task.every {
            Every::Ms(ms) => now + Duration::from_millis(ms),
            Every::Minute => now + until_boundary(60_000),
            Every::Second => now + until_boundary(1_000),
        };
    }

    /// Start the job on its own thread. If it is still running, a timer run is
    /// skipped (a slow script with a short interval isn't run back to back), but
    /// a refresh runs it once more as soon as it finishes, however often it came.
    ///
    /// A thread per run rather than a pool: a hung script (up to its 30 s
    /// timeout) must never hold up another widget, and starting a thread costs
    /// about 20 µs, against ~160 µs for the bar to start a script and far more
    /// for the script itself (measured; see `scripts/measure.sh`).
    fn run(&mut self, tx: &Sender<Msg>) {
        let asked = std::mem::take(&mut self.asked);
        loop {
            let busy = match self.state.compare_exchange(IDLE, RUNNING, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => break,
                Err(AGAIN) => true,
                Err(_) => {
                    // RUNNING: mark it AGAIN, unless it finished in between (then start it)
                    !asked || self.state.compare_exchange(RUNNING, AGAIN, Ordering::SeqCst, Ordering::SeqCst).is_ok()
                }
            };
            if busy {
                SKIPPED.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        RUNS.fetch_add(1, Ordering::Relaxed);
        counted();
        let job = self.task.job.clone();
        let state = self.state.clone();
        let (key, tx) = (self.task.key.clone(), tx.clone());
        let started = std::thread::Builder::new().name(format!("w:{key}")).stack_size(256 * 1024).spawn(move || {
            job();
            if state.swap(IDLE, Ordering::SeqCst) == AGAIN {
                let _ = tx.send(Msg::Refresh(key, 0));
            }
        });
        if started.is_err() {
            // never leave it marked as running, or it would never run again
            self.state.store(IDLE, Ordering::SeqCst);
        }
    }
}

pub fn start(tasks: Vec<Task>) {
    let (tx, rx) = channel::<Msg>();
    let _ = TX.set(tx.clone());
    std::thread::Builder::new()
        .name("scheduler".into())
        .spawn(move || run_loop(tasks, &tx, &rx))
        .expect("scheduler thread");
}

/// The slots for `tasks`. A key that had a slot in `old` keeps its timing and
/// its run state (so it never runs twice at once across a reload), unless its
/// period changed; any other runs at `now`. A kept key whose old task is
/// running runs its new task once more as soon as that run ends: the old
/// run's result may be dropped (its widget was made anew), and the next timed
/// run could be a long way off.
fn slots_for(tasks: Vec<Task>, old: Vec<Slot>, now: Instant) -> Vec<Slot> {
    let mut old: HashMap<String, Slot> = old.into_iter().map(|s| (s.task.key.clone(), s)).collect();
    tasks
        .into_iter()
        .map(|task| match old.remove(&task.key) {
            Some(prev) => {
                let _ = prev.state.compare_exchange(RUNNING, AGAIN, Ordering::SeqCst, Ordering::SeqCst);
                let next = if prev.task.every == task.every { prev.next } else { now };
                Slot { task, next, asked: prev.asked, state: prev.state }
            }
            None => Slot { task, next: now, asked: false, state: Arc::new(AtomicU8::new(IDLE)) },
        })
        .collect()
}

/// Slot indices by key.
fn index_of(slots: &[Slot]) -> HashMap<String, Vec<usize>> {
    slots.iter().enumerate().fold(HashMap::new(), |mut m, (i, s)| {
        m.entry(s.task.key.clone()).or_default().push(i);
        m
    })
}

/// Run what is due, then sleep until the next task is due or a message arrives.
fn run_loop(tasks: Vec<Task>, tx: &Sender<Msg>, rx: &Receiver<Msg>) {
    let mut slots = slots_for(tasks, Vec::new(), Instant::now());
    let mut index = index_of(&slots);
    loop {
        let now = Instant::now();
        for s in slots.iter_mut() {
            if s.next <= now {
                s.schedule_next(now);
                s.run(tx);
            }
        }
        let wake = slots.iter().map(|s| s.next).min().unwrap_or(now + Duration::from_secs(3600));
        let wait = wake.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Msg::Refresh(k, delay)) => {
                if let Some(ix) = index.get(&k) {
                    let at = Instant::now() + Duration::from_millis(delay);
                    for &i in ix {
                        slots[i].asked = true;
                        if slots[i].next > at {
                            slots[i].next = at;
                        }
                    }
                }
            }
            Ok(Msg::All) => {
                let at = Instant::now();
                for s in slots.iter_mut() {
                    s.asked = true;
                    s.next = at;
                }
            }
            Ok(Msg::Replace(tasks)) => {
                slots = slots_for(tasks, std::mem::take(&mut slots), Instant::now());
                index = index_of(&slots);
            }
            #[cfg(test)]
            Ok(Msg::Quit) => break,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn command(program: &str, args: &[&str]) -> Command {
    let mut c = Command::new(program);
    c.args(args).env("PATH", PATH).stdin(Stdio::null()).stderr(Stdio::null());
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    c
}

/// Why a program didn't finish cleanly. The variants for a program that did run
/// keep what it printed, because callers have always used that output anyway.
#[derive(Clone, Debug, PartialEq)]
pub enum RunError {
    /// It never started (or its output couldn't be captured).
    Spawn(String),
    /// The watchdog killed it after `after`.
    Timeout { after: Duration, stdout: String },
    /// It exited with a non-zero status.
    Exit { code: i32, stdout: String },
    /// Something other than the watchdog killed it.
    Signal { signal: i32, stdout: String },
    /// Waiting for it failed.
    Wait(String),
}

impl RunError {
    /// What the program printed, if it ran at all.
    pub fn stdout(self) -> Option<String> {
        match self {
            RunError::Timeout { stdout, .. } | RunError::Exit { stdout, .. } | RunError::Signal { stdout, .. } => {
                Some(stdout)
            }
            RunError::Spawn(_) | RunError::Wait(_) => None,
        }
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Spawn(e) => write!(f, "failed to start: {e}"),
            RunError::Timeout { after, .. } => write!(f, "timed out after {}s", after.as_secs_f64()),
            RunError::Exit { code, .. } => write!(f, "exited with status {code}"),
            RunError::Signal { signal, .. } => write!(f, "killed by signal {signal}"),
            RunError::Wait(e) => write!(f, "failed to wait: {e}"),
        }
    }
}

/// The contract `run_program` had before it reported errors: the output,
/// whatever the exit status, or `None` if the program never ran.
pub fn output(r: Result<String, RunError>) -> Option<String> {
    r.or_else(|e| e.stdout().ok_or(())).ok()
}

/// Run a program and return its output, killing it (and its children) after `timeout`.
///
/// The watchdog is a tiny parked thread per command; it only wakes if the
/// command overruns. (It costs ~20 µs per command, a fraction of what starting
/// the command costs, so a shared timer thread wouldn't pay for its complexity.)
pub fn run_program(program: &str, args: &[&str], timeout: Duration) -> Result<String, RunError> {
    run_program_limited(program, args, timeout, None)
}

/// The most output a run keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limit {
    pub lines: usize,
    pub bytes: usize,
}

/// Read `r` to the end, or until `limit` is reached; then the output is cut
/// after the last whole line that fits. Returns the output and whether it was cut.
fn read_limited(r: &mut impl Read, limit: Limit) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut lines = 0;
    loop {
        let n = match r.read(&mut chunk) {
            Ok(0) => return (buf, false),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return (buf, false),
        };
        for &b in &chunk[..n] {
            if buf.len() >= limit.bytes {
                // a line cut short could be half a command: drop it
                let keep = buf.iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1);
                buf.truncate(keep);
                return (buf, true);
            }
            buf.push(b);
            if b == b'\n' {
                lines += 1;
                if lines >= limit.lines {
                    return (buf, true);
                }
            }
        }
    }
}

/// `run_program`, keeping at most `limit` of the output. A program that prints
/// more is killed (with its process group) once the limit is reached, and what
/// was kept counts as a clean run.
pub fn run_program_limited(
    program: &str,
    args: &[&str],
    timeout: Duration,
    limit: Option<Limit>,
) -> Result<String, RunError> {
    let mut child =
        command(program, args).stdout(Stdio::piped()).spawn().map_err(|e| RunError::Spawn(e.to_string()))?;
    let pid = child.id() as i32;
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(RunError::Spawn("no stdout pipe".into()));
    };
    let done = Arc::new(AtomicBool::new(false));
    let killed = Arc::new(AtomicBool::new(false));
    let (d2, k2) = (done.clone(), killed.clone());
    counted();
    let guard = std::thread::Builder::new().stack_size(64 * 1024).spawn(move || {
        std::thread::park_timeout(timeout);
        if !d2.load(Ordering::SeqCst) {
            k2.store(true, Ordering::SeqCst);
            // SAFETY: kill(2) takes plain integers; -pid targets the child's own process group.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    });
    let (buf, cut) = match limit {
        Some(limit) => read_limited(&mut out, limit),
        None => {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            (buf, false)
        }
    };
    if cut {
        // SAFETY: kill(2) takes plain integers; -pid targets the child's own process
        // group, and the child isn't reaped yet, so its pid is still the group's id.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    drop(out);
    let status = child.wait();
    done.store(true, Ordering::SeqCst);
    if let Ok(g) = guard {
        g.thread().unpark();
    }
    let status = status.map_err(|e| RunError::Wait(e.to_string()))?;
    let stdout = String::from_utf8_lossy(&buf).into_owned();
    if status.success() || cut {
        return Ok(stdout);
    }
    if killed.load(Ordering::SeqCst) {
        return Err(RunError::Timeout { after: timeout, stdout });
    }
    match (status.code(), std::os::unix::process::ExitStatusExt::signal(&status)) {
        (Some(code), _) => Err(RunError::Exit { code, stdout }),
        (None, Some(signal)) => Err(RunError::Signal { signal, stdout }),
        (None, None) => Err(RunError::Exit { code: -1, stdout }),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn run_shell(cmd: &str, timeout: Duration) -> Result<String, RunError> {
    run_program("/bin/bash", &["-c", cmd], timeout)
}

/// `run_shell`, keeping at most `limit` of the output (see `run_program_limited`).
pub fn run_shell_limited(cmd: &str, timeout: Duration, limit: Limit) -> Result<String, RunError> {
    run_program_limited("/bin/bash", &["-c", cmd], timeout, Some(limit))
}

/// Start a shell command and forget it (a click action). Reaped on a tiny thread.
pub fn spawn_shell(cmd: &str) {
    let cmd = cmd.trim();
    if cmd.is_empty() || cmd == "true" {
        return;
    }
    spawn_program("/bin/bash", &["-c", cmd]);
}

/// Start a program with these arguments (no shell) and forget it, in its own
/// process group with no stdin/stdout/stderr. Reaped on a tiny thread.
pub fn spawn_program(program: &str, args: &[&str]) {
    if let Ok(mut child) = command(program, args).stdout(Stdio::null()).spawn() {
        counted();
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(move || {
                let _ = child.wait();
            })
            .ok();
    }
}

/// A scheduler of its own (not the global one), for tests; stopped when dropped.
#[cfg(test)]
pub(crate) struct TestLoop {
    tx: Sender<Msg>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(test)]
impl TestLoop {
    pub(crate) fn start(tasks: Vec<Task>) -> TestLoop {
        let (tx, rx) = channel();
        let tx2 = tx.clone();
        let thread = std::thread::spawn(move || run_loop(tasks, &tx2, &rx));
        TestLoop { tx, thread: Some(thread) }
    }

    /// `replace`, on this scheduler.
    pub(crate) fn replace(&self, tasks: Vec<Task>) {
        let _ = self.tx.send(Msg::Replace(tasks));
    }

    /// `refresh`, on this scheduler.
    pub(crate) fn refresh(&self, key: &str) {
        let _ = self.tx.send(Msg::Refresh(key.to_string(), 0));
    }
}

#[cfg(test)]
impl Drop for TestLoop {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn until_boundary_is_just_past_the_next_boundary() {
        for unit in [1_000, 60_000] {
            for _ in 0..50 {
                let d = until_boundary(unit);
                assert!(d > Duration::from_millis(20) && d <= Duration::from_millis(unit + 20), "{unit}: {d:?}");
            }
        }
    }

    #[test]
    fn refresh_is_a_no_op_before_the_scheduler_starts() {
        // the server routes call these; they must not block or panic without a scheduler
        assert!(TX.get().is_none());
        refresh("anything");
        refresh_in("anything", 10);
        refresh_all();
    }

    use super::TestLoop as Loop;

    fn task(key: &str, every: Every, job: impl Fn() + Send + Sync + 'static) -> Task {
        Task { key: key.into(), every, job: Arc::new(job) }
    }

    /// Wait (up to 5 s) until `f` holds.
    fn eventually(what: &str, f: impl Fn() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_widget_never_runs_twice_at_once() {
        // due every 5 ms, but each run takes 40 ms
        let (now, most, runs) =
            (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (n, m, r) = (now.clone(), most.clone(), runs.clone());
        let l = Loop::start(vec![task("slow", Every::Ms(5), move || {
            let inside = n.fetch_add(1, Ordering::SeqCst) + 1;
            m.fetch_max(inside, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(40));
            n.fetch_sub(1, Ordering::SeqCst);
            r.fetch_add(1, Ordering::SeqCst);
        })]);
        eventually("five runs", || runs.load(Ordering::SeqCst) >= 5);
        drop(l);
        assert_eq!(most.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_hung_widget_does_not_hold_up_the_others() {
        let release = Arc::new(AtomicBool::new(false));
        let fast = Arc::new(AtomicUsize::new(0));
        let (rel, f) = (release.clone(), fast.clone());
        let l = Loop::start(vec![
            task("hung", Every::Ms(10), move || {
                while !rel.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }),
            task("fast", Every::Ms(10), move || {
                f.fetch_add(1, Ordering::SeqCst);
            }),
        ]);
        eventually("the fast widget runs while the other hangs", || fast.load(Ordering::SeqCst) >= 10);
        release.store(true, Ordering::SeqCst);
        drop(l);
    }

    #[test]
    fn a_refresh_while_running_runs_it_once_more_afterwards() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let gate = Arc::new(AtomicBool::new(false));
        let (lg, g) = (log.clone(), gate.clone());
        // a long period, so only the start and the refreshes make it run
        let l = Loop::start(vec![task("spaces", Every::Ms(3_600_000), move || {
            lg.lock().unwrap().push("start");
            while !g.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
            }
            lg.lock().unwrap().push("end");
        })]);
        eventually("the first run starts", || log.lock().unwrap().len() == 1);
        // three refreshes while it runs: they fold into one more run
        for _ in 0..3 {
            l.tx.send(Msg::Refresh("spaces".into(), 0)).unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(*log.lock().unwrap(), ["start"], "not started twice at once");
        gate.store(true, Ordering::SeqCst);
        eventually("the second run ends", || log.lock().unwrap().len() == 4);
        std::thread::sleep(Duration::from_millis(50));
        drop(l);
        assert_eq!(*log.lock().unwrap(), ["start", "end", "start", "end"]);
    }

    #[test]
    fn refresh_runs_a_widget_early_and_others_are_left_alone() {
        let (a, b) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (a2, b2) = (a.clone(), b.clone());
        let l = Loop::start(vec![
            task("a", Every::Ms(3_600_000), move || {
                a2.fetch_add(1, Ordering::SeqCst);
            }),
            task("b", Every::Ms(3_600_000), move || {
                b2.fetch_add(1, Ordering::SeqCst);
            }),
        ]);
        eventually("both run at the start", || a.load(Ordering::SeqCst) == 1 && b.load(Ordering::SeqCst) == 1);
        l.tx.send(Msg::Refresh("a".into(), 0)).unwrap();
        l.tx.send(Msg::Refresh("no-such-widget".into(), 0)).unwrap();
        eventually("a runs again", || a.load(Ordering::SeqCst) == 2);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(b.load(Ordering::SeqCst), 1);
        l.tx.send(Msg::All).unwrap();
        eventually("all run again", || a.load(Ordering::SeqCst) == 3 && b.load(Ordering::SeqCst) == 2);
    }

    #[test]
    fn replacing_tasks_starts_new_keys_stops_removed_ones_and_keeps_the_rest() {
        let count = |n: &Arc<AtomicUsize>| n.load(Ordering::SeqCst);
        let counter = |n: &Arc<AtomicUsize>| {
            let n = n.clone();
            move || {
                n.fetch_add(1, Ordering::SeqCst);
            }
        };
        let (a, b, a2, c) = (
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
            Arc::new(AtomicUsize::new(0)),
        );
        let hour = Every::Ms(3_600_000);
        let l = Loop::start(vec![task("a", hour, counter(&a)), task("b", Every::Ms(10), counter(&b))]);
        eventually("both run", || count(&a) == 1 && count(&b) >= 2);
        // "a" stays (new job, same period: keeps its timing), "b" goes, "c" is new
        l.tx.send(Msg::Replace(vec![task("a", hour, counter(&a2)), task("c", hour, counter(&c))])).unwrap();
        eventually("the new key runs at once", || count(&c) == 1);
        let b_then = count(&b);
        std::thread::sleep(Duration::from_millis(80));
        assert!(count(&b) <= b_then + 1, "the removed key stopped (one run may have been under way)");
        assert_eq!((count(&a), count(&a2)), (1, 0), "the kept key waits for its time");
        // refreshes reach the new tasks by key
        l.tx.send(Msg::Refresh("a".into(), 0)).unwrap();
        l.tx.send(Msg::Refresh("b".into(), 0)).unwrap();
        eventually("a's new job runs", || count(&a2) == 1);
        // a changed period runs it at once
        l.tx.send(Msg::Replace(vec![task("a", Every::Ms(1_800_000), counter(&a2))])).unwrap();
        eventually("a runs again", || count(&a2) == 2);
        assert_eq!(count(&c), 1);
    }

    #[test]
    fn a_kept_widget_running_during_a_replace_runs_again_when_it_ends() {
        // a 10-minute widget whose run takes a while; the settings are reloaded
        // while it runs. What that run brings may be dropped (its widget may
        // have been made anew), so the new task runs once more right after it,
        // instead of in 10 minutes.
        let (old, new) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let started = Arc::new(AtomicBool::new(false));
        let (o, s) = (old.clone(), started.clone());
        let ten_minutes = Every::Ms(600_000);
        let l = Loop::start(vec![task("slow", ten_minutes, move || {
            s.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(200));
            o.fetch_add(1, Ordering::SeqCst);
        })]);
        eventually("the first run starts", || started.load(Ordering::SeqCst));
        let n = new.clone();
        l.replace(vec![task("slow", ten_minutes, move || {
            n.fetch_add(1, Ordering::SeqCst);
        })]);
        eventually("the new task runs after the old run ends", || new.load(Ordering::SeqCst) == 1);
        assert_eq!(old.load(Ordering::SeqCst), 1, "never at the same time");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(new.load(Ordering::SeqCst), 1, "once");
    }

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn run_shell_success() {
        assert_eq!(run_shell("echo hi", T), Ok("hi\n".to_string()));
    }

    #[test]
    fn run_shell_exit_code_keeps_output() {
        let e = run_shell("echo partial; exit 3", T).unwrap_err();
        assert_eq!(e, RunError::Exit { code: 3, stdout: "partial\n".into() });
        assert_eq!(e.to_string(), "exited with status 3");
        assert_eq!(output(Err(e)), Some("partial\n".into()));
    }

    #[test]
    fn run_shell_timeout_kills_the_group() {
        let start = Instant::now();
        // the background sleep prints its pid, so we can check it was killed too
        let e = run_shell("echo early; sleep 30 & echo $!; wait", Duration::from_millis(300)).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
        let RunError::Timeout { after, stdout } = &e else { panic!("{e:?}") };
        assert_eq!(*after, Duration::from_millis(300));
        let mut lines = stdout.lines();
        assert_eq!(lines.next(), Some("early"));
        let grandchild: i32 = lines.next().and_then(|p| p.parse().ok()).expect("the sleep's pid");
        // SAFETY: signal 0 only asks whether the process exists; it was ours.
        eventually("the grandchild is gone", || unsafe { libc::kill(grandchild, 0) } != 0);
        assert_eq!(e.to_string(), "timed out after 0.3s");
        assert_eq!(
            RunError::Timeout { after: Duration::from_secs(30), stdout: String::new() }.to_string(),
            "timed out after 30s"
        );
    }

    #[test]
    fn run_shell_signal() {
        let e = run_shell("kill -TERM $$", T).unwrap_err();
        assert_eq!(e, RunError::Signal { signal: libc::SIGTERM, stdout: String::new() });
    }

    fn limited(input: &str) -> (String, bool) {
        let limit = Limit { lines: 3, bytes: 20 };
        let (out, cut) = read_limited(&mut std::io::Cursor::new(input.as_bytes()), limit);
        (String::from_utf8(out).unwrap(), cut)
    }

    #[test]
    fn read_limited_cuts_at_whole_lines() {
        assert_eq!(limited(""), (String::new(), false));
        assert_eq!(limited("a\nb\n"), ("a\nb\n".into(), false));
        assert_eq!(limited("a\nb\nno newline"), ("a\nb\nno newline".into(), false), "a clean end keeps it");
        assert_eq!(limited("a\nb\nc\n"), ("a\nb\nc\n".into(), true), "at the limit: cut (and killed)");
        assert_eq!(limited("a\nb\nc\nd\ne\n"), ("a\nb\nc\n".into(), true));
        // 20 bytes: a line that doesn't fit is dropped whole
        assert_eq!(limited("0123456789\nabcdefghijklmnop\n"), ("0123456789\n".into(), true));
        assert_eq!(limited("0123456789012345678\nx"), ("0123456789012345678\n".into(), true), "exactly 20 bytes");
        assert_eq!(limited("01234567890123456789012345"), (String::new(), true), "one endless line: nothing");
    }

    #[test]
    fn run_shell_limited_kills_a_flood_quickly() {
        let limit = Limit { lines: 64, bytes: 16 * 1024 };
        let start = Instant::now();
        // endless output from a pipeline: the whole group must go
        let out = run_shell_limited("yes 'menu: x | y' | cat", Duration::from_secs(30), limit).unwrap();
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
        assert_eq!(out.lines().count(), 64);
        assert!(out.lines().all(|l| l == "menu: x | y"));
        // 100k lines that would end by themselves
        let start = Instant::now();
        let out = run_shell_limited("seq 100000", Duration::from_secs(30), limit).unwrap();
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
        assert_eq!((out.lines().count(), out.lines().last()), (64, Some("64")));
        // one huge line: nothing whole fits after line 1
        let out = run_shell_limited("echo 'ok|x'; head -c 100000 /dev/zero | tr '\\0' a", T, limit).unwrap();
        assert_eq!(out, "ok|x\n");
        // under the limit: as before, errors included
        assert_eq!(run_shell_limited("echo hi", T, limit), Ok("hi\n".into()));
        let e = run_shell_limited("echo partial; exit 3", T, limit).unwrap_err();
        assert_eq!(e, RunError::Exit { code: 3, stdout: "partial\n".into() });
    }

    #[test]
    fn run_program_that_cannot_start() {
        let e = run_program("/nonexistent/wingbar-test-binary", &[], T).unwrap_err();
        assert!(matches!(e, RunError::Spawn(_)), "{e:?}");
        assert!(e.to_string().starts_with("failed to start: "));
        assert_eq!(output(Err(e)), None);
    }
}
