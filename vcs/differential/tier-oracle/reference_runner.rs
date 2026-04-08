//! Reference Implementation Runner for Tier Oracle
//!
//! This module provides the core infrastructure for running Verum programs
//! across different execution tiers and comparing their outputs.
//!
//! # Architecture
//!
//! The reference runner establishes Tier 0 (interpreter) as the canonical
//! implementation. All other tiers must match Tier 0 behavior exactly.
//!
//! ```text
//! +-------------------+     +-------------------+
//! |    Source File    | --> |  Tier 0 (Interp)  | --> Reference Output
//! +-------------------+     +-------------------+
//!           |
//!           v
//! +-------------------+     +-------------------+
//! |   Same Source     | --> |  Tier 3 (AOT)     | --> Test Output
//! +-------------------+     +-------------------+
//!           |
//!           v
//! +-------------------+
//! | Output Comparator | --> Pass/Fail + Divergence Report
//! +-------------------+
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Execution tier identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionTier {
    /// Tier 0: Direct AST interpretation (reference)
    Interpreter,
    /// Tier 1: Bytecode compilation and execution
    Bytecode,
    /// Tier 2: Just-in-time compilation
    Jit,
    /// Tier 3: Ahead-of-time LLVM compilation
    Aot,
}

impl ExecutionTier {
    /// Get the tier number
    pub fn number(&self) -> u8 {
        match self {
            ExecutionTier::Interpreter => 0,
            ExecutionTier::Bytecode => 1,
            ExecutionTier::Jit => 2,
            ExecutionTier::Aot => 3,
        }
    }

    /// Get the tier name
    pub fn name(&self) -> &'static str {
        match self {
            ExecutionTier::Interpreter => "interpreter",
            ExecutionTier::Bytecode => "bytecode",
            ExecutionTier::Jit => "jit",
            ExecutionTier::Aot => "aot",
        }
    }

    /// Get the default binary name
    pub fn default_binary(&self) -> &'static str {
        match self {
            ExecutionTier::Interpreter => "verum-interpret",
            ExecutionTier::Bytecode => "verum-bc",
            ExecutionTier::Jit => "verum-jit",
            ExecutionTier::Aot => "verum-run",
        }
    }

    /// Create from tier number
    pub fn from_number(n: u8) -> Option<Self> {
        match n {
            0 => Some(ExecutionTier::Interpreter),
            1 => Some(ExecutionTier::Bytecode),
            2 => Some(ExecutionTier::Jit),
            3 => Some(ExecutionTier::Aot),
            _ => None,
        }
    }
}

/// Configuration for the reference runner
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceRunnerConfig {
    /// Binary paths for each tier
    pub tier_binaries: HashMap<ExecutionTier, PathBuf>,

    /// Reference tier (default: Interpreter)
    pub reference_tier: ExecutionTier,

    /// Tiers to test against reference
    pub test_tiers: Vec<ExecutionTier>,

    /// Default timeout in milliseconds
    pub timeout_ms: u64,

    /// Working directory
    pub work_dir: PathBuf,

    /// Environment variables
    pub env_vars: HashMap<String, String>,

    /// Whether to capture stderr
    pub capture_stderr: bool,

    /// Number of retries for flaky tests
    pub retry_count: usize,

    /// Comparison configuration
    pub comparison_config: ComparisonConfig,
}

impl Default for ReferenceRunnerConfig {
    fn default() -> Self {
        let mut tier_binaries = HashMap::new();
        tier_binaries.insert(ExecutionTier::Interpreter, PathBuf::from("verum-interpret"));
        tier_binaries.insert(ExecutionTier::Bytecode, PathBuf::from("verum-bc"));
        tier_binaries.insert(ExecutionTier::Jit, PathBuf::from("verum-jit"));
        tier_binaries.insert(ExecutionTier::Aot, PathBuf::from("verum-run"));

        Self {
            tier_binaries,
            reference_tier: ExecutionTier::Interpreter,
            test_tiers: vec![ExecutionTier::Aot],
            timeout_ms: 30_000,
            work_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            env_vars: HashMap::new(),
            capture_stderr: true,
            retry_count: 0,
            comparison_config: ComparisonConfig::default(),
        }
    }
}

