//! Every C binding wingbar declares itself, each behind a small safe wrapper.
//!
//! The rest of the crate (`sys`, `state`, `ui`) calls these wrappers and holds
//! no `extern "C"` blocks of its own. CoreFoundation objects come back as
//! `CFRetained` (from `objc2-core-foundation`), so ownership is decided once,
//! here, when a pointer crosses the boundary:
//!
//! - **Create/Copy rule**: the caller owns a +1 reference. Wrapped with
//!   [`cf::owned`], released exactly once when the `CFRetained` drops.
//! - **Get rule**: the reference is borrowed from its container. Exposed as a
//!   plain `&T` whose lifetime is tied to the owner, never released.
//!
//! `kqueue` is plain file descriptors (`OwnedFd`), closed when dropped.
//!
//! Main-queue dispatch uses `dispatch2` directly (see `state.rs`); it has no
//! unsafe surface left for us to wrap.

pub mod ax;
pub mod cf;
pub mod cg;
pub mod coreaudio;
pub mod iokit;
pub mod kqueue;
pub mod mach;
pub mod posix;
pub mod proc;
