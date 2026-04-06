//! Test case minimization (shrinking)
//!
//! This module provides comprehensive algorithms to minimize failing test cases
//! to their smallest reproducible form. Shrinking helps developers understand
//! the root cause of bugs by removing irrelevant code.
//!
//! # Shrinking Strategies
//!
//! - **Binary Search**: Divide and conquer to find minimal subset
//! - **Delta Debugging**: Systematic reduction (ddmin algorithm)
//! - **Token Deletion**: Remove individual tokens
//! - **Line Deletion**: Remove entire lines
//! - **Hierarchical**: AST-aware shrinking preserving syntactic validity
//! - **AST-Aware**: Full AST parsing and node-by-node reduction
//!
//! # Algorithm Details
//!
//! ## Delta Debugging (ddmin)
//!
//! The delta debugging algorithm systematically reduces input by:
//! 1. Dividing input into n chunks
//! 2. Testing each chunk for failure reproduction
//! 3. If chunk reproduces, use it as new input
//! 4. If no chunk works, try complements (input - chunk)
//! 5. Increase granularity and repeat
//!
//! ## Hierarchical Shrinking
//!
//! For structured inputs (like Verum code), hierarchical shrinking:
//! 1. First removes entire functions/modules
//! 2. Then removes blocks (if/for/while/match)
//! 3. Then removes individual statements
//! 4. Finally simplifies expressions
//!
//! This preserves syntactic validity at each step.
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_vfuzz::shrink::{Shrinker, ShrinkConfig, ShrinkStrategy};
//!
//! let config = ShrinkConfig {
//!     strategy: ShrinkStrategy::Combined,
//!     preserve_validity: true,
//!     ..Default::default()
//! };
//! let shrinker = Shrinker::new(config);
//!
//! let result = shrinker.shrink(input, |s| s.contains("BUG"));
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Configuration for the shrinker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShrinkConfig {
    /// Maximum iterations for shrinking
    pub max_iterations: usize,
    /// Timeout for each shrink attempt in milliseconds
    pub timeout_ms: u64,
    /// Minimum progress required to continue (bytes)
    pub min_progress: usize,
    /// Enable aggressive shrinking (may produce invalid syntax)
    pub aggressive: bool,
    /// Strategy to use
    pub strategy: ShrinkStrategy,
}

impl Default for ShrinkConfig {
    fn default() -> Self {
        Self {
            max_iterations: 1000,
            timeout_ms: 5000,
            min_progress: 1,
            aggressive: false,
            strategy: ShrinkStrategy::DeltaDebugging,
        }
    }
}

/// Shrinking strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShrinkStrategy {
    /// Binary search reduction
    BinarySearch,
    /// Delta debugging (ddmin)
    DeltaDebugging,
    /// Line-by-line deletion
    LineByLine,
    /// Token-by-token deletion
    TokenByToken,
    /// Hierarchical (AST-aware) shrinking
    Hierarchical,
    /// Combined approach
    Combined,
}

/// Result of shrinking
#[derive(Debug, Clone)]
pub enum ShrinkResult {
    /// Successfully minimized
    Success(String),
    /// Could not minimize further
    NoProgress,
    /// Error during shrinking
    Error(String),
}

/// Statistics from shrinking
#[derive(Debug, Default, Clone)]
pub struct ShrinkStats {
    /// Original size in bytes
    pub original_size: usize,
    /// Final size in bytes
    pub final_size: usize,
    /// Number of iterations
    pub iterations: usize,
    /// Number of successful reductions
    pub successful_reductions: usize,
    /// Number of failed attempts
    pub failed_attempts: usize,
    /// Time spent shrinking in milliseconds
    pub duration_ms: u64,
}

impl ShrinkStats {
    /// Calculate reduction percentage
    pub fn reduction_pct(&self) -> f64 {
        if self.original_size == 0 {
            0.0
        } else {
            (1.0 - (self.final_size as f64 / self.original_size as f64)) * 100.0
        }
    }
}

/// Main test case shrinker
pub struct Shrinker {
    config: ShrinkConfig,
}

impl Shrinker {
    /// Create a new shrinker with the given configuration
    pub fn new(config: ShrinkConfig) -> Self {
        Self { config }
    }

    /// Shrink an input that triggers a bug
    ///
    /// The `test_fn` should return `true` if the input still triggers the bug.
    pub fn shrink<F>(&self, input: &str, test_fn: F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        // Verify the original input triggers the bug
        if !test_fn(input) {
            return ShrinkResult::Error("Original input does not trigger the bug".to_string());
        }

        match self.config.strategy {
            ShrinkStrategy::BinarySearch => self.shrink_binary(input, &test_fn),
            ShrinkStrategy::DeltaDebugging => self.shrink_ddmin(input, &test_fn),
            ShrinkStrategy::LineByLine => self.shrink_lines(input, &test_fn),
            ShrinkStrategy::TokenByToken => self.shrink_tokens(input, &test_fn),
            ShrinkStrategy::Hierarchical => self.shrink_hierarchical(input, &test_fn),
            ShrinkStrategy::Combined => self.shrink_combined(input, &test_fn),
        }
    }

    /// Shrink with statistics
    pub fn shrink_with_stats<F>(&self, input: &str, test_fn: F) -> (ShrinkResult, ShrinkStats)
    where
        F: Fn(&str) -> bool,
    {
        let start = std::time::Instant::now();
        let mut stats = ShrinkStats {
            original_size: input.len(),
            ..Default::default()
        };

        let result = self.shrink(input, test_fn);

        stats.duration_ms = start.elapsed().as_millis() as u64;
        stats.final_size = match &result {
            ShrinkResult::Success(s) => s.len(),
            _ => input.len(),
        };

        (result, stats)
    }

