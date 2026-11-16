//! Workspaces from AeroSpace or yabai, and switching between them.

use crate::config::{self, IconSet, WidgetType, Wm};
use crate::sched::{output, run_program, spawn_shell};
use crate::state::{self, Space};
use std::time::Duration;

const T: Duration = Duration::from_secs(3);

/// Read the workspaces into the state. Their output is used even when the CLI
/// fails (as before); the first failure is returned for the widget's error.
pub fn refresh_spaces() -> Result<(), String> {
    let cfg = config::current();
    let mut err = None;
    // app icons in the workspaces (simple-bar's look) need each one's windows
    let apps = cfg.theme.icons == IconSet::SimpleBar;
    let spaces = match &cfg.wm {
        Wm::Aerospace(bin) => aerospace(bin, cfg.bar.hide_empty_spaces, &mut err).map(|mut s| {
            if apps
                && let Some(out) =
                    cli(bin, &["list-windows", "--all", "--format", "%{workspace}|%{app-name}"], &mut err)
            {
                add_apps(&mut s, out.lines().filter_map(|l| l.split_once('|')));
            }
            s
        }),
        Wm::Yabai(bin) => yabai(bin, cfg.bar.hide_empty_spaces, apps, &mut err),
        Wm::None => return Ok(()),
    };
    if let Some(mut s) = spaces {
        s.retain(|sp| !cfg.bar.space_exclusions.contains(&sp.name));
        state::set_spaces(s);
    }
    err.map_or(Ok(()), Err)
}

/// Run the WM's CLI: what it printed (also after a failure), or `None` if it
/// never ran. The first failure is kept in `err` as "<program> <command>: <why>".
fn cli(bin: &str, args: &[&str], err: &mut Option<String>) -> Option<String> {
    let r = run_program(bin, args, T);
    if let Err(e) = &r {
        let name = std::path::Path::new(bin).file_name().map_or(bin.into(), |n| n.to_string_lossy());
        err.get_or_insert_with(|| format!("{name} {}: {e}", args.first().unwrap_or(&"")));
    }
    output(r)
}

fn aerospace(bin: &str, hide_empty: bool, err: &mut Option<String>) -> Option<Vec<Space>> {
    let fmt = "%{workspace}|%{workspace-is-focused}|%{monitor-appkit-nsscreen-screens-id}";
    let listing = if hide_empty {
        cli(bin, &["list-workspaces", "--monitor", "all", "--empty", "no", "--format", fmt], err)?
    } else {
        cli(bin, &["list-workspaces", "--all", "--format", fmt], err)?
    };
    let mut spaces: Vec<Space> = listing
        .lines()
        .filter_map(|l| {
            let mut p = l.split('|');
            Some(Space {
                name: p.next()?.trim().to_string(),
                focused: p.next()? == "true",
                screen: p.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0),
                apps: Vec::new(),
            })
        })
        .filter(|s| !s.name.is_empty())
        .collect();
    if hide_empty && !spaces.iter().any(|s| s.focused) {
        // the focused workspace is empty: still show it, in its place
        let f = cli(bin, &["list-workspaces", "--focused", "--format", fmt], err)?;
        if let Some(l) = f.lines().next() {
            let mut p = l.split('|');
            let name = p.next().unwrap_or("").trim().to_string();
            let _ = p.next();
            let screen = p.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
            if !name.is_empty() {
                spaces.push(Space { name, focused: true, screen, apps: Vec::new() });
                spaces.sort_by(|a, b| natural(&a.name, &b.name));
            }
        }
    }
    Some(spaces)
}

/// Give each workspace the apps of its windows (`(workspace, app)` pairs), in
/// order, each once.
fn add_apps<'a>(spaces: &mut [Space], windows: impl Iterator<Item = (&'a str, &'a str)>) {
    for (ws, app) in windows {
        let (ws, app) = (ws.trim(), app.trim());
        if let Some(s) = spaces.iter_mut().find(|s| s.name == ws)
            && !app.is_empty()
            && !s.apps.iter().any(|a| a == app)
        {
            s.apps.push(app.to_string());
        }
    }
}

/// `(space index, app)` for each window `yabai -m query --windows` lists.
fn yabai_windows(out: &str) -> Vec<(i64, String)> {
    let Ok(serde_json::Value::Array(ws)) = serde_json::from_str::<serde_json::Value>(out) else { return vec![] };
    ws.iter().filter_map(|w| Some((w.get("space")?.as_i64()?, w.get("app")?.as_str()?.to_string()))).collect()
}

fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<i64>(), b.parse::<i64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        _ => a.cmp(b),
    }
}

