//! Tier Oracle Runner
//!
//! This module provides the infrastructure for running the same test across
//! different execution tiers (Tier 0 interpreter vs Tier 3 AOT) and comparing
//! results to ensure semantic equivalence.
//!
//! The oracle approach uses the interpreter as the reference implementation,
//! and validates that AOT produces identical observable behavior.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

// ============================================================================
// Tier Configuration
// ============================================================================

/// Execution tier identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Tier {
    /// Tier 0: Direct AST interpretation
    Interpreter = 0,
    /// Tier 1: Bytecode compilation and execution
    Bytecode = 1,
    /// Tier 2: JIT compilation
    Jit = 2,
    /// Tier 3: Ahead-of-time LLVM compilation
    Aot = 3,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tier::Interpreter => write!(f, "Tier 0 (Interpreter)"),
            Tier::Bytecode => write!(f, "Tier 1 (Bytecode)"),
            Tier::Jit => write!(f, "Tier 2 (JIT)"),
            Tier::Aot => write!(f, "Tier 3 (AOT)"),
        }
    }
}

impl Tier {
    /// Get the numeric tier value
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }

    /// Create from numeric value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Tier::Interpreter),
            1 => Some(Tier::Bytecode),
            2 => Some(Tier::Jit),
            3 => Some(Tier::Aot),
            _ => None,
        }
    }
}

/// Configuration for a specific tier
#[derive(Debug, Clone)]
pub struct TierConfig {
    /// Tier identifier
    pub tier: Tier,
    /// Path to the executable
    pub executable: PathBuf,
    /// Additional arguments to pass
    pub args: Vec<String>,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Default timeout for this tier
    pub default_timeout: Duration,
    /// Whether this tier is available
    pub available: bool,
}

impl TierConfig {
    /// Create interpreter configuration
    pub fn interpreter() -> Self {
        Self {
            tier: Tier::Interpreter,
            executable: PathBuf::from("verum-interpret"),
            args: vec![],
            env: HashMap::new(),
            default_timeout: Duration::from_secs(60),
            available: true,
        }
    }

    /// Create bytecode configuration
    pub fn bytecode() -> Self {
        Self {
            tier: Tier::Bytecode,
            executable: PathBuf::from("verum-bc"),
            args: vec![],
            env: HashMap::new(),
            default_timeout: Duration::from_secs(30),
            available: true,
        }
    }

    /// Create JIT configuration
    pub fn jit() -> Self {
        Self {
            tier: Tier::Jit,
            executable: PathBuf::from("verum-jit"),
            args: vec![],
            env: HashMap::new(),
            default_timeout: Duration::from_secs(30),
            available: true,
        }
    }

    /// Create AOT configuration
    pub fn aot() -> Self {
        Self {
            tier: Tier::Aot,
            executable: PathBuf::from("verum-run"),
            args: vec!["--release".to_string()],
            env: HashMap::new(),
            default_timeout: Duration::from_secs(30),
            available: true,
        }
    }

    /// Set the executable path
    pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = path.into();
        self
    }

    /// Add an argument
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Set environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Check if the tier is available (executable exists)
    pub fn check_availability(&mut self) -> bool {
        self.available = which::which(&self.executable).is_ok();
        self.available
    }
}

// ============================================================================
// Execution Results
// ============================================================================

/// Result of executing a test on a single tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierExecution {
    /// Which tier this execution is from
    pub tier: Tier,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Exit code (None if killed/crashed)
    pub exit_code: Option<i32>,
    /// Whether execution succeeded (exit code 0)
    pub success: bool,
    /// Execution duration
    pub duration: Duration,
    /// Whether the process timed out
    pub timed_out: bool,
    /// Whether the process crashed (segfault, etc.)
    pub crashed: bool,
    /// Signal that killed the process (if any)
    pub signal: Option<i32>,
}

impl TierExecution {
    /// Create a successful execution result
    pub fn success(tier: Tier, stdout: String, stderr: String, duration: Duration) -> Self {
        Self {
            tier,
            stdout,
            stderr,
            exit_code: Some(0),
            success: true,
            duration,
            timed_out: false,
            crashed: false,
            signal: None,
        }
    }

