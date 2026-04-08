//! Cross-Implementation Testing Module
//!
//! This module provides infrastructure for testing across different
//! Verum language implementations. It supports:
//!
//! - Testing reference implementation against alternatives
//! - Version compatibility testing
//! - Protocol-based communication between implementations
//! - Adapter layer for different implementation interfaces
//!
//! # Architecture
//!
//! ```text
//! +------------------------------------------------------------------+
//! |                  CROSS-IMPLEMENTATION TESTING                    |
//! +------------------------------------------------------------------+
//! |                                                                  |
//! |  +-------------------+    +-------------------+                  |
//! |  | Reference Impl    |    | Alternative Impls |                  |
//! |  | (Interpreter)     |    | (AOT, JIT, etc.)  |                  |
//! |  +-------------------+    +-------------------+                  |
//! |           |                        |                             |
//! |           v                        v                             |
//! |  +---------------------------------------------------+          |
//! |  |                    Protocol Layer                  |          |
//! |  |  - Execute program                                 |          |
//! |  |  - Query capabilities                              |          |
//! |  |  - Report results                                  |          |
//! |  +---------------------------------------------------+          |
//! |           |                        |                             |
//! |           v                        v                             |
//! |  +-------------------+    +-------------------+                  |
//! |  |     Adapter       |    |     Adapter       |                  |
//! |  | (Interpreter)     |    | (AOT/JIT/etc.)    |                  |
//! |  +-------------------+    +-------------------+                  |
//! |                                                                  |
//! +------------------------------------------------------------------+
//! ```

pub mod protocol;
pub mod adapter;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use protocol::{
    Protocol, Message, Request, Response,
    ExecuteRequest, ExecuteResponse,
    CapabilityRequest, CapabilityResponse,
    Capability,
};

pub use adapter::{
    Adapter, AdapterConfig, AdapterKind,
    ProcessAdapter, SocketAdapter, EmbeddedAdapter,
};

/// Configuration for cross-implementation testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossImplConfig {
    /// Reference implementation
    pub reference: Implementation,
    /// Alternative implementations to test
    pub alternatives: Vec<Implementation>,
    /// Timeout for each execution
    pub timeout_ms: u64,
    /// Whether to test version compatibility
    pub test_version_compat: bool,
    /// Features to require from all implementations
    pub required_features: Vec<String>,
    /// Output directory for reports
    pub report_dir: PathBuf,
}

impl Default for CrossImplConfig {
    fn default() -> Self {
        Self {
            reference: Implementation::new("interpreter", "verum-interpret"),
            alternatives: vec![
                Implementation::new("aot", "verum-run"),
            ],
            timeout_ms: 30_000,
            test_version_compat: false,
            required_features: Vec::new(),
            report_dir: PathBuf::from("cross_impl_reports"),
        }
    }
}

/// An implementation of the Verum language
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Implementation {
    /// Name of the implementation
    pub name: String,
    /// Path to the implementation binary
    pub binary_path: PathBuf,
    /// Version string
    pub version: Option<String>,
    /// Supported features
    pub features: Vec<String>,
    /// Additional arguments
    pub extra_args: Vec<String>,
    /// Environment variables
    pub env_vars: HashMap<String, String>,
    /// Whether this implementation is available
    pub available: bool,
}

impl Implementation {
    /// Create a new implementation
    pub fn new(name: impl Into<String>, binary: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            binary_path: binary.into(),
            version: None,
            features: Vec::new(),
            extra_args: Vec::new(),
            env_vars: HashMap::new(),
            available: true,
        }
    }

    /// Add an argument
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Add environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Add feature
    pub fn with_feature(mut self, feature: impl Into<String>) -> Self {
        self.features.push(feature.into());
        self
    }

    /// Check if binary exists
    pub fn check_available(&mut self) -> bool {
        self.available = self.binary_path.exists()
            || which::which(&self.binary_path).is_ok();
        self.available
    }

    /// Query version
    pub fn query_version(&mut self) -> Result<String> {
        use std::process::Command;

        let output = Command::new(&self.binary_path)
            .arg("--version")
            .output()
            .context("Failed to query version")?;

        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        self.version = Some(version.clone());
        Ok(version)
    }
}

/// Result of executing a test on an implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplExecutionResult {
    /// Implementation that was tested
    pub implementation: String,
    /// Whether execution succeeded
    pub success: bool,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Duration
    pub duration: Duration,
    /// Whether it timed out
    pub timed_out: bool,
    /// Whether it crashed
    pub crashed: bool,
}

/// Result of cross-implementation testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossImplResult {
    /// Test path
    pub test_path: PathBuf,
    /// Test name
    pub test_name: String,
    /// Whether all implementations agree
    pub all_agree: bool,
    /// Results per implementation
    pub results: HashMap<String, ImplExecutionResult>,
    /// Comparisons between implementations
    pub comparisons: Vec<ImplComparison>,
    /// Total duration
    pub duration: Duration,
}

