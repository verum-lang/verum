//! Parser integration for incremental parsing.
//!
//! This module provides the parser-side implementation that works with
//! `verum_syntax::incremental` to enable efficient incremental reparsing.
//!
//! # Integration Points
//!
//! 1. **ReparseContext dispatch**: Routes reparsing to the correct grammar rule
//! 2. **GreenNode construction**: Uses event-based parsing to build subtrees
//! 3. **LSP integration**: Provides high-level API for document changes
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_parser::incremental::IncrementalParserEngine;
//! use verum_syntax::{TextEdit, TextRange};
//!
//! let mut engine = IncrementalParserEngine::new();
//!
//! // Initial parse
//! let tree = engine.parse_full("fn foo() { }");
//!
//! // Incremental update
//! let edit = TextEdit::replace(TextRange::new(3, 6), "bar");
//! let new_tree = engine.apply_edit(&tree, &edit, "fn foo() { }");
//! ```

use std::time::Instant;

use verum_syntax::{
    ChangeTracker, GreenBuilder, GreenNode, IncrementalEngine, IncrementalStats,
    LspChange, LspRange, ReparseContext, SyntaxKind, TextEdit, TextRange, TextSize,
    lsp_change_to_edit,
};

use crate::syntax_bridge::{EventBasedParser, EventBasedParse};

// ============================================================================
// Parser-Integrated Incremental Engine
// ============================================================================

/// Incremental parsing engine integrated with the Verum parser.
///
/// This provides the high-level API for incremental parsing, combining:
/// - Tree manipulation from `verum_syntax::IncrementalEngine`
/// - Parsing logic from `EventBasedParser`
/// - Change tracking for LSP integration
#[derive(Debug)]
pub struct IncrementalParserEngine {
    /// The underlying incremental engine for tree manipulation.
    engine: IncrementalEngine,
    /// Change tracker for document synchronization.
    change_tracker: ChangeTracker,
    /// Current source text.
    source: String,
    /// Current green tree (if parsed).
    tree: Option<GreenNode>,
}

impl IncrementalParserEngine {
    /// Create a new incremental parser engine.
    pub fn new() -> Self {
        Self {
            engine: IncrementalEngine::new(),
            change_tracker: ChangeTracker::new(),
            source: String::new(),
            tree: None,
        }
    }

    /// Create an engine with initial source.
    pub fn with_source(source: &str) -> Self {
        let mut engine = Self::new();
        engine.parse_full(source);
        engine
    }

    /// Parse the full source text and cache the tree.
    pub fn parse_full(&mut self, source: &str) -> GreenNode {
        self.source = source.to_string();
        self.change_tracker = ChangeTracker::with_content(source);

        let parse = EventBasedParser::parse_source(source);
        self.tree = Some(parse.green.clone());

        parse.green
    }

    /// Get the current source text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Get the current tree (if parsed).
    pub fn tree(&self) -> Option<&GreenNode> {
        self.tree.as_ref()
    }

    /// Get incremental parsing statistics.
    pub fn stats(&self) -> &IncrementalStats {
        self.engine.stats()
    }

    /// Reset statistics.
    pub fn reset_stats(&mut self) {
        self.engine.reset_stats();
    }

    /// Check if there are pending edits.
    pub fn has_pending_edits(&self) -> bool {
        self.change_tracker.has_pending_edits()
    }

    /// Record an edit without immediately applying it.
    ///
    /// This is useful for batching edits in rapid succession.
    pub fn record_edit(&mut self, edit: TextEdit) {
        self.change_tracker.record_edit(edit);
    }

    /// Apply a single edit and return the new tree.
    pub fn apply_edit(&mut self, edit: TextEdit) -> GreenNode {
        // If no tree exists, parse first
        if self.tree.is_none() {
            let source = self.source.clone();
            self.parse_full(&source);
        }

        let tree = self.tree.clone().unwrap();
        let source = self.source.clone();

        let new_tree = self.engine.apply_edit(
            &tree,
            &edit,
            Self::reparse_with_context_static,
            &source,
        );

        // Update source
        self.source = edit.apply(&source);
        self.tree = Some(new_tree.clone());
        self.change_tracker.update_hash(&self.source);

        new_tree
    }

