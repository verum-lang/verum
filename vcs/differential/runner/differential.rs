//! Differential testing runner for VCS
//!
//! This module provides infrastructure for comparing outputs between
//! different execution tiers (Tier 0 interpreter vs Tier 3 AOT).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Result of differential testing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffResult {
    /// Outputs match perfectly
    Match,

    /// Stdout differs between tiers
    StdoutMismatch {
        tier0: String,
        tier3: String,
        diff: String,
    },

    /// Stderr differs between tiers
    StderrMismatch { tier0: String, tier3: String },

    /// Exit codes differ
    ExitCodeMismatch {
        tier0: Option<i32>,
        tier3: Option<i32>,
    },

    /// One tier timed out
    Timeout { tier: String, timeout_ms: u64 },

    /// One tier crashed/panicked
    Crash {
        tier: String,
        signal: Option<i32>,
        message: String,
    },

    /// Execution failed
    ExecutionError { tier: String, error: String },
}

impl DiffResult {
    pub fn is_success(&self) -> bool {
        matches!(self, DiffResult::Match)
    }
}

/// Test metadata parsed from source file
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestMetadata {
    pub tiers: Vec<u8>,
    pub level: Option<String>,
    pub tags: Vec<String>,
    pub timeout_ms: Option<u64>,
    pub expected_output: Option<String>,
}

impl TestMetadata {
    /// Parse metadata from file comments
    pub fn parse(source: &str) -> Self {
        let mut meta = TestMetadata::default();

        for line in source.lines() {
            let line = line.trim();

            // Parse @tier annotation
            if line.starts_with("// @tier:") {
                let tiers_str = line.trim_start_matches("// @tier:").trim();
                meta.tiers = tiers_str
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
            }

            // Parse @level annotation
            if line.starts_with("// @level:") {
                meta.level = Some(line.trim_start_matches("// @level:").trim().to_string());
            }

            // Parse @tags annotation
            if line.starts_with("// @tags:") {
                let tags_str = line.trim_start_matches("// @tags:").trim();
                meta.tags = tags_str.split(',').map(|s| s.trim().to_string()).collect();
            }

            // Parse @timeout annotation
            if line.starts_with("// @timeout:") {
                let timeout_str = line.trim_start_matches("// @timeout:").trim();
                meta.timeout_ms = timeout_str.parse().ok();
            }
        }

        // Default tiers if not specified
        if meta.tiers.is_empty() {
            meta.tiers = vec![0, 3];
        }

        meta
    }
}

/// Execution output from a single tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierOutput {
    pub tier: u8,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub success: bool,
}

/// Differential testing runner
pub struct DifferentialRunner {
    /// Path to interpreter binary (Tier 0)
    tier0_binary: PathBuf,

    /// Path to AOT compiler/runner (Tier 3)
    tier3_binary: PathBuf,

    /// Working directory for test execution
    work_dir: PathBuf,

    /// Default timeout in milliseconds
    default_timeout_ms: u64,

    /// Environment variables for execution
    env_vars: HashMap<String, String>,

    /// Whether to capture stderr
    capture_stderr: bool,
}

impl DifferentialRunner {
    /// Create a new runner with default settings
    pub fn new() -> Self {
        Self {
            tier0_binary: PathBuf::from("verum-interpret"),
            tier3_binary: PathBuf::from("verum-run"),
            work_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            default_timeout_ms: 30_000,
            env_vars: HashMap::new(),
            capture_stderr: true,
        }
    }

    /// Set the interpreter binary path
    pub fn with_interpreter(mut self, path: impl Into<PathBuf>) -> Self {
        self.tier0_binary = path.into();
        self
    }

    /// Set the AOT binary path
    pub fn with_aot(mut self, path: impl Into<PathBuf>) -> Self {
        self.tier3_binary = path.into();
        self
    }

