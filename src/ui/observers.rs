//! Everything macOS can tell us instead of us asking: screens changing, the
//! frontmost app, Space switches, waking up, and the players' now-playing
//! notifications.

use crate::config::WidgetType;
use crate::sched;
use crate::state;
use crate::sys;
use crate::widgets;
use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSWorkspace, NSWorkspaceActiveSpaceDidChangeNotification,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidWakeNotification,
};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSOperationQueue, NSString,
};
use std::ptr::NonNull;

fn update_front_app() {
    if let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() {
        let pid = app.processIdentifier();
        let name = app.localizedName().map(|n| n.to_string()).unwrap_or_default();
        let mut st = state::lock();
        st.front_pid = pid;
        st.front_name = name;
    }
}

/// Run `f` on the main queue for every `name` notification, for the life of the app.
fn observe(center: &NSNotificationCenter, name: &NSString, f: impl Fn(&NSNotification) + 'static) {
    // SAFETY: the centre hands the block a valid notification for the duration of the call.
    let block = RcBlock::new(move |n: NonNull<NSNotification>| f(unsafe { n.as_ref() }));
    // SAFETY: the block is copied by the centre and is 'static; it runs on the main
    // queue only, so it may touch main-thread state. The token is kept forever.
    let token: Retained<ProtocolObject<dyn NSObjectProtocol>> = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        )
    };
    std::mem::forget(token); // observers live as long as the app
}

fn user_info_str(n: &NSNotification, key: &str) -> String {
    let Some(info) = n.userInfo() else { return String::new() };
    let k = NSString::from_str(key);
    info.objectForKey(&k as &AnyObject)
        .and_then(|v| v.downcast_ref::<NSString>().map(|s| s.to_string()))
        .unwrap_or_default()
}

pub fn install_observers(mtm: MainThreadMarker) {
    update_front_app();
    // SAFETY: the notification names are immutable AppKit constants, initialised on load.
    let (screens, activated, space, wake) = unsafe {
        (
            NSApplicationDidChangeScreenParametersNotification,
            NSWorkspaceDidActivateApplicationNotification,
            NSWorkspaceActiveSpaceDidChangeNotification,
            NSWorkspaceDidWakeNotification,
        )
    };
    observe(&NSNotificationCenter::defaultCenter(), screens, move |_| {
        super::build_bars(mtm);
        widgets::refresh_type(WidgetType::Spaces);
    });
    let ws = NSWorkspace::sharedWorkspace().notificationCenter();
    observe(&ws, activated, |_| {
        update_front_app();
        widgets::refresh_type(WidgetType::Title);
        widgets::refresh_type_in(WidgetType::Spaces, 80);
    });
    observe(&ws, space, |_| {
        widgets::refresh_type(WidgetType::Spaces);
        widgets::refresh_type(WidgetType::Title);
    });
    observe(&ws, wake, |_| sched::refresh_all());
    let dist = NSDistributedNotificationCenter::defaultCenter();
    // For the player widgets in the registry at the time of the event (by type,
    // so a script that happens to be keyed `spotify` is never written over).
    for p in widgets::PLAYERS {
        observe(&dist, &NSString::from_str(p.notification), move |n| {
            let keys = widgets::current().keys_of_type(p.ty);
            if keys.is_empty() {
                return;
            }
            let playing = user_info_str(n, "Player State") == "Playing";
            let (name, artist) = (user_info_str(n, "Name"), user_info_str(n, "Artist"));
            for k in keys {
                widgets::now_playing_changed(&k, playing, &name, &artist);
            }
        });
    }
    sys::watch_battery();
    sys::watch_audio();
}
