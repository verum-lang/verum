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

    /// Walk the synthetic-origin chain to the deepest user-source
    /// ancestor (#274).  `resolver` looks up `SyntheticOrigin`
    /// entries by FileId — typically the session's source registry.
    ///
    /// Returns the topmost user-visible span (one whose FileId has
    /// no synthetic origin) plus the chain of synthetic kinds that
    /// were traversed, ordered from leaf (this span's file) to
    /// user source.  When this span itself is already user-source,
    /// returns `(self, [])` — the empty chain signals "already
    /// resolved".
    ///
    /// Cycle defence: bails after `max_depth` iterations and
    /// returns the deepest reached span.  Synthetic chains
    /// shouldn't loop in well-formed compiler output, but
    /// defence-in-depth keeps a malformed invariant from hanging
    /// the diagnostic renderer.
    pub fn resolve_to_user_source<F>(
        self,
        resolver: F,
    ) -> ResolvedSpan
    where
        F: Fn(FileId) -> Option<SyntheticOrigin>,
    {
        const MAX_DEPTH: usize = 32;
        let mut chain: Vec<SyntheticKind> = Vec::new();
        let mut current = self;
        for _ in 0..MAX_DEPTH {
            match resolver(current.file_id) {
                None => {
                    return ResolvedSpan {
                        user_span: current,
                        expansion_chain: chain,
                    };
                }
                Some(origin) => {
                    chain.push(origin.kind);
                    current = origin.call_site_span;
                }
            }
        }
        ResolvedSpan {
            user_span: current,
            expansion_chain: chain,
        }
    }
}

/// Result of `Span::resolve_to_user_source` (#274).
///
/// Pairs the user-visible span with the chain of synthetic
/// expansions that were traversed to reach it.  The renderer uses
/// the chain to construct labels like
/// "in @derive expansion → in macro expansion".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSpan {
    /// User-visible span (or the deepest reached if MAX_DEPTH
    /// terminated the walk).
    pub user_span: Span,
    /// Chain of synthetic expansion kinds traversed, ordered from
    /// the original (leaf) span's file outward to user source.
    /// Empty when the input span was already user-source.
    pub expansion_chain: Vec<SyntheticKind>,
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

/// What kind of expansion produced a synthetic source file.
///
/// Recorded on `SourceFile.synthetic_origin` so the diagnostic
/// renderer can produce labels like "in macro expansion of @derive"
/// vs "in monomorphization of `List<T>`".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyntheticKind {
    /// Macro invocation expansion (`@my_macro(...)` callsite).
    MacroExpansion,
    /// `@derive(Eq)` / `@derive(Show)` etc. attribute expansion.
    DeriveExpansion,
    /// Monomorphization of a generic function or type.
    Monomorphization,
    /// `@delegate(target)` body synthesis.
    DelegateBody,
    /// Other synthetic origin (forward-compatibility).
    Other,
}

impl SyntheticKind {
    /// Human-readable label used by the diagnostic renderer.
    pub fn label(&self) -> &'static str {
        match self {
            SyntheticKind::MacroExpansion => "macro expansion",
            SyntheticKind::DeriveExpansion => "@derive expansion",
            SyntheticKind::Monomorphization => "monomorphization",
            SyntheticKind::DelegateBody => "@delegate body synthesis",
            SyntheticKind::Other => "synthetic expansion",
        }
    }
}

