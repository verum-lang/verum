//! Source File Registry for Error Diagnostics
//!
//! This module provides a registry for tracking source files and converting
//! byte-offset Spans to human-readable LineColSpans for error messages.
//!
//! # Design Principles
//!
//! 1. **Lazy Loading**: Files are loaded only when needed for error messages
//! 2. **Thread-Safe**: Uses RwLock for concurrent access
//! 3. **Efficient Lookup**: FileId-indexed HashMap for O(1) lookup
//! 4. **Fallback Handling**: Graceful degradation when source unavailable
//!
//! # Usage
//!
//! ```ignore
//! use verum_types::source_files::SourceFileRegistry;
//!
//! let registry = SourceFileRegistry::new();
//! registry.register_file(file_id, "path/to/file.vr", source_code);
//!
//! let line_col_span = registry.span_to_line_col(span);
//! ```

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use verum_common::span::{FileId, LineColSpan, SourceFile, Span};
use verum_common::Text;

/// A thread-safe registry of source files for span conversion.
///
/// This registry maintains mappings from FileId to SourceFile, enabling
/// accurate conversion of byte-offset Spans to line/column positions
/// for diagnostic messages.
#[derive(Debug, Clone)]
pub struct SourceFileRegistry {
    /// Map from FileId to SourceFile
    files: Arc<RwLock<HashMap<u32, SourceFile>>>,
    /// Map from file path to FileId for reverse lookup
    path_to_id: Arc<RwLock<HashMap<String, FileId>>>,
    /// Next available FileId
    next_id: Arc<RwLock<u32>>,
}

impl Default for SourceFileRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceFileRegistry {
    /// Create a new empty source file registry.
    pub fn new() -> Self {
        Self {
            files: Arc::new(RwLock::new(HashMap::new())),
            path_to_id: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(0)),
        }
    }

    /// Register a source file with the given content.
    ///
    /// Returns the FileId assigned to this file.
    pub fn register_file(&self, name: impl Into<String>, source: impl Into<String>) -> FileId {
        let name: String = name.into();
        let source: String = source.into();

        // Check if file already registered
        if let Some(&id) = self.path_to_id.read().unwrap().get(&name) {
            return id;
        }

        // Allocate new FileId
        let id = {
            let mut next = self.next_id.write().unwrap();
            let id = FileId::new(*next);
            *next += 1;
            id
        };

        // Create SourceFile and register it
        let source_file = SourceFile::new(id, name.clone(), source);

        self.files.write().unwrap().insert(id.raw(), source_file);
        self.path_to_id.write().unwrap().insert(name, id);

        id
    }

    /// Register a source file with a specific FileId.
    ///
    /// Use this when the FileId is already known (e.g., from the parser).
    pub fn register_file_with_id(
        &self,
        id: FileId,
        name: impl Into<String>,
        source: impl Into<String>,
    ) {
        let name: String = name.into();
        let source: String = source.into();

        // Skip if already registered
        if self.files.read().unwrap().contains_key(&id.raw()) {
            return;
        }

        let source_file = SourceFile::new(id, name.clone(), source);

        self.files.write().unwrap().insert(id.raw(), source_file);
        self.path_to_id.write().unwrap().insert(name, id);

        // Update next_id if necessary
        let mut next = self.next_id.write().unwrap();
        if id.raw() >= *next {
            *next = id.raw() + 1;
        }
    }

    /// Get the filename for a FileId.
    pub fn get_filename(&self, id: FileId) -> Option<Text> {
        self.files
            .read()
            .unwrap()
            .get(&id.raw())
            .map(|f| f.name.clone())
    }

    /// Convert a Span to a LineColSpan using the registered source files.
    ///
    /// Returns a LineColSpan with:
    /// - The actual filename if the file is registered
    /// - Accurate line and column numbers from the source
    ///
    /// If the file is not registered, returns a fallback span with
    /// the FileId in the filename for debugging.
    pub fn span_to_line_col(&self, span: Span) -> LineColSpan {
        // Handle dummy spans
        if span.is_dummy() {
            return LineColSpan::new("<generated>", 0, 0, 0);
        }

        // Look up the source file
        if let Some(source_file) = self.files.read().unwrap().get(&span.file_id.raw())
            && let Some(lc_span) = source_file.span_to_line_col(span)
        {
            return lc_span;
        }

        // Fallback: create a span with FileId info for debugging
        // This is better than "unknown" as it provides traceability
        LineColSpan::new(
            format!("<file:{}>", span.file_id.raw()),
            1,
            span.start as usize,
            span.end as usize,
        )
    }

    /// Get the source text for a span.
    pub fn span_text(&self, span: Span) -> Option<Text> {
        self.files
            .read()
            .unwrap()
            .get(&span.file_id.raw())
            .and_then(|f| f.span_text(span).map(|s| s.into()))
    }

    /// Get the line containing a span.
    pub fn span_line(&self, span: Span) -> Option<Text> {
        self.files
            .read()
            .unwrap()
            .get(&span.file_id.raw())
            .and_then(|f| f.span_line(span).map(|s| s.into()))
    }

    /// Check if a file is registered.
    pub fn has_file(&self, id: FileId) -> bool {
        self.files.read().unwrap().contains_key(&id.raw())
    }

    /// Get the number of registered files.
    pub fn file_count(&self) -> usize {
        self.files.read().unwrap().len()
    }
}

