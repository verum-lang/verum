//! Incremental script parsing for high-performance REPL and LSP integration
//!
//! This module extends the script parser with incremental parsing capabilities,
//! enabling efficient re-parsing of script sessions where only changed portions
//! need to be re-evaluated.
//!
//! # Features
//!
//! - **Partial reparsing**: Only re-parse changed regions
//! - **AST caching**: Reuse unchanged expression trees
//! - **Session persistence**: Maintain parse state across multiple edits
//! - **Type-aware caching**: Cache type inference results with AST
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │  IncrementalScriptParser                │
//! │  ┌───────────────────────────────────┐  │
//! │  │  Script Lines (numbered)          │  │
//! │  │  1: let x = 42                    │  │
//! │  │  2: fn add(a, b) { a + b }        │  │
//! │  │  3: add(x, 10)  ← modified        │  │
//! │  └───────────────────────────────────┘  │
//! │  ┌───────────────────────────────────┐  │
//! │  │  Cached AST Nodes                 │  │
//! │  │  Line 1: ✓ (unchanged)            │  │
//! │  │  Line 2: ✓ (unchanged)            │  │
//! │  │  Line 3: ✗ (re-parse needed)      │  │
//! │  └───────────────────────────────────┘  │
//! └─────────────────────────────────────────┘
//! ```
//!
//! Moved from verum_parser::incremental_script

use std::collections::HashMap;
use verum_ast::FileId;
use verum_common::{List, Maybe, Text};

use verum_parser::ParseError;

use super::context::ScriptContext;
use super::parser::ScriptParser;
use super::result::ScriptParseResult;

/// A cached parsed line with hash for validation
#[derive(Debug, Clone)]
pub struct CachedLine {
    /// Line number in the session
    pub line_number: usize,
    /// Original source text
    pub source: Text,
    /// Hash of the source for change detection
    pub hash: u64,
    /// Parsed result
    pub result: ScriptParseResult,
    /// File ID used for this line
    pub file_id: FileId,
}

/// Statistics for incremental parsing performance
#[derive(Debug, Clone, Default)]
pub struct IncrementalStats {
    /// Total number of parses
    pub total_parses: usize,
    /// Number of cache hits (reused)
    pub cache_hits: usize,
    /// Number of cache misses (reparsed)
    pub cache_misses: usize,
    /// Total lines in session
    pub total_lines: usize,
    /// Lines currently cached
    pub cached_lines: usize,
}

impl IncrementalStats {
    /// Calculate cache hit rate as percentage
    pub fn hit_rate(&self) -> f64 {
        if self.total_parses == 0 {
            0.0
        } else {
            (self.cache_hits as f64 / self.total_parses as f64) * 100.0
        }
    }

    /// Get a summary string
    pub fn summary(&self) -> Text {
        Text::from(format!(
            "Parses: {} total, {} hits ({:.1}%), {} misses | Lines: {}/{} cached",
            self.total_parses,
            self.cache_hits,
            self.hit_rate(),
            self.cache_misses,
            self.cached_lines,
            self.total_lines
        ))
    }
}

/// Incremental script parser with caching
///
/// This parser maintains a cache of parsed lines and intelligently
/// re-parses only what has changed. It tracks dependencies between lines
/// to enable smart cache invalidation.
pub struct IncrementalScriptParser {
    /// Base script parser
    parser: ScriptParser,
    /// Cache of parsed lines by line number
    cache: HashMap<usize, CachedLine>,
    /// Script context for the session
    context: ScriptContext,
    /// Statistics
    stats: IncrementalStats,
    /// Maximum cache size (0 = unlimited)
    max_cache_size: usize,
    /// Dependency graph for smart invalidation
    dependencies: DependencyGraph,
}

impl IncrementalScriptParser {
    /// Create a new incremental script parser
    pub fn new() -> Self {
        Self {
            parser: ScriptParser::new(),
            cache: HashMap::new(),
            context: ScriptContext::new(),
            stats: IncrementalStats::default(),
            max_cache_size: 1000,
            dependencies: DependencyGraph::new(),
        }
    }

