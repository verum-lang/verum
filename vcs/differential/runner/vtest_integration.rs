//! VTest integration for differential testing
//!
//! This module provides integration between the differential testing
//! infrastructure and the vtest test runner, enabling seamless execution
//! of differential tests alongside other VCS tests.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::differential::{DiffResult, DifferentialRunner, TierOutput};
use crate::divergence::{
    Divergence, DivergenceReporter, ReportFormat, Tier, TierExecution, create_divergence,
};
use crate::normalizer::{NormalizationConfig, Normalizer};
use crate::semantic_equiv::{EquivalenceConfig, EquivalenceResult, SemanticEquivalenceChecker};

/// Configuration for vtest differential mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferentialTestConfig {
    /// Tiers to compare
    pub tiers: Vec<u8>,
    /// Reference tier (results compared against this)
    pub reference_tier: u8,
    /// Timeout for each tier in milliseconds
    pub timeout_ms: u64,
    /// Normalization configuration
    pub normalization: NormalizationConfig,
    /// Equivalence configuration
    pub equivalence: EquivalenceConfig,
    /// Whether to fail on first divergence
    pub fail_fast: bool,
    /// Report format
    pub report_format: ReportFormat,
    /// Report output directory
    pub report_dir: PathBuf,
    /// Whether to generate regression tests from failures
    pub generate_regression_tests: bool,
    /// Regression test output directory
    pub regression_test_dir: PathBuf,
    /// Path to interpreter binary
    pub interpreter_path: PathBuf,
    /// Path to bytecode VM binary
    pub bytecode_path: Option<PathBuf>,
    /// Path to JIT binary
    pub jit_path: Option<PathBuf>,
    /// Path to AOT binary
    pub aot_path: PathBuf,
    /// Whether to enable semantic comparison (looser matching)
    pub semantic_comparison: bool,
    /// Float epsilon for comparison
    pub float_epsilon: f64,
}

impl Default for DifferentialTestConfig {
    fn default() -> Self {
        Self {
            tiers: vec![0, 3],
            reference_tier: 0,
            timeout_ms: 30_000,
            normalization: NormalizationConfig::semantic(),
            equivalence: EquivalenceConfig::default(),
            fail_fast: false,
            report_format: ReportFormat::Markdown,
            report_dir: PathBuf::from("differential_reports"),
            generate_regression_tests: true,
            regression_test_dir: PathBuf::from("generated_tests/regression"),
            interpreter_path: PathBuf::from("verum-interpret"),
            bytecode_path: Some(PathBuf::from("verum-bc")),
            jit_path: Some(PathBuf::from("verum-jit")),
            aot_path: PathBuf::from("verum-run"),
            semantic_comparison: true,
            float_epsilon: 1e-10,
        }
    }
}

/// Result of a differential test run for vtest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferentialTestResult {
    /// Path to the test file
    pub path: PathBuf,
    /// Test name
    pub name: String,
    /// Whether all tiers agree
    pub passed: bool,
    /// Alias for passed - whether the test succeeded
    pub success: bool,
    /// Total duration
    pub duration: Duration,
    /// Results per tier
    pub tier_results: Vec<TierTestResult>,
    /// Divergences found (if any)
    pub divergences: Vec<DivergenceInfo>,
    /// Tags from the test file
    pub tags: Vec<String>,
    /// Level from the test file
    pub level: String,
    /// Detailed description of any divergences
    pub description: String,
}

/// Result from a single tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierTestResult {
    pub tier: u8,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub success: bool,
}

/// Summarized divergence info for vtest reporting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceInfo {
    pub tier1: u8,
    pub tier2: u8,
    pub category: String,
    pub summary: String,
    pub location: Option<String>,
}

/// Differential test executor for vtest integration
pub struct DifferentialExecutor {
    config: DifferentialTestConfig,
    runner: DifferentialRunner,
    normalizer: Normalizer,
    checker: SemanticEquivalenceChecker,
    reporter: DivergenceReporter,
}

impl DifferentialExecutor {
    /// Create a new executor with the given configuration
    pub fn new(config: DifferentialTestConfig) -> Self {
        let runner = DifferentialRunner::new().with_timeout(config.timeout_ms);

        let normalizer = Normalizer::new(config.normalization.clone());
        let checker = SemanticEquivalenceChecker::new(config.equivalence.clone());
        let reporter =
            DivergenceReporter::new(config.report_dir.clone()).with_format(config.report_format);

        Self {
            config,
            runner,
            normalizer,
            checker,
            reporter,
        }
    }

