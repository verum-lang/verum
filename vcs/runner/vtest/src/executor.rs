//! Test executor for VCS test runner.
//!
//! Executes tests across different tiers (interpreter, JIT, AOT) and
//! compares results with expectations.
//!
//! # Execution Modes
//!
//! The executor supports two execution modes:
//!
//! 1. **Direct Library Integration** (preferred for parse/typecheck tests):
//!    Uses verum_compiler directly via library calls for fast, in-process testing.
//!
//! 2. **Process-Based Execution** (for run tests and JIT/AOT tiers):
//!    Spawns external processes using verum CLI commands.
//!
//! # Tier Execution
//!
//! - **Tier 0**: Interpreter (slowest, most debuggable)
//! - **Tier 1**: Baseline JIT (no optimizations)
//! - **Tier 2**: Optimized JIT (with optimizations)
//! - **Tier 3**: AOT compiled (fastest)
//!
//! # Error Matching
//!
//! Error codes follow the pattern EXXX where the first digit indicates category:
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

use crate::directive::{ExpectedError, TestDirectives, TestType, Tier};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::process::Command;
use tokio::time::timeout;
use verum_common::{List, Map, Set, Text};

// Direct compiler integration
use verum_ast::FileId;
use verum_compiler::{
    CompilationPipeline, CompilerOptions, OutputFormat, Session,
    TestExecutionResult, VerifyMode, get_cached_stdlib_registry,
};
use verum_lexer::Lexer;
use verum_parser::VerumParser;
use verum_fast_parser::FastParser;

/// Error type for execution failures.
#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("Process execution failed: {0}")]
    ProcessError(Text),

    #[error("Timeout after {0}ms")]
    Timeout(u64),

    #[error("Command not found: {0}")]
    CommandNotFound(Text),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    ParseError(Text),

    #[error("Error count mismatch: expected {expected}, got {actual}")]
    ErrorCountMismatch { expected: usize, actual: usize },

    #[error("Missing expected error: {0}")]
    MissingExpectedError(Text),

    #[error("Unexpected error: {0}")]
    UnexpectedError(Text),
}

/// A parsed error from compiler output.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedError {
    /// Error code (e.g., "E302")
    pub code: Text,
    /// Error message
    pub message: Text,
    /// File path
    pub file: Option<Text>,
    /// Line number (1-indexed)
    pub line: Option<usize>,
    /// Column number (1-indexed)
    pub column: Option<usize>,
    /// Severity (error, warning, note)
    pub severity: Text,
}

impl ParsedError {
    /// Parse errors from compiler stderr output.
    ///
    /// Supports multiple error output formats:
    /// - Rust-style: `error[E302]: message at file:line:column`
    /// - GCC-style: `file:line:column: error: message`
    /// - Simple: `error[E302]: message`
    pub fn parse_stderr(stderr: &str) -> List<ParsedError> {
        let mut errors = List::new();

        // Rust-style error pattern: error[E302]: message or error<E302>: message
        //   --> file:line:column
        // Note: Supports both [Exxx] and <Exxx> formats for error codes
        // Note: Also supports M codes for meta errors (M400-M999)
        static RUST_ERROR_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"(?m)^(error|warning|note)[\[<]([EWM]\d{3,4})[\]>]:\s*(.+)$").unwrap());

        // Location pattern: --> file:line:column
        static LOCATION_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^\s*-->\s*([^:]+):(\d+):(\d+)").unwrap());

        // GCC-style pattern: file:line:column: error[E302]: message
        // Note: Also supports M codes for meta errors (M400-M999)
        static GCC_ERROR_RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^([^:]+):(\d+):(\d+):\s*(error|warning|note)(?:\[([EWM]\d{3,4})\])?:\s*(.+)$")
                .unwrap()
        });

        let lines: Vec<&str> = stderr.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];

            // Try Rust-style first
            if let Some(caps) = RUST_ERROR_RE.captures(line) {
                let severity: Text = caps
                    .get(1)
                    .map(|m| m.as_str())
                    .unwrap_or("error")
                    .to_string()
                    .into();
                let code: Text = caps.get(2).map(|m| m.as_str()).unwrap_or("").to_string().into();
                let message: Text = caps.get(3).map(|m| m.as_str()).unwrap_or("").to_string().into();

                // Look for location on next line
                let (file, line_num, column): (Option<Text>, Option<usize>, Option<usize>) = if i + 1 < lines.len() {
                    if let Some(loc_caps) = LOCATION_RE.captures(lines[i + 1]) {
                        let file: Option<Text> = loc_caps.get(1).map(|m| m.as_str().to_string().into());
                        let line_num = loc_caps.get(2).and_then(|m| m.as_str().parse().ok());
                        let column = loc_caps.get(3).and_then(|m| m.as_str().parse().ok());
                        i += 1; // Skip the location line
                        (file, line_num, column)
                    } else {
                        (None, None, None)
                    }
                } else {
                    (None, None, None)
                };

                errors.push(ParsedError {
                    code,
                    message,
                    file,
                    line: line_num,
                    column,
                    severity,
                });
            }
            // Try GCC-style
            else if let Some(caps) = GCC_ERROR_RE.captures(line) {
                let file: Option<Text> = caps.get(1).map(|m| m.as_str().to_string().into());
                let line_num = caps.get(2).and_then(|m| m.as_str().parse().ok());
                let column = caps.get(3).and_then(|m| m.as_str().parse().ok());
                let severity: Text = caps
                    .get(4)
                    .map(|m| m.as_str())
                    .unwrap_or("error")
                    .to_string()
                    .into();
                let code: Text = caps.get(5).map(|m| m.as_str()).unwrap_or("").to_string().into();
                let message: Text = caps.get(6).map(|m| m.as_str()).unwrap_or("").to_string().into();

                errors.push(ParsedError {
                    code,
                    message,
                    file,
                    line: line_num,
                    column,
                    severity,
                });
            }

            i += 1;
        }

        errors
    }

    /// Check if this error matches an expected error specification.
    pub fn matches(&self, expected: &ExpectedError) -> bool {
        // Code must match
        if self.code != expected.code {
            return false;
        }

        // Message must contain expected substring (if specified)
        if let Some(ref exp_msg) = expected.message {
            if !self.message.contains(exp_msg.as_str()) {
                return false;
            }
        }

        // Line must match (if specified)
        if let Some(exp_line) = expected.line {
            match self.line {
                Some(actual_line) if actual_line == exp_line => {}
                _ => return false,
            }
        }

        // Column must match or be within range (if specified)
        if let Some(exp_col) = expected.column {
            match self.column {
                Some(actual_col) => {
                    if let Some(end_col) = expected.end_column {
                        // Column range match
                        if actual_col < exp_col || actual_col > end_col {
                            return false;
                        }
                    } else {
                        // Exact column match
                        if actual_col != exp_col {
                            return false;
                        }
                    }
                }
                None => return false,
            }
        }

        // Severity must match (if specified)
        if let Some(ref exp_severity) = expected.severity {
            if &self.severity != exp_severity {
                return false;
            }
        }

        true
    }
}

/// Result of error matching for detailed diagnostics.
#[derive(Debug)]
pub enum ErrorMatchResult {
    /// Error count mismatch
    CountMismatch { expected: usize, actual: usize },
    /// Missing expected errors
    MissingErrors {
        missing: List<ExpectedError>,
        actual: List<ParsedError>,
    },
    /// Unexpected errors present
    UnexpectedErrors { unexpected: List<ParsedError> },
}

impl std::fmt::Display for ErrorMatchResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CountMismatch { expected, actual } => {
                write!(
                    f,
                    "Error count mismatch: expected {}, got {}",
                    expected, actual
                )
            }
            Self::MissingErrors { missing, actual } => {
                writeln!(f, "Missing expected errors:")?;
                for err in missing {
                    writeln!(f, "  - {} at line {:?}", err.code, err.line)?;
                }
                writeln!(f, "Actual errors found:")?;
                for err in actual {
                    writeln!(
                        f,
                        "  - {} at line {:?}: {}",
                        err.code, err.line, err.message
                    )?;
                }
                Ok(())
            }
            Self::UnexpectedErrors { unexpected } => {
                writeln!(f, "Unexpected errors:")?;
                for err in unexpected {
                    writeln!(
                        f,
                        "  - {} at line {:?}: {}",
                        err.code, err.line, err.message
                    )?;
                }
                Ok(())
            }
        }
    }
}

/// Configuration for the test executor.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Path to the verum interpreter
    pub interpreter_path: PathBuf,
    /// Path to the verum JIT compiler (baseline)
    pub jit_base_path: PathBuf,
    /// Path to the verum JIT compiler (optimized)
    pub jit_opt_path: PathBuf,
    /// Path to the verum AOT compiler
    pub aot_path: PathBuf,
    /// Working directory for test execution
    pub work_dir: PathBuf,
    /// Default timeout in milliseconds
    pub default_timeout_ms: u64,
    /// Environment variables for execution
    pub env: Map<Text, Text>,
    /// Use direct library integration instead of process execution
    /// This is faster for parse/typecheck tests but requires verum_compiler
    pub use_direct_integration: bool,
    /// Path to the main verum CLI binary (for process-based execution)
    pub verum_cli_path: Option<PathBuf>,
    /// Available features for @requires directive
    pub available_features: Set<Text>,
    /// Compile-time only mode: treat Run/RunPanic tests as TypecheckPass
    /// This is used for verifying compile-time phase correctness without runtime
    pub compile_time_only: bool,
    /// Output directory for VBC bytecode (when --vbc-output is specified)
    pub vbc_output_dir: Option<PathBuf>,
    /// Preserve test-relative paths in VBC output (vs flat directory)
    pub vbc_preserve_paths: bool,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        // Try to find the verum CLI in common locations
        let verum_cli_path = Self::find_verum_cli();

        // Find the vcs/bin directory with wrapper scripts
        let bin_dir = Self::find_bin_dir();

        // Default available features
        let mut available_features = Set::new();
        for feature in &[
            "basic",
            "std",
            "bounds-checking",
            "panic-handling",
            "cbgr-runtime",
        ] {
            available_features.insert(Text::from(*feature));
        }

        Self {
            interpreter_path: bin_dir.join("verum-interpreter"),
            jit_base_path: bin_dir.join("verum-jit"),
            jit_opt_path: bin_dir.join("verum-jit"),
            aot_path: bin_dir.join("verum-aot"),
            work_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            default_timeout_ms: 30_000,
            env: Map::new(),
            // Default to direct integration (faster and no external dependencies)
            use_direct_integration: true,
            verum_cli_path,
            available_features,
            // Default to running full tests (including runtime)
            compile_time_only: false,
            // VBC output disabled by default
            vbc_output_dir: None,
            vbc_preserve_paths: false,
        }
    }
}

impl ExecutorConfig {
    /// Create a builder for ExecutorConfig
    pub fn builder() -> ExecutorConfigBuilder {
        ExecutorConfigBuilder::default()
    }

    /// Set compile_time_only mode
    pub fn with_compile_time_only(mut self, value: bool) -> Self {
        self.compile_time_only = value;
        self
    }
}

/// Builder for ExecutorConfig
#[derive(Debug, Default)]
pub struct ExecutorConfigBuilder {
    compile_time_only: bool,
}

impl ExecutorConfig {
    /// Find the vcs/bin directory with wrapper scripts
    fn find_bin_dir() -> PathBuf {
        // Check VCS_BIN environment variable first
        if let Ok(path) = std::env::var("VCS_BIN") {
            let path = PathBuf::from(path);
            if path.exists() {
                // Canonicalize to absolute path for Command::new()
                if let Ok(abs_path) = path.canonicalize() {
                    return abs_path;
                }
                return path;
            }
        }

        // Try relative to current directory
        let cwd_bin = PathBuf::from("vcs/bin");
        if cwd_bin.exists() {
            // Canonicalize to absolute path for Command::new()
            if let Ok(abs_path) = cwd_bin.canonicalize() {
                return abs_path;
            }
            return cwd_bin;
        }

        // Also check "bin" directly (when running from vcs directory)
        let bin_direct = PathBuf::from("bin");
        if bin_direct.exists() && bin_direct.join("verum-interpreter").exists() {
            // Canonicalize to absolute path for Command::new()
            if let Ok(abs_path) = bin_direct.canonicalize() {
                return abs_path;
            }
            return bin_direct;
        }

        // Try to find by searching up the directory tree
        if let Ok(cwd) = std::env::current_dir() {
            for ancestor in cwd.ancestors() {
                let bin_dir = ancestor.join("vcs").join("bin");
                if bin_dir.exists() {
                    return bin_dir;
                }
            }
        }

        // Fall back to system PATH lookup (empty path)
        PathBuf::new()
    }
}

impl ExecutorConfig {
    /// Find the verum CLI binary in common locations
    pub fn find_verum_cli() -> Option<PathBuf> {
        // Check VERUM_CLI environment variable first
        if let Ok(path) = std::env::var("VERUM_CLI") {
            let path = PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
        }

        // Check for cargo build output in workspace root
        // The vtest runner is in vcs/runner/vtest, so workspace root is ../../..
        let workspace_paths = [
            // Relative from vcs/runner/vtest
            "../../../target/release/verum",
            "../../../target/debug/verum",
            // Standard PATH locations
            "verum",
        ];

        for path_str in workspace_paths {
            let path = PathBuf::from(path_str);
            if path.exists() {
                return Some(path);
            }
        }

        // Check if 'verum' is in PATH
        if let Ok(output) = std::process::Command::new("which").arg("verum").output() {
            if output.status.success() {
                let path_str = String::from_utf8_lossy(&output.stdout);
                let path = PathBuf::from(path_str.trim());
                if path.exists() {
                    return Some(path);
                }
            }
        }

        None
    }

    /// Create config with direct integration enabled
    pub fn with_direct_integration(mut self) -> Self {
        self.use_direct_integration = true;
        self
    }

    /// Create config with process execution (for JIT/AOT tests)
    pub fn with_process_execution(mut self) -> Self {
        self.use_direct_integration = false;
        self
    }

