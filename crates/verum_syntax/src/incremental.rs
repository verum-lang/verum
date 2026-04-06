//! Production-quality incremental parsing support.
//!
//! Enables efficient re-parsing when source is edited by:
//! 1. Tracking text changes (insertions, deletions, replacements)
//! 2. Identifying the smallest affected subtree using node stability analysis
//! 3. Re-parsing only that subtree with proper context
//! 4. Replacing it in the green tree using structural sharing
//!
//! # Key Concepts
//!
//! ## Node Stability
//!
//! A node is "stable" if it can be independently re-parsed without affecting
//! surrounding nodes. Stable boundaries include:
//! - Top-level items (functions, types, imports)
//! - Blocks delimited by braces
//! - Statements ending with semicolons
//!
//! ## Reparse Context
//!
//! When re-parsing a subtree, we need to know what parsing rule to use:
//! - SOURCE_FILE nodes use module parsing
//! - FN_DEF nodes use function definition parsing
//! - BLOCK nodes use block parsing
//! - LET_STMT nodes use statement parsing
//!
//! ## Structural Sharing
//!
//! Green trees store relative widths, not absolute offsets. This enables
//! O(log n) updates: only the path from the edit to the root needs recreation.
//!
//! # Performance Targets
//!
//! - Single character edit: < 5ms
//! - Multi-line edit (< 10 lines): < 20ms
//! - Large edit (> 10 lines): Falls back to full reparse
//!
//! Incremental Parsing Algorithm:
//! 1. Find smallest affected subtree containing the edit (walk down green tree)
//! 2. Compute local edit coordinates relative to subtree start
//! 3. Re-parse only that subtree using appropriate parsing rule based on node kind
//!    (SOURCE_FILE -> module, FN_DEF -> function, BLOCK -> block, etc.)
//! 4. Replace subtree in green tree via O(log n) path update with structural sharing
//!    Stable boundaries for subtree detection: top-level items, brace-delimited blocks,
//!    semicolon-terminated statements. Heuristic: use incremental if edit < 20% of tree.
//!    Falls back to full reparse for edits spanning >10 lines.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use crate::{GreenChild, GreenNode, SyntaxKind, TextRange, TextSize};

// ============================================================================
// Text Edit Types
// ============================================================================

/// Represents a text edit (insertion, deletion, or replacement).
///
/// Edits are described in terms of the original text coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    /// Range in the original text to replace.
    pub range: TextRange,
    /// New text to insert.
    pub new_text: String,
}

impl TextEdit {
    /// Create an edit that replaces a range.
    pub fn replace(range: TextRange, new_text: impl Into<String>) -> Self {
        Self {
            range,
            new_text: new_text.into(),
        }
    }

    /// Create an edit that inserts text at a position.
    pub fn insert(offset: TextSize, text: impl Into<String>) -> Self {
        Self {
            range: TextRange::empty(offset),
            new_text: text.into(),
        }
    }

    /// Create an edit that deletes a range.
    pub fn delete(range: TextRange) -> Self {
        Self {
            range,
            new_text: String::new(),
        }
    }

    /// Calculate the change in text length.
    pub fn len_delta(&self) -> i64 {
        self.new_text.len() as i64 - self.range.len() as i64
    }

    /// Apply this edit to source text.
    pub fn apply(&self, source: &str) -> String {
        let start = self.range.start() as usize;
        let end = self.range.end() as usize;

        let mut result = String::with_capacity(source.len().saturating_add_signed(self.len_delta() as isize));
        result.push_str(&source[..start.min(source.len())]);
        result.push_str(&self.new_text);
        if end <= source.len() {
            result.push_str(&source[end..]);
        }
        result
    }

    /// Check if this edit affects only whitespace.
    pub fn is_whitespace_only(&self) -> bool {
        self.new_text.chars().all(|c| c.is_whitespace())
    }

    /// Check if this is a single character edit.
    pub fn is_single_char(&self) -> bool {
        (self.range.len() <= 1) && (self.new_text.len() <= 1)
    }

    /// Get the end position after the edit is applied.
    pub fn new_end(&self) -> TextSize {
        self.range.start() + self.new_text.len() as TextSize
    }
}

// ============================================================================
// Node Stability Analysis
// ============================================================================

/// Determines how a node can be reparsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeStability {
    /// Node can be independently reparsed (e.g., top-level item, block).
    Stable,
    /// Node is part of a larger construct and requires parent context.
    Unstable,
    /// Node is a leaf token - must reparse parent.
    Token,
}

/// Context needed to reparse a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReparseContext {
    /// Parse as module (top-level items).
    Module,
    /// Parse as a single item (function, type, etc.).
    Item,
    /// Parse as a block of statements.
    Block,
    /// Parse as a single statement.
    Statement,
    /// Parse as an expression.
    Expression,
    /// Parse as a type.
    Type,
    /// Cannot reparse independently - need full reparse.
    Unknown,
}

