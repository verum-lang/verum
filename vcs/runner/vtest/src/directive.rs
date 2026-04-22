//! Directive parsing for VCS test files.
//!
//! Parses test directives from `.vr` files according to the VCS specification.
//! Directives are embedded in comments and control test execution.
//!
//! # Supported Directives
//!
//! - `@test: <type>` - Test type (parse-pass, typecheck-fail, run, etc.)
//! - `@tier: <tiers>` - Execution tiers (0, 1, 2, 3, all, compiled)
//! - `@level: <level>` - Strictness level (L0, L1, L2, L3, L4)
//! - `@tags: <tags>` - Comma-separated test tags
//! - `@timeout: <ms>` - Timeout in milliseconds
//! - `@expected-error: <spec>` - Expected error specification
//! - `@expected-stdout: <text>` - Expected stdout output (supports multi-line with @expected-stdout-begin/@expected-stdout-end)
//! - `@expected-stderr: <text>` - Expected stderr output
//! - `@expected-exit: <code>` - Expected exit code
//! - `@expected-panic: <msg>` - Expected panic message
//! - `@expected-performance: <spec>` - Performance expectation
//! - `@expected-stdout-file: <path>` - File containing expected stdout
//! - `@expected-error-count: <n>` - Expected number of errors
//! - `@expected-warning: <spec>` - Expected warning
//! - `@expected-warning-count: <n>` - Expected number of warnings
//! - `@solver: <name>` - SMT solver name
//! - `@solver-version: <version>` - Required solver version
//! - `@skip: <reason>` - Skip this test with given reason
//! - `@description: <text>` - Human-readable test description
//! - `@requires: <feature>` - Required feature flags
//!
//! # Multi-line Values
//!
//! For expected values that span multiple lines, use the block syntax:
//!
//! ```text
//! // @expected-stdout-begin
//! // Line 1
//! // Line 2
//! // @expected-stdout-end
//! ```
//!
//! # Error Code Categories
//!
//! Error codes follow the pattern EXXX where X is a digit:
//! - E0XX: Parse errors
//! - E1XX: Lexer errors
//! - E2XX: Type errors
//! - E3XX: Borrow/ownership errors
//! - E4XX: Verification errors
//! - E5XX: Context system errors
//! - E6XX: Module/coherence errors
//! - E7XX: Async errors
//! - E8XX: FFI errors
//! - E9XX: Internal compiler errors

use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;
use thiserror::Error;
use verum_common::{List, Set, Text};

/// Error type for directive parsing failures.
#[derive(Debug, Error)]
pub enum DirectiveError {
    #[error("Invalid test type: {0}")]
    InvalidTestType(Text),

    #[error("Invalid tier specification: {0}")]
    InvalidTier(Text),

    #[error("Invalid level: {0}")]
    InvalidLevel(Text),

    #[error("Invalid timeout value: {0}")]
    InvalidTimeout(Text),

    #[error("Missing required directive: @test")]
    MissingTestDirective,

    #[error("Invalid error specification: {0}")]
    InvalidErrorSpec(Text),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Parse error at line {line}: {message}")]
    ParseError { line: usize, message: Text },

    #[error("Unclosed multi-line block '{block}' started at line {start_line}")]
    UnclosedBlock { block: Text, start_line: usize },

    #[error("Unexpected block end '{block}' at line {line} without matching begin")]
    UnexpectedBlockEnd { block: Text, line: usize },

    #[error("Invalid error code format: {0} (expected E0XX through E9XX)")]
    InvalidErrorCode(Text),

    #[error("Conflicting directives: {0}")]
    ConflictingDirectives(Text),

    #[error("Invalid performance specification: {0}")]
    InvalidPerformanceSpec(Text),

    #[error("Validation error: {0}")]
    ValidationError(Text),
}

/// Test type classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TestType {
    /// Code should parse successfully
    ParsePass,
    /// Code should fail to parse
    ParseFail,
    /// Parser should report errors but recover and continue parsing
    ParseRecover,
    /// Code should typecheck successfully
    TypecheckPass,
    /// Code should fail typechecking
    TypecheckFail,
    /// Verification should pass
    VerifyPass,
    /// Verification should fail
    VerifyFail,
    /// Program should run and produce expected output
    Run,
    /// Program should panic with expected message
    RunPanic,
    /// Only compile, do not run
    CompileOnly,
    /// Differential test (compare tiers)
    Differential,
    /// Performance benchmark
    Benchmark,

    // === VBC-First Pipeline Verification Tests ===

    /// Full common pipeline test (parse + types + contracts + context)
    /// Uses verum_compiler::api::run_common_pipeline()
    CommonPipeline,
    /// Common pipeline should fail at some stage
    CommonPipelineFail,
    /// VBC codegen test (common pipeline + VBC generation)
    /// Uses verum_compiler::api::compile_to_vbc()
    VbcCodegen,
    /// VBC codegen should fail
    VbcCodegenFail,

    // === Meta-System Tests ===

    /// Meta-code should compile and evaluate successfully at compile-time
    /// Tests meta functions, builtins, and compile-time evaluation
    MetaPass,
    /// Meta-code should fail with expected error during meta evaluation
    /// For testing context requirements, type errors, sandbox violations
    MetaFail,
    /// Meta-code should evaluate to an expected value at compile-time
    /// Use with @expected-value directive
    MetaEval,

    // === Interpreter-Specific Execution Tests ===

    /// Program should run on Tier 0 VBC interpreter and produce expected output.
    /// Unlike `Run`, this always uses the interpreter (no AOT/JIT fallback).
    RunInterpreter,
    /// Program should panic on Tier 0 VBC interpreter with expected message.
    RunInterpreterPanic,
}

impl TestType {
    /// Parse a test type from string.
    pub fn from_str(s: &str) -> Result<Self, DirectiveError> {
        match s.trim().to_lowercase().as_str() {
            "parse-pass" | "parsepass" | "parse" | "parser" => Ok(Self::ParsePass),
            "parse-fail" | "parsefail" => Ok(Self::ParseFail),
            "parse-recover" | "parserecover" => Ok(Self::ParseRecover),
            "typecheck-pass" | "typecheckpass" | "unit" => Ok(Self::TypecheckPass),
            "typecheck-fail" | "typecheckfail" => Ok(Self::TypecheckFail),
            "verify-pass" | "verifypass" => Ok(Self::VerifyPass),
            "verify-fail" | "verifyfail" => Ok(Self::VerifyFail),
            "run" => Ok(Self::Run),
            "run-panic" | "runpanic" => Ok(Self::RunPanic),
            "compile-only" | "compileonly" => Ok(Self::CompileOnly),
            "differential" => Ok(Self::Differential),
            "benchmark" | "bench" => Ok(Self::Benchmark),
            // VBC-first pipeline tests
            "common-pipeline" | "commonpipeline" => Ok(Self::CommonPipeline),
            "common-pipeline-fail" | "commonpipelinefail" => Ok(Self::CommonPipelineFail),
            "vbc-codegen" | "vbccodegen" => Ok(Self::VbcCodegen),
            "vbc-codegen-fail" | "vbccodegenfail" => Ok(Self::VbcCodegenFail),
            // Meta-system tests
            "meta-pass" | "metapass" => Ok(Self::MetaPass),
            "meta-fail" | "metafail" => Ok(Self::MetaFail),
            "meta-eval" | "metaeval" => Ok(Self::MetaEval),
            // Interpreter-specific execution tests
            "run-interpreter" | "runinterpreter" => Ok(Self::RunInterpreter),
            "run-interpreter-panic" | "runinterpreterpanic" => Ok(Self::RunInterpreterPanic),
            _ => Err(DirectiveError::InvalidTestType(s.to_string().into())),
        }
    }