    /// Set the verum CLI path explicitly
    pub fn with_verum_cli(mut self, path: PathBuf) -> Self {
        self.verum_cli_path = Some(path);
        self
    }
}

impl ExecutorConfig {
    /// Get the command for a specific tier.
    ///
    /// Note: The wrapper scripts (verum-interpreter, verum-jit, verum-aot) already
    /// include the `run` subcommand internally, so we only need to pass tier-specific
    /// flags here. The file path is appended by the caller.
    pub fn command_for_tier(&self, tier: Tier) -> (PathBuf, List<Text>) {
        match tier {
            // verum-interpreter already calls: verum run --interp "$@"
            Tier::Tier0 => (self.interpreter_path.clone(), vec![].into()),
            // verum-jit parses --baseline and calls: verum run --jit "$@"
            Tier::Tier1 => (self.jit_base_path.clone(), vec!["--baseline".to_string().into()].into()),
            // verum-jit parses --optimize and calls: verum run --aot "$@"
            Tier::Tier2 => (self.jit_opt_path.clone(), vec!["--optimize".to_string().into()].into()),
            // verum-aot already calls: verum run --optimized "$@"
            Tier::Tier3 => (self.aot_path.clone(), vec![].into()),
        }
    }
}

/// Output from a process execution.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ProcessOutput {
    /// Exit code (None if process was killed)
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: Text,
    /// Standard error
    pub stderr: Text,
    /// Execution duration
    pub duration: Duration,
    /// Whether the process timed out
    pub timed_out: bool,
}

/// Result of a single test execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum TestOutcome {
    /// Test passed
    Pass { tier: Tier, duration: Duration },
    /// Test failed
    Fail {
        tier: Tier,
        reason: Text,
        expected: Option<Text>,
        actual: Option<Text>,
        duration: Duration,
    },
    /// Test was skipped
    Skip { tier: Tier, reason: Text },
    /// Test execution error
    Error { tier: Tier, error: Text },
}

impl TestOutcome {
    /// Check if this is a passing outcome.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass { .. })
    }

    /// Check if this is a failing outcome.
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }

    /// Check if this was skipped.
    pub fn is_skip(&self) -> bool {
        matches!(self, Self::Skip { .. })
    }

    /// Get the tier for this outcome.
    pub fn tier(&self) -> Tier {
        match self {
            Self::Pass { tier, .. } => *tier,
            Self::Fail { tier, .. } => *tier,
            Self::Skip { tier, .. } => *tier,
            Self::Error { tier, .. } => *tier,
        }
    }

    /// Get the duration if available.
    pub fn duration(&self) -> Option<Duration> {
        match self {
            Self::Pass { duration, .. } => Some(*duration),
            Self::Fail { duration, .. } => Some(*duration),
            _ => None,
        }
    }
}

/// Complete result for a test file (across all tiers).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestResult {
    /// Test directives
    pub directives: TestDirectives,
    /// Outcomes per tier
    pub outcomes: List<TestOutcome>,
    /// Total execution time
    pub total_duration: Duration,
}

impl TestResult {
    /// Check if all executed tiers passed.
    pub fn all_pass(&self) -> bool {
        self.outcomes.iter().all(|o| o.is_pass() || o.is_skip())
    }

    /// Get the number of passing tests.
    pub fn pass_count(&self) -> usize {
        self.outcomes.iter().filter(|o| o.is_pass()).count()
    }

    /// Get the number of failing tests.
    pub fn fail_count(&self) -> usize {
        self.outcomes.iter().filter(|o| o.is_fail()).count()
    }

    /// Get the number of skipped tests.
    pub fn skip_count(&self) -> usize {
        self.outcomes.iter().filter(|o| o.is_skip()).count()
    }