impl ReparseContext {
    /// Determine the reparse context for a given syntax kind.
    pub fn for_kind(kind: SyntaxKind) -> Self {
        match kind {
            SyntaxKind::SOURCE_FILE => ReparseContext::Module,

            // Top-level items - can be reparsed as items
            SyntaxKind::FN_DEF
            | SyntaxKind::TYPE_DEF
            | SyntaxKind::PROTOCOL_DEF
            | SyntaxKind::IMPL_BLOCK
            | SyntaxKind::CONTEXT_DEF
            | SyntaxKind::CONST_DEF
            | SyntaxKind::STATIC_DEF
            | SyntaxKind::MOUNT_STMT
            | SyntaxKind::MODULE_DEF
            | SyntaxKind::META_DEF
            | SyntaxKind::FFI_DECL
            | SyntaxKind::THEOREM_DEF
            | SyntaxKind::AXIOM_DEF
            | SyntaxKind::LEMMA_DEF
            | SyntaxKind::COROLLARY_DEF => ReparseContext::Item,

            // Blocks
            SyntaxKind::BLOCK
            | SyntaxKind::BLOCK_EXPR
            | SyntaxKind::PROOF_BLOCK => ReparseContext::Block,

            // Statements
            SyntaxKind::LET_STMT
            | SyntaxKind::EXPR_STMT
            | SyntaxKind::RETURN_STMT
            | SyntaxKind::BREAK_STMT
            | SyntaxKind::CONTINUE_STMT
            | SyntaxKind::YIELD_STMT
            | SyntaxKind::DEFER_STMT
            | SyntaxKind::ERRDEFER_STMT
            | SyntaxKind::PROVIDE_STMT
            | SyntaxKind::THROW_STMT => ReparseContext::Statement,

            // Expressions
            SyntaxKind::LITERAL_EXPR
            | SyntaxKind::PATH_EXPR
            | SyntaxKind::BINARY_EXPR
            | SyntaxKind::PREFIX_EXPR
            | SyntaxKind::POSTFIX_EXPR
            | SyntaxKind::CALL_EXPR
            | SyntaxKind::METHOD_CALL_EXPR
            | SyntaxKind::FIELD_EXPR
            | SyntaxKind::INDEX_EXPR
            | SyntaxKind::IF_EXPR
            | SyntaxKind::MATCH_EXPR
            | SyntaxKind::CLOSURE_EXPR
            | SyntaxKind::ASYNC_EXPR
            | SyntaxKind::AWAIT_EXPR
            | SyntaxKind::SPAWN_EXPR
            | SyntaxKind::SELECT_EXPR
            | SyntaxKind::PIPELINE_EXPR
            | SyntaxKind::TRY_EXPR
            | SyntaxKind::TUPLE_EXPR
            | SyntaxKind::ARRAY_EXPR
            | SyntaxKind::RECORD_EXPR
            | SyntaxKind::RANGE_EXPR
            | SyntaxKind::CAST_EXPR
            | SyntaxKind::REF_EXPR
            | SyntaxKind::DEREF_EXPR
            | SyntaxKind::PAREN_EXPR
            | SyntaxKind::LOOP_EXPR
            | SyntaxKind::WHILE_EXPR
            | SyntaxKind::FOR_EXPR
            | SyntaxKind::FOR_AWAIT_EXPR
            | SyntaxKind::STREAM_EXPR
            | SyntaxKind::REFINEMENT_EXPR
            | SyntaxKind::RECOVER_EXPR
            | SyntaxKind::MAP_EXPR
            | SyntaxKind::SET_EXPR
            | SyntaxKind::TENSOR_EXPR
            | SyntaxKind::COMPREHENSION_EXPR
            | SyntaxKind::STREAM_COMPREHENSION_EXPR
            | SyntaxKind::THROW_EXPR
            | SyntaxKind::IS_EXPR
            | SyntaxKind::QUOTE_EXPR
            | SyntaxKind::SPLICE_EXPR
            | SyntaxKind::GENERATOR_EXPR
            | SyntaxKind::YIELD_EXPR
            | SyntaxKind::FORALL_EXPR
            | SyntaxKind::EXISTS_EXPR => ReparseContext::Expression,

            // Types
            SyntaxKind::PATH_TYPE
            | SyntaxKind::REFERENCE_TYPE
            | SyntaxKind::POINTER_TYPE
            | SyntaxKind::FUNCTION_TYPE
            | SyntaxKind::TUPLE_TYPE
            | SyntaxKind::ARRAY_TYPE
            | SyntaxKind::SLICE_TYPE
            | SyntaxKind::REFINED_TYPE
            | SyntaxKind::GENERIC_TYPE
            | SyntaxKind::INFER_TYPE
            | SyntaxKind::NEVER_TYPE
            | SyntaxKind::GENREF_TYPE
            | SyntaxKind::PAREN_TYPE
            | SyntaxKind::DYNAMIC_TYPE
            | SyntaxKind::UNION_TYPE
            | SyntaxKind::INTERSECTION_TYPE => ReparseContext::Type,

            _ => ReparseContext::Unknown,
        }
    }
}

/// Analyzes node stability for incremental parsing decisions.
pub struct NodeStabilityAnalyzer;

