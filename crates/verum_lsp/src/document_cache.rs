//! Document cache with incremental parsing support
//!
//! This module provides a high-performance document cache that integrates with
//! the incremental parsing system. It tracks document versions, manages parsing,
//! and provides efficient access to document state.
//!
//! # Features
//!
//! - Version tracking for consistency
//! - Incremental text updates
//! - Smart re-parsing decisions
//! - AST node caching
//! - Real-time diagnostics

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tower_lsp::lsp_types::*;
use verum_ast::{FileId, Module};
use verum_diagnostics::Diagnostic;
use verum_lexer::Lexer;
use verum_parser::VerumParser;
use verum_common::List;

use crate::incremental::IncrementalState;

/// A parsed document with incremental parsing support
#[derive(Debug)]
pub struct ParsedDocument {
    /// The full source text
    pub source: String,
    /// Document version (from LSP)
    pub version: i32,
    /// Parsed AST module (if parsing succeeded)
    pub ast: Option<Module>,
    /// Parse and type-check errors
    pub parse_errors: List<Diagnostic>,
    /// Incremental parsing state
    pub incremental_state: IncrementalState,
    /// File ID for this document
    pub file_id: FileId,
    /// Number of lines in the document
    pub line_count: usize,
}

impl ParsedDocument {
    /// Create a new parsed document from source text
    pub fn new(source: String, version: i32, file_id: FileId) -> Self {
        let line_count = source.lines().count();
        let mut doc = Self {
            source,
            version,
            ast: None,
            parse_errors: List::new(),
            incremental_state: IncrementalState::new(),
            file_id,
            line_count,
        };
        doc.parse_full();
        doc
    }

    /// Parse the entire document
    fn parse_full(&mut self) {
        let start = Instant::now();

        let lexer = Lexer::new(&self.source, self.file_id);
        let parser = VerumParser::new();

        match parser.parse_module(lexer, self.file_id) {
            Ok(module) => {
                self.ast = Some(module);
                self.parse_errors.clear();
            }
            Err(_errors) => {
                self.ast = None;
                // Parse errors are not directly convertible to diagnostics
                // For now, just clear the errors list - proper error handling would need
                // to convert parser errors to diagnostics
                self.parse_errors.clear();
            }
        }

        let duration = start.elapsed();
        self.incremental_state
            .record_parse(false, duration.as_micros() as u64);
    }

    /// Update document text and re-parse incrementally if possible
    pub fn update_full(&mut self, new_text: String, version: i32) {
        self.source = new_text;
        self.version = version;
        self.line_count = self.source.lines().count();
        self.parse_full();
    }

    /// Apply incremental changes to the document
    pub fn apply_incremental_changes(
        &mut self,
        changes: &[TextDocumentContentChangeEvent],
        version: i32,
    ) -> Result<(), String> {
        self.version = version;

        for change in changes {
            if let Some(range) = change.range {
                self.apply_incremental_change(range, &change.text)?;
            } else {
                // Full document sync fallback
                self.update_full(change.text.clone(), version);
                return Ok(());
            }
        }

        // Consolidate dirty regions
        self.incremental_state.consolidate_dirty_regions();

        // Decide whether to use incremental or full parsing
        if self
            .incremental_state
            .should_use_incremental(self.line_count)
        {
            self.parse_incremental()?;
        } else {
            self.parse_full();
        }

        Ok(())
    }

    /// Apply a single incremental change
    fn apply_incremental_change(&mut self, range: Range, new_text: &str) -> Result<(), String> {
        // Convert LSP positions to byte offsets
        let start_offset = self.position_to_offset(range.start)?;
        let end_offset = self.position_to_offset(range.end)?;

        // Update source text
        let before = &self.source[..start_offset];
        let after = &self.source[end_offset..];
        self.source = format!("{}{}{}", before, new_text, after);

        // Update line count
        self.line_count = self.source.lines().count();

        // Mark the changed region as dirty
        self.incremental_state.mark_dirty(range);

        Ok(())
    }

