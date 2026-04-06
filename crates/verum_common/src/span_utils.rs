//! Span conversion utilities for LSP and diagnostics.
//!
//! This module provides efficient conversions between different span
//! representations used in the Verum compiler ecosystem:
//!
//! - Byte offsets → Line/column positions
//! - `Span` → `LineColSpan`
//! - Verum spans → LSP Range/Position (when tower_lsp feature enabled)
//!
//! # Design Principles
//!
//! 1. **Lazy Conversion**: Only convert when needed for output
//! 2. **Cache Line Starts**: Pre-compute line boundaries for O(log n) lookup
//! 3. **Zero Copy Where Possible**: Use references and indices
//!
//! # Specification
//!
//! Centralized span conversion utilities shared across all compiler crates.
//!
//! # Examples
//!
//! ```rust
//! use verum_common::span_utils::offset_to_line_col;
//!
//! let source = "line 1\nline 2\nline 3";
//! let (line, col) = offset_to_line_col(7, source);
//! assert_eq!(line, 1); // Second line (0-indexed)
//! assert_eq!(col, 0);  // Start of line
//! ```

use crate::span::{LineColSpan, Span};

#[cfg(feature = "lsp")]
use crate::span::FileId;

/// Convert a byte offset to line and column position (0-indexed).
///
/// This function scans through the source text character by character.
/// For repeated conversions, use [`SourceFile`] which caches line starts.
///
/// # Arguments
///
/// * `offset` - Byte offset in the source text
/// * `text` - Source text to calculate position from
///
/// # Returns
///
/// A tuple of `(line, column)` where both are 0-indexed.
///
/// # Performance
///
/// - Time complexity: O(n) where n is the offset
/// - For multiple conversions, use `SourceFile::line_col()` which is O(log m)
///   where m is the number of lines
///
/// # Examples
///
/// ```rust
/// use verum_common::span_utils::offset_to_line_col;
///
/// let text = "line 1\nline 2";
/// let (line, col) = offset_to_line_col(7, text);
/// assert_eq!(line, 1);
/// assert_eq!(col, 0);
/// ```
pub fn offset_to_line_col(offset: usize, text: &str) -> (usize, usize) {
    let mut line = 0;
    let mut column = 0;
    let mut current_offset = 0;

    for ch in text.chars() {
        if current_offset >= offset {
            break;
        }

        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }

        current_offset += ch.len_utf8();
    }

    (line, column)
}

/// Convert line and column position to byte offset (0-indexed).
///
/// # Arguments
///
/// * `line` - Line number (0-indexed)
/// * `column` - Column number (0-indexed)
/// * `text` - Source text
///
/// # Returns
///
/// The byte offset, or `None` if the position is out of bounds.
///
/// # Examples
///
/// ```rust
/// use verum_common::span_utils::line_col_to_offset;
///
/// let text = "line 1\nline 2";
/// let offset = line_col_to_offset(1, 0, text).unwrap();
/// assert_eq!(offset, 7);
/// ```
pub fn line_col_to_offset(line: usize, column: usize, text: &str) -> Option<usize> {
    let mut current_line = 0;
    let mut current_column = 0;
    let mut offset = 0;

    for ch in text.chars() {
        if current_line == line && current_column == column {
            return Some(offset);
        }

        if ch == '\n' {
            current_line += 1;
            current_column = 0;
        } else {
            current_column += 1;
        }

        offset += ch.len_utf8();
    }

    // Handle end of file position
    if current_line == line && current_column == column {
        Some(offset)
    } else {
        None
    }
}