    /// Apply all pending edits and return the new tree.
    pub fn apply_pending_edits(&mut self) -> GreenNode {
        if !self.change_tracker.has_pending_edits() {
            if self.tree.is_none() {
                let source = self.source.clone();
                self.parse_full(&source);
            }
            return self.tree.clone().unwrap();
        }

        // Merge edits for efficiency
        self.change_tracker.merge_edits();

        // Apply each edit incrementally
        let edits: Vec<TextEdit> = self.change_tracker.pending_edits().to_vec();
        self.change_tracker.clear_pending();

        for edit in edits {
            // Get or create tree first
            if self.tree.is_none() {
                let source = self.source.clone();
                self.parse_full(&source);
            }
            let tree = self.tree.clone().unwrap();
            let source = self.source.clone();

            let new_tree = self.engine.apply_edit(
                &tree,
                &edit,
                Self::reparse_with_context_static,
                &source,
            );

            self.source = edit.apply(&source);
            self.tree = Some(new_tree);
        }

        self.change_tracker.update_hash(&self.source);
        self.tree.clone().unwrap()
    }

    /// Apply an LSP-style content change.
    pub fn apply_lsp_change(&mut self, change: LspChange) -> GreenNode {
        let edit = lsp_change_to_edit(change, &self.source);
        self.apply_edit(edit)
    }

    /// Apply multiple LSP-style content changes (e.g., from didChange).
    pub fn apply_lsp_changes(&mut self, changes: Vec<LspChange>) -> GreenNode {
        for change in changes {
            let edit = lsp_change_to_edit(change, &self.source);
            self.record_edit(edit);
        }
        self.apply_pending_edits()
    }

    /// Check if incremental parsing is beneficial for the current edits.
    pub fn should_use_incremental(&self) -> bool {
        let tree = match &self.tree {
            Some(t) => t,
            None => return false,
        };

        // Compose pending edits if possible
        let composed = self.change_tracker.compose();
        match composed {
            Some(edit) => self.engine.should_use_incremental(tree, &edit),
            None => {
                // Multiple non-contiguous edits - use heuristics
                let pending = self.change_tracker.pending_edits();
                let total_edit_size: usize = pending.iter()
                    .map(|e| e.new_text.len() + e.range.len() as usize)
                    .sum();
                let tree_size = tree.width() as usize;

                // If total edits are less than 20% of tree, use incremental
                total_edit_size < tree_size / 5
            }
        }
    }

    /// Reparse source with the appropriate context (static version for use in closures).
    fn reparse_with_context_static(source: &str, context: ReparseContext) -> GreenNode {
        match context {
            ReparseContext::Module => {
                let parse = EventBasedParser::parse_source(source);
                parse.green
            }
            ReparseContext::Item => {
                let parse = EventBasedParser::parse_item(source);
                parse.green
            }
            ReparseContext::Block => {
                let parse = EventBasedParser::parse_block(source);
                parse.green
            }
            ReparseContext::Statement => {
                let parse = EventBasedParser::parse_statement(source);
                parse.green
            }
            ReparseContext::Expression => {
                let parse = EventBasedParser::parse_expression(source);
                parse.green
            }
            ReparseContext::Type => {
                let parse = EventBasedParser::parse_type(source);
                parse.green
            }
            ReparseContext::Unknown => {
                // Fall back to full module parse
                let parse = EventBasedParser::parse_source(source);
                parse.green
            }
        }
    }
}

impl Default for IncrementalParserEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// LSP Document Integration
// ============================================================================

/// A document with incremental parsing support for LSP integration.
///
/// This provides a complete document management solution with:
/// - Incremental parsing on edits
/// - Version tracking
/// - Parse error collection
#[derive(Debug)]
pub struct IncrementalDocument {
    /// The incremental parser engine.
    engine: IncrementalParserEngine,
    /// Document version (for LSP).
    version: i32,
    /// Parse errors from the last parse.
    errors: Vec<verum_syntax::ParseError>,
}

impl IncrementalDocument {
    /// Create a new document with initial content.
    pub fn new(content: &str, version: i32) -> Self {
        let mut engine = IncrementalParserEngine::new();
        let parse = EventBasedParser::parse_source(content);
        engine.source = content.to_string();
        engine.tree = Some(parse.green);
        engine.change_tracker = ChangeTracker::with_content(content);

        Self {
            engine,
            version,
            errors: parse.errors,
        }
    }

    /// Get the current source text.
    pub fn text(&self) -> &str {
        self.engine.source()
    }

    /// Get the current version.
    pub fn version(&self) -> i32 {
        self.version
    }

