//! Tier Oracle - Differential Testing Runner for Tier 0 vs Tier 3
//!
//! This module provides the main differential test runner that compares
//! execution results between Tier 0 (Interpreter) and Tier 3 (AOT) to
//! ensure semantic equivalence across compilation tiers.
//!
//! # Architecture
//!
//! ```text
//! +------------------------------------------------------------------+
//! |                     TIER ORACLE                                   |
//! +------------------------------------------------------------------+
//! |                                                                  |
//! |  +-------------------+    +-------------------+                  |
//! |  |    Executor       |    |    Comparator     |                  |
//! |  +-------------------+    +-------------------+                  |
//! |           |                        |                             |
//! |           v                        v                             |
//! |  +-------------------+    +-------------------+                  |
//! |  | Tier 0 Interpreter|    |   Output Diff     |                  |
//! |  | Tier 3 AOT        |    |   Exit Code       |                  |
//! |  +-------------------+    |   Behavior        |                  |
//! |           |               +-------------------+                  |
//! |           v                        |                             |
//! |  +-------------------+             v                             |
//! |  | Normalized Output |    +-------------------+                  |
//! |  +-------------------+    | Divergence Report |                  |
//! |                           +-------------------+                  |
//! +------------------------------------------------------------------+
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use tier_oracle::{TierOracle, OracleConfig, TierSpec};
//!
//! let oracle = TierOracle::new(OracleConfig {
//!     interpreter_path: PathBuf::from("verum-interpret"),
//!     aot_path: PathBuf::from("verum-run"),
//!     timeout_ms: 30_000,
//!     ..Default::default()
//! });
//!
//! let result = oracle.test_file(Path::new("test.vr"))?;
//!
//! if result.divergences.is_empty() {
//!     println!("Tiers agree!");
//! } else {
//!     for div in &result.divergences {
//!         println!("Divergence: {}", div.summary);
//!     }
//! }
//! ```

pub mod executor;
pub mod comparator;
pub mod divergence_report;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use executor::{TierExecutor, TierSpec, ExecutionResult, ExecutorConfig};
pub use comparator::{Comparator, ComparatorConfig, ComparisonResult, BehaviorDiff};
pub use divergence_report::{DivergenceReport, ReportBuilder, ReportFormat, ReportSection};

/// Configuration for the Tier Oracle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleConfig {
    /// Path to the interpreter binary (Tier 0)
    pub interpreter_path: PathBuf,
    /// Path to the bytecode VM binary (Tier 1)
    pub bytecode_path: Option<PathBuf>,
    /// Path to the JIT binary (Tier 2)
    pub jit_path: Option<PathBuf>,
    /// Path to the AOT binary (Tier 3)
    pub aot_path: PathBuf,
    /// Timeout in milliseconds for each execution
    pub timeout_ms: u64,
    /// Whether to normalize outputs before comparison
    pub normalize_output: bool,
    /// Whether to allow float epsilon differences
    pub float_epsilon: f64,
    /// Whether to fail fast on first divergence
    pub fail_fast: bool,
    /// Number of parallel workers for batch testing
    pub parallel_workers: usize,
    /// Output directory for reports
    pub report_dir: PathBuf,
    /// Whether to generate regression tests from divergences
    pub generate_regression_tests: bool,
    /// Environment variables to set for execution
    pub env_vars: HashMap<String, String>,
    /// Additional arguments for all tiers
    pub extra_args: Vec<String>,
}

impl Default for OracleConfig {
    fn default() -> Self {
        Self {
            interpreter_path: PathBuf::from("verum-interpret"),
            bytecode_path: Some(PathBuf::from("verum-bc")),
            jit_path: Some(PathBuf::from("verum-jit")),
            aot_path: PathBuf::from("verum-run"),
            timeout_ms: 30_000,
            normalize_output: true,
            float_epsilon: 1e-10,
            fail_fast: false,
            parallel_workers: num_cpus::get(),
            report_dir: PathBuf::from("differential_reports"),
            generate_regression_tests: true,
            env_vars: HashMap::new(),
            extra_args: Vec::new(),
        }
    }
}