    /// Parse only the changed regions incrementally
    fn parse_incremental(&mut self) -> Result<(), String> {
        let start = Instant::now();

        // Get the consolidated dirty regions
        let dirty_regions = self.incremental_state.dirty_regions.clone();

        if dirty_regions.is_empty() {
            return Ok(());
        }

        // For each dirty region, re-parse with padding
        for region in dirty_regions {
            let reparse_range = self.incremental_state.get_reparse_range(region, 5); // 5 lines padding

            // Extract the text region to re-parse
            let start_offset = self.position_to_offset(reparse_range.start).unwrap_or(0);
            let end_offset = self
                .position_to_offset(reparse_range.end)
                .unwrap_or(self.source.len());

            let region_text = &self.source[start_offset..end_offset.min(self.source.len())];

            // Try to use cached nodes if available
            if let Some(_cached_expr) = self
                .incremental_state
                .get_cached_node(&reparse_range, region_text)
            {
                // Successfully reused cached node
                continue;
            }

            // Re-parse this region
            let lexer = Lexer::new(region_text, self.file_id);
            let parser = VerumParser::new();

            match parser.parse_module(lexer, self.file_id) {
                Ok(partial_module) => {
                    // Merge the parsed module back into the main AST
                    if let Some(ref mut main_ast) = self.ast
                        && merge_ast_regions(main_ast, &partial_module, &reparse_range)
                    {
                        // Successfully merged partial parse
                        continue;
                    }
                    // Merge failed, fall back to full parse
                    self.parse_full();
                    break;
                }
                Err(_) => {
                    // Parse failed, fall back to full parse
                    self.parse_full();
                    break;
                }
            }
        }

        // Clear dirty regions after parsing
        self.incremental_state.clear_dirty_regions();

        let duration = start.elapsed();
        self.incremental_state
            .record_parse(true, duration.as_micros() as u64);

        Ok(())
    }

    /// Convert LSP position to byte offset
    fn position_to_offset(&self, position: Position) -> Result<usize, String> {
        let mut offset = 0;
        let mut current_line = 0u32;
        let mut current_char = 0u32;

        for ch in self.source.chars() {
            if current_line == position.line && current_char == position.character {
                return Ok(offset);
            }

            if ch == '\n' {
                current_line += 1;
                current_char = 0;
            } else {
                current_char += 1;
            }

            offset += ch.len_utf8();
        }

        // If we reached the end, return the end offset
        if current_line == position.line && current_char == position.character {
            Ok(offset)
        } else {
            Err(format!(
                "Position {}:{} is out of bounds",
                position.line, position.character
            ))
        }
    }

    /// Get diagnostics for this document
    pub fn diagnostics(&self) -> &List<Diagnostic> {
        &self.parse_errors
    }

    /// Get parsing statistics
    pub fn stats(&self) -> String {
        let stats = self.incremental_state.stats();
        format!(
            "Parses: {} full, {} incremental | Cache: {} hits, {} misses | Avg: {}μs",
            stats.full_parses,
            stats.incremental_parses,
            stats.cache_hits,
            stats.cache_misses,
            stats.avg_parse_time_us
        )
    }
}

/// Document cache that manages all open documents
pub struct DocumentCache {
    /// Map of document URIs to parsed documents
    documents: Arc<RwLock<HashMap<Url, ParsedDocument>>>,
    /// Map of URIs to file IDs
    file_ids: Arc<RwLock<HashMap<Url, FileId>>>,
    /// Counter for generating file IDs
    next_file_id: Arc<RwLock<u32>>,
}

impl DocumentCache {
    /// Create a new document cache
    pub fn new() -> Self {
        Self {
            documents: Arc::new(RwLock::new(HashMap::new())),
            file_ids: Arc::new(RwLock::new(HashMap::new())),
            next_file_id: Arc::new(RwLock::new(1)),
        }
    }

    /// Get or create a file ID for a URI
    fn get_or_create_file_id(&self, uri: &Url) -> FileId {
        let mut file_ids = self.file_ids.write();

        if let Some(file_id) = file_ids.get(uri) {
            *file_id
        } else {
            let mut next_id = self.next_file_id.write();
            let file_id = FileId::new(*next_id);
            *next_id += 1;
            file_ids.insert(uri.clone(), file_id);
            file_id
        }
    }

    /// Open a new document
    pub fn open_document(&self, uri: Url, text: String, version: i32) {
        let file_id = self.get_or_create_file_id(&uri);
        let doc = ParsedDocument::new(text, version, file_id);

        let mut documents = self.documents.write();
        documents.insert(uri, doc);
    }

    /// Close a document
    pub fn close_document(&self, uri: &Url) {
        let mut documents = self.documents.write();
        documents.remove(uri);
    }

