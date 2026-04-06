//! Unified source location tracking for the Verum compiler.
//!
//! This module provides two span representations:
//!
//! - [`Span`]: Efficient byte-offset based spans (12 bytes, Copy)
//! - [`LineColSpan`]: Human-readable line/column spans for diagnostics
//!
//! # Design Principles
//!
//! 1. **Efficiency First**: Use `Span` for AST nodes and internal processing
//! 2. **Display Quality**: Convert to `LineColSpan` only for error messages
//! 3. **Lazy Conversion**: Defer expensive line/column calculations
//! 4. **Zero Copy**: `Span` is Copy, no heap allocations
//!
//! # Specification
//!
//! Unified span handling used across all compiler crates for source location tracking.
//!
//! # Examples
//!
//! ```rust
//! use verum_common::span::{Span, FileId};
//!
//! let span = Span::new(0, 10, FileId::new(0));
//! assert_eq!(span.len(), 10);
//!
//! let merged = span.merge(Span::new(5, 15, FileId::new(0)));
//! assert_eq!(merged.start, 0);
//! assert_eq!(merged.end, 15);
//! ```

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// Import Text for diagnostic messages
use crate::Text;

/// A unique identifier for a source file.
///
/// File IDs are assigned sequentially during compilation and used to
/// distinguish spans from different files efficiently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(u32);

impl FileId {
    /// Create a new file ID.
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Create a dummy file ID for testing or generated code.
    pub const fn dummy() -> Self {
        Self(u32::MAX)
    }

    /// Get the raw file ID value.
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Check if this is a dummy file ID.
    pub const fn is_dummy(self) -> bool {
        self.0 == u32::MAX
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_dummy() {
            write!(f, "FileId(dummy)")
        } else {
            write!(f, "FileId({})", self.0)
        }
    }
}

/// A byte-offset based source span (primary representation).
///
/// This is the canonical span representation used throughout the compiler.
/// It's efficient (12 bytes), copyable, and suitable for AST nodes.
///
/// # Performance Characteristics
///
/// - Size: 12 bytes (3 × u32)
/// - Copy: Yes (no heap allocation)
/// - Comparison: O(1)
/// - Merge: O(1)
///
/// # Specification
///
/// Performance: Spans are 12 bytes (3 x u32), Copy, and require < 5% memory
/// overhead vs unsafe code. Comparison and merge are O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    /// Starting byte offset in the source file
    pub start: u32,
    /// Ending byte offset in the source file (exclusive)
    pub end: u32,
    /// File ID for multi-file compilation
    pub file_id: FileId,
}

impl Span {
    /// Create a new span from byte offsets.
    pub const fn new(start: u32, end: u32, file_id: FileId) -> Self {
        Self {
            start,
            end,
            file_id,
        }
    }

    /// Create a dummy span for testing or generated code.
    pub const fn dummy() -> Self {
        Self {
            start: 0,
            end: 0,
            file_id: FileId::dummy(),
        }
    }

    /// Get the length of this span in bytes.
    pub const fn len(&self) -> u32 {
        self.end - self.start
    }

    /// Check if this span is empty.
    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Merge two spans into one that covers both.
    ///
    /// # Panics
    ///
    /// Panics if spans are from different files.
    pub fn merge(self, other: Span) -> Span {
        assert_eq!(
            self.file_id, other.file_id,
            "Cannot merge spans from different files"
        );
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            file_id: self.file_id,
        }
    }

    /// Check if this span contains another span.
    pub fn contains(&self, other: Span) -> bool {
        self.file_id == other.file_id && self.start <= other.start && other.end <= self.end
    }

    /// Check if this span overlaps with another span.
    pub fn overlaps(&self, other: Span) -> bool {
        self.file_id == other.file_id && self.start < other.end && other.start < self.end
    }

    /// Check if this is a dummy span.
    pub const fn is_dummy(&self) -> bool {
        self.file_id.is_dummy()
    }
}

impl Default for Span {
    fn default() -> Self {
        Self::dummy()
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}-{}", self.file_id, self.start, self.end)
    }
}