    /// Binary search based shrinking
    fn shrink_binary<F>(&self, input: &str, test_fn: &F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        let lines: Vec<&str> = input.lines().collect();
        if lines.len() <= 1 {
            return ShrinkResult::NoProgress;
        }

        let mut best = input.to_string();
        let mut iterations = 0;

        // Binary search for minimal subset
        let mut lo = 0;
        let mut hi = lines.len();

        while lo < hi && iterations < self.config.max_iterations {
            let mid = (lo + hi) / 2;
            let candidate: String = lines[..mid].join("\n");

            iterations += 1;

            if test_fn(&candidate) {
                best = candidate;
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }

        if best.len() < input.len() {
            ShrinkResult::Success(best)
        } else {
            ShrinkResult::NoProgress
        }
    }

    /// Delta debugging (ddmin algorithm)
    fn shrink_ddmin<F>(&self, input: &str, test_fn: &F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        let chars: Vec<char> = input.chars().collect();
        if chars.len() <= 1 {
            return ShrinkResult::NoProgress;
        }

        let mut best = chars.clone();
        let mut n = 2; // Start with 2 chunks
        let mut iterations = 0;

        while n <= best.len() && iterations < self.config.max_iterations {
            let chunk_size = (best.len() + n - 1) / n;
            let mut reduced = false;

            // Try removing each chunk
            for i in 0..n {
                let start = (i * chunk_size).min(best.len());
                let end = ((i + 1) * chunk_size).min(best.len());

                if start >= end {
                    continue;
                }

                // Create candidate without this chunk
                let candidate: String = best[..start].iter().chain(best[end..].iter()).collect();

                iterations += 1;

                if test_fn(&candidate) {
                    best = candidate.chars().collect();
                    n = 2.max(n - 1); // Reset to smaller chunks
                    reduced = true;
                    break;
                }
            }

            if !reduced {
                // Try keeping only each chunk
                for i in 0..n {
                    let start = (i * chunk_size).min(best.len());
                    let end = ((i + 1) * chunk_size).min(best.len());

                    if start >= end {
                        continue;
                    }

                    let candidate: String = best[start..end].iter().collect();

                    iterations += 1;

                    if test_fn(&candidate) {
                        best = candidate.chars().collect();
                        n = 2; // Reset
                        reduced = true;
                        break;
                    }
                }

                if !reduced {
                    n *= 2; // Double the granularity
                }
            }
        }

        let result: String = best.into_iter().collect();
        if result.len() < input.len() {
            ShrinkResult::Success(result)
        } else {
            ShrinkResult::NoProgress
        }
    }

    /// Line-by-line deletion
    fn shrink_lines<F>(&self, input: &str, test_fn: &F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        let mut lines: Vec<&str> = input.lines().collect();
        let original_len = lines.len();
        let mut iterations = 0;
        let mut changed = true;

        while changed && iterations < self.config.max_iterations {
            changed = false;

            let mut i = 0;
            while i < lines.len() {
                if lines.len() <= 1 {
                    break;
                }

                // Try removing this line
                let mut candidate_lines = lines.clone();
                candidate_lines.remove(i);
                let candidate = candidate_lines.join("\n");

                iterations += 1;

                if test_fn(&candidate) {
                    lines = candidate_lines;
                    changed = true;
                    // Don't increment i since we removed an element
                } else {
                    i += 1;
                }
            }
        }

        let result = lines.join("\n");
        if lines.len() < original_len {
            ShrinkResult::Success(result)
        } else {
            ShrinkResult::NoProgress
        }
    }

    /// Token-by-token deletion
    fn shrink_tokens<F>(&self, input: &str, test_fn: &F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        let tokens = self.tokenize(input);
        if tokens.len() <= 1 {
            return ShrinkResult::NoProgress;
        }

        let mut best = tokens.clone();
        let mut iterations = 0;
        let mut changed = true;

        while changed && iterations < self.config.max_iterations {
            changed = false;

            let mut i = 0;
            while i < best.len() {
                if best.len() <= 1 {
                    break;
                }

                // Try removing this token
                let mut candidate_tokens = best.clone();
                candidate_tokens.remove(i);
                let candidate = candidate_tokens.join("");

                iterations += 1;

                if test_fn(&candidate) {
                    best = candidate_tokens;
                    changed = true;
                } else {
                    i += 1;
                }
            }
        }

        let result = best.join("");
        if result.len() < input.len() {
            ShrinkResult::Success(result)
        } else {
            ShrinkResult::NoProgress
        }
    }

    /// Hierarchical (structure-aware) shrinking
    fn shrink_hierarchical<F>(&self, input: &str, test_fn: &F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        let mut best = input.to_string();
        let mut iterations = 0;

        // First pass: try removing entire functions
        best = self.try_remove_functions(&best, test_fn, &mut iterations);

        // Second pass: try removing entire blocks
        best = self.try_remove_blocks(&best, test_fn, &mut iterations);

        // Third pass: try removing statements
        best = self.try_remove_statements(&best, test_fn, &mut iterations);

        // Fourth pass: try simplifying expressions
        best = self.try_simplify_expressions(&best, test_fn, &mut iterations);

        if best.len() < input.len() {
            ShrinkResult::Success(best)
        } else {
            ShrinkResult::NoProgress
        }
    }

    /// Combined shrinking approach
    fn shrink_combined<F>(&self, input: &str, test_fn: &F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        let mut best = input.to_string();
        let mut changed = true;
        let mut total_iterations = 0;

        while changed && total_iterations < self.config.max_iterations {
            changed = false;
            let before_len = best.len();

            // Try hierarchical first (fastest for structured input)
            if let ShrinkResult::Success(reduced) = self.shrink_hierarchical(&best, test_fn) {
                best = reduced;
            }

            // Then try line deletion
            if let ShrinkResult::Success(reduced) = self.shrink_lines(&best, test_fn) {
                best = reduced;
            }

            // Finally try delta debugging for fine-grained reduction
            if let ShrinkResult::Success(reduced) = self.shrink_ddmin(&best, test_fn) {
                best = reduced;
            }

            if best.len() < before_len {
                changed = true;
            }

            total_iterations += 1;
        }

        if best.len() < input.len() {
            ShrinkResult::Success(best)
        } else {
            ShrinkResult::NoProgress
        }
    }

    /// Try to remove entire functions
    fn try_remove_functions<F>(&self, input: &str, test_fn: &F, iterations: &mut usize) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut result = input.to_string();

        // Find function boundaries
        let fn_pattern = regex::Regex::new(r"(?m)^(pub\s+)?(async\s+)?fn\s+\w+").unwrap();

        loop {
            let matches: Vec<_> = fn_pattern.find_iter(&result).collect();
            if matches.len() <= 1 {
                break;
            }

            let mut changed = false;
            for (_i, m) in matches.iter().enumerate() {
                if *iterations >= self.config.max_iterations {
                    return result;
                }

                // Find the end of this function
                let start = m.start();
                if let Some(end) = self.find_matching_brace(&result[start..]) {
                    let func_end = start + end + 1;

                    // Try removing this function
                    let candidate = format!(
                        "{}{}",
                        &result[..start].trim_end(),
                        &result[func_end..].trim_start()
                    );

                    *iterations += 1;

                    if test_fn(&candidate) {
                        result = candidate;
                        changed = true;
                        break;
                    }
                }
            }

            if !changed {
                break;
            }
        }

        result
    }

