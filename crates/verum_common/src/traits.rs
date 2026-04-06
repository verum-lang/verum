//! Core trait definitions
//!
//! Re-exports of fundamental traits from core/std

/// Send marker trait (re-export from core)
pub use core::marker::Send;

/// Sync marker trait (re-export from core)
pub use core::marker::Sync;

/// Clone trait
pub use core::clone::Clone;

/// Default trait
pub use core::default::Default;

/// Debug trait
pub use core::fmt::Debug;

/// Display trait
pub use core::fmt::Display;

/// PartialEq trait
pub use core::cmp::PartialEq;

/// Eq trait
pub use core::cmp::Eq;

/// PartialOrd trait
pub use core::cmp::PartialOrd;

/// Ord trait
pub use core::cmp::Ord;

/// Hash trait
pub use core::hash::Hash;