    /// Execute a differential test
    pub fn execute(&self, test_path: &Path) -> Result<DifferentialTestResult> {
        let start = Instant::now();

        // Read test file and parse metadata
        let source = std::fs::read_to_string(test_path)
            .with_context(|| format!("Failed to read test file: {}", test_path.display()))?;

        let metadata = self.parse_metadata(&source);

        // Determine which tiers to test
        let tiers = if metadata.tiers.is_empty() {
            &self.config.tiers
        } else {
            &metadata.tiers
        };

        // Execute on each tier
        let mut tier_results = Vec::new();
        for &tier in tiers {
            let result = self.execute_tier(test_path, tier)?;
            tier_results.push(result);
        }

        // Find reference result
        let ref_tier = if tiers.contains(&self.config.reference_tier) {
            self.config.reference_tier
        } else {
            tiers[0]
        };

        let ref_result = tier_results
            .iter()
            .find(|r| r.tier == ref_tier)
            .ok_or_else(|| anyhow::anyhow!("Reference tier result not found"))?;

        // Compare with other tiers
        let mut divergences = Vec::new();
        let mut all_equivalent = true;

        for result in &tier_results {
            if result.tier == ref_tier {
                continue;
            }

            // Normalize outputs
            let ref_stdout = self.normalizer.normalize(&ref_result.stdout);
            let result_stdout = self.normalizer.normalize(&result.stdout);

            // Check equivalence
            match self.checker.check(&ref_stdout, &result_stdout) {
                EquivalenceResult::Equivalent => {}
                EquivalenceResult::Different(diffs) => {
                    all_equivalent = false;

                    for diff in diffs {
                        divergences.push(DivergenceInfo {
                            tier1: ref_tier,
                            tier2: result.tier,
                            category: format!("{:?}", diff.kind),
                            summary: format!(
                                "Expected '{}', got '{}'",
                                truncate(&diff.expected, 50),
                                truncate(&diff.actual, 50)
                            ),
                            location: Some(format!("{}", diff.location)),
                        });
                    }
                }
            }

            // Also check exit codes
            if ref_result.exit_code != result.exit_code {
                all_equivalent = false;
                divergences.push(DivergenceInfo {
                    tier1: ref_tier,
                    tier2: result.tier,
                    category: "ExitCode".to_string(),
                    summary: format!(
                        "Exit code mismatch: {:?} vs {:?}",
                        ref_result.exit_code, result.exit_code
                    ),
                    location: None,
                });
            }
        }

        let duration = start.elapsed();

        let description = if divergences.is_empty() {
            "All tiers agree".to_string()
        } else {
            divergences
                .iter()
                .map(|d| format!("Tier {} vs {}: {}", d.tier1, d.tier2, d.summary))
                .collect::<Vec<_>>()
                .join("; ")
        };

        Ok(DifferentialTestResult {
            path: test_path.to_path_buf(),
            name: test_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            passed: all_equivalent,
            success: all_equivalent,
            duration,
            tier_results,
            divergences,
            tags: metadata.tags,
            level: metadata.level.unwrap_or_else(|| "L1".to_string()),
            description,
        })
    }

    /// Execute on a single tier
    fn execute_tier(&self, test_path: &Path, tier: u8) -> Result<TierTestResult> {
        let output = self
            .runner
            .run_tier(tier, test_path, self.config.timeout_ms)?;

        Ok(TierTestResult {
            tier,
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.exit_code,
            duration_ms: output.duration_ms,
            success: output.success,
        })
    }

    /// Parse test metadata from source
    fn parse_metadata(&self, source: &str) -> TestMetadata {
        let mut metadata = TestMetadata::default();

        for line in source.lines() {
            let line = line.trim();

            if line.starts_with("// @tier:") {
                let tiers_str = line.trim_start_matches("// @tier:").trim();
                metadata.tiers = tiers_str
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
            }

            if line.starts_with("// @level:") {
                metadata.level = Some(line.trim_start_matches("// @level:").trim().to_string());
            }

            if line.starts_with("// @tags:") {
                metadata.tags = line
                    .trim_start_matches("// @tags:")
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
            }

            if line.starts_with("// @timeout:") {
                metadata.timeout_ms = line.trim_start_matches("// @timeout:").trim().parse().ok();
            }
        }

        metadata
    }

