//! Position and range conversion utilities for LSP
//!
//! This module re-exports span utilities from `verum_common` for backward compatibility.
//! All new code should import from `verum_common::span_utils` directly.

// Re-export the utility functions from verum_common
pub use verum_common::span_utils::lsp::*;
pub use verum_common::span_utils::{line_col_to_offset, offset_to_line_col, span_to_line_col_span};

// For backward compatibility, also provide the old function names
use verum_common::span::{LineColSpan, Span};

/// Converts a verum_ast::Span to LSP Range (backward compatibility)
///
/// # Arguments
///
/// * `span` - AST span with byte offsets
/// * `text` - Source text to calculate positions from
pub fn ast_span_to_range(span: &Span, text: &str) -> Range {
    span_to_lsp_range(*span, text)
}

/// Converts a verum_diagnostics::Span to LSP Range (backward compatibility)
///
/// # Arguments
///
/// * `span` - Diagnostic span with line/column info
/// * `_text` - Source text (unused but kept for API consistency)
pub fn verum_span_to_range(span: &LineColSpan, _text: &str) -> Range {
    line_col_span_to_lsp_range(span)
}
