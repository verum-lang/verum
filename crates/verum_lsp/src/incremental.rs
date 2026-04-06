//! Incremental parsing infrastructure
//!
//! This module provides the core incremental parsing system for the LSP server.
//! It enables efficient re-parsing of documents by tracking changes and reusing
//! unchanged AST subtrees.
//!
//! # Performance Characteristics
//!
//! - Document sync: <10ms for typical changes
//! - Incremental parse: <50ms for 1000 LOC files
//! - Memory overhead: <10MB per open document
//!
//! # Architecture
//!
//! The incremental parsing system works by:
//! 1. Tracking changed text ranges when documents are edited
//! 2. Identifying which AST nodes are affected by the changes
//! 3. Re-parsing only the affected regions with some padding
//! 4. Reusing cached, unchanged AST subtrees
//! 5. Updating parent nodes incrementally
//!
//! # Integration with verum_syntax and verum_parser
//!
//! This module integrates with:
//! - `verum_syntax::IncrementalEngine` for green tree manipulation
//! - `verum_parser::IncrementalParserEngine` for parsing integration
//! - `verum_parser::IncrementalDocument` for high-level document management

use std::collections::HashMap;
use std::time::Instant;
use tower_lsp::lsp_types::{Position, Range};
use verum_ast::{Expr, ExprKind, Module, Span};
use verum_syntax::{
    GreenNode, SyntaxNode, TextEdit, TextRange, TextSize,
    LspChange as SyntaxLspChange, LspRange as SyntaxLspRange,
};
use verum_parser::IncrementalDocument;

/// A hashable range key for HashMap lookups
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RangeKey {
    start_line: u32,
    start_character: u32,
    end_line: u32,
    end_character: u32,
}

impl From<&Range> for RangeKey {
    fn from(range: &Range) -> Self {
        Self {
            start_line: range.start.line,
            start_character: range.start.character,
            end_line: range.end.line,
            end_character: range.end.character,
        }
    }
}

impl From<Range> for RangeKey {
    fn from(range: Range) -> Self {
        Self::from(&range)
    }
}

/// Tracks incremental parsing state for a document
#[derive(Debug, Clone)]
pub struct IncrementalState {
    /// Ranges that have changed since the last parse
    changed_ranges: Vec<Range>,
    /// Cache of unchanged AST subtrees mapped by their text range
    cached_nodes: HashMap<RangeKey, CachedNode>,
    /// Regions that need re-parsing
    pub(crate) dirty_regions: Vec<Range>,
    /// Timestamp of the last parse operation
    last_parse_time: Instant,
    /// Statistics for performance monitoring
    stats: ParseStats,
}

/// A cached AST node with its associated metadata
#[derive(Debug, Clone)]
pub struct CachedNode {
    /// The cached expression node
    pub expr: Box<Expr>,
    /// The text range this node covers
    pub range: Range,
    /// Hash of the text content (for validation)
    pub content_hash: u64,
    /// Size in bytes of the cached subtree
    pub size_bytes: usize,
}

/// Statistics about parsing operations
#[derive(Debug, Clone, Default)]
pub struct ParseStats {
    /// Number of full parses performed
    pub full_parses: u64,
    /// Number of incremental parses performed
    pub incremental_parses: u64,
    /// Number of cache hits (reused nodes)
    pub cache_hits: u64,
    /// Number of cache misses
    pub cache_misses: u64,
    /// Total time spent parsing (microseconds)
    pub total_parse_time_us: u64,
    /// Average parse time (microseconds)
    pub avg_parse_time_us: u64,
    /// Total bytes cached
    pub total_cached_bytes: usize,
}

impl IncrementalState {
    /// Create a new incremental state
    pub fn new() -> Self {
        Self {
            changed_ranges: Vec::new(),
            cached_nodes: HashMap::new(),
            dirty_regions: Vec::new(),
            last_parse_time: Instant::now(),
            stats: ParseStats::default(),
        }
    }

