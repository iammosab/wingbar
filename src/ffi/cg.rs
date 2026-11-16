//! CoreGraphics window levels (CGWindowLevel.h).

/// `kCGDesktopIconWindowLevelKey`.
pub const DESKTOP_ICON_WINDOW_LEVEL_KEY: i32 = 18;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
}

/// The window level for a `CGWindowLevelKey`.
pub fn window_level_for_key(key: i32) -> i32 {
    // SAFETY: a pure lookup by integer key, with no pointers involved.
    unsafe { CGWindowLevelForKey(key) }
}