    /// Run all differential tests in a directory
    /// Alias for execute - run a single test
    pub fn run(&self, test_path: &Path) -> Result<DifferentialTestResult> {
        self.execute(test_path)
    }

    /// Run all differential tests in a directory (sequential)
    pub fn run_directory_sequential(&self, dir: &Path) -> Result<DifferentialSummary> {
        let mut results = Vec::new();

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map_or(false, |ext| ext == "vr") {
                // Check if it's a differential test
                let source = std::fs::read_to_string(&path)?;
                if source.contains("@test: differential") {
                    match self.execute(&path) {
                        Ok(result) => results.push(result),
                        Err(e) => {
                            eprintln!("Error running {}: {}", path.display(), e);
                            if self.config.fail_fast {
                                return Err(e);
                            }
                        }
                    }
                }
            }
        }

        Ok(DifferentialSummary::from_results(results))
    }

    /// Run all differential tests in a directory with optional parallel execution
    pub fn run_directory(
        &self,
        dir: &Path,
        parallel: bool,
        workers: usize,
    ) -> Result<Vec<DifferentialTestResult>> {
        // Collect test files
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

        if parallel && workers > 1 {
            self.run_parallel(&test_files, workers)
        } else {
            self.run_sequential(&test_files)
        }
    }

    /// Run tests sequentially
    fn run_sequential(&self, test_files: &[PathBuf]) -> Result<Vec<DifferentialTestResult>> {
        let mut results = Vec::new();

        for path in test_files {
            match self.execute(path) {
                Ok(result) => results.push(result),
                Err(e) => {
                    eprintln!("Error running {}: {}", path.display(), e);
                    if self.config.fail_fast {
                        return Err(e);
                    }
                }
            }
        }

        Ok(results)
    }

    /// Run tests in parallel using rayon
    fn run_parallel(
        &self,
        test_files: &[PathBuf],
        workers: usize,
    ) -> Result<Vec<DifferentialTestResult>> {
        use rayon::prelude::*;

        // Configure thread pool
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .context("Failed to create thread pool")?;

        let config = self.config.clone();

        pool.install(|| {
            let results: Vec<_> = test_files
                .par_iter()
                .filter_map(|path| {
                    // Create executor for each thread
                    let executor = DifferentialExecutor::new(config.clone());
                    match executor.execute(path) {
                        Ok(result) => Some(result),
                        Err(e) => {
                            eprintln!("Error running {}: {}", path.display(), e);
                            None
                        }
                    }
                })
                .collect();

            Ok(results)
        })
    }
}

/// Test metadata parsed from source
#[derive(Debug, Clone, Default)]
struct TestMetadata {
    tiers: Vec<u8>,
    level: Option<String>,
    tags: Vec<String>,
    timeout_ms: Option<u64>,
}

/// Summary of differential test run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferentialSummary {
    /// Total tests run
    pub total: usize,
    /// Tests that passed (all tiers agree)
    pub passed: usize,
    /// Tests that failed (divergences found)
    pub failed: usize,
    /// Tests that errored
    pub errored: usize,
    /// Total duration
    pub duration: Duration,
    /// Results by tier
    pub by_tier: Vec<TierSummary>,
    /// Failed test paths
    pub failed_tests: Vec<PathBuf>,
    /// Divergence count by category
    pub divergences_by_category: Vec<(String, usize)>,
}

/// Summary for a single tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierSummary {
    pub tier: u8,
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub avg_duration_ms: u64,
}

impl DifferentialSummary {
    /// Create summary from test results
    pub fn from_results(results: Vec<DifferentialTestResult>) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = results.iter().filter(|r| !r.passed).count();

        let duration = results.iter().map(|r| r.duration).sum();