/// Global source file registry for use when a local registry is not available.
///
/// This is a fallback for code that doesn't have access to the TypeChecker's
/// registry. Prefer using the TypeChecker's registry when possible.
static GLOBAL_REGISTRY: std::sync::OnceLock<SourceFileRegistry> = std::sync::OnceLock::new();

/// Get the global source file registry.
pub fn global_registry() -> &'static SourceFileRegistry {
    GLOBAL_REGISTRY.get_or_init(SourceFileRegistry::new)
}

/// Register a file in the global registry.
///
/// This should be called during parsing to make source files available
/// for error diagnostics throughout compilation.
pub fn register_global_file(name: impl Into<String>, source: impl Into<String>) -> FileId {
    global_registry().register_file(name, source)
}

/// Register a file with a specific FileId in the global registry.
pub fn register_global_file_with_id(
    id: FileId,
    name: impl Into<String>,
    source: impl Into<String>,
) {
    global_registry().register_file_with_id(id, name, source);
}

/// Convert a Span to LineColSpan using the global registry.
///
/// This is a convenience function for code that doesn't have access to
/// a local registry. It provides a fallback that's better than "unknown".
pub fn span_to_line_col(span: Span) -> LineColSpan {
    global_registry().span_to_line_col(span)
}

/// Extract line and column numbers from a Span for call graph building.
///
/// Returns a tuple of (line, column) where both are 1-based.
/// For dummy spans or unknown files, returns (0, 0).
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts - Call Graph Verification
pub fn span_to_line_column(span: Span) -> (u32, u32) {
    let lc = span_to_line_col(span);
    (lc.line as u32, lc.column as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_file() {
        let registry = SourceFileRegistry::new();
        let source = "fn main() {\n    println!(\"hello\");\n}";
        let id = registry.register_file("test.vr", source);

        assert!(registry.has_file(id));
        assert_eq!(registry.get_filename(id), Some(Text::from("test.vr")));
    }

    #[test]
    fn test_span_to_line_col() {
        let registry = SourceFileRegistry::new();
        let source = "fn main() {\n    println!(\"hello\");\n}";
        let id = registry.register_file("test.vr", source);

        // Create a span pointing to "println" (offset 16-23)
        let span = Span::new(16, 23, id);
        let lc = registry.span_to_line_col(span);

        assert_eq!(lc.file, "test.vr");
        assert_eq!(lc.line, 2); // Second line (1-indexed)
        assert_eq!(lc.column, 5); // 5th column (1-indexed, after 4 spaces)
    }

    #[test]
    fn test_fallback_for_unknown_file() {
        let registry = SourceFileRegistry::new();
        let unknown_id = FileId::new(999);
        let span = Span::new(10, 20, unknown_id);

        let lc = registry.span_to_line_col(span);

        assert_eq!(lc.file, "<file:999>");
        assert_eq!(lc.line, 1);
    }

    #[test]
    fn test_dummy_span() {
        let registry = SourceFileRegistry::new();
        let span = Span::dummy();

        let lc = registry.span_to_line_col(span);

        assert_eq!(lc.file, "<generated>");
    }
}