/// Comparison between two implementations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplComparison {
    /// First implementation
    pub impl1: String,
    /// Second implementation
    pub impl2: String,
    /// Whether they agree
    pub agree: bool,
    /// Differences found
    pub differences: Vec<ImplDifference>,
}

/// A difference between implementations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplDifference {
    /// Kind of difference
    pub kind: String,
    /// Description
    pub description: String,
    /// Value from impl1
    pub value1: String,
    /// Value from impl2
    pub value2: String,
}

/// Cross-implementation test runner
pub struct CrossImplRunner {
    config: CrossImplConfig,
    adapters: HashMap<String, Box<dyn Adapter>>,
}

impl CrossImplRunner {
    /// Create a new runner
    pub fn new(config: CrossImplConfig) -> Self {
        let mut runner = Self {
            config,
            adapters: HashMap::new(),
        };

        runner.initialize_adapters();
        runner
    }

    /// Initialize adapters for all implementations
    fn initialize_adapters(&mut self) {
        // Add reference implementation adapter
        if let Ok(adapter) = ProcessAdapter::new(&self.config.reference) {
            self.adapters.insert(
                self.config.reference.name.clone(),
                Box::new(adapter),
            );
        }

        // Add alternative implementation adapters
        for alt in &self.config.alternatives {
            if let Ok(adapter) = ProcessAdapter::new(alt) {
                self.adapters.insert(alt.name.clone(), Box::new(adapter));
            }
        }
    }

    /// Run a test file on all implementations
    pub fn run(&self, test_path: &Path) -> Result<CrossImplResult> {
        let start = Instant::now();
        let mut results = HashMap::new();

        // Run on reference implementation
        let ref_result = self.run_on_impl(&self.config.reference, test_path)?;
        results.insert(self.config.reference.name.clone(), ref_result);

        // Run on alternative implementations
        for alt in &self.config.alternatives {
            let alt_result = self.run_on_impl(alt, test_path)?;
            results.insert(alt.name.clone(), alt_result);
        }

        // Compare results
        let comparisons = self.compare_all_results(&results);

        let all_agree = comparisons.iter().all(|c| c.agree);

        let test_name = test_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Ok(CrossImplResult {
            test_path: test_path.to_path_buf(),
            test_name,
            all_agree,
            results,
            comparisons,
            duration: start.elapsed(),
        })
    }

    /// Run a test on a specific implementation
    fn run_on_impl(&self, impl_: &Implementation, test_path: &Path) -> Result<ImplExecutionResult> {
        use std::process::Command;

        let start = Instant::now();
        let timeout = Duration::from_millis(self.config.timeout_ms);

        let mut cmd = Command::new(&impl_.binary_path);
        cmd.arg(test_path);
        cmd.args(&impl_.extra_args);
        cmd.envs(&impl_.env_vars);

        let output = cmd.output()
            .with_context(|| format!("Failed to run {}", impl_.name))?;

        let duration = start.elapsed();

        Ok(ImplExecutionResult {
            implementation: impl_.name.clone(),
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration,
            timed_out: duration >= timeout,
            crashed: output.status.code().is_none(),
        })
    }

    /// Compare all results pairwise
    fn compare_all_results(
        &self,
        results: &HashMap<String, ImplExecutionResult>,
    ) -> Vec<ImplComparison> {
        let mut comparisons = Vec::new();
        let impl_names: Vec<_> = results.keys().collect();

        for i in 0..impl_names.len() {
            for j in (i + 1)..impl_names.len() {
                let name1 = impl_names[i];
                let name2 = impl_names[j];

                if let (Some(r1), Some(r2)) = (results.get(name1), results.get(name2)) {
                    let comparison = self.compare_results(name1, r1, name2, r2);
                    comparisons.push(comparison);
                }
            }
        }

        comparisons
    }

    /// Compare two implementation results
    fn compare_results(
        &self,
        name1: &str,
        r1: &ImplExecutionResult,
        name2: &str,
        r2: &ImplExecutionResult,
    ) -> ImplComparison {
        let mut differences = Vec::new();

        // Compare exit codes
        if r1.exit_code != r2.exit_code {
            differences.push(ImplDifference {
                kind: "exit_code".to_string(),
                description: "Exit codes differ".to_string(),
                value1: format!("{:?}", r1.exit_code),
                value2: format!("{:?}", r2.exit_code),
            });
        }

        // Compare stdout
        let stdout1 = normalize_output(&r1.stdout);
        let stdout2 = normalize_output(&r2.stdout);

        if stdout1 != stdout2 {
            differences.push(ImplDifference {
                kind: "stdout".to_string(),
                description: "Standard output differs".to_string(),
                value1: truncate(&stdout1, 200),
                value2: truncate(&stdout2, 200),
            });
        }

        // Compare crash/timeout status
        if r1.crashed != r2.crashed {
            differences.push(ImplDifference {
                kind: "crash".to_string(),
                description: "Crash status differs".to_string(),
                value1: r1.crashed.to_string(),
                value2: r2.crashed.to_string(),
            });
        }

        if r1.timed_out != r2.timed_out {
            differences.push(ImplDifference {
                kind: "timeout".to_string(),
                description: "Timeout status differs".to_string(),
                value1: r1.timed_out.to_string(),
                value2: r2.timed_out.to_string(),
            });
        }

        ImplComparison {
            impl1: name1.to_string(),
            impl2: name2.to_string(),
            agree: differences.is_empty(),
            differences,
        }
    }

