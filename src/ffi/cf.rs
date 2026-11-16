//! CoreFoundation helpers on top of `objc2-core-foundation`.

use std::ptr::NonNull;

pub use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFRunLoop, CFRunLoopSource, CFString, CFType, Type,
};

/// A dictionary with string keys and values of any CF type (property lists,
/// IOKit power source descriptions, and the like).
pub type Dict = CFDictionary<CFString, CFType>;

/// Take ownership of a pointer returned under the Create/Copy rule.
/// Returns `None` for NULL. The reference is released when the result drops.
///
/// # Safety
///
/// `ptr` must be NULL or point to a live `T` on which the caller holds a +1
/// reference that nothing else will release.
pub unsafe fn owned<T: Type>(ptr: *const T) -> Option<CFRetained<T>> {
    let ptr = NonNull::new(ptr.cast_mut())?;
    // SAFETY: non-null, and the caller guarantees it is a +1 reference to a `T`
    // that we now own; `CFRetained` releases it exactly once on drop.
    Some(unsafe { CFRetained::from_raw(ptr) })
}

/// The string value of `v`, if it is a CFString.
pub fn string(v: &CFType) -> Option<String> {
    v.downcast_ref::<CFString>().map(|s| s.to_string())
}

/// The integer value of `v`, if it is a CFNumber that fits in an i64.
pub fn i64(v: &CFType) -> Option<i64> {
    v.downcast_ref::<CFNumber>()?.as_i64()
}

/// The value of `v`, if it is a CFBoolean.
pub fn bool(v: &CFType) -> Option<bool> {
    v.downcast_ref::<CFBoolean>().map(|b| b.as_bool())
}

/// Look up a string key (Get rule: retained only for as long as the result lives).
pub fn get(d: &Dict, key: &'static str) -> Option<CFRetained<CFType>> {
    d.get(&CFString::from_static_str(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_round_trips_through_cfstring() {
        for s in ["", "AXTitle", "Kärnten – ünïcødé 🎵", "a\0b"] {
            let cf = CFString::from_str(s);
            assert_eq!(string(&cf).as_deref(), Some(s));
        }
    }

    #[test]
    fn typed_reads_reject_other_types() {
        let n = CFNumber::new_i64(42);
        let s = CFString::from_static_str("42");
        let t = CFBoolean::new(true);
        assert_eq!(i64(&n), Some(42));
        assert_eq!(i64(&s), None);
        assert_eq!(string(&n), None);
        assert_eq!(bool(t), Some(true));
        assert_eq!(bool(&n), None);
    }

    #[test]
    fn dictionary_lookup_by_str() {
        let k = CFString::from_static_str("Current Capacity");
        let v = CFNumber::new_i64(87);
        let d: CFRetained<Dict> = CFDictionary::from_slices(&[&*k], &[&**v]);
        assert_eq!(get(&d, "Current Capacity").and_then(|v| i64(&v)), Some(87));
        assert!(get(&d, "Max Capacity").is_none());
    }

    #[test]
    fn owned_null_is_none() {
        // SAFETY: NULL is explicitly allowed.
        assert!(unsafe { owned::<CFString>(std::ptr::null()) }.is_none());
    }

    #[test]
    fn owned_takes_over_a_create_rule_reference() {
        let s = CFString::from_str("owned");
        let raw = CFRetained::into_raw(s);
        // SAFETY: `into_raw` hands back the +1 reference we just gave up.
        let back = unsafe { owned(raw.as_ptr().cast_const()) }.unwrap();
        assert_eq!(string(&back).as_deref(), Some("owned"));
    }
}