/// Tier identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Tier {
    /// Tier 0: Tree-walking interpreter
    Interpreter = 0,
    /// Tier 1: Bytecode VM
    Bytecode = 1,
    /// Tier 2: JIT compiler
    Jit = 2,
    /// Tier 3: AOT compiler
    Aot = 3,
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Interpreter => write!(f, "Tier 0 (Interpreter)"),
            Tier::Bytecode => write!(f, "Tier 1 (Bytecode)"),
            Tier::Jit => write!(f, "Tier 2 (JIT)"),
            Tier::Aot => write!(f, "Tier 3 (AOT)"),
        }
    }
}

impl From<u8> for Tier {
    fn from(v: u8) -> Self {
        match v {
            0 => Tier::Interpreter,
            1 => Tier::Bytecode,
            2 => Tier::Jit,
            3 => Tier::Aot,
            _ => Tier::Interpreter,
        }
    }
}

/// Result of a differential test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleResult {
    /// Path to the test file
    pub test_path: PathBuf,
    /// Test name
    pub test_name: String,
    /// Whether all tiers agree
    pub success: bool,
    /// Total duration
    pub duration: Duration,
    /// Execution results per tier
    pub tier_results: HashMap<Tier, ExecutionResult>,
    /// List of divergences found
    pub divergences: Vec<TierDivergence>,
    /// Parsed test annotations
    pub annotations: TestAnnotations,
}

impl OracleResult {
    /// Create a successful result with no divergences
    pub fn success(
        test_path: PathBuf,
        test_name: String,
        duration: Duration,
        tier_results: HashMap<Tier, ExecutionResult>,
        annotations: TestAnnotations,
    ) -> Self {
        Self {
            test_path,
            test_name,
            success: true,
            duration,
            tier_results,
            divergences: Vec::new(),
            annotations,
        }
    }

    /// Create a failed result with divergences
    pub fn failed(
        test_path: PathBuf,
        test_name: String,
        duration: Duration,
        tier_results: HashMap<Tier, ExecutionResult>,
        divergences: Vec<TierDivergence>,
        annotations: TestAnnotations,
    ) -> Self {
        Self {
            test_path,
            test_name,
            success: false,
            duration,
            tier_results,
            divergences,
            annotations,
        }
    }
}

/// A specific divergence between tiers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierDivergence {
    /// First tier in comparison
    pub tier1: Tier,
    /// Second tier in comparison
    pub tier2: Tier,
    /// Category of divergence
    pub category: DivergenceCategory,
    /// Summary description
    pub summary: String,
    /// Detailed differences
    pub details: Vec<DivergenceDetail>,
    /// Suggested fix (if any)
    pub suggested_fix: Option<String>,
}

/// Category of divergence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DivergenceCategory {
    /// Exit code differs
    ExitCode,
    /// Standard output differs
    Stdout,
    /// Standard error differs
    Stderr,
    /// One tier crashed
    Crash,
    /// Timeout on one tier
    Timeout,
    /// Float precision difference
    FloatPrecision,
    /// Collection ordering difference
    Ordering,
    /// Memory-related difference
    Memory,
    /// Async/timing related
    Async,
    /// Other
    Other,
}

impl std::fmt::Display for DivergenceCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DivergenceCategory::ExitCode => write!(f, "Exit Code"),
            DivergenceCategory::Stdout => write!(f, "Standard Output"),
            DivergenceCategory::Stderr => write!(f, "Standard Error"),
            DivergenceCategory::Crash => write!(f, "Crash"),
            DivergenceCategory::Timeout => write!(f, "Timeout"),
            DivergenceCategory::FloatPrecision => write!(f, "Float Precision"),
            DivergenceCategory::Ordering => write!(f, "Collection Ordering"),
            DivergenceCategory::Memory => write!(f, "Memory"),
            DivergenceCategory::Async => write!(f, "Async/Timing"),
            DivergenceCategory::Other => write!(f, "Other"),
        }
    }
}

