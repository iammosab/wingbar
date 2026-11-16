//! CoreAudio HAL properties (AudioHardware.h) and the AudioToolbox
//! "hardware service" properties (the virtual main volume).

use std::ffi::c_void;

/// `AudioObjectID`.
pub type ObjectId = u32;

/// `AudioObjectPropertyAddress`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Address {
    pub selector: u32,
    pub scope: u32,
    pub element: u32,
}

impl Address {
    pub const fn new(selector: u32, scope: u32, element: u32) -> Address {
        Address { selector, scope, element }
    }
}

pub const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

pub const SYSTEM_OBJECT: ObjectId = 1;
pub const SCOPE_GLOBAL: u32 = fourcc(b"glob");
pub const SCOPE_OUTPUT: u32 = fourcc(b"outp");
pub const DEFAULT_OUTPUT_DEVICE: u32 = fourcc(b"dOut");
pub const VIRTUAL_MAIN_VOLUME: u32 = fourcc(b"vmvc");
pub const VOLUME_SCALAR: u32 = fourcc(b"volm");
pub const MUTE: u32 = fourcc(b"mute");

/// `AudioObjectPropertyListenerProc`.
pub type Listener =
    extern "C" fn(obj: ObjectId, n_addresses: u32, addresses: *const Address, client_data: *mut c_void) -> i32;

type GetFn = unsafe extern "C" fn(ObjectId, *const Address, u32, *const c_void, *mut u32, *mut c_void) -> i32;
type SetFn = unsafe extern "C" fn(ObjectId, *const Address, u32, *const c_void, u32, *const c_void) -> i32;
type HasFn = unsafe extern "C" fn(ObjectId, *const Address) -> u8;

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyData(
        o: ObjectId,
        a: *const Address,
        qs: u32,
        q: *const c_void,
        s: *mut u32,
        d: *mut c_void,
    ) -> i32;
    fn AudioObjectSetPropertyData(
        o: ObjectId,
        a: *const Address,
        qs: u32,
        q: *const c_void,
        s: u32,
        d: *const c_void,
    ) -> i32;
    fn AudioObjectHasProperty(o: ObjectId, a: *const Address) -> u8;
    fn AudioObjectAddPropertyListener(o: ObjectId, a: *const Address, l: Listener, c: *mut c_void) -> i32;
    fn AudioObjectRemovePropertyListener(o: ObjectId, a: *const Address, l: Listener, c: *mut c_void) -> i32;
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioHardwareServiceGetPropertyData(
        o: ObjectId,
        a: *const Address,
        qs: u32,
        q: *const c_void,
        s: *mut u32,
        d: *mut c_void,
    ) -> i32;
    fn AudioHardwareServiceSetPropertyData(
        o: ObjectId,
        a: *const Address,
        qs: u32,
        q: *const c_void,
        s: u32,
        d: *const c_void,
    ) -> i32;
    fn AudioHardwareServiceHasProperty(o: ObjectId, a: *const Address) -> u8;
}

/// A fixed-size property value that any bit pattern is valid for. The
/// properties used here are plain `UInt32`/`Float32` scalars: no CF objects,
/// so there is nothing to retain or release.
pub trait Scalar: Copy + Default + private::Sealed {}
impl Scalar for u32 {}
impl Scalar for f32 {}
mod private {
    pub trait Sealed {}
    impl Sealed for u32 {}
    impl Sealed for f32 {}
}

/// Which API owns the property: the HAL (`AudioObject*`) or AudioToolbox's
/// hardware service (`AudioHardwareService*`, needed for the virtual main volume).
#[derive(Clone, Copy)]
pub enum Api {
    Object,
    Service,
}

impl Api {
    fn fns(self) -> (HasFn, GetFn, SetFn) {
        match self {
            Api::Object => (AudioObjectHasProperty, AudioObjectGetPropertyData, AudioObjectSetPropertyData),
            Api::Service => (
                AudioHardwareServiceHasProperty,
                AudioHardwareServiceGetPropertyData,
                AudioHardwareServiceSetPropertyData,
            ),
        }
    }

    pub fn has(self, obj: ObjectId, a: &Address) -> bool {
        // SAFETY: `a` points to a valid address for the duration of the call.
        unsafe { (self.fns().0)(obj, a) != 0 }
    }

    /// Read a scalar property; `None` on error or a size mismatch.
    pub fn get<T: Scalar>(self, obj: ObjectId, a: &Address) -> Option<T> {
        let mut v = T::default();
        let want = size_of::<T>() as u32;
        let mut size = want;
        // SAFETY: `a` is valid; no qualifier is passed; `v` is a writable buffer of
        // exactly `size` bytes, and any bytes CoreAudio writes are a valid `T`.
        let r = unsafe { (self.fns().1)(obj, a, 0, std::ptr::null(), &mut size, (&raw mut v).cast()) };
        (r == 0 && size == want).then_some(v)
    }

    /// Write a scalar property; true on success.
    pub fn set<T: Scalar>(self, obj: ObjectId, a: &Address, v: T) -> bool {
        // SAFETY: `a` is valid; no qualifier is passed; `v` is a readable buffer of
        // exactly the size passed.
        unsafe { (self.fns().2)(obj, a, 0, std::ptr::null(), size_of::<T>() as u32, (&raw const v).cast()) == 0 }
    }
}

/// Register `l` for changes of `a` on `obj` (with NULL client data).
/// Each successful add stays in force until a matching [`remove_listener`],
/// so callers must keep track of what they registered.
pub fn add_listener(obj: ObjectId, a: &Address, l: Listener) -> bool {
    // SAFETY: `a` is valid for the call (CoreAudio copies it); `l` is a
    // `'static` fn that ignores its NULL client data.
    unsafe { AudioObjectAddPropertyListener(obj, a, l, std::ptr::null_mut()) == 0 }
}

/// Undo [`add_listener`]. Fails harmlessly if the object has gone away.
pub fn remove_listener(obj: ObjectId, a: &Address, l: Listener) -> bool {
    // SAFETY: as for `add_listener`; the same (proc, NULL) pair identifies the registration.
    unsafe { AudioObjectRemovePropertyListener(obj, a, l, std::ptr::null_mut()) == 0 }
}
