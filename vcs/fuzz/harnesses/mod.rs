//! Fuzz testing harnesses for Verum compiler components
//!
//! This module provides specialized fuzzing harnesses for each stage
//! of the Verum compilation pipeline:
//!
//! - **Lexer harness**: Fuzz the tokenizer
//! - **Parser harness**: Fuzz the parser
//! - **Type check harness**: Fuzz the type checker
//! - **Codegen harness**: Fuzz the code generator
//!
//! Each harness is designed to:
//! - Detect crashes and hangs
//! - Find edge cases in the implementation
//! - Track coverage and interesting inputs
//! - Provide detailed error information

pub mod lexer_harness;
pub mod parser_harness;
pub mod typecheck_harness;
pub mod codegen_harness;
pub mod escape_harness;

pub use escape_harness::{
    EscapeHarness, EscapeResult, EscapeError, EscapeStats,
};

pub use lexer_harness::{
    LexerHarness, LexerHarnessConfig, LexerResult, LexerError, LexerStats, ErrorSeverity,
};

pub use parser_harness::{
    ParserHarness, ParserHarnessConfig, ParserResult, ParserError, ParserWarning, ParserStats,
};

pub use typecheck_harness::{
    TypeCheckHarness, TypeCheckHarnessConfig, TypeCheckResult, TypeError, TypeWarning, TypeCheckStats,
};

pub use codegen_harness::{
    CodegenHarness, CodegenHarnessConfig, CodegenResult, CodegenError, CodegenWarning,
    CodegenPhase, CodegenStats,
};

use std::time::Duration;

/// Unified harness configuration
#[derive(Debug, Clone)]
pub struct UnifiedHarnessConfig {
    /// Lexer configuration
    pub lexer: LexerHarnessConfig,
    /// Parser configuration
    pub parser: ParserHarnessConfig,
    /// Type checker configuration
    pub typecheck: TypeCheckHarnessConfig,
    /// Code generator configuration
    pub codegen: CodegenHarnessConfig,
    /// Whether to continue to next phase on error
    pub continue_on_error: bool,
}

impl Default for UnifiedHarnessConfig {
    fn default() -> Self {
        Self {
            lexer: LexerHarnessConfig::default(),
            parser: ParserHarnessConfig::default(),
            typecheck: TypeCheckHarnessConfig::default(),
            codegen: CodegenHarnessConfig::default(),
            continue_on_error: false,
        }
    }
}

/// Result from the unified harness
#[derive(Debug)]
pub struct UnifiedResult {
    /// Lexer result
    pub lexer: Option<LexerResult>,
    /// Parser result
    pub parser: Option<ParserResult>,
    /// Type check result
    pub typecheck: Option<TypeCheckResult>,
    /// Codegen result
    pub codegen: Option<CodegenResult>,
    /// Total time
    pub total_duration: Duration,
    /// Which phase failed (if any)
    pub failed_phase: Option<String>,
}

impl UnifiedResult {
    /// Check if any phase failed
    pub fn has_error(&self) -> bool {
        self.failed_phase.is_some()
            || self.lexer.as_ref().map_or(false, |r| !r.errors.is_empty())
            || self.parser.as_ref().map_or(false, |r| !r.success)
            || self.typecheck.as_ref().map_or(false, |r| !r.success)
            || self.codegen.as_ref().map_or(false, |r| !r.success)
    }

    /// Get all errors across phases
    pub fn all_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if let Some(ref r) = self.lexer {
            for e in &r.errors {
                errors.push(format!("[Lexer] {}", e.message));
            }
        }

        if let Some(ref r) = self.parser {
            for e in &r.errors {
                errors.push(format!("[Parser] {}", e.message));
            }
        }

        if let Some(ref r) = self.typecheck {
            for e in &r.errors {
                errors.push(format!("[TypeCheck] {}", e.message));
            }
        }

        if let Some(ref r) = self.codegen {
            for e in &r.errors {
                errors.push(format!("[Codegen] {}", e.message));
            }
        }

        errors
    }
}

/// Unified fuzzing harness that tests all compilation phases
pub struct UnifiedPipelineHarness {
    config: UnifiedHarnessConfig,
    lexer: LexerHarness,
    parser: ParserHarness,
    typecheck: TypeCheckHarness,
    codegen: CodegenHarness,
}

impl UnifiedPipelineHarness {
    /// Create a new unified harness
    pub fn new(config: UnifiedHarnessConfig) -> Self {
        Self {
            lexer: LexerHarness::new(config.lexer.clone()),
            parser: ParserHarness::new(config.parser.clone()),
            typecheck: TypeCheckHarness::new(config.typecheck.clone()),
            codegen: CodegenHarness::new(config.codegen.clone()),
            config,
        }
    }

    /// Fuzz all compilation phases
    pub fn fuzz(&mut self, input: &[u8]) -> UnifiedResult {
        let start = std::time::Instant::now();

        // Phase 1: Lexing
        let lexer_result = self.lexer.fuzz(input);
        if !lexer_result.errors.is_empty() && !self.config.continue_on_error {
            return UnifiedResult {
                lexer: Some(lexer_result),
                parser: None,
                typecheck: None,
                codegen: None,
                total_duration: start.elapsed(),
                failed_phase: Some("Lexer".to_string()),
            };
        }

        // Phase 2: Parsing
        let parser_result = self.parser.fuzz(input);
        if !parser_result.success && !self.config.continue_on_error {
            return UnifiedResult {
                lexer: Some(lexer_result),
                parser: Some(parser_result),
                typecheck: None,
                codegen: None,
                total_duration: start.elapsed(),
                failed_phase: Some("Parser".to_string()),
            };
        }

        // Phase 3: Type Checking
        let typecheck_result = self.typecheck.fuzz(input);
        if !typecheck_result.success && !self.config.continue_on_error {
            return UnifiedResult {
                lexer: Some(lexer_result),
                parser: Some(parser_result),
                typecheck: Some(typecheck_result),
                codegen: None,
                total_duration: start.elapsed(),
                failed_phase: Some("TypeCheck".to_string()),
            };
        }

        // Phase 4: Code Generation
        let codegen_result = self.codegen.fuzz(input);
        let failed_phase = if !codegen_result.success {
            Some("Codegen".to_string())
        } else {
            None
        };

        UnifiedResult {
            lexer: Some(lexer_result),
            parser: Some(parser_result),
            typecheck: Some(typecheck_result),
            codegen: Some(codegen_result),
            total_duration: start.elapsed(),
            failed_phase,
        }
    }

    /// Reset all statistics
    pub fn reset_stats(&mut self) {
        self.lexer.reset_stats();
        self.parser.reset_stats();
        self.typecheck.reset_stats();
        self.codegen.reset_stats();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_harness() {
        let config = UnifiedHarnessConfig::default();
        let mut harness = UnifiedPipelineHarness::new(config);

        let input = b"fn main() { let x = 42; }";
        let result = harness.fuzz(input);

        assert!(result.lexer.is_some());
        assert!(result.parser.is_some());
        assert!(result.typecheck.is_some());
        assert!(result.codegen.is_some());
    }

    #[test]
    fn test_unified_harness_error_collection() {
        let config = UnifiedHarnessConfig::default();
        let mut harness = UnifiedPipelineHarness::new(config);

        let input = b"fn main() { let x = 10 / 0; }";
        let result = harness.fuzz(input);

        let errors = result.all_errors();
        assert!(!errors.is_empty());
    }
}
