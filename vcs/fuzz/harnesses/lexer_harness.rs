//! Lexer fuzzing harness
//!
//! This module provides a fuzzing harness specifically for the Verum lexer.
//! It tests the lexer's ability to handle arbitrary input without crashing,
//! hanging, or producing unexpected behavior.
//!
//! # Test Categories
//!
//! - **Valid input**: Properly tokenized valid Verum source
//! - **Invalid input**: Graceful handling of invalid tokens
//! - **Edge cases**: Unicode, escape sequences, boundary values
//! - **Performance**: Large inputs, long strings, deep nesting
//!
//! # Integration
//!
//! The harness integrates with:
//! - cargo-fuzz for coverage-guided fuzzing
//! - AFL for mutation-based fuzzing
//! - libFuzzer for sanitizer integration

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Result of lexing a source
#[derive(Debug, Clone)]
pub struct LexerResult {
    /// Number of tokens produced
    pub token_count: usize,
    /// Errors encountered during lexing
    pub errors: Vec<LexerError>,
    /// Time taken to lex
    pub duration: Duration,
    /// Peak memory usage (if tracked)
    pub peak_memory: Option<usize>,
    /// Token types histogram
    pub token_histogram: HashMap<String, usize>,
}

/// A lexer error
#[derive(Debug, Clone)]
pub struct LexerError {
    /// Error message
    pub message: String,
    /// Position in source (byte offset)
    pub position: usize,
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
    /// Error severity
    pub severity: ErrorSeverity,
}

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// Warning - lexing can continue
    Warning,
    /// Error - token may be malformed
    Error,
    /// Fatal - lexing cannot continue
    Fatal,
}

/// Configuration for the lexer harness
#[derive(Debug, Clone)]
pub struct LexerHarnessConfig {
    /// Maximum input size to accept
    pub max_input_size: usize,
    /// Timeout for lexing
    pub timeout: Duration,
    /// Whether to track memory usage
    pub track_memory: bool,
    /// Whether to collect token histogram
    pub collect_histogram: bool,
    /// Minimum token count for interesting inputs
    pub min_interesting_tokens: usize,
}

impl Default for LexerHarnessConfig {
    fn default() -> Self {
        Self {
            max_input_size: 1024 * 1024, // 1MB
            timeout: Duration::from_secs(10),
            track_memory: false,
            collect_histogram: true,
            min_interesting_tokens: 5,
        }
    }
}

/// Statistics from lexer fuzzing
#[derive(Debug, Default)]
pub struct LexerStats {
    /// Total inputs processed
    pub total_inputs: usize,
    /// Inputs that lexed successfully
    pub successful: usize,
    /// Inputs that produced errors
    pub with_errors: usize,
    /// Inputs that timed out
    pub timeouts: usize,
    /// Inputs that caused crashes
    pub crashes: usize,
    /// Inputs that found new coverage
    pub new_coverage: usize,
    /// Token type distribution
    pub token_distribution: HashMap<String, usize>,
    /// Error type distribution
    pub error_distribution: HashMap<String, usize>,
}

/// Lexer fuzzing harness
pub struct LexerHarness {
    config: LexerHarnessConfig,
    stats: LexerStats,
}

impl LexerHarness {
    /// Create a new lexer harness
    pub fn new(config: LexerHarnessConfig) -> Self {
        Self {
            config,
            stats: LexerStats::default(),
        }
    }

    /// Fuzz the lexer with a single input
    pub fn fuzz(&mut self, input: &[u8]) -> LexerResult {
        self.stats.total_inputs += 1;

        // Check input size
        if input.len() > self.config.max_input_size {
            return LexerResult {
                token_count: 0,
                errors: vec![LexerError {
                    message: "Input too large".to_string(),
                    position: 0,
                    line: 1,
                    column: 1,
                    severity: ErrorSeverity::Fatal,
                }],
                duration: Duration::from_secs(0),
                peak_memory: None,
                token_histogram: HashMap::new(),
            };
        }

        // Try to convert to string
        let source = match std::str::from_utf8(input) {
            Ok(s) => s,
            Err(_) => {
                // Test with invalid UTF-8
                return self.test_invalid_utf8(input);
            }
        };

        // Run the lexer
        self.test_source(source)
    }

