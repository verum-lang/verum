//! Core type definitions
//!
//! Verum uses semantic type names: Text (not String), List (not Vec), Maybe (not Option).
//! These aliases provide meaningful domain names while mapping to Rust std types.

/// Text type (semantic alternative to String)
///
/// This is the fundamental string type in Verum, following v6.0-BALANCED naming.
pub type Text = String;

/// List type (semantic alternative to Vec)
pub type List<T> = Vec<T>;

/// Numeric types
pub type Int = i64;
pub type Float = f64;
pub type Bool = bool;

/// Maybe type (semantic alternative to Option)
pub type Maybe<T> = Option<T>;

/// Result type with default error as Text
pub type VerumResult<T, E = Text> = Result<T, E>;
