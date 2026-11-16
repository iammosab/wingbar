//! A tiny local HTTP endpoint that speaks simple-bar-server's routes, so the
//! person's AeroSpace hook and widget scripts keep working unchanged:
//!
//!   /aerospace/spaces/refresh            /yabai/spaces/refresh
//!   /widget/<name>/(refresh|toggle|enable|disable)
//!   /widget/user-widget/(refresh|toggle|enable|disable)/<n>
//!   /status   /reload   /quit
//!   /widgets   (JSON: each widget's key, what it shows and its last error,
//!              for `wingbar widget list`)
//!
//! Only 127.0.0.1 is listened on. One small thread, blocked in accept().

use crate::config::WidgetType;
use crate::sched;
use crate::state;
use crate::widgets::{self, Registry};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub fn start(port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "wingbar: can't listen on 127.0.0.1:{port} ({e}). Is simple-bar-server still running? \
                 The bar works, but AeroSpace's instant refresh won't reach it."
            );
            return;
        }
    };
    std::thread::Builder::new()
        .name("server".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            for s in listener.incoming().flatten() {
                // the registry in use now (settings can be reloaded)
                handle(s, &widgets::current());
            }
        })
        .ok();
}

/// The part of a request the server looks at.
#[derive(Debug, PartialEq)]
struct Request {
    path: String,
    /// It carries a header only browsers send (`Origin`, `Sec-Fetch-Site`,
    /// `Sec-Fetch-Mode`): a web page is trying to reach the bar.
    from_browser: bool,
}

/// Read the request line and headers from the raw request text.
fn parse_request(raw: &str) -> Request {
    let mut lines = raw.lines();
    let target = lines.next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/").to_string();
    let from_browser = lines.take_while(|l| !l.trim().is_empty()).any(|l| {
        l.split_once(':').is_some_and(|(name, _)| {
            let name = name.trim();
            ["origin", "sec-fetch-site", "sec-fetch-mode"].iter().any(|h| name.eq_ignore_ascii_case(h))
        })
    });
    Request { path, from_browser }
}

const MAX_REQUEST: usize = 8192;