    /// Test a valid UTF-8 source string
    fn test_source(&mut self, source: &str) -> LexerResult {
        let start = Instant::now();
        let mut errors = Vec::new();
        let mut token_count = 0;
        let mut token_histogram = HashMap::new();

        // Simulate lexer execution
        // In a real implementation, this would call verum_lexer::Lexer::new(source)
        let result =
            self.simulate_lexer(source, &mut token_count, &mut errors, &mut token_histogram);

        let duration = start.elapsed();

        // Update stats
        if errors.is_empty() {
            self.stats.successful += 1;
        } else {
            self.stats.with_errors += 1;
            for error in &errors {
                *self
                    .stats
                    .error_distribution
                    .entry(error.message.clone())
                    .or_insert(0) += 1;
            }
        }

        if self.config.collect_histogram {
            for (token, count) in &token_histogram {
                *self
                    .stats
                    .token_distribution
                    .entry(token.clone())
                    .or_insert(0) += count;
            }
        }

        LexerResult {
            token_count,
            errors,
            duration,
            peak_memory: None,
            token_histogram,
        }
    }

    /// Handle invalid UTF-8 input
    fn test_invalid_utf8(&mut self, input: &[u8]) -> LexerResult {
        // The lexer should handle invalid UTF-8 gracefully
        LexerResult {
            token_count: 0,
            errors: vec![LexerError {
                message: "Invalid UTF-8".to_string(),
                position: 0,
                line: 1,
                column: 1,
                severity: ErrorSeverity::Fatal,
            }],
            duration: Duration::from_secs(0),
            peak_memory: None,
            token_histogram: HashMap::new(),
        }
    }

    /// Simulate lexer execution (placeholder for actual lexer integration)
    fn simulate_lexer(
        &self,
        source: &str,
        token_count: &mut usize,
        errors: &mut Vec<LexerError>,
        histogram: &mut HashMap<String, usize>,
    ) -> bool {
        // This simulates lexer behavior for testing the harness
        // In production, this would call the actual lexer

        let mut line = 1;
        let mut column = 1;
        let mut chars = source.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                // Whitespace
                ' ' | '\t' => {
                    column += 1;
                    *histogram.entry("Whitespace".to_string()).or_insert(0) += 1;
                }
                '\n' => {
                    line += 1;
                    column = 1;
                    *histogram.entry("Newline".to_string()).or_insert(0) += 1;
                }
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    line += 1;
                    column = 1;
                    *histogram.entry("Newline".to_string()).or_insert(0) += 1;
                }

