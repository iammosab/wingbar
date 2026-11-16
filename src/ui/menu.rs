//! Native menus from a plain description. A `Menu` lists items (a title,
//! enabled or not, a check mark, an action or a submenu) and separators; an
//! action is a Rust closure, run on the main thread when its item is chosen.
//! `pop_up` shows the menu under a point of a view and returns when it closes.
//!
//! ```ignore
//! let mut m = Menu::new();
//! m.info("CPU");
//! m.item("Move Left", move || edit::move_left(&key));
//! m.push(Item::new("Hide CPU").enabled(can_hide).action(move || hide(&key)));
//! m.separator();
//! let mut shown = Menu::new();
//! shown.push(Item::new("Workspaces").checked(true).action(|| toggle("spaces")));
//! m.submenu("Widgets", shown);
//! menu::pop_up(m, &view, NSPoint::new(x, bar_height));
//! ```
//!
//! Each closure goes into a table on a target object; an item's tag is its
//! index there. The menu and its target are kept until the next menu opens,
//! so the closures live as long as AppKit may call them.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSControlStateValue, NSControlStateValueMixed, NSControlStateValueOff, NSControlStateValueOn, NSMenu, NSMenuItem,
    NSView,
};
use objc2_foundation::{NSPoint, NSString};
use std::cell::RefCell;

/// What choosing an item does (on the main thread, once).
pub type Action = Box<dyn FnOnce()>;

/// An item's check mark.
#[allow(dead_code)] // On / Mixed: for the bar's own menu
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Check {
    #[default]
    Off,
    On,
    /// A dash: some of what it stands for is on.
    Mixed,
}

pub struct Item {
    pub title: String,
    pub enabled: bool,
    pub check: Check,
    /// Shown when the pointer rests on the item.
    pub tooltip: Option<String>,
    pub action: Option<Action>,
    pub submenu: Option<Menu>,
}

impl Item {
    /// An enabled item with no action yet.
    pub fn new(title: impl Into<String>) -> Item {
        Item { title: title.into(), enabled: true, check: Check::Off, tooltip: None, action: None, submenu: None }
    }
    pub fn enabled(mut self, enabled: bool) -> Item {
        self.enabled = enabled;
        self
    }
    pub fn checked(mut self, on: bool) -> Item {
        self.check = if on { Check::On } else { Check::Off };
        self
    }
    #[allow(dead_code)] // for the bar's own menu
    pub fn check(mut self, check: Check) -> Item {
        self.check = check;
        self
    }
    pub fn tooltip(mut self, t: impl Into<String>) -> Item {
        self.tooltip = Some(t.into());
        self
    }
    pub fn action(mut self, f: impl FnOnce() + 'static) -> Item {
        self.action = Some(Box::new(f));
        self
    }
    pub fn submenu(mut self, m: Menu) -> Item {
        self.submenu = Some(m);
        self
    }
}

pub enum Entry {
    Item(Item),
    Separator,
}

/// A menu, as a plain list (no AppKit until it is shown).
#[derive(Default)]
pub struct Menu {
    pub entries: Vec<Entry>,
}

