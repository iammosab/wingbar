//! Now playing in Spotify or Music. macOS broadcasts every change (the bar
//! listens, see `ui::install_observers`); the poll only asks at start and
//! every five minutes.

use super::{Button, Failure, Show, Widget, item};
use crate::config::{self, Rgba, Theme, WidgetType};
use crate::sched::{self, Every};
use crate::state;
use crate::sys;
use std::time::Duration;

pub struct Player {
    /// The widget type that shows it.
    pub ty: WidgetType,
    /// The app's process name, and its name for AppleScript.
    pub app: &'static str,
    /// The distributed notification it posts on every change.
    pub notification: &'static str,
}

pub const PLAYERS: &[Player] = &[
    Player { ty: WidgetType::Spotify, app: "Spotify", notification: "com.spotify.client.PlaybackStateChanged" },
    Player { ty: WidgetType::Music, app: "Music", notification: "com.apple.Music.playerInfo" },
];

pub struct NowPlaying {
    key: String,
    app: &'static str,
    icon: &'static str,
    icon_color: Rgba,
    text_color: Rgba,
}

impl NowPlaying {
    pub fn new(key: &str, p: &Player, t: &Theme) -> NowPlaying {
        NowPlaying { key: key.into(), app: p.app, icon: playing_icon(t), icon_color: green(t), text_color: t.text }
    }
}

fn green(t: &Theme) -> Rgba {
    t.named.get("green").copied().unwrap_or(t.text)
}

/// The icon while playing (the widget hides otherwise).
fn playing_icon(t: &Theme) -> &'static str {
    t.icons.pick("music.note", "Playing")
}

fn present(playing: bool, name: &str, artist: &str, icon: &str, icon_color: Rgba, text_color: Rgba) -> Show {
    if !playing || name.is_empty() {
        return Show::Hide;
    }
    let text = if artist.is_empty() { name.to_string() } else { format!("{name} – {artist}") };
    let mut it = item(Some(icon), text, icon_color, text_color);
    it.level = "playing".into();
    Show::Item(it)
}

/// A player's change notification (from the main thread), for one of its widgets.
pub fn now_playing_changed(key: &str, playing: bool, name: &str, artist: &str) {
    let cfg = config::current();
    let t = &cfg.theme;
    let it = match present(playing, name, artist, playing_icon(t), green(t), t.text) {
        Show::Item(it) => Some(it),
        _ => None,
    };
    state::set_item(key, it);
}

impl Widget for NowPlaying {
    fn key(&self) -> &str {
        &self.key
    }
    fn schedule(&self) -> Every {
        Every::Ms(300_000)
    }
    fn poll(&self) -> Result<Show, Failure> {
        if !sys::process_running(self.app) {
            return Ok(Show::Hide);
        }
        let app = self.app;
        let script = format!(
            "tell application \"{app}\" to if player state is playing then return (name of current track) & \"\\n\" & (artist of current track)"
        );
        let r = sched::run_program("/usr/bin/osascript", &["-e", &script], Duration::from_secs(5));
        let error = r.as_ref().err().map(|e| format!("osascript: {e}"));
        let out = sched::output(r).unwrap_or_default();
        let mut l = out.lines();
        let name = l.next().unwrap_or("").trim();
        let artist = l.next().unwrap_or("").trim();
        let show = present(!name.is_empty(), name, artist, self.icon, self.icon_color, self.text_color);
        match error {
            None => Ok(show),
            Some(error) => Err(Failure::new(error, show)),
        }
    }
    fn clickable(&self) -> bool {
        true
    }
    /// Left click: play / pause.
    fn click(&self, b: Button, _: Option<&str>) -> bool {
        if b != Button::Left {
            return false;
        }
        let script = format!("tell application \"{}\" to playpause", self.app);
        sched::spawn_program("/usr/bin/osascript", &["-e", &script]);
        sched::refresh_in(&self.key, 400);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_playing_text() {
        let (g, w) = (Rgba(0.0, 1.0, 0.0, 1.0), Rgba(1.0, 1.0, 1.0, 1.0));
        let Show::Item(it) = present(true, "Song", "Band", "music.note", g, w) else { panic!() };
        assert_eq!((it.text.as_str(), it.level.as_str(), it.icon_color), ("Song – Band", "playing", g));
        let Show::Item(it) = present(true, "Song", "", "music.note", g, w) else { panic!() };
        assert_eq!(it.text, "Song");
        assert_eq!(present(false, "Song", "Band", "music.note", g, w), Show::Hide);
        assert_eq!(present(true, "", "Band", "music.note", g, w), Show::Hide);
    }
}