    /// Mark a range as dirty (needs re-parsing)
    pub fn mark_dirty(&mut self, range: Range) {
        self.changed_ranges.push(range);
        self.dirty_regions.push(range);

        // Invalidate cached nodes that overlap with the dirty range
        let keys_to_remove: Vec<RangeKey> = self
            .cached_nodes
            .iter()
            .filter(|(_, cached_node)| ranges_overlap(&cached_node.range, &range))
            .map(|(key, _)| *key)
            .collect();

        for key in keys_to_remove {
            if let Some(node) = self.cached_nodes.remove(&key) {
                self.stats.total_cached_bytes = self
                    .stats
                    .total_cached_bytes
                    .saturating_sub(node.size_bytes);
            }
        }
    }

    /// Cache an AST node for reuse
    pub fn cache_node(&mut self, range: Range, expr: Box<Expr>, content: &str) {
        let content_hash = calculate_hash(content);
        let size_bytes = std::mem::size_of_val(&*expr) + content.len();

        let cached = CachedNode {
            expr,
            range,
            content_hash,
            size_bytes,
        };

        self.stats.total_cached_bytes += size_bytes;
        self.cached_nodes.insert(RangeKey::from(range), cached);
    }

    /// Try to retrieve a cached node for a range
    pub fn get_cached_node(&mut self, range: &Range, content: &str) -> Option<Box<Expr>> {
        let key = RangeKey::from(range);
        if let Some(cached) = self.cached_nodes.get(&key) {
            // Validate the cache by checking content hash
            let current_hash = calculate_hash(content);
            if current_hash == cached.content_hash {
                self.stats.cache_hits += 1;
                return Some(cached.expr.clone());
            } else {
                // Hash mismatch - content changed, invalidate cache
                self.stats.cache_misses += 1;
                self.cached_nodes.remove(&key);
            }
        } else {
            self.stats.cache_misses += 1;
        }
        None
    }

    /// Clear all dirty regions after parsing
    pub fn clear_dirty_regions(&mut self) {
        self.dirty_regions.clear();
        self.changed_ranges.clear();
    }

    /// Record a parse operation
    pub fn record_parse(&mut self, is_incremental: bool, duration_us: u64) {
        if is_incremental {
            self.stats.incremental_parses += 1;
        } else {
            self.stats.full_parses += 1;
        }

        self.stats.total_parse_time_us += duration_us;
        let total_parses = self.stats.full_parses + self.stats.incremental_parses;
        self.stats.avg_parse_time_us = self.stats.total_parse_time_us / total_parses.max(1);

        self.last_parse_time = Instant::now();
    }

    /// Get the current parse statistics
    pub fn stats(&self) -> &ParseStats {
        &self.stats
    }

    /// Check if incremental parsing is beneficial for this change
    pub fn should_use_incremental(&self, total_lines: usize) -> bool {
        // Use incremental parsing if:
        // 1. We have dirty regions (indicates a change)
        // 2. The dirty regions cover less than 30% of the document
        // 3. We have some cached nodes to reuse

        if self.dirty_regions.is_empty() {
            return false;
        }

        let dirty_lines: usize = self
            .dirty_regions
            .iter()
            .map(|r| (r.end.line - r.start.line + 1) as usize)
            .sum();

        let dirty_ratio = dirty_lines as f64 / total_lines.max(1) as f64;

        dirty_ratio < 0.3 && !self.cached_nodes.is_empty()
    }

    /// Get an expanded range for re-parsing with padding
    pub fn get_reparse_range(&self, changed_range: Range, padding_lines: u32) -> Range {
        Range {
            start: Position {
                line: changed_range.start.line.saturating_sub(padding_lines),
                character: 0,
            },
            end: Position {
                line: changed_range.end.line + padding_lines,
                character: u32::MAX, // End of line
            },
        }
    }