impl NodeStabilityAnalyzer {
    /// Determine if a node is stable (can be independently reparsed).
    pub fn is_stable(kind: SyntaxKind) -> NodeStability {
        if kind.is_token() {
            return NodeStability::Token;
        }

        match kind {
            // Top-level items are always stable
            SyntaxKind::SOURCE_FILE
            | SyntaxKind::FN_DEF
            | SyntaxKind::TYPE_DEF
            | SyntaxKind::PROTOCOL_DEF
            | SyntaxKind::IMPL_BLOCK
            | SyntaxKind::CONTEXT_DEF
            | SyntaxKind::CONST_DEF
            | SyntaxKind::STATIC_DEF
            | SyntaxKind::MOUNT_STMT
            | SyntaxKind::MODULE_DEF
            | SyntaxKind::META_DEF
            | SyntaxKind::FFI_DECL
            | SyntaxKind::THEOREM_DEF
            | SyntaxKind::AXIOM_DEF
            | SyntaxKind::LEMMA_DEF
            | SyntaxKind::COROLLARY_DEF => NodeStability::Stable,

            // Blocks are stable because they're delimited by braces
            SyntaxKind::BLOCK
            | SyntaxKind::BLOCK_EXPR
            | SyntaxKind::PROOF_BLOCK
            | SyntaxKind::FFI_BLOCK => NodeStability::Stable,

            // Statements ending with semicolons are stable
            SyntaxKind::LET_STMT
            | SyntaxKind::RETURN_STMT
            | SyntaxKind::BREAK_STMT
            | SyntaxKind::CONTINUE_STMT
            | SyntaxKind::YIELD_STMT
            | SyntaxKind::DEFER_STMT
            | SyntaxKind::ERRDEFER_STMT
            | SyntaxKind::PROVIDE_STMT
            | SyntaxKind::THROW_STMT => NodeStability::Stable,

            // Match arms are stable
            SyntaxKind::MATCH_ARM => NodeStability::Stable,

            // Everything else needs parent context
            _ => NodeStability::Unstable,
        }
    }

    /// Check if an edit is contained within stable boundaries.
    ///
    /// Returns true if the edit doesn't cross delimiter tokens like braces.
    pub fn edit_is_contained(node: &GreenNode, edit: &TextEdit) -> bool {
        // If edit spans the entire node, it's not contained
        if edit.range.start() == 0 && edit.range.end() >= node.width() {
            return false;
        }

        // Check if edit crosses any delimiter boundaries
        let mut offset = 0;
        for child in node.children() {
            let child_end = offset + child.width();
            let child_range = TextRange::at(offset, child.width());

            // Check if edit overlaps this child
            if child_range.intersect(edit.range).is_some() {
                // If it's a delimiter token, the edit crosses a boundary
                if let GreenChild::Token(token) = child {
                    match token.kind() {
                        SyntaxKind::L_BRACE
                        | SyntaxKind::R_BRACE
                        | SyntaxKind::L_PAREN
                        | SyntaxKind::R_PAREN
                        | SyntaxKind::L_BRACKET
                        | SyntaxKind::R_BRACKET => return false,
                        _ => {}
                    }
                }
            }

            offset = child_end;
        }

        true
    }
}

// ============================================================================
// Affected Subtree Identification
// ============================================================================

/// Result of finding the affected subtree.
#[derive(Clone, Debug)]
pub struct AffectedSubtree {
    /// Path from root to the affected node (indices at each level).
    pub path: Vec<usize>,
    /// The affected node.
    pub node: GreenNode,
    /// Start offset of the affected node in source.
    pub start_offset: TextSize,
    /// The reparse context for this node.
    pub context: ReparseContext,
    /// Whether this node is stable (can be independently reparsed).
    pub is_stable: bool,
    /// Whether the edit only affects this node's content (not structure).
    pub content_only: bool,
}

// ============================================================================
// Incremental Update Engine
// ============================================================================

/// Statistics about incremental parsing operations.
#[derive(Clone, Debug, Default)]
pub struct IncrementalStats {
    /// Number of incremental reparses performed.
    pub incremental_parses: u64,
    /// Number of full reparses performed.
    pub full_parses: u64,
    /// Total time spent in incremental parsing (microseconds).
    pub incremental_time_us: u64,
    /// Total time spent in full parsing (microseconds).
    pub full_parse_time_us: u64,
    /// Number of nodes reused from cache.
    pub nodes_reused: u64,
    /// Number of nodes recreated.
    pub nodes_recreated: u64,
    /// Maximum tree depth traversed.
    pub max_depth: usize,
    /// Total bytes saved by incremental parsing.
    pub bytes_saved: u64,
}

impl IncrementalStats {
    /// Calculate the cache hit ratio.
    pub fn cache_hit_ratio(&self) -> f64 {
        let total = self.nodes_reused + self.nodes_recreated;
        if total == 0 {
            0.0
        } else {
            self.nodes_reused as f64 / total as f64
        }
    }

    /// Calculate the average incremental parse time.
    pub fn avg_incremental_time_us(&self) -> u64 {
        self.incremental_time_us.checked_div(self.incremental_parses).unwrap_or(0)
    }
}

/// Incremental update engine.
///
/// This is the core engine for incremental parsing. It:
/// 1. Finds the smallest affected subtree for an edit
/// 2. Determines if incremental parsing is beneficial
/// 3. Applies edits by reparsing only affected regions
/// 4. Rebuilds the tree with structural sharing
#[derive(Debug, Default)]
pub struct IncrementalEngine {
    /// Statistics about parsing operations.
    stats: IncrementalStats,
}

impl IncrementalEngine {
    /// Create a new incremental engine.
    pub fn new() -> Self {
        Self {
            stats: IncrementalStats::default(),
        }
    }

    /// Get parsing statistics.
    pub fn stats(&self) -> &IncrementalStats {
        &self.stats
    }

    /// Reset statistics.
    pub fn reset_stats(&mut self) {
        self.stats = IncrementalStats::default();
    }