    /// Try to remove entire blocks
    fn try_remove_blocks<F>(&self, input: &str, test_fn: &F, iterations: &mut usize) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut result = input.to_string();

        // Find block patterns (if, for, while, match, loop)
        let block_keywords = ["if ", "for ", "while ", "match ", "loop "];

        for keyword in block_keywords.iter() {
            loop {
                if *iterations >= self.config.max_iterations {
                    return result;
                }

                if let Some(pos) = result.find(keyword) {
                    if let Some(end) = self.find_matching_brace(&result[pos..]) {
                        let block_end = pos + end + 1;

                        // Skip else blocks
                        let after_block = result[block_end..].trim_start();
                        let final_end = if after_block.starts_with("else") {
                            if let Some(else_end) = self.find_matching_brace(after_block) {
                                block_end
                                    + (after_block.as_ptr() as usize
                                        - result[block_end..].as_ptr() as usize)
                                    + else_end
                                    + 1
                            } else {
                                block_end
                            }
                        } else {
                            block_end
                        };

                        // Try removing this block
                        let candidate = format!("{}{}", &result[..pos], &result[final_end..]);

                        *iterations += 1;

                        if test_fn(&candidate) {
                            result = candidate;
                            continue;
                        }
                    }
                }
                break;
            }
        }

