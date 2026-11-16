//! Bundled icons, drawn from SVG:
//!
//! - simple-bar's icon library (MIT, see `SIMPLE-BAR-LICENSE`), generated into
//!   `simple_bar.rs` by `scripts/simplebar-icons.py`;
//! - `extra.rs`: the line-art icons of the "wings" theme wingbar replaced, for
//!   what simple-bar has no icon for or the theme drew differently (`Memory`,
//!   `Storage`, `Ethernet`, `Speaker1`–`3`, `SpeakerMuted`, one per prayer).
//!
//! An icon name that starts with a capital letter (`CPU`, `ClickUp`, `MicOff`)
//! is one of these; any other name is an SF Symbol (`cpu`, `mic.slash`). SF
//! Symbol names are all lowercase, so the two never collide.

mod extra;
mod simple_bar;

use simple_bar::{APPS, ICONS};

/// Whether `name` names a bundled icon (rather than an SF Symbol).
pub fn is_bundled(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
}

/// simple-bar's icon called `name`: (viewBox width, height, SVG markup).
fn simple_bar(name: &str) -> Option<(u16, u16, &'static str)> {
    let i = ICONS.binary_search_by(|(n, ..)| n.cmp(&name)).ok()?;
    let (_, w, h, body) = ICONS[i];
    Some((w, h, body))
}

/// The theme's icon called `name`: a whole SVG document drawn in `black`.
fn theme(name: &str) -> Option<&'static str> {
    let i = extra::ICONS.binary_search_by(|(n, _)| n.cmp(&name)).ok()?;
    Some(extra::ICONS[i].1)
}

/// Whether a bundled icon is called `name`.
pub fn exists(name: &str) -> bool {
    simple_bar(name).is_some() || theme(name).is_some()
}

/// Every bundled icon's name.
#[cfg(test)]
pub fn names() -> impl Iterator<Item = &'static str> {
    ICONS.iter().map(|(n, ..)| *n).chain(extra::ICONS.iter().map(|(n, _)| *n))
}

/// simple-bar's icon for an app, by its name as macOS shows it ("Google
/// Chrome"); `None` for an app it has none for (simple-bar drew a generic
/// window there, which the theme hid).
pub fn for_app(app: &str) -> Option<&'static str> {
    let i = APPS.binary_search_by(|(a, _)| a.cmp(&app)).ok()?;
    Some(APPS[i].1).filter(|icon| *icon != "Default")
}

/// A standalone SVG document for the bundled icon `name`, in `color`
/// (`#rrggbb`) at `opacity`, `px` points tall. Parts of a simple-bar icon that
/// carry their own colour keep it.
pub fn svg(name: &str, color: &str, opacity: f64, px: f64) -> Option<String> {
    if let Some(doc) = theme(name) {
        let doc = doc.replace("\"black\"", &format!("\"{color}\""));
        let sized = format!("<svg width=\"{px:.2}\" height=\"{px:.2}\" opacity=\"{opacity:.3}\" ");
        return Some(doc.replacen("<svg ", &sized, 1));
    }
    let (w, h, body) = simple_bar(name)?;
    let width = px * f64::from(w) / f64::from(h);
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{width:.2}\" height=\"{px:.2}\" \
         fill=\"{color}\" fill-opacity=\"{opacity:.3}\">{body}</svg>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_so_lookups_work() {
        assert!(ICONS.windows(2).all(|p| p[0].0 < p[1].0), "ICONS sorted, no duplicates");
        assert!(extra::ICONS.windows(2).all(|p| p[0].0 < p[1].0), "theme icons sorted");
        assert!(extra::ICONS.iter().all(|(n, _)| simple_bar(n).is_none()), "theme icons don't shadow simple-bar's");
        assert!(APPS.windows(2).all(|p| p[0].0 < p[1].0), "APPS sorted, no duplicates");
        assert!(APPS.iter().all(|(_, icon)| exists(icon)), "every app's icon exists");
        assert!(names().all(is_bundled), "bundled names start with a capital");
    }

    #[test]
    fn lookups() {
        for n in ["CPU", "ClickUp", "Claude", "MicOn", "MicOff", "GitHub", "AndroidStudio", "Wifi", "WifiOff"] {
            assert!(simple_bar(n).is_some(), "{n}");
        }
        for n in ["Memory", "Storage", "Ethernet", "Speaker1", "SpeakerMuted", "Fajr", "Dohr", "Ishae"] {
            assert!(theme(n).is_some(), "{n}");
        }
        assert!(!exists("cpu"), "lowercase is an SF Symbol");
        assert!(!is_bundled("mic.slash") && is_bundled("MicOff"));
        assert_eq!(for_app("Ghostty"), Some("Ghostty"));
        assert_eq!(for_app("Google Chrome"), Some("GoogleChrome"));
        assert_eq!(for_app("cmux"), None, "no glyph: nothing rather than a generic window");
    }

    #[test]
    fn svg_documents() {
        let s = svg("CPU", "#ff0000", 1.0, 14.0).unwrap();
        assert!(s.starts_with("<svg ") && s.ends_with("</svg>"), "{s}");
        assert!(s.contains("width=\"14.00\" height=\"14.00\"") && s.contains("fill=\"#ff0000\""), "{s}");
        assert!(!s.contains('{') && !s.contains("className"), "plain SVG, no JSX left: {s}");
        let t = svg("Storage", "#00ff00", 0.5, 16.0).unwrap();
        assert!(t.starts_with("<svg width=\"16.00\" height=\"16.00\" opacity=\"0.500\" xmlns="), "{t}");
        assert!(t.contains("stroke=\"#00ff00\"") && !t.contains("black"), "the tint replaces black: {t}");
        assert!(svg("NoSuchIcon", "#fff", 1.0, 14.0).is_none());
    }
}