    /// Create a failed execution result
    pub fn failure(
        tier: Tier,
        stdout: String,
        stderr: String,
        exit_code: i32,
        duration: Duration,
    ) -> Self {
        Self {
            tier,
            stdout,
            stderr,
            exit_code: Some(exit_code),
            success: false,
            duration,
            timed_out: false,
            crashed: false,
            signal: None,
        }
    }

    /// Create a timeout result
    pub fn timeout(tier: Tier, duration: Duration) -> Self {
        Self {
            tier,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            success: false,
            duration,
            timed_out: true,
            crashed: false,
            signal: None,
        }
    }

    /// Create a crash result
    pub fn crash(tier: Tier, stderr: String, signal: Option<i32>, duration: Duration) -> Self {
        Self {
            tier,
            stdout: String::new(),
            stderr,
            exit_code: None,
            success: false,
            duration,
            timed_out: false,
            crashed: true,
            signal,
        }
    }
}

// ============================================================================
// Oracle Comparison Results
// ============================================================================

/// The type of divergence detected
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivergenceType {
    /// Standard output differs
    StdoutDiffers,
    /// Standard error differs
    StderrDiffers,
    /// Exit codes differ
    ExitCodeDiffers,
    /// One tier succeeded, another failed
    SuccessStateDiffers,
    /// One tier timed out
    TimeoutDifference,
    /// One tier crashed
    CrashDifference,
    /// Float precision difference (acceptable within tolerance)
    FloatPrecision,
    /// Ordering difference (for unordered collections)
    OrderingDifference,
}

/// Detailed divergence information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Divergence {
    /// Type of divergence
    pub divergence_type: DivergenceType,
    /// Description of the divergence
    pub description: String,
    /// Expected value (from reference tier)
    pub expected: String,
    /// Actual value (from tested tier)
    pub actual: String,
    /// Line number where divergence occurred (if applicable)
    pub line: Option<usize>,
    /// Whether this divergence is acceptable
    pub acceptable: bool,
}

/// Result of comparing two tier executions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleResult {
    /// The test file path
    pub test_path: PathBuf,
    /// Reference tier execution (usually interpreter)
    pub reference: TierExecution,
    /// Tested tier execution (usually AOT)
    pub tested: TierExecution,
    /// Whether the results match (no divergences, or all acceptable)
    pub matches: bool,
    /// List of divergences found
    pub divergences: Vec<Divergence>,
    /// Performance comparison
    pub speedup: f64,
}

impl OracleResult {
    /// Check if the result is a pass (no unacceptable divergences)
    pub fn passed(&self) -> bool {
        self.matches && self.divergences.iter().all(|d| d.acceptable)
    }

    /// Get the number of divergences
    pub fn divergence_count(&self) -> usize {
        self.divergences.len()
    }

    /// Get only unacceptable divergences
    pub fn critical_divergences(&self) -> Vec<&Divergence> {
        self.divergences.iter().filter(|d| !d.acceptable).collect()
    }
}

// ============================================================================
// Oracle Runner
// ============================================================================

/// Configuration for the oracle runner
#[derive(Debug, Clone)]
pub struct OracleConfig {
    /// Reference tier configuration
    pub reference_tier: TierConfig,
    /// Tested tier configuration
    pub tested_tier: TierConfig,
    /// Float comparison epsilon
    pub float_epsilon: f64,
    /// Whether to allow unordered collection comparisons
    pub allow_unordered_collections: bool,
    /// Whether to normalize whitespace
    pub normalize_whitespace: bool,
    /// Whether to strip ANSI codes
    pub strip_ansi: bool,
    /// Whether to normalize line endings
    pub normalize_line_endings: bool,
    /// Working directory for test execution
    pub work_dir: PathBuf,
    /// Enable verbose output
    pub verbose: bool,
}

impl Default for OracleConfig {
    fn default() -> Self {
        Self {
            reference_tier: TierConfig::interpreter(),
            tested_tier: TierConfig::aot(),
            float_epsilon: 1e-10,
            allow_unordered_collections: true,
            normalize_whitespace: false,
            strip_ansi: true,
            normalize_line_endings: true,
            work_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            verbose: false,
        }
    }
}

/// The main oracle runner
pub struct OracleRunner {
    config: OracleConfig,
}

impl OracleRunner {
    /// Create a new oracle runner with the given configuration
    pub fn new(config: OracleConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(OracleConfig::default())
    }