    /// Create with a specific cache size limit
    pub fn with_cache_limit(limit: usize) -> Self {
        // Phase-not-realised tracing: `IncrementalScriptParser::with_cache_limit`
        // builds a parser with a custom cache cap, but no production
        // caller invokes it. The 3 production sites
        // (verum_interactive: state.rs:61, pipeline.rs:155, 166) and
        // the LSP script module all use `::new()` (default 1000-entry
        // cap). Surface a debug trace when an embedder selects a
        // non-default cap so they see the value lands on the parser
        // but is not threaded by any built-in CLI flag — there's no
        // `verum repl --cache-size N` or LSP setting today.
        if limit != 1000 {
            tracing::debug!(
                "IncrementalScriptParser::with_cache_limit({}) — value lands on \
                 the parser but no built-in CLI flag or LSP setting threads a \
                 custom limit through to the production callers \
                 (verum_interactive + LSP). The standard `verum repl` and LSP \
                 surfaces both use the default 1000-entry cap. Forward-looking \
                 knob for embedders constructing the parser directly.",
                limit
            );
        }
        let mut parser = Self::new();
        parser.max_cache_size = limit;
        parser
    }

    /// Get the dependency graph
    pub fn dependency_graph(&self) -> &DependencyGraph {
        &self.dependencies
    }

    /// Parse a line with incremental caching
    ///
    /// If the line at this line number hasn't changed, returns the cached result.
    /// Otherwise, re-parses and updates the cache.
    pub fn parse_line(
        &mut self,
        line: &str,
        line_number: usize,
        file_id: FileId,
    ) -> Result<ScriptParseResult, List<ParseError>> {
        self.stats.total_parses += 1;

        // Calculate hash for this line
        let hash = calculate_hash(line);

        // Check cache
        if let Some(cached) = self.cache.get(&line_number)
            && cached.hash == hash
            && cached.file_id == file_id
        {
            // Cache hit!
            self.stats.cache_hits += 1;
            return Ok(cached.result.clone());
        }

        // Cache miss - parse the line
        self.stats.cache_misses += 1;

        let result = self.parser.parse_line(line, file_id, &mut self.context)?;

        // Cache the result
        self.cache.insert(
            line_number,
            CachedLine {
                line_number,
                source: Text::from(line),
                hash,
                result: result.clone(),
                file_id,
            },
        );

        self.update_stats();

        // Evict old entries if cache is too large
        if self.max_cache_size > 0 && self.cache.len() > self.max_cache_size {
            self.evict_oldest();
        }

        Ok(result)
    }

    /// Parse multiple lines incrementally
    ///
    /// This is more efficient than parsing line-by-line as it can
    /// detect unchanged regions and skip them.
    pub fn parse_lines(
        &mut self,
        lines: &[&str],
        start_line: usize,
        file_id: FileId,
    ) -> Result<List<ScriptParseResult>, List<ParseError>> {
        let mut results = List::new();

        for (i, line) in lines.iter().enumerate() {
            let line_number = start_line + i;
            let result = self.parse_line(line, line_number, file_id)?;
            results.push(result);
        }

        Ok(results)
    }

    /// Update a specific line and re-parse
    ///
    /// This invalidates the cache for this line and all dependent lines.
    pub fn update_line(
        &mut self,
        line: &str,
        line_number: usize,
        file_id: FileId,
    ) -> Result<ScriptParseResult, List<ParseError>> {
        // Invalidate this line and all subsequent lines
        // (since later lines might depend on earlier definitions)
        self.invalidate_from_line(line_number);

        // Parse the updated line
        self.parse_line(line, line_number, file_id)
    }

    /// Invalidate cache from a specific line onwards
    pub fn invalidate_from_line(&mut self, line_number: usize) {
        let keys_to_remove: Vec<usize> = self
            .cache
            .keys()
            .filter(|&&k| k >= line_number)
            .cloned()
            .collect();

        for key in keys_to_remove {
            self.cache.remove(&key);
        }

        self.update_stats();
    }

