//! Parser fuzzing harness
//!
//! This module provides a fuzzing harness for the Verum parser.
//! It tests the parser's robustness against malformed, edge-case,
//! and valid input to find crashes, hangs, and incorrect parsing.
//!
//! # Test Strategies
//!
//! - **Grammar coverage**: Ensure all grammar productions are tested
//! - **Error recovery**: Test parser's ability to recover from errors
//! - **Edge cases**: Deeply nested structures, long chains, etc.
//! - **Boundary values**: Maximum nesting, identifier lengths, etc.
//!
//! # Safety Properties
//!
//! The parser should:
//! - Never panic on any input
//! - Always terminate in bounded time
//! - Produce valid AST for valid input
//! - Provide helpful error messages for invalid input

use std::time::{Duration, Instant};
use std::collections::HashMap;

/// Result of parsing a source
#[derive(Debug, Clone)]
pub struct ParserResult {
    /// Whether parsing succeeded
    pub success: bool,
    /// Number of AST nodes produced
    pub node_count: usize,
    /// Maximum nesting depth observed
    pub max_depth: usize,
    /// Errors encountered
    pub errors: Vec<ParserError>,
    /// Warnings produced
    pub warnings: Vec<ParserWarning>,
    /// Time taken to parse
    pub duration: Duration,
    /// AST node type histogram
    pub node_histogram: HashMap<String, usize>,
}

/// A parser error
#[derive(Debug, Clone)]
pub struct ParserError {
    /// Error message
    pub message: String,
    /// Span start (byte offset)
    pub span_start: usize,
    /// Span end (byte offset)
    pub span_end: usize,
    /// Expected tokens
    pub expected: Vec<String>,
    /// Found token
    pub found: Option<String>,
    /// Error code for deduplication
    pub code: String,
}

/// A parser warning
#[derive(Debug, Clone)]
pub struct ParserWarning {
    /// Warning message
    pub message: String,
    /// Span start (byte offset)
    pub span_start: usize,
    /// Span end (byte offset)
    pub span_end: usize,
    /// Warning code
    pub code: String,
}

/// Configuration for the parser harness
#[derive(Debug, Clone)]
pub struct ParserHarnessConfig {
    /// Maximum input size
    pub max_input_size: usize,
    /// Timeout for parsing
    pub timeout: Duration,
    /// Maximum nesting depth allowed
    pub max_nesting_depth: usize,
    /// Whether to collect AST statistics
    pub collect_stats: bool,
    /// Whether to test error recovery
    pub test_error_recovery: bool,
}

impl Default for ParserHarnessConfig {
    fn default() -> Self {
        Self {
            max_input_size: 1024 * 1024, // 1MB
            timeout: Duration::from_secs(30),
            max_nesting_depth: 256,
            collect_stats: true,
            test_error_recovery: true,
        }
    }
}

/// Statistics from parser fuzzing
#[derive(Debug, Default)]
pub struct ParserStats {
    /// Total inputs processed
    pub total_inputs: usize,
    /// Inputs that parsed successfully
    pub successful_parses: usize,
    /// Inputs with parse errors
    pub parse_errors: usize,
    /// Inputs that timed out
    pub timeouts: usize,
    /// Average parse time
    pub avg_parse_time_ms: f64,
    /// Maximum observed nesting depth
    pub max_observed_depth: usize,
    /// AST node distribution
    pub node_distribution: HashMap<String, usize>,
    /// Error type distribution
    pub error_distribution: HashMap<String, usize>,
}

/// Parser fuzzing harness
pub struct ParserHarness {
    config: ParserHarnessConfig,
    stats: ParserStats,
    total_parse_time_ms: f64,
}

impl ParserHarness {
    /// Create a new parser harness
    pub fn new(config: ParserHarnessConfig) -> Self {
        Self {
            config,
            stats: ParserStats::default(),
            total_parse_time_ms: 0.0,
        }
    }

    /// Fuzz the parser with input bytes
    pub fn fuzz(&mut self, input: &[u8]) -> ParserResult {
        self.stats.total_inputs += 1;

        // Check input size
        if input.len() > self.config.max_input_size {
            return ParserResult {
                success: false,
                node_count: 0,
                max_depth: 0,
                errors: vec![ParserError {
                    message: "Input too large".to_string(),
                    span_start: 0,
                    span_end: input.len(),
                    expected: vec![],
                    found: None,
                    code: "E0001".to_string(),
                }],
                warnings: vec![],
                duration: Duration::from_secs(0),
                node_histogram: HashMap::new(),
            };
        }

        // Convert to string
        let source = match std::str::from_utf8(input) {
            Ok(s) => s,
            Err(e) => {
                return ParserResult {
                    success: false,
                    node_count: 0,
                    max_depth: 0,
                    errors: vec![ParserError {
                        message: format!("Invalid UTF-8: {}", e),
                        span_start: e.valid_up_to(),
                        span_end: e.valid_up_to() + 1,
                        expected: vec![],
                        found: None,
                        code: "E0002".to_string(),
                    }],
                    warnings: vec![],
                    duration: Duration::from_secs(0),
                    node_histogram: HashMap::new(),
                };
            }
        };

        // Parse the source
        self.parse_source(source)
    }

