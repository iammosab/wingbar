//! What the bar shows. Workers write here; the UI reads it when it draws.
//! A change asks the main thread for exactly one redraw (coalesced).

use crate::config::Rgba;
use dispatch2::DispatchQueue;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// SF Symbol name.
    pub icon: Option<String>,
    pub text: String,
    pub icon_color: Rgba,
    pub text_color: Rgba,
    pub bold: bool,
    /// The level that produced it (for level widgets), else "".
    pub level: String,
    /// A small icon after the text (the caffeinate cup on the battery).
    pub trail: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Space {
    pub name: String,
    pub focused: bool,
    /// 1-based index into NSScreen.screens; 0 = every screen.
    pub screen: usize,
    /// The apps with a window on it, in order, each once (read only when the
    /// bar shows app icons).
    pub apps: Vec<String>,
}

#[derive(Default)]
pub struct State {
    pub items: HashMap<String, Item>,
    pub spaces: Vec<Space>,
    pub title: String,
    pub disabled: HashSet<String>,
    pub front_pid: i32,
    pub front_name: String,
    /// The last failure of each widget's script, cleared when it next succeeds.
    /// Only `/status` shows these, so they never cause a redraw.
    pub errors: HashMap<String, String>,
    /// The last generation handed out (see `invalidate`).
    pub generation: u64,
    /// Each widget's generation: that of the widget now running under the key
    /// (0 until a reload removes the key or makes its widget anew). A widget
    /// job carries its widget's generation, and its result is dropped when
    /// that is no longer the key's (see `apply`), so a reload drops only the
    /// results of widgets it removed or replaced.
    pub generations: HashMap<String, u64>,
}

impl State {
    /// The generation of the widget now running under `key`.
    pub fn generation_of(&self, key: &str) -> u64 {
        self.generations.get(key).copied().unwrap_or(0)
    }

    /// A reload removed `key`, or made its widget anew (or added it): what a
    /// job started before now brings for it is dropped.
    pub fn invalidate(&mut self, key: &str) {
        self.generation = self.generation.wrapping_add(1);
        self.generations.insert(key.to_string(), self.generation);
    }

    /// Show `item` for `key` (`None`: nothing). Whether that changed anything.
    fn put_item(&mut self, key: &str, item: Option<Item>) -> bool {
        match item {
            Some(it) => {
                if self.items.get(key) == Some(&it) {
                    false
                } else {
                    self.items.insert(key.to_string(), it);
                    true
                }
            }
            None => self.items.remove(key).is_some(),
        }
    }

    /// A widget job's result: record its error (or clear it) and, unless
    /// `item` is `None` (keep what is shown), show `item`. Dropped (`None`)
    /// when the job is from an earlier generation of the key's widget; else
    /// whether what the bar shows changed.
    pub fn apply(
        &mut self,
        generation: u64,
        key: &str,
        error: Option<String>,
        item: Option<Option<Item>>,
    ) -> Option<bool> {
        if generation != self.generation_of(key) {
            return None;
        }
        match error {
            Some(e) => {
                self.errors.insert(key.to_string(), e);
            }
            None => {
                self.errors.remove(key);
            }
        }
        Some(item.is_some_and(|it| self.put_item(key, it)))
    }

    /// Everything known about a widget that went away.
    pub fn forget(&mut self, key: &str) {
        self.items.remove(key);
        self.errors.remove(key);
        self.disabled.remove(key);
    }
}

pub static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::default()));

pub fn lock() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_item(key: &str, item: Option<Item>) {
    let changed = lock().put_item(key, item);
    if changed {
        request_redraw();
    }
}

/// The bar's state, for code that is given another one in tests.
pub fn cell() -> &'static Mutex<State> {
    &STATE
}

/// `State::apply` on `st` (the bar's state, or a test's), with a redraw when
/// something changed. False when the result was dropped (a job from before a reload).
pub fn apply(st: &Mutex<State>, generation: u64, key: &str, error: Option<String>, item: Option<Option<Item>>) -> bool {
    let r = st.lock().unwrap_or_else(|e| e.into_inner()).apply(generation, key, error, item);
    if r == Some(true) {
        request_redraw();
    }
    r.is_some()
}