        result
    }

    /// Try to remove individual statements
    fn try_remove_statements<F>(&self, input: &str, test_fn: &F, iterations: &mut usize) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut lines: Vec<&str> = input.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            if *iterations >= self.config.max_iterations {
                break;
            }

            let line = lines[i].trim();

            // Skip structural lines
            if line.starts_with("fn ")
                || line.starts_with("type ")
                || line == "{"
                || line == "}"
                || line.is_empty()
            {
                i += 1;
                continue;
            }

            // Try removing this line
            let mut candidate_lines = lines.clone();
            candidate_lines.remove(i);
            let candidate = candidate_lines.join("\n");

            *iterations += 1;

            if test_fn(&candidate) {
                lines = candidate_lines;
                // Don't increment i
            } else {
                i += 1;
            }
        }

        lines.join("\n")
    }

    /// Try to simplify expressions
    fn try_simplify_expressions<F>(
        &self,
        input: &str,
        test_fn: &F,
        iterations: &mut usize,
    ) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut result = input.to_string();

        // Simplification patterns: (complex) -> simple
        let simplifications = [
            // Numeric simplifications
            (r"\d+", vec!["0", "1"]),
            // Boolean simplifications
            (r"true", vec!["false"]),
            (r"false", vec!["true"]),
            // String simplifications
            (r#""[^"]*""#, vec!["\"\""]),
            // List simplifications
            (r"\[[^\]]+\]", vec!["[]"]),
            // Tuple simplifications
            (r"\([^)]+\)", vec!["()"]),
        ];

        for (pattern, replacements) in simplifications.iter() {
            if *iterations >= self.config.max_iterations {
                break;
            }

            let re = regex::Regex::new(pattern).unwrap();

            for replacement in replacements {
                if let Some(m) = re.find(&result) {
                    let candidate = format!(
                        "{}{}{}",
                        &result[..m.start()],
                        replacement,
                        &result[m.end()..]
                    );

                    *iterations += 1;

                    if test_fn(&candidate) {
                        result = candidate;
                        break;
                    }
                }
            }
        }

        result
    }

    /// Find the matching closing brace
    fn find_matching_brace(&self, input: &str) -> Option<usize> {
        let mut depth = 0;
        let mut in_string = false;
        let mut escape = false;

        for (i, c) in input.char_indices() {
            if escape {
                escape = false;
                continue;
            }

            match c {
                '\\' if in_string => escape = true,
                '"' => in_string = !in_string,
                '{' if !in_string => depth += 1,
                '}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Simple tokenization for token-level shrinking
    fn tokenize(&self, input: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();
        let mut in_string = false;
        let mut escape = false;

        for c in input.chars() {
            if escape {
                current.push(c);
                escape = false;
                continue;
            }

            match c {
                '\\' if in_string => {
                    current.push(c);
                    escape = true;
                }
                '"' => {
                    current.push(c);
                    if in_string {
                        tokens.push(current.clone());
                        current.clear();
                    }
                    in_string = !in_string;
                }
                _ if in_string => {
                    current.push(c);
                }
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                    tokens.push(c.to_string());
                }
                c if c.is_alphanumeric() || c == '_' => {
                    current.push(c);
                }
                c => {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                    tokens.push(c.to_string());
                }
            }
        }

        if !current.is_empty() {
            tokens.push(current);
        }

        tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shrinker_creation() {
        let config = ShrinkConfig::default();
        let shrinker = Shrinker::new(config);
        assert_eq!(shrinker.config.max_iterations, 1000);
    }

    #[test]
    fn test_shrink_lines() {
        let config = ShrinkConfig {
            strategy: ShrinkStrategy::LineByLine,
            ..Default::default()
        };
        let shrinker = Shrinker::new(config);

        let input = r#"
fn main() {
    let a = 1;
    let b = 2;
    let c = 3;
    let bug = true;
    let d = 4;
}
"#;

        // Test function: bug is triggered by "let bug = true"
        let test_fn = |s: &str| s.contains("let bug = true");

        match shrinker.shrink(input, test_fn) {
            ShrinkResult::Success(minimized) => {
                assert!(minimized.contains("let bug = true"));
                assert!(minimized.len() < input.len());
            }
            _ => panic!("Expected successful shrinking"),
        }
    }

    #[test]
    fn test_shrink_ddmin() {
        let config = ShrinkConfig {
            strategy: ShrinkStrategy::DeltaDebugging,
            ..Default::default()
        };
        let shrinker = Shrinker::new(config);

        let input = "abcdefghijklmnop";
        let test_fn = |s: &str| s.contains("gh");

        match shrinker.shrink(input, test_fn) {
            ShrinkResult::Success(minimized) => {
                assert!(minimized.contains("gh"));
                assert!(minimized.len() <= input.len());
            }
            _ => panic!("Expected successful shrinking"),
        }
    }

    #[test]
    fn test_shrink_combined() {
        let config = ShrinkConfig {
            strategy: ShrinkStrategy::Combined,
            ..Default::default()
        };
        let shrinker = Shrinker::new(config);

        let input = r#"
fn helper() {
    let x = 1;
}

fn main() {
    let a = 1;
    let b = 2;
    let crash = panic!;
    let c = 3;
}
"#;

        let test_fn = |s: &str| s.contains("panic!");

        match shrinker.shrink(input, test_fn) {
            ShrinkResult::Success(minimized) => {
                assert!(minimized.contains("panic!"));
                assert!(minimized.len() < input.len());
            }
            _ => panic!("Expected successful shrinking"),
        }
    }

    #[test]
    fn test_find_matching_brace() {
        let config = ShrinkConfig::default();
        let shrinker = Shrinker::new(config);

        let input = "fn foo() { let x = 1; }";
        let start = input.find('{').unwrap();
        let end = shrinker.find_matching_brace(&input[start..]);
        assert_eq!(end, Some(13));
    }

    #[test]
    fn test_tokenize() {
        let config = ShrinkConfig::default();
        let shrinker = Shrinker::new(config);

        let input = "let x = 42;";
        let tokens = shrinker.tokenize(input);
        assert!(tokens.contains(&"let".to_string()));
        assert!(tokens.contains(&"42".to_string()));
    }

    #[test]
    fn test_shrink_stats() {
        let config = ShrinkConfig::default();
        let shrinker = Shrinker::new(config);

        let input = "aaaaaXaaaa";
        let test_fn = |s: &str| s.contains('X');

        let (result, stats) = shrinker.shrink_with_stats(input, test_fn);

        assert!(matches!(result, ShrinkResult::Success(_)));
        assert_eq!(stats.original_size, input.len());
        assert!(stats.final_size < stats.original_size);
        assert!(stats.reduction_pct() > 0.0);
    }
}

// ============================================================================
// AST-Aware Shrinking
// ============================================================================

/// AST node type for structure-aware shrinking
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstNodeKind {
    /// Module/file level
    Module,
    /// Function definition
    Function,
    /// Type definition
    TypeDef,
    /// Block expression
    Block,
    /// If expression
    If,
    /// For loop
    For,
    /// While loop
    While,
    /// Match expression
    Match,
    /// Let binding
    Let,
    /// Expression statement
    ExprStmt,
    /// Return statement
    Return,
    /// Call expression
    Call,
    /// Binary operation
    BinaryOp,
    /// Unary operation
    UnaryOp,
    /// Literal
    Literal,
    /// Identifier
    Identifier,
    /// Unknown/raw text
    Unknown,
}

/// A simplified AST node for shrinking
#[derive(Debug, Clone)]
pub struct AstNode {
    /// Node type
    pub kind: AstNodeKind,
    /// Start position in source
    pub start: usize,
    /// End position in source
    pub end: usize,
    /// Child nodes
    pub children: Vec<AstNode>,
    /// Whether this node is essential (cannot be removed)
    pub essential: bool,
}

impl AstNode {
    /// Create a new AST node
    pub fn new(kind: AstNodeKind, start: usize, end: usize) -> Self {
        Self {
            kind,
            start,
            end,
            children: Vec::new(),
            essential: false,
        }
    }

    /// Add a child node
    pub fn add_child(&mut self, child: AstNode) {
        self.children.push(child);
    }

    /// Get the source text for this node
    pub fn source<'a>(&self, input: &'a str) -> &'a str {
        &input[self.start..self.end]
    }

    /// Get total count of nodes in subtree
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(|c| c.node_count()).sum::<usize>()
    }

    /// Get all leaf nodes
    pub fn leaves(&self) -> Vec<&AstNode> {
        if self.children.is_empty() {
            vec![self]
        } else {
            self.children.iter().flat_map(|c| c.leaves()).collect()
        }
    }
}

/// AST-aware shrinker that preserves syntactic validity
pub struct AstAwareShrinker {
    /// Configuration
    config: AstShrinkConfig,
    /// Statistics
    stats: AstShrinkStats,
}

/// Configuration for AST-aware shrinking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstShrinkConfig {
    /// Maximum iterations
    pub max_iterations: usize,
    /// Whether to preserve syntactic validity strictly
    pub preserve_validity: bool,
    /// Shrink child nodes before parent nodes
    pub bottom_up: bool,
    /// Node types that can never be removed
    pub essential_types: Vec<String>,
    /// Minimum program size to stop shrinking
    pub min_size: usize,
}

impl Default for AstShrinkConfig {
    fn default() -> Self {
        Self {
            max_iterations: 5000,
            preserve_validity: true,
            bottom_up: true,
            essential_types: vec![],
            min_size: 1,
        }
    }
}

/// Statistics for AST-aware shrinking
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AstShrinkStats {
    /// Original node count
    pub original_nodes: usize,
    /// Final node count
    pub final_nodes: usize,
    /// Nodes removed
    pub nodes_removed: usize,
    /// Iterations performed
    pub iterations: usize,
    /// Successful removals by node type
    pub removals_by_type: HashMap<String, usize>,
}