    /// Merge overlapping dirty regions
    pub fn consolidate_dirty_regions(&mut self) {
        if self.dirty_regions.len() <= 1 {
            return;
        }

        // Sort by start position
        self.dirty_regions
            .sort_by_key(|r| (r.start.line, r.start.character));

        let mut consolidated = Vec::new();
        let mut current = self.dirty_regions[0];

        for region in self.dirty_regions.iter().skip(1) {
            if ranges_overlap(&current, region) || ranges_adjacent(&current, region) {
                // Merge regions
                current = Range {
                    start: Position {
                        line: current.start.line.min(region.start.line),
                        character: if current.start.line == region.start.line {
                            current.start.character.min(region.start.character)
                        } else if current.start.line < region.start.line {
                            current.start.character
                        } else {
                            region.start.character
                        },
                    },
                    end: Position {
                        line: current.end.line.max(region.end.line),
                        character: if current.end.line == region.end.line {
                            current.end.character.max(region.end.character)
                        } else if current.end.line > region.end.line {
                            current.end.character
                        } else {
                            region.end.character
                        },
                    },
                };
            } else {
                consolidated.push(current);
                current = *region;
            }
        }
        consolidated.push(current);

        self.dirty_regions = consolidated;
    }
}

impl Default for IncrementalState {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if two ranges overlap
pub fn ranges_overlap(a: &Range, b: &Range) -> bool {
    // Ranges overlap if one starts before the other ends
    let a_start = (a.start.line, a.start.character);
    let a_end = (a.end.line, a.end.character);
    let b_start = (b.start.line, b.start.character);
    let b_end = (b.end.line, b.end.character);

    // Check if ranges overlap
    (a_start <= b_end) && (b_start <= a_end)
}

/// Check if two ranges are adjacent (can be merged)
pub fn ranges_adjacent(a: &Range, b: &Range) -> bool {
    // Ranges are adjacent if they're on consecutive lines or same line with touching positions
    if a.end.line + 1 == b.start.line && b.start.character == 0 {
        return true;
    }

    if a.end.line == b.start.line && a.end.character == b.start.character {
        return true;
    }

    false
}

/// Calculate a simple hash of text content
pub fn calculate_hash(text: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Find AST nodes affected by a change range
///
/// # Arguments
///
/// * `module` - The AST module to search
/// * `changed_range` - The LSP range that changed
/// * `text` - The source text for byte-offset to line/column conversion
///
/// # Returns
///
/// A vector of spans that overlap with the changed range
pub fn find_affected_nodes(module: &Module, changed_range: Range, text: &str) -> Vec<Span> {
    let mut affected = Vec::new();

    // Walk the AST and collect spans that overlap with the changed range
    for item in module.items.iter() {
        if span_overlaps_range(&item.span, &changed_range, text) {
            affected.push(item.span);
        }
    }

    affected
}

/// Check if a Verum span overlaps with an LSP range
///
/// This function converts the byte-offset span to an LSP range and checks for overlap.
///
/// # Arguments
///
/// * `span` - The byte-offset span to check
/// * `range` - The LSP range to check against
/// * `text` - The source text for byte-offset to line/column conversion
///
/// # Returns
///
/// `true` if the span overlaps with the range, `false` otherwise
fn span_overlaps_range(span: &Span, range: &Range, text: &str) -> bool {
    use verum_common::span_utils::lsp::span_to_lsp_range;

    if span.is_empty() {
        return false;
    }

    // Convert the byte-offset span to an LSP range
    let span_range = span_to_lsp_range(*span, text);

    // Check if the ranges overlap
    ranges_overlap(&span_range, range)
}

/// Estimate the size of an AST subtree in bytes
pub fn estimate_node_size(expr: &Expr) -> usize {
    // Base size of the expression node
    let mut size = std::mem::size_of::<Expr>();

    // Add size based on expression kind
    match &expr.kind {
        ExprKind::Block(block) => {
            size += block.stmts.len() * 64; // Rough estimate per statement
        }
        ExprKind::Tuple(elements) => {
            size += elements.len() * 32;
        }
        ExprKind::Array(_) => {
            size += 32; // Rough estimate for arrays
        }
        ExprKind::Call { args, .. } => {
            size += args.len() * 32;
        }
        ExprKind::Literal(_) => {
            size += 32;
        }
        _ => {
            size += 16;
        }
    }

    size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ranges_overlap() {
        let range1 = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 2,
                character: 10,
            },
        };

        let range2 = Range {
            start: Position {
                line: 1,
                character: 5,
            },
            end: Position {
                line: 3,
                character: 0,
            },
        };

        assert!(ranges_overlap(&range1, &range2));
    }