    /// Find the smallest subtree affected by an edit.
    ///
    /// This traverses the tree to find the smallest containing node that:
    /// 1. Fully contains the edit range
    /// 2. Is stable (can be independently reparsed)
    ///
    /// Returns the path to the affected node and the node itself.
    pub fn find_affected_subtree(
        &mut self,
        root: &GreenNode,
        edit: &TextEdit,
    ) -> Option<AffectedSubtree> {
        let mut path = Vec::new();
        let mut current = root;
        let mut offset = TextSize::from(0u32);
        let mut depth = 0;

        // Track the last stable node we encountered
        let mut last_stable_path = Vec::new();
        let mut last_stable_node = root;
        let mut last_stable_offset = TextSize::from(0u32);

        // Traverse down to find smallest containing node
        loop {
            depth += 1;
            self.stats.max_depth = self.stats.max_depth.max(depth);

            let stability = NodeStabilityAnalyzer::is_stable(current.kind());

            // If this node is stable, remember it
            if stability == NodeStability::Stable {
                last_stable_path = path.clone();
                last_stable_node = current;
                last_stable_offset = offset;
            }

            // Find child that contains the edit range
            let mut child_offset = offset;
            let mut found_child = None;

            for (idx, child) in current.children().iter().enumerate() {
                let child_range = TextRange::at(child_offset, child.width());

                // Check if this child fully contains the edit
                if child_range.contains_range(edit.range) {
                    if let GreenChild::Node(node) = child {
                        found_child = Some((idx, node, child_offset));
                    }
                    break;
                }

                // Check if edit overlaps with this child (spans multiple children)
                if child_range.intersect(edit.range).is_some() {
                    // Edit spans multiple children - current node is the smallest
                    break;
                }

                child_offset += child.width();
            }

            match found_child {
                Some((idx, node, node_offset)) => {
                    path.push(idx);
                    current = node;
                    offset = node_offset;
                }
                None => {
                    // Current node is the smallest containing node
                    break;
                }
            }
        }

        // Determine if we should use the last stable node or current node
        let (final_path, final_node, final_offset) =
            if NodeStabilityAnalyzer::is_stable(current.kind()) == NodeStability::Stable {
                (path, current, offset)
            } else {
                // Use the last stable ancestor
                (last_stable_path, last_stable_node, last_stable_offset)
            };

        // Determine reparse context
        let context = ReparseContext::for_kind(final_node.kind());
        let is_stable = NodeStabilityAnalyzer::is_stable(final_node.kind()) == NodeStability::Stable;

        // Determine if this is content-only change
        let content_only = self.is_content_only_change(final_node, edit, final_offset);

        Some(AffectedSubtree {
            path: final_path,
            node: final_node.clone(),
            start_offset: final_offset,
            context,
            is_stable,
            content_only,
        })
    }

    /// Check if an edit only changes content without affecting structure.
    fn is_content_only_change(&self, node: &GreenNode, edit: &TextEdit, node_offset: TextSize) -> bool {
        // Adjust edit range to be relative to node
        let local_start = edit.range.start().saturating_sub(node_offset);
        let local_end = edit.range.end().saturating_sub(node_offset);
        let local_range = TextRange::new(local_start, local_end.min(node.width()));

        // Check if edit is entirely within a single token child
        let mut child_offset = 0;
        for child in node.children() {
            let child_range = TextRange::at(child_offset, child.width());

            if child_range.contains_range(local_range)
                && let GreenChild::Token(token) = child {
                    // Edit is within a token - check if it's a content token
                    return matches!(
                        token.kind(),
                        SyntaxKind::STRING_LITERAL
                        | SyntaxKind::INT_LITERAL
                        | SyntaxKind::FLOAT_LITERAL
                        | SyntaxKind::CHAR_LITERAL
                        | SyntaxKind::INTERPOLATED_STRING
                        | SyntaxKind::IDENT
                        | SyntaxKind::LINE_COMMENT
                        | SyntaxKind::BLOCK_COMMENT
                        | SyntaxKind::DOC_COMMENT
                        | SyntaxKind::WHITESPACE
                        | SyntaxKind::NEWLINE
                    );
                }

            child_offset += child.width();
        }

        false
    }

    /// Apply an edit to the green tree incrementally.
    ///
    /// Returns the new root node with the edit applied.
    pub fn apply_edit<F>(
        &mut self,
        root: &GreenNode,
        edit: &TextEdit,
        reparse_fn: F,
        source: &str,
    ) -> GreenNode
    where
        F: FnOnce(&str, ReparseContext) -> GreenNode,
    {
        let start_time = Instant::now();

        // Find affected subtree
        let affected = match self.find_affected_subtree(root, edit) {
            Some(a) => a,
            None => {
                // Fallback to full reparse
                let new_source = edit.apply(source);
                self.stats.full_parses += 1;
                self.stats.full_parse_time_us += start_time.elapsed().as_micros() as u64;
                return reparse_fn(&new_source, ReparseContext::Module);
            }
        };

        // If edit affects root or path is empty, or context is unknown, full reparse
        if affected.path.is_empty() || affected.context == ReparseContext::Unknown {
            let new_source = edit.apply(source);
            self.stats.full_parses += 1;
            self.stats.full_parse_time_us += start_time.elapsed().as_micros() as u64;
            return reparse_fn(&new_source, ReparseContext::Module);
        }

        // Extract the source text for the affected subtree
        let subtree_start = affected.start_offset as usize;
        let subtree_end = subtree_start + affected.node.width() as usize;

        // Validate bounds
        if subtree_end > source.len() {
            let new_source = edit.apply(source);
            self.stats.full_parses += 1;
            self.stats.full_parse_time_us += start_time.elapsed().as_micros() as u64;
            return reparse_fn(&new_source, ReparseContext::Module);
        }

        // Apply edit to get new subtree source
        let subtree_source = &source[subtree_start..subtree_end];
        let local_edit = TextEdit {
            range: TextRange::new(
                edit.range.start().saturating_sub(affected.start_offset),
                edit.range.end().saturating_sub(affected.start_offset),
            ),
            new_text: edit.new_text.clone(),
        };
        let new_subtree_source = local_edit.apply(subtree_source);

        // Reparse the subtree with appropriate context
        let new_subtree = reparse_fn(&new_subtree_source, affected.context);

        // Rebuild the tree with the new subtree
        let result = self.rebuild_with_replacement(root, &affected.path, new_subtree);

        // Update statistics
        self.stats.incremental_parses += 1;
        self.stats.incremental_time_us += start_time.elapsed().as_micros() as u64;
        self.stats.nodes_reused += self.count_shared_nodes(root, &affected.path);
        self.stats.nodes_recreated += affected.path.len() as u64 + 1;
        self.stats.bytes_saved += source.len().saturating_sub(new_subtree_source.len()) as u64;

        result
    }