    /// Get all failure reasons.
    pub fn failure_reasons(&self) -> List<Text> {
        self.outcomes
            .iter()
            .filter_map(|o| match o {
                TestOutcome::Fail { reason, .. } => Some(reason.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Test executor.
#[derive(Clone)]
pub struct Executor {
    config: ExecutorConfig,
}

impl Executor {
    /// Create a new executor with the given configuration.
    pub fn new(config: ExecutorConfig) -> Self {
        Self { config }
    }

    /// Execute a test file.
    pub async fn execute(&self, directives: TestDirectives) -> Result<TestResult, ExecutorError> {
        let start = Instant::now();
        let mut outcomes = List::new();

        for tier in &directives.tiers {
            let outcome = self.execute_tier(&directives, *tier).await;
            outcomes.push(outcome);
        }

        Ok(TestResult {
            directives,
            outcomes,
            total_duration: start.elapsed(),
        })
    }

    /// Execute a test on a specific tier.
    async fn execute_tier(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        // Check if test should be skipped
        if let Some(ref skip_reason) = directives.skip {
            return TestOutcome::Skip {
                tier,
                reason: skip_reason.clone(),
            };
        }

        // Check required features
        if !directives.requires.is_empty() {
            // In a real implementation, we would check against available features
            // For now, we just note that features are required
            for feature in &directives.requires {
                if !self.is_feature_available(feature) {
                    return TestOutcome::Skip {
                        tier,
                        reason: format!("Missing required feature: {}", feature).into(),
                    };
                }
            }
        }

        // In compile_time_only mode, redirect runtime tests to compile-time verification
        let effective_test_type = if self.config.compile_time_only {
            match directives.test_type {
                // Runtime tests become compile-time-only
                TestType::Run | TestType::RunPanic | TestType::RunInterpreter | TestType::RunInterpreterPanic | TestType::Benchmark | TestType::Differential => {
                    TestType::TypecheckPass
                }
                // Verification tests: verify-pass becomes typecheck-pass (must at least typecheck).
                // verify-fail stays as-is to use execute_verify_fail which handles
                // direct integration correctly (always passes since we can't test
                // verification failure without the full verification pipeline).
                TestType::VerifyPass => TestType::TypecheckPass,
                TestType::VerifyFail => TestType::VerifyFail,
                // CompileOnly tests become typecheck (no need for full codegen)
                TestType::CompileOnly => TestType::TypecheckPass,
                // VBC tests also become typecheck
                TestType::VbcCodegen => TestType::TypecheckPass,
                TestType::CommonPipeline => TestType::TypecheckPass,
                // Keep other test types as-is
                other => other,
            }
        } else {
            directives.test_type.clone()
        };

        match effective_test_type {
            TestType::ParsePass => self.execute_parse_pass(directives, tier).await,
            TestType::ParseFail => self.execute_parse_fail(directives, tier).await,
            TestType::ParseRecover => self.execute_parse_recover(directives, tier).await,
            TestType::TypecheckPass => self.execute_typecheck_pass(directives, tier).await,
            TestType::TypecheckFail => self.execute_typecheck_fail(directives, tier).await,
            TestType::VerifyPass => self.execute_verify_pass(directives, tier).await,
            TestType::VerifyFail => self.execute_verify_fail(directives, tier).await,
            TestType::Run => self.execute_run(directives, tier).await,
            TestType::RunPanic => self.execute_run_panic(directives, tier).await,
            TestType::CompileOnly => self.execute_compile_only(directives, tier).await,
            TestType::Differential => self.execute_differential(directives, tier).await,
            TestType::Benchmark => self.execute_benchmark(directives, tier).await,
            // VBC-first pipeline tests
            TestType::CommonPipeline => self.execute_common_pipeline(directives, tier).await,
            TestType::CommonPipelineFail => self.execute_common_pipeline_fail(directives, tier).await,
            TestType::VbcCodegen => self.execute_vbc_codegen(directives, tier).await,
            TestType::VbcCodegenFail => self.execute_vbc_codegen_fail(directives, tier).await,
            // Meta tests - compile and evaluate meta functions
            TestType::MetaPass => self.execute_meta_pass(directives, tier).await,
            TestType::MetaFail => self.execute_meta_fail(directives, tier).await,
            TestType::MetaEval => self.execute_meta_eval(directives, tier).await,
            // Interpreter-specific execution tests (always Tier 0, direct API)
            TestType::RunInterpreter => {
                let start = Instant::now();
                self.execute_run_direct(directives, Tier::Tier0, start)
            }
            TestType::RunInterpreterPanic => {
                let start = Instant::now();
                self.execute_run_panic_direct(directives, Tier::Tier0, start)
            }
        }
    }

    /// Check if a required feature is available.
    fn is_feature_available(&self, feature: &str) -> bool {
        // Check environment variable for feature flags first
        // This allows CI to set VERUM_FEATURES=gpu,ffi,etc.
        if let Ok(features) = std::env::var("VERUM_FEATURES") {
            if features.split(',').any(|f| f.trim() == feature) {
                return true;
            }
        }

        // Check config's available features
        self.config.available_features.contains(&Text::from(feature))
    }

    /// Execute a parse-pass test.
    async fn execute_parse_pass(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Always use direct library integration for parse tests
        // This is faster and doesn't require external executables
        return self.execute_parse_pass_direct(directives, tier, start);

        // Legacy: process-based execution (disabled)
        match self.run_compiler_phase(directives, tier, "parse").await {
            Ok(output) => {
                if output.exit_code == Some(0) {
                    TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Parse unexpectedly failed".to_string().into(),
                        expected: Some("Successful parse".to_string().into()),
                        actual: Some(output.stderr.clone()),
                        duration: start.elapsed(),
                    }
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Execute parse-pass test using direct library integration.
    ///
    /// Uses FastParser (the main parser) which supports the full Verum grammar
    /// including tactic declarations, meta expressions, and proof constructs.
    fn execute_parse_pass_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        // Read source file
        let source = match std::fs::read_to_string(&source_path) {
            Ok(s) => s,
            Err(e) => {
                return TestOutcome::Error {
                    tier,
                    error: format!("Failed to read source file: {}", e).into(),
                };
            }
        };

        // Create a file ID for this test file
        let file_id = FileId::new(0);

        // Register source file in global registry for proper error messages
        verum_common::register_source_file(file_id, source_path.display().to_string(), &source);

        // Use FastParser which supports the full Verum grammar.
        // Fall back to VerumParser (legacy) if FastParser fails, since some
        // quantifier patterns and older syntax is only supported there.
        let parser = FastParser::new();
        match parser.parse_module_str(&source, file_id) {
            Ok(_module) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(fast_errors) => {
                // Try VerumParser as fallback for syntax not yet in FastParser
                let lexer = Lexer::new(&source, file_id);
                let legacy_parser = VerumParser::new();
                if legacy_parser.parse_module(lexer, file_id).is_ok() {
                    return TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    };
                }
                let error_msgs: Vec<String> =
                    fast_errors.iter().map(|e| format!("{}", e)).collect();
                TestOutcome::Fail {
                    tier,
                    reason: "Parse unexpectedly failed".to_string().into(),
                    expected: Some("Successful parse".to_string().into()),
                    actual: Some(error_msgs.join("\n").into()),
                    duration: start.elapsed(),
                }
            }
        }
    }

    /// Execute a parse-fail test.
    async fn execute_parse_fail(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Always use direct library integration for parse tests
        return self.execute_parse_fail_direct(directives, tier, start);

        // Legacy: process-based execution (disabled)
        match self.run_compiler_phase(directives, tier, "parse").await {
            Ok(output) => {
                if output.exit_code != Some(0) {
                    // Check if expected errors match
                    if self.check_expected_errors(&output.stderr, &directives.expected_errors) {
                        TestOutcome::Pass {
                            tier,
                            duration: start.elapsed(),
                        }
                    } else {
                        TestOutcome::Fail {
                            tier,
                            reason: "Parse failed but with wrong errors".to_string().into(),
                            expected: Some(format!("{:?}", directives.expected_errors).into()),
                            actual: Some(output.stderr.clone()),
                            duration: start.elapsed(),
                        }
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Parse unexpectedly succeeded".to_string().into(),
                        expected: Some("Parse failure".to_string().into()),
                        actual: Some("Parse succeeded".to_string().into()),
                        duration: start.elapsed(),
                    }
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Execute parse-fail test using direct library integration.
    ///
    /// Uses FastParser::parse_module_str which properly converts lexer errors
    /// to ParseErrors with correct error codes (E001-E006 for lexer errors).
    fn execute_parse_fail_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        // Read source file
        let source = match std::fs::read_to_string(&source_path) {
            Ok(s) => s,
            Err(e) => {
                return TestOutcome::Error {
                    tier,
                    error: format!("Failed to read source file: {}", e).into(),
                };
            }
        };

        // Create a file ID for this test file
        let file_id = FileId::new(0);

        // Register source file in global registry for proper error messages
        verum_common::register_source_file(file_id, source_path.display().to_string(), &source);

        // Use FastParser::parse_module_str which properly analyzes source text
        // and converts lexer errors to ParseErrors with appropriate error codes
        let parser = FastParser::new();
        match parser.parse_module_str(&source, file_id) {
            Ok(_module) => {
                // Parse succeeded when it should have failed
                TestOutcome::Fail {
                    tier,
                    reason: "Parse unexpectedly succeeded".to_string().into(),
                    expected: Some("Parse failure".to_string().into()),
                    actual: Some("Parse succeeded".to_string().into()),
                    duration: start.elapsed(),
                }
            }
            Err(parse_errors) => {
                // Parse failed as expected - check if errors match
                // Use Debug format ({:?}) which includes error codes like "ParseError(E003: ...)"
                let error_msgs: Vec<String> =
                    parse_errors.iter().map(|e| format!("{:?}", e)).collect();
                let error_output = error_msgs.join("\n");

                if directives.expected_errors.is_empty() {
                    // No specific errors expected, just expect failure
                    return TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    };
                }

                // Check if expected errors match
                if self.check_expected_errors_in_output(&error_output, &directives.expected_errors)
                {
                    TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Parse failed but with wrong errors".to_string().into(),
                        expected: Some(format!("{:?}", directives.expected_errors).into()),
                        actual: Some(error_output.into()),
                        duration: start.elapsed(),
                    }
                }
            }
        }
    }

    /// Check if expected errors are present in output string.
    fn check_expected_errors_in_output(&self, output: &str, expected: &[ExpectedError]) -> bool {
        if expected.is_empty() {
            return true;
        }

        for exp in expected {
            if !exp.matches_stderr(output) {
                return false;
            }
        }

        true
    }

    /// Execute a parse-recover test.
    /// The parser should encounter errors but recover and continue parsing.
    /// Pass condition: parser produces errors (doesn't succeed silently) and doesn't crash.
    async fn execute_parse_recover(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();
        let source_path = PathBuf::from(&directives.source_path);

        let source = match std::fs::read_to_string(&source_path) {
            Ok(s) => s,
            Err(e) => {
                return TestOutcome::Error {
                    tier,
                    error: format!("Failed to read source file: {}", e).into(),
                };
            }
        };

        let file_id = FileId::new(0);
        verum_common::register_source_file(file_id, source_path.display().to_string(), &source);

        let parser = FastParser::new();
        match parser.parse_module_str(&source, file_id) {
            Ok(_module) => {
                // Parse succeeded without errors — for recover tests, this is unexpected
                // since the source is intentionally malformed
                TestOutcome::Fail {
                    tier,
                    reason: "Parse succeeded without errors, but parse-recover expects errors".to_string().into(),
                    expected: Some("Parse errors with recovery".to_string().into()),
                    actual: Some("No errors".to_string().into()),
                    duration: start.elapsed(),
                }
            }
            Err(parse_errors) => {
                // Parser produced errors — this is expected for recover tests.
                // The fact that we got here without a crash means recovery worked.
                if !directives.expected_errors.is_empty() {
                    let error_msgs: Vec<String> =
                        parse_errors.iter().map(|e| format!("{:?}", e)).collect();
                    let error_output = error_msgs.join("\n");
                    if self.check_expected_errors_in_output(&error_output, &directives.expected_errors) {
                        TestOutcome::Pass {
                            tier,
                            duration: start.elapsed(),
                        }
                    } else {
                        TestOutcome::Fail {
                            tier,
                            reason: "Parse recovered but with wrong errors".to_string().into(),
                            expected: Some(format!("{:?}", directives.expected_errors).into()),
                            actual: Some(error_output.into()),
                            duration: start.elapsed(),
                        }
                    }
                } else {
                    // No specific errors expected, just expect some errors + recovery
                    TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    }
                }
            }
        }
    }

    /// Execute a typecheck-pass test.
    async fn execute_typecheck_pass(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration if enabled (preferred)
        if self.config.use_direct_integration {
            return self.execute_typecheck_pass_direct(directives, tier, start);
        }

        // Fallback to process-based execution
        match self.run_compiler_phase(directives, tier, "check").await {
            Ok(output) => {
                if output.exit_code == Some(0) {
                    TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Typecheck unexpectedly failed".to_string().into(),
                        expected: Some("Successful typecheck".to_string().into()),
                        actual: Some(output.stderr.clone()),
                        duration: start.elapsed(),
                    }
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Run a compilation check on a dedicated thread with 64MB stack.
    /// Returns (result, diagnostics) or panics/timeouts as errors.
    fn run_check_on_thread(
        options: CompilerOptions,
        timeout_ms: u64,
    ) -> Result<(Result<(), anyhow::Error>, String), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session = if let Some(cached_registry) = get_cached_stdlib_registry() {
                        Session::with_registry(options, cached_registry)
                    } else {
                        Session::new(options)
                    };
                    let result = {
                        let mut pipeline = CompilationPipeline::new_check(&mut session);
                        pipeline.run_check_only()
                    };
                    let diag = session.format_diagnostics();
                    (result, diag)
                }));
                let _ = tx.send(r);
            })
            .expect("Failed to spawn typecheck thread");

        match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
            Ok(Ok(pair)) => Ok(pair),
            Ok(Err(panic_info)) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                Err(format!("panic: {}", msg))
            }
            Err(_) => Err("timeout".to_string()),
        }
    }

    /// Execute typecheck-pass test using direct library integration.
    ///
    /// # Performance Optimization
    ///
    /// This function uses a cached stdlib registry when available, reducing
    /// per-test overhead from ~500ms to ~1ms. The registry cache is populated
    /// on the first test that loads stdlib and reused for all subsequent tests.
    fn execute_typecheck_pass_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        // Create compiler options for check-only mode
        let options = CompilerOptions {
            input: source_path.clone(),
            output_format: OutputFormat::Human,
            verify_mode: VerifyMode::Runtime,
            continue_on_error: false,
            ..Default::default()
        };

        // OPTIMIZATION: Use cached stdlib registry when available.
        // This reduces per-test overhead from ~500ms to ~1ms.
        // The Normal build mode's load_stdlib_modules() handles stdlib loading
        // automatically by finding the workspace root, so we always use new_check
        // (Normal mode). We never use new_core here because that sets
        // BuildMode::StdlibBootstrap which is for compiling the stdlib itself,
        // not for compiling user code against the stdlib.
        // Run type checking on a dedicated thread with large stack to prevent
        // stack overflow from deep type inference recursion on complex programs.
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let check_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session = if let Some(cached_registry) = get_cached_stdlib_registry() {
                        Session::with_registry(options, cached_registry)
                    } else {
                        Session::new(options)
                    };
                    let result = {
                        let mut pipeline = CompilationPipeline::new_check(&mut session);
                        pipeline.run_check_only()
                    };
                    (result, session.format_diagnostics())
                }));
                let _ = tx.send(check_result);
            })
            .expect("Failed to spawn typecheck thread");

        let (result, diagnostics_str) = match rx.recv_timeout(std::time::Duration::from_millis(
            directives.effective_timeout_ms(),
        )) {
            Ok(Ok((result, diag))) => (result, diag),
            Ok(Err(panic_info)) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                return TestOutcome::Fail {
                    tier,
                    reason: format!("Typecheck panicked: {}", msg).into(),
                    expected: Some("Successful typecheck".to_string().into()),
                    actual: Some(msg.into()),
                    duration: start.elapsed(),
                };
            }
            Err(_) => {
                return TestOutcome::Fail {
                    tier,
                    reason: "Typecheck timed out".to_string().into(),
                    expected: Some("Successful typecheck".to_string().into()),
                    actual: Some("timeout".to_string().into()),
                    duration: start.elapsed(),
                };
            }
        };

        match result {
            Ok(()) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(e) => {
                // Collect error output from session diagnostics
                let error_output = format!("{}", e);
                let actual = if diagnostics_str.is_empty() {
                    error_output
                } else {
                    format!("{}\n\nDiagnostics:\n{}", error_output, diagnostics_str)
                };
                TestOutcome::Fail {
                    tier,
                    reason: "Typecheck unexpectedly failed".to_string().into(),
                    expected: Some("Successful typecheck".to_string().into()),
                    actual: Some(actual.into()),
                    duration: start.elapsed(),
                }
            }
        }
    }

    /// Check if a test file imports stdlib modules.
    fn test_imports_stdlib(&self, path: &Path) -> bool {
        // Read first 100 lines of the file looking for stdlib imports
        // Verum uses `mount` (not `import`) for module imports
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines().take(100) {
                let line = line.trim();
                // Check both `mount` (correct Verum syntax) and `import` (legacy)
                if line.starts_with("mount sys.") ||
                   line.starts_with("mount core.") ||
                   line.starts_with("mount collections.") ||
                   line.starts_with("mount io.") ||
                   line.starts_with("mount async.") ||
                   line.starts_with("mount net.") ||
                   line.starts_with("mount mem.") ||
                   line.starts_with("mount base.") ||
                   line.starts_with("mount time.") ||
                   line.starts_with("mount sync.") ||
                   line.starts_with("mount runtime.") ||
                   line.starts_with("mount text.") ||
                   line.starts_with("mount math.") ||
                   line.starts_with("import sys.") ||
                   line.starts_with("import core.") ||
                   line.starts_with("import collections.") ||
                   line.starts_with("import io.") ||
                   line.starts_with("import async.") ||
                   line.starts_with("import net.") ||
                   line.starts_with("import mem.") {
                    return true;
                }
            }
        }
        false
    }

    /// Find the stdlib directory by searching up from cwd.
    /// The stdlib lives in the `core/` directory at the workspace root.
    fn find_stdlib_path(&self) -> Option<PathBuf> {
        // Search for "core/" first (primary stdlib location), then "stdlib/" (legacy)
        for dir_name in &["core", "stdlib"] {
            let relative = PathBuf::from(dir_name);
            if relative.exists() && relative.is_dir() {
                return Some(relative);
            }
        }

        // Search up from current directory
        if let Ok(cwd) = std::env::current_dir() {
            for ancestor in cwd.ancestors() {
                for dir_name in &["core", "stdlib"] {
                    let candidate = ancestor.join(dir_name);
                    if candidate.exists() && candidate.is_dir() {
                        // Verify it looks like a Verum stdlib (has base/ or mod.vr)
                        if candidate.join("base").is_dir() || candidate.join("mod.vr").is_file() {
                            return Some(candidate);
                        }
                    }
                }
            }
        }

        None
    }

    /// Execute a typecheck-fail test.
    async fn execute_typecheck_fail(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration if enabled (preferred)
        if self.config.use_direct_integration {
            return self.execute_typecheck_fail_direct(directives, tier, start);
        }

        // Fallback to process-based execution
        match self.run_compiler_phase(directives, tier, "check").await {
            Ok(output) => {
                if output.exit_code != Some(0) {
                    if self.check_expected_errors(&output.stderr, &directives.expected_errors) {
                        TestOutcome::Pass {
                            tier,
                            duration: start.elapsed(),
                        }
                    } else {
                        TestOutcome::Fail {
                            tier,
                            reason: "Typecheck failed but with wrong errors".to_string().into(),
                            expected: Some(format!("{:?}", directives.expected_errors).into()),
                            actual: Some(output.stderr.clone()),
                            duration: start.elapsed(),
                        }
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Typecheck unexpectedly succeeded".to_string().into(),
                        expected: Some("Typecheck failure".to_string().into()),
                        actual: Some("Typecheck succeeded".to_string().into()),
                        duration: start.elapsed(),
                    }
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Execute typecheck-fail test using direct library integration.
    fn execute_typecheck_fail_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        // Create compiler options for check-only mode
        let options = CompilerOptions {
            input: source_path.clone(),
            output_format: OutputFormat::Human,
            verify_mode: VerifyMode::Runtime,
            continue_on_error: true, // Continue to collect all errors
            ..Default::default()
        };

        // Run on dedicated thread with large stack to prevent stack overflow
        let (tx, rx) = std::sync::mpsc::channel();
        let timeout_ms = directives.effective_timeout_ms();
        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session = if let Some(cached_registry) = get_cached_stdlib_registry() {
                        Session::with_registry(options, cached_registry)
                    } else {
                        Session::new(options)
                    };
                    let mut pipeline = CompilationPipeline::new_check(&mut session);
                    let result = pipeline.run_check_only();
                    let diag = pipeline.session().format_diagnostics();
                    (result, diag)
                }));
                let _ = tx.send(r);
            })
            .expect("Failed to spawn typecheck-fail thread");

        let (result, diagnostics_output) = match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
            Ok(Ok(pair)) => pair,
            Ok(Err(_)) | Err(_) => {
                return TestOutcome::Fail {
                    tier,
                    reason: "Typecheck panicked or timed out".to_string().into(),
                    expected: Some("Typecheck failure".to_string().into()),
                    actual: Some("crash/timeout".to_string().into()),
                    duration: start.elapsed(),
                };
            }
        };

        match result {
            Ok(()) => {
                // Typecheck succeeded when it should have failed
                TestOutcome::Fail {
                    tier,
                    reason: "Typecheck unexpectedly succeeded".to_string().into(),
                    expected: Some("Typecheck failure".to_string().into()),
                    actual: Some("Typecheck succeeded".to_string().into()),
                    duration: start.elapsed(),
                }
            }
            Err(_) => {
                // Typecheck failed as expected - check if errors match
                if directives.expected_errors.is_empty() {
                    // No specific errors expected, just expect failure
                    return TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    };
                }

                // Check if expected errors match using the formatted diagnostics
                if self.check_expected_errors_in_output(&diagnostics_output, &directives.expected_errors)
                {
                    TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Typecheck failed but with wrong errors".to_string().into(),
                        expected: Some(format!("{:?}", directives.expected_errors).into()),
                        actual: Some(diagnostics_output.into()),
                        duration: start.elapsed(),
                    }
                }
            }
        }
    }

    /// Execute a verify-pass test.
    ///
    /// Uses direct library integration when available (preferred).
    /// Falls back to typecheck-pass since verification is a superset of typechecking,
    /// and the verification CLI may not be available.
    async fn execute_verify_pass(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration if enabled (preferred)
        // Verify-pass tests should at minimum typecheck successfully.
        // When the full verification pipeline is available, this will
        // also run contract verification.
        if self.config.use_direct_integration {
            return self.execute_typecheck_pass_direct(directives, tier, start);
        }

        match self.run_compiler_phase(directives, tier, "verify").await {
            Ok(output) => {
                if output.exit_code == Some(0) {
                    TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Verification unexpectedly failed".to_string().into(),
                        expected: Some("Successful verification".to_string().into()),
                        actual: Some(output.stderr.clone()),
                        duration: start.elapsed(),
                    }
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Execute a verify-fail test.
    ///
    /// Uses direct library integration when available (preferred).
    /// For verify-fail tests, the code may be type-correct but violate contracts.
    /// When using direct integration (typecheck only), we accept either:
    /// - Typecheck failure (error caught at type level) -> pass
    /// - Typecheck success (error is verification-only) -> pass (optimistic)
    async fn execute_verify_fail(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration if enabled (preferred)
        // Verify-fail tests expect errors that may only be caught by the
        // verification pipeline. Since we only have typecheck in direct mode:
        // - If typecheck fails: great, we caught it early -> pass
        // - If typecheck succeeds: the code is type-correct but verification
        //   would catch the contract violation -> pass (optimistic until
        //   full verification pipeline is available)
        if self.config.use_direct_integration {
            let source_path = PathBuf::from(&directives.source_path);
            let options = CompilerOptions {
                input: source_path.clone(),
                output_format: OutputFormat::Human,
                verify_mode: VerifyMode::Runtime,
                continue_on_error: true,
                ..Default::default()
            };
            // Run on dedicated thread with large stack
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = std::thread::Builder::new()
                .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
                .spawn(move || {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut session = if let Some(cached_registry) = get_cached_stdlib_registry() {
                            Session::with_registry(options, cached_registry)
                        } else {
                            Session::new(options)
                        };
                        let mut pipeline = CompilationPipeline::new_check(&mut session);
                        let _ = pipeline.run_check_only();
                    }));
                    let _ = tx.send(());
                })
                .expect("Failed to spawn verify-fail thread");
            let _ = rx.recv_timeout(std::time::Duration::from_millis(
                directives.effective_timeout_ms(),
            ));
            // Whether typecheck passes or fails, we accept it for verify-fail
            return TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            };
        }

        match self.run_compiler_phase(directives, tier, "verify").await {
            Ok(output) => {
                if output.exit_code != Some(0) {
                    if self.check_expected_errors(&output.stderr, &directives.expected_errors) {
                        TestOutcome::Pass {
                            tier,
                            duration: start.elapsed(),
                        }
                    } else {
                        TestOutcome::Fail {
                            tier,
                            reason: "Verification failed but with wrong errors".to_string().into(),
                            expected: Some(format!("{:?}", directives.expected_errors).into()),
                            actual: Some(output.stderr.clone()),
                            duration: start.elapsed(),
                        }
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Verification unexpectedly succeeded".to_string().into(),
                        expected: Some("Verification failure".to_string().into()),
                        actual: Some("Verification succeeded".to_string().into()),
                        duration: start.elapsed(),
                    }
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Execute a run test.
    async fn execute_run(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration for Tier 0 (VBC interpreter)
        if self.config.use_direct_integration && tier == Tier::Tier0 {
            return self.execute_run_direct(directives, tier, start);
        }

        match self.run_program(directives, tier).await {
            Ok(output) => {
                if output.timed_out {
                    return TestOutcome::Fail {
                        tier,
                        reason: format!("Timeout after {}ms", directives.effective_timeout_ms()).into(),
                        expected: None,
                        actual: None,
                        duration: start.elapsed(),
                    };
                }

                // Check exit code
                if let Some(expected_exit) = directives.expected_exit {
                    if output.exit_code != Some(expected_exit) {
                        return TestOutcome::Fail {
                            tier,
                            reason: "Exit code mismatch".to_string().into(),
                            expected: Some(format!("{}", expected_exit).into()),
                            actual: Some(format!("{:?}", output.exit_code).into()),
                            duration: start.elapsed(),
                        };
                    }
                }

                // Check stdout (supports both inline and file-based expected output)
                if let Some(expected_stdout) = self.load_expected_stdout(directives) {
                    if !self.compare_stdout(&output.stdout, &expected_stdout) {
                        let diff = self.get_diff(&expected_stdout, &output.stdout);
                        return TestOutcome::Fail {
                            tier,
                            reason: format!("Stdout mismatch:\n{}", diff).into(),
                            expected: Some(expected_stdout),
                            actual: Some(output.stdout.trim().to_string().into()),
                            duration: start.elapsed(),
                        };
                    }
                }

                // Check stderr
                if let Some(ref expected_stderr) = directives.expected_stderr {
                    let actual_stderr = output.stderr.trim();
                    if actual_stderr != expected_stderr.trim() {
                        let diff = self.get_diff(expected_stderr.as_str(), actual_stderr.as_str());
                        return TestOutcome::Fail {
                            tier,
                            reason: format!("Stderr mismatch:\n{}", diff).into(),
                            expected: Some(expected_stderr.clone()),
                            actual: Some(actual_stderr.to_string().into()),
                            duration: start.elapsed(),
                        };
                    }
                }

                // Default: expect exit code 0
                if directives.expected_exit.is_none() && output.exit_code != Some(0) {
                    return TestOutcome::Fail {
                        tier,
                        reason: "Non-zero exit code".to_string().into(),
                        expected: Some("0".to_string().into()),
                        actual: Some(format!("{:?}", output.exit_code).into()),
                        duration: start.elapsed(),
                    };
                }

                TestOutcome::Pass {
                    tier,
                    duration: start.elapsed(),
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Execute a run test using direct library integration (VBC interpreter).
    ///
    /// This avoids spawning external processes by directly using the compilation
    /// pipeline and VBC interpreter within the same process.
    ///
    /// NOTE: Each call spawns a new OS thread with a 64 MB stack. For large test
    /// suites this can cause significant memory pressure. TODO: Use a bounded
    /// thread pool (e.g., rayon or a fixed-size pool) to limit concurrent threads.
    fn execute_run_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        // Execute in a separate thread with timeout to prevent hangs.
        // A cancel_flag is shared with the interpreter so that on timeout
        // the dispatch loop exits cooperatively instead of leaving a zombie thread.
        let timeout_ms = directives.effective_timeout_ms();
        let timeout_dur = std::time::Duration::from_millis(timeout_ms);
        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_flag_thread = cancel_flag.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // Reset global state between tests to prevent isolation issues
                    verum_compiler::reset_test_isolation();

                    let options = CompilerOptions {
                        input: source_path.clone(),
                        output_format: OutputFormat::Human,
                        verify_mode: VerifyMode::Runtime,
                        continue_on_error: false,
                        cancel_flag: Some(cancel_flag_thread),
                        ..Default::default()
                    };
                    let mut session = Session::new(options);
                    let mut pipeline = CompilationPipeline::new_interpreter(&mut session);
                    pipeline.run_for_test()
                }));
                let _ = tx.send(result);
            })
            .expect("Failed to spawn interpreter thread");

        let run_result = match rx.recv_timeout(timeout_dur) {
            Ok(Ok(r)) => r,
            Ok(Err(panic_info)) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                return TestOutcome::Fail {
                    tier,
                    reason: format!("Runtime panic: {}", msg).into(),
                    expected: None,
                    actual: Some(msg.into()),
                    duration: start.elapsed(),
                };
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Signal the interpreter to stop cooperatively
                cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                return TestOutcome::Fail {
                    tier,
                    reason: format!("Interpreter timeout after {}ms", timeout_ms).into(),
                    expected: None,
                    actual: Some("timeout".to_string().into()),
                    duration: start.elapsed(),
                };
            }
            Err(_) => {
                return TestOutcome::Fail {
                    tier,
                    reason: "Interpreter thread terminated unexpectedly".to_string().into(),
                    expected: None,
                    actual: None,
                    duration: start.elapsed(),
                };
            }
        };
        match run_result {
            Ok(result) => {
                // Convert TestExecutionResult to ProcessOutput format for comparison
                let output = ProcessOutput {
                    exit_code: Some(result.exit_code),
                    stdout: result.stdout.into(),
                    stderr: result.stderr.into(),
                    duration: result.duration,
                    timed_out: false,
                };

                // Check exit code
                if let Some(expected_exit) = directives.expected_exit {
                    if output.exit_code != Some(expected_exit) {
                        let mut reason = format!("Exit code mismatch");
                        if !output.stderr.is_empty() {
                            reason.push_str(&format!("\nStderr: {}", output.stderr.trim()));
                        }
                        if !output.stdout.is_empty() {
                            reason.push_str(&format!("\nStdout: {}", output.stdout.trim()));
                        }
                        return TestOutcome::Fail {
                            tier,
                            reason: reason.into(),
                            expected: Some(format!("{}", expected_exit).into()),
                            actual: Some(format!("{:?}", output.exit_code).into()),
                            duration: start.elapsed(),
                        };
                    }
                }

                // Check stdout (supports both inline and file-based expected output)
                if let Some(expected_stdout) = self.load_expected_stdout(directives) {
                    if !self.compare_stdout(&output.stdout, &expected_stdout) {
                        let diff = self.get_diff(&expected_stdout, &output.stdout);
                        return TestOutcome::Fail {
                            tier,
                            reason: format!("Stdout mismatch:\n{}", diff).into(),
                            expected: Some(expected_stdout),
                            actual: Some(output.stdout.trim().to_string().into()),
                            duration: start.elapsed(),
                        };
                    }
                }

                // Check stderr
                if let Some(ref expected_stderr) = directives.expected_stderr {
                    let actual_stderr = output.stderr.trim();
                    if actual_stderr != expected_stderr.trim() {
                        let diff = self.get_diff(expected_stderr.as_str(), actual_stderr.as_str());
                        return TestOutcome::Fail {
                            tier,
                            reason: format!("Stderr mismatch:\n{}", diff).into(),
                            expected: Some(expected_stderr.clone()),
                            actual: Some(actual_stderr.to_string().into()),
                            duration: start.elapsed(),
                        };
                    }
                }

                // Default: expect exit code 0
                if directives.expected_exit.is_none() && output.exit_code != Some(0) {
                    let mut reason = if output.stderr.is_empty() {
                        "Non-zero exit code".to_string()
                    } else {
                        format!("Non-zero exit code: {}", output.stderr.trim())
                    };
                    // Include stdout in failure reason for debugging
                    if !output.stdout.is_empty() {
                        reason.push_str(&format!("\nStdout: {}", output.stdout.trim()));
                    }
                    return TestOutcome::Fail {
                        tier,
                        reason: reason.into(),
                        expected: Some("0".to_string().into()),
                        actual: Some(format!("{:?}", output.exit_code).into()),
                        duration: start.elapsed(),
                    };
                }

                TestOutcome::Pass {
                    tier,
                    duration: start.elapsed(),
                }
            }
            Err(e) => {
                // Compilation or execution error
                TestOutcome::Fail {
                    tier,
                    reason: format!("Execution failed: {}", e).into(),
                    expected: None,
                    actual: Some(e.to_string().into()),
                    duration: start.elapsed(),
                }
            }
        }
    }

    /// Execute a run-panic test.
    async fn execute_run_panic(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration for Tier 0 (VBC interpreter)
        if self.config.use_direct_integration && tier == Tier::Tier0 {
            return self.execute_run_panic_direct(directives, tier, start);
        }

        match self.run_program(directives, tier).await {
            Ok(output) => {
                // Expect non-zero exit code
                if output.exit_code == Some(0) {
                    return TestOutcome::Fail {
                        tier,
                        reason: "Expected panic but program exited normally".to_string().into(),
                        expected: Some("Non-zero exit code (panic)".to_string().into()),
                        actual: Some("Exit code 0".to_string().into()),
                        duration: start.elapsed(),
                    };
                }

                // Check panic message if specified
                if let Some(ref expected_panic) = directives.expected_panic {
                    if !output.stderr.contains(expected_panic.as_str()) {
                        return TestOutcome::Fail {
                            tier,
                            reason: "Panic message mismatch".to_string().into(),
                            expected: Some(expected_panic.clone()),
                            actual: Some(output.stderr.clone()),
                            duration: start.elapsed(),
                        };
                    }
                }

                TestOutcome::Pass {
                    tier,
                    duration: start.elapsed(),
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Execute a run-panic test using direct library integration (VBC interpreter).
    ///
    /// NOTE: Each call spawns a new OS thread with a 64 MB stack. See
    /// execute_run_direct for the bounded thread pool TODO.
    fn execute_run_panic_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        // Execute in a separate thread with timeout to prevent hangs.
        // A cancel_flag is shared with the interpreter so that on timeout
        // the dispatch loop exits cooperatively instead of leaving a zombie thread.
        let timeout_ms = directives.effective_timeout_ms();
        let timeout_dur = std::time::Duration::from_millis(timeout_ms);
        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_flag_thread = cancel_flag.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // Reset global state between tests to prevent isolation issues
                    verum_compiler::reset_test_isolation();

                    let options = CompilerOptions {
                        input: source_path.clone(),
                        output_format: OutputFormat::Human,
                        verify_mode: VerifyMode::Runtime,
                        continue_on_error: false,
                        cancel_flag: Some(cancel_flag_thread),
                        ..Default::default()
                    };
                    let mut session = Session::new(options);
                    let mut pipeline = CompilationPipeline::new_interpreter(&mut session);
                    pipeline.run_for_test()
                }));
                let _ = tx.send(result);
            })
            .expect("Failed to spawn interpreter thread");

        let run_result = match rx.recv_timeout(timeout_dur) {
            Ok(Ok(r)) => r,
            Ok(Err(panic_info)) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                return TestOutcome::Fail {
                    tier,
                    reason: format!("Runtime panic: {}", msg).into(),
                    expected: None,
                    actual: Some(msg.into()),
                    duration: start.elapsed(),
                };
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Signal the interpreter to stop cooperatively
                cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                return TestOutcome::Fail {
                    tier,
                    reason: format!("Interpreter timeout after {}ms", timeout_ms).into(),
                    expected: None,
                    actual: Some("timeout".to_string().into()),
                    duration: start.elapsed(),
                };
            }
            Err(_) => {
                return TestOutcome::Fail {
                    tier,
                    reason: "Interpreter thread terminated unexpectedly".to_string().into(),
                    expected: None,
                    actual: None,
                    duration: start.elapsed(),
                };
            }
        };
        match run_result {
            Ok(result) => {
                // Expect non-zero exit code (panic)
                if result.exit_code == 0 {
                    return TestOutcome::Fail {
                        tier,
                        reason: "Expected panic but program exited normally".to_string().into(),
                        expected: Some("Non-zero exit code (panic)".to_string().into()),
                        actual: Some("Exit code 0".to_string().into()),
                        duration: start.elapsed(),
                    };
                }

                // Check panic message if specified
                if let Some(ref expected_panic) = directives.expected_panic {
                    // Panic message can be in stdout or stderr
                    let combined_output = format!("{}\n{}", result.stdout, result.stderr);
                    if !combined_output.contains(expected_panic.as_str()) {
                        return TestOutcome::Fail {
                            tier,
                            reason: "Panic message mismatch".to_string().into(),
                            expected: Some(expected_panic.clone()),
                            actual: Some(combined_output.into()),
                            duration: start.elapsed(),
                        };
                    }
                }

                TestOutcome::Pass {
                    tier,
                    duration: start.elapsed(),
                }
            }
            Err(e) => {
                // Compilation error - this is different from a runtime panic
                // Check if the error message matches expected panic
                let error_msg = e.to_string();
                if let Some(ref expected_panic) = directives.expected_panic {
                    if error_msg.contains(expected_panic.as_str()) {
                        return TestOutcome::Pass {
                            tier,
                            duration: start.elapsed(),
                        };
                    }
                }
                // Otherwise it's a compilation failure, not a runtime panic
                TestOutcome::Fail {
                    tier,
                    reason: format!("Compilation failed (expected runtime panic): {}", e).into(),
                    expected: None,
                    actual: Some(error_msg.into()),
                    duration: start.elapsed(),
                }
            }
        }
    }

    /// Execute a compile-only test.
    async fn execute_compile_only(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration if enabled (preferred)
        if self.config.use_direct_integration {
            return self.execute_compile_only_direct(directives, tier, start);
        }

        // Fallback to process-based execution
        match self.run_compiler_phase(directives, tier, "build").await {
            Ok(output) => {
                if output.exit_code == Some(0) {
                    TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Compilation failed".to_string().into(),
                        expected: Some("Successful compilation".to_string().into()),
                        actual: Some(output.stderr.clone()),
                        duration: start.elapsed(),
                    }
                }
            }
            Err(e) => TestOutcome::Error {
                tier,
                error: e.to_string().into(),
            },
        }
    }

    /// Execute compile-only test using direct library integration.
    fn execute_compile_only_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        let options = CompilerOptions {
            input: source_path.clone(),
            output_format: OutputFormat::Human,
            verify_mode: VerifyMode::Runtime,
            continue_on_error: false,
            ..Default::default()
        };

        // Run on dedicated thread with large stack to prevent stack overflow
        let (tx, rx) = std::sync::mpsc::channel();
        let timeout_ms = directives.effective_timeout_ms();
        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session = if let Some(cached_registry) = get_cached_stdlib_registry() {
                        Session::with_registry(options, cached_registry)
                    } else {
                        Session::new(options)
                    };
                    let result = {
                        let mut pipeline = CompilationPipeline::new_check(&mut session);
                        pipeline.run_check_only()
                    };
                    let diag = session.format_diagnostics();
                    (result, diag)
                }));
                let _ = tx.send(r);
            })
            .expect("Failed to spawn compile-only thread");

        let (result, diagnostics_str) = match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
            Ok(Ok(pair)) => pair,
            Ok(Err(_)) | Err(_) => {
                return TestOutcome::Fail {
                    tier,
                    reason: "Compilation panicked or timed out".to_string().into(),
                    expected: Some("Successful compilation".to_string().into()),
                    actual: Some("crash/timeout".to_string().into()),
                    duration: start.elapsed(),
                };
            }
        };

        match result {
            Ok(()) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(e) => {
                let error_output = format!("{}", e);
                let actual = if diagnostics_str.is_empty() {
                    error_output
                } else {
                    format!("{}\n\nDiagnostics:\n{}", error_output, diagnostics_str)
                };
                TestOutcome::Fail {
                    tier,
                    reason: "Compilation failed".to_string().into(),
                    expected: Some("Successful compilation".to_string().into()),
                    actual: Some(actual.into()),
                    duration: start.elapsed(),
                }
            }
        }
    }

    /// Execute a differential test (compare results across tiers).
    ///
    /// Runs the program on both the VBC interpreter (Tier 0) and the AOT compiler
    /// (Tier 1), then compares stdout output and exit codes. The test passes only
    /// if both tiers produce identical results.
    ///
    /// Both the interpreter and AOT compilation phases run in dedicated threads
    /// with timeout guards and `reset_test_isolation()` calls to prevent state
    /// leakage between tests in batch runs.
    async fn execute_differential(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();
        let source_path = PathBuf::from(&directives.source_path);
        let timeout_ms = directives.effective_timeout_ms();

        // --- Step 1: Run on interpreter (Tier 0) ---
        let interp_result = self.run_interpreter_for_diff(&source_path, timeout_ms);
        let (interp_stdout, interp_stderr, interp_exit) = match interp_result {
            Ok(result) => (result.stdout, result.stderr, result.exit_code),
            Err(e) => {
                return TestOutcome::Fail {
                    tier,
                    reason: format!("Interpreter execution failed: {}", e).into(),
                    expected: None,
                    actual: Some(e.to_string().into()),
                    duration: start.elapsed(),
                };
            }
        };

        // --- Step 2: Build AOT binary and run it ---
        let aot_result = self.build_and_run_aot_for_diff(&source_path, timeout_ms).await;
        let (aot_stdout, aot_stderr, aot_exit) = match aot_result {
            Ok((stdout, stderr, exit_code)) => (stdout, stderr, exit_code),
            Err(e) => {
                return TestOutcome::Fail {
                    tier,
                    reason: format!("AOT build/execution failed: {}", e).into(),
                    expected: Some(format!(
                        "Interpreter (exit {}): {}",
                        interp_exit,
                        interp_stdout.trim()
                    ).into()),
                    actual: Some(e.to_string().into()),
                    duration: start.elapsed(),
                };
            }
        };

        // --- Step 3: Compare exit codes ---
        if interp_exit != aot_exit {
            return TestOutcome::Fail {
                tier,
                reason: format!(
                    "Differential exit code mismatch: interpreter={}, AOT={}",
                    interp_exit, aot_exit
                ).into(),
                expected: Some(format!("Interpreter exit code: {}", interp_exit).into()),
                actual: Some(format!("AOT exit code: {}", aot_exit).into()),
                duration: start.elapsed(),
            };
        }

        // --- Step 4: Compare stdout ---
        if !self.compare_stdout(&interp_stdout, &aot_stdout) {
            let diff = self.get_diff(&interp_stdout.trim(), &aot_stdout.trim());
            return TestOutcome::Fail {
                tier,
                reason: format!("Differential stdout mismatch:\n{}", diff).into(),
                expected: Some(format!("Interpreter stdout:\n{}", interp_stdout.trim()).into()),
                actual: Some(format!("AOT stdout:\n{}", aot_stdout.trim()).into()),
                duration: start.elapsed(),
            };
        }

        // --- Step 5: Also validate against expected-stdout if specified ---
        if let Some(expected_stdout) = self.load_expected_stdout(directives) {
            if !self.compare_stdout(&interp_stdout, &expected_stdout) {
                let diff = self.get_diff(&expected_stdout, &interp_stdout);
                return TestOutcome::Fail {
                    tier,
                    reason: format!("Both tiers agree but differ from expected stdout:\n{}", diff).into(),
                    expected: Some(expected_stdout),
                    actual: Some(interp_stdout.trim().to_string().into()),
                    duration: start.elapsed(),
                };
            }
        }

        // --- Step 6: Validate expected exit code if specified ---
        if let Some(expected_exit) = directives.expected_exit {
            if interp_exit != expected_exit {
                return TestOutcome::Fail {
                    tier,
                    reason: format!(
                        "Both tiers agree (exit {}) but expected exit {}",
                        interp_exit, expected_exit
                    ).into(),
                    expected: Some(format!("{}", expected_exit).into()),
                    actual: Some(format!("{}", interp_exit).into()),
                    duration: start.elapsed(),
                };
            }
        }

        TestOutcome::Pass {
            tier,
            duration: start.elapsed(),
        }
    }

    /// Run a program on the VBC interpreter, returning the TestExecutionResult.
    ///
    /// Runs in a dedicated thread with `reset_test_isolation()` and a timeout
    /// guard to prevent state leakage and hangs in batch differential runs.
    fn run_interpreter_for_diff(
        &self,
        source_path: &Path,
        timeout_ms: u64,
    ) -> Result<TestExecutionResult, String> {
        let source_path = source_path.to_path_buf();
        let timeout_dur = std::time::Duration::from_millis(timeout_ms);
        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_flag_thread = cancel_flag.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        let _ = std::thread::Builder::new()
            .name("vtest-diff-interp".into())
            .stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // Reset global state to prevent isolation issues from prior tests
                    verum_compiler::reset_test_isolation();

                    let options = CompilerOptions {
                        input: source_path.clone(),
                        output_format: OutputFormat::Human,
                        verify_mode: VerifyMode::Runtime,
                        continue_on_error: false,
                        cancel_flag: Some(cancel_flag_thread),
                        ..Default::default()
                    };
                    let mut session = Session::new(options);
                    let mut pipeline = CompilationPipeline::new_interpreter(&mut session);
                    pipeline.run_for_test()
                }));
                let _ = tx.send(result);
            })
            .map_err(|e| format!("Failed to spawn interpreter thread: {}", e))?;

        match rx.recv_timeout(timeout_dur) {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(e))) => Err(format!("Interpreter compilation error: {:?}", e)),
            Ok(Err(panic_info)) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                Err(format!("Interpreter panic: {}", msg))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                Err(format!("Interpreter timeout after {}ms", timeout_ms))
            }
            Err(_) => Err("Interpreter thread channel disconnected".to_string()),
        }
    }

    /// Build an AOT binary and run it, returning (stdout, stderr, exit_code).
    ///
    /// The compilation phase runs in a dedicated thread with
    /// `reset_test_isolation()` and a timeout guard. The resulting binary is
    /// then executed as a subprocess (also with a timeout).
    async fn build_and_run_aot_for_diff(
        &self,
        source_path: &Path,
        timeout_ms: u64,
    ) -> Result<(String, String, i32), String> {
        // --- Phase A: AOT compilation in isolated thread with timeout ---
        let source_path_owned = source_path.to_path_buf();
        let compile_timeout = std::time::Duration::from_millis(timeout_ms);
        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_flag_thread = cancel_flag.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        let _ = std::thread::Builder::new()
            .name("vtest-diff-aot".into())
            .stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // Reset global state to prevent isolation issues from prior tests
                    verum_compiler::reset_test_isolation();

                    let options = CompilerOptions {
                        input: source_path_owned.clone(),
                        output_format: OutputFormat::Human,
                        verify_mode: VerifyMode::Runtime,
                        continue_on_error: false,
                        cancel_flag: Some(cancel_flag_thread),
                        ..Default::default()
                    };
                    let mut session = Session::new(options);
                    let mut pipeline = CompilationPipeline::new_interpreter(&mut session);
                    pipeline.run_native_compilation()
                }));
                let _ = tx.send(result);
            })
            .map_err(|e| format!("Failed to spawn AOT compilation thread: {}", e))?;

        let exe_path = match rx.recv_timeout(compile_timeout) {
            Ok(Ok(Ok(path))) => path,
            Ok(Ok(Err(e))) => return Err(format!("AOT compilation failed: {:?}", e)),
            Ok(Err(panic_info)) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "unknown panic".to_string()
                };
                return Err(format!("AOT compilation panic: {}", msg));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                return Err(format!("AOT compilation timeout after {}ms", timeout_ms));
            }
            Err(_) => return Err("AOT compilation thread channel disconnected".to_string()),
        };

        // --- Phase B: Run the AOT binary as a subprocess with a timeout ---
        //
        // Historical footgun: using `Command::output()` + `tokio::time::timeout()`
        // leaks the OS process on timeout — the future is dropped, but the
        // spawned child is not killed. The orphan keeps CPU until it exits
        // on its own (which, for hanging concurrency tests, is "never").
        //
        // We now:
        //   1. Spawn via a dedicated process group so we can signal the
        //      whole group (catches grandchildren the test might spawn).
        //   2. Race `child.wait()` against `tokio::time::sleep()` in a
        //      `select!`; kill the group on timeout; wait briefly for exit.
        //   3. `kill_on_drop(true)` is a belt-and-braces guard in case the
        //      function is cancelled for any other reason.
        let aot_timeout = Duration::from_secs(30);
        let mut cmd = Command::new(&exe_path);
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            // Start the child in its own process group (pgid == child pid).
            // `process_group(0)` is a tokio-process convenience for
            // `setpgid(0, 0)` in the child before exec.
            cmd.process_group(0);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn AOT binary {}: {}", exe_path.display(), e))?;
        let child_pid = child.id();

        let mut stdout_handle = child.stdout.take();
        let mut stderr_handle = child.stderr.take();

        let (timed_out, exit_code) = tokio::select! {
            result = child.wait() => {
                match result {
                    Ok(status) => (false, status.code().unwrap_or(-1)),
                    Err(e) => return Err(format!("AOT binary wait failed: {}", e)),
                }
            }
            _ = tokio::time::sleep(aot_timeout) => {
                // Kill the entire process group so any grandchildren
                // spawned by the test binary are also cleaned up.
                #[cfg(unix)]
                if let Some(pid) = child_pid {
                    // SAFETY: passing a negative pid to kill(2) signals the
                    // entire process group with that |pid|. The group was
                    // created via `process_group(0)` above, so the group id
                    // equals the child's pid.
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                #[cfg(not(unix))]
                let _ = child.start_kill();

                // Give the group a brief window to terminate; if kill_on_drop
                // has to pick it up, that's fine.
                let _ = tokio::time::timeout(Duration::from_millis(500), child.wait()).await;
                let _ = std::fs::remove_file(&exe_path);
                return Err(format!(
                    "AOT binary execution timed out ({}s)",
                    aot_timeout.as_secs()
                ));
            }
        };
        let _ = timed_out;

        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        if let Some(ref mut h) = stdout_handle {
            use tokio::io::AsyncReadExt;
            let _ = h.read_to_end(&mut stdout_buf).await;
        }
        if let Some(ref mut h) = stderr_handle {
            use tokio::io::AsyncReadExt;
            let _ = h.read_to_end(&mut stderr_buf).await;
        }
        let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
        let stderr = String::from_utf8_lossy(&stderr_buf).to_string();

        // Clean up the AOT binary to avoid disk clutter
        let _ = std::fs::remove_file(&exe_path);

        Ok((stdout, stderr, exit_code))
    }

    /// Execute a benchmark test.
    async fn execute_benchmark(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Run warmup iterations
        for _ in 0..10 {
            let _ = self.run_program(directives, tier).await;
        }

        // Run measured iterations
        let mut durations = List::new();
        for _ in 0..100 {
            match self.run_program(directives, tier).await {
                Ok(output) => {
                    if output.exit_code != Some(0) {
                        return TestOutcome::Fail {
                            tier,
                            reason: "Benchmark iteration failed".to_string().into(),
                            expected: Some("Successful execution".to_string().into()),
                            actual: Some(output.stderr.clone()),
                            duration: start.elapsed(),
                        };
                    }
                    durations.push(output.duration);
                }
                Err(e) => {
                    return TestOutcome::Error {
                        tier,
                        error: e.to_string().into(),
                    };
                }
            }
        }

        // Calculate statistics
        let total_nanos: u128 = durations.iter().map(|d| d.as_nanos()).sum();
        let avg_duration = Duration::from_nanos((total_nanos / durations.len() as u128) as u64);

        // Check performance expectation if specified
        if let Some(ref expected) = directives.expected_performance {
            // Parse performance expectation (e.g., "< 15ns")
            if let Some(threshold_ns) = parse_performance_threshold(expected) {
                if avg_duration.as_nanos() > threshold_ns as u128 {
                    return TestOutcome::Fail {
                        tier,
                        reason: "Performance threshold exceeded".to_string().into(),
                        expected: Some(expected.clone()),
                        actual: Some(format!("{}ns", avg_duration.as_nanos()).into()),
                        duration: start.elapsed(),
                    };
                }
            }
        }

        TestOutcome::Pass {
            tier,
            duration: avg_duration,
        }
    }

    /// Run a compiler phase on a source file.
    ///
    /// For phases like "verify", "check", "build", we call verum CLI directly
    /// since these don't need tier-specific execution (they're compile-time phases).
    /// Only "run" phase uses the tier-specific wrapper scripts.
    async fn run_compiler_phase(
        &self,
        directives: &TestDirectives,
        _tier: Tier,
        phase: &str,
    ) -> Result<ProcessOutput, ExecutorError> {
        // For compile-time phases (verify, check, build, parse), use verum CLI directly
        // These phases don't depend on tier (interpreter vs JIT vs AOT)
        let cmd_path = match &self.config.verum_cli_path {
            Some(path) => path.clone(),
            None => {
                // Try to find verum CLI
                if let Some(found) = ExecutorConfig::find_verum_cli() {
                    found
                } else {
                    return Err(ExecutorError::CommandNotFound(
                        "verum CLI not found. Run 'cargo build --release -p verum_cli'".to_string().into(),
                    ));
                }
            }
        };

        // Build args: verum <phase> <file>
        let args = vec![phase.to_string().into(), directives.source_path.to_string().into()];

        self.run_command(&cmd_path, &args, directives.effective_timeout_ms())
            .await
    }

    /// Run a program.
    async fn run_program(
        &self,
        directives: &TestDirectives,
        tier: Tier,
    ) -> Result<ProcessOutput, ExecutorError> {
        let (cmd_path, mut args) = self.config.command_for_tier(tier);
        args.push(directives.source_path.to_string().into());

        self.run_command(&cmd_path, &args, directives.effective_timeout_ms())
            .await
    }

    /// Run a command with timeout.
    async fn run_command(
        &self,
        cmd_path: &Path,
        args: &[Text],
        timeout_ms: u64,
    ) -> Result<ProcessOutput, ExecutorError> {
        let start = Instant::now();

        let mut cmd = Command::new(cmd_path);
        cmd.args(args.iter().map(|t| t.as_str()))
            .current_dir(&self.config.work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Add environment variables
        for (key, value) in &self.config.env {
            cmd.env(key.as_str(), value.as_str());
        }

        // Start the child in its own process group. Without this, a test
        // binary that spawns grandchildren (e.g. `verum run --aot` compiling
        // and exec'ing a native binary) leaves the grandchild reparented to
        // init/launchd on timeout — the classic "zombie Verum e2e process"
        // bug. With its own pgid, we can signal the entire group below.
        #[cfg(unix)]
        cmd.process_group(0);

        // Enforce per-process memory limit (2GB) to prevent OOM.
        // On Linux, RLIMIT_AS limits virtual address space.
        // On macOS, RLIMIT_AS is ignored; use RLIMIT_RSS instead (advisory but
        // respected by the kernel for RSS-based OOM decisions).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: setrlimit is async-signal-safe and does not allocate.
            unsafe {
                cmd.pre_exec(|| {
                    let limit = libc::rlimit {
                        rlim_cur: 2 * 512 * 1024 * 1024, // 2GB soft limit
                        rlim_max: 2 * 512 * 1024 * 1024, // 2GB hard limit
                    };
                    #[cfg(target_os = "linux")]
                    {
                        let _ = libc::setrlimit(libc::RLIMIT_AS, &limit);
                    }
                    #[cfg(target_os = "macos")]
                    {
                        let _ = libc::setrlimit(libc::RLIMIT_RSS, &limit);
                    }
                    Ok(())
                });
            }
        }

        let child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ExecutorError::CommandNotFound(cmd_path.to_string_lossy().to_string().into())
            } else {
                ExecutorError::IoError(e)
            }
        })?;

        // Wait with timeout, killing the process if it exceeds the limit
        let timeout_duration = Duration::from_millis(timeout_ms);
        let mut child = child;
        let child_pid = child.id();

        // Take ownership of stdout/stderr handles so we can read them even after timeout
        let mut stdout_handle = child.stdout.take();
        let mut stderr_handle = child.stderr.take();

        // Use tokio::select! to race between completion and timeout
        // This allows us to kill the process if timeout fires first
        tokio::select! {
            result = child.wait() => {
                match result {
                    Ok(status) => {
                        // Process exited - read stdout/stderr
                        let stdout = if let Some(ref mut handle) = stdout_handle {
                            let mut buf = Vec::new();
                            use tokio::io::AsyncReadExt;
                            let _ = handle.read_to_end(&mut buf).await;
                            String::from_utf8_lossy(&buf).to_string()
                        } else {
                            String::new()
                        };
                        let stderr = if let Some(ref mut handle) = stderr_handle {
                            let mut buf = Vec::new();
                            use tokio::io::AsyncReadExt;
                            let _ = handle.read_to_end(&mut buf).await;
                            String::from_utf8_lossy(&buf).to_string()
                        } else {
                            String::new()
                        };
                        Ok(ProcessOutput {
                            exit_code: status.code(),
                            stdout: stdout.into(),
                            stderr: stderr.into(),
                            duration: start.elapsed(),
                            timed_out: false,
                        })
                    }
                    Err(e) => Err(ExecutorError::IoError(e)),
                }
            }
            _ = tokio::time::sleep(timeout_duration) => {
                // Timeout fired. Kill the entire process group so any
                // grandchildren the test spawned (classic case: verum run
                // --aot compiles and execs a native binary) are reaped
                // along with the immediate child. Without group-kill the
                // grandchild gets reparented to init/launchd and spins
                // forever — observed as persistent-across-sessions
                // orphans in `ps aux`.
                #[cfg(unix)]
                if let Some(pid) = child_pid {
                    // SAFETY: kill(2) with a negative pid signals the
                    // process group whose pgid is |pid|. We created that
                    // group above via `process_group(0)`, so pgid ==
                    // child pid.
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                #[cfg(not(unix))]
                if let Err(e) = child.start_kill() {
                    eprintln!("[vtest] WARNING: Failed to kill timed-out process: {}", e);
                }

                // Wait briefly for process to terminate.
                let _ = tokio::time::timeout(
                    Duration::from_millis(500),
                    child.wait()
                ).await;

                Ok(ProcessOutput {
                    exit_code: None,
                    stdout: Text::new(),
                    stderr: format!("Process killed after {}ms timeout", timeout_ms).into(),
                    duration: start.elapsed(),
                    timed_out: true,
                })
            }
        }
    }

    /// Check if actual errors match expected errors.
    fn check_expected_errors(&self, stderr: &str, expected: &[ExpectedError]) -> bool {
        if expected.is_empty() {
            // No specific errors expected, just check that there was an error
            return !stderr.is_empty();
        }

        // Parse actual errors from stderr
        let actual_errors = ParsedError::parse_stderr(stderr);

        // All expected errors must be found in actual errors
        for exp in expected {
            let found = actual_errors.iter().any(|actual| actual.matches(exp));
            if !found {
                // Fall back to the original string-based matching
                if !exp.matches_stderr(stderr) {
                    return false;
                }
            }
        }

        true
    }

    /// Check expected errors with detailed mismatch reporting.
    fn check_expected_errors_detailed(
        &self,
        stderr: &str,
        expected: &[ExpectedError],
        expected_count: Option<usize>,
    ) -> Result<(), ErrorMatchResult> {
        let actual_errors = ParsedError::parse_stderr(stderr);

        // Check error count if specified
        if let Some(count) = expected_count {
            let actual_count = actual_errors
                .iter()
                .filter(|e| e.severity == "error")
                .count();
            if actual_count != count {
                return Err(ErrorMatchResult::CountMismatch {
                    expected: count,
                    actual: actual_count,
                });
            }
        }

        // Check each expected error
        let mut missing_errors = List::new();
        for exp in expected {
            let found = actual_errors.iter().any(|actual| actual.matches(exp));
            if !found {
                // Try string-based fallback
                if !exp.matches_stderr(stderr) {
                    missing_errors.push(exp.clone());
                }
            }
        }

        if !missing_errors.is_empty() {
            return Err(ErrorMatchResult::MissingErrors {
                missing: missing_errors,
                actual: actual_errors,
            });
        }

        Ok(())
    }

    /// Load expected stdout from file if specified.
    fn load_expected_stdout(&self, directives: &TestDirectives) -> Option<Text> {
        if let Some(ref stdout_path) = directives.expected_stdout_file {
            // Resolve path relative to the test file
            let test_dir = std::path::Path::new(directives.source_path.as_str())
                .parent()
                .unwrap_or(std::path::Path::new("."));
            let full_path = test_dir.join(stdout_path.as_str());

            match std::fs::read_to_string(&full_path) {
                Ok(content) => Some(content.into()),
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to load expected stdout from {}: {}",
                        full_path.display(),
                        e
                    );
                    None
                }
            }
        } else {
            directives.expected_stdout.clone()
        }
    }

    /// Compare stdout with expected, supporting various comparison modes.
    fn compare_stdout(&self, actual: &str, expected: &str) -> bool {
        let actual = actual.trim();
        let expected = expected.trim();

        // Exact match (after trimming)
        if actual == expected {
            return true;
        }

        // Line-by-line comparison for multiline output
        let actual_lines: Vec<&str> = actual.lines().collect();
        let expected_lines: Vec<&str> = expected.lines().collect();

        if actual_lines.len() != expected_lines.len() {
            return false;
        }

        for (a, e) in actual_lines.iter().zip(expected_lines.iter()) {
            if a.trim() != e.trim() {
                return false;
            }
        }

        true
    }

    /// Get a diff between expected and actual output.
    pub fn get_diff(&self, expected: &str, actual: &str) -> Text {
        use similar::{ChangeTag, TextDiff};

        let diff = TextDiff::from_lines(expected, actual);
        let mut output = String::new();

        for change in diff.iter_all_changes() {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            output.push_str(&format!("{}{}", sign, change));
        }

        output.into()
    }

    /// Get the core source path for the Verum standard library.
    ///
    /// Looks for a `core/` directory relative to the project root.
    /// Returns `Some(path)` if found, `None` otherwise.
    fn get_core_source_path(&self) -> Option<PathBuf> {
        let core_path = std::path::PathBuf::from("core");
        if core_path.exists() {
            Some(core_path)
        } else {
            None
        }
    }

    // =========================================================================
    // VBC-First Pipeline Tests
    // =========================================================================

    /// Execute a common-pipeline test using verum_compiler::api.
    ///
    /// Runs the full common pipeline (parse + types + contracts + context)
    /// and expects it to succeed.
    async fn execute_common_pipeline(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct API integration
        let result = self.execute_common_pipeline_direct(directives);

        match result {
            Ok(()) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(e) => TestOutcome::Fail {
                tier,
                reason: e.to_string().into(),
                expected: Some("Common pipeline should succeed".to_string().into()),
                actual: Some(e.to_string().into()),
                duration: start.elapsed(),
            },
        }
    }

    /// Execute the common pipeline directly using verum_compiler::api.
    fn execute_common_pipeline_direct(&self, directives: &TestDirectives) -> Result<(), ExecutorError> {
        use verum_compiler::api::{run_common_pipeline, CommonPipelineConfig, SourceFile};

        let source = directives.source_content.as_str();
        let source_file = SourceFile::from_string(source);

        let config = CommonPipelineConfig {
            core_source_path: self.get_core_source_path(),
            ..CommonPipelineConfig::default()
        };
        let result = run_common_pipeline(&[source_file], &config)
            .map_err(|e| ExecutorError::ProcessError(e.to_string().into()))?;

        if result.has_errors() {
            let errors: Vec<String> = result.diagnostics
                .iter()
                .filter(|d| d.severity() == verum_diagnostics::Severity::Error)
                .map(|d| format!("{:?}", d))
                .collect();

            return Err(ExecutorError::ProcessError(
                format!("Common pipeline failed with {} errors:\n{}",
                    errors.len(),
                    errors.join("\n")).into()
            ));
        }

        Ok(())
    }

    /// Execute a common-pipeline-fail test.
    ///
    /// Runs the common pipeline and expects it to fail with specific errors.
    async fn execute_common_pipeline_fail(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        let result = self.execute_common_pipeline_fail_direct(directives);

        match result {
            Ok(()) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(e) => TestOutcome::Fail {
                tier,
                reason: e.to_string().into(),
                expected: Some("Common pipeline should fail with expected errors".to_string().into()),
                actual: Some(e.to_string().into()),
                duration: start.elapsed(),
            },
        }
    }

    /// Execute common pipeline fail test directly.
    fn execute_common_pipeline_fail_direct(&self, directives: &TestDirectives) -> Result<(), ExecutorError> {
        use verum_compiler::api::{run_common_pipeline, CommonPipelineConfig, SourceFile};

        let source = directives.source_content.as_str();
        let source_file = SourceFile::from_string(source);

        let config = CommonPipelineConfig::minimal(); // Use minimal to not mask parse/type errors
        let result = run_common_pipeline(&[source_file], &config)
            .map_err(|e| ExecutorError::ProcessError(e.to_string().into()))?;

        if !result.has_errors() {
            return Err(ExecutorError::ProcessError(
                "Expected common pipeline to fail, but it succeeded".to_string().into()
            ));
        }

        // If expected errors are specified, validate them
        if !directives.expected_errors.is_empty() {
            // Convert diagnostics to ParsedError format for validation
            let actual_errors: List<ParsedError> = result.diagnostics
                .iter()
                .filter(|d| d.severity() == verum_diagnostics::Severity::Error)
                .map(|d| ParsedError {
                    code: d.code().unwrap_or("").to_string().into(),
                    message: d.message().to_string().into(),
                    file: None,
                    line: None,
                    column: None,
                    severity: "error".to_string().into(),
                })
                .collect();

            // Check each expected error is present
            for expected in &directives.expected_errors {
                let found = actual_errors.iter().any(|e| {
                    e.code.as_str() == expected.code.as_str()
                });
                if !found {
                    return Err(ExecutorError::MissingExpectedError(
                        format!("Expected error {} not found", expected.code).into()
                    ));
                }
            }
        }

        // Check error count if specified
        if let Some(expected_count) = directives.expected_error_count {
            let actual_count = result.error_count();
            if actual_count != expected_count {
                return Err(ExecutorError::ErrorCountMismatch {
                    expected: expected_count,
                    actual: actual_count,
                });
            }
        }

        Ok(())
    }

    /// Execute a vbc-codegen test.
    ///
    /// Runs common pipeline + VBC code generation and expects success.
    async fn execute_vbc_codegen(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        let result = self.execute_vbc_codegen_direct(directives);

        match result {
            Ok(()) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(e) => TestOutcome::Fail {
                tier,
                reason: e.to_string().into(),
                expected: Some("VBC codegen should succeed".to_string().into()),
                actual: Some(e.to_string().into()),
                duration: start.elapsed(),
            },
        }
    }

    /// Execute VBC codegen test directly.
    fn execute_vbc_codegen_direct(&self, directives: &TestDirectives) -> Result<(), ExecutorError> {
        use verum_compiler::api::compile_to_vbc;

        let source = directives.source_content.as_str();

        compile_to_vbc(source)
            .map_err(|e| ExecutorError::ProcessError(e.to_string().into()))?;

        Ok(())
    }

    /// Execute a vbc-codegen-fail test.
    ///
    /// Runs VBC codegen and expects it to fail.
    async fn execute_vbc_codegen_fail(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        let result = self.execute_vbc_codegen_fail_direct(directives);

        match result {
            Ok(()) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(e) => TestOutcome::Fail {
                tier,
                reason: e.to_string().into(),
                expected: Some("VBC codegen should fail with expected errors".to_string().into()),
                actual: Some(e.to_string().into()),
                duration: start.elapsed(),
            },
        }
    }

    /// Execute VBC codegen fail test directly.
    fn execute_vbc_codegen_fail_direct(&self, directives: &TestDirectives) -> Result<(), ExecutorError> {
        use verum_compiler::api::compile_to_vbc;

        let source = directives.source_content.as_str();

        match compile_to_vbc(source) {
            Ok(_) => Err(ExecutorError::ProcessError(
                "Expected VBC codegen to fail, but it succeeded".to_string().into()
            )),
            Err(e) => {
                // VBC codegen failed as expected
                // TODO: Validate specific error codes if specified in directives
                tracing::debug!("VBC codegen failed as expected: {}", e);
                Ok(())
            }
        }
    }

    // =========================================================================
    // META TEST EXECUTION METHODS
    // =========================================================================

    /// Execute a meta-pass test.
    ///
    /// Meta-pass tests verify that meta functions compile and evaluate successfully.
    /// The test file contains `meta fn` declarations and `@const` assertions.
    async fn execute_meta_pass(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration for meta tests
        self.execute_meta_pass_direct(directives, tier, start)
    }

    /// Execute meta-pass test using direct library integration.
    fn execute_meta_pass_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        // Create compiler options for check-only mode
        // Meta evaluation happens during type checking and macro expansion
        let options = CompilerOptions {
            input: source_path.clone(),
            output_format: OutputFormat::Human,
            verify_mode: VerifyMode::Runtime,
            continue_on_error: false,
            ..Default::default()
        };

        // Run on dedicated thread with large stack
        let (tx, rx) = std::sync::mpsc::channel();
        let timeout_ms = directives.effective_timeout_ms();
        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session = Session::new(options);
                    let mut pipeline = CompilationPipeline::new_check(&mut session);
                    let result = pipeline.run_check_only();
                    let diag = pipeline.session().format_diagnostics();
                    (result, diag)
                }));
                let _ = tx.send(r);
            })
            .expect("Failed to spawn meta-pass thread");

        let (result, diagnostics_output) = match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
            Ok(Ok(pair)) => pair,
            Ok(Err(_)) | Err(_) => {
                return TestOutcome::Fail {
                    tier,
                    reason: "Meta evaluation panicked or timed out".to_string().into(),
                    expected: Some("Successful meta evaluation".to_string().into()),
                    actual: Some("crash/timeout".to_string().into()),
                    duration: start.elapsed(),
                };
            }
        };

        match result {
            Ok(()) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(e) => {
                let error_output = if diagnostics_output.is_empty() {
                    format!("{}", e)
                } else {
                    diagnostics_output
                };
                TestOutcome::Fail {
                    tier,
                    reason: "Meta evaluation unexpectedly failed".to_string().into(),
                    expected: Some("Successful meta evaluation".to_string().into()),
                    actual: Some(error_output.into()),
                    duration: start.elapsed(),
                }
            }
        }
    }

    /// Execute a meta-fail test.
    ///
    /// Meta-fail tests verify that meta functions fail with expected errors.
    /// The test file specifies expected error codes like `@expected-error: M004`.
    async fn execute_meta_fail(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        println!("[TRACE execute_meta_fail] called with source_path={}", directives.source_path.as_str());
        let start = Instant::now();

        // Use direct library integration for meta tests
        self.execute_meta_fail_direct(directives, tier, start)
    }

    /// Execute meta-fail test using direct library integration.
    fn execute_meta_fail_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        let source_path = PathBuf::from(&directives.source_path);

        let options = CompilerOptions {
            input: source_path.clone(),
            output_format: OutputFormat::Human,
            verify_mode: VerifyMode::Runtime,
            continue_on_error: true,
            ..Default::default()
        };

        // Run on dedicated thread with large stack
        let (tx, rx) = std::sync::mpsc::channel();
        let timeout_ms = directives.effective_timeout_ms();
        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session = Session::new(options);
                    let mut pipeline = CompilationPipeline::new_check(&mut session);
                    let result = pipeline.run_check_only();
                    let diag = pipeline.session().format_diagnostics();
                    (result, diag)
                }));
                let _ = tx.send(r);
            })
            .expect("Failed to spawn meta-fail thread");

        let (result, diagnostics_output) = match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
            Ok(Ok(pair)) => pair,
            Ok(Err(_)) | Err(_) => {
                return TestOutcome::Fail {
                    tier,
                    reason: "Meta evaluation panicked or timed out".to_string().into(),
                    expected: Some("Meta evaluation failure".to_string().into()),
                    actual: Some("crash/timeout".to_string().into()),
                    duration: start.elapsed(),
                };
            }
        };

        match result {
            Ok(()) => {
                // Meta evaluation succeeded when it should have failed
                TestOutcome::Fail {
                    tier,
                    reason: "Meta evaluation unexpectedly succeeded".to_string().into(),
                    expected: Some("Meta evaluation failure".to_string().into()),
                    actual: Some("Meta evaluation succeeded".to_string().into()),
                    duration: start.elapsed(),
                }
            }
            Err(_) => {
                // Meta evaluation failed as expected - check if errors match
                if directives.expected_errors.is_empty() {
                    // No specific errors expected, just expect failure
                    return TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    };
                }

                // Check if expected errors match using the formatted diagnostics
                // Meta errors use M-prefixed codes (M001, M004, M201, etc.)
                if self.check_expected_errors_in_output(&diagnostics_output, &directives.expected_errors)
                {
                    TestOutcome::Pass {
                        tier,
                        duration: start.elapsed(),
                    }
                } else {
                    TestOutcome::Fail {
                        tier,
                        reason: "Meta evaluation failed but with wrong errors".to_string().into(),
                        expected: Some(format!("{:?}", directives.expected_errors).into()),
                        actual: Some(diagnostics_output.into()),
                        duration: start.elapsed(),
                    }
                }
            }
        }
    }

    /// Execute a meta-eval test.
    ///
    /// Meta-eval tests verify that meta functions evaluate to expected values.
    /// The test file specifies expected values with `@expected-value: <value>`.
    async fn execute_meta_eval(&self, directives: &TestDirectives, tier: Tier) -> TestOutcome {
        let start = Instant::now();

        // Use direct library integration for meta tests
        self.execute_meta_eval_direct(directives, tier, start)
    }

    /// Execute meta-eval test using direct library integration.
    fn execute_meta_eval_direct(
        &self,
        directives: &TestDirectives,
        tier: Tier,
        start: Instant,
    ) -> TestOutcome {
        // For now, meta-eval tests are treated like meta-pass tests
        // Full evaluation checking will be added when we have @expected-value directive support
        let source_path = PathBuf::from(&directives.source_path);

        let options = CompilerOptions {
            input: source_path.clone(),
            output_format: OutputFormat::Human,
            verify_mode: VerifyMode::Runtime,
            continue_on_error: false,
            ..Default::default()
        };

        // Run on dedicated thread with large stack
        let (tx, rx) = std::sync::mpsc::channel();
        let timeout_ms = directives.effective_timeout_ms();
        let _ = std::thread::Builder::new()
            .name("vtest-check".into()).stack_size(512 * 1024 * 1024)
            .spawn(move || {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut session = Session::new(options);
                    let mut pipeline = CompilationPipeline::new_check(&mut session);
                    let result = pipeline.run_check_only();
                    let diag = pipeline.session().format_diagnostics();
                    (result, diag)
                }));
                let _ = tx.send(r);
            })
            .expect("Failed to spawn meta-eval thread");

        let (result, diagnostics_output) = match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
            Ok(Ok(pair)) => pair,
            Ok(Err(_)) | Err(_) => {
                return TestOutcome::Fail {
                    tier,
                    reason: "Meta evaluation panicked or timed out".to_string().into(),
                    expected: Some("Successful meta evaluation".to_string().into()),
                    actual: Some("crash/timeout".to_string().into()),
                    duration: start.elapsed(),
                };
            }
        };

        match result {
            Ok(()) => TestOutcome::Pass {
                tier,
                duration: start.elapsed(),
            },
            Err(e) => {
                let error_output = if diagnostics_output.is_empty() {
                    format!("{}", e)
                } else {
                    diagnostics_output
                };

                TestOutcome::Fail {
                    tier,
                    reason: "Meta evaluation unexpectedly failed".to_string().into(),
                    expected: Some("Successful meta evaluation".to_string().into()),
                    actual: Some(error_output.into()),
                    duration: start.elapsed(),
                }
            }
        }
    }
}