    /// Get the current green tree.
    pub fn tree(&self) -> Option<&GreenNode> {
        self.engine.tree()
    }

    /// Get parse errors from the last parse.
    pub fn errors(&self) -> &[verum_syntax::ParseError] {
        &self.errors
    }

    /// Check if the document parsed without errors.
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get parsing statistics.
    pub fn stats(&self) -> &IncrementalStats {
        self.engine.stats()
    }

    /// Apply content changes from an LSP didChange notification.
    pub fn apply_changes(&mut self, changes: Vec<LspChange>, version: i32) {
        self.version = version;

        for change in changes {
            let edit = lsp_change_to_edit(change, self.engine.source());
            self.engine.record_edit(edit);
        }

        // Apply all pending changes
        let tree = self.engine.apply_pending_edits();

        // Re-parse to collect errors (the tree is already updated)
        let parse = EventBasedParser::parse_source(self.engine.source());
        self.errors = parse.errors;
    }

    /// Update the entire document content (full sync).
    pub fn set_content(&mut self, content: &str, version: i32) {
        self.version = version;
        let parse = EventBasedParser::parse_source(content);
        self.engine.source = content.to_string();
        self.engine.tree = Some(parse.green);
        self.engine.change_tracker = ChangeTracker::with_content(content);
        self.errors = parse.errors;
    }
}

// ============================================================================
// Benchmarking Support
// ============================================================================

/// Result of an incremental vs full parse benchmark.
#[derive(Clone, Debug)]
pub struct BenchmarkResult {
    /// Time for incremental parse (nanoseconds).
    pub incremental_ns: u64,
    /// Time for full parse (nanoseconds).
    pub full_parse_ns: u64,
    /// Speedup factor (full_time / incremental_time).
    pub speedup: f64,
    /// Number of nodes reused in incremental parse.
    pub nodes_reused: u64,
    /// Whether the trees are equivalent.
    pub trees_equal: bool,
    /// Source size in bytes.
    pub source_size: usize,
    /// Edit size in bytes.
    pub edit_size: usize,
}

/// Benchmark incremental parsing vs full parsing.
///
/// Returns detailed statistics about the performance of both approaches.
pub fn benchmark_incremental_vs_full(
    source: &str,
    edit: &TextEdit,
    iterations: usize,
) -> BenchmarkResult {
    let new_source = edit.apply(source);

    // Benchmark incremental parsing
    let mut incremental_total = 0u64;
    let mut nodes_reused = 0u64;
    let mut incremental_tree = None;

    for _ in 0..iterations {
        let mut engine = IncrementalParserEngine::with_source(source);

        let start = Instant::now();
        let tree = engine.apply_edit(edit.clone());
        incremental_total += start.elapsed().as_nanos() as u64;

        nodes_reused = engine.stats().nodes_reused;
        incremental_tree = Some(tree);
    }

    // Benchmark full parsing
    let mut full_total = 0u64;
    let mut full_tree = None;

    for _ in 0..iterations {
        let start = Instant::now();
        let parse = EventBasedParser::parse_source(&new_source);
        full_total += start.elapsed().as_nanos() as u64;

        full_tree = Some(parse.green);
    }

    let incremental_avg = incremental_total / iterations as u64;
    let full_avg = full_total / iterations as u64;

    // Compare trees
    let trees_equal = match (&incremental_tree, &full_tree) {
        (Some(inc), Some(full)) => {
            // Compare by text reconstruction (semantic equality)
            let inc_text: String = inc.text();
            let full_text: String = full.text();
            inc_text == full_text
        }
        _ => false,
    };

    BenchmarkResult {
        incremental_ns: incremental_avg,
        full_parse_ns: full_avg,
        speedup: if incremental_avg > 0 {
            full_avg as f64 / incremental_avg as f64
        } else {
            f64::INFINITY
        },
        nodes_reused,
        trees_equal,
        source_size: source.len(),
        edit_size: edit.new_text.len() + edit.range.len() as usize,
    }
}