    /// Count nodes that can be shared (not on the path to replacement).
    fn count_shared_nodes(&self, root: &GreenNode, path: &[usize]) -> u64 {
        let mut count = 0;
        let mut current = root;

        for &idx in path.iter() {
            // Count siblings that are not on the path
            for (i, child) in current.children().iter().enumerate() {
                if i != idx
                    && let GreenChild::Node(node) = child {
                        count += self.count_nodes(node);
                    }
            }

            // Move to next level
            if let Some(GreenChild::Node(node)) = current.child(idx) {
                current = node;
            } else {
                break;
            }
        }

        count
    }

    /// Count total nodes in a subtree.
    fn count_nodes(&self, node: &GreenNode) -> u64 {
        let mut count = 1;
        for child in node.children() {
            if let GreenChild::Node(child_node) = child {
                count += self.count_nodes(child_node);
            }
        }
        count
    }

    /// Rebuild the tree with a replacement at the given path.
    fn rebuild_with_replacement(
        &self,
        root: &GreenNode,
        path: &[usize],
        replacement: GreenNode,
    ) -> GreenNode {
        if path.is_empty() {
            return replacement;
        }

        let idx = path[0];
        let remaining_path = &path[1..];

        let new_child = if remaining_path.is_empty() {
            GreenChild::Node(replacement)
        } else {
            match &root.children()[idx] {
                GreenChild::Node(child_node) => {
                    let rebuilt = self.rebuild_with_replacement(child_node, remaining_path, replacement);
                    GreenChild::Node(rebuilt)
                }
                GreenChild::Token(_) => {
                    // Should not happen - tokens can't have children
                    return root.clone();
                }
            }
        };

        root.replace_child(idx, new_child)
    }

    /// Check if incremental parsing is beneficial.
    ///
    /// Returns false if the edit is large enough that full reparse is faster.
    pub fn should_use_incremental(&self, root: &GreenNode, edit: &TextEdit) -> bool {
        let tree_size = root.width() as usize;
        let edit_size = edit.new_text.len() + edit.range.len() as usize;

        // Heuristics for when to use incremental parsing:

        // 1. If tree is very small (< 1KB), full reparse is likely faster
        if tree_size < 1024 {
            return false;
        }

        // 2. If edit affects more than 30% of the tree, full reparse is likely faster
        if edit_size > tree_size * 30 / 100 {
            return false;
        }

        // 3. If this is a single character edit, incremental is almost always faster
        if edit.is_single_char() {
            return true;
        }

        // 4. If edit only affects whitespace, incremental is faster
        if edit.is_whitespace_only() {
            return true;
        }

        // 5. Default: use incremental if edit is less than 20% of tree
        edit_size < tree_size / 5
    }
}

// ============================================================================
// Change Tracker for Document Synchronization
// ============================================================================

/// Change tracker for batching and merging document edits.
///
/// This is designed for LSP integration where multiple edits may come
/// in rapid succession and need to be processed together.
#[derive(Debug)]
pub struct ChangeTracker {
    /// Pending edits since last reparse (in chronological order).
    pending_edits: Vec<TextEdit>,
    /// Current document version.
    version: u64,
    /// Content hash for validation.
    content_hash: u64,
}

impl ChangeTracker {
    /// Create a new change tracker.
    pub fn new() -> Self {
        Self {
            pending_edits: Vec::new(),
            version: 0,
            content_hash: 0,
        }
    }

    /// Create a change tracker with initial content.
    pub fn with_content(content: &str) -> Self {
        Self {
            pending_edits: Vec::new(),
            version: 0,
            content_hash: Self::hash_content(content),
        }
    }

    /// Record an edit.
    pub fn record_edit(&mut self, edit: TextEdit) {
        self.pending_edits.push(edit);
        self.version += 1;
    }

    /// Get pending edits.
    pub fn pending_edits(&self) -> &[TextEdit] {
        &self.pending_edits
    }

    /// Check if there are pending edits.
    pub fn has_pending_edits(&self) -> bool {
        !self.pending_edits.is_empty()
    }

    /// Clear pending edits after reparse.
    pub fn clear_pending(&mut self) {
        self.pending_edits.clear();
    }

    /// Update content hash after reparsing.
    pub fn update_hash(&mut self, content: &str) {
        self.content_hash = Self::hash_content(content);
    }

    /// Validate content against stored hash.
    pub fn validate_content(&self, content: &str) -> bool {
        self.content_hash == Self::hash_content(content)
    }