    /// Check if this test type requires execution.
    pub fn requires_execution(&self) -> bool {
        matches!(
            self,
            Self::Run | Self::RunPanic | Self::RunInterpreter | Self::RunInterpreterPanic | Self::Differential | Self::Benchmark
        )
    }

    /// Check if this test type expects compilation to succeed.
    pub fn expects_compile_success(&self) -> bool {
        matches!(
            self,
            Self::TypecheckPass
                | Self::VerifyPass
                | Self::Run
                | Self::RunPanic
                | Self::RunInterpreter
                | Self::RunInterpreterPanic
                | Self::CompileOnly
                | Self::Differential
                | Self::Benchmark
                | Self::CommonPipeline
                | Self::VbcCodegen
                | Self::MetaPass
                | Self::MetaEval
        )
    }

    /// Check if this test uses the direct compiler API (no subprocess).
    pub fn uses_direct_api(&self) -> bool {
        matches!(
            self,
            Self::ParsePass
                | Self::ParseFail
                | Self::ParseRecover
                | Self::TypecheckPass
                | Self::TypecheckFail
                | Self::RunInterpreter
                | Self::RunInterpreterPanic
                | Self::CommonPipeline
                | Self::CommonPipelineFail
                | Self::VbcCodegen
                | Self::VbcCodegenFail
                | Self::MetaPass
                | Self::MetaFail
                | Self::MetaEval
        )
    }

    /// Check if this is a compile-time only test (no execution needed).
    ///
    /// Compile-time tests only exercise the common pipeline (parse, typecheck, verify)
    /// and don't need tier specification. The `@tier` directive is ignored for these tests.
    /// Meta-system tests are also compile-time only since they evaluate during compilation.
    pub fn is_compile_time_only(&self) -> bool {
        matches!(
            self,
            Self::ParsePass
                | Self::ParseFail
                | Self::ParseRecover
                | Self::TypecheckPass
                | Self::TypecheckFail
                | Self::VerifyPass
                | Self::VerifyFail
                | Self::CommonPipeline
                | Self::CommonPipelineFail
                | Self::CompileOnly
                | Self::MetaPass
                | Self::MetaFail
                | Self::MetaEval
        )
    }

    /// Get default tiers for this test type.
    ///
    /// - Compile-time tests: single tier (Tier0) - tier doesn't affect compilation
    ///   but we need at least one for the executor loop to run
    /// - Execution tests: all tiers
    pub fn default_tiers(&self) -> List<Tier> {
        if matches!(self, Self::RunInterpreter | Self::RunInterpreterPanic) {
            // Interpreter tests always run on Tier 0 only
            return vec![Tier::Tier0].into();
        }
        if self.is_compile_time_only() {
            // Compile-time tests need at least one tier for the executor to run
            // The specific tier doesn't matter since compilation is tier-independent
            vec![Tier::Tier0].into()
        } else if matches!(self, Self::Differential) {
            // Differential tests need at least two tiers
            vec![Tier::Tier0, Tier::Tier3].into()
        } else {
            // Execution tests default to all tiers
            Tier::all()
        }
    }

    /// Get the pipeline stage for this test type.
    pub fn stage(&self) -> TestStage {
        if self.is_compile_time_only() {
            TestStage::Compile
        } else {
            TestStage::Execute
        }
    }
}

/// Test stage - compile-time or execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TestStage {
    /// Compile-time only (common pipeline)
    Compile,
    /// Requires execution (VBC interpreter or JIT/AOT)
    Execute,
}

impl std::fmt::Display for TestType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::ParsePass => "parse-pass",
            Self::ParseFail => "parse-fail",
            Self::ParseRecover => "parse-recover",
            Self::TypecheckPass => "typecheck-pass",
            Self::TypecheckFail => "typecheck-fail",
            Self::VerifyPass => "verify-pass",
            Self::VerifyFail => "verify-fail",
            Self::Run => "run",
            Self::RunPanic => "run-panic",
            Self::CompileOnly => "compile-only",
            Self::Differential => "differential",
            Self::Benchmark => "benchmark",
            // VBC-first pipeline tests
            Self::CommonPipeline => "common-pipeline",
            Self::CommonPipelineFail => "common-pipeline-fail",
            Self::VbcCodegen => "vbc-codegen",
            Self::VbcCodegenFail => "vbc-codegen-fail",
            // Meta tests
            Self::MetaPass => "meta-pass",
            Self::MetaFail => "meta-fail",
            Self::MetaEval => "meta-eval",
            // Interpreter-specific execution tests
            Self::RunInterpreter => "run-interpreter",
            Self::RunInterpreterPanic => "run-interpreter-panic",
        };
        write!(f, "{}", s)
    }
}

/// Execution tier for the test.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Tier {
    /// Tier 0: Interpreter
    Tier0 = 0,
    /// Tier 1: JIT baseline
    Tier1 = 1,
    /// Tier 2: JIT optimized
    Tier2 = 2,
    /// Tier 3: AOT compiled
    Tier3 = 3,
}

impl Tier {
    /// Parse a tier from a string.
    pub fn from_str(s: &str) -> Result<Self, DirectiveError> {
        match s.trim() {
            "0" => Ok(Self::Tier0),
            "1" => Ok(Self::Tier1),
            "2" => Ok(Self::Tier2),
            "3" => Ok(Self::Tier3),
            _ => Err(DirectiveError::InvalidTier(s.to_string().into())),
        }
    }

    /// Get all tiers.
    pub fn all() -> List<Self> {
        vec![Self::Tier0, Self::Tier1, Self::Tier2, Self::Tier3].into()
    }

    /// Get compiled tiers (1, 2, 3).
    pub fn compiled() -> List<Self> {
        vec![Self::Tier1, Self::Tier2, Self::Tier3].into()
    }
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", *self as u8)
    }
}

/// Strictness level for the test.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Level {
    /// L0: Critical (100% pass required)
    L0 = 0,
    /// L1: Core (100% pass required)
    L1 = 1,
    /// L2: Standard (95%+ pass required)
    L2 = 2,
    /// L3: Extended (90%+ pass required)
    L3 = 3,
    /// L4: Performance (advisory)
    L4 = 4,
}