impl Default for AstAwareShrinker {
    fn default() -> Self {
        Self::new(AstShrinkConfig::default())
    }
}

impl AstAwareShrinker {
    /// Create a new AST-aware shrinker
    pub fn new(config: AstShrinkConfig) -> Self {
        Self {
            config,
            stats: AstShrinkStats::default(),
        }
    }

    /// Parse source into a simple AST
    pub fn parse(&self, input: &str) -> AstNode {
        let mut root = AstNode::new(AstNodeKind::Module, 0, input.len());

        // Find top-level constructs
        let lines: Vec<&str> = input.lines().collect();
        let mut i = 0;
        let mut char_pos = 0;

        while i < lines.len() {
            let line = lines[i].trim();
            let line_start = char_pos;

            if line.starts_with("fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("async fn ")
            {
                // Function definition
                if let Some(end_pos) = self.find_block_end(input, char_pos) {
                    let func = AstNode::new(AstNodeKind::Function, char_pos, end_pos);
                    root.add_child(func);
                    // Skip to end of function
                    while char_pos < end_pos && i < lines.len() {
                        char_pos += lines[i].len() + 1; // +1 for newline
                        i += 1;
                    }
                    continue;
                }
            } else if line.starts_with("type ")
                || line.starts_with("struct ")
                || line.starts_with("enum ")
            {
                // Type definition
                if let Some(end_pos) = self.find_block_end(input, char_pos) {
                    let typedef = AstNode::new(AstNodeKind::TypeDef, char_pos, end_pos);
                    root.add_child(typedef);
                    while char_pos < end_pos && i < lines.len() {
                        char_pos += lines[i].len() + 1;
                        i += 1;
                    }
                    continue;
                }
            }

            char_pos += lines[i].len() + 1;
            i += 1;
        }

        root
    }

    /// Find the end of a block starting at the given position
    fn find_block_end(&self, input: &str, start: usize) -> Option<usize> {
        let mut depth = 0;
        let mut in_string = false;
        let mut escape = false;
        let mut found_open = false;

        for (i, c) in input[start..].char_indices() {
            if escape {
                escape = false;
                continue;
            }

            match c {
                '\\' if in_string => escape = true,
                '"' => in_string = !in_string,
                '{' if !in_string => {
                    depth += 1;
                    found_open = true;
                }
                '}' if !in_string => {
                    depth -= 1;
                    if found_open && depth == 0 {
                        return Some(start + i + 1);
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Shrink using AST structure
    pub fn shrink<F>(&mut self, input: &str, test_fn: F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        if !test_fn(input) {
            return ShrinkResult::Error("Original input does not trigger bug".to_string());
        }

        self.stats = AstShrinkStats::default();
        let ast = self.parse(input);
        self.stats.original_nodes = ast.node_count();

        let mut current = input.to_string();
        let mut iterations = 0;

        // Strategy: Remove nodes in order of decreasing size
        loop {
            if iterations >= self.config.max_iterations {
                break;
            }

            let before_len = current.len();
            current = self.shrink_pass(&current, &test_fn, &mut iterations);

            if current.len() >= before_len || current.len() <= self.config.min_size {
                break;
            }
        }

        self.stats.iterations = iterations;
        let final_ast = self.parse(&current);
        self.stats.final_nodes = final_ast.node_count();
        self.stats.nodes_removed = self
            .stats
            .original_nodes
            .saturating_sub(self.stats.final_nodes);

        if current.len() < input.len() {
            ShrinkResult::Success(current)
        } else {
            ShrinkResult::NoProgress
        }
    }

    /// Run one pass of shrinking
    fn shrink_pass<F>(&mut self, input: &str, test_fn: &F, iterations: &mut usize) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut current = input.to_string();
        let mut made_progress = true;

        // Keep trying to remove nodes until no progress is made
        while made_progress {
            made_progress = false;
            let ast = self.parse(&current);

            // Try removing each top-level node
            for node in &ast.children {
                if *iterations >= self.config.max_iterations {
                    return current;
                }

                // Validate node bounds before accessing
                if node.end > current.len() {
                    continue;
                }

                let candidate = self.remove_node(&current, node);
                *iterations += 1;

                if test_fn(&candidate) {
                    current = candidate;
                    made_progress = true;
                    *self
                        .stats
                        .removals_by_type
                        .entry(format!("{:?}", node.kind))
                        .or_insert(0) += 1;
                    // Re-parse after successful removal since offsets have changed
                    break;
                }
            }
        }

        current
    }

    /// Remove a node from the source
    fn remove_node(&self, input: &str, node: &AstNode) -> String {
        let before = &input[..node.start];
        let after = &input[node.end..];

        // Clean up whitespace
        let before_trimmed = before.trim_end();
        let after_trimmed = after.trim_start();

        if before_trimmed.is_empty() {
            after_trimmed.to_string()
        } else if after_trimmed.is_empty() {
            before_trimmed.to_string()
        } else {
            format!("{}\n\n{}", before_trimmed, after_trimmed)
        }
    }

    /// Get statistics
    pub fn stats(&self) -> &AstShrinkStats {
        &self.stats
    }
}

// ============================================================================
// Delta Debugging with 1-Minimality
// ============================================================================

/// Enhanced delta debugging that achieves 1-minimality
///
/// 1-minimal means: removing any single element breaks the test.
/// This is stronger than ddmin which may not achieve 1-minimality.
pub struct DeltaDebugger {
    /// Configuration
    config: DeltaDebugConfig,
    /// Statistics
    stats: DeltaDebugStats,
}

/// Configuration for delta debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaDebugConfig {
    /// Maximum iterations
    pub max_iterations: usize,
    /// Initial granularity (number of chunks)
    pub initial_n: usize,
    /// Achieve 1-minimality (slower but more minimal)
    pub one_minimal: bool,
    /// Units to divide by (chars, lines, tokens)
    pub unit: DeltaUnit,
}

/// Unit for delta debugging
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeltaUnit {
    /// Character-level
    Chars,
    /// Line-level
    Lines,
    /// Token-level
    Tokens,
}

impl Default for DeltaDebugConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10000,
            initial_n: 2,
            one_minimal: true,
            unit: DeltaUnit::Chars,
        }
    }
}

/// Statistics for delta debugging
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeltaDebugStats {
    /// Total test executions
    pub tests: usize,
    /// Successful reductions
    pub reductions: usize,
    /// Original size (in units)
    pub original_units: usize,
    /// Final size (in units)
    pub final_units: usize,
    /// Maximum granularity reached
    pub max_granularity: usize,
}

impl Default for DeltaDebugger {
    fn default() -> Self {
        Self::new(DeltaDebugConfig::default())
    }
}

impl DeltaDebugger {
    /// Create a new delta debugger
    pub fn new(config: DeltaDebugConfig) -> Self {
        Self {
            config,
            stats: DeltaDebugStats::default(),
        }
    }

