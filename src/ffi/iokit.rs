//! IOKit power sources (IOPowerSources.h).

use super::cf::{self, CFArray, CFRetained, CFRunLoop, CFRunLoopSource, CFType, Dict};
use std::ffi::c_void;

/// `IOPowerSourceCallbackType`.
pub type PowerCallback = extern "C" fn(context: *mut c_void);

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    /// Create rule: returns a +1 opaque blob (or NULL).
    fn IOPSCopyPowerSourcesInfo() -> *const CFType;
    /// Create rule: returns a +1 CFArray of power source handles (or NULL).
    fn IOPSCopyPowerSourcesList(blob: &CFType) -> *const CFArray<CFType>;
    /// Get rule: the dictionary belongs to `blob`; do not release it.
    fn IOPSGetPowerSourceDescription(blob: &CFType, ps: &CFType) -> *const Dict;
    /// Create rule: returns a +1 run loop source (or NULL).
    fn IOPSNotificationCreateRunLoopSource(cb: PowerCallback, ctx: *mut c_void) -> *const CFRunLoopSource;
}

/// A snapshot of every power source (battery, UPS, ...), released on drop.
pub struct PowerSources {
    blob: CFRetained<CFType>,
    list: CFRetained<CFArray<CFType>>,
}

impl PowerSources {
    pub fn copy() -> Option<PowerSources> {
        // SAFETY: no preconditions; the result follows the Create rule (+1, owned).
        let blob = unsafe { cf::owned(IOPSCopyPowerSourcesInfo()) }?;
        // SAFETY: `blob` is a live power-sources blob; the list follows the Create rule.
        let list = unsafe { cf::owned(IOPSCopyPowerSourcesList(&blob)) }?;
        Some(PowerSources { blob, list })
    }

    /// The description dictionary of each power source. They are borrowed
    /// from the snapshot (Get rule), so they cannot outlive it.
    pub fn descriptions(&self) -> impl Iterator<Item = &Dict> + '_ {
        self.list.iter().filter_map(move |ps| {
            // SAFETY: `blob` and `ps` are live, and `ps` came from this blob's list.
            let d = unsafe { IOPSGetPowerSourceDescription(&self.blob, &ps) };
            // SAFETY: Get rule: the dictionary is owned by `self.blob`, which is
            // immutable and outlives the returned `&self` borrow. NULL becomes None.
            unsafe { d.as_ref() }
        })
    }
}

/// Call `cb` on the main run loop whenever any power source changes.
/// Returns false if IOKit could not create the source.
pub fn notify_on_main_run_loop(cb: PowerCallback) -> bool {
    // SAFETY: `cb` is a valid `extern "C"` fn for the whole program and ignores
    // its (NULL) context. The source follows the Create rule.
    let Some(src) = (unsafe { cf::owned(IOPSNotificationCreateRunLoopSource(cb, std::ptr::null_mut())) }) else {
        return false;
    };
    let Some(main) = CFRunLoop::main() else { return false };
    // SAFETY: reading an immutable framework constant.
    let mode = unsafe { objc2_core_foundation::kCFRunLoopDefaultMode };
    // The run loop retains the source, so dropping our +1 reference here is
    // correct: the source lives as long as it stays scheduled.
    main.add_source(Some(&src), mode);
    true
}