/// Convert a byte-offset Span to a LineColSpan (1-indexed).
///
/// This is a convenience function for one-off conversions. For multiple
/// conversions on the same file, use `SourceFile::span_to_line_col()`.
///
/// # Arguments
///
/// * `span` - The byte-offset span to convert
/// * `text` - Source text
/// * `file_name` - Name of the source file for display
///
/// # Returns
///
/// A LineColSpan with 1-indexed line and column numbers.
///
/// # Examples
///
/// ```rust
/// use verum_common::span::{Span, FileId};
/// use verum_common::span_utils::span_to_line_col_span;
///
/// let source = "line 1\nline 2";
/// let span = Span::new(0, 6, FileId::new(0));
/// let lc_span = span_to_line_col_span(span, source, "test.vr");
///
/// assert_eq!(lc_span.line, 1);
/// assert_eq!(lc_span.column, 1);
/// ```
pub fn span_to_line_col_span(span: Span, text: &str, file_name: &str) -> LineColSpan {
    let (start_line, start_col) = offset_to_line_col(span.start as usize, text);
    let (end_line, end_col) = offset_to_line_col(span.end as usize, text);

    if start_line == end_line {
        LineColSpan::new(
            file_name.to_string(),
            start_line + 1, // Convert to 1-indexed
            start_col + 1,
            end_col + 1,
        )
    } else {
        LineColSpan::new_multiline(
            file_name.to_string(),
            start_line + 1,
            start_col + 1,
            end_line + 1,
            end_col + 1,
        )
    }
}

// LSP-specific utilities (only available with tower_lsp)
#[cfg(feature = "lsp")]
pub mod lsp {
    use super::*;

    // Re-export LSP types for convenience
    pub use tower_lsp::lsp_types::{Position, Range};

    /// Convert byte offset to LSP Position (0-indexed).
    ///
    /// LSP uses 0-indexed line and character positions.
    ///
    /// # Arguments
    ///
    /// * `offset` - Byte offset in the source text
    /// * `text` - Source text
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let text = "line 1\nline 2";
    /// let pos = offset_to_lsp_position(7, text);
    /// assert_eq!(pos.line, 1);
    /// assert_eq!(pos.character, 0);
    /// ```
    pub fn offset_to_lsp_position(offset: usize, text: &str) -> Position {
        let (line, character) = super::offset_to_line_col(offset, text);
        Position {
            line: line as u32,
            character: character as u32,
        }
    }

    /// Convert a byte-offset Span to LSP Range.
    ///
    /// # Arguments
    ///
    /// * `span` - The span to convert
    /// * `text` - Source text
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let span = Span::new(0, 6, FileId::new(0));
    /// let range = span_to_lsp_range(span, "line 1");
    /// assert_eq!(range.start.line, 0);
    /// ```
    pub fn span_to_lsp_range(span: Span, text: &str) -> Range {
        Range {
            start: offset_to_lsp_position(span.start as usize, text),
            end: offset_to_lsp_position(span.end as usize, text),
        }
    }

    /// Convert a LineColSpan to LSP Range.
    ///
    /// Note: LineColSpan uses 1-indexed positions, LSP uses 0-indexed.
    ///
    /// # Arguments
    ///
    /// * `span` - The line/column span to convert
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let span = LineColSpan::new("test.vr", 1, 1, 5);
    /// let range = line_col_span_to_lsp_range(&span);
    /// assert_eq!(range.start.line, 0);
    /// assert_eq!(range.start.character, 0);
    /// ```
    pub fn line_col_span_to_lsp_range(span: &LineColSpan) -> Range {
        let start = Position {
            line: span.line.saturating_sub(1) as u32, // Convert to 0-indexed
            character: span.column.saturating_sub(1) as u32,
        };
        let end = Position {
            line: span.end_line().saturating_sub(1) as u32,
            character: span.end_column.saturating_sub(1) as u32,
        };
        Range { start, end }
    }

    /// Convert LSP Position to byte offset.
    ///
    /// # Arguments
    ///
    /// * `position` - LSP position (0-indexed)
    /// * `text` - Source text
    ///
    /// # Returns
    ///
    /// The byte offset, or `None` if the position is out of bounds.
    pub fn lsp_position_to_offset(position: Position, text: &str) -> Option<usize> {
        super::line_col_to_offset(position.line as usize, position.character as usize, text)
    }

    /// Convert LSP Range to a byte-offset Span.
    ///
    /// # Arguments
    ///
    /// * `range` - LSP range
    /// * `text` - Source text
    /// * `file_id` - File ID for the resulting span
    ///
    /// # Returns
    ///
    /// A Span, or `None` if the range is out of bounds.
    pub fn lsp_range_to_span(range: Range, text: &str, file_id: FileId) -> Option<Span> {
        let start = lsp_position_to_offset(range.start, text)?;
        let end = lsp_position_to_offset(range.end, text)?;
        Some(Span::new(start as u32, end as u32, file_id))
    }
}