    /// Run all tests in a directory
    pub fn run_directory(&self, dir: &Path) -> Result<Vec<CrossImplResult>> {
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
                        eprintln!("Error testing {}: {}", path.display(), e);
                    }
                }
            }
        }

        Ok(results)
    }

    /// Generate a summary report
    pub fn generate_summary(&self, results: &[CrossImplResult]) -> CrossImplSummary {
        let total = results.len();
        let passed = results.iter().filter(|r| r.all_agree).count();
        let failed = total - passed;

        let total_duration: Duration = results.iter().map(|r| r.duration).sum();

        let mut impl_stats: HashMap<String, ImplStats> = HashMap::new();

        for result in results {
            for (impl_name, impl_result) in &result.results {
                let stats = impl_stats.entry(impl_name.clone()).or_insert(ImplStats::default());
                stats.total += 1;
                if impl_result.success {
                    stats.succeeded += 1;
                } else {
                    stats.failed += 1;
                }
                stats.total_duration += impl_result.duration;
            }
        }

        let failed_tests: Vec<PathBuf> = results
            .iter()
            .filter(|r| !r.all_agree)
            .map(|r| r.test_path.clone())
            .collect();

        CrossImplSummary {
            total,
            passed,
            failed,
            duration: total_duration,
            impl_stats,
            failed_tests,
        }
    }
}

/// Summary of cross-implementation testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossImplSummary {
    /// Total tests
    pub total: usize,
    /// Passed tests (all impls agree)
    pub passed: usize,
    /// Failed tests (impls disagree)
    pub failed: usize,
    /// Total duration
    pub duration: Duration,
    /// Stats per implementation
    pub impl_stats: HashMap<String, ImplStats>,
    /// Failed test paths
    pub failed_tests: Vec<PathBuf>,
}

/// Statistics for an implementation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImplStats {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub total_duration: Duration,
}

impl CrossImplSummary {
    /// Print summary to stdout
    pub fn print(&self) {
        println!("\n=== Cross-Implementation Test Summary ===");
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

        println!("By Implementation:");
        for (name, stats) in &self.impl_stats {
            println!(
                "  {}: {}/{} succeeded, avg {:?}",
                name,
                stats.succeeded,
                stats.total,
                stats.total_duration / stats.total.max(1) as u32
            );
        }

        if !self.failed_tests.is_empty() {
            println!("\nFailed Tests:");
            for path in &self.failed_tests {
                println!("  - {}", path.display());
            }
        }
    }

    /// Get exit code (0 if all pass)
    pub fn exit_code(&self) -> i32 {
        if self.failed == 0 { 0 } else { 1 }
    }
}

/// Normalize output for comparison
fn normalize_output(s: &str) -> String {
    s.replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncate a string
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Standard implementations helper
pub fn standard_implementations() -> Vec<Implementation> {
    vec![
        Implementation::new("interpreter", "verum-interpret"),
        Implementation::new("bytecode", "verum-bc"),
        Implementation::new("jit", "verum-jit"),
        Implementation::new("aot", "verum-run"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_implementation_builder() {
        let impl_ = Implementation::new("test", "/usr/bin/test")
            .with_arg("--verbose")
            .with_env("DEBUG", "1")
            .with_feature("async");

        assert_eq!(impl_.name, "test");
        assert!(impl_.extra_args.contains(&"--verbose".to_string()));
        assert_eq!(impl_.env_vars.get("DEBUG"), Some(&"1".to_string()));
        assert!(impl_.features.contains(&"async".to_string()));
    }

    #[test]
    fn test_cross_impl_config_default() {
        let config = CrossImplConfig::default();
        assert_eq!(config.reference.name, "interpreter");
        assert!(!config.alternatives.is_empty());
        assert_eq!(config.timeout_ms, 30_000);
    }

    #[test]
    fn test_normalize_output() {
        assert_eq!(normalize_output("hello\r\nworld"), "hello\nworld");
        assert_eq!(normalize_output("line1  \nline2\t\n"), "line1\nline2");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello...");
    }
}