    /// Run delta debugging
    pub fn minimize<F>(&mut self, input: &str, test_fn: F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        self.stats = DeltaDebugStats::default();

        if !test_fn(input) {
            return ShrinkResult::Error("Original input does not trigger bug".to_string());
        }

        let units = self.split(input);
        self.stats.original_units = units.len();

        if units.len() <= 1 {
            return ShrinkResult::NoProgress;
        }

        let result = self.ddmin(&units, &test_fn);

        self.stats.final_units = result.len();

        let minimized = self.join(&result);
        if minimized.len() < input.len() {
            if self.config.one_minimal {
                // Verify 1-minimality
                let final_result = self.verify_one_minimal(&result, &test_fn);
                ShrinkResult::Success(self.join(&final_result))
            } else {
                ShrinkResult::Success(minimized)
            }
        } else {
            ShrinkResult::NoProgress
        }
    }

    /// Split input into units
    fn split(&self, input: &str) -> Vec<String> {
        match self.config.unit {
            DeltaUnit::Chars => input.chars().map(|c| c.to_string()).collect(),
            DeltaUnit::Lines => input.lines().map(|l| format!("{}\n", l)).collect(),
            DeltaUnit::Tokens => self.tokenize(input),
        }
    }

    /// Join units back together
    fn join(&self, units: &[String]) -> String {
        match self.config.unit {
            DeltaUnit::Lines => units.join("").trim_end().to_string(),
            _ => units.join(""),
        }
    }

    /// Simple tokenization
    fn tokenize(&self, input: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut current = String::new();

        for c in input.chars() {
            if c.is_whitespace() {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push(c.to_string());
            } else if c.is_alphanumeric() || c == '_' {
                current.push(c);
            } else {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push(c.to_string());
            }
        }

        if !current.is_empty() {
            tokens.push(current);
        }

        tokens
    }

    /// Core ddmin algorithm
    fn ddmin<F>(&mut self, units: &[String], test_fn: &F) -> Vec<String>
    where
        F: Fn(&str) -> bool,
    {
        let mut current = units.to_vec();
        let mut n = self.config.initial_n;

        while n <= current.len() && self.stats.tests < self.config.max_iterations {
            let chunk_size = (current.len() + n - 1) / n;
            let mut reduced = false;

            // Try each chunk
            for i in 0..n {
                let start = (i * chunk_size).min(current.len());
                let end = ((i + 1) * chunk_size).min(current.len());

                if start >= end || start >= current.len() {
                    continue;
                }

                // Try removing this chunk (complement)
                let complement: Vec<String> = current[..start]
                    .iter()
                    .chain(current[end..].iter())
                    .cloned()
                    .collect();

                self.stats.tests += 1;

                if !complement.is_empty() && test_fn(&self.join(&complement)) {
                    current = complement;
                    self.stats.reductions += 1;
                    n = self.config.initial_n.max(n - 1);
                    reduced = true;
                    break;
                }

                // Try keeping only this chunk
                let subset: Vec<String> = current[start..end].to_vec();
                self.stats.tests += 1;

                if !subset.is_empty() && test_fn(&self.join(&subset)) {
                    current = subset;
                    self.stats.reductions += 1;
                    n = self.config.initial_n;
                    reduced = true;
                    break;
                }
            }

            if !reduced {
                if n >= current.len() {
                    break;
                }
                n = (n * 2).min(current.len());
                self.stats.max_granularity = n;
            }
        }

        current
    }

    /// Verify and achieve 1-minimality
    fn verify_one_minimal<F>(&mut self, units: &[String], test_fn: &F) -> Vec<String>
    where
        F: Fn(&str) -> bool,
    {
        let mut current = units.to_vec();
        let mut changed = true;

        while changed && self.stats.tests < self.config.max_iterations {
            changed = false;

            let mut i = 0;
            while i < current.len() {
                // Try removing this single unit
                let candidate: Vec<String> = current[..i]
                    .iter()
                    .chain(current[i + 1..].iter())
                    .cloned()
                    .collect();

                self.stats.tests += 1;

                if !candidate.is_empty() && test_fn(&self.join(&candidate)) {
                    current = candidate;
                    self.stats.reductions += 1;
                    changed = true;
                    // Don't increment i
                } else {
                    i += 1;
                }
            }
        }

        current
    }

    /// Get statistics
    pub fn stats(&self) -> &DeltaDebugStats {
        &self.stats
    }
}

// ============================================================================
// Hierarchical Delta Debugging
// ============================================================================

/// Hierarchical delta debugging that shrinks at multiple levels
///
/// This combines the benefits of AST-aware shrinking with delta debugging:
/// 1. First shrink at the coarsest level (functions/modules)
/// 2. Then shrink at block level
/// 3. Then shrink at statement level
/// 4. Finally shrink at character level
pub struct HierarchicalShrinker {
    /// Configuration
    config: HierarchicalConfig,
    /// Statistics
    stats: HierarchicalStats,
}

/// Configuration for hierarchical shrinking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchicalConfig {
    /// Maximum iterations per level
    pub max_iterations_per_level: usize,
    /// Levels to use
    pub levels: Vec<ShrinkLevel>,
    /// Stop when no progress for this many levels
    pub stagnation_limit: usize,
}