    /// Parse a source string
    fn parse_source(&mut self, source: &str) -> ParserResult {
        let start = Instant::now();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut node_histogram = HashMap::new();
        let mut node_count = 0;
        let mut max_depth = 0;

        // Simulate parsing (replace with actual parser call)
        let success = self.simulate_parse(
            source,
            &mut node_count,
            &mut max_depth,
            &mut errors,
            &mut warnings,
            &mut node_histogram,
        );

        let duration = start.elapsed();
        let duration_ms = duration.as_secs_f64() * 1000.0;

        // Update stats
        self.total_parse_time_ms += duration_ms;
        self.stats.avg_parse_time_ms = self.total_parse_time_ms / self.stats.total_inputs as f64;

        if success {
            self.stats.successful_parses += 1;
        } else {
            self.stats.parse_errors += 1;
        }

        if max_depth > self.stats.max_observed_depth {
            self.stats.max_observed_depth = max_depth;
        }

        for error in &errors {
            *self.stats.error_distribution
                .entry(error.code.clone())
                .or_insert(0) += 1;
        }

        if self.config.collect_stats {
            for (node, count) in &node_histogram {
                *self.stats.node_distribution.entry(node.clone()).or_insert(0) += count;
            }
        }

        ParserResult {
            success,
            node_count,
            max_depth,
            errors,
            warnings,
            duration,
            node_histogram,
        }
    }

    /// Simulate parsing (placeholder for actual parser)
    fn simulate_parse(
        &self,
        source: &str,
        node_count: &mut usize,
        max_depth: &mut usize,
        errors: &mut Vec<ParserError>,
        warnings: &mut Vec<ParserWarning>,
        histogram: &mut HashMap<String, usize>,
    ) -> bool {
        // Track nesting depth
        let mut current_depth = 0;
        let mut paren_depth = 0;
        let mut brace_depth = 0;
        let mut bracket_depth = 0;

        let chars: Vec<char> = source.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];