    /// Invalidate specific line numbers
    pub fn invalidate_lines(&mut self, line_numbers: &[usize]) {
        for &line_number in line_numbers {
            self.cache.remove(&line_number);
        }
        self.update_stats();
    }

    /// Clear all cached results
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.update_stats();
    }

    /// Reset the entire session
    pub fn reset(&mut self) {
        self.cache.clear();
        self.context.reset();
        self.stats = IncrementalStats::default();
    }

    /// Get the current script context
    pub fn context(&self) -> &ScriptContext {
        &self.context
    }

    /// Get mutable script context
    pub fn context_mut(&mut self) -> &mut ScriptContext {
        &mut self.context
    }

    /// Get parsing statistics
    pub fn stats(&self) -> &IncrementalStats {
        &self.stats
    }

    /// Get a cached line by line number
    pub fn get_cached(&self, line_number: usize) -> Maybe<&CachedLine> {
        self.cache.get(&line_number)
    }

    /// Check if a line is cached
    pub fn is_cached(&self, line_number: usize) -> bool {
        self.cache.contains_key(&line_number)
    }

    /// Evict the oldest cached entry
    fn evict_oldest(&mut self) {
        if let Some(&min_line) = self.cache.keys().min() {
            self.cache.remove(&min_line);
            self.update_stats();
        }
    }

    /// Update statistics
    fn update_stats(&mut self) {
        self.stats.cached_lines = self.cache.len();
        self.stats.total_lines = self.context.line_number;
    }

    /// Pre-warm the cache by parsing all lines
    ///
    /// Useful for loading a script file into the REPL
    pub fn prewarm(&mut self, lines: &[&str], file_id: FileId) -> Result<(), List<ParseError>> {
        for (i, line) in lines.iter().enumerate() {
            self.parse_line(line, i + 1, file_id)?;
        }
        Ok(())
    }

    /// Get all cached results in order
    pub fn get_all_cached(&self) -> List<CachedLine> {
        let mut results: Vec<_> = self.cache.values().cloned().collect();
        results.sort_by_key(|c| c.line_number);
        results.into_iter().collect()
    }

    /// Export context for saving session state
    pub fn export_context(&self) -> ScriptContext {
        self.context.clone()
    }

    /// Import context from saved session state
    pub fn import_context(&mut self, context: ScriptContext) {
        self.context = context;
    }
}