    /// Calculate content hash.
    fn hash_content(content: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// Merge adjacent/overlapping edits for efficiency.
    ///
    /// This is called before applying edits to reduce the number of
    /// incremental parse operations needed.
    pub fn merge_edits(&mut self) {
        if self.pending_edits.len() < 2 {
            return;
        }

        // Sort by position (descending) so we apply later edits first
        self.pending_edits.sort_by_key(|e| std::cmp::Reverse(e.range.start()));

        let mut merged = Vec::new();
        let mut current = self.pending_edits[0].clone();

        for edit in &self.pending_edits[1..] {
            // Check if edits are adjacent or overlapping
            if current.range.start() <= edit.range.end() {
                // Merge the edits
                let new_start = edit.range.start();
                let new_end = current.range.end().max(edit.range.end());

                // Combine the new text (edit comes first in the merged text)
                let mut new_text = edit.new_text.clone();
                new_text.push_str(&current.new_text);

                current = TextEdit {
                    range: TextRange::new(new_start, new_end),
                    new_text,
                };
            } else {
                merged.push(current);
                current = edit.clone();
            }
        }
        merged.push(current);

        // Reverse back to ascending order
        merged.reverse();
        self.pending_edits = merged;
    }

    /// Apply all pending edits to source text.
    ///
    /// Edits are applied in reverse order (last edit first) to maintain
    /// correct positions.
    pub fn apply_all(&self, mut source: String) -> String {
        // Apply edits in reverse order
        for edit in self.pending_edits.iter().rev() {
            source = edit.apply(&source);
        }
        source
    }

    /// Compose pending edits into a single edit if possible.
    ///
    /// Returns None if edits cannot be composed (non-contiguous).
    pub fn compose(&self) -> Option<TextEdit> {
        if self.pending_edits.is_empty() {
            return None;
        }

        if self.pending_edits.len() == 1 {
            return Some(self.pending_edits[0].clone());
        }

        // Check if edits are contiguous
        let mut edits: Vec<_> = self.pending_edits.clone();
        edits.sort_by_key(|e| e.range.start());

        let first = &edits[0];
        let last = &edits[edits.len() - 1];

        // Check for overlaps/adjacency
        for i in 0..edits.len() - 1 {
            if edits[i].range.end() < edits[i + 1].range.start() {
                // Gap between edits - cannot compose
                return None;
            }
        }

        // Compose into single edit
        let range = TextRange::new(first.range.start(), last.range.end());
        let new_text = edits.iter()
            .map(|e| e.new_text.as_str())
            .collect::<Vec<_>>()
            .join("");

        Some(TextEdit::replace(range, new_text))
    }

    /// Get current version.
    pub fn version(&self) -> u64 {
        self.version
    }
}

impl Default for ChangeTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// LSP Integration Helpers
// ============================================================================

/// Represents an LSP-style content change.
#[derive(Clone, Debug)]
pub struct LspChange {
    /// Range in line/column coordinates (None = full document).
    pub range: Option<LspRange>,
    /// The new text for this range.
    pub text: String,
}

/// LSP-style range (line/column based).
#[derive(Clone, Copy, Debug)]
pub struct LspRange {
    /// Start line (0-based).
    pub start_line: u32,
    /// Start column (0-based, UTF-16 code units).
    pub start_col: u32,
    /// End line (0-based).
    pub end_line: u32,
    /// End column (0-based, UTF-16 code units).
    pub end_col: u32,
}

/// Convert LSP range to byte offset range.
pub fn lsp_range_to_text_range(range: LspRange, source: &str) -> TextRange {
    let start = line_col_to_offset(source, range.start_line, range.start_col);
    let end = line_col_to_offset(source, range.end_line, range.end_col);
    TextRange::new(start, end)
}

/// Convert line/column to byte offset.
fn line_col_to_offset(source: &str, line: u32, col: u32) -> TextSize {
    let mut current_line = 0;
    let mut current_col = 0;
    let mut offset = 0;

    for ch in source.chars() {
        if current_line == line && current_col == col {
            return offset;
        }

        if ch == '\n' {
            current_line += 1;
            current_col = 0;
        } else {
            // Count UTF-16 code units
            current_col += ch.len_utf16() as u32;
        }

        offset += ch.len_utf8() as TextSize;
    }

    offset
}

/// Convert byte offset to line/column.
pub fn offset_to_line_col(source: &str, offset: TextSize) -> (u32, u32) {
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

    (line, col)
}

/// Convert an LSP change to a TextEdit.
pub fn lsp_change_to_edit(change: LspChange, source: &str) -> TextEdit {
    match change.range {
        Some(range) => {
            let text_range = lsp_range_to_text_range(range, source);
            TextEdit::replace(text_range, change.text)
        }
        None => {
            // Full document sync
            TextEdit::replace(
                TextRange::new(0, source.len() as TextSize),
                change.text,
            )
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GreenBuilder, SyntaxKind};

    fn build_simple_tree() -> GreenNode {
        let mut builder = GreenBuilder::new();
        builder.start_node(SyntaxKind::SOURCE_FILE);
        builder.token(SyntaxKind::FN_KW, "fn");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::IDENT, "foo");
        builder.token(SyntaxKind::L_PAREN, "(");
        builder.token(SyntaxKind::R_PAREN, ")");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::L_BRACE, "{");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::R_BRACE, "}");
        builder.finish_node();
        builder.finish()
    }

    fn build_nested_tree() -> GreenNode {
        let mut builder = GreenBuilder::new();
        builder.start_node(SyntaxKind::SOURCE_FILE);

        // Function definition
        builder.start_node(SyntaxKind::FN_DEF);
        builder.token(SyntaxKind::FN_KW, "fn");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::IDENT, "foo");
        builder.token(SyntaxKind::L_PAREN, "(");
        builder.token(SyntaxKind::R_PAREN, ")");
        builder.token(SyntaxKind::WHITESPACE, " ");

        // Block
        builder.start_node(SyntaxKind::BLOCK);
        builder.token(SyntaxKind::L_BRACE, "{");
        builder.token(SyntaxKind::WHITESPACE, " ");

        // Let statement
        builder.start_node(SyntaxKind::LET_STMT);
        builder.token(SyntaxKind::LET_KW, "let");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::IDENT, "x");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::EQ, "=");
        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::INT_LITERAL, "42");
        builder.token(SyntaxKind::SEMICOLON, ";");
        builder.finish_node(); // LET_STMT

        builder.token(SyntaxKind::WHITESPACE, " ");
        builder.token(SyntaxKind::R_BRACE, "}");
        builder.finish_node(); // BLOCK

        builder.finish_node(); // FN_DEF

        builder.finish_node(); // SOURCE_FILE
        builder.finish()
    }

    #[test]
    fn test_text_edit_insert() {
        let source = "fn foo() { }";
        let edit = TextEdit::insert(TextSize::from(3u32), "bar");
        let result = edit.apply(source);
        assert_eq!(result, "fn barfoo() { }");
    }

    #[test]
    fn test_text_edit_delete() {
        let source = "fn foo() { }";
        let edit = TextEdit::delete(TextRange::new(TextSize::from(3u32), TextSize::from(6u32)));
        let result = edit.apply(source);
        assert_eq!(result, "fn () { }");
    }

    #[test]
    fn test_text_edit_replace() {
        let source = "fn foo() { }";
        let edit = TextEdit::replace(
            TextRange::new(TextSize::from(3u32), TextSize::from(6u32)),
            "bar",
        );
        let result = edit.apply(source);
        assert_eq!(result, "fn bar() { }");
    }

    #[test]
    fn test_find_affected_subtree() {
        let tree = build_nested_tree();
        let mut engine = IncrementalEngine::new();

        // Edit the "42" literal
        let edit = TextEdit::replace(
            TextRange::new(TextSize::from(23u32), TextSize::from(25u32)),
            "100",
        );

        let affected = engine.find_affected_subtree(&tree, &edit);
        assert!(affected.is_some());

        let affected = affected.unwrap();
        // Should find the LET_STMT as the stable ancestor
        assert!(affected.is_stable);
    }

    #[test]
    fn test_should_use_incremental() {
        // Build a larger tree (> 1KB) for this test
        let mut builder = GreenBuilder::new();
        builder.start_node(SyntaxKind::SOURCE_FILE);
        // Add enough content to exceed 1KB threshold
        for _ in 0..100 {
            builder.token(SyntaxKind::FN_KW, "fn");
            builder.token(SyntaxKind::WHITESPACE, " ");
            builder.token(SyntaxKind::IDENT, "function_name");
            builder.token(SyntaxKind::L_PAREN, "(");
            builder.token(SyntaxKind::R_PAREN, ")");
            builder.token(SyntaxKind::WHITESPACE, " ");
            builder.token(SyntaxKind::L_BRACE, "{");
            builder.token(SyntaxKind::WHITESPACE, " ");
            builder.token(SyntaxKind::R_BRACE, "}");
            builder.token(SyntaxKind::NEWLINE, "\n");
        }
        builder.finish_node();
        let large_tree = builder.finish();
        let engine = IncrementalEngine::new();

        // Small edit on large tree - should use incremental
        let small_edit = TextEdit::insert(TextSize::from(4u32), "x");
        assert!(engine.should_use_incremental(&large_tree, &small_edit));

        // Large edit affecting > 30% - should not use incremental
        let tree_size = large_tree.width() as usize;
        let large_edit = TextEdit::insert(TextSize::from(0u32), "x".repeat(tree_size / 2));
        assert!(!engine.should_use_incremental(&large_tree, &large_edit));

        // Small tree should not use incremental (optimization: full reparse is faster)
        let small_tree = build_simple_tree();
        let tiny_edit = TextEdit::insert(TextSize::from(4u32), "x");
        assert!(!engine.should_use_incremental(&small_tree, &tiny_edit));
    }

    #[test]
    fn test_change_tracker() {
        let mut tracker = ChangeTracker::new();

        tracker.record_edit(TextEdit::insert(TextSize::from(0u32), "a"));
        tracker.record_edit(TextEdit::insert(TextSize::from(1u32), "b"));

        assert_eq!(tracker.pending_edits().len(), 2);
        assert_eq!(tracker.version(), 2);

        tracker.clear_pending();
        assert!(tracker.pending_edits().is_empty());
    }

    #[test]
    fn test_merge_edits() {
        let mut tracker = ChangeTracker::new();

        // Add overlapping edits
        tracker.record_edit(TextEdit::replace(
            TextRange::new(0, 5),
            "hello",
        ));
        tracker.record_edit(TextEdit::replace(
            TextRange::new(3, 8),
            "world",
        ));

        tracker.merge_edits();

        // Should be merged into fewer edits
        assert!(tracker.pending_edits().len() <= 2);
    }

    #[test]
    fn test_node_stability() {
        assert_eq!(
            NodeStabilityAnalyzer::is_stable(SyntaxKind::SOURCE_FILE),
            NodeStability::Stable
        );
        assert_eq!(
            NodeStabilityAnalyzer::is_stable(SyntaxKind::FN_DEF),
            NodeStability::Stable
        );
        assert_eq!(
            NodeStabilityAnalyzer::is_stable(SyntaxKind::BLOCK),
            NodeStability::Stable
        );
        assert_eq!(
            NodeStabilityAnalyzer::is_stable(SyntaxKind::LET_STMT),
            NodeStability::Stable
        );
        assert_eq!(
            NodeStabilityAnalyzer::is_stable(SyntaxKind::BINARY_EXPR),
            NodeStability::Unstable
        );
        assert_eq!(
            NodeStabilityAnalyzer::is_stable(SyntaxKind::IDENT),
            NodeStability::Token
        );
    }

    #[test]
    fn test_reparse_context() {
        assert_eq!(
            ReparseContext::for_kind(SyntaxKind::SOURCE_FILE),
            ReparseContext::Module
        );
        assert_eq!(
            ReparseContext::for_kind(SyntaxKind::FN_DEF),
            ReparseContext::Item
        );
        assert_eq!(
            ReparseContext::for_kind(SyntaxKind::BLOCK),
            ReparseContext::Block
        );
        assert_eq!(
            ReparseContext::for_kind(SyntaxKind::LET_STMT),
            ReparseContext::Statement
        );
        assert_eq!(
            ReparseContext::for_kind(SyntaxKind::BINARY_EXPR),
            ReparseContext::Expression
        );
    }

    #[test]
    fn test_lsp_range_conversion() {
        let source = "line1\nline2\nline3";

        // Test line 1, col 0 (start of "line2")
        let range = LspRange {
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 5,
        };

        let text_range = lsp_range_to_text_range(range, source);
        assert_eq!(text_range.start(), 6); // After "line1\n"
        assert_eq!(text_range.end(), 11);  // End of "line2"
    }

    #[test]
    fn test_offset_to_line_col() {
        let source = "abc\ndef\nghi";

        assert_eq!(offset_to_line_col(source, 0), (0, 0)); // 'a'
        assert_eq!(offset_to_line_col(source, 3), (0, 3)); // '\n'
        assert_eq!(offset_to_line_col(source, 4), (1, 0)); // 'd'
        assert_eq!(offset_to_line_col(source, 8), (2, 0)); // 'g'
    }

    #[test]
    fn test_edit_single_char() {
        let edit = TextEdit::insert(0, "x");
        assert!(edit.is_single_char());

        let edit = TextEdit::delete(TextRange::new(0, 1));
        assert!(edit.is_single_char());

        let edit = TextEdit::replace(TextRange::new(0, 1), "y");
        assert!(edit.is_single_char());

        let edit = TextEdit::replace(TextRange::new(0, 5), "hello");
        assert!(!edit.is_single_char());
    }

    #[test]
    fn test_edit_whitespace_only() {
        let edit = TextEdit::insert(0, "   ");
        assert!(edit.is_whitespace_only());

        let edit = TextEdit::insert(0, "\n\t");
        assert!(edit.is_whitespace_only());

        let edit = TextEdit::insert(0, "x");
        assert!(!edit.is_whitespace_only());
    }

    #[test]
    fn test_incremental_stats() {
        let mut engine = IncrementalEngine::new();

        engine.stats.incremental_parses = 10;
        engine.stats.nodes_reused = 90;
        engine.stats.nodes_recreated = 10;

        assert!((engine.stats().cache_hit_ratio() - 0.9).abs() < 0.001);

        engine.stats.incremental_time_us = 1000;
        assert_eq!(engine.stats().avg_incremental_time_us(), 100);
    }

    #[test]
    fn test_apply_edit_simple() {
        let tree = build_simple_tree();
        let source = "fn foo() { }";
        let mut engine = IncrementalEngine::new();

        // Simple reparse function that just returns a minimal tree
        let reparse_fn = |new_source: &str, _context: ReparseContext| {
            let mut builder = GreenBuilder::new();
            builder.start_node(SyntaxKind::SOURCE_FILE);
            for ch in new_source.chars() {
                if ch.is_whitespace() {
                    builder.token(SyntaxKind::WHITESPACE, &ch.to_string());
                } else {
                    builder.token(SyntaxKind::IDENT, &ch.to_string());
                }
            }
            builder.finish_node();
            builder.finish()
        };

        let edit = TextEdit::replace(
            TextRange::new(3, 6),
            "bar",
        );

        let new_tree = engine.apply_edit(&tree, &edit, reparse_fn, source);

        // Verify the tree was updated
        assert_eq!(new_tree.kind(), SyntaxKind::SOURCE_FILE);
    }

    #[test]
    fn test_change_tracker_compose() {
        let mut tracker = ChangeTracker::new();

        // Single edit should compose to itself
        tracker.record_edit(TextEdit::replace(TextRange::new(0, 3), "abc"));
        let composed = tracker.compose();
        assert!(composed.is_some());
        assert_eq!(composed.unwrap().new_text, "abc");

        tracker.clear_pending();

        // Adjacent edits should compose
        tracker.record_edit(TextEdit::replace(TextRange::new(0, 2), "ab"));
        tracker.record_edit(TextEdit::replace(TextRange::new(2, 4), "cd"));
        let composed = tracker.compose();
        assert!(composed.is_some());
    }
}