/// Detailed information about a divergence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceDetail {
    /// Location in the output (line number, etc.)
    pub location: String,
    /// Expected value (from reference tier)
    pub expected: String,
    /// Actual value (from comparison tier)
    pub actual: String,
    /// Context around the difference
    pub context: Vec<String>,
}

/// Parsed test annotations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestAnnotations {
    /// Test type (differential, run, etc.)
    pub test_type: Option<String>,
    /// Tiers to test
    pub tiers: Vec<u8>,
    /// Verification level
    pub level: Option<String>,
    /// Tags
    pub tags: Vec<String>,
    /// Timeout override
    pub timeout_ms: Option<u64>,
    /// Required features
    pub requires: Vec<String>,
    /// Platform constraints
    pub platform: Option<String>,
    /// Custom annotations
    pub custom: HashMap<String, String>,
}

/// Main Tier Oracle
pub struct TierOracle {
    config: OracleConfig,
    executor: TierExecutor,
    comparator: Comparator,
}

impl TierOracle {
    /// Create a new Tier Oracle with the given configuration
    pub fn new(config: OracleConfig) -> Self {
        let executor = TierExecutor::new(ExecutorConfig {
            interpreter_path: config.interpreter_path.clone(),
            bytecode_path: config.bytecode_path.clone(),
            jit_path: config.jit_path.clone(),
            aot_path: config.aot_path.clone(),
            timeout_ms: config.timeout_ms,
            env_vars: config.env_vars.clone(),
            extra_args: config.extra_args.clone(),
        });

        let comparator = Comparator::new(ComparatorConfig {
            normalize: config.normalize_output,
            float_epsilon: config.float_epsilon,
            ..Default::default()
        });

        Self {
            config,
            executor,
            comparator,
        }
    }

    /// Test a single file
    pub fn test_file(&self, path: &Path) -> Result<OracleResult> {
        let start = Instant::now();

        // Read and parse annotations
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read test file: {}", path.display()))?;

        let annotations = self.parse_annotations(&source);

        // Determine which tiers to test
        let tiers = if annotations.tiers.is_empty() {
            vec![Tier::Interpreter, Tier::Aot]
        } else {
            annotations.tiers.iter().map(|&t| Tier::from(t)).collect()
        };

        // Execute on each tier
        let mut tier_results = HashMap::new();
        for tier in &tiers {
            let result = self.executor.execute(*tier, path)?;
            tier_results.insert(*tier, result);
        }

        // Compare results
        let divergences = self.compare_tier_results(&tier_results, &tiers)?;

        let duration = start.elapsed();
        let test_name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        if divergences.is_empty() {
            Ok(OracleResult::success(
                path.to_path_buf(),
                test_name,
                duration,
                tier_results,
                annotations,
            ))
        } else {
            Ok(OracleResult::failed(
                path.to_path_buf(),
                test_name,
                duration,
                tier_results,
                divergences,
                annotations,
            ))
        }
    }