        let failed_tests: Vec<PathBuf> = results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| r.path.clone())
            .collect();

        // Aggregate divergences by category
        let mut category_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for result in &results {
            for div in &result.divergences {
                *category_counts.entry(div.category.clone()).or_default() += 1;
            }
        }
        let mut divergences_by_category: Vec<(String, usize)> =
            category_counts.into_iter().collect();
        divergences_by_category.sort_by(|a, b| b.1.cmp(&a.1));

        // Build tier summaries
        let mut tier_data: std::collections::HashMap<u8, (usize, usize, usize, u64)> =
            std::collections::HashMap::new();
        for result in &results {
            for tier_result in &result.tier_results {
                let entry = tier_data.entry(tier_result.tier).or_default();
                entry.0 += 1; // total
                if tier_result.success {
                    entry.1 += 1; // succeeded
                } else {
                    entry.2 += 1; // failed
                }
                entry.3 += tier_result.duration_ms; // total duration
            }
        }

        let by_tier: Vec<TierSummary> = tier_data
            .into_iter()
            .map(
                |(tier, (total, succeeded, failed, total_duration))| TierSummary {
                    tier,
                    total,
                    succeeded,
                    failed,
                    avg_duration_ms: if total > 0 {
                        total_duration / total as u64
                    } else {
                        0
                    },
                },
            )
            .collect();

        Self {
            total,
            passed,
            failed,
            errored: 0,
            duration,
            by_tier,
            failed_tests,
            divergences_by_category,
        }
    }

    /// Print summary to stdout
    pub fn print(&self) {
        println!("\n=== Differential Test Summary ===");
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
        println!();

        println!("By Tier:");
        for tier in &self.by_tier {
            println!(
                "  Tier {}: {}/{} passed, avg {}ms",
                tier.tier, tier.succeeded, tier.total, tier.avg_duration_ms
            );
        }

        if !self.divergences_by_category.is_empty() {
            println!("\nDivergences by Category:");
            for (category, count) in &self.divergences_by_category {
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

    /// Generate exit code (0 if all pass, 1 otherwise)
    pub fn exit_code(&self) -> i32 {
        if self.failed == 0 && self.errored == 0 {
            0
        } else {
            1
        }
    }
}

/// VTest directive handler for differential tests
pub fn handle_differential_directive(
    source: &str,
    path: &Path,
    config: &DifferentialTestConfig,
) -> Result<DifferentialTestResult> {
    let executor = DifferentialExecutor::new(config.clone());
    executor.execute(path)
}

/// Check if a test file is a differential test
pub fn is_differential_test(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim();
        line == "// @test: differential" || line.starts_with("// @test: differential ")
    })
}

/// Helper to truncate strings
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_differential_test() {
        let source = r#"
// @test: differential
// @tier: 0, 3

fn main() {
    println("hello");
}
"#;
        assert!(is_differential_test(source));

        let non_diff = r#"
// @test: run

fn main() {
    println("hello");
}
"#;
        assert!(!is_differential_test(non_diff));
    }

    #[test]
    fn test_differential_summary() {
        let results = vec![
            DifferentialTestResult {
                path: PathBuf::from("test1.vr"),
                name: "test1".to_string(),
                passed: true,
                success: true,
                duration: Duration::from_millis(100),
                tier_results: vec![
                    TierTestResult {
                        tier: 0,
                        stdout: "hello".to_string(),
                        stderr: String::new(),
                        exit_code: Some(0),
                        duration_ms: 50,
                        success: true,
                    },
                    TierTestResult {
                        tier: 3,
                        stdout: "hello".to_string(),
                        stderr: String::new(),
                        exit_code: Some(0),
                        duration_ms: 50,
                        success: true,
                    },
                ],
                divergences: vec![],
                tags: vec!["test".to_string()],
                level: "L1".to_string(),
                description: "All tiers agree".to_string(),
            },
            DifferentialTestResult {
                path: PathBuf::from("test2.vr"),
                name: "test2".to_string(),
                passed: false,
                success: false,
                duration: Duration::from_millis(150),
                tier_results: vec![
                    TierTestResult {
                        tier: 0,
                        stdout: "hello".to_string(),
                        stderr: String::new(),
                        exit_code: Some(0),
                        duration_ms: 75,
                        success: true,
                    },
                    TierTestResult {
                        tier: 3,
                        stdout: "world".to_string(),
                        stderr: String::new(),
                        exit_code: Some(0),
                        duration_ms: 75,
                        success: true,
                    },
                ],
                divergences: vec![DivergenceInfo {
                    tier1: 0,
                    tier2: 3,
                    category: "ValueMismatch".to_string(),
                    summary: "Output differs".to_string(),
                    location: Some("line 1".to_string()),
                }],
                tags: vec!["test".to_string()],
                level: "L1".to_string(),
                description: "Tier 0 vs 3: Output differs".to_string(),
            },
        ];

        let summary = DifferentialSummary::from_results(results);

        assert_eq!(summary.total, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.exit_code(), 1);
    }
}