/// Provenance record for a synthetic source file (#274).
///
/// When a macro / derive / monomorphization / @delegate produces a
/// new source artefact, the resulting `SourceFile` carries this
/// origin pointing back at the user-source span that triggered the
/// expansion. The diagnostic renderer walks `.parent` chains
/// transitively (via `Span::resolve_to_user_source`) to find the
/// deepest user-visible location.
///
/// Without this, errors in generated code surface with synthetic
/// `FileId` locations like `<macro:Eq>:1:1` — opaque to users
/// debugging their own program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntheticOrigin {
    /// FileId of the parent (the file in which the expansion was
    /// triggered). May itself be synthetic — chains are walked
    /// transitively.
    pub parent_file: FileId,
    /// Span within `parent_file` that triggered the expansion.
    /// Where the user typed `@derive(Eq)` / `my_macro!()` /
    /// the generic call that monomorphized to this artefact.
    pub call_site_span: Span,
    /// What kind of expansion produced this synthetic source.
    pub kind: SyntheticKind,
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
    /// Synthetic-origin provenance.  `None` for user-source files
    /// loaded from disk; `Some` for files produced by macro /
    /// derive / monomorphization / @delegate expansions.  The
    /// diagnostic renderer walks the chain via
    /// `Span::resolve_to_user_source` to surface user-visible
    /// locations even for errors in generated code.  Closes #274.
    #[serde(default)]
    pub synthetic_origin: Option<SyntheticOrigin>,
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
            synthetic_origin: None,
        }
    }

    /// Create a synthetic source file produced by macro/derive/
    /// monomorphization/@delegate expansion.  Carries a
    /// `SyntheticOrigin` back-pointer at the parent span so the
    /// diagnostic renderer can resolve errors to user-visible
    /// locations via `Span::resolve_to_user_source`.  Closes #274.
    pub fn synthetic(
        id: FileId,
        name: String,
        source: String,
        origin: SyntheticOrigin,
    ) -> Self {
        let line_starts = Self::compute_line_starts(&source);
        Self {
            id,
            path: None,
            name: Text::from(name),
            source: Text::from(source),
            line_starts,
            synthetic_origin: Some(origin),
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
            synthetic_origin: None,
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

// =============================================================================
// Source-map synthetic-origin pin tests (task #274).
//
// The architectural primitive is `Span::resolve_to_user_source` plus the
// `SourceFile.synthetic_origin` field that lets the diagnostic renderer
// walk the parent chain from a synthetic FileId back to the user-source
// span that triggered the expansion.  Each test pins a layer of the
// architecture so a future refactor that breaks the contract trips loudly.
// =============================================================================

#[cfg(test)]
mod source_map_tests {
    use super::*;
    use std::collections::HashMap;

    fn user_source_span(file_id_raw: u32) -> Span {
        Span::new(0, 10, FileId::new(file_id_raw))
    }

    #[test]
    fn synthetic_origin_round_trips_through_source_file() {
        // Pin: SyntheticOrigin lands on SourceFile.synthetic_origin
        // and round-trips through `synthetic` constructor.
        let parent_span = user_source_span(7);
        let origin = SyntheticOrigin {
            parent_file: parent_span.file_id,
            call_site_span: parent_span,
            kind: SyntheticKind::DeriveExpansion,
        };
        let sf = SourceFile::synthetic(
            FileId::new(99),
            "<derive:Eq>".into(),
            "fn eq(...) { ... }".into(),
            origin,
        );
        let stored = sf.synthetic_origin
            .expect("synthetic_origin must be Some");
        assert_eq!(stored.parent_file, FileId::new(7));
        assert_eq!(stored.call_site_span, parent_span);
        assert_eq!(stored.kind, SyntheticKind::DeriveExpansion);
    }

    #[test]
    fn user_source_files_have_no_synthetic_origin() {
        // Pin: regular SourceFile::new produces a None origin so
        // user-source files don't accidentally appear synthetic.
        let sf = SourceFile::new(FileId::new(1), "main.vr".into(), "".into());
        assert!(sf.synthetic_origin.is_none(),
            "user-source SourceFile must have None origin");
        let mut sf2 = SourceFile::new(FileId::new(2), "main.vr".into(), "".into());
        sf2.path = Some(std::path::PathBuf::from("/tmp/main.vr"));
        assert!(sf2.synthetic_origin.is_none());
    }

    #[test]
    fn resolve_user_source_span_returns_self_with_empty_chain() {
        // Pin: when input span is already user-source (resolver
        // returns None for its FileId), result is (input, []).
        let user_span = user_source_span(1);
        let resolver = |_: FileId| None;
        let result = user_span.resolve_to_user_source(resolver);
        assert_eq!(result.user_span, user_span);
        assert!(result.expansion_chain.is_empty(),
            "user-source span must produce empty expansion_chain");
    }

    #[test]
    fn resolve_walks_single_synthetic_layer() {
        // Pin: a one-layer synthetic span resolves to its
        // call_site_span with chain = [kind].
        let user_file = FileId::new(1);
        let synth_file = FileId::new(99);
        let user_span = Span::new(100, 200, user_file);

        let mut chain: HashMap<FileId, SyntheticOrigin> = HashMap::new();
        chain.insert(synth_file, SyntheticOrigin {
            parent_file: user_file,
            call_site_span: user_span,
            kind: SyntheticKind::MacroExpansion,
        });

        let synth_span = Span::new(0, 50, synth_file);
        let resolver = |fid: FileId| chain.get(&fid).copied();
        let result = synth_span.resolve_to_user_source(resolver);

        assert_eq!(result.user_span, user_span,
            "single-layer resolution must return call_site_span");
        assert_eq!(result.expansion_chain,
            vec![SyntheticKind::MacroExpansion]);
    }

    #[test]
    fn resolve_walks_nested_synthetic_chain() {
        // Pin: derive-expanded code that itself macro-expands
        // resolves through both layers.  Chain order is leaf-to-
        // root: [DeriveExpansion, MacroExpansion] when the
        // synthetic file came from a derive within a macro body.
        let user_file = FileId::new(1);
        let macro_file = FileId::new(50);
        let derive_file = FileId::new(99);

        let user_span = Span::new(100, 200, user_file);
        let macro_call_in_user = user_span;
        let derive_call_in_macro = Span::new(0, 30, macro_file);

        let mut chain: HashMap<FileId, SyntheticOrigin> = HashMap::new();
        chain.insert(macro_file, SyntheticOrigin {
            parent_file: user_file,
            call_site_span: macro_call_in_user,
            kind: SyntheticKind::MacroExpansion,
        });
        chain.insert(derive_file, SyntheticOrigin {
            parent_file: macro_file,
            call_site_span: derive_call_in_macro,
            kind: SyntheticKind::DeriveExpansion,
        });

        // Span inside the derive output:
        let leaf_span = Span::new(0, 5, derive_file);
        let resolver = |fid: FileId| chain.get(&fid).copied();
        let result = leaf_span.resolve_to_user_source(resolver);

        assert_eq!(result.user_span, macro_call_in_user,
            "two-layer resolution must reach user source");
        assert_eq!(
            result.expansion_chain,
            vec![SyntheticKind::DeriveExpansion, SyntheticKind::MacroExpansion],
            "chain must be ordered leaf-to-root"
        );
    }

    #[test]
    fn resolve_terminates_on_cyclic_chain() {
        // Pin: defence-in-depth — a malformed synthetic chain that
        // cycles must NOT hang the renderer.  After MAX_DEPTH
        // (32) iterations the resolver returns whatever it has.
        let file_a = FileId::new(10);
        let file_b = FileId::new(20);
        let mut chain: HashMap<FileId, SyntheticOrigin> = HashMap::new();
        // A → B → A cycle.
        chain.insert(file_a, SyntheticOrigin {
            parent_file: file_b,
            call_site_span: Span::new(0, 5, file_b),
            kind: SyntheticKind::Other,
        });
        chain.insert(file_b, SyntheticOrigin {
            parent_file: file_a,
            call_site_span: Span::new(0, 5, file_a),
            kind: SyntheticKind::Other,
        });

        let leaf = Span::new(0, 5, file_a);
        let resolver = |fid: FileId| chain.get(&fid).copied();
        // Must not hang.  The chain must be capped.
        let result = leaf.resolve_to_user_source(resolver);
        assert!(
            result.expansion_chain.len() <= 32,
            "cycle defence must cap chain length at MAX_DEPTH; got {}",
            result.expansion_chain.len()
        );
    }

    #[test]
    fn synthetic_kind_label_is_human_readable() {
        // Pin: every variant has a label suitable for diagnostic
        // output ("in <label>").  Future SyntheticKind variants
        // must add a label (compile-time exhaustiveness via match).
        assert_eq!(SyntheticKind::MacroExpansion.label(), "macro expansion");
        assert_eq!(SyntheticKind::DeriveExpansion.label(), "@derive expansion");
        assert_eq!(SyntheticKind::Monomorphization.label(), "monomorphization");
        assert_eq!(SyntheticKind::DelegateBody.label(), "@delegate body synthesis");
        assert_eq!(SyntheticKind::Other.label(), "synthetic expansion");
    }
}