impl Menu {
    pub fn new() -> Menu {
        Menu::default()
    }
    pub fn push(&mut self, item: Item) -> &mut Menu {
        self.entries.push(Entry::Item(item));
        self
    }
    /// An item that runs `f`.
    pub fn item(&mut self, title: impl Into<String>, f: impl FnOnce() + 'static) -> &mut Menu {
        self.push(Item::new(title).action(f))
    }
    /// A disabled item: shown, not clickable (a header, a reading).
    pub fn info(&mut self, title: impl Into<String>) -> &mut Menu {
        self.push(Item::new(title).enabled(false))
    }
    pub fn separator(&mut self) -> &mut Menu {
        self.entries.push(Entry::Separator);
        self
    }
    #[allow(dead_code)] // the bar's menu builds its submenus as items
    pub fn submenu(&mut self, title: impl Into<String>, m: Menu) -> &mut Menu {
        self.push(Item::new(title).submenu(m))
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A menu with its actions taken out: what AppKit is given. `tag` indexes the
/// action table.
#[derive(Debug, PartialEq)]
enum Node {
    Item {
        title: String,
        enabled: bool,
        check: Check,
        tooltip: Option<String>,
        tag: Option<usize>,
        submenu: Option<Vec<Node>>,
    },
    Separator,
}

/// The actions of a shown menu, by tag. Each runs once at most.
#[derive(Default)]
struct Actions(Vec<Option<Action>>);

impl Actions {
    /// Take out the action for `tag` (`None` if there is none, or it ran).
    fn take(&mut self, tag: isize) -> Option<Action> {
        self.0.get_mut(usize::try_from(tag).ok()?)?.take()
    }
}

/// Take the actions out of `menu` into `actions`, numbering them. Separators
/// at the start or the end, or after another separator, are left out.
fn plan(menu: Menu, actions: &mut Actions) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    for e in menu.entries {
        match e {
            Entry::Separator => {
                if out.last().is_some_and(|n| *n != Node::Separator) {
                    out.push(Node::Separator);
                }
            }
            Entry::Item(it) => {
                let tag = it.action.map(|f| {
                    actions.0.push(Some(f));
                    actions.0.len() - 1
                });
                let submenu = it.submenu.map(|m| plan(m, actions));
                out.push(Node::Item {
                    title: it.title,
                    enabled: it.enabled,
                    check: it.check,
                    tooltip: it.tooltip,
                    tag,
                    submenu,
                });
            }
        }
    }
    if out.last() == Some(&Node::Separator) {
        out.pop();
    }
    out
}

define_class!(
    /// The target of every item of a shown menu: runs the action its tag names.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WingbarMenuTarget"]
    #[ivars = RefCell<Actions>]
    struct Target;

    impl Target {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &NSMenuItem) {
            // the action may open another menu, which lets go of this target
            let _alive = self.retain();
            let f = self.ivars().borrow_mut().take(sender.tag());
            if let Some(f) = f {
                f();
            }
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, actions: Actions) -> Retained<Target> {
        let this = Target::alloc(mtm).set_ivars(RefCell::new(actions));
        // SAFETY: NSObject's designated initialiser, called once on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

/// A menu made for AppKit, with the target that keeps its actions.
pub struct Built {
    pub menu: Retained<NSMenu>,
    #[allow(dead_code)] // kept alive with the menu
    target: Retained<Target>,
}

fn state(c: Check) -> NSControlStateValue {
    match c {
        Check::Off => NSControlStateValueOff,
        Check::On => NSControlStateValueOn,
        Check::Mixed => NSControlStateValueMixed,
    }
}

fn ns_menu(nodes: &[Node], target: &Target, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(""));
    // items are enabled as described, not by AppKit's validation
    menu.setAutoenablesItems(false);
    for n in nodes {
        let Node::Item { title, enabled, check, tooltip, tag, submenu } = n else {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        };
        // SAFETY: NSMenuItem's designated initialiser on a fresh allocation; no
        // action yet (set below with its target) and an empty key equivalent.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        };
        item.setEnabled(*enabled);
        item.setState(state(*check));
        if let Some(t) = tooltip {
            item.setToolTip(Some(&NSString::from_str(t)));
        }
        if let Some(tag) = tag {
            item.setTag(*tag as isize);
            // SAFETY: `fire:` is a method of `Target` taking the sending item; the
            // target is kept alive by `Built` (and `KEEP`) for as long as the menu.
            unsafe {
                item.setTarget(Some(target as &AnyObject));
                item.setAction(Some(sel!(fire:)));
            }
        }
        if let Some(sub) = submenu {
            item.setSubmenu(Some(&ns_menu(sub, target, mtm)));
        }
        menu.addItem(&item);
    }
    menu
}

/// Make the AppKit menu for `menu` (main thread only).
pub fn build(menu: Menu, mtm: MainThreadMarker) -> Built {
    let mut actions = Actions::default();
    let nodes = plan(menu, &mut actions);
    let target = Target::new(mtm, actions);
    Built { menu: ns_menu(&nodes, &target, mtm), target }
}

thread_local! {
    /// The menu shown last, with its target: kept until the next one opens.
    static KEEP: RefCell<Option<Built>> = const { RefCell::new(None) };
}

/// Show `menu` with its top-left corner at `at` (in `view`'s coordinates).
/// Returns once it closes (after running the chosen action): true if an item
/// was chosen. An empty menu isn't shown.
///
/// Call it from an event handler or a run-loop timer, not from a main-queue
/// block (`state::on_main_thread`, `DispatchQueue::main()`): the menu runs its
/// own loop until it closes, and the main queue would wait for it, redraws included.
pub fn pop_up(menu: Menu, view: &NSView, at: NSPoint) -> bool {
    if menu.is_empty() {
        return false;
    }
    let built = build(menu, view.mtm());
    let ns = built.menu.clone();
    KEEP.with(|k| *k.borrow_mut() = Some(built));
    ns.popUpMenuPositioningItem_atLocation_inView(None, at, Some(view))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn titles(nodes: &[Node]) -> Vec<String> {
        nodes
            .iter()
            .map(|n| match n {
                Node::Item { title, .. } => title.clone(),
                Node::Separator => "---".into(),
            })
            .collect()
    }

    #[test]
    fn the_description_becomes_nodes_with_tags() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = |s: &'static str| {
            let log = log.clone();
            move || log.borrow_mut().push(s)
        };
        let mut sub = Menu::new();
        sub.push(Item::new("Workspaces").checked(true).action(l("spaces")));
        sub.push(Item::new("Clock").check(Check::Mixed).action(l("clock")));
        let mut m = Menu::new();
        m.info("CPU").item("Move Left", l("left")).separator();
        m.push(Item::new("Hide").enabled(false).action(l("hide")));
        m.submenu("Widgets", sub);

        let mut actions = Actions::default();
        let nodes = plan(m, &mut actions);
        let item = |title: &str, enabled, check, tag, submenu| Node::Item {
            title: title.into(),
            enabled,
            check,
            tooltip: None,
            tag,
            submenu,
        };
        assert_eq!(
            nodes,
            [
                item("CPU", false, Check::Off, None, None),
                item("Move Left", true, Check::Off, Some(0), None),
                Node::Separator,
                item("Hide", false, Check::Off, Some(1), None),
                item(
                    "Widgets",
                    true,
                    Check::Off,
                    None,
                    Some(vec![
                        item("Workspaces", true, Check::On, Some(2), None),
                        item("Clock", true, Check::Mixed, Some(3), None),
                    ])
                ),
            ]
        );
        assert_eq!(actions.0.len(), 4);

        // a tag runs its own action, once
        for tag in [2, 0, 2, 9, -1] {
            if let Some(f) = actions.take(tag) {
                f();
            }
        }
        assert_eq!(*log.borrow(), ["spaces", "left"]);
    }

    #[test]
    fn stray_separators_are_left_out() {
        let mut m = Menu::new();
        m.separator().info("a").separator().separator().info("b").separator();
        assert_eq!(titles(&plan(m, &mut Actions::default())), ["a", "---", "b"]);
        let mut m = Menu::new();
        m.separator().separator();
        assert!(plan(m, &mut Actions::default()).is_empty());
        assert!(Menu::new().is_empty());
    }

    #[test]
    fn actions_own_what_they_capture() {
        // FnOnce: an action may move what it captured out
        let got = Rc::new(RefCell::new(None));
        let g = got.clone();
        let owned = String::from("moved");
        let mut m = Menu::new();
        m.item("x", move || *g.borrow_mut() = Some(owned));
        let mut actions = Actions::default();
        plan(m, &mut actions);
        actions.take(0).unwrap()();
        assert_eq!(got.borrow().as_deref(), Some("moved"));
        assert!(actions.take(0).is_none(), "ran already");
    }
}