fn yabai(bin: &str, hide_empty: bool, apps: bool, err: &mut Option<String>) -> Option<Vec<Space>> {
    let out = cli(bin, &["-m", "query", "--spaces"], err)?;
    let v: serde_json::Value = match serde_json::from_str(&out) {
        Ok(v) => v,
        Err(e) => {
            err.get_or_insert_with(|| format!("yabai -m query --spaces: unreadable output ({e})"));
            return None;
        }
    };
    let mut index_names = Vec::new();
    let mut spaces: Vec<Space> = v
        .as_array()?
        .iter()
        .filter_map(|s| {
            let index = s.get("index")?.as_i64()?;
            let label = s.get("label").and_then(|l| l.as_str()).unwrap_or("");
            let focused = s.get("has-focus").and_then(|x| x.as_bool()).unwrap_or(false);
            let empty = s.get("windows").and_then(|w| w.as_array()).map(|w| w.is_empty()).unwrap_or(true);
            if hide_empty && empty && !focused {
                return None;
            }
            let name = if label.is_empty() { index.to_string() } else { label.to_string() };
            index_names.push((index, name.clone()));
            Some(Space {
                name,
                focused,
                screen: s.get("display").and_then(|d| d.as_u64()).unwrap_or(0) as usize,
                apps: Vec::new(),
            })
        })
        .collect();
    if apps && let Some(out) = cli(bin, &["-m", "query", "--windows"], err) {
        let windows = yabai_windows(&out);
        let named = windows.iter().filter_map(|(i, app)| {
            let name = index_names.iter().find(|(n, _)| n == i)?;
            Some((name.1.as_str(), app.as_str()))
        });
        add_apps(&mut spaces, named);
    }
    Some(spaces)
}

pub fn focus(name: &str) {
    let safe: String = name.chars().filter(|c| c.is_alphanumeric() || "-_.".contains(*c)).collect();
    match &config::current().wm {
        Wm::Aerospace(bin) => spawn_shell(&format!("'{bin}' workspace '{safe}'")),
        Wm::Yabai(bin) => spawn_shell(&format!("'{bin}' -m space --focus '{safe}'")),
        Wm::None => {}
    }
    crate::widgets::refresh_type_in(WidgetType::Spaces, 60);
    crate::widgets::refresh_type_in(WidgetType::Title, 120);
}

/// "App / Title" of the focused window when Accessibility isn't allowed (uses the WM).
pub fn focused_title_from_wm() -> Option<(String, String)> {
    match &config::current().wm {
        Wm::Aerospace(bin) => {
            let o =
                output(run_program(bin, &["list-windows", "--focused", "--format", "%{app-name}|%{window-title}"], T))?;
            let l = o.lines().next()?;
            let (a, t) = l.split_once('|')?;
            Some((a.to_string(), t.to_string()))
        }
        Wm::Yabai(bin) => {
            let o = output(run_program(bin, &["-m", "query", "--windows", "--window"], T))?;
            let v: serde_json::Value = serde_json::from_str(&o).ok()?;
            Some((v.get("app")?.as_str()?.to_string(), v.get("title")?.as_str()?.to_string()))
        }
        Wm::None => None,
    }
}

/// Next / previous workspace (scroll over the numbers).
pub fn cycle(next: bool) {
    let dir = if next { "next" } else { "prev" };
    match &config::current().wm {
        Wm::Aerospace(bin) => spawn_shell(&format!("'{bin}' workspace --wrap-around {dir}")),
        Wm::Yabai(bin) => spawn_shell(&format!(
            "'{bin}' -m space --focus {dir} || '{bin}' -m space --focus {}",
            if next { "first" } else { "last" }
        )),
        Wm::None => {}
    }
    crate::widgets::refresh_type_in(WidgetType::Spaces, 60);
    crate::widgets::refresh_type_in(WidgetType::Title, 120);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn apps_per_workspace_in_order_each_once() {
        let sp = |n: &str| Space { name: n.into(), focused: false, screen: 0, apps: vec![] };
        let mut spaces = vec![sp("1"), sp("2")];
        let listing = "2|Finder\n2|Finder\n1|Zen\n2|cmux\n9|Elsewhere\n2| \n";
        add_apps(&mut spaces, listing.lines().filter_map(|l| l.split_once('|')));
        assert_eq!(spaces[0].apps, ["Zen"]);
        assert_eq!(spaces[1].apps, ["Finder", "cmux"], "duplicates and blank names left out");
        let y = r#"[{"space": 2, "app": "Safari"}, {"space": 1, "app": "Mail"}, {"app": "No space"}]"#;
        assert_eq!(yabai_windows(y), [(2, "Safari".to_string()), (1, "Mail".to_string())]);
        assert!(yabai_windows("not json").is_empty());
    }

    #[test]
    fn natural_ordering() {
        assert_eq!(natural("2", "10"), Less);
        assert_eq!(natural("10", "2"), Greater);
        assert_eq!(natural("3", "3"), Equal);
        assert_eq!(natural("9", "A"), Less, "numbers before names");
        assert_eq!(natural("A", "9"), Greater);
        assert_eq!(natural("B", "A"), Greater);
        let mut v = vec!["10", "B", "2", "A", "1", "-1"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, ["-1", "1", "2", "10", "A", "B"]);
    }
}
