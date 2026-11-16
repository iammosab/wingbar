//! The date and the time, redrawn just after each minute (or second) boundary.

use super::{Button, Failure, Show, Widget, has_action, item, run_action};
use crate::config::{Rgba, Theme, TimeOptions};
use crate::sched::Every;
use crate::sys;

pub struct Clock {
    key: String,
    /// strftime(3) format.
    format: &'static str,
    every: Every,
    color: Rgba,
    /// What a left click runs (a shell command; empty for nothing).
    click: String,
}

impl Clock {
    pub fn date(key: &str, click: &str, theme: &Theme) -> Clock {
        Clock { key: key.into(), format: "%a %-d %b", every: Every::Minute, color: theme.text, click: click.into() }
    }

    pub fn time(key: &str, o: &TimeOptions, theme: &Theme) -> Clock {
        let format = match (o.hour12, o.seconds) {
            (false, false) => "%H:%M",
            (false, true) => "%H:%M:%S",
            (true, false) => "%-I:%M %p",
            (true, true) => "%-I:%M:%S %p",
        };
        let every = if o.seconds { Every::Second } else { Every::Minute };
        Clock { key: key.into(), format, every, color: theme.text, click: o.click.clone() }
    }
}

impl Widget for Clock {
    fn key(&self) -> &str {
        &self.key
    }
    fn schedule(&self) -> Every {
        self.every
    }
    fn poll(&self) -> Result<Show, Failure> {
        let mut it = item(None, sys::strftime(self.format), self.color, self.color);
        it.bold = true;
        Ok(Show::Item(it))
    }
    fn clickable(&self) -> bool {
        has_action(&self.click)
    }
    fn click(&self, b: Button, _: Option<&str>) -> bool {
        b == Button::Left && run_action(&self.click)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_format_follows_the_settings() {
        let t = crate::config::fixture().theme;
        let time = |hour12, seconds| Clock::time("time", &TimeOptions { hour12, seconds, click: String::new() }, &t);
        assert_eq!((time(false, false).format, time(false, false).every), ("%H:%M", Every::Minute));
        assert_eq!((time(true, false).format, time(true, true).format), ("%-I:%M %p", "%-I:%M:%S %p"));
        assert_eq!((time(false, true).format, time(false, true).every), ("%H:%M:%S", Every::Second));
        let date = Clock::date("date", "open -a Calendar", &t);
        let Ok(Show::Item(it)) = date.poll() else { panic!("the date always shows") };
        assert!(it.bold && !it.text.is_empty() && it.icon.is_none());
        assert!(date.clickable() && !time(true, false).clickable());
        assert!(!Clock::date("d2", "", &t).clickable(), "no click command");
    }
}