/// Run a comprehensive benchmark suite.
pub fn run_benchmark_suite(source: &str) -> Vec<(String, BenchmarkResult)> {
    let mut results = Vec::new();

    // Single character insert at beginning
    let edit = TextEdit::insert(0, "x");
    results.push((
        "Single char insert (beginning)".to_string(),
        benchmark_incremental_vs_full(source, &edit, 10),
    ));

    // Single character insert in middle
    let mid = source.len() as TextSize / 2;
    let edit = TextEdit::insert(mid, "x");
    results.push((
        "Single char insert (middle)".to_string(),
        benchmark_incremental_vs_full(source, &edit, 10),
    ));

    // Single character delete
    if source.len() > 1 {
        let edit = TextEdit::delete(TextRange::new(0, 1));
        results.push((
            "Single char delete".to_string(),
            benchmark_incremental_vs_full(source, &edit, 10),
        ));
    }

    // Word replacement
    if source.len() > 10 {
        let edit = TextEdit::replace(TextRange::new(0, 5.min(source.len() as TextSize)), "hello");
        results.push((
            "Word replacement".to_string(),
            benchmark_incremental_vs_full(source, &edit, 10),
        ));
    }

    // Multi-line edit (10 chars)
    let edit = TextEdit::insert(0, "// comment\n");
    results.push((
        "Multi-line insert (11 chars)".to_string(),
        benchmark_incremental_vs_full(source, &edit, 10),
    ));

    results
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SOURCE: &str = r#"
fn main() {
    let x = 42;
    let y = x + 1;
    print(y);
}

fn helper(n: Int) -> Int {
    n * 2
}
"#;

    #[test]
    fn test_incremental_engine_creation() {
        let engine = IncrementalParserEngine::new();
        assert!(engine.tree().is_none());
        assert_eq!(engine.source(), "");
    }

    #[test]
    fn test_full_parse() {
        let mut engine = IncrementalParserEngine::new();
        let tree = engine.parse_full(SAMPLE_SOURCE);

        assert_eq!(tree.kind(), SyntaxKind::SOURCE_FILE);
        assert_eq!(engine.source(), SAMPLE_SOURCE);
    }

    #[test]
    fn test_incremental_single_char_edit() {
        let mut engine = IncrementalParserEngine::with_source(SAMPLE_SOURCE);

        let edit = TextEdit::insert(20, "x");
        let new_tree = engine.apply_edit(edit);

        assert_eq!(new_tree.kind(), SyntaxKind::SOURCE_FILE);
        assert!(engine.source().contains("letx"));
    }

    #[test]
    fn test_incremental_word_replacement() {
        let source = "fn foo() { }";
        let mut engine = IncrementalParserEngine::with_source(source);

        let edit = TextEdit::replace(TextRange::new(3, 6), "bar");
        let new_tree = engine.apply_edit(edit);

        assert_eq!(new_tree.kind(), SyntaxKind::SOURCE_FILE);
        assert_eq!(engine.source(), "fn bar() { }");
    }

    #[test]
    fn test_lsp_document() {
        let mut doc = IncrementalDocument::new(SAMPLE_SOURCE, 1);

        assert_eq!(doc.version(), 1);
        assert!(!doc.text().is_empty());

        // Apply a change
        let change = LspChange {
            range: Some(LspRange {
                start_line: 2,
                start_col: 8,
                end_line: 2,
                end_col: 9,
            }),
            text: "z".to_string(),
        };

        doc.apply_changes(vec![change], 2);
        assert_eq!(doc.version(), 2);
    }

    #[test]
    fn test_pending_edits() {
        let mut engine = IncrementalParserEngine::with_source("fn foo() { }");

        engine.record_edit(TextEdit::insert(3, "x"));
        engine.record_edit(TextEdit::insert(4, "y"));

        assert!(engine.has_pending_edits());

        let tree = engine.apply_pending_edits();
        assert_eq!(tree.kind(), SyntaxKind::SOURCE_FILE);
        assert!(!engine.has_pending_edits());
    }

    #[test]
    fn test_benchmark_result() {
        let source = "fn main() { let x = 1; }";
        let edit = TextEdit::insert(10, " ");

        let result = benchmark_incremental_vs_full(source, &edit, 3);

        assert!(result.incremental_ns > 0);
        assert!(result.full_parse_ns > 0);
        assert_eq!(result.source_size, source.len());
    }

    #[test]
    fn test_should_use_incremental() {
        let source = "fn foo() { ".to_string() + &"let x = 1; ".repeat(100) + "}";
        let engine = IncrementalParserEngine::with_source(&source);

        // Small edit on large tree should use incremental
        let mut engine_with_edit = IncrementalParserEngine::with_source(&source);
        engine_with_edit.record_edit(TextEdit::insert(10, "x"));
        assert!(engine_with_edit.should_use_incremental());
    }
}