/// Configuration for output comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonConfig {
    /// Float comparison epsilon
    pub float_epsilon: f64,

    /// Whether to normalize whitespace
    pub normalize_whitespace: bool,

    /// Whether to normalize line endings
    pub normalize_line_endings: bool,

    /// Whether to strip ANSI color codes
    pub strip_ansi_codes: bool,

    /// Whether to strip memory addresses
    pub strip_addresses: bool,

    /// Whether to allow float formatting differences
    pub allow_float_formatting: bool,

    /// Patterns to ignore in output
    pub ignore_patterns: Vec<String>,
}

impl Default for ComparisonConfig {
    fn default() -> Self {
        Self {
            float_epsilon: 1e-10,
            normalize_whitespace: false,
            normalize_line_endings: true,
            strip_ansi_codes: true,
            strip_addresses: true,
            allow_float_formatting: true,
            ignore_patterns: vec![],
        }
    }
}

/// Result of executing a program on a specific tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierExecutionResult {
    /// The tier that was executed
    pub tier: ExecutionTier,

    /// Standard output
    pub stdout: String,

    /// Standard error
    pub stderr: String,

    /// Exit code (None if terminated by signal)
    pub exit_code: Option<i32>,

    /// Execution duration in milliseconds
    pub duration_ms: u64,

    /// Whether execution was successful
    pub success: bool,

    /// Error message if execution failed
    pub error: Option<String>,
}

/// Result of comparing two tier executions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    /// Reference tier
    pub reference: ExecutionTier,

    /// Test tier
    pub test_tier: ExecutionTier,

    /// Whether outputs match
    pub match_result: MatchResult,

    /// Reference tier output
    pub reference_output: TierExecutionResult,

    /// Test tier output
    pub test_output: TierExecutionResult,

    /// Detailed differences (if any)
    pub differences: Vec<Difference>,
}

/// The result of a match comparison
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchResult {
    /// Outputs match exactly
    ExactMatch,

    /// Outputs match semantically (after normalization)
    SemanticMatch,

    /// Stdout differs
    StdoutMismatch,

    /// Stderr differs
    StderrMismatch,

    /// Exit codes differ
    ExitCodeMismatch,

    /// One or both tiers crashed
    Crash,

    /// Execution timed out
    Timeout,

    /// Execution error
    ExecutionError,
}

impl MatchResult {
    /// Whether the match result indicates success
    pub fn is_success(&self) -> bool {
        matches!(self, MatchResult::ExactMatch | MatchResult::SemanticMatch)
    }
}

/// A specific difference between outputs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Difference {
    /// Line number (1-indexed)
    pub line: usize,

    /// Column number (1-indexed, if applicable)
    pub column: Option<usize>,

    /// Expected value (from reference)
    pub expected: String,

    /// Actual value (from test tier)
    pub actual: String,

    /// Kind of difference
    pub kind: DifferenceKind,
}

/// Kind of difference between outputs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DifferenceKind {
    /// Text content differs
    TextDiff,

    /// Float precision differs
    FloatPrecision,

    /// Line count differs
    LineCount,

    /// Whitespace differs
    Whitespace,

    /// Memory address differs (should be normalized away)
    Address,

    /// Ordering differs (for unordered outputs)
    Ordering,
}

/// The reference implementation runner
pub struct ReferenceRunner {
    config: ReferenceRunnerConfig,
}

impl ReferenceRunner {
    /// Create a new reference runner with the given configuration
    pub fn new(config: ReferenceRunnerConfig) -> Self {
        Self { config }
    }

    /// Create a new reference runner with default configuration
    pub fn with_defaults() -> Self {
        Self::new(ReferenceRunnerConfig::default())
    }

    /// Run a program on all configured tiers and compare against reference
    pub fn run_differential(&self, source_path: &Path) -> Result<Vec<ComparisonResult>> {
        // Execute on reference tier
        let reference_result = self.execute_tier(self.config.reference_tier, source_path)?;

        // Execute on all test tiers and compare
        let mut results = Vec::new();
        for &test_tier in &self.config.test_tiers {
            let test_result = self.execute_tier(test_tier, source_path)?;
            let comparison = self.compare_results(&reference_result, &test_result);
            results.push(comparison);
        }

        Ok(results)
    }