    #[test]
    fn test_ranges_no_overlap() {
        let range1 = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 2,
                character: 10,
            },
        };

        let range2 = Range {
            start: Position {
                line: 5,
                character: 0,
            },
            end: Position {
                line: 7,
                character: 0,
            },
        };

        assert!(!ranges_overlap(&range1, &range2));
    }

    #[test]
    fn test_ranges_adjacent() {
        let range1 = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 2,
                character: 10,
            },
        };

        let range2 = Range {
            start: Position {
                line: 2,
                character: 10,
            },
            end: Position {
                line: 4,
                character: 0,
            },
        };

        assert!(ranges_adjacent(&range1, &range2));
    }

    #[test]
    fn test_calculate_hash() {
        let text1 = "fn main() { }";
        let text2 = "fn main() { }";
        let text3 = "fn test() { }";

        assert_eq!(calculate_hash(text1), calculate_hash(text2));
        assert_ne!(calculate_hash(text1), calculate_hash(text3));
    }

    #[test]
    fn test_incremental_state_mark_dirty() {
        let mut state = IncrementalState::new();

        let range = Range {
            start: Position {
                line: 5,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 0,
            },
        };

        state.mark_dirty(range);
        assert_eq!(state.dirty_regions.len(), 1);
        assert_eq!(state.changed_ranges.len(), 1);
    }

    #[test]
    fn test_consolidate_dirty_regions() {
        let mut state = IncrementalState::new();

        // Add overlapping regions
        state.dirty_regions = vec![
            Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 5,
                    character: 0,
                },
            },
            Range {
                start: Position {
                    line: 3,
                    character: 0,
                },
                end: Position {
                    line: 8,
                    character: 0,
                },
            },
            Range {
                start: Position {
                    line: 10,
                    character: 0,
                },
                end: Position {
                    line: 15,
                    character: 0,
                },
            },
        ];

        state.consolidate_dirty_regions();

        // Should consolidate to 2 regions: [0-8] and [10-15]
        assert_eq!(state.dirty_regions.len(), 2);
    }

    #[test]
    fn test_span_overlaps_range() {
        use verum_common::span::{FileId, Span};

        let source = "fn main() {\n    let x = 42;\n}\n";
        // "fn main() {" is bytes 0-11 (line 0, col 0 to line 0, col 11)
        // "\n    let x = 42;" is bytes 11-27 (line 0, col 11 to line 1, col 15)

        let span = Span::new(0, 11, FileId::new(0));
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 5,
            },
        };

        assert!(super::span_overlaps_range(&span, &range, source));

        // Test non-overlapping ranges
        let span2 = Span::new(0, 11, FileId::new(0)); // line 0, "fn main() {"
        let range2 = Range {
            start: Position {
                line: 2,
                character: 0,
            },
            end: Position {
                line: 2,
                character: 1,
            },
        };

        assert!(!super::span_overlaps_range(&span2, &range2, source));
    }

    #[test]
    fn test_span_overlaps_range_empty_span() {
        use verum_common::span::{FileId, Span};

        let source = "fn main() {}";
        let empty_span = Span::new(5, 5, FileId::new(0));
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 10,
            },
        };

        assert!(!super::span_overlaps_range(&empty_span, &range, source));
    }
}

// ============================================================================
// LSP Document Integration with verum_parser Incremental Engine
// ============================================================================

/// A managed document with full incremental parsing support.
///
/// This wraps `verum_parser::IncrementalDocument` for use in the LSP server,
/// providing additional LSP-specific functionality like range conversion.
#[derive(Debug)]
pub struct LspIncrementalDocument {
    /// The underlying incremental document.
    inner: IncrementalDocument,
    /// Additional LSP state tracking.
    state: IncrementalState,
}