/// A line/column based span for human-readable diagnostics.
///
/// This representation is more expensive (heap allocation for file name)
/// but provides better error messages. Use only for diagnostic output.
///
/// # Design Notes
///
/// - Lines and columns are 1-indexed (human-friendly)
/// - Supports both single-line and multi-line spans
/// - Lazy conversion from `Span` using source file information
///
/// # Performance
///
/// This type allocates a String for the file path, so avoid using it
/// in hot paths. Convert from `Span` only when displaying errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineColSpan {
    /// Source file path or name
    pub file: Text,
    /// Starting line (1-indexed)
    pub line: usize,
    /// Starting column (1-indexed)
    pub column: usize,
    /// Ending column (1-indexed, exclusive)
    pub end_column: usize,
    /// Ending line (1-indexed, None for single-line spans)
    pub end_line: Option<usize>,
}

impl LineColSpan {
    /// Create a new single-line span.
    pub fn new(file: impl Into<String>, line: usize, column: usize, end_column: usize) -> Self {
        Self {
            file: Text::from(file.into()),
            line,
            column,
            end_column,
            end_line: None,
        }
    }

    /// Create a new multi-line span.
    pub fn new_multiline(
        file: impl Into<String>,
        line: usize,
        column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Self {
        Self {
            file: Text::from(file.into()),
            line,
            column,
            end_column,
            end_line: Some(end_line),
        }
    }

    /// Check if this span covers multiple lines.
    pub fn is_multiline(&self) -> bool {
        self.end_line.is_some() && self.end_line.unwrap() != self.line
    }

    /// Get the length of the span on a single line.
    ///
    /// Returns 0 for multi-line spans.
    pub fn length(&self) -> usize {
        if self.is_multiline() {
            0
        } else {
            self.end_column.saturating_sub(self.column)
        }
    }

    /// Get the ending line (same as starting line for single-line spans).
    pub fn end_line(&self) -> usize {
        self.end_line.unwrap_or(self.line)
    }
}

impl fmt::Display for LineColSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_multiline() {
            write!(
                f,
                "{}:{}:{}-{}:{}",
                self.file,
                self.line,
                self.column,
                self.end_line.unwrap(),
                self.end_column
            )
        } else {
            write!(f, "{}:{}:{}", self.file, self.line, self.column)
        }
    }
}

/// Information about a source file for span conversion.
///
/// This type maintains the mapping between byte offsets and line/column
/// positions, enabling efficient conversion from `Span` to `LineColSpan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    /// Unique identifier for this file
    pub id: FileId,
    /// Path to the file (if it exists on disk)
    pub path: Option<PathBuf>,
    /// Name of the file for display purposes
    pub name: Text,
    /// Source code content
    pub source: Text,
    /// Line start positions (byte offsets) for quick line lookup
    pub line_starts: Vec<u32>,
}

impl SourceFile {
    /// Create a new source file.
    pub fn new(id: FileId, name: String, source: String) -> Self {
        let line_starts = Self::compute_line_starts(&source);
        Self {
            id,
            path: None,
            name: Text::from(name),
            source: Text::from(source),
            line_starts,
        }
    }

    /// Create a source file from a file path.
    pub fn from_path(id: FileId, path: PathBuf) -> std::io::Result<Self> {
        let source = std::fs::read_to_string(&path)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let line_starts = Self::compute_line_starts(&source);
        Ok(Self {
            id,
            path: Some(path),
            name: Text::from(name),
            source: Text::from(source),
            line_starts,
        })
    }

    /// Compute line start positions from source text.
    ///
    /// Supports all three line ending conventions:
    /// - LF (\n)      - Unix/Linux
    /// - CRLF (\r\n)  - Windows
    /// - CR (\r)      - Classic Mac
    fn compute_line_starts(source: &str) -> Vec<u32> {
        let mut starts = Vec::new();
        starts.push(0);

        let bytes = source.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\n' => {
                    // LF: Unix line ending
                    starts.push((i + 1) as u32);
                    i += 1;
                }
                b'\r' => {
                    // Check if CRLF or just CR
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                        // CRLF: Windows line ending
                        starts.push((i + 2) as u32);
                        i += 2;
                    } else {
                        // CR: Classic Mac line ending
                        starts.push((i + 1) as u32);
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        starts
    }