    /// Execute a program on a specific tier
    pub fn execute_tier(&self, tier: ExecutionTier, source_path: &Path) -> Result<TierExecutionResult> {
        let binary = self.config.tier_binaries.get(&tier)
            .ok_or_else(|| anyhow::anyhow!("No binary configured for tier {:?}", tier))?;

        let start = Instant::now();

        let mut cmd = Command::new(binary);
        cmd.arg(source_path)
            .current_dir(&self.config.work_dir)
            .stdout(Stdio::piped())
            .stderr(if self.config.capture_stderr {
                Stdio::piped()
            } else {
                Stdio::null()
            });

        // Set environment variables
        for (key, value) in &self.config.env_vars {
            cmd.env(key, value);
        }

        // Execute with timeout
        let output = self.execute_with_timeout(&mut cmd, Duration::from_millis(self.config.timeout_ms))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(TierExecutionResult {
            tier,
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code(),
            duration_ms,
            success: output.status.success(),
            error: None,
        })
    }

    /// Execute command with timeout
    fn execute_with_timeout(&self, cmd: &mut Command, timeout: Duration) -> Result<Output> {
        use std::io::Read;

        let mut child = cmd.spawn().context("Failed to spawn process")?;

        let result = wait_timeout::ChildExt::wait_timeout(&mut child, timeout)?;

        match result {
            Some(status) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();

                if let Some(mut stdout_handle) = child.stdout.take() {
                    stdout_handle.read_to_end(&mut stdout)?;
                }
                if let Some(mut stderr_handle) = child.stderr.take() {
                    stderr_handle.read_to_end(&mut stderr)?;
                }

                Ok(Output {
                    status,
                    stdout,
                    stderr,
                })
            }
            None => {
                child.kill()?;
                bail!("Process timed out after {:?}", timeout);
            }
        }
    }

    /// Compare results from two tier executions
    fn compare_results(&self, reference: &TierExecutionResult, test: &TierExecutionResult) -> ComparisonResult {
        let mut differences = Vec::new();

        // Check for execution errors
        if !reference.success && reference.exit_code.is_none() {
            return ComparisonResult {
                reference: reference.tier,
                test_tier: test.tier,
                match_result: MatchResult::Crash,
                reference_output: reference.clone(),
                test_output: test.clone(),
                differences,
            };
        }

        if !test.success && test.exit_code.is_none() {
            return ComparisonResult {
                reference: reference.tier,
                test_tier: test.tier,
                match_result: MatchResult::Crash,
                reference_output: reference.clone(),
                test_output: test.clone(),
                differences,
            };
        }

        // Compare exit codes
        if reference.exit_code != test.exit_code {
            differences.push(Difference {
                line: 0,
                column: None,
                expected: format!("{:?}", reference.exit_code),
                actual: format!("{:?}", test.exit_code),
                kind: DifferenceKind::TextDiff,
            });

            return ComparisonResult {
                reference: reference.tier,
                test_tier: test.tier,
                match_result: MatchResult::ExitCodeMismatch,
                reference_output: reference.clone(),
                test_output: test.clone(),
                differences,
            };
        }

        // Normalize and compare stdout
        let ref_stdout = self.normalize_output(&reference.stdout);
        let test_stdout = self.normalize_output(&test.stdout);

        if ref_stdout != test_stdout {
            differences = self.compute_differences(&ref_stdout, &test_stdout);

            // Check if differences are acceptable (semantic match)
            if self.is_semantic_match(&differences) {
                return ComparisonResult {
                    reference: reference.tier,
                    test_tier: test.tier,
                    match_result: MatchResult::SemanticMatch,
                    reference_output: reference.clone(),
                    test_output: test.clone(),
                    differences,
                };
            }

            return ComparisonResult {
                reference: reference.tier,
                test_tier: test.tier,
                match_result: MatchResult::StdoutMismatch,
                reference_output: reference.clone(),
                test_output: test.clone(),
                differences,
            };
        }

        // Compare stderr if configured
        if self.config.capture_stderr {
            let ref_stderr = self.normalize_output(&reference.stderr);
            let test_stderr = self.normalize_output(&test.stderr);

            if ref_stderr != test_stderr {
                differences = self.compute_differences(&ref_stderr, &test_stderr);

                return ComparisonResult {
                    reference: reference.tier,
                    test_tier: test.tier,
                    match_result: MatchResult::StderrMismatch,
                    reference_output: reference.clone(),
                    test_output: test.clone(),
                    differences,
                };
            }
        }

        // All checks passed
        ComparisonResult {
            reference: reference.tier,
            test_tier: test.tier,
            match_result: MatchResult::ExactMatch,
            reference_output: reference.clone(),
            test_output: test.clone(),
            differences,
        }
    }

    /// Normalize output according to configuration
    fn normalize_output(&self, output: &str) -> String {
        let mut result = output.to_string();

        if self.config.comparison_config.normalize_line_endings {
            result = result.replace("\r\n", "\n");
        }

        if self.config.comparison_config.strip_ansi_codes {
            // Simple ANSI escape code stripping
            let ansi_regex = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
            result = ansi_regex.replace_all(&result, "").to_string();
        }

        if self.config.comparison_config.strip_addresses {
            // Strip memory addresses like 0x7fff1234abcd
            let addr_regex = regex::Regex::new(r"0x[0-9a-fA-F]{6,16}").unwrap();
            result = addr_regex.replace_all(&result, "<ADDRESS>").to_string();
        }

        if self.config.comparison_config.normalize_whitespace {
            // Normalize multiple spaces to single space
            let ws_regex = regex::Regex::new(r"[ \t]+").unwrap();
            result = ws_regex.replace_all(&result, " ").to_string();
            result = result.trim().to_string();
        }

        result
    }

    /// Compute differences between two outputs
    fn compute_differences(&self, expected: &str, actual: &str) -> Vec<Difference> {
        let expected_lines: Vec<&str> = expected.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();

        let mut differences = Vec::new();

        // Check line count
        if expected_lines.len() != actual_lines.len() {
            differences.push(Difference {
                line: 0,
                column: None,
                expected: format!("{} lines", expected_lines.len()),
                actual: format!("{} lines", actual_lines.len()),
                kind: DifferenceKind::LineCount,
            });
        }

        // Compare line by line
        let max_lines = expected_lines.len().max(actual_lines.len());
        for i in 0..max_lines {
            let exp_line = expected_lines.get(i).copied().unwrap_or("<missing>");
            let act_line = actual_lines.get(i).copied().unwrap_or("<missing>");

            if exp_line != act_line {
                // Check if it's a float precision difference
                let kind = if self.is_float_difference(exp_line, act_line) {
                    DifferenceKind::FloatPrecision
                } else {
                    DifferenceKind::TextDiff
                };

                differences.push(Difference {
                    line: i + 1,
                    column: None,
                    expected: exp_line.to_string(),
                    actual: act_line.to_string(),
                    kind,
                });
            }
        }

        differences
    }

    /// Check if a difference is just float precision
    fn is_float_difference(&self, expected: &str, actual: &str) -> bool {
        // Try to parse both as floats
        let exp_float: Result<f64, _> = expected.trim().parse();
        let act_float: Result<f64, _> = actual.trim().parse();

        match (exp_float, act_float) {
            (Ok(e), Ok(a)) => {
                let diff = (e - a).abs();
                diff <= self.config.comparison_config.float_epsilon
            }
            _ => false,
        }
    }

    /// Check if differences are acceptable for semantic match
    fn is_semantic_match(&self, differences: &[Difference]) -> bool {
        if !self.config.comparison_config.allow_float_formatting {
            return false;
        }

        // All differences must be float precision differences
        differences.iter().all(|d| matches!(d.kind, DifferenceKind::FloatPrecision))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_tier_from_number() {
        assert_eq!(ExecutionTier::from_number(0), Some(ExecutionTier::Interpreter));
        assert_eq!(ExecutionTier::from_number(3), Some(ExecutionTier::Aot));
        assert_eq!(ExecutionTier::from_number(4), None);
    }

    #[test]
    fn test_match_result_is_success() {
        assert!(MatchResult::ExactMatch.is_success());
        assert!(MatchResult::SemanticMatch.is_success());
        assert!(!MatchResult::StdoutMismatch.is_success());
    }

    #[test]
    fn test_normalize_line_endings() {
        let config = ReferenceRunnerConfig::default();
        let runner = ReferenceRunner::new(config);

        let input = "line1\r\nline2\r\n";
        let normalized = runner.normalize_output(input);
        assert_eq!(normalized, "line1\nline2\n");
    }

    #[test]
    fn test_normalize_addresses() {
        let config = ReferenceRunnerConfig::default();
        let runner = ReferenceRunner::new(config);

        let input = "ptr at 0x7fff12345678";
        let normalized = runner.normalize_output(input);
        assert_eq!(normalized, "ptr at <ADDRESS>");
    }
}