/// Read until the end of the headers or until the client closes its side
/// (`echo "GET /quit" | nc` sends no blank line). A client that stalls mid-headers
/// is refused rather than acted on, so the browser check above always sees every
/// header line. 500 ms per read, 1 s in all, so a slow client can't hold the one
/// server thread.
fn read_request(s: &mut TcpStream) -> Result<String, (&'static str, &'static str)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(("408 Request Timeout", "request timed out\n"));
        }
        match s.read(&mut chunk) {
            Ok(0) => break, // client closed: what it sent is the whole request
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return Err(("408 Request Timeout", "request timed out\n")),
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.windows(2).any(|w| w == b"\n\n") {
            break;
        }
        if buf.len() >= MAX_REQUEST {
            return Err(("431 Request Header Fields Too Large", "request too large\n"));
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn respond(s: &mut TcpStream, code: &str, body: &str) {
    // a JSON answer starts with '{'; everything else is text
    let ty = if body.starts_with('{') { "application/json" } else { "text/plain; charset=utf-8" };
    let resp = format!(
        "HTTP/1.1 {code}\r\nContent-Type: {ty}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = s.write_all(resp.as_bytes());
}

pub(crate) fn handle(mut s: TcpStream, reg: &Registry) {
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let raw = match read_request(&mut s) {
        Ok(raw) => raw,
        Err((code, body)) => {
            respond(&mut s, code, body);
            return;
        }
    };
    let req = parse_request(&raw);
    // Any web page can make the browser send requests to 127.0.0.1. Browsers mark
    // those with these headers; curl, AeroSpace hooks and `wingbar <cmd>` don't.
    if req.from_browser {
        respond(&mut s, "403 Forbidden", "forbidden\n");
        return;
    }
    let r = parse_route(reg, &req.path);
    let (code, body) = route(reg, &r);
    respond(&mut s, code, &body);
    if r == Route::Quit {
        std::process::exit(0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Refresh,
    Toggle,
    Enable,
    Disable,
}

impl Action {
    fn parse(s: &str) -> Option<Action> {
        Some(match s {
            "refresh" => Action::Refresh,
            "toggle" => Action::Toggle,
            "enable" => Action::Enable,
            "disable" => Action::Disable,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq)]
enum Route {
    Status,
    Widgets,
    Reload,
    Quit,
    RefreshSpaces,
    RefreshAll,
    /// A widget by (wingbar) key.
    Widget(String, Action),
    /// A simple-bar user widget by number.
    UserWidget(u32, Action),
    NotFound,
}

/// `reg` translates simple-bar's widget names to keys.
fn parse_route(reg: &Registry, path: &str) -> Route {
    let p: Vec<&str> = path.trim_matches('/').split('/').filter(|x| !x.is_empty()).collect();
    match p.as_slice() {
        ["status"] => Route::Status,
        ["widgets"] => Route::Widgets,
        ["reload"] => Route::Reload,
        ["quit"] => Route::Quit,
        ["aerospace" | "yabai", ..] | ["spaces", ..] => Route::RefreshSpaces,
        ["widget", "user-widget", action, n] => match (Action::parse(action), n.parse::<u32>()) {
            (Some(a), Ok(n)) => Route::UserWidget(n, a),
            _ => Route::NotFound,
        },
        ["widget", name, action] => match Action::parse(action) {
            Some(a) => Route::Widget(reg.key_for_name(name), a),
            None => Route::NotFound,
        },
        ["refresh"] => Route::RefreshAll,
        _ => Route::NotFound,
    }
}

fn act(reg: &Registry, key: &str, action: Action) {
    match action {
        Action::Refresh => {
            if reg.get(key).is_some_and(|w| w.kind() == widgets::Kind::Spaces) {
                refresh_spaces(reg);
            } else {
                sched::refresh(key);
            }
        }
        Action::Toggle | Action::Enable | Action::Disable => {
            {
                let mut st = state::lock();
                let off = match action {
                    Action::Enable => false,
                    Action::Disable => true,
                    _ => !st.disabled.contains(key),
                };
                if off {
                    st.disabled.insert(key.to_string());
                } else {
                    st.disabled.remove(key);
                }
            }
            state::request_redraw();
            sched::refresh(key);
        }
    }
}

/// The workspaces changed: re-read them, and the focused window's title.
fn refresh_spaces(reg: &Registry) {
    for k in reg.keys_of_type(WidgetType::Spaces).iter().chain(&reg.keys_of_type(WidgetType::Title)) {
        sched::refresh(k);
    }
}

fn route(reg: &Registry, r: &Route) -> (&'static str, String) {
    let ok = ("200 OK", "ok\n".to_string());
    let nf = ("404 Not Found", "unknown route\n".to_string());
    match r {
        Route::Status => {
            let mut out = status_text(&state::lock());
            out.push_str(&settings_text(&crate::config::status()));
            ("200 OK", out)
        }
        Route::Widgets => ("200 OK", widgets_json(reg, &state::lock(), &crate::config::status().path)),
        // the settings applied in place; the answer says what changed, or why nothing did
        Route::Reload => match crate::reload::reload() {
            Ok(o) => ("200 OK", o.lines().iter().map(|l| format!("{l}\n")).collect()),
            Err(e) => ("422 Unprocessable Entity", format!("{e}\nstill using the previous settings\n")),
        },
        Route::Quit => ok,
        Route::RefreshSpaces => {
            refresh_spaces(reg);
            ok
        }
        Route::UserWidget(n, a) => match reg.key_for_user_widget(*n) {
            Some(k) => {
                act(reg, &k, *a);
                ok
            }
            None => nf,
        },
        Route::Widget(k, a) => {
            act(reg, k, *a);
            ok
        }
        Route::RefreshAll => {
            sched::refresh_all();
            ok
        }
        Route::NotFound => nf,
    }
}

fn status_text(st: &state::State) -> String {
    let mut keys: Vec<&String> =
        st.items.keys().chain(st.errors.keys().filter(|k| !st.items.contains_key(*k))).collect();
    keys.sort();
    let mut out = String::new();
    out.push_str(&format!(
        "spaces: {}\n",
        st.spaces
            .iter()
            .map(|s| if s.focused { format!("[{}]", s.name) } else { s.name.clone() })
            .collect::<Vec<_>>()
            .join(" ")
    ));
    out.push_str(&format!("title: {}\n", st.title));
    for k in keys {
        let (level, text) = st.items.get(k).map(|it| (it.level.as_str(), it.text.as_str())).unwrap_or(("", ""));
        let off = if st.disabled.contains(k) { " (hidden)" } else { "" };
        let err = st.errors.get(k).map(|e| format!("  [error: {e}]")).unwrap_or_default();
        out.push_str(&format!("{k}: {level} {text}{off}{err}\n"));
    }
    out
}

/// `/widgets`: the settings file in use and, for each running widget (in
/// wing order) and each widget with a recorded error, what it shows and its
/// last error.
fn widgets_json(reg: &Registry, st: &state::State, settings: &std::path::Path) -> String {
    let mut widgets = serde_json::Map::new();
    let mut keys: Vec<&str> = reg.keys().collect();
    let mut stray: Vec<&str> = st.errors.keys().map(String::as_str).filter(|k| !keys.contains(k)).collect();
    stray.sort();
    keys.extend(stray);
    for k in keys {
        let item = st.items.get(k);
        widgets.insert(
            k.to_string(),
            serde_json::json!({
                "running": reg.get(k).is_some(),
                "showing": reg.get(k).map(|w| crate::widgets::showing(st, k, w.kind())),
                "level": item.map(|it| it.level.as_str()),
                "text": item.map(|it| it.text.as_str()),
                "disabled": st.disabled.contains(k),
                "error": st.errors.get(k),
            }),
        );
    }
    let v = serde_json::json!({ "settings": settings.display().to_string(), "widgets": widgets });
    format!("{v}\n")
}

/// The settings file, and what reading it last came to.
fn settings_text(s: &crate::config::LoadStatus) -> String {
    let mut out = match &s.error {
        None => format!("settings: {} ok\n", s.path.display()),
        Some(e) => format!("settings: {} error: {e} (still using the previous settings)\n", s.path.display()),
    };
    for r in &s.restart {
        out.push_str(&format!("settings: restart wingbar to apply: {r}\n"));
    }
    for w in &s.warnings {
        out.push_str(&format!("settings: warning: {w}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Item, Space, State};
    use crate::widgets::fixture_registry;

    #[test]
    fn plain_clients_are_accepted() {
        // what `wingbar <cmd>` sends
        let cli = parse_request("GET /status HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n");
        assert_eq!(cli, Request { path: "/status".into(), from_browser: false });
        // what curl sends (the AeroSpace hook, sb_refresh)
        let curl = parse_request(
            "GET /aerospace/spaces/refresh?x=1 HTTP/1.1\r\nHost: 127.0.0.1:7776\r\nUser-Agent: curl/8.7.1\r\nAccept: */*\r\n\r\n",
        );
        assert_eq!(curl, Request { path: "/aerospace/spaces/refresh".into(), from_browser: false });
        // bare `echo "GET /quit" | nc`
        assert_eq!(parse_request("GET /quit\n"), Request { path: "/quit".into(), from_browser: false });
    }

    #[test]
    fn browser_requests_are_flagged() {
        for header in [
            "Origin: https://evil.example",
            "origin: null",
            "Sec-Fetch-Site: cross-site",
            "sec-fetch-mode: no-cors",
            "SEC-FETCH-SITE:none",
        ] {
            let raw = format!("GET /quit HTTP/1.1\r\nHost: 127.0.0.1:7776\r\n{header}\r\nAccept: */*\r\n\r\n");
            assert!(parse_request(&raw).from_browser, "{header}");
        }
    }

    #[test]
    fn only_headers_count() {
        // a header name that merely contains "origin", and "Origin:" in the body, don't count
        let raw = "POST /refresh HTTP/1.1\r\nX-Original: 1\r\nReferrer-Origin-Note: a\r\n\r\nOrigin: x\r\n";
        assert_eq!(parse_request(raw), Request { path: "/refresh".into(), from_browser: false });
        // the request line itself is not a header
        assert!(!parse_request("GET /Origin:x HTTP/1.1\r\n\r\n").from_browser);
    }

    #[test]
    fn garbage_requests() {
        assert_eq!(parse_request(""), Request { path: "/".into(), from_browser: false });
        assert_eq!(parse_request("GET"), Request { path: "/".into(), from_browser: false });
        assert_eq!(parse_route(&fixture_registry(), &parse_request("\u{fffd}\u{fffd}").path), Route::NotFound);
    }

    #[test]
    fn documented_routes() {
        use Action::*;
        let reg = fixture_registry();
        let cases = [
            ("/status", Route::Status),
            ("/widgets", Route::Widgets),
            ("/widgets/", Route::Widgets),
            ("/reload", Route::Reload),
            ("/quit", Route::Quit),
            ("/refresh", Route::RefreshAll),
            ("/aerospace/spaces/refresh", Route::RefreshSpaces),
            ("/yabai/spaces/refresh", Route::RefreshSpaces),
            ("/spaces", Route::RefreshSpaces),
            ("/widget/clickup/refresh", Route::Widget("clickup".into(), Refresh)),
            ("/widget/clickup/toggle", Route::Widget("clickup".into(), Toggle)),
            ("/widget/clickup/enable", Route::Widget("clickup".into(), Enable)),
            ("/widget/clickup/disable/", Route::Widget("clickup".into(), Disable)),
            ("/widget/date-display/refresh", Route::Widget("date".into(), Refresh)),
            ("/widget/process/toggle", Route::Widget("title".into(), Toggle)),
            ("/widget/window/refresh", Route::Widget("title".into(), Refresh)),
            ("/widget/yabai/disable", Route::Widget("spaces".into(), Disable)),
            // names as 0.2 accepted them (an sbw group "ClickUp"): keys are lowercase
            ("/widget/ClickUp/refresh", Route::Widget("clickup".into(), Refresh)),
            ("/widget/CLICKUP/toggle", Route::Widget("clickup".into(), Toggle)),
            ("/widget/Process/refresh", Route::Widget("title".into(), Refresh)),
            ("/widget/user-widget/refresh/3", Route::UserWidget(3, Refresh)),
            ("/widget/user-widget/disable/12", Route::UserWidget(12, Disable)),
        ];
        for (path, want) in cases {
            assert_eq!(parse_route(&reg, path), want, "{path}");
        }
    }

    #[test]
    fn unknown_routes() {
        let reg = fixture_registry();
        for path in [
            "/",
            "",
            "/nope",
            "/status/extra",
            "/widgets/cpu",
            "/widget/clickup",
            "/widget/clickup/explode",
            "/widget/user-widget/refresh/x",
            "/widget/user-widget/refresh/-1",
            "/widget/user-widget/explode/3",
            "/widget/a/b/c/d",
        ] {
            assert_eq!(parse_route(&reg, path), Route::NotFound, "{path}");
        }
    }

    #[test]
    fn route_responses() {
        // no scheduler runs in tests, so the refreshes are no-ops
        let reg = fixture_registry();
        assert_eq!(route(&reg, &Route::NotFound), ("404 Not Found", "unknown route\n".to_string()));
        assert_eq!(route(&reg, &Route::RefreshAll), ("200 OK", "ok\n".to_string()));
        assert_eq!(route(&reg, &Route::RefreshSpaces), ("200 OK", "ok\n".to_string()));
        assert_eq!(route(&reg, &Route::Quit).0, "200 OK");
        assert_eq!(route(&reg, &Route::Widget("test-refresh".into(), Action::Refresh)).0, "200 OK");
        assert!(route(&reg, &Route::Status).1.starts_with("spaces: "));
        // user widget numbers: the fixture's 1 and 2 are the clickup script, 4 is inactive
        assert_eq!(route(&reg, &Route::UserWidget(2, Action::Refresh)).0, "200 OK");
        assert_eq!(route(&reg, &Route::UserWidget(4, Action::Refresh)).0, "404 Not Found");
        assert_eq!(route(&reg, &Route::UserWidget(99, Action::Refresh)).0, "404 Not Found");
    }

    #[test]
    fn toggle_enable_disable() {
        let reg = fixture_registry();
        let k = "server-test-toggle";
        let off = || state::lock().disabled.contains(k);
        route(&reg, &Route::Widget(k.into(), Action::Toggle));
        assert!(off());
        route(&reg, &Route::Widget(k.into(), Action::Toggle));
        assert!(!off());
        route(&reg, &Route::Widget(k.into(), Action::Disable));
        route(&reg, &Route::Widget(k.into(), Action::Disable));
        assert!(off());
        route(&reg, &Route::Widget(k.into(), Action::Enable));
        assert!(!off());
    }

    /// One request through `handle` on an ephemeral loopback port (never the bar's own).
    fn exchange(raw: &[u8]) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || handle(listener.accept().unwrap().0, &fixture_registry()));
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(raw).unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        server.join().unwrap();
        out
    }

    #[test]
    fn handle_answers_plain_clients_without_cors() {
        let out = exchange(b"GET /refresh HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n");
        assert!(out.starts_with("HTTP/1.1 200 OK\r\n"), "{out}");
        assert!(out.ends_with("\r\n\r\nok\n"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("access-control"), "{out}");
    }

    #[test]
    fn handle_rejects_browsers() {
        let k = "server-test-browser";
        let raw = format!("GET /widget/{k}/disable HTTP/1.1\r\nHost: 127.0.0.1\r\nSec-Fetch-Site: cross-site\r\n\r\n");
        let out = exchange(raw.as_bytes());
        assert!(out.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{out}");
        assert!(!state::lock().disabled.contains(k), "a rejected request must not act");
    }

    #[test]
    fn handle_refuses_headers_that_stall_midway() {
        // The browser header arrives after a pause longer than one read timeout;
        // the request must be refused, not acted on without seeing that header.
        let k = "server-test-stall";
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || handle(listener.accept().unwrap().0, &fixture_registry()));
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(format!("GET /widget/{k}/disable HTTP/1.1\r\nHost: x\r\n").as_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(700));
        let _ = c.write_all(b"Sec-Fetch-Site: cross-site\r\n\r\n");
        let mut out = String::new();
        let _ = c.read_to_string(&mut out);
        server.join().unwrap();
        assert!(out.starts_with("HTTP/1.1 408 "), "{out}");
        assert!(!state::lock().disabled.contains(k), "a stalled request must not act");
    }

    #[test]
    fn handle_accepts_request_without_blank_line_when_client_closes() {
        // `echo "GET /refresh" | nc 127.0.0.1 7776` style: no end of headers, then EOF.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || handle(listener.accept().unwrap().0, &fixture_registry()));
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(b"GET /refresh\n").unwrap();
        c.shutdown(std::net::Shutdown::Write).unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).unwrap();
        server.join().unwrap();
        assert!(out.starts_with("HTTP/1.1 200 OK\r\n"), "{out}");
    }

    #[test]
    fn handle_rejects_oversized_headers() {
        // exactly MAX_REQUEST bytes and no end of headers (sending more would leave
        // unread bytes, and the close would reset the connection before we read the reply)
        let mut raw = b"GET /refresh HTTP/1.1\r\nX-Pad: ".to_vec();
        raw.resize(MAX_REQUEST, b'a');
        let out = exchange(&raw);
        assert!(out.starts_with("HTTP/1.1 431 "), "{out}");
    }

    #[test]
    fn status_lists_items_and_errors() {
        let grey = crate::config::Rgba(0.5, 0.5, 0.5, 1.0);
        let item = |level: &str, text: &str| Item {
            icon: None,
            text: text.into(),
            icon_color: grey,
            text_color: grey,
            bold: false,
            level: level.into(),
            trail: None,
        };
        let mut st = State {
            spaces: vec![
                Space { name: "1".into(), focused: false, screen: 1, apps: vec![] },
                Space { name: "2".into(), focused: true, screen: 1, apps: vec![] },
            ],
            title: "Ghostty / zsh".into(),
            ..State::default()
        };
        st.items.insert("clickup".into(), item("warn", "3 tasks"));
        st.items.insert("time".into(), item("", "12:00"));
        st.disabled.insert("time".into());
        st.errors.insert("clickup".into(), "timed out after 10s".into());
        st.errors.insert("adb".into(), "exited with status 1".into());
        assert_eq!(
            status_text(&st),
            "spaces: 1 [2]\n\
             title: Ghostty / zsh\n\
             adb:    [error: exited with status 1]\n\
             clickup: warn 3 tasks  [error: timed out after 10s]\n\
             time:  12:00 (hidden)\n"
        );
    }

    #[test]
    fn widgets_lists_what_each_shows_and_its_error() {
        let reg = fixture_registry();
        let grey = crate::config::Rgba(0.5, 0.5, 0.5, 1.0);
        let mut st = State::default();
        st.items.insert(
            "clickup".into(),
            Item {
                icon: None,
                text: "3 tasks".into(),
                icon_color: grey,
                text_color: grey,
                bold: false,
                level: "warn".into(),
                trail: None,
            },
        );
        st.errors.insert("clickup".into(), "timed out after 10s".into());
        st.errors.insert("gone".into(), "exited with status 1".into());
        st.disabled.insert("time".into());
        let out = widgets_json(&reg, &st, std::path::Path::new("/tmp/x/config.json"));
        assert!(out.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["settings"], "/tmp/x/config.json");
        let w = v["widgets"].as_object().unwrap();
        let keys: Vec<&String> = w.keys().collect();
        let reg_keys: Vec<&str> = reg.keys().collect();
        assert_eq!(keys.len(), reg_keys.len() + 1, "every running widget, and the stray error");
        assert_eq!(keys[..reg_keys.len()], reg_keys, "in wing order");
        assert_eq!(
            w["clickup"],
            serde_json::json!({
                "running": true, "showing": true, "level": "warn", "text": "3 tasks", "disabled": false,
                "error": "timed out after 10s"
            })
        );
        assert_eq!(w["time"]["disabled"], true);
        assert_eq!(w["time"]["showing"], false, "a disabled widget takes no room");
        assert_eq!(w["gone"]["showing"], serde_json::Value::Null);
        assert_eq!(w["time"]["error"], serde_json::Value::Null);
        assert_eq!(w["gone"]["running"], false);
        assert_eq!(w["gone"]["error"], "exited with status 1");
    }

    #[test]
    fn widgets_route_answers_json_and_refuses_browsers() {
        let out = exchange(b"GET /widgets HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n");
        assert!(out.starts_with("HTTP/1.1 200 OK\r\n"), "{out}");
        assert!(out.contains("Content-Type: application/json\r\n"), "{out}");
        let body = out.split_once("\r\n\r\n").unwrap().1;
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        assert!(v["widgets"].is_object(), "{body}");
        let out = exchange(b"GET /widgets HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://evil.example\r\n\r\n");
        assert!(out.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{out}");
        assert!(!out.contains("widgets"), "{out}");
    }

    #[test]
    fn status_says_how_the_settings_read() {
        use crate::config::LoadStatus;
        let path = std::path::PathBuf::from("/tmp/x/config.json");
        let ok = LoadStatus { path: path.clone(), ..LoadStatus::default() };
        assert_eq!(settings_text(&ok), "settings: /tmp/x/config.json ok\n");
        let bad = LoadStatus {
            path,
            error: Some("/tmp/x/config.json: not valid JSON at line 3".into()),
            warnings: vec!["widgets.x.every: expected a duration".into()],
            restart: vec!["bar.port is now 7000 (the bar still listens on 7776)".into()],
        };
        assert_eq!(
            settings_text(&bad),
            "settings: /tmp/x/config.json error: /tmp/x/config.json: not valid JSON at line 3 (still using the previous settings)\n\
             settings: restart wingbar to apply: bar.port is now 7000 (the bar still listens on 7776)\n\
             settings: warning: widgets.x.every: expected a duration\n"
        );
    }
}
