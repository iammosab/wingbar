//! Accessibility (AXUIElement.h, ApplicationServices).

use super::cf::{self, CFBoolean, CFDictionary, CFRetained, CFString, CFType};
use objc2_core_foundation::CFGetTypeID;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(opts: &CFDictionary<CFString, CFType>) -> u8;
    fn AXUIElementGetTypeID() -> usize;
    /// Create rule: returns a +1 AXUIElement (or NULL).
    fn AXUIElementCreateApplication(pid: i32) -> *const CFType;
    /// Copy rule: on success `*out` holds a +1 reference the caller must release.
    fn AXUIElementCopyAttributeValue(el: &CFType, attr: &CFString, out: *mut *const CFType) -> i32;
    fn AXUIElementSetMessagingTimeout(el: &CFType, secs: f32) -> i32;
    static kAXTrustedCheckOptionPrompt: &'static CFString;
}

/// Whether this process may use the Accessibility API. With `prompt`, macOS
/// shows its "allow in System Settings" dialog if it may not.
pub fn is_trusted(prompt: bool) -> bool {
    if !prompt {
        // SAFETY: no arguments, no preconditions.
        return unsafe { AXIsProcessTrusted() } != 0;
    }
    // SAFETY: reading an immutable framework constant.
    let key = unsafe { kAXTrustedCheckOptionPrompt };
    let opts = CFDictionary::<CFString, CFType>::from_slices(&[key], &[CFBoolean::new(true)]);
    // SAFETY: `opts` is a live dictionary of the documented key and value types.
    unsafe { AXIsProcessTrustedWithOptions(&opts) != 0 }
}

/// An owned AXUIElement (released on drop).
pub struct Element(CFRetained<CFType>);

impl Element {
    /// The element for the application with this pid. It is created even for
    /// a pid that doesn't exist; attribute reads on it then fail.
    pub fn application(pid: i32) -> Option<Element> {
        // SAFETY: any pid is accepted; the result follows the Create rule.
        unsafe { cf::owned(AXUIElementCreateApplication(pid)) }.map(Element)
    }

    /// Wrap a value read from another element, if it is an AXUIElement.
    fn from_value(v: CFRetained<CFType>) -> Option<Element> {
        // SAFETY: no arguments, no preconditions.
        let ax = unsafe { AXUIElementGetTypeID() };
        (CFGetTypeID(Some(&v)) == ax).then_some(Element(v))
    }

    /// How long a read may block on an unresponsive app.
    pub fn set_timeout(&self, secs: f32) {
        // SAFETY: `self.0` is a live AXUIElement.
        unsafe { AXUIElementSetMessagingTimeout(&self.0, secs) };
    }

    /// Read an attribute (Copy rule: the value is owned and released on drop).
    pub fn attribute(&self, name: &CFString) -> Option<CFRetained<CFType>> {
        let mut out: *const CFType = std::ptr::null();
        // SAFETY: `self.0` is a live AXUIElement, `name` a live CFString and
        // `out` a valid place to write one pointer.
        let err = unsafe { AXUIElementCopyAttributeValue(&self.0, name, &mut out) };
        if err != 0 {
            return None;
        }
        // SAFETY: on success the value follows the Copy rule (+1, ours to release).
        unsafe { cf::owned(out) }
    }

    /// Read an attribute whose value is itself an element (a window, a button...).
    pub fn element_attribute(&self, name: &CFString) -> Option<Element> {
        self.attribute(name).and_then(Element::from_value)
    }
}