            match c {
                '(' => {
                    paren_depth += 1;
                    current_depth = paren_depth.max(brace_depth).max(bracket_depth);
                    if current_depth > *max_depth {
                        *max_depth = current_depth;
                    }
                    if current_depth > self.config.max_nesting_depth {
                        errors.push(ParserError {
                            message: format!("Maximum nesting depth ({}) exceeded", self.config.max_nesting_depth),
                            span_start: i,
                            span_end: i + 1,
                            expected: vec![],
                            found: Some("(".to_string()),
                            code: "E0100".to_string(),
                        });
                    }
                }
                ')' => {
                    if paren_depth == 0 {
                        errors.push(ParserError {
                            message: "Unmatched closing parenthesis".to_string(),
                            span_start: i,
                            span_end: i + 1,
                            expected: vec![],
                            found: Some(")".to_string()),
                            code: "E0101".to_string(),
                        });
                    } else {
                        paren_depth -= 1;
                    }
                }
                '{' => {
                    brace_depth += 1;
                    *histogram.entry("Block".to_string()).or_insert(0) += 1;
                    current_depth = paren_depth.max(brace_depth).max(bracket_depth);
                    if current_depth > *max_depth {
                        *max_depth = current_depth;
                    }
                }
                '}' => {
                    if brace_depth == 0 {
                        errors.push(ParserError {
                            message: "Unmatched closing brace".to_string(),
                            span_start: i,
                            span_end: i + 1,
                            expected: vec![],
                            found: Some("}".to_string()),
                            code: "E0102".to_string(),
                        });
                    } else {
                        brace_depth -= 1;
                    }
                }
                '[' => {
                    bracket_depth += 1;
                    current_depth = paren_depth.max(brace_depth).max(bracket_depth);
                    if current_depth > *max_depth {
                        *max_depth = current_depth;
                    }
                }
                ']' => {
                    if bracket_depth == 0 {
                        errors.push(ParserError {
                            message: "Unmatched closing bracket".to_string(),
                            span_start: i,
                            span_end: i + 1,
                            expected: vec![],
                            found: Some("]".to_string()),
                            code: "E0103".to_string(),
                        });
                    } else {
                        bracket_depth -= 1;
                    }
                }

                // Check for keywords
                'f' => {
                    if source[i..].starts_with("fn ") {
                        *histogram.entry("FunctionDef".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 2;
                    } else if source[i..].starts_with("for ") {
                        *histogram.entry("ForLoop".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 3;
                    }
                }
                'l' => {
                    if source[i..].starts_with("let ") {
                        *histogram.entry("LetStmt".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 3;
                    } else if source[i..].starts_with("loop ") {
                        *histogram.entry("Loop".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 4;
                    }
                }
                'i' => {
                    if source[i..].starts_with("if ") {
                        *histogram.entry("IfExpr".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 2;
                    } else if source[i..].starts_with("implement ") {
                        *histogram.entry("ImplBlock".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 9;
                    }
                }
                'm' => {
                    if source[i..].starts_with("match ") {
                        *histogram.entry("MatchExpr".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 5;
                    }
                }
                't' => {
                    if source[i..].starts_with("type ") {
                        *histogram.entry("TypeDef".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 4;
                    }
                }
                'w' => {
                    if source[i..].starts_with("while ") {
                        *histogram.entry("WhileLoop".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 5;
                    }
                }
                'r' => {
                    if source[i..].starts_with("return") {
                        *histogram.entry("ReturnExpr".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 5;
                    }
                }
                'a' => {
                    if source[i..].starts_with("async ") {
                        *histogram.entry("AsyncFn".to_string()).or_insert(0) += 1;
                        *node_count += 1;
                        i += 5;
                    }
                }

                // Skip string literals
                '"' => {
                    i += 1;
                    let mut escaped = false;
                    while i < chars.len() {
                        if chars[i] == '\\' && !escaped {
                            escaped = true;
                        } else if chars[i] == '"' && !escaped {
                            break;
                        } else {
                            escaped = false;
                        }
                        i += 1;
                    }
                    *histogram.entry("StringLit".to_string()).or_insert(0) += 1;
                    *node_count += 1;
                }

                _ => {}
            }

            i += 1;
        }

        // Check for unclosed delimiters
        if paren_depth > 0 {
            errors.push(ParserError {
                message: format!("Unclosed parenthesis (depth {})", paren_depth),
                span_start: source.len(),
                span_end: source.len(),
                expected: vec![")".to_string()],
                found: None,
                code: "E0104".to_string(),
            });
        }
        if brace_depth > 0 {
            errors.push(ParserError {
                message: format!("Unclosed brace (depth {})", brace_depth),
                span_start: source.len(),
                span_end: source.len(),
                expected: vec!["}".to_string()],
                found: None,
                code: "E0105".to_string(),
            });
        }
        if bracket_depth > 0 {
            errors.push(ParserError {
                message: format!("Unclosed bracket (depth {})", bracket_depth),
                span_start: source.len(),
                span_end: source.len(),
                expected: vec!["]".to_string()],
                found: None,
                code: "E0106".to_string(),
            });
        }

        errors.is_empty()
    }

    /// Get current statistics
    pub fn get_stats(&self) -> &ParserStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = ParserStats::default();
        self.total_parse_time_ms = 0.0;
    }

    /// Check if result indicates an interesting input
    pub fn is_interesting(&self, result: &ParserResult) -> bool {
        // Interesting inputs:
        // 1. Found new error types
        // 2. Deep nesting
        // 3. Many AST nodes
        // 4. Rare node types
        !result.errors.is_empty()
            || result.max_depth > 10
            || result.node_count > 50
            || result.node_histogram.contains_key("AsyncFn")
    }
}

/// Entry point for cargo-fuzz
#[cfg(feature = "fuzz")]
pub fn fuzz_target(data: &[u8]) {
    let config = ParserHarnessConfig::default();
    let mut harness = ParserHarness::new(config);
    let _ = harness.fuzz(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_harness_basic() {
        let config = ParserHarnessConfig::default();
        let mut harness = ParserHarness::new(config);

        let input = b"fn main() { let x = 42; }";
        let result = harness.fuzz(input);

        assert!(result.success);
        assert!(result.node_count > 0);
    }

    #[test]
    fn test_parser_harness_unmatched_braces() {
        let config = ParserHarnessConfig::default();
        let mut harness = ParserHarness::new(config);

        let input = b"fn main() { {";
        let result = harness.fuzz(input);

        assert!(!result.success);
        assert!(result.errors.iter().any(|e| e.message.contains("Unclosed")));
    }

    #[test]
    fn test_parser_harness_deep_nesting() {
        let config = ParserHarnessConfig::default();
        let mut harness = ParserHarness::new(config);

        let input = b"fn main() { ((((((((((1)))))))))) }";
        let result = harness.fuzz(input);

        assert!(result.max_depth >= 10);
    }

    #[test]
    fn test_parser_harness_stats() {
        let config = ParserHarnessConfig::default();
        let mut harness = ParserHarness::new(config);

        harness.fuzz(b"fn main() {}");
        harness.fuzz(b"fn foo() {}");
        harness.fuzz(b"let x = 1;");

        let stats = harness.get_stats();
        assert_eq!(stats.total_inputs, 3);
    }
}