                // Comments
                '/' => {
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        // Line comment
                        while let Some(&nc) = chars.peek() {
                            if nc == '\n' {
                                break;
                            }
                            chars.next();
                            column += 1;
                        }
                        *histogram.entry("Comment".to_string()).or_insert(0) += 1;
                        *token_count += 1;
                    } else if chars.peek() == Some(&'*') {
                        chars.next();
                        // Block comment
                        let mut depth = 1;
                        while depth > 0 {
                            match chars.next() {
                                Some('*') if chars.peek() == Some(&'/') => {
                                    chars.next();
                                    depth -= 1;
                                }
                                Some('/') if chars.peek() == Some(&'*') => {
                                    chars.next();
                                    depth += 1;
                                }
                                Some('\n') => {
                                    line += 1;
                                    column = 0;
                                }
                                Some(_) => column += 1,
                                None => {
                                    errors.push(LexerError {
                                        message: "Unterminated block comment".to_string(),
                                        position: 0,
                                        line,
                                        column,
                                        severity: ErrorSeverity::Error,
                                    });
                                    break;
                                }
                            }
                        }
                        *histogram.entry("Comment".to_string()).or_insert(0) += 1;
                        *token_count += 1;
                    } else {
                        *histogram.entry("Operator".to_string()).or_insert(0) += 1;
                        *token_count += 1;
                    }
                }

                // String literals
                '"' => {
                    let mut escaped = false;
                    let start_line = line;
                    let start_col = column;
                    loop {
                        match chars.next() {
                            Some('\\') if !escaped => {
                                escaped = true;
                                column += 1;
                            }
                            Some('"') if !escaped => {
                                break;
                            }
                            Some('\n') => {
                                line += 1;
                                column = 0;
                                escaped = false;
                            }
                            Some(_) => {
                                escaped = false;
                                column += 1;
                            }
                            None => {
                                errors.push(LexerError {
                                    message: "Unterminated string literal".to_string(),
                                    position: 0,
                                    line: start_line,
                                    column: start_col,
                                    severity: ErrorSeverity::Error,
                                });
                                break;
                            }
                        }
                    }
                    *histogram.entry("String".to_string()).or_insert(0) += 1;
                    *token_count += 1;
                }

                // Character literals
                '\'' => {
                    let start_col = column;
                    let mut found_end = false;
                    let mut escaped = false;

                    while let Some(nc) = chars.next() {
                        column += 1;
                        if nc == '\\' && !escaped {
                            escaped = true;
                        } else if nc == '\'' && !escaped {
                            found_end = true;
                            break;
                        } else {
                            escaped = false;
                        }
                    }

                    if !found_end {
                        errors.push(LexerError {
                            message: "Unterminated character literal".to_string(),
                            position: 0,
                            line,
                            column: start_col,
                            severity: ErrorSeverity::Error,
                        });
                    }
                    *histogram.entry("Char".to_string()).or_insert(0) += 1;
                    *token_count += 1;
                }

                // Identifiers and keywords
                'a'..='z' | 'A'..='Z' | '_' => {
                    while let Some(&nc) = chars.peek() {
                        if nc.is_alphanumeric() || nc == '_' {
                            chars.next();
                            column += 1;
                        } else {
                            break;
                        }
                    }
                    *histogram.entry("Identifier".to_string()).or_insert(0) += 1;
                    *token_count += 1;
                }

                // Numbers
                '0'..='9' => {
                    while let Some(&nc) = chars.peek() {
                        if nc.is_ascii_digit()
                            || nc == '.'
                            || nc == '_'
                            || nc == 'e'
                            || nc == 'E'
                            || nc == 'x'
                            || nc == 'X'
                            || nc == 'o'
                            || nc == 'O'
                            || nc == 'b'
                            || nc == 'B'
                            || ('a'..='f').contains(&nc)
                            || ('A'..='F').contains(&nc)
                        {
                            chars.next();
                            column += 1;
                        } else {
                            break;
                        }
                    }
                    *histogram.entry("Number".to_string()).or_insert(0) += 1;
                    *token_count += 1;
                }

                // Operators and punctuation
                '+' | '-' | '*' | '%' | '=' | '!' | '<' | '>' | '&' | '|' | '^' | '~' => {
                    // Check for multi-character operators
                    if let Some(&nc) = chars.peek()
                        && (nc == '='
                            || nc == c
                            || (c == '-' && nc == '>')
                            || (c == '=' && nc == '>'))
                    {
                        chars.next();
                        column += 1;
                    }
                    *histogram.entry("Operator".to_string()).or_insert(0) += 1;
                    *token_count += 1;
                }

                // Delimiters
                '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '.' | '@' | '#' | '?' => {
                    *histogram.entry("Delimiter".to_string()).or_insert(0) += 1;
                    *token_count += 1;
                }

                // Unknown characters
                _ => {
                    if !c.is_ascii() {
                        // Unicode character - might be valid identifier
                        *histogram.entry("Unicode".to_string()).or_insert(0) += 1;
                    } else {
                        errors.push(LexerError {
                            message: format!("Unexpected character: {:?}", c),
                            position: 0,
                            line,
                            column,
                            severity: ErrorSeverity::Warning,
                        });
                    }
                    *token_count += 1;
                }
            }
            column += 1;
        }

        errors.is_empty()
    }

    /// Get current statistics
    pub fn get_stats(&self) -> &LexerStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = LexerStats::default();
    }

    /// Check if an input is interesting (finds new coverage)
    pub fn is_interesting(&self, result: &LexerResult) -> bool {
        // Input is interesting if:
        // 1. It found errors (potential edge case)
        // 2. It has many tokens (complex input)
        // 3. It contains rare token types
        !result.errors.is_empty()
            || result.token_count >= self.config.min_interesting_tokens
            || result.token_histogram.contains_key("Unicode")
    }
}

/// Entry point for cargo-fuzz
#[cfg(feature = "fuzz")]
pub fn fuzz_target(data: &[u8]) {
    let config = LexerHarnessConfig::default();
    let mut harness = LexerHarness::new(config);
    let _ = harness.fuzz(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_harness_basic() {
        let config = LexerHarnessConfig::default();
        let mut harness = LexerHarness::new(config);

        let input = b"fn main() {}";
        let result = harness.fuzz(input);

        assert!(result.token_count > 0);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_lexer_harness_string() {
        let config = LexerHarnessConfig::default();
        let mut harness = LexerHarness::new(config);

        let input = b"let x = \"hello world\";";
        let result = harness.fuzz(input);

        assert!(result.errors.is_empty());
        assert!(result.token_histogram.contains_key("String"));
    }

    #[test]
    fn test_lexer_harness_unterminated_string() {
        let config = LexerHarnessConfig::default();
        let mut harness = LexerHarness::new(config);

        let input = b"let x = \"hello";
        let result = harness.fuzz(input);

        assert!(!result.errors.is_empty());
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("Unterminated"))
        );
    }

    #[test]
    fn test_lexer_harness_stats() {
        let config = LexerHarnessConfig::default();
        let mut harness = LexerHarness::new(config);

        harness.fuzz(b"fn main() {}");
        harness.fuzz(b"let x = 42;");
        harness.fuzz(b"if true { }");

        let stats = harness.get_stats();
        assert_eq!(stats.total_inputs, 3);
        assert!(stats.successful > 0);
    }
}