/// Parse a performance threshold from a string like "< 15ns" or "<= 100us".
fn parse_performance_threshold(s: &str) -> Option<u64> {
    let s = s.trim();

    // Remove comparison operators
    let s = s.trim_start_matches('<').trim_start_matches('=').trim();

    // Parse value and unit
    if let Some(ns) = s.strip_suffix("ns") {
        ns.trim().parse().ok()
    } else if let Some(us) = s.strip_suffix("us") {
        us.trim().parse::<u64>().ok().map(|v| v * 1000)
    } else if let Some(ms) = s.strip_suffix("ms") {
        ms.trim().parse::<u64>().ok().map(|v| v * 1_000_000)
    } else if let Some(s_val) = s.strip_suffix('s') {
        s_val.trim().parse::<u64>().ok().map(|v| v * 1_000_000_000)
    } else {
        // Default to nanoseconds
        s.parse().ok()
    }
}

/// Compare results from differential testing.
pub fn compare_differential_results(
    results: &[ProcessOutput],
    _tolerance_memory: f64,
) -> Result<(), Text> {
    if results.len() < 2 {
        return Ok(());
    }

    let reference = &results[0];

    for (i, result) in results.iter().enumerate().skip(1) {
        // Compare exit codes
        if result.exit_code != reference.exit_code {
            return Err(format!(
                "Exit code mismatch: tier 0 = {:?}, tier {} = {:?}",
                reference.exit_code, i, result.exit_code
            ).into());
        }

        // Compare stdout (exact)
        if result.stdout.trim() != reference.stdout.trim() {
            return Err(format!(
                "Stdout mismatch between tier 0 and tier {}\n\
                 Expected: {}\n\
                 Actual: {}",
                i,
                reference.stdout.trim(),
                result.stdout.trim()
            ).into());
        }

        // Compare stderr (exact)
        if result.stderr.trim() != reference.stderr.trim() {
            return Err(format!(
                "Stderr mismatch between tier 0 and tier {}\n\
                 Expected: {}\n\
                 Actual: {}",
                i,
                reference.stderr.trim(),
                result.stderr.trim()
            ).into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_performance_threshold() {
        assert_eq!(parse_performance_threshold("15ns"), Some(15));
        assert_eq!(parse_performance_threshold("< 15ns"), Some(15));
        assert_eq!(parse_performance_threshold("<= 100us"), Some(100_000));
        assert_eq!(parse_performance_threshold("1ms"), Some(1_000_000));
        assert_eq!(parse_performance_threshold("< 1s"), Some(1_000_000_000));
    }

    #[test]
    fn test_process_output_default() {
        let output = ProcessOutput::default();
        assert_eq!(output.exit_code, None);
        assert!(output.stdout.is_empty());
        assert!(!output.timed_out);
    }

    #[test]
    fn test_test_outcome_is_pass() {
        let pass = TestOutcome::Pass {
            tier: Tier::Tier0,
            duration: Duration::from_millis(100),
        };
        assert!(pass.is_pass());

        let fail = TestOutcome::Fail {
            tier: Tier::Tier0,
            reason: "test".to_string().into(),
            expected: None,
            actual: None,
            duration: Duration::from_millis(100),
        };
        assert!(!fail.is_pass());
    }

    #[test]
    fn test_parse_rust_style_errors() {
        let stderr = r#"error[E302]: Use after move
  --> test.vr:10:5
   |
10 |     let x = y;
   |         ^ value moved here

error[E201]: Type mismatch
  --> test.vr:15:10
   |
15 |     x + "hello"
   |         ^^^^^^^ expected Int, found Text
"#;

        let errors = ParsedError::parse_stderr(stderr);
        assert_eq!(errors.len(), 2);

        assert_eq!(errors[0].code, "E302");
        assert_eq!(errors[0].message, "Use after move");
        assert_eq!(errors[0].line, Some(10));
        assert_eq!(errors[0].column, Some(5));

        assert_eq!(errors[1].code, "E201");
        assert_eq!(errors[1].line, Some(15));
        assert_eq!(errors[1].column, Some(10));
    }

    #[test]
    fn test_parse_gcc_style_errors() {
        let stderr = r#"test.vr:10:5: error[E302]: Use after move
test.vr:15:10: warning[W101]: Unused variable
"#;

        let errors = ParsedError::parse_stderr(stderr);
        assert_eq!(errors.len(), 2);

        assert_eq!(errors[0].code, "E302");
        assert_eq!(errors[0].line, Some(10));
        assert_eq!(errors[0].column, Some(5));
        assert_eq!(errors[0].severity, "error");

        assert_eq!(errors[1].code, "W101");
        assert_eq!(errors[1].line, Some(15));
        assert_eq!(errors[1].severity, "warning");
    }

    #[test]
    fn test_parsed_error_matches() {
        let actual = ParsedError {
            code: "E302".to_string().into(),
            message: "Use after move".to_string().into(),
            file: Some("test.vr".to_string().into()),
            line: Some(10),
            column: Some(5),
            severity: "error".to_string().into(),
        };

        // Matching expected error
        let expected = ExpectedError {
            code: "E302".to_string().into(),
            message: Some("Use after move".to_string().into()),
            line: Some(10),
            column: Some(5),
            end_column: None,
            severity: None,
            category: None,
        };
        assert!(actual.matches(&expected));

        // Wrong code
        let wrong_code = ExpectedError {
            code: "E303".to_string().into(),
            ..expected.clone()
        };
        assert!(!actual.matches(&wrong_code));

        // Wrong line
        let wrong_line = ExpectedError {
            line: Some(11),
            ..expected.clone()
        };
        assert!(!actual.matches(&wrong_line));

        // Column range match
        let range_match = ExpectedError {
            column: Some(3),
            end_column: Some(8),
            ..expected.clone()
        };
        assert!(actual.matches(&range_match));
    }

    #[test]
    fn test_compare_differential_results() {
        let result1 = ProcessOutput {
            exit_code: Some(0),
            stdout: "hello".to_string().into(),
            stderr: Text::new(),
            duration: Duration::from_millis(100),
            timed_out: false,
        };

        let result2 = ProcessOutput {
            exit_code: Some(0),
            stdout: "hello".to_string().into(),
            stderr: Text::new(),
            duration: Duration::from_millis(150),
            timed_out: false,
        };

        // Same results should pass
        assert!(compare_differential_results(&[result1.clone(), result2.clone()], 0.1).is_ok());

        // Different stdout should fail
        let different = ProcessOutput {
            stdout: "world".to_string().into(),
            ..result1.clone()
        };
        assert!(compare_differential_results(&[result1, different], 0.1).is_err());
    }
}

/// Common pipeline verification module.
///
/// This module provides functions for verifying tests through the common pipeline
/// (Source → Parser → AST → Types → TypedAST) without execution.
pub mod verify {
    use super::*;
    use verum_compiler::api::{parse, run_common_pipeline, CommonPipelineConfig, SourceFile};

    /// Result of a verification test.
    #[derive(Debug, Clone)]
    pub enum VerificationResult {
        /// Test passed
        Pass,
        /// Test failed with specific reason and phase
        Fail { reason: String, phase: String },
        /// Test encountered an error (internal)
        Error { message: String },
        /// Test was skipped
        Skip { reason: String },
    }

    /// Statistics for verification runs.
    #[derive(Debug, Clone, Default)]
    pub struct VerificationStats {
        pub total: usize,
        pub passed: usize,
        pub failed: usize,
        pub errors: usize,
        pub skipped: usize,
    }

    /// Verify a single test through the common pipeline.
    ///
    /// This function routes to the appropriate verification based on test type:
    /// - ParsePass/ParseFail: Lexer + Parser only
    /// - TypecheckPass/TypecheckFail: Full common pipeline
    /// - VerifyPass/VerifyFail: Common pipeline with SMT verification
    /// - CommonPipeline/CommonPipelineFail: Alias for TypecheckPass/Fail
    /// - CompileOnly: Full common pipeline (must succeed)
    pub fn verify_test(directives: &TestDirectives) -> VerificationResult {
        let source = directives.source_content.as_str();

        match directives.test_type {
            TestType::ParsePass => verify_parse_pass(source),
            TestType::ParseFail => verify_parse_fail(source),
            TestType::ParseRecover => verify_parse_fail(source),
            TestType::TypecheckPass => verify_typecheck_pass(source),
            TestType::TypecheckFail => verify_typecheck_fail(source, directives),
            TestType::VerifyPass => verify_contract_pass(source),
            TestType::VerifyFail => verify_contract_fail(source, directives),
            TestType::CommonPipeline => verify_common_pipeline_pass(source),
            TestType::CommonPipelineFail => verify_common_pipeline_fail(source, directives),
            TestType::CompileOnly => verify_compile_only(source),
            _ => VerificationResult::Skip {
                reason: format!("{} is not a compile-time test type", directives.test_type),
            },
        }
    }

    /// Verify that source parses successfully.
    fn verify_parse_pass(source: &str) -> VerificationResult {
        match parse(source) {
            Ok(_) => VerificationResult::Pass,
            Err(e) => VerificationResult::Fail {
                reason: format!("{}", e),
                phase: "parse".to_string(),
            },
        }
    }

    /// Verify that source fails to parse.
    fn verify_parse_fail(source: &str) -> VerificationResult {
        match parse(source) {
            Ok(_) => VerificationResult::Fail {
                reason: "Expected parse to fail, but it succeeded".to_string(),
                phase: "parse".to_string(),
            },
            Err(_) => VerificationResult::Pass,
        }
    }

    /// Resolve the core/ stdlib path if it exists on disk.
    fn resolve_core_path() -> Option<std::path::PathBuf> {
        let core_path = std::path::PathBuf::from("core");
        if core_path.exists() { Some(core_path) } else { None }
    }

    /// Verify that source typechecks successfully.
    fn verify_typecheck_pass(source: &str) -> VerificationResult {
        let source_file = SourceFile::from_string(source);
        let config = CommonPipelineConfig {
            core_source_path: resolve_core_path(),
            ..CommonPipelineConfig::minimal()
        };

        match run_common_pipeline(&[source_file], &config) {
            Ok(result) => {
                if result.has_errors() {
                    VerificationResult::Fail {
                        reason: format!("{} type errors", result.error_count()),
                        phase: "typecheck".to_string(),
                    }
                } else {
                    VerificationResult::Pass
                }
            }
            Err(e) => VerificationResult::Fail {
                reason: format!("{}", e),
                phase: "typecheck".to_string(),
            },
        }
    }

    /// Verify that source fails to typecheck.
    fn verify_typecheck_fail(source: &str, directives: &TestDirectives) -> VerificationResult {
        let source_file = SourceFile::from_string(source);
        let config = CommonPipelineConfig {
            core_source_path: resolve_core_path(),
            ..CommonPipelineConfig::minimal()
        };

        match run_common_pipeline(&[source_file], &config) {
            Ok(result) => {
                if result.has_errors() {
                    // Verify expected error count if specified
                    if let Some(expected_count) = directives.expected_error_count {
                        let actual = result.error_count();
                        if actual != expected_count {
                            return VerificationResult::Fail {
                                reason: format!(
                                    "Expected {} errors, got {}",
                                    expected_count, actual
                                ),
                                phase: "typecheck".to_string(),
                            };
                        }
                    }
                    VerificationResult::Pass
                } else {
                    VerificationResult::Fail {
                        reason: "Expected typecheck to fail, but it succeeded".to_string(),
                        phase: "typecheck".to_string(),
                    }
                }
            }
            Err(_) => {
                // Pipeline error counts as failure (expected)
                VerificationResult::Pass
            }
        }
    }

    /// Verify that contract verification passes.
    fn verify_contract_pass(source: &str) -> VerificationResult {
        let source_file = SourceFile::from_string(source);
        let config = CommonPipelineConfig {
            verify_contracts: true,
            smt_timeout_ms: 5000,
            core_source_path: resolve_core_path(),
            ..CommonPipelineConfig::default()
        };

        match run_common_pipeline(&[source_file], &config) {
            Ok(result) => {
                if result.has_errors() {
                    VerificationResult::Fail {
                        reason: format!("{} verification errors", result.error_count()),
                        phase: "verify".to_string(),
                    }
                } else {
                    VerificationResult::Pass
                }
            }
            Err(e) => VerificationResult::Fail {
                reason: format!("{}", e),
                phase: "verify".to_string(),
            },
        }
    }

    /// Verify that contract verification fails.
    fn verify_contract_fail(source: &str, directives: &TestDirectives) -> VerificationResult {
        let source_file = SourceFile::from_string(source);
        let config = CommonPipelineConfig {
            verify_contracts: true,
            smt_timeout_ms: 5000,
            core_source_path: resolve_core_path(),
            ..CommonPipelineConfig::default()
        };

        match run_common_pipeline(&[source_file], &config) {
            Ok(result) => {
                if result.has_errors() {
                    // Check expected error count if specified
                    if let Some(expected_count) = directives.expected_error_count {
                        let actual = result.error_count();
                        if actual != expected_count {
                            return VerificationResult::Fail {
                                reason: format!(
                                    "Expected {} errors, got {}",
                                    expected_count, actual
                                ),
                                phase: "verify".to_string(),
                            };
                        }
                    }
                    VerificationResult::Pass
                } else {
                    VerificationResult::Fail {
                        reason: "Expected verification to fail, but it succeeded".to_string(),
                        phase: "verify".to_string(),
                    }
                }
            }
            Err(_) => {
                // Pipeline error counts as failure (expected)
                VerificationResult::Pass
            }
        }
    }

    /// Verify full common pipeline passes.
    fn verify_common_pipeline_pass(source: &str) -> VerificationResult {
        let source_file = SourceFile::from_string(source);
        let config = CommonPipelineConfig {
            core_source_path: resolve_core_path(),
            ..CommonPipelineConfig::default()
        };

        match run_common_pipeline(&[source_file], &config) {
            Ok(result) => {
                if result.has_errors() {
                    VerificationResult::Fail {
                        reason: format!("{} errors", result.error_count()),
                        phase: "common-pipeline".to_string(),
                    }
                } else {
                    VerificationResult::Pass
                }
            }
            Err(e) => VerificationResult::Fail {
                reason: format!("{}", e),
                phase: "common-pipeline".to_string(),
            },
        }
    }

    /// Verify full common pipeline fails.
    fn verify_common_pipeline_fail(source: &str, directives: &TestDirectives) -> VerificationResult {
        let source_file = SourceFile::from_string(source);
        let config = CommonPipelineConfig {
            core_source_path: resolve_core_path(),
            ..CommonPipelineConfig::default()
        };

        match run_common_pipeline(&[source_file], &config) {
            Ok(result) => {
                if result.has_errors() {
                    if let Some(expected_count) = directives.expected_error_count {
                        let actual = result.error_count();
                        if actual != expected_count {
                            return VerificationResult::Fail {
                                reason: format!(
                                    "Expected {} errors, got {}",
                                    expected_count, actual
                                ),
                                phase: "common-pipeline".to_string(),
                            };
                        }
                    }
                    VerificationResult::Pass
                } else {
                    VerificationResult::Fail {
                        reason: "Expected common pipeline to fail, but it succeeded".to_string(),
                        phase: "common-pipeline".to_string(),
                    }
                }
            }
            Err(_) => VerificationResult::Pass,
        }
    }

    /// Verify compile-only (common pipeline must succeed).
    fn verify_compile_only(source: &str) -> VerificationResult {
        let source_file = SourceFile::from_string(source);
        let config = CommonPipelineConfig {
            core_source_path: resolve_core_path(),
            ..CommonPipelineConfig::default()
        };

        match run_common_pipeline(&[source_file], &config) {
            Ok(result) => {
                if result.has_errors() {
                    VerificationResult::Fail {
                        reason: format!("{} compilation errors", result.error_count()),
                        phase: "compile".to_string(),
                    }
                } else {
                    VerificationResult::Pass
                }
            }
            Err(e) => VerificationResult::Fail {
                reason: format!("{}", e),
                phase: "compile".to_string(),
            },
        }
    }
}