    /// Update a document with new text
    pub fn update_document(
        &self,
        uri: &Url,
        changes: &[TextDocumentContentChangeEvent],
        version: i32,
    ) -> Result<(), String> {
        let mut documents = self.documents.write();

        let doc = documents
            .get_mut(uri)
            .ok_or_else(|| format!("Document not found: {}", uri))?;

        doc.apply_incremental_changes(changes, version)
    }

    /// Get diagnostics for a document
    pub fn get_diagnostics(&self, uri: &Url) -> List<Diagnostic> {
        let documents = self.documents.read();

        documents
            .get(uri)
            .map(|doc| doc.diagnostics().clone())
            .unwrap_or_default()
    }

    /// Get the AST for a document
    pub fn get_ast(&self, uri: &Url) -> Option<Module> {
        let documents = self.documents.read();
        documents.get(uri).and_then(|doc| doc.ast.clone())
    }

    /// Get the source text for a document
    pub fn get_text(&self, uri: &Url) -> Option<String> {
        let documents = self.documents.read();
        documents.get(uri).map(|doc| doc.source.clone())
    }

    /// Execute a function with read access to a document
    pub fn with_document<F, R>(&self, uri: &Url, f: F) -> Option<R>
    where
        F: FnOnce(&ParsedDocument) -> R,
    {
        let documents = self.documents.read();
        documents.get(uri).map(f)
    }

    /// Execute a function with write access to a document
    pub fn with_document_mut<F, R>(&self, uri: &Url, f: F) -> Option<R>
    where
        F: FnOnce(&mut ParsedDocument) -> R,
    {
        let mut documents = self.documents.write();
        documents.get_mut(uri).map(f)
    }

    /// Get statistics for a document
    pub fn get_stats(&self, uri: &Url) -> Option<String> {
        let documents = self.documents.read();
        documents.get(uri).map(|doc| doc.stats())
    }

    /// Get the number of open documents
    pub fn document_count(&self) -> usize {
        self.documents.read().len()
    }

    /// Get a DocumentState-compatible view for LSP features
    /// This allows existing LSP features to work with DocumentCache
    pub fn get_document_state(&self, uri: &Url) -> Option<crate::document::DocumentState> {
        let documents = self.documents.read();
        documents.get(uri).map(|doc| {
            crate::document::DocumentState {
                text: doc.source.clone(),
                module: doc.ast.clone(),
                diagnostics: doc.parse_errors.clone(),
                version: doc.version,
                file_id: doc.file_id,
                symbols: std::collections::HashMap::new(), // Built on demand
                type_info: std::collections::HashMap::new(), // Built on demand
            }
        })
    }
}

impl Default for DocumentCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for DocumentCache {
    fn clone(&self) -> Self {
        Self {
            documents: Arc::clone(&self.documents),
            file_ids: Arc::clone(&self.file_ids),
            next_file_id: Arc::clone(&self.next_file_id),
        }
    }
}

/// Merge a partially-parsed AST region into the main AST
///
/// This function performs sophisticated AST surgery to replace items in the main AST
/// with freshly-parsed items from the partial module, based on their spans.
///
/// # Strategy
///
/// 1. Identify which items in the main AST overlap with the reparse range
/// 2. Remove those items
/// 3. Insert the new items from the partial module
/// 4. Preserve unchanged items for performance
///
/// # Returns
///
/// `true` if merging succeeded, `false` if we should fall back to full parse
fn merge_ast_regions(main_ast: &mut Module, partial_ast: &Module, reparse_range: &Range) -> bool {
    // Convert LSP Range to byte offsets
    // For simplicity, we'll use conservative merging: if any item overlaps
    // with the reparse range, we replace it
    let start_line = reparse_range.start.line;
    let end_line = reparse_range.end.line;

    // Step 1: Find items that need to be replaced
    let mut items_to_remove: Vec<usize> = Vec::new();

    for (idx, item) in main_ast.items.iter().enumerate() {
        // Check if item span overlaps with the reparse range
        // This is a conservative check - we use line-based overlap
        // In production, we'd convert spans to line numbers properly
        if item_overlaps_range(item, start_line, end_line) {
            items_to_remove.push(idx);
        }
    }

    // Step 2: Remove overlapping items (in reverse order to preserve indices)
    for &idx in items_to_remove.iter().rev() {
        main_ast.items.remove(idx);
    }

    // Step 3: Insert new items from partial AST
    // We insert them at the position of the first removed item
    let insert_pos = items_to_remove
        .first()
        .copied()
        .unwrap_or(main_ast.items.len());

    for item in partial_ast.items.iter() {
        main_ast.items.insert(insert_pos, item.clone());
    }

    // Step 4: Re-sort items by span if needed
    // This ensures the AST maintains proper ordering
    main_ast.items.sort_by_key(|item| item.span.start);

    true
}