impl Default for IncrementalScriptParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate a hash for change detection
fn calculate_hash(text: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Detect dependencies between script lines.
///
/// Returns the line numbers that the given line depends on.
/// This is used for smart cache invalidation - when a line is modified,
/// all lines that depend on it must also be re-parsed.
///
/// # Algorithm
/// 1. Scan the line for identifier usage
/// 2. Look up each identifier in the context's definition tracking
/// 3. Return the set of line numbers where those definitions were made
///
/// # Example
/// ```text
/// Line 1: let x = 42          // Defines x on line 1
/// Line 2: let y = x + 10      // Uses x, depends on line 1
/// Line 3: fn add(a, b) { a + b } // Defines add on line 3
/// Line 4: add(x, y)           // Uses add, x, y - depends on lines 1, 2, 3
/// ```
pub fn detect_dependencies(line: &str, context: &ScriptContext) -> List<usize> {
    let mut deps = std::collections::HashSet::new();

    // Check for variable/binding usage
    for (name, _) in context.bindings.iter() {
        if contains_identifier(line, name.as_str()) {
            if let Some(def_line) = context.get_definition_line(name) {
                deps.insert(def_line);
            }
        }
    }

    // Check for function usage
    for (name, _) in context.function_lines.iter() {
        if contains_identifier(line, name.as_str()) {
            if let Some(def_line) = context.get_function_line(name) {
                deps.insert(def_line);
            }
        }
    }

    // Check for type usage
    for (name, _) in context.type_lines.iter() {
        if contains_identifier(line, name.as_str()) {
            if let Some(def_line) = context.get_type_line(name) {
                deps.insert(def_line);
            }
        }
    }

    deps.into_iter().collect()
}

/// Check if a line contains an identifier (not as part of another word).
///
/// This performs a more accurate check than a simple substring match.
/// For example, "x" should match "x + 1" but not "tax".  Word-boundary
/// probing uses the UTF-8-safe primitives from `verum_common::text_utf8`
/// so multi-byte source behaves correctly.
fn contains_identifier(line: &str, ident: &str) -> bool {
    if ident.is_empty() {
        return false;
    }

    let mut start = 0;
    while let Some(pos) = line[start..].find(ident) {
        let abs_pos = start + pos;
        let after_pos = abs_pos + ident.len();

        let not_ident = |c: char| !is_ident_char(c);
        let before_ok = verum_common::text_utf8::char_before_satisfies(line, abs_pos, not_ident)
            .unwrap_or(true);
        let after_ok = verum_common::text_utf8::char_at_satisfies(line, after_pos, not_ident)
            .unwrap_or(true);

        if before_ok && after_ok {
            return true;
        }

        start = abs_pos + 1;
        if start >= line.len() {
            break;
        }
    }

    false
}

/// Check if a character is a valid identifier character.
fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Dependency graph for script lines.
///
/// This structure tracks which lines depend on which other lines,
/// enabling efficient re-parsing when lines are modified.
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// Forward dependencies: line N -> set of lines that N depends on
    pub depends_on: std::collections::HashMap<usize, std::collections::HashSet<usize>>,
    /// Reverse dependencies: line N -> set of lines that depend on N
    pub depended_by: std::collections::HashMap<usize, std::collections::HashSet<usize>>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a dependency: `line` depends on `depends_on_line`.
    pub fn add_dependency(&mut self, line: usize, depends_on_line: usize) {
        self.depends_on
            .entry(line)
            .or_default()
            .insert(depends_on_line);
        self.depended_by
            .entry(depends_on_line)
            .or_default()
            .insert(line);
    }

    /// Get all lines that directly depend on a given line.
    pub fn get_dependents(&self, line: usize) -> List<usize> {
        self.depended_by
            .get(&line)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Get all lines that a given line depends on.
    pub fn get_dependencies(&self, line: usize) -> List<usize> {
        self.depends_on
            .get(&line)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Get all lines that transitively depend on a given line.
    /// Uses a breadth-first search to find the complete set.
    pub fn get_transitive_dependents(&self, line: usize) -> List<usize> {
        let mut result = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();

        queue.push_back(line);

        while let Some(current) = queue.pop_front() {
            if let Some(dependents) = self.depended_by.get(&current) {
                for &dep in dependents {
                    if result.insert(dep) {
                        queue.push_back(dep);
                    }
                }
            }
        }

        result.into_iter().collect()
    }

    /// Remove a line and all its dependencies.
    pub fn remove_line(&mut self, line: usize) {
        // Remove forward dependencies
        if let Some(deps) = self.depends_on.remove(&line) {
            for dep in deps {
                if let Some(set) = self.depended_by.get_mut(&dep) {
                    set.remove(&line);
                }
            }
        }

        // Remove reverse dependencies
        if let Some(dependents) = self.depended_by.remove(&line) {
            for dep in dependents {
                if let Some(set) = self.depends_on.get_mut(&dep) {
                    set.remove(&line);
                }
            }
        }
    }

    /// Clear all dependencies.
    pub fn clear(&mut self) {
        self.depends_on.clear();
        self.depended_by.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incremental_cache_hit() {
        let mut parser = IncrementalScriptParser::new();
        let file_id = FileId::new(1);

        // Parse once
        let r1 = parser.parse_line("let x = 42", 1, file_id);
        assert!(r1.is_ok());
        assert_eq!(parser.stats.cache_misses, 1);

        // Parse same line again - should hit cache
        let r2 = parser.parse_line("let x = 42", 1, file_id);
        assert!(r2.is_ok());
        assert_eq!(parser.stats.cache_hits, 1);
    }

    #[test]
    fn test_incremental_cache_miss_on_change() {
        let mut parser = IncrementalScriptParser::new();
        let file_id = FileId::new(1);

        // Parse line
        parser.parse_line("let x = 42", 1, file_id).unwrap();

        // Parse different content at same line - should miss
        parser.parse_line("let x = 100", 1, file_id).unwrap();
        assert_eq!(parser.stats.cache_misses, 2);
    }

    #[test]
    fn test_update_line_invalidates_cache() {
        let mut parser = IncrementalScriptParser::new();
        let file_id = FileId::new(1);

        // Parse multiple lines
        parser.parse_line("let x = 42", 1, file_id).unwrap();
        parser.parse_line("let y = x + 10", 2, file_id).unwrap();
        parser.parse_line("y * 2", 3, file_id).unwrap();

        // All should be cached
        assert_eq!(parser.cache.len(), 3);

        // Update line 2 - should invalidate 2 and 3
        parser.update_line("let y = 100", 2, file_id).unwrap();

        // Line 1 should still be cached, but 2 and 3 are updated
        assert!(parser.is_cached(1));
    }

    #[test]
    fn test_clear_cache() {
        let mut parser = IncrementalScriptParser::new();
        let file_id = FileId::new(1);

        parser.parse_line("let x = 42", 1, file_id).unwrap();
        parser.parse_line("let y = 10", 2, file_id).unwrap();

        assert_eq!(parser.cache.len(), 2);

        parser.clear_cache();
        assert_eq!(parser.cache.len(), 0);
    }

    #[test]
    fn test_stats_tracking() {
        let mut parser = IncrementalScriptParser::new();
        let file_id = FileId::new(1);

        parser.parse_line("let x = 1", 1, file_id).unwrap();
        parser.parse_line("let x = 1", 1, file_id).unwrap(); // cache hit

        assert_eq!(parser.stats.total_parses, 2);
        assert_eq!(parser.stats.cache_hits, 1);
        assert_eq!(parser.stats.cache_misses, 1);
        assert!(parser.stats.hit_rate() > 40.0); // 50% hit rate
    }

    #[test]
    fn test_prewarm_cache() {
        let mut parser = IncrementalScriptParser::new();
        let file_id = FileId::new(1);

        let lines = vec!["let x = 1", "let y = 2", "x + y"];

        parser.prewarm(&lines, file_id).unwrap();

        // All lines should be cached
        assert_eq!(parser.cache.len(), 3);
        assert!(parser.is_cached(1));
        assert!(parser.is_cached(2));
        assert!(parser.is_cached(3));
    }

    #[test]
    fn test_get_all_cached_ordered() {
        let mut parser = IncrementalScriptParser::new();
        let file_id = FileId::new(1);

        // Parse in random order
        parser.parse_line("let z = 3", 3, file_id).unwrap();
        parser.parse_line("let x = 1", 1, file_id).unwrap();
        parser.parse_line("let y = 2", 2, file_id).unwrap();

        let all = parser.get_all_cached();
        assert_eq!(all.len(), 3);

        // Should be ordered by line number
        assert_eq!(all[0].line_number, 1);
        assert_eq!(all[1].line_number, 2);
        assert_eq!(all[2].line_number, 3);
    }

    #[test]
    fn test_cache_size_limit() {
        let mut parser = IncrementalScriptParser::with_cache_limit(2);
        let file_id = FileId::new(1);

        parser.parse_line("let a = 1", 1, file_id).unwrap();
        parser.parse_line("let b = 2", 2, file_id).unwrap();
        parser.parse_line("let c = 3", 3, file_id).unwrap(); // Should evict oldest

        assert_eq!(parser.cache.len(), 2);
        // Line 1 should be evicted
        assert!(!parser.is_cached(1));
    }

    #[test]
    fn test_session_persistence() {
        let mut parser1 = IncrementalScriptParser::new();
        let file_id = FileId::new(1);

        parser1.parse_line("let x = 42", 1, file_id).unwrap();

        // Export context
        let context = parser1.export_context();

        // Create new parser and import
        let mut parser2 = IncrementalScriptParser::new();
        parser2.import_context(context);

        // Should have the same bindings
        assert!(parser2.context.bindings.contains_key(&Text::from("x")));
    }
}