impl Level {
    /// Parse a level from a string.
    pub fn from_str(s: &str) -> Result<Self, DirectiveError> {
        match s.trim().to_uppercase().as_str() {
            "L0" => Ok(Self::L0),
            "L1" => Ok(Self::L1),
            "L2" => Ok(Self::L2),
            "L3" => Ok(Self::L3),
            "L4" => Ok(Self::L4),
            _ => Err(DirectiveError::InvalidLevel(s.to_string().into())),
        }
    }

    /// Get the pass threshold for this level.
    pub fn pass_threshold(&self) -> f64 {
        match self {
            Self::L0 | Self::L1 => 1.0,
            Self::L2 => 0.95,
            Self::L3 => 0.90,
            Self::L4 => 0.0, // Advisory
        }
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
            Self::L4 => "L4",
        };
        write!(f, "{}", s)
    }
}

/// Error code category for validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErrorCategory {
    /// E0XX: Parse errors
    Parse,
    /// E1XX: Lexer errors
    Lexer,
    /// E2XX: Type errors
    Type,
    /// E3XX: Borrow/ownership errors
    Borrow,
    /// E4XX: Verification errors
    Verification,
    /// E5XX: Context system errors
    Context,
    /// E6XX: Module/coherence errors
    Module,
    /// E7XX: Async errors
    Async,
    /// E8XX: FFI errors
    Ffi,
    /// E9XX: Internal compiler errors
    Internal,

    // === Meta-System Error Categories (M-prefix) ===

    /// M0XX: Core meta errors (function not found, evaluation failed)
    MetaCore,
    /// M1XX: Builtin errors (unknown builtin, arity mismatch)
    MetaBuiltin,
    /// M2XX: Context errors (context not enabled, denied)
    MetaContext,
    /// M3XX: Sandbox errors (forbidden op, resource limit)
    MetaSandbox,
    /// M4XX: Quote/Hygiene errors (invalid quote, hygiene violation)
    MetaQuote,
    /// M5XX: Type-level errors (reduction failed, normalization)
    MetaTypeLevel,
    /// M6XX: Const evaluation errors (overflow, division by zero)
    MetaConst,
}

impl ErrorCategory {
    /// Parse error category from error code.
    ///
    /// Supports two formats:
    /// - EXXX / WXXX: Standard compiler errors/warnings (E0XX through E9XX)
    /// - MXXX: Meta-system errors (M0XX through M6XX)
    pub fn from_code(code: &str) -> Option<Self> {
        let code = code.trim();
        if code.len() < 2 {
            return None;
        }

        let first_char = code.chars().next()?;
        let category_char = code.chars().nth(1)?;

        match first_char {
            // Standard compiler errors/warnings (E/W prefix)
            'E' | 'W' => match category_char {
                '0' => Some(Self::Parse),
                '1' => Some(Self::Lexer),
                '2' => Some(Self::Type),
                '3' => Some(Self::Borrow),
                '4' => Some(Self::Verification),
                '5' => Some(Self::Context),
                '6' => Some(Self::Module),
                '7' => Some(Self::Async),
                '8' => Some(Self::Ffi),
                '9' => Some(Self::Internal),
                _ => None,
            },
            // Meta-system errors (M prefix)
            'M' => match category_char {
                '0' => Some(Self::MetaCore),
                '1' => Some(Self::MetaBuiltin),
                '2' => Some(Self::MetaContext),
                '3' => Some(Self::MetaSandbox),
                '4' => Some(Self::MetaQuote),
                '5' => Some(Self::MetaTypeLevel),
                '6' => Some(Self::MetaConst),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get human-readable name for this category.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Lexer => "lexer",
            Self::Type => "type",
            Self::Borrow => "borrow/ownership",
            Self::Verification => "verification",
            Self::Context => "context system",
            Self::Module => "module/coherence",
            Self::Async => "async",
            Self::Ffi => "FFI",
            Self::Internal => "internal",
            // Meta-system error categories
            Self::MetaCore => "meta-core",
            Self::MetaBuiltin => "meta-builtin",
            Self::MetaContext => "meta-context",
            Self::MetaSandbox => "meta-sandbox",
            Self::MetaQuote => "meta-quote/hygiene",
            Self::MetaTypeLevel => "meta-type-level",
            Self::MetaConst => "meta-const-eval",
        }
    }
}

/// Expected error specification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExpectedError {
    /// Error code (e.g., "E302")
    pub code: Text,
    /// Error message pattern (optional)
    pub message: Option<Text>,
    /// Expected line number (optional)
    pub line: Option<usize>,
    /// Expected column number (optional)
    pub column: Option<usize>,
    /// End column for range (optional)
    pub end_column: Option<usize>,
    /// Expected severity (error, warning, note)
    pub severity: Option<Text>,
    /// Error category (derived from code)
    pub category: Option<ErrorCategory>,
}