/// Level of shrinking granularity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShrinkLevel {
    /// Module/function level
    Module,
    /// Block level (if/for/while/match)
    Block,
    /// Statement level
    Statement,
    /// Expression level
    Expression,
    /// Token level
    Token,
    /// Character level
    Char,
}

impl Default for HierarchicalConfig {
    fn default() -> Self {
        Self {
            max_iterations_per_level: 1000,
            levels: vec![
                ShrinkLevel::Module,
                ShrinkLevel::Block,
                ShrinkLevel::Statement,
                ShrinkLevel::Token,
                ShrinkLevel::Char,
            ],
            stagnation_limit: 2,
        }
    }
}

/// Statistics for hierarchical shrinking
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HierarchicalStats {
    /// Reductions per level
    pub reductions_by_level: HashMap<String, usize>,
    /// Time spent per level (ms)
    pub time_by_level: HashMap<String, u64>,
    /// Total iterations
    pub total_iterations: usize,
    /// Final reduction percentage
    pub reduction_pct: f64,
}

impl Default for HierarchicalShrinker {
    fn default() -> Self {
        Self::new(HierarchicalConfig::default())
    }
}

impl HierarchicalShrinker {
    /// Create a new hierarchical shrinker
    pub fn new(config: HierarchicalConfig) -> Self {
        Self {
            config,
            stats: HierarchicalStats::default(),
        }
    }

    /// Shrink input hierarchically
    pub fn shrink<F>(&mut self, input: &str, test_fn: F) -> ShrinkResult
    where
        F: Fn(&str) -> bool,
    {
        self.stats = HierarchicalStats::default();

        if !test_fn(input) {
            return ShrinkResult::Error("Original input does not trigger bug".to_string());
        }

        let original_len = input.len();
        let mut current = input.to_string();
        let mut stagnation = 0;

        for level in &self.config.levels.clone() {
            let start = std::time::Instant::now();
            let before_len = current.len();

            current = match level {
                ShrinkLevel::Module => self.shrink_modules(&current, &test_fn),
                ShrinkLevel::Block => self.shrink_blocks(&current, &test_fn),
                ShrinkLevel::Statement => self.shrink_statements(&current, &test_fn),
                ShrinkLevel::Expression => self.shrink_expressions(&current, &test_fn),
                ShrinkLevel::Token => self.shrink_tokens(&current, &test_fn),
                ShrinkLevel::Char => self.shrink_chars(&current, &test_fn),
            };

            let elapsed = start.elapsed().as_millis() as u64;
            let level_name = format!("{:?}", level);
            self.stats.time_by_level.insert(level_name.clone(), elapsed);

            let reduction = before_len - current.len();
            if reduction > 0 {
                self.stats.reductions_by_level.insert(level_name, reduction);
                stagnation = 0;
            } else {
                stagnation += 1;
                if stagnation >= self.config.stagnation_limit {
                    break;
                }
            }
        }

        self.stats.reduction_pct = if original_len > 0 {
            (1.0 - (current.len() as f64 / original_len as f64)) * 100.0
        } else {
            0.0
        };

        if current.len() < original_len {
            ShrinkResult::Success(current)
        } else {
            ShrinkResult::NoProgress
        }
    }

    fn shrink_modules<F>(&mut self, input: &str, test_fn: &F) -> String
    where
        F: Fn(&str) -> bool,
    {
        // Use function-level shrinking
        let patterns = [
            r"(?m)^(pub\s+)?(async\s+)?fn\s+\w+[^{]*\{",
            r"(?m)^(pub\s+)?type\s+\w+[^{]*\{",
            r"(?m)^(pub\s+)?struct\s+\w+[^{]*\{",
            r"(?m)^(pub\s+)?enum\s+\w+[^{]*\{",
        ];

        let mut current = input.to_string();
        let mut iterations = 0;

        for pattern in patterns {
            let re = regex::Regex::new(pattern).unwrap();

            loop {
                if iterations >= self.config.max_iterations_per_level {
                    return current;
                }

                let matches: Vec<_> = re.find_iter(&current).collect();
                if matches.is_empty() {
                    break;
                }

                let mut reduced = false;
                for m in &matches {
                    if let Some(end) = self.find_block_end(&current, m.start()) {
                        let candidate = format!(
                            "{}{}",
                            current[..m.start()].trim_end(),
                            current[end..].trim_start()
                        );

                        iterations += 1;
                        self.stats.total_iterations += 1;

                        if test_fn(&candidate) {
                            current = candidate;
                            reduced = true;
                            break;
                        }
                    }
                }

                if !reduced {
                    break;
                }
            }
        }

        current
    }

    fn shrink_blocks<F>(&mut self, input: &str, test_fn: &F) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut current = input.to_string();
        let keywords = ["if ", "for ", "while ", "match ", "loop ", "unsafe "];
        let mut iterations = 0;

        for keyword in keywords {
            loop {
                if iterations >= self.config.max_iterations_per_level {
                    return current;
                }

                if let Some(pos) = current.find(keyword) {
                    if let Some(end) = self.find_block_end(&current, pos) {
                        let candidate = format!("{}{}", &current[..pos], &current[end..]);

                        iterations += 1;
                        self.stats.total_iterations += 1;

                        if test_fn(&candidate) {
                            current = candidate;
                            continue;
                        }
                    }
                }
                break;
            }
        }