pub fn set_spaces(spaces: Vec<Space>) {
    let changed = {
        let mut st = lock();
        if st.spaces == spaces {
            false
        } else {
            st.spaces = spaces;
            true
        }
    };
    if changed {
        request_redraw();
    }
}

pub fn set_title(t: String) {
    let changed = {
        let mut st = lock();
        if st.title == t {
            false
        } else {
            st.title = t;
            true
        }
    };
    if changed {
        request_redraw();
    }
}

// ── main-thread bridge (libdispatch, no polling) ───────────────────────────
static PENDING: AtomicBool = AtomicBool::new(false);

pub fn request_redraw() {
    if !PENDING.swap(true, Ordering::SeqCst) {
        DispatchQueue::main().exec_async(|| {
            PENDING.store(false, Ordering::SeqCst);
            crate::ui::redraw_all();
        });
    }
}

/// Run a closure on the main thread.
pub fn on_main_thread<F: FnOnce() + Send + 'static>(f: F) {
    DispatchQueue::main().exec_async(f);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str) -> Item {
        let c = Rgba(1.0, 1.0, 1.0, 1.0);
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

    #[test]
    fn results_from_an_earlier_generation_are_dropped() {
        let mut st = State::default();
        let g0 = st.generation_of("a");
        assert_eq!(st.apply(g0, "a", None, Some(Some(item("1")))), Some(true));
        assert_eq!(st.apply(g0, "a", None, Some(Some(item("1")))), Some(false), "no change");
        // a reload removes "a" (or makes it anew): it moves on, and its state is forgotten
        st.invalidate("a");
        st.forget("a");
        let g1 = st.generation_of("a");
        assert_ne!(g1, g0);
        // a job started before the reload finishes now: nothing of it is written
        assert_eq!(st.apply(g0, "a", Some("late".into()), Some(Some(item("late")))), None);
        assert!(st.items.is_empty() && st.errors.is_empty());
        // a job of the new generation is applied
        assert_eq!(st.apply(g1, "a", Some("e".into()), Some(Some(item("2")))), Some(true));
        assert_eq!((st.items["a"].text.as_str(), st.errors["a"].as_str()), ("2", "e"));
        assert_eq!(st.apply(g1, "a", None, None), Some(false), "keep: the error is cleared, the item stays");
        assert!(st.errors.is_empty() && st.items.contains_key("a"));
        assert_eq!(st.apply(g1, "a", None, Some(None)), Some(true), "hide");
        assert!(st.items.is_empty());
    }

    #[test]
    fn a_reload_drops_only_the_results_of_the_widgets_it_replaced() {
        let mut st = State::default();
        let (ga, gb) = (st.generation_of("a"), st.generation_of("b"));
        // jobs of "a" and "b" are under way; a reload makes "b" anew and keeps "a"
        st.invalidate("b");
        assert_eq!(st.apply(ga, "a", None, Some(Some(item("a")))), Some(true), "the kept widget's run is shown");
        assert_eq!(st.apply(gb, "b", None, Some(Some(item("b")))), None, "the replaced one's is dropped");
        // a second key invalidated later doesn't touch the first
        st.invalidate("c");
        assert_eq!(st.apply(ga, "a", None, Some(Some(item("a2")))), Some(true));
        assert_eq!(st.apply(st.generation_of("b"), "b", None, Some(Some(item("b2")))), Some(true));
    }

    #[test]
    fn forget_clears_items_errors_and_the_disabled_mark() {
        let mut st = State::default();
        st.items.insert("a".into(), item("x"));
        st.errors.insert("a".into(), "e".into());
        st.disabled.insert("a".into());
        st.items.insert("b".into(), item("y"));
        st.forget("a");
        assert!(!st.items.contains_key("a") && st.errors.is_empty() && st.disabled.is_empty());
        assert!(st.items.contains_key("b"));
    }
}