    /// Get the line and column for a byte offset.
    ///
    /// Returns (line, column) both 0-indexed for internal use.
    /// Add 1 to each for human-readable 1-indexed positions.
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        // Binary search for the line
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(next) => next.saturating_sub(1),
        };

        let line_start = self.line_starts.get(line).copied().unwrap_or(0);
        let col = offset.saturating_sub(line_start);
        (line as u32, col)
    }

    /// Convert a byte-offset Span to a LineColSpan.
    ///
    /// Lines and columns in the result are 1-indexed.
    pub fn span_to_line_col(&self, span: Span) -> Option<LineColSpan> {
        if span.file_id != self.id {
            return None;
        }

        let (start_line, start_col) = self.line_col(span.start);
        let (end_line, end_col) = self.line_col(span.end);

        Some(if start_line == end_line {
            LineColSpan::new(
                self.name.clone(),
                (start_line + 1) as usize,
                (start_col + 1) as usize,
                (end_col + 1) as usize,
            )
        } else {
            LineColSpan::new_multiline(
                self.name.clone(),
                (start_line + 1) as usize,
                (start_col + 1) as usize,
                (end_line + 1) as usize,
                (end_col + 1) as usize,
            )
        })
    }

    /// Get the source text for a span.
    pub fn span_text(&self, span: Span) -> Option<&str> {
        if span.file_id != self.id {
            return None;
        }
        let start = span.start as usize;
        let end = span.end as usize;
        self.source.get(start..end)
    }

    /// Get the line containing a span.
    pub fn span_line(&self, span: Span) -> Option<&str> {
        if span.file_id != self.id {
            return None;
        }
        let (line, _) = self.line_col(span.start);
        let line_start = self.line_starts.get(line as usize).copied()? as usize;
        let line_end = self
            .line_starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(self.source.len() as u32) as usize;
        self.source.get(line_start..line_end)
    }
}

/// A trait for types that have a source span.
pub trait Spanned {
    /// Get the span of this value.
    fn span(&self) -> Span;
}

impl Spanned for Span {
    fn span(&self) -> Span {
        *self
    }
}

impl<T: Spanned> Spanned for Box<T> {
    fn span(&self) -> Span {
        (**self).span()
    }
}

impl<T: Spanned> Spanned for &T {
    fn span(&self) -> Span {
        (*self).span()
    }
}

// =============================================================================
// Global Source File Registry
// =============================================================================

use std::collections::HashMap;
use std::sync::RwLock;

/// Global source file registry for span-to-location conversion.
///
/// This registry allows the parser and other components to convert
/// byte-offset Spans to human-readable file:line:column format.
static GLOBAL_SOURCE_FILES: std::sync::OnceLock<RwLock<HashMap<u32, SourceFile>>> =
    std::sync::OnceLock::new();

fn global_registry() -> &'static RwLock<HashMap<u32, SourceFile>> {
    GLOBAL_SOURCE_FILES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register a source file in the global registry.
///
/// Call this when loading/parsing a source file to enable proper
/// error message formatting.
pub fn register_source_file(id: FileId, name: impl Into<String>, source: impl Into<String>) {
    let source_file = SourceFile::new(id, name.into(), source.into());
    global_registry()
        .write()
        .expect("source file registry poisoned")
        .insert(id.raw(), source_file);
}

/// Convert a Span to a LineColSpan using the global registry.
///
/// Returns a human-readable location like "file.vr:42:15" if the
/// source file is registered, or a fallback format otherwise.
pub fn global_span_to_line_col(span: Span) -> LineColSpan {
    // Handle dummy spans
    if span.is_dummy() {
        return LineColSpan::new("<generated>", 0, 0, 0);
    }

    // Look up the source file
    if let Ok(guard) = global_registry().read()
        && let Some(source_file) = guard.get(&span.file_id.raw())
        && let Some(lc_span) = source_file.span_to_line_col(span)
    {
        return lc_span;
    }

    // Fallback: create a span with FileId info for debugging
    LineColSpan::new(
        format!("<file:{}>", span.file_id.raw()),
        1,
        span.start as usize,
        span.end as usize,
    )
}

/// Get the filename for a FileId from the global registry.
pub fn global_get_filename(id: FileId) -> Option<String> {
    global_registry()
        .read()
        .ok()?
        .get(&id.raw())
        .map(|f| f.name.clone().into_string())
}