    /// Set working directory
    pub fn with_work_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.work_dir = path.into();
        self
    }

    /// Set default timeout
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.default_timeout_ms = timeout_ms;
        self
    }

    /// Add environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Run differential test on a source file
    pub fn run_differential(&self, source_path: &Path) -> Result<DiffResult> {
        // Read and parse source
        let source = fs::read_to_string(source_path)
            .with_context(|| format!("Failed to read source file: {:?}", source_path))?;

        let metadata = TestMetadata::parse(&source);
        let timeout_ms = metadata.timeout_ms.unwrap_or(self.default_timeout_ms);

        // Run in Tier 0 (interpreter)
        let tier0_output = self.run_tier(0, source_path, timeout_ms)?;

        // Run in Tier 3 (AOT)
        let tier3_output = self.run_tier(3, source_path, timeout_ms)?;

        // Compare outputs
        self.compare_outputs(&tier0_output, &tier3_output)
    }

    /// Run a specific tier
    pub fn run_tier(&self, tier: u8, source_path: &Path, timeout_ms: u64) -> Result<TierOutput> {
        let binary = match tier {
            0 => &self.tier0_binary,
            3 => &self.tier3_binary,
            _ => bail!("Unsupported tier: {}", tier),
        };

        let start = Instant::now();

        let mut cmd = Command::new(binary);
        cmd.arg(source_path)
            .current_dir(&self.work_dir)
            .stdout(Stdio::piped())
            .stderr(if self.capture_stderr {
                Stdio::piped()
            } else {
                Stdio::null()
            });

        // Set environment variables
        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }

        // Execute with timeout
        let output = self.execute_with_timeout(&mut cmd, Duration::from_millis(timeout_ms))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(TierOutput {
            tier,
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code(),
            duration_ms,
            success: output.status.success(),
        })
    }

    /// Execute command with timeout
    fn execute_with_timeout(&self, cmd: &mut Command, timeout: Duration) -> Result<Output> {
        use std::io::Read;

        let mut child = cmd.spawn().context("Failed to spawn process")?;

        // Wait with timeout
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
                // Timeout - kill process
                child.kill()?;
                bail!("Process timed out after {:?}", timeout);
            }
        }
    }

    /// Compare outputs from two tiers
    fn compare_outputs(&self, tier0: &TierOutput, tier3: &TierOutput) -> Result<DiffResult> {
        // Check for crashes
        if !tier0.success && tier0.exit_code.is_none() {
            return Ok(DiffResult::Crash {
                tier: "tier0".to_string(),
                signal: None,
                message: tier0.stderr.clone(),
            });
        }

        if !tier3.success && tier3.exit_code.is_none() {
            return Ok(DiffResult::Crash {
                tier: "tier3".to_string(),
                signal: None,
                message: tier3.stderr.clone(),
            });
        }

        // Compare exit codes
        if tier0.exit_code != tier3.exit_code {
            return Ok(DiffResult::ExitCodeMismatch {
                tier0: tier0.exit_code,
                tier3: tier3.exit_code,
            });
        }

        // Compare stdout
        if tier0.stdout != tier3.stdout {
            let diff = compute_diff(&tier0.stdout, &tier3.stdout);
            return Ok(DiffResult::StdoutMismatch {
                tier0: tier0.stdout.clone(),
                tier3: tier3.stdout.clone(),
                diff,
            });
        }

        // Compare stderr (optional - some differences may be acceptable)
        if tier0.stderr != tier3.stderr && self.capture_stderr {
            return Ok(DiffResult::StderrMismatch {
                tier0: tier0.stderr.clone(),
                tier3: tier3.stderr.clone(),
            });
        }

        Ok(DiffResult::Match)
    }

    /// Run all differential tests in a directory
    pub fn run_directory(&self, dir: &Path) -> Result<TestReport> {
        let mut report = TestReport::new();

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map_or(false, |ext| ext == "vr") {
                let result = self.run_differential(&path)?;
                report.add_result(path.clone(), result);
            }
        }

        Ok(report)
    }
}

impl Default for DifferentialRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// Test report aggregating multiple results
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TestReport {
    pub results: Vec<(PathBuf, DiffResult)>,
    pub passed: usize,
    pub failed: usize,
    pub total_duration_ms: u64,
}

impl TestReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_result(&mut self, path: PathBuf, result: DiffResult) {
        if result.is_success() {
            self.passed += 1;
        } else {
            self.failed += 1;
        }
        self.results.push((path, result));
    }

    pub fn print_summary(&self) {
        println!("\n=== Differential Test Report ===");
        println!("Passed: {}", self.passed);
        println!("Failed: {}", self.failed);
        println!("Total:  {}", self.passed + self.failed);
        println!();

        if self.failed > 0 {
            println!("Failed tests:");
            for (path, result) in &self.results {
                if !result.is_success() {
                    println!("  {:?}: {:?}", path, result);
                }
            }
        }
    }

    pub fn success(&self) -> bool {
        self.failed == 0
    }
}