impl LspIncrementalDocument {
    /// Create a new incremental document with initial content.
    pub fn new(content: &str, version: i32) -> Self {
        Self {
            inner: IncrementalDocument::new(content, version),
            state: IncrementalState::new(),
        }
    }

    /// Get the current source text.
    pub fn text(&self) -> &str {
        self.inner.text()
    }

    /// Get the current version.
    pub fn version(&self) -> i32 {
        self.inner.version()
    }

    /// Get the current green tree.
    pub fn tree(&self) -> Option<&GreenNode> {
        self.inner.tree()
    }

    /// Get the syntax tree for navigation.
    pub fn syntax(&self) -> Option<SyntaxNode> {
        self.inner.tree().map(|g| SyntaxNode::new_root(g.clone()))
    }

    /// Check if the document parsed without errors.
    pub fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }

    /// Get parse errors from the last parse.
    pub fn errors(&self) -> &[verum_syntax::ParseError] {
        self.inner.errors()
    }

    /// Get parsing statistics.
    pub fn parser_stats(&self) -> &verum_syntax::IncrementalStats {
        self.inner.stats()
    }

    /// Get LSP-level statistics.
    pub fn lsp_stats(&self) -> &ParseStats {
        self.state.stats()
    }

    /// Apply content changes from an LSP didChange notification.
    pub fn apply_changes(&mut self, changes: Vec<tower_lsp::lsp_types::TextDocumentContentChangeEvent>, version: i32) {
        let start_time = Instant::now();

        // Convert LSP changes to our format
        let syntax_changes: Vec<SyntaxLspChange> = changes
            .into_iter()
            .map(|change| {
                let range = change.range.map(|r| SyntaxLspRange {
                    start_line: r.start.line,
                    start_col: r.start.character,
                    end_line: r.end.line,
                    end_col: r.end.character,
                });
                SyntaxLspChange {
                    range,
                    text: change.text,
                }
            })
            .collect();

        // Apply to inner document
        self.inner.apply_changes(syntax_changes, version);

        // Record timing
        let duration_us = start_time.elapsed().as_micros() as u64;
        self.state.record_parse(true, duration_us);
    }

    /// Update the entire document content (full sync).
    pub fn set_content(&mut self, content: &str, version: i32) {
        let start_time = Instant::now();

        self.inner.set_content(content, version);

        let duration_us = start_time.elapsed().as_micros() as u64;
        self.state.record_parse(false, duration_us);
    }
}

/// Convert an LSP range to a TextEdit.
pub fn lsp_range_to_text_edit(range: Range, new_text: String, source: &str) -> TextEdit {
    let start = lsp_position_to_offset(range.start, source);
    let end = lsp_position_to_offset(range.end, source);
    TextEdit::replace(TextRange::new(start, end), new_text)
}

/// Convert an LSP position to a byte offset.
fn lsp_position_to_offset(pos: Position, source: &str) -> TextSize {
    let mut line = 0;
    let mut col = 0;
    let mut offset = 0;

    for ch in source.chars() {
        if line == pos.line && col == pos.character {
            return offset;
        }

        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf16() as u32;
        }

        offset += ch.len_utf8() as TextSize;
    }

    offset
}

/// Convert a byte offset to an LSP position.
pub fn offset_to_lsp_position(offset: TextSize, source: &str) -> Position {
    let mut line = 0;
    let mut col = 0;
    let mut current_offset = 0;

    for ch in source.chars() {
        if current_offset >= offset {
            break;
        }

        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf16() as u32;
        }

        current_offset += ch.len_utf8() as TextSize;
    }

    Position {
        line,
        character: col,
    }
}

/// Convert a TextRange to an LSP Range.
pub fn text_range_to_lsp_range(range: TextRange, source: &str) -> Range {
    Range {
        start: offset_to_lsp_position(range.start(), source),
        end: offset_to_lsp_position(range.end(), source),
    }
}