    /// Set the reference tier
    pub fn with_reference(mut self, tier: TierConfig) -> Self {
        self.config.reference_tier = tier;
        self
    }

    /// Set the tested tier
    pub fn with_tested(mut self, tier: TierConfig) -> Self {
        self.config.tested_tier = tier;
        self
    }

    /// Set float epsilon
    pub fn with_float_epsilon(mut self, epsilon: f64) -> Self {
        self.config.float_epsilon = epsilon;
        self
    }

    /// Enable verbose output
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.config.verbose = verbose;
        self
    }

    /// Run oracle test on a single file
    pub fn run(&self, test_path: &Path) -> Result<OracleResult> {
        // Execute on reference tier
        let reference = self.execute_tier(&self.config.reference_tier, test_path)?;

        // Execute on tested tier
        let tested = self.execute_tier(&self.config.tested_tier, test_path)?;

        // Compare results
        let (matches, divergences) = self.compare(&reference, &tested);

        // Calculate speedup
        let speedup = if tested.duration.as_nanos() > 0 {
            reference.duration.as_secs_f64() / tested.duration.as_secs_f64()
        } else {
            f64::INFINITY
        };

        Ok(OracleResult {
            test_path: test_path.to_path_buf(),
            reference,
            tested,
            matches,
            divergences,
            speedup,
        })
    }

    /// Run oracle tests on all files in a directory
    pub fn run_directory(&self, dir: &Path) -> Result<Vec<OracleResult>> {
        let mut results = Vec::new();

        for entry in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "vr") {
                match self.run(path) {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        if self.config.verbose {
                            eprintln!("Error running {}: {}", path.display(), e);
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    /// Execute a test on a specific tier
    fn execute_tier(&self, tier_config: &TierConfig, test_path: &Path) -> Result<TierExecution> {
        let start = Instant::now();

        let mut cmd = Command::new(&tier_config.executable);
        cmd.arg(test_path)
            .current_dir(&self.config.work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Add tier-specific arguments
        for arg in &tier_config.args {
            cmd.arg(arg);
        }

        // Set environment variables
        for (key, value) in &tier_config.env {
            cmd.env(key, value);
        }

        // Spawn and wait with timeout
        let mut child = cmd.spawn().with_context(|| {
            format!(
                "Failed to spawn {} for tier {:?}",
                tier_config.executable.display(),
                tier_config.tier
            )
        })?;

        let timeout = tier_config.default_timeout;

        match wait_timeout::ChildExt::wait_timeout(&mut child, timeout)? {
            Some(status) => {
                let duration = start.elapsed();
                let stdout = child
                    .stdout
                    .take()
                    .map(|mut s| {
                        use std::io::Read;
                        let mut buf = Vec::new();
                        s.read_to_end(&mut buf).ok();
                        String::from_utf8_lossy(&buf).to_string()
                    })
                    .unwrap_or_default();

                let stderr = child
                    .stderr
                    .take()
                    .map(|mut s| {
                        use std::io::Read;
                        let mut buf = Vec::new();
                        s.read_to_end(&mut buf).ok();
                        String::from_utf8_lossy(&buf).to_string()
                    })
                    .unwrap_or_default();

                if status.success() {
                    Ok(TierExecution::success(
                        tier_config.tier,
                        stdout,
                        stderr,
                        duration,
                    ))
                } else if let Some(code) = status.code() {
                    Ok(TierExecution::failure(
                        tier_config.tier,
                        stdout,
                        stderr,
                        code,
                        duration,
                    ))
                } else {
                    // Process was killed by signal
                    #[cfg(unix)]
                    let signal = {
                        use std::os::unix::process::ExitStatusExt;
                        status.signal()
                    };
                    #[cfg(not(unix))]
                    let signal = None;

                    Ok(TierExecution::crash(tier_config.tier, stderr, signal, duration))
                }
            }
            None => {
                // Timeout - kill the process
                let _ = child.kill();
                Ok(TierExecution::timeout(tier_config.tier, timeout))
            }
        }
    }

    /// Compare two tier executions
    fn compare(
        &self,
        reference: &TierExecution,
        tested: &TierExecution,
    ) -> (bool, Vec<Divergence>) {
        let mut divergences = Vec::new();

        // Check for crash/timeout differences first
        if reference.crashed != tested.crashed {
            divergences.push(Divergence {
                divergence_type: DivergenceType::CrashDifference,
                description: format!(
                    "Crash state differs: {} crashed={}, {} crashed={}",
                    reference.tier, reference.crashed, tested.tier, tested.crashed
                ),
                expected: format!("crashed={}", reference.crashed),
                actual: format!("crashed={}", tested.crashed),
                line: None,
                acceptable: false,
            });
        }

        if reference.timed_out != tested.timed_out {
            divergences.push(Divergence {
                divergence_type: DivergenceType::TimeoutDifference,
                description: format!(
                    "Timeout state differs: {} timed_out={}, {} timed_out={}",
                    reference.tier, reference.timed_out, tested.tier, tested.timed_out
                ),
                expected: format!("timed_out={}", reference.timed_out),
                actual: format!("timed_out={}", tested.timed_out),
                line: None,
                acceptable: false,
            });
        }

        // If either crashed or timed out, don't compare outputs
        if reference.crashed || tested.crashed || reference.timed_out || tested.timed_out {
            return (divergences.is_empty(), divergences);
        }

        // Check success state
        if reference.success != tested.success {
            divergences.push(Divergence {
                divergence_type: DivergenceType::SuccessStateDiffers,
                description: format!(
                    "Success state differs: {} success={}, {} success={}",
                    reference.tier, reference.success, tested.tier, tested.success
                ),
                expected: format!("success={}", reference.success),
                actual: format!("success={}", tested.success),
                line: None,
                acceptable: false,
            });
        }

        // Check exit codes
        if reference.exit_code != tested.exit_code {
            divergences.push(Divergence {
                divergence_type: DivergenceType::ExitCodeDiffers,
                description: format!(
                    "Exit code differs: {} = {:?}, {} = {:?}",
                    reference.tier, reference.exit_code, tested.tier, tested.exit_code
                ),
                expected: format!("{:?}", reference.exit_code),
                actual: format!("{:?}", tested.exit_code),
                line: None,
                acceptable: false,
            });
        }

        // Compare stdout
        let ref_stdout = self.normalize_output(&reference.stdout);
        let test_stdout = self.normalize_output(&tested.stdout);

        if ref_stdout != test_stdout {
            // Try semantic comparison for floats
            let (is_float_diff, line) = self.check_float_difference(&ref_stdout, &test_stdout);

            if is_float_diff {
                divergences.push(Divergence {
                    divergence_type: DivergenceType::FloatPrecision,
                    description: "Float precision difference detected".to_string(),
                    expected: ref_stdout.clone(),
                    actual: test_stdout.clone(),
                    line,
                    acceptable: true, // Float precision differences are often acceptable
                });
            } else {
                divergences.push(Divergence {
                    divergence_type: DivergenceType::StdoutDiffers,
                    description: "Standard output differs".to_string(),
                    expected: ref_stdout,
                    actual: test_stdout,
                    line: self.find_first_difference_line(&reference.stdout, &tested.stdout),
                    acceptable: false,
                });
            }
        }

        // Compare stderr (usually less strict)
        let ref_stderr = self.normalize_output(&reference.stderr);
        let test_stderr = self.normalize_output(&tested.stderr);

        if ref_stderr != test_stderr && !ref_stderr.is_empty() && !test_stderr.is_empty() {
            divergences.push(Divergence {
                divergence_type: DivergenceType::StderrDiffers,
                description: "Standard error differs".to_string(),
                expected: ref_stderr,
                actual: test_stderr,
                line: None,
                // Stderr differences are often acceptable (debug info, etc.)
                acceptable: true,
            });
        }

        let matches = divergences.is_empty() || divergences.iter().all(|d| d.acceptable);
        (matches, divergences)
    }

    /// Normalize output for comparison
    fn normalize_output(&self, output: &str) -> String {
        let mut result = output.to_string();

        if self.config.normalize_line_endings {
            result = result.replace("\r\n", "\n");
        }

        if self.config.strip_ansi {
            // Strip ANSI escape codes
            let ansi_re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
            result = ansi_re.replace_all(&result, "").to_string();
        }

        if self.config.normalize_whitespace {
            // Collapse multiple whitespace to single space
            let ws_re = regex::Regex::new(r"\s+").unwrap();
            result = ws_re.replace_all(&result, " ").to_string();
            result = result.trim().to_string();
        }

        result
    }

    /// Check if difference is only in float precision
    fn check_float_difference(&self, expected: &str, actual: &str) -> (bool, Option<usize>) {
        let exp_lines: Vec<&str> = expected.lines().collect();
        let act_lines: Vec<&str> = actual.lines().collect();

        if exp_lines.len() != act_lines.len() {
            return (false, None);
        }

        for (i, (exp, act)) in exp_lines.iter().zip(act_lines.iter()).enumerate() {
            if exp != act {
                // Try parsing as floats
                if let (Ok(exp_f), Ok(act_f)) = (exp.trim().parse::<f64>(), act.trim().parse::<f64>())
                {
                    if (exp_f - act_f).abs() > self.config.float_epsilon {
                        return (false, Some(i + 1));
                    }
                    // Float difference within epsilon
                    continue;
                }

                // Try parsing as space-separated floats
                let exp_parts: Vec<&str> = exp.split_whitespace().collect();
                let act_parts: Vec<&str> = act.split_whitespace().collect();

                if exp_parts.len() == act_parts.len() {
                    let mut all_floats = true;
                    for (e, a) in exp_parts.iter().zip(act_parts.iter()) {
                        if let (Ok(ef), Ok(af)) = (e.parse::<f64>(), a.parse::<f64>()) {
                            if (ef - af).abs() > self.config.float_epsilon {
                                return (false, Some(i + 1));
                            }
                        } else if e != a {
                            all_floats = false;
                            break;
                        }
                    }
                    if all_floats {
                        continue;
                    }
                }

                return (false, Some(i + 1));
            }
        }

        (true, None)
    }

    /// Find the first line where outputs differ
    fn find_first_difference_line(&self, a: &str, b: &str) -> Option<usize> {
        let a_lines: Vec<&str> = a.lines().collect();
        let b_lines: Vec<&str> = b.lines().collect();

        for (i, (la, lb)) in a_lines.iter().zip(b_lines.iter()).enumerate() {
            if la != lb {
                return Some(i + 1);
            }
        }

        if a_lines.len() != b_lines.len() {
            return Some(a_lines.len().min(b_lines.len()) + 1);
        }

        None
    }

    /// Generate a detailed report for an oracle result
    pub fn generate_report(&self, result: &OracleResult) -> String {
        let mut report = String::new();

        report.push_str(&format!(
            "=== Oracle Test Report ===\n\
             Test: {}\n\
             Status: {}\n\n",
            result.test_path.display(),
            if result.passed() { "PASS" } else { "FAIL" }
        ));

        report.push_str(&format!(
            "Reference ({}):\n\
             \tExit code: {:?}\n\
             \tDuration: {:?}\n\
             \tSuccess: {}\n\n",
            result.reference.tier,
            result.reference.exit_code,
            result.reference.duration,
            result.reference.success
        ));

        report.push_str(&format!(
            "Tested ({}):\n\
             \tExit code: {:?}\n\
             \tDuration: {:?}\n\
             \tSuccess: {}\n\
             \tSpeedup: {:.2}x\n\n",
            result.tested.tier,
            result.tested.exit_code,
            result.tested.duration,
            result.tested.success,
            result.speedup
        ));

        if !result.divergences.is_empty() {
            report.push_str("Divergences:\n");
            for (i, div) in result.divergences.iter().enumerate() {
                report.push_str(&format!(
                    "\n[{}] {:?} ({})\n\
                     \tDescription: {}\n\
                     \tExpected: {}\n\
                     \tActual: {}\n",
                    i + 1,
                    div.divergence_type,
                    if div.acceptable {
                        "acceptable"
                    } else {
                        "CRITICAL"
                    },
                    div.description,
                    div.expected,
                    div.actual
                ));
                if let Some(line) = div.line {
                    report.push_str(&format!("\tLine: {}\n", line));
                }
            }
        }

        report
    }
}

// ============================================================================
// Batch Runner
// ============================================================================

/// Summary of a batch run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSummary {
    /// Total number of tests
    pub total: usize,
    /// Number of passed tests
    pub passed: usize,
    /// Number of failed tests
    pub failed: usize,
    /// Number of tests with only acceptable divergences
    pub warnings: usize,
    /// Total duration
    pub duration: Duration,
    /// Average speedup (reference / tested)
    pub average_speedup: f64,
    /// Failed test paths
    pub failed_tests: Vec<PathBuf>,
}

impl BatchSummary {
    /// Create a new summary from results
    pub fn from_results(results: &[OracleResult], duration: Duration) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.divergences.is_empty()).count();
        let failed = results.iter().filter(|r| !r.passed()).count();
        let warnings = results
            .iter()
            .filter(|r| r.passed() && !r.divergences.is_empty())
            .count();

        let average_speedup = if !results.is_empty() {
            results.iter().map(|r| r.speedup).sum::<f64>() / results.len() as f64
        } else {
            1.0
        };

        let failed_tests = results
            .iter()
            .filter(|r| !r.passed())
            .map(|r| r.test_path.clone())
            .collect();

        Self {
            total,
            passed,
            failed,
            warnings,
            duration,
            average_speedup,
            failed_tests,
        }
    }

    /// Print a summary report
    pub fn print_summary(&self) {
        println!("\n=== Batch Oracle Summary ===");
        println!("Total:    {}", self.total);
        println!("Passed:   {} ({:.1}%)", self.passed, 100.0 * self.passed as f64 / self.total as f64);
        println!("Failed:   {} ({:.1}%)", self.failed, 100.0 * self.failed as f64 / self.total as f64);
        println!("Warnings: {}", self.warnings);
        println!("Duration: {:?}", self.duration);
        println!("Avg Speedup: {:.2}x", self.average_speedup);

        if !self.failed_tests.is_empty() {
            println!("\nFailed Tests:");
            for path in &self.failed_tests {
                println!("  - {}", path.display());
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_from_u8() {
        assert_eq!(Tier::from_u8(0), Some(Tier::Interpreter));
        assert_eq!(Tier::from_u8(3), Some(Tier::Aot));
        assert_eq!(Tier::from_u8(5), None);
    }

    #[test]
    fn test_tier_display() {
        assert_eq!(format!("{}", Tier::Interpreter), "Tier 0 (Interpreter)");
        assert_eq!(format!("{}", Tier::Aot), "Tier 3 (AOT)");
    }

    #[test]
    fn test_tier_execution_success() {
        let exec = TierExecution::success(
            Tier::Interpreter,
            "output".to_string(),
            "".to_string(),
            Duration::from_millis(100),
        );
        assert!(exec.success);
        assert_eq!(exec.exit_code, Some(0));
        assert!(!exec.crashed);
        assert!(!exec.timed_out);
    }

    #[test]
    fn test_tier_execution_timeout() {
        let exec = TierExecution::timeout(Tier::Aot, Duration::from_secs(30));
        assert!(!exec.success);
        assert!(exec.timed_out);
        assert!(exec.exit_code.is_none());
    }

    #[test]
    fn test_oracle_config_default() {
        let config = OracleConfig::default();
        assert_eq!(config.reference_tier.tier, Tier::Interpreter);
        assert_eq!(config.tested_tier.tier, Tier::Aot);
        assert!(config.strip_ansi);
        assert!(config.normalize_line_endings);
    }

    #[test]
    fn test_divergence_types() {
        let div = Divergence {
            divergence_type: DivergenceType::StdoutDiffers,
            description: "test".to_string(),
            expected: "a".to_string(),
            actual: "b".to_string(),
            line: Some(1),
            acceptable: false,
        };

        assert!(!div.acceptable);
        assert_eq!(div.line, Some(1));
    }

    #[test]
    fn test_batch_summary() {
        let results = vec![
            OracleResult {
                test_path: PathBuf::from("test1.vr"),
                reference: TierExecution::success(
                    Tier::Interpreter,
                    "ok".to_string(),
                    "".to_string(),
                    Duration::from_millis(100),
                ),
                tested: TierExecution::success(
                    Tier::Aot,
                    "ok".to_string(),
                    "".to_string(),
                    Duration::from_millis(10),
                ),
                matches: true,
                divergences: vec![],
                speedup: 10.0,
            },
        ];

        let summary = BatchSummary::from_results(&results, Duration::from_secs(1));
        assert_eq!(summary.total, 1);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 0);
    }
}