/// Compute a simple line-by-line diff
fn compute_diff(a: &str, b: &str) -> String {
    let lines_a: Vec<&str> = a.lines().collect();
    let lines_b: Vec<&str> = b.lines().collect();

    let mut diff = String::new();
    let max_lines = lines_a.len().max(lines_b.len());

    for i in 0..max_lines {
        let line_a = lines_a.get(i).copied().unwrap_or("<missing>");
        let line_b = lines_b.get(i).copied().unwrap_or("<missing>");

        if line_a != line_b {
            diff.push_str(&format!("Line {}: \n", i + 1));
            diff.push_str(&format!("  tier0: {}\n", line_a));
            diff.push_str(&format!("  tier3: {}\n", line_b));
        }
    }

    diff
}

/// Fuzzer for generating random test inputs
pub struct DifferentialFuzzer {
    runner: DifferentialRunner,
    seed: u64,
}

impl DifferentialFuzzer {
    pub fn new(runner: DifferentialRunner, seed: u64) -> Self {
        Self { runner, seed }
    }

    /// Generate and test random programs
    pub fn fuzz(&mut self, iterations: usize) -> Result<Vec<(String, DiffResult)>> {
        let mut results = Vec::new();

        for i in 0..iterations {
            let program = self.generate_random_program(i);

            // Write to temp file
            let temp_path = std::env::temp_dir().join(format!("fuzz_{}.vr", i));
            fs::write(&temp_path, &program)?;

            // Run differential test
            let result = self.runner.run_differential(&temp_path)?;

            if !result.is_success() {
                results.push((program, result));
            }

            // Cleanup
            let _ = fs::remove_file(&temp_path);
        }

        Ok(results)
    }

    fn generate_random_program(&self, iteration: usize) -> String {
        // Simple program generator - can be expanded
        let seed = self.seed.wrapping_add(iteration as u64);

        format!(
            r#"// @test: differential
// @tier: 0, 3
// Generated by fuzzer, seed: {}

fn main() {{
    let a = {};
    let b = {};
    let c = a + b;
    let d = a * b;
    let e = if a > b {{ a }} else {{ b }};
    println(f"{{a}} {{b}} {{c}} {{d}} {{e}}");
}}
"#,
            seed,
            (seed % 1000) as i32 - 500,
            ((seed >> 16) % 1000) as i32 - 500,
        )
    }
}

/// Property-based testing support
pub struct PropertyTest<F> {
    name: String,
    generator: F,
    iterations: usize,
}

impl<F> PropertyTest<F>
where
    F: Fn(u64) -> String,
{
    pub fn new(name: impl Into<String>, generator: F) -> Self {
        Self {
            name: name.into(),
            generator,
            iterations: 100,
        }
    }

    pub fn with_iterations(mut self, n: usize) -> Self {
        self.iterations = n;
        self
    }

    pub fn run(&self, runner: &DifferentialRunner) -> Result<TestReport> {
        let mut report = TestReport::new();

        for i in 0..self.iterations {
            let program = (self.generator)(i as u64);

            let temp_path = std::env::temp_dir().join(format!("prop_{}_{}.vr", self.name, i));
            fs::write(&temp_path, &program)?;

            let result = runner.run_differential(&temp_path)?;
            report.add_result(temp_path.clone(), result);

            let _ = fs::remove_file(&temp_path);
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_parsing() {
        let source = r#"
// @test: differential
// @tier: 0, 3
// @level: L1
// @tags: differential, arithmetic
// @timeout: 5000

fn main() {
    println("hello");
}
"#;

        let meta = TestMetadata::parse(source);
        assert_eq!(meta.tiers, vec![0, 3]);
        assert_eq!(meta.level, Some("L1".to_string()));
        assert_eq!(meta.tags, vec!["differential", "arithmetic"]);
        assert_eq!(meta.timeout_ms, Some(5000));
    }

    #[test]
    fn test_diff_result_success() {
        assert!(DiffResult::Match.is_success());
        assert!(
            !DiffResult::ExitCodeMismatch {
                tier0: Some(0),
                tier3: Some(1)
            }
            .is_success()
        );
    }

    #[test]
    fn test_compute_diff() {
        let a = "line1\nline2\nline3";
        let b = "line1\nmodified\nline3";

        let diff = compute_diff(a, b);
        assert!(diff.contains("Line 2"));
        assert!(diff.contains("line2"));
        assert!(diff.contains("modified"));
    }
}