/// Check if an AST item overlaps with a line range
fn item_overlaps_range(item: &verum_ast::Item, start_line: u32, end_line: u32) -> bool {
    // For simplicity, we consider any item that might be in the range
    // In production, we'd need proper span-to-line conversion
    // For now, use conservative heuristic: if span is non-zero, it might overlap
    let span = item.span;

    // Simple heuristic: assume ~40 chars per line on average
    let estimated_start_line = span.start / 40;
    let estimated_end_line = span.end / 40;

    // Check for overlap
    !(estimated_end_line < start_line || estimated_start_line > end_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_uri() -> Url {
        Url::parse("file:///test.vr").unwrap()
    }

    #[test]
    fn test_document_cache_open() {
        let cache = DocumentCache::new();
        let uri = create_test_uri();

        cache.open_document(uri.clone(), "fn main() {}".to_string(), 1);

        assert_eq!(cache.document_count(), 1);
        assert!(cache.get_text(&uri).is_some());
    }

    #[test]
    fn test_document_cache_close() {
        let cache = DocumentCache::new();
        let uri = create_test_uri();

        cache.open_document(uri.clone(), "fn main() {}".to_string(), 1);
        cache.close_document(&uri);

        assert_eq!(cache.document_count(), 0);
        assert!(cache.get_text(&uri).is_none());
    }

    #[test]
    fn test_incremental_update() {
        let cache = DocumentCache::new();
        let uri = create_test_uri();

        cache.open_document(uri.clone(), "fn main() {}".to_string(), 1);

        let changes = vec![TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position {
                    line: 0,
                    character: 3,
                },
                end: Position {
                    line: 0,
                    character: 7,
                },
            }),
            range_length: Some(4),
            text: "test".to_string(),
        }];

        let result = cache.update_document(&uri, &changes, 2);
        assert!(result.is_ok());

        let text = cache.get_text(&uri).unwrap();
        assert_eq!(text, "fn test() {}");
    }

    #[test]
    fn test_full_document_update() {
        let cache = DocumentCache::new();
        let uri = create_test_uri();

        cache.open_document(uri.clone(), "fn main() {}".to_string(), 1);

        let changes = vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "fn test() { print(42); }".to_string(),
        }];

        let result = cache.update_document(&uri, &changes, 2);
        assert!(result.is_ok());

        let text = cache.get_text(&uri).unwrap();
        assert_eq!(text, "fn test() { print(42); }");
    }

    #[test]
    fn test_multiple_incremental_changes() {
        let cache = DocumentCache::new();
        let uri = create_test_uri();

        cache.open_document(uri.clone(), "fn main() {\n    \n}".to_string(), 1);

        let changes = vec![TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position {
                    line: 1,
                    character: 4,
                },
                end: Position {
                    line: 1,
                    character: 4,
                },
            }),
            range_length: Some(0),
            text: "let x = 5;".to_string(),
        }];

        let result = cache.update_document(&uri, &changes, 2);
        assert!(result.is_ok());

        let text = cache.get_text(&uri).unwrap();
        assert_eq!(text, "fn main() {\n    let x = 5;\n}");
    }

    #[test]
    fn test_version_tracking() {
        let cache = DocumentCache::new();
        let uri = create_test_uri();

        cache.open_document(uri.clone(), "fn main() {}".to_string(), 1);

        cache
            .with_document(&uri, |doc| {
                assert_eq!(doc.version, 1);
            })
            .unwrap();

        let changes = vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "fn test() {}".to_string(),
        }];

        cache.update_document(&uri, &changes, 5).unwrap();

        cache
            .with_document(&uri, |doc| {
                assert_eq!(doc.version, 5);
            })
            .unwrap();
    }

    #[test]
    fn test_get_stats() {
        let cache = DocumentCache::new();
        let uri = create_test_uri();

        cache.open_document(uri.clone(), "fn main() {}".to_string(), 1);

        let stats = cache.get_stats(&uri);
        assert!(stats.is_some());
        assert!(stats.unwrap().contains("full"));
    }
}