    /// Test all files in a directory
    pub fn test_directory(&self, dir: &Path) -> Result<Vec<OracleResult>> {
        let mut results = Vec::new();

        for entry in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "vr") {
                // Check if it's a differential test
                if let Ok(source) = std::fs::read_to_string(path) {
                    if source.contains("@test: differential") {
                        match self.test_file(path) {
                            Ok(result) => results.push(result),
                            Err(e) => {
                                eprintln!("Error testing {}: {}", path.display(), e);
                                if self.config.fail_fast {
                                    return Err(e);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    /// Test files in parallel
    pub fn test_directory_parallel(&self, dir: &Path) -> Result<Vec<OracleResult>> {
        use rayon::prelude::*;

        // Collect all test files
        let mut test_files = Vec::new();
        for entry in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "vr") {
                if let Ok(source) = std::fs::read_to_string(path) {
                    if source.contains("@test: differential") {
                        test_files.push(path.to_path_buf());
                    }
                }
            }
        }

        // Create thread pool
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.config.parallel_workers)
            .build()
            .context("Failed to create thread pool")?;

        let config = self.config.clone();

        pool.install(|| {
            let results: Vec<OracleResult> = test_files
                .par_iter()
                .filter_map(|path| {
                    let oracle = TierOracle::new(config.clone());
                    match oracle.test_file(path) {
                        Ok(result) => Some(result),
                        Err(e) => {
                            eprintln!("Error testing {}: {}", path.display(), e);
                            None
                        }
                    }
                })
                .collect();

            Ok(results)
        })
    }

    /// Parse test annotations from source
    fn parse_annotations(&self, source: &str) -> TestAnnotations {
        let mut annotations = TestAnnotations::default();

        for line in source.lines() {
            let line = line.trim();

            if let Some(rest) = line.strip_prefix("// @test:") {
                annotations.test_type = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("// @tier:") {
                annotations.tiers = rest
                    .trim()
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
            } else if let Some(rest) = line.strip_prefix("// @level:") {
                annotations.level = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("// @tags:") {
                annotations.tags = rest
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
            } else if let Some(rest) = line.strip_prefix("// @timeout:") {
                annotations.timeout_ms = rest.trim().parse().ok();
            } else if let Some(rest) = line.strip_prefix("// @require:") {
                annotations.requires = rest
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
            } else if let Some(rest) = line.strip_prefix("// @platform:") {
                annotations.platform = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("// @") {
                if let Some((key, value)) = rest.split_once(':') {
                    annotations.custom.insert(
                        key.trim().to_string(),
                        value.trim().to_string(),
                    );
                }
            }
        }

        annotations
    }

    /// Compare results from multiple tiers
    fn compare_tier_results(
        &self,
        results: &HashMap<Tier, ExecutionResult>,
        tiers: &[Tier],
    ) -> Result<Vec<TierDivergence>> {
        let mut divergences = Vec::new();

        if tiers.len() < 2 {
            return Ok(divergences);
        }

        // Use the first tier as reference
        let reference_tier = tiers[0];
        let reference = results.get(&reference_tier)
            .ok_or_else(|| anyhow::anyhow!("Missing reference tier result"))?;

        for &tier in &tiers[1..] {
            let result = results.get(&tier)
                .ok_or_else(|| anyhow::anyhow!("Missing tier {} result", tier))?;

            // Compare using the comparator
            let comparison = self.comparator.compare(reference, result)?;

            if !comparison.equivalent {
                for diff in comparison.differences {
                    divergences.push(TierDivergence {
                        tier1: reference_tier,
                        tier2: tier,
                        category: self.categorize_diff(&diff),
                        summary: diff.summary.clone(),
                        details: diff.details.iter().map(|d| DivergenceDetail {
                            location: d.location.clone(),
                            expected: d.expected.clone(),
                            actual: d.actual.clone(),
                            context: d.context.clone(),
                        }).collect(),
                        suggested_fix: diff.suggested_fix.clone(),
                    });
                }
            }
        }

        Ok(divergences)
    }

    /// Categorize a behavior difference
    fn categorize_diff(&self, diff: &BehaviorDiff) -> DivergenceCategory {
        match diff.kind.as_str() {
            "exit_code" => DivergenceCategory::ExitCode,
            "stdout" => DivergenceCategory::Stdout,
            "stderr" => DivergenceCategory::Stderr,
            "crash" => DivergenceCategory::Crash,
            "timeout" => DivergenceCategory::Timeout,
            "float" => DivergenceCategory::FloatPrecision,
            "ordering" => DivergenceCategory::Ordering,
            "memory" => DivergenceCategory::Memory,
            "async" => DivergenceCategory::Async,
            _ => DivergenceCategory::Other,
        }
    }

    /// Generate a summary report
    pub fn generate_summary(&self, results: &[OracleResult]) -> OracleSummary {
        let total = results.len();
        let passed = results.iter().filter(|r| r.success).count();
        let failed = total - passed;

        let total_duration: Duration = results.iter().map(|r| r.duration).sum();

        let mut divergence_counts: HashMap<DivergenceCategory, usize> = HashMap::new();
        for result in results {
            for div in &result.divergences {
                *divergence_counts.entry(div.category).or_insert(0) += 1;
            }
        }

        let failed_tests: Vec<PathBuf> = results
            .iter()
            .filter(|r| !r.success)
            .map(|r| r.test_path.clone())
            .collect();

        OracleSummary {
            total,
            passed,
            failed,
            duration: total_duration,
            divergence_counts,
            failed_tests,
        }
    }
}

/// Summary of oracle test run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleSummary {
    /// Total tests run
    pub total: usize,
    /// Tests that passed
    pub passed: usize,
    /// Tests that failed
    pub failed: usize,
    /// Total duration
    pub duration: Duration,
    /// Divergence counts by category
    pub divergence_counts: HashMap<DivergenceCategory, usize>,
    /// List of failed test paths
    pub failed_tests: Vec<PathBuf>,
}

impl OracleSummary {
    /// Print a human-readable summary
    pub fn print(&self) {
        println!("\n=== Tier Oracle Summary ===");
        println!("Total:   {}", self.total);
        println!(
            "Passed:  {} ({:.1}%)",
            self.passed,
            100.0 * self.passed as f64 / self.total.max(1) as f64
        );
        println!(
            "Failed:  {} ({:.1}%)",
            self.failed,
            100.0 * self.failed as f64 / self.total.max(1) as f64
        );
        println!("Duration: {:?}", self.duration);

        if !self.divergence_counts.is_empty() {
            println!("\nDivergences by Category:");
            for (category, count) in &self.divergence_counts {
                println!("  {}: {}", category, count);
            }
        }

        if !self.failed_tests.is_empty() {
            println!("\nFailed Tests:");
            for path in &self.failed_tests {
                println!("  - {}", path.display());
            }
        }
    }

    /// Get exit code (0 if all pass, 1 otherwise)
    pub fn exit_code(&self) -> i32 {
        if self.failed == 0 { 0 } else { 1 }
    }
}

/// Check if a test file is marked for differential testing
pub fn is_differential_test(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim();
        line == "// @test: differential" || line.starts_with("// @test: differential ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_from_u8() {
        assert_eq!(Tier::from(0), Tier::Interpreter);
        assert_eq!(Tier::from(1), Tier::Bytecode);
        assert_eq!(Tier::from(2), Tier::Jit);
        assert_eq!(Tier::from(3), Tier::Aot);
        assert_eq!(Tier::from(99), Tier::Interpreter); // Invalid defaults to Interpreter
    }

    #[test]
    fn test_tier_display() {
        assert_eq!(format!("{}", Tier::Interpreter), "Tier 0 (Interpreter)");
        assert_eq!(format!("{}", Tier::Aot), "Tier 3 (AOT)");
    }

    #[test]
    fn test_is_differential_test() {
        assert!(is_differential_test("// @test: differential\nfn main() {}"));
        assert!(is_differential_test("  // @test: differential foo\nfn main() {}"));
        assert!(!is_differential_test("// @test: run\nfn main() {}"));
        assert!(!is_differential_test("fn main() {}"));
    }

    #[test]
    fn test_divergence_category_display() {
        assert_eq!(format!("{}", DivergenceCategory::ExitCode), "Exit Code");
        assert_eq!(format!("{}", DivergenceCategory::FloatPrecision), "Float Precision");
    }

    #[test]
    fn test_oracle_config_default() {
        let config = OracleConfig::default();
        assert_eq!(config.timeout_ms, 30_000);
        assert!(config.normalize_output);
        assert!(!config.fail_fast);
    }
}
