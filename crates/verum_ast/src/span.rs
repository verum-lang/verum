//! Source location tracking for the AST.
//!
//! This module re-exports span types from `verum_common` for backward compatibility.
//! All new code should import from `verum_common::span` directly.

// Re-export all span types from verum_common
pub use verum_common::span::{FileId, LineColSpan, SourceFile, Span, Spanned};