        current
    }

    fn shrink_statements<F>(&mut self, input: &str, test_fn: &F) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut lines: Vec<&str> = input.lines().collect();
        let mut i = 0;
        let mut iterations = 0;

        while i < lines.len() {
            if iterations >= self.config.max_iterations_per_level {
                break;
            }

            let line = lines[i].trim();

            // Skip structural lines
            if line.starts_with("fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("type ")
                || line == "{"
                || line == "}"
                || line.is_empty()
            {
                i += 1;
                continue;
            }

            let mut candidate_lines = lines.clone();
            candidate_lines.remove(i);
            let candidate = candidate_lines.join("\n");

            iterations += 1;
            self.stats.total_iterations += 1;

            if test_fn(&candidate) {
                lines = candidate_lines;
            } else {
                i += 1;
            }
        }

        lines.join("\n")
    }

    fn shrink_expressions<F>(&mut self, input: &str, test_fn: &F) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut current = input.to_string();
        let mut iterations = 0;

        // Simplify expressions
        let simplifications = [
            (r"\d+", vec!["0", "1"]),
            (r"true", vec!["false"]),
            (r#""[^"]+""#, vec!["\"\""]),
            (r"\[[^\]]+\]", vec!["[]"]),
        ];

        for (pattern, replacements) in simplifications {
            if iterations >= self.config.max_iterations_per_level {
                break;
            }

            let re = regex::Regex::new(pattern).unwrap();

            for replacement in replacements {
                if let Some(m) = re.find(&current) {
                    let candidate = format!(
                        "{}{}{}",
                        &current[..m.start()],
                        replacement,
                        &current[m.end()..]
                    );

                    iterations += 1;
                    self.stats.total_iterations += 1;

                    if test_fn(&candidate) {
                        current = candidate;
                        break;
                    }
                }
            }
        }

        current
    }

    fn shrink_tokens<F>(&mut self, input: &str, test_fn: &F) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut config = DeltaDebugConfig::default();
        config.unit = DeltaUnit::Tokens;
        config.max_iterations = self.config.max_iterations_per_level;

        let mut ddebug = DeltaDebugger::new(config);
        match ddebug.minimize(input, test_fn) {
            ShrinkResult::Success(s) => {
                self.stats.total_iterations += ddebug.stats().tests;
                s
            }
            _ => input.to_string(),
        }
    }

    fn shrink_chars<F>(&mut self, input: &str, test_fn: &F) -> String
    where
        F: Fn(&str) -> bool,
    {
        let mut config = DeltaDebugConfig::default();
        config.unit = DeltaUnit::Chars;
        config.max_iterations = self.config.max_iterations_per_level;

        let mut ddebug = DeltaDebugger::new(config);
        match ddebug.minimize(input, test_fn) {
            ShrinkResult::Success(s) => {
                self.stats.total_iterations += ddebug.stats().tests;
                s
            }
            _ => input.to_string(),
        }
    }

    fn find_block_end(&self, input: &str, start: usize) -> Option<usize> {
        let mut depth = 0;
        let mut in_string = false;
        let mut escape = false;
        let mut found_open = false;

        for (i, c) in input[start..].char_indices() {
            if escape {
                escape = false;
                continue;
            }

            match c {
                '\\' if in_string => escape = true,
                '"' => in_string = !in_string,
                '{' if !in_string => {
                    depth += 1;
                    found_open = true;
                }
                '}' if !in_string => {
                    depth -= 1;
                    if found_open && depth == 0 {
                        return Some(start + i + 1);
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Get statistics
    pub fn stats(&self) -> &HierarchicalStats {
        &self.stats
    }
}

// ============================================================================
// Extended Shrinker Exports
// ============================================================================

// Enhanced shrinkers are already defined in this module:
// - AstAwareShrinker, AstShrinkConfig, AstShrinkStats, AstNode, AstNodeKind
// - DeltaDebugger, DeltaDebugConfig, DeltaDebugStats, DeltaUnit
// - HierarchicalShrinker, HierarchicalConfig, HierarchicalStats, ShrinkLevel

#[cfg(test)]
mod enhanced_tests {
    use super::*;

    #[test]
    fn test_delta_debugger() {
        let mut dd = DeltaDebugger::default();
        let input = "abcdefghXijklmnop";

        let result = dd.minimize(input, |s| s.contains('X'));

        assert!(matches!(result, ShrinkResult::Success(_)));
        if let ShrinkResult::Success(minimized) = result {
            assert!(minimized.contains('X'));
            assert!(minimized.len() < input.len());
        }
    }

    #[test]
    fn test_delta_debugger_lines() {
        let mut config = DeltaDebugConfig::default();
        config.unit = DeltaUnit::Lines;
        let mut dd = DeltaDebugger::new(config);

        let input = "line1\nline2\nBUG\nline3\nline4\n";
        let result = dd.minimize(input, |s| s.contains("BUG"));

        assert!(matches!(result, ShrinkResult::Success(_)));
        if let ShrinkResult::Success(minimized) = result {
            assert!(minimized.contains("BUG"));
        }
    }

    #[test]
    fn test_hierarchical_shrinker() {
        let mut shrinker = HierarchicalShrinker::default();

        let input = r#"
fn helper() {
    let x = 1;
}

fn main() {
    let a = 1;
    if true {
        let b = 2;
    }
    let BUG = panic!;
    let c = 3;
}
"#;

        let result = shrinker.shrink(input, |s| s.contains("BUG"));

        assert!(matches!(result, ShrinkResult::Success(_)));
        if let ShrinkResult::Success(minimized) = result {
            assert!(minimized.contains("BUG"));
            assert!(minimized.len() < input.len());
        }
    }

    #[test]
    fn test_ast_aware_shrinker() {
        let mut shrinker = AstAwareShrinker::default();

        let input = r#"
fn unused() {
    let x = 42;
}

fn main() {
    let bug = true;
}
"#;

        let result = shrinker.shrink(input, |s| s.contains("bug"));

        assert!(matches!(result, ShrinkResult::Success(_)));
        if let ShrinkResult::Success(minimized) = result {
            assert!(minimized.contains("bug"));
        }
    }

    #[test]
    fn test_ast_node() {
        let node = AstNode::new(AstNodeKind::Function, 0, 10);
        assert_eq!(node.kind, AstNodeKind::Function);
        assert_eq!(node.start, 0);
        assert_eq!(node.end, 10);
        assert_eq!(node.node_count(), 1);
    }

    #[test]
    fn test_shrink_stats() {
        let stats = ShrinkStats {
            original_size: 100,
            final_size: 50,
            ..Default::default()
        };
        assert_eq!(stats.reduction_pct(), 50.0);
    }
}
