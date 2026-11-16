//! wingbar: a native macOS status bar in Rust that replaces Übersicht + simple-bar.
//! See `cli::HELP` (`wingbar --help`) for the commands.

#![warn(clippy::undocumented_unsafe_blocks)]

mod catalog;
mod cli;
mod config;
mod ffi;
mod icons;
mod manage;
mod reload;
mod sched;
mod server;
mod single;
mod state;
mod sys;
mod ui;
mod widgets;
mod wm;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use std::time::Duration;

/// `GET path` from the running bar; prints its answer. The exit code.
fn client(port: u16, path: &str) -> i32 {
    match cli::request(port, path, Duration::from_secs(10)) {
        Ok((code, body)) => {
            print!("{body}");
            if code == 200 { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// With `--quit-after`, `WINGBAR_MEASURE=<file>` writes what the bar used (as
/// `key value` lines) just before it quits, for `scripts/measure.sh`; with
/// `WINGBAR_MEASURE_WARMUP=<seconds>` also what it had used by then (keys
/// prefixed `warm.`), so the startup cost can be told from the steady state.
fn usage_lines(prefix: &str, secs: u64) -> String {
    let u = ffi::proc::usage();
    let s = sched::stats();
    let lines = format!(
        "seconds {secs}\nuser {:.3}\nsystem {:.3}\nchild_user {:.3}\nchild_system {:.3}\n\
         voluntary_switches {}\ninvoluntary_switches {}\nmax_rss {}\nphys_footprint {}\nmax_phys_footprint {}\n\
         idle_wakeups {}\ninterrupt_wakeups {}\ninstructions {}\ncycles {}\n\
         runs {}\nskipped {}\nthreads_started {}\n",
        u.user,
        u.system,
        u.child_user,
        u.child_system,
        u.voluntary_switches,
        u.involuntary_switches,
        u.max_rss,
        u.phys_footprint,
        u.max_phys_footprint,
        u.idle_wakeups,
        u.interrupt_wakeups,
        u.instructions,
        u.cycles,
        s.runs,
        s.skipped,
        s.threads,
    );
    lines.lines().map(|l| format!("{prefix}{l}\n")).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (port, no_server, quit_after, snap) = match cli::parse(&args) {
        Err(e) => {
            eprintln!("wingbar: {e}");
            std::process::exit(2);
        }
        Ok(cli::Command::Help(text)) => {
            print!("{text}");
            return;
        }
        Ok(cli::Command::Version) => {
            println!("wingbar {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        // talking to a running bar reads its port and writes nothing
        Ok(cli::Command::Client { port, path }) => {
            std::process::exit(client(port.unwrap_or_else(config::client_port), &path))
        }
        Ok(cli::Command::Widget { port, cmd }) => std::process::exit(cli::widget(port, cmd)),
        Ok(cli::Command::Config { port, cmd }) => std::process::exit(cli::config_cmd(port, cmd)),
        Ok(cli::Command::Run { port, no_server, quit_after, snapshot }) => (port, no_server, quit_after, snapshot),
    };

    // the bar writes its settings at the first start: never simple-bar's
    if let Err(e) = config::usable_settings_path() {
        eprintln!("wingbar: {e}");
        std::process::exit(1);
    }
    let cfg = config::load(port, no_server);
    // one bar per settings file: a bar already on them steps aside (not for
    // the short runs that measure or take pictures, which only look)
    single::exit_cleanly_on_sigterm();
    if quit_after.is_none() && snap.is_none() {
        let port = cfg.bar.server.then_some(cfg.bar.port);
        if let Some(note) = single::take_over(&config::settings_path(), port) {
            eprintln!("wingbar: {note}");
        }
    }
    widgets::set_current(widgets::Registry::build(&cfg));

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    // Accessibility lets us read window titles in-process instead of asking AeroSpace.
    let prompt = quit_after.is_none() && std::env::var_os("WINGBAR_NO_PROMPT").is_none();
    widgets::AX_OK.store(sys::ax_trusted(prompt), std::sync::atomic::Ordering::Relaxed);

    ui::build_bars(mtm);
    ui::install_observers(mtm);
    sched::start(widgets::current().tasks(state::cell()));
    if cfg.bar.server {
        server::start(cfg.bar.port);
    }
    // saving the settings file applies it
    reload::watch();
    if let Some(dir) = snap {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            state::on_main_thread(move || {
                if let Some(v) = std::env::var_os("WINGBAR_DEBUG_MENUS") {
                    let mtm = MainThreadMarker::new().expect("main thread");
                    if v == "manage" {
                        ui::debug_bar_menus(mtm);
                    } else {
                        ui::debug_menus(mtm, v == "fire");
                    }
                }
                let saved = ui::snapshot(&dir);
                std::process::exit(if saved > 0 { 0 } else { 1 });
            });
        });
    }
    if let Some(secs) = quit_after {
        let measure = std::env::var_os("WINGBAR_MEASURE");
        let warmup = measure
            .as_ref()
            .and_then(|_| std::env::var("WINGBAR_MEASURE_WARMUP").ok()?.parse::<u64>().ok())
            .unwrap_or(0)
            .min(secs);
        std::thread::spawn(move || {
            let mut report = String::new();
            if warmup > 0 {
                std::thread::sleep(Duration::from_secs(warmup));
                report = usage_lines("warm.", warmup);
            }
            std::thread::sleep(Duration::from_secs(secs - warmup));
            if let Some(path) = measure {
                report += &usage_lines("", secs);
                if let Err(e) = std::fs::write(&path, report) {
                    eprintln!("wingbar: can't write {}: {e}", path.display());
                }
            }
            std::process::exit(0);
        });
    }
    app.run();
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_client_is_not_mistaken_for_a_browser() {
        // the server on an ephemeral loopback port (never the bar's own)
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            crate::server::handle(listener.accept().unwrap().0, &crate::widgets::fixture_registry())
        });
        assert_eq!(super::client(port, "/refresh"), 0);
        server.join().unwrap();
    }
}