impl ExpectedError {
    /// Parse an expected error from a directive string.
    ///
    /// Formats supported:
    /// - `E302 "Use after move" at line 8, col 10`
    /// - `E302 "Use after move" at line 8, col 10-15`
    /// - `E302 at line 8`
    /// - `E302`
    /// - `[error] E302 "message"`
    /// - `E302 at 8:10` (compact line:column format)
    /// - `E302 at 8:10-15` (compact with column range)
    /// - `M201 "Context not enabled"` (meta-system error)
    pub fn parse(s: &str) -> Result<Self, DirectiveError> {
        // Pattern for parsing error specifications with optional severity
        // Supports both "at line X, col Y" and compact "at X:Y" formats
        // Supports E/W (compiler errors/warnings) and M (meta-system errors) prefixes
        static ERROR_RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(
                r#"^(?:\[(\w+)\]\s*)?([EWM]\d{3})\s*(?:"([^"]+)")?\s*(?:at\s+(?:line\s+)?(\d+))?(?:(?:,\s*col\s+|:)(\d+)(?:-(\d+))?)?"#,
            )
            .unwrap()
        });

        let s = s.trim();

        if let Some(caps) = ERROR_RE.captures(s) {
            let severity: Option<Text> = caps.get(1).map(|m| m.as_str().to_string().into());
            let code: Text = caps.get(2).map(|m| m.as_str().to_string().into()).unwrap();
            let message: Option<Text> = caps.get(3).map(|m| m.as_str().to_string().into());
            let line = caps.get(4).and_then(|m| m.as_str().parse().ok());
            let column = caps.get(5).and_then(|m| m.as_str().parse().ok());
            let end_column = caps.get(6).and_then(|m| m.as_str().parse().ok());

            // Validate error code format
            let category = ErrorCategory::from_code(&code);
            if category.is_none() {
                return Err(DirectiveError::InvalidErrorCode(code));
            }

            Ok(Self {
                code,
                message,
                line,
                column,
                end_column,
                severity,
                category,
            })
        } else {
            Err(DirectiveError::InvalidErrorSpec(s.to_string().into()))
        }
    }

    /// Create a new ExpectedError with the given code.
    pub fn with_code(code: impl Into<Text>) -> Self {
        let code = code.into();
        let category = ErrorCategory::from_code(&code);
        Self {
            code,
            message: None,
            line: None,
            column: None,
            end_column: None,
            severity: None,
            category,
        }
    }

    /// Add a message pattern to match.
    pub fn with_message(mut self, message: impl Into<Text>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Add line/column position.
    pub fn at_position(mut self, line: usize, column: Option<usize>) -> Self {
        self.line = Some(line);
        self.column = column;
        self
    }

    /// Add column range.
    pub fn with_column_range(mut self, start: usize, end: usize) -> Self {
        self.column = Some(start);
        self.end_column = Some(end);
        self
    }

    /// Check if this error matches an actual error.
    pub fn matches(
        &self,
        code: &str,
        message: Option<&str>,
        line: Option<usize>,
        column: Option<usize>,
    ) -> bool {
        // Code must match exactly
        if self.code != code {
            return false;
        }

        // Message pattern match (substring)
        if let Some(ref expected_msg) = self.message {
            if let Some(actual_msg) = message {
                if !actual_msg.contains(expected_msg.as_str()) {
                    return false;
                }
            } else {
                return false;
            }
        }

        // Line match (exact)
        if let Some(expected_line) = self.line {
            if let Some(actual_line) = line {
                if expected_line != actual_line {
                    return false;
                }
            }
        }

        // Column match (exact or range)
        if let Some(expected_col) = self.column {
            if let Some(actual_col) = column {
                if let Some(end_col) = self.end_column {
                    // Range check
                    if actual_col < expected_col || actual_col > end_col {
                        return false;
                    }
                } else {
                    // Exact match
                    if expected_col != actual_col {
                        return false;
                    }
                }
            }
        }

        true
    }

    /// Check if this error matches output from stderr.
    ///
    /// Parses stderr for error patterns and checks for matches.
    pub fn matches_stderr(&self, stderr: &str) -> bool {
        // Look for error code in stderr
        if !stderr.contains(self.code.as_str()) {
            return false;
        }

        // Check message if specified
        if let Some(ref msg) = self.message {
            if !stderr.contains(msg.as_str()) {
                return false;
            }
        }

        // Check line number if specified
        if let Some(line) = self.line {
            let line_pattern = format!("line {}", line);
            let line_pattern2 = format!(":{}", line);
            let line_pattern3 = format!(":{}", line);
            if !stderr.contains(&line_pattern)
                && !stderr.contains(&line_pattern2)
                && !stderr.contains(&line_pattern3)
            {
                return false;
            }

            // Check column if specified
            if let Some(col) = self.column {
                let col_pattern = format!(":{}:", col);
                let col_pattern2 = format!("col {}", col);
                let col_pattern3 = format!("column {}", col);
                // Column check is optional - only fail if we can detect column info
                // but it doesn't match
                if stderr.contains(":")
                    && !stderr.contains(&col_pattern)
                    && !stderr.contains(&col_pattern2)
                    && !stderr.contains(&col_pattern3)
                {
                    // Try range match if end_column is specified
                    if let Some(end_col) = self.end_column {
                        let found_col_match = (col..=end_col).any(|c| {
                            stderr.contains(&format!(":{}:", c))
                                || stderr.contains(&format!("col {}", c))
                                || stderr.contains(&format!("column {}", c))
                        });
                        if !found_col_match {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
            }
        }

        true
    }
}

/// Parsed test directives.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestDirectives {
    /// Test type
    pub test_type: TestType,
    /// Execution tiers
    pub tiers: List<Tier>,
    /// Strictness level
    pub level: Level,
    /// Tags for filtering
    pub tags: Set<Text>,
    /// Timeout in milliseconds
    pub timeout_ms: Option<u64>,
    /// Expected errors
    pub expected_errors: List<ExpectedError>,
    /// Expected warnings
    pub expected_warnings: List<ExpectedError>,
    /// Expected error count
    pub expected_error_count: Option<usize>,
    /// Expected warning count
    pub expected_warning_count: Option<usize>,
    /// Expected stdout content
    pub expected_stdout: Option<Text>,
    /// Expected stdout from file
    pub expected_stdout_file: Option<Text>,
    /// Expected stderr content
    pub expected_stderr: Option<Text>,
    /// Expected exit code
    pub expected_exit: Option<i32>,
    /// Expected panic message
    pub expected_panic: Option<Text>,
    /// Expected performance specification
    pub expected_performance: Option<Text>,
    /// SMT solver name
    pub solver: Option<Text>,
    /// SMT solver version requirement
    pub solver_version: Option<Text>,
    /// Source file path
    pub source_path: Text,
    /// Source content
    pub source_content: Text,
    /// Skip this test with reason
    pub skip: Option<Text>,
    /// Human-readable description
    pub description: Option<Text>,
    /// Required feature flags
    pub requires: Set<Text>,
    /// Parse errors encountered during directive parsing (non-fatal)
    pub parse_warnings: List<Text>,

    // === Meta-System Test Directives ===

    /// Expected value for meta-eval tests (e.g., "42", "[1, 2, 3]", "\"hello\"")
    pub expected_value: Option<Text>,
    /// Expected type of result for meta tests (e.g., "Int", "List<Text>")
    pub expected_type: Option<Text>,
    /// Required meta contexts for the test (e.g., ["MetaTypes", "MetaRuntime"])
    pub contexts: Set<Text>,
}

impl Default for TestDirectives {
    fn default() -> Self {
        Self {
            test_type: TestType::Run,
            tiers: List::new(), // Will be set based on test_type after parsing
            level: Level::L1,
            tags: Set::new(),
            timeout_ms: None,
            expected_errors: List::new(),
            expected_warnings: List::new(),
            expected_error_count: None,
            expected_warning_count: None,
            expected_stdout: None,
            expected_stdout_file: None,
            expected_stderr: None,
            expected_exit: None,
            expected_panic: None,
            expected_performance: None,
            solver: None,
            solver_version: None,
            source_path: Text::new(),
            source_content: Text::new(),
            skip: None,
            description: None,
            // Meta-system defaults
            expected_value: None,
            expected_type: None,
            contexts: Set::new(),
            requires: Set::new(),
            parse_warnings: List::new(),
        }
    }
}

/// Block type for multi-line directives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockType {
    Stdout,
    Stderr,
}

impl TestDirectives {
    /// Parse directives from a file path.
    pub fn from_file(path: &Path) -> Result<Self, DirectiveError> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content, path.to_string_lossy().to_string().into())
    }

    /// Parse directives from source content.
    ///
    /// Supports both single-line and multi-line directives.
    /// Multi-line blocks use @directive-begin and @directive-end markers.
    pub fn parse(content: &str, source_path: Text) -> Result<Self, DirectiveError> {
        let mut directives = Self {
            source_path,
            source_content: content.to_string().into(),
            ..Default::default()
        };

        let mut found_test = false;

        // State for multi-line block parsing
        let mut current_block: Option<(BlockType, usize, Vec<String>)> = None;

        let lines: Vec<&str> = content.lines().collect();

        // Track whether we've exited the header (doc comments and directives)
        let mut past_header = false;

        for (line_num, line) in lines.iter().enumerate() {
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Stop parsing directives after the first non-comment line
            // This prevents @skip: inside function bodies from being parsed as directives
            if !line.starts_with("//") && !line.starts_with("///") {
                past_header = true;
            }

            // Skip non-comment lines
            if !line.starts_with("//") {
                continue;
            }

            // Skip directives after we've passed the header
            // (but still process multi-line blocks that were started in the header)
            if past_header && current_block.is_none() {
                continue;
            }

            let comment = line.trim_start_matches("//").trim();

            // Check for block end markers first
            if comment == "@expected-stdout-end" {
                if let Some((BlockType::Stdout, _, block_lines)) = current_block.take() {
                    directives.expected_stdout = Some(block_lines.join("\n").into());
                } else {
                    return Err(DirectiveError::UnexpectedBlockEnd {
                        block: "expected-stdout".to_string().into(),
                        line: line_num + 1,
                    });
                }
                continue;
            } else if comment == "@expected-stderr-end" {
                if let Some((BlockType::Stderr, _, block_lines)) = current_block.take() {
                    directives.expected_stderr = Some(block_lines.join("\n").into());
                } else {
                    return Err(DirectiveError::UnexpectedBlockEnd {
                        block: "expected-stderr".to_string().into(),
                        line: line_num + 1,
                    });
                }
                continue;
            }

            // If we're in a block, accumulate lines
            if let Some((_, _, ref mut block_lines)) = current_block {
                block_lines.push(comment.to_string());
                continue;
            }

            // Check for block begin markers
            if comment == "@expected-stdout-begin" {
                current_block = Some((BlockType::Stdout, line_num + 1, Vec::new()));
                continue;
            } else if comment == "@expected-stderr-begin" {
                current_block = Some((BlockType::Stderr, line_num + 1, Vec::new()));
                continue;
            }

            // Parse @directive: value patterns
            if let Some(rest) = comment.strip_prefix("@test:") {
                directives.test_type = TestType::from_str(rest.trim())?;
                found_test = true;
            } else if let Some(rest) = comment.strip_prefix("@tier:") {
                directives.tiers = parse_tiers(rest.trim())?;
            } else if let Some(rest) = comment.strip_prefix("@level:") {
                directives.level = Level::from_str(rest.trim())?;
            } else if let Some(rest) = comment.strip_prefix("@tags:") {
                directives.tags = parse_tags(rest.trim());
            } else if let Some(rest) = comment.strip_prefix("@timeout:") {
                directives.timeout_ms = Some(parse_timeout(rest.trim())?);
            } else if let Some(rest) = comment.strip_prefix("@expected-error:") {
                match ExpectedError::parse(rest.trim()) {
                    Ok(err) => directives.expected_errors.push(err),
                    Err(e) => {
                        directives
                            .parse_warnings
                            .push(format!("Line {}: {}", line_num + 1, e).into())
                    }
                }
            } else if let Some(rest) = comment.strip_prefix("@expected-warning:") {
                match ExpectedError::parse(rest.trim()) {
                    Ok(err) => directives.expected_warnings.push(err),
                    Err(e) => {
                        directives
                            .parse_warnings
                            .push(format!("Line {}: {}", line_num + 1, e).into())
                    }
                }
            } else if let Some(rest) = comment.strip_prefix("@expected-error-count:") {
                directives.expected_error_count =
                    Some(
                        rest.trim()
                            .parse()
                            .map_err(|_| DirectiveError::ParseError {
                                line: line_num + 1,
                                message: format!("Invalid error count: {}", rest).into(),
                            })?,
                    );
            } else if let Some(rest) = comment.strip_prefix("@expected-warning-count:") {
                directives.expected_warning_count =
                    Some(
                        rest.trim()
                            .parse()
                            .map_err(|_| DirectiveError::ParseError {
                                line: line_num + 1,
                                message: format!("Invalid warning count: {}", rest).into(),
                            })?,
                    );
            } else if let Some(rest) = comment.strip_prefix("@expected-stdout:") {
                directives.expected_stdout = Some(unescape_string(rest.trim()));
            } else if let Some(rest) = comment.strip_prefix("@expected-stdout-file:") {
                directives.expected_stdout_file = Some(rest.trim().to_string().into());
            } else if let Some(rest) = comment.strip_prefix("@expected-stderr:") {
                directives.expected_stderr = Some(unescape_string(rest.trim()));
            } else if let Some(rest) = comment.strip_prefix("@expected-exit:") {
                directives.expected_exit =
                    Some(
                        rest.trim()
                            .parse()
                            .map_err(|_| DirectiveError::ParseError {
                                line: line_num + 1,
                                message: format!("Invalid exit code: {}", rest).into(),
                            })?,
                    );
            } else if let Some(rest) = comment.strip_prefix("@expected-panic:") {
                // Remove quotes if present
                let msg = rest.trim();
                let msg = msg
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(msg);
                directives.expected_panic = Some(msg.to_string().into());
            } else if let Some(rest) = comment.strip_prefix("@expected-performance:") {
                let perf_spec = rest.trim();
                // Validate performance specification format
                if !validate_performance_spec(perf_spec) {
                    return Err(DirectiveError::InvalidPerformanceSpec(
                        perf_spec.to_string().into(),
                    ));
                }
                directives.expected_performance = Some(perf_spec.to_string().into());
            } else if let Some(rest) = comment.strip_prefix("@solver:") {
                directives.solver = Some(rest.trim().to_string().into());
            } else if let Some(rest) = comment.strip_prefix("@solver-version:") {
                directives.solver_version = Some(rest.trim().to_string().into());
            } else if let Some(rest) = comment.strip_prefix("@skip:") {
                directives.skip = Some(rest.trim().to_string().into());
            } else if let Some(rest) = comment.strip_prefix("@description:") {
                directives.description = Some(rest.trim().to_string().into());
            } else if let Some(rest) = comment.strip_prefix("@requires:") {
                for feature in rest.split(',') {
                    let feature = feature.trim();
                    if !feature.is_empty() {
                        directives.requires.insert(feature.to_string().into());
                    }
                }
            } else if comment.starts_with('@')
                && let Some(directive_name) = comment.split(':').next()
                && !directive_name.is_empty()
            {
                // Unknown `@foo:` directive — surface as a parse warning
                // so test-file bugs (e.g. `@expect: error(...)` instead of
                // `@expected-error: ...`) don't silently get ignored.
                //
                // Allow `@spec:` / `@description:` / `@author:` style
                // documentation-only directives to pass without warning.
                const DOC_ONLY: &[&str] = &["@spec", "@author", "@note", "@reference", "@see"];
                if !DOC_ONLY.contains(&directive_name) {
                    directives.parse_warnings.push(
                        format!(
                            "Line {}: unknown directive `{}:` — did you mean `@expected-error:`? Unknown directives are ignored; rename or remove to silence this warning.",
                            line_num + 1,
                            directive_name
                        )
                        .into(),
                    );
                }
            }
        }

        // Check for unclosed blocks
        if let Some((block_type, start_line, _)) = current_block {
            let block_name = match block_type {
                BlockType::Stdout => "expected-stdout",
                BlockType::Stderr => "expected-stderr",
            };
            return Err(DirectiveError::UnclosedBlock {
                block: block_name.to_string().into(),
                start_line,
            });
        }

        if !found_test {
            return Err(DirectiveError::MissingTestDirective);
        }

        // Set default tiers based on test type if not explicitly specified
        // Compile-time tests don't need tiers; execution tests default to all tiers
        if directives.tiers.is_empty() {
            directives.tiers = directives.test_type.default_tiers();
        }

        // Validate the complete directives
        directives.validate()?;

        Ok(directives)
    }

    /// Validate the parsed directives for consistency.
    fn validate(&self) -> Result<(), DirectiveError> {
        // Check for conflicting expectations
        if self.expected_stdout.is_some() && self.expected_stdout_file.is_some() {
            return Err(DirectiveError::ConflictingDirectives(
                "Cannot specify both @expected-stdout and @expected-stdout-file".to_string().into(),
            ));
        }

        // Validate error count matches error list if both specified
        if let (Some(count), errors) = (self.expected_error_count, &self.expected_errors) {
            if !errors.is_empty() && errors.len() != count {
                return Err(DirectiveError::ConflictingDirectives(format!(
                    "@expected-error-count ({}) does not match number of @expected-error directives ({})",
                    count,
                    errors.len()
                ).into()));
            }
        }

        // Validate test type vs expectations
        match self.test_type {
            TestType::ParsePass | TestType::TypecheckPass | TestType::VerifyPass => {
                if !self.expected_errors.is_empty() {
                    return Err(DirectiveError::ConflictingDirectives(format!(
                        "{} test should not have @expected-error directives",
                        self.test_type
                    ).into()));
                }
            }
            TestType::RunPanic => {
                if self.expected_panic.is_none() {
                    return Err(DirectiveError::ValidationError(
                        "run-panic test requires @expected-panic directive".to_string().into(),
                    ));
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Get a unique identifier for this test.
    pub fn test_id(&self) -> Text {
        self.source_path.clone()
    }

    /// Get a display name for this test.
    pub fn display_name(&self) -> Text {
        Path::new(self.source_path.as_str())
            .file_name()
            .map(|s| s.to_string_lossy().to_string().into())
            .unwrap_or_else(|| self.source_path.clone())
    }

    /// Check if this test should run on a specific tier.
    pub fn should_run_on_tier(&self, tier: Tier) -> bool {
        self.tiers.contains(&tier)
    }

    /// Check if this test matches the given tag filter.
    pub fn matches_tags(&self, include: &Set<Text>, exclude: &Set<Text>) -> bool {
        // If include set is non-empty, at least one tag must match
        if !include.is_empty() && !self.tags.iter().any(|t| include.contains(t)) {
            return false;
        }

        // If exclude set is non-empty, none of the tags can match
        if !exclude.is_empty() && self.tags.iter().any(|t| exclude.contains(t)) {
            return false;
        }

        true
    }

    /// Get the effective timeout for this test.
    pub fn effective_timeout_ms(&self) -> u64 {
        self.timeout_ms.unwrap_or(30_000) // Default 30 seconds
    }
}

/// Parse tier specification.
fn parse_tiers(s: &str) -> Result<List<Tier>, DirectiveError> {
    let s = s.trim().to_lowercase();

    if s == "all" {
        return Ok(Tier::all());
    }

    if s == "compiled" {
        return Ok(Tier::compiled());
    }

    let mut tiers = List::new();
    for part in s.split(',') {
        tiers.push(Tier::from_str(part.trim())?);
    }

    Ok(tiers)
}

/// Parse comma-separated tags.
fn parse_tags(s: &str) -> Set<Text> {
    s.split(',')
        .map(|t| t.trim().to_string().into())
        .filter(|t: &Text| !t.is_empty())
        .collect()
}

/// Parse timeout value (supports "ms", "s" suffixes).
fn parse_timeout(s: &str) -> Result<u64, DirectiveError> {
    let s = s.trim();

    if let Some(secs) = s.strip_suffix('s') {
        if secs.ends_with('m') {
            // Already handled by ms
            let ms = secs.strip_suffix('m').unwrap_or(secs);
            return ms
                .trim()
                .parse()
                .map_err(|_| DirectiveError::InvalidTimeout(s.to_string().into()));
        }
        // Plain seconds
        return secs
            .trim()
            .parse::<u64>()
            .map(|v| v * 1000)
            .map_err(|_| DirectiveError::InvalidTimeout(s.to_string().into()));
    }

    let s = s.strip_suffix("ms").unwrap_or(s);
    s.parse()
        .map_err(|_| DirectiveError::InvalidTimeout(s.to_string().into()))
}

/// Unescape a string value (handle \n, \t, etc.).
fn unescape_string(s: &str) -> Text {
    // Remove surrounding quotes if present
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);

    s.replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\r", "\r")
        .replace("\\\\", "\\")
        .into()
}

/// Validate performance specification format.
/// Valid formats: "< 15ns", "<= 100us", "< 1ms", etc.
fn validate_performance_spec(s: &str) -> bool {
    static PERF_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^[<>]=?\s*\d+(?:\.\d+)?\s*(?:ns|us|ms|s)$").unwrap());

    PERF_RE.is_match(s.trim())
}

/// Test discovery: find all test files in a directory or return a single file.
pub fn discover_tests(
    base_path: &Path,
    pattern: &str,
    exclude_patterns: &[&str],
) -> Result<List<Text>, DirectiveError> {
    let mut tests = List::new();

    // If base_path is a file, check if it matches the pattern and exclusions directly
    if base_path.is_file() {
        let path_str = base_path.to_string_lossy();

        // Check if file matches the pattern (e.g., "**/*.vr" should match any .vr file)
        let matches_pattern = if pattern.contains("*") {
            // Extract extension from pattern like "**/*.vr" -> ".vr"
            if let Some(ext) = pattern.rsplit('.').next() {
                base_path.extension().map(|e| e == ext).unwrap_or(false)
            } else {
                true // No extension in pattern, accept all
            }
        } else {
            // Exact pattern match
            glob::Pattern::new(pattern)
                .map(|pat| pat.matches(&path_str))
                .unwrap_or(true)
        };

        // Check exclusions
        let excluded = exclude_patterns.iter().any(|p| {
            path_str.contains(p)
                || glob::Pattern::new(p)
                    .map(|pat| pat.matches(&path_str))
                    .unwrap_or(false)
        });

        if matches_pattern && !excluded {
            tests.push(path_str.to_string().into());
        }

        return Ok(tests);
    }

    // For directories, use the existing glob logic
    let glob_pattern = base_path.join(pattern).to_string_lossy().to_string();

    for entry in glob::glob(&glob_pattern).map_err(|e| DirectiveError::ParseError {
        line: 0,
        message: format!("Invalid glob pattern: {}", e).into(),
    })? {
        let path = entry.map_err(|e| DirectiveError::IoError(e.into_error()))?;

        // Check exclusions
        let path_str = path.to_string_lossy();
        let excluded = exclude_patterns.iter().any(|p| {
            path_str.contains(p)
                || glob::Pattern::new(p)
                    .map(|pat| pat.matches(&path_str))
                    .unwrap_or(false)
        });

        if !excluded && path.is_file() {
            tests.push(path.to_string_lossy().to_string().into());
        }
    }

    Ok(tests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_test_type() {
        assert_eq!(
            TestType::from_str("parse-pass").unwrap(),
            TestType::ParsePass
        );
        assert_eq!(
            TestType::from_str("typecheck-fail").unwrap(),
            TestType::TypecheckFail
        );
        assert_eq!(TestType::from_str("run").unwrap(), TestType::Run);
        assert_eq!(
            TestType::from_str("differential").unwrap(),
            TestType::Differential
        );
    }

    #[test]
    fn test_parse_tier() {
        assert_eq!(Tier::from_str("0").unwrap(), Tier::Tier0);
        assert_eq!(Tier::from_str("3").unwrap(), Tier::Tier3);
    }

    #[test]
    fn test_parse_tiers() {
        let tiers = parse_tiers("all").unwrap();
        assert_eq!(tiers.len(), 4);

        let tiers = parse_tiers("0, 3").unwrap();
        assert_eq!(tiers.len(), 2);
        assert!(tiers.contains(&Tier::Tier0));
        assert!(tiers.contains(&Tier::Tier3));
    }

    #[test]
    fn test_parse_level() {
        assert_eq!(Level::from_str("L0").unwrap(), Level::L0);
        assert_eq!(Level::from_str("l2").unwrap(), Level::L2);
    }

    #[test]
    fn test_error_category() {
        assert_eq!(ErrorCategory::from_code("E001"), Some(ErrorCategory::Parse));
        assert_eq!(ErrorCategory::from_code("E123"), Some(ErrorCategory::Lexer));
        assert_eq!(ErrorCategory::from_code("E234"), Some(ErrorCategory::Type));
        assert_eq!(
            ErrorCategory::from_code("E345"),
            Some(ErrorCategory::Borrow)
        );
        assert_eq!(
            ErrorCategory::from_code("E456"),
            Some(ErrorCategory::Verification)
        );
        assert_eq!(
            ErrorCategory::from_code("E567"),
            Some(ErrorCategory::Context)
        );
        assert_eq!(
            ErrorCategory::from_code("E678"),
            Some(ErrorCategory::Module)
        );
        assert_eq!(ErrorCategory::from_code("E789"), Some(ErrorCategory::Async));
        assert_eq!(ErrorCategory::from_code("E890"), Some(ErrorCategory::Ffi));
        assert_eq!(
            ErrorCategory::from_code("E901"),
            Some(ErrorCategory::Internal)
        );
        assert_eq!(
            ErrorCategory::from_code("W302"),
            Some(ErrorCategory::Borrow)
        );
        assert_eq!(ErrorCategory::from_code("invalid"), None);
    }

    #[test]
    fn test_parse_expected_error() {
        let err = ExpectedError::parse(r#"E302 "Use after move" at line 8"#).unwrap();
        assert_eq!(err.code.as_str(), "E302");
        assert_eq!(err.message.as_ref().map(|t| t.as_str()), Some("Use after move"));
        assert_eq!(err.line, Some(8));
        assert_eq!(err.column, None);
        assert_eq!(err.category, Some(ErrorCategory::Borrow));

        let err = ExpectedError::parse(r#"E401 "Type mismatch" at line 15, col 10"#).unwrap();
        assert_eq!(err.code.as_str(), "E401");
        assert_eq!(err.line, Some(15));
        assert_eq!(err.column, Some(10));
    }

    #[test]
    fn test_parse_expected_error_compact_format() {
        // Compact line:column format
        let err = ExpectedError::parse(r#"E302 at 8:10"#).unwrap();
        assert_eq!(err.code.as_str(), "E302");
        assert_eq!(err.line, Some(8));
        assert_eq!(err.column, Some(10));

        // Compact with column range
        let err = ExpectedError::parse(r#"E302 at 8:10-15"#).unwrap();
        assert_eq!(err.code.as_str(), "E302");
        assert_eq!(err.line, Some(8));
        assert_eq!(err.column, Some(10));
        assert_eq!(err.end_column, Some(15));
    }

    #[test]
    fn test_parse_expected_error_with_column_range() {
        let err = ExpectedError::parse(r#"E302 "error" at line 5, col 10-15"#).unwrap();
        assert_eq!(err.code.as_str(), "E302");
        assert_eq!(err.line, Some(5));
        assert_eq!(err.column, Some(10));
        assert_eq!(err.end_column, Some(15));
    }

    #[test]
    fn test_parse_expected_error_with_severity() {
        let err = ExpectedError::parse(r#"[error] E302 "Use after move""#).unwrap();
        assert_eq!(err.code.as_str(), "E302");
        assert_eq!(err.severity.as_ref().map(|t| t.as_str()), Some("error"));
        assert_eq!(err.message.as_ref().map(|t| t.as_str()), Some("Use after move"));
    }

    #[test]
    fn test_expected_error_builder() {
        let err = ExpectedError::with_code("E302")
            .with_message("Use after move")
            .at_position(10, Some(5));

        assert_eq!(err.code.as_str(), "E302");
        assert_eq!(err.message.as_ref().map(|t| t.as_str()), Some("Use after move"));
        assert_eq!(err.line, Some(10));
        assert_eq!(err.column, Some(5));
    }

    #[test]
    fn test_expected_error_matches() {
        let err = ExpectedError {
            code: "E302".to_string().into(),
            message: Some("Use after move".to_string().into()),
            line: Some(10),
            column: Some(5),
            end_column: None,
            severity: None,
            category: Some(ErrorCategory::Borrow),
        };

        // Exact match
        assert!(err.matches("E302", Some("Use after move"), Some(10), Some(5)));

        // Message substring
        assert!(err.matches(
            "E302",
            Some("Error: Use after move in this context"),
            Some(10),
            Some(5)
        ));

        // Wrong code
        assert!(!err.matches("E303", Some("Use after move"), Some(10), Some(5)));

        // Wrong line
        assert!(!err.matches("E302", Some("Use after move"), Some(11), Some(5)));

        // Wrong column
        assert!(!err.matches("E302", Some("Use after move"), Some(10), Some(6)));
    }

    #[test]
    fn test_expected_error_matches_column_range() {
        let err = ExpectedError {
            code: "E302".to_string().into(),
            message: None,
            line: Some(10),
            column: Some(5),
            end_column: Some(10),
            severity: None,
            category: Some(ErrorCategory::Borrow),
        };

        // Within range
        assert!(err.matches("E302", None, Some(10), Some(7)));
        assert!(err.matches("E302", None, Some(10), Some(5)));
        assert!(err.matches("E302", None, Some(10), Some(10)));

        // Outside range
        assert!(!err.matches("E302", None, Some(10), Some(4)));
        assert!(!err.matches("E302", None, Some(10), Some(11)));
    }

    #[test]
    fn test_expected_error_matches_stderr() {
        let err = ExpectedError {
            code: "E302".to_string().into(),
            message: Some("Use after move".to_string().into()),
            line: Some(10),
            column: None,
            end_column: None,
            severity: None,
            category: Some(ErrorCategory::Borrow),
        };

        let stderr = "error[E302]: Use after move\n  --> test.vr:10:5\n   |\n10 |     let x = y;\n   |         ^ value moved here";
        assert!(err.matches_stderr(stderr));

        let wrong_stderr = "error[E303]: Different error\n  --> test.vr:10:5";
        assert!(!err.matches_stderr(wrong_stderr));
    }

    #[test]
    fn test_parse_directives() {
        let content = r#"
// @test: typecheck-fail
// @expected-error: E302 "Use after move" at line 11
// @tier: all
// @level: L0
// @tags: ownership, cbgr, move-semantics

fn main() {
    let s1 = Text.from("hello");
    let s2 = s1;
    println(s1);
}
"#;

        let directives = TestDirectives::parse(content, "test.vr".to_string().into()).unwrap();
        assert_eq!(directives.test_type, TestType::TypecheckFail);
        assert_eq!(directives.level, Level::L0);
        assert_eq!(directives.tiers.len(), 4);
        assert!(directives.tags.contains(&"ownership".to_string().into()));
        assert!(directives.tags.contains(&"cbgr".to_string().into()));
        assert_eq!(directives.expected_errors.len(), 1);
        assert_eq!(directives.expected_errors[0].code.as_str(), "E302");
    }

    #[test]
    fn test_parse_run_directives() {
        let content = r#"
// @test: run
// @expected-stdout: 55
// @tier: 0, 3
// @level: L1
// @tags: recursion, fib
// @timeout: 5000

fn fib(n: Int) -> Int {
    if n <= 1 { n }
    else { fib(n - 1) + fib(n - 2) }
}

fn main() {
    print(fib(10));
}
"#;

        let directives = TestDirectives::parse(content, "fib.vr".to_string().into()).unwrap();
        assert_eq!(directives.test_type, TestType::Run);
        assert_eq!(directives.expected_stdout.as_ref().map(|t| t.as_str()), Some("55"));
        assert_eq!(directives.timeout_ms, Some(5000));
        assert_eq!(directives.tiers.len(), 2);
    }

    #[test]
    fn test_parse_multiline_stdout() {
        let content = r#"
// @test: run
// @level: L1
// @expected-stdout-begin
// Line 1
// Line 2
// Line 3
// @expected-stdout-end

fn main() {
    println("Line 1");
    println("Line 2");
    println("Line 3");
}
"#;

        let directives = TestDirectives::parse(content, "multi.vr".to_string().into()).unwrap();
        assert_eq!(
            directives.expected_stdout.as_ref().map(|t| t.as_str()),
            Some("Line 1\nLine 2\nLine 3")
        );
    }

    #[test]
    fn test_unclosed_block_error() {
        let content = r#"
// @test: run
// @level: L1
// @expected-stdout-begin
// Line 1

fn main() {}
"#;

        let result = TestDirectives::parse(content, "test.vr".to_string().into());
        assert!(matches!(result, Err(DirectiveError::UnclosedBlock { .. })));
    }

    #[test]
    fn test_unexpected_block_end_error() {
        let content = r#"
// @test: run
// @level: L1
// @expected-stdout-end

fn main() {}
"#;

        let result = TestDirectives::parse(content, "test.vr".to_string().into());
        assert!(matches!(
            result,
            Err(DirectiveError::UnexpectedBlockEnd { .. })
        ));
    }

    #[test]
    fn test_parse_skip_directive() {
        let content = r#"
// @test: run
// @level: L1
// @skip: Not implemented yet

fn main() {}
"#;

        let directives = TestDirectives::parse(content, "test.vr".to_string().into()).unwrap();
        assert_eq!(directives.skip.as_ref().map(|t| t.as_str()), Some("Not implemented yet"));
    }

    #[test]
    fn test_parse_requires_directive() {
        let content = r#"
// @test: run
// @level: L1
// @requires: gpu, ffi

fn main() {}
"#;

        let directives = TestDirectives::parse(content, "test.vr".to_string().into()).unwrap();
        assert!(directives.requires.contains(&"gpu".to_string().into()));
        assert!(directives.requires.contains(&"ffi".to_string().into()));
    }

    #[test]
    fn test_parse_timeout_with_units() {
        assert_eq!(parse_timeout("5000").unwrap(), 5000);
        assert_eq!(parse_timeout("5000ms").unwrap(), 5000);
        assert_eq!(parse_timeout("5s").unwrap(), 5000);
    }

    #[test]
    fn test_unescape_string() {
        assert_eq!(unescape_string("hello\\nworld"), "hello\nworld");
        assert_eq!(unescape_string("tab\\there"), "tab\there");
        assert_eq!(unescape_string("\"quoted\""), "quoted");
    }

    #[test]
    fn test_validate_performance_spec() {
        assert!(validate_performance_spec("< 15ns"));
        assert!(validate_performance_spec("<= 100us"));
        assert!(validate_performance_spec("< 1ms"));
        assert!(validate_performance_spec("> 5s"));
        assert!(validate_performance_spec("< 1.5ms"));
        assert!(!validate_performance_spec("invalid"));
        assert!(!validate_performance_spec("15"));
    }

    #[test]
    fn test_conflicting_directives_error() {
        let content = r#"
// @test: run
// @level: L1
// @expected-stdout: hello
// @expected-stdout-file: output.txt

fn main() {}
"#;

        let result = TestDirectives::parse(content, "test.vr".to_string().into());
        assert!(matches!(
            result,
            Err(DirectiveError::ConflictingDirectives(_))
        ));
    }

    #[test]
    fn test_run_panic_requires_expected_panic() {
        let content = r#"
// @test: run-panic
// @level: L1

fn main() { panic("oops"); }
"#;

        let result = TestDirectives::parse(content, "test.vr".to_string().into());
        assert!(matches!(result, Err(DirectiveError::ValidationError(_))));
    }
}
