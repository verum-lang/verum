//! Benchmark execution engine for VCS benchmarks.
//!
//! This module provides the core benchmark runner that discovers, executes,
//! and collects results from VCS benchmark files (.vr files with @test: benchmark).
//!
//! # Features
//!
//! - Advanced warmup strategies (adaptive, fixed, timed)
//! - Statistical analysis with outlier detection (IQR, MAD, Grubbs)
//! - Confidence intervals (95%, 99%)
//! - Memory profiling integration
//! - Progress reporting with ETA
//!
//! # Example
//!
//! ```ignore
//! let results = BenchmarkGroup::new("my_benchmarks")
//!     .warmup_strategy(WarmupStrategy::Adaptive { min: 10, max: 1000, target_cv: 0.05 })
//!     .outlier_detection(OutlierMethod::Iqr { k: 1.5 })
//!     .memory_profiling(true)
//!     .bench("fast_op", || { /* ... */ })
//!     .run();
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::metrics::{
    BenchmarkCategory, BenchmarkResult, Measurement, PerformanceTargets, Statistics,
};

// ============================================================================
// Warmup Strategies
// ============================================================================

/// Strategy for determining warmup iterations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WarmupStrategy {
    /// Fixed number of warmup iterations.
    Fixed(usize),

    /// Timed warmup (run until duration elapses).
    Timed(Duration),

    /// Adaptive warmup that stops when measurements stabilize.
    Adaptive {
        /// Minimum warmup iterations.
        min: usize,
        /// Maximum warmup iterations.
        max: usize,
        /// Target coefficient of variation (CV) to achieve.
        target_cv: f64,
    },

    /// Skip warmup entirely (for cold-start benchmarks).
    None,
}

impl Default for WarmupStrategy {
    fn default() -> Self {
        WarmupStrategy::Fixed(100)
    }
}

impl WarmupStrategy {
    /// Run warmup for the given benchmark function.
    pub fn run_warmup<F: Fn()>(&self, f: &F) -> usize {
        match self {
            WarmupStrategy::Fixed(n) => {
                for _ in 0..*n {
                    std::hint::black_box(f());
                }
                *n
            }
            WarmupStrategy::Timed(duration) => {
                let start = Instant::now();
                let mut count = 0;
                while start.elapsed() < *duration {
                    std::hint::black_box(f());
                    count += 1;
                }
                count
            }
            WarmupStrategy::Adaptive {
                min,
                max,
                target_cv,
            } => {
                let mut measurements = Vec::with_capacity(*max);
                let mut count = 0;

                // Run minimum iterations
                for _ in 0..*min {
                    let start = Instant::now();
                    std::hint::black_box(f());
                    measurements.push(start.elapsed().as_nanos() as f64);
                    count += 1;
                }

                // Continue until CV is below target or max reached
                while count < *max {
                    let start = Instant::now();
                    std::hint::black_box(f());
                    measurements.push(start.elapsed().as_nanos() as f64);
                    count += 1;

                    // Check CV on recent measurements
                    if count >= 20 {
                        let recent = &measurements[count - 20..];
                        let mean = recent.iter().sum::<f64>() / recent.len() as f64;
                        if mean > 0.0 {
                            let variance = recent.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                                / recent.len() as f64;
                            let cv = variance.sqrt() / mean;
                            if cv < *target_cv {
                                break;
                            }
                        }
                    }
                }

                count
            }
            WarmupStrategy::None => 0,
        }
    }
}

// ============================================================================
// Outlier Detection
// ============================================================================

/// Method for detecting and handling outliers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutlierMethod {
    /// No outlier removal.
    None,

    /// Interquartile range (IQR) method.
    /// Outliers are values outside [Q1 - k*IQR, Q3 + k*IQR].
    Iqr {
        /// Multiplier for IQR (typically 1.5 or 3.0).
        k: f64,
    },

    /// Median Absolute Deviation (MAD) method.
    /// More robust than IQR for non-normal distributions.
    Mad {
        /// Threshold multiplier (typically 3.0).
        threshold: f64,
    },

    /// Grubbs' test for single outliers.
    /// Assumes normal distribution.
    Grubbs {
        /// Significance level (typically 0.05).
        alpha: f64,
    },

    /// Percentile-based trimming.
    Percentile {
        /// Lower percentile to trim (e.g., 5.0 for 5th percentile).
        lower: f64,
        /// Upper percentile to trim (e.g., 95.0 for 95th percentile).
        upper: f64,
    },
}

impl Default for OutlierMethod {
    fn default() -> Self {
        OutlierMethod::Iqr { k: 1.5 }
    }
}

/// Result of outlier detection.
#[derive(Debug, Clone)]
pub struct OutlierResult {
    /// Original measurements.
    pub original: Vec<f64>,
    /// Measurements with outliers removed.
    pub filtered: Vec<f64>,
    /// Indices of detected outliers.
    pub outlier_indices: Vec<usize>,
    /// Lower bound for non-outliers.
    pub lower_bound: f64,
    /// Upper bound for non-outliers.
    pub upper_bound: f64,
}

impl OutlierMethod {
    /// Detect and remove outliers from measurements.
    pub fn filter(&self, measurements: &[f64]) -> OutlierResult {
        if measurements.is_empty() {
            return OutlierResult {
                original: vec![],
                filtered: vec![],
                outlier_indices: vec![],
                lower_bound: 0.0,
                upper_bound: 0.0,
            };
        }

        let (lower, upper) = match self {
            OutlierMethod::None => {
                let min = measurements.iter().cloned().fold(f64::INFINITY, f64::min);
                let max = measurements
                    .iter()
                    .cloned()
                    .fold(f64::NEG_INFINITY, f64::max);
                (min, max)
            }
            OutlierMethod::Iqr { k } => {
                let mut sorted = measurements.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

                let q1 = percentile(&sorted, 25.0);
                let q3 = percentile(&sorted, 75.0);
                let iqr = q3 - q1;

                (q1 - k * iqr, q3 + k * iqr)
            }
            OutlierMethod::Mad { threshold } => {
                let med = median(measurements);
                let deviations: Vec<f64> = measurements.iter().map(|x| (x - med).abs()).collect();
                let mad = median(&deviations) * 1.4826; // Scale factor for normal distribution

                (med - threshold * mad, med + threshold * mad)
            }
            OutlierMethod::Grubbs { alpha } => {
                let n = measurements.len() as f64;
                let mean = measurements.iter().sum::<f64>() / n;
                let std_dev = (measurements.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
                    / (n - 1.0))
                    .sqrt();

                // Critical value approximation for Grubbs' test
                let t_critical = t_critical_value(n as usize - 2, *alpha / (2.0 * n));
                let g_critical = ((n - 1.0) / n.sqrt())
                    * (t_critical.powi(2) / (n - 2.0 + t_critical.powi(2))).sqrt();

                (mean - g_critical * std_dev, mean + g_critical * std_dev)
            }
            OutlierMethod::Percentile { lower, upper } => {
                let mut sorted = measurements.to_vec();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

                (percentile(&sorted, *lower), percentile(&sorted, *upper))
            }
        };

        let mut outlier_indices = Vec::new();
        let mut filtered = Vec::with_capacity(measurements.len());

        for (i, &value) in measurements.iter().enumerate() {
            if value >= lower && value <= upper {
                filtered.push(value);
            } else {
                outlier_indices.push(i);
            }
        }

        OutlierResult {
            original: measurements.to_vec(),
            filtered,
            outlier_indices,
            lower_bound: lower,
            upper_bound: upper,
        }
    }
}

/// Calculate the median of a slice.
fn median(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut sorted = data.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Calculate a percentile of a sorted slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Approximate t critical value for Grubbs' test.
fn t_critical_value(df: usize, _alpha: f64) -> f64 {
    // Simple approximation for common cases
    let df = df as f64;
    let z = -2.326; // Z-score for common alpha values
    z * (1.0 + 1.0 / (4.0 * df) + 1.0 / (32.0 * df.powi(2))).sqrt()
}

// ============================================================================
// Memory Profiling
// ============================================================================

/// Memory usage snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    /// Heap allocation in bytes.
    pub heap_bytes: usize,
    /// Peak heap allocation in bytes.
    pub peak_heap_bytes: usize,
    /// Number of allocations.
    pub allocation_count: usize,
    /// Number of deallocations.
    pub deallocation_count: usize,
    /// Resident set size (RSS) in bytes.
    pub rss_bytes: Option<usize>,
}

impl MemorySnapshot {
    /// Take a memory snapshot.
    #[cfg(target_os = "macos")]
    pub fn capture() -> Self {
        use std::process::Command;

        // Get RSS from ps command on macOS
        let rss = Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .and_then(|s| s.trim().parse::<usize>().ok())
            .map(|kb| kb * 1024); // Convert KB to bytes

        Self {
            heap_bytes: 0,
            peak_heap_bytes: 0,
            allocation_count: 0,
            deallocation_count: 0,
            rss_bytes: rss,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn capture() -> Self {
        // Read from /proc/self/statm on Linux
        let rss = std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<usize>().ok())
            .map(|pages| pages * 4096); // Assume 4KB pages

        Self {
            heap_bytes: 0,
            peak_heap_bytes: 0,
            allocation_count: 0,
            deallocation_count: 0,
            rss_bytes: rss,
        }
    }

    #[cfg(windows)]
    pub fn capture() -> Self {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        let rss = unsafe {
            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
                Some(pmc.WorkingSetSize)
            } else {
                None
            }
        };

        Self {
            heap_bytes: 0,
            peak_heap_bytes: 0,
            allocation_count: 0,
            deallocation_count: 0,
            rss_bytes: rss,
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    pub fn capture() -> Self {
        Self {
            heap_bytes: 0,
            peak_heap_bytes: 0,
            allocation_count: 0,
            deallocation_count: 0,
            rss_bytes: None,
        }
    }
}

/// Memory profiling result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProfile {
    /// Memory before benchmark.
    pub before: MemorySnapshot,
    /// Memory after benchmark.
    pub after: MemorySnapshot,
    /// Peak memory during benchmark.
    pub peak: MemorySnapshot,
    /// Memory delta.
    pub delta_bytes: i64,
}

impl MemoryProfile {
    /// Capture memory profile for a benchmark run.
    pub fn capture<F: Fn()>(f: &F) -> Self {
        let before = MemorySnapshot::capture();
        std::hint::black_box(f());
        let after = MemorySnapshot::capture();

        let delta_bytes =
            after.rss_bytes.unwrap_or(0) as i64 - before.rss_bytes.unwrap_or(0) as i64;

        Self {
            before: before.clone(),
            after: after.clone(),
            peak: after.clone(), // Approximation
            delta_bytes,
        }
    }
}

// ============================================================================
// Progress Reporting
// ============================================================================

/// Progress callback type.
pub type ProgressCallback = Box<dyn Fn(ProgressReport) + Send + Sync>;

/// Progress report for benchmark execution.
#[derive(Debug, Clone)]
pub struct ProgressReport {
    /// Current benchmark name.
    pub benchmark_name: String,
    /// Current iteration.
    pub current_iteration: usize,
    /// Total iterations.
    pub total_iterations: usize,
    /// Completed benchmarks.
    pub completed_benchmarks: usize,
    /// Total benchmarks.
    pub total_benchmarks: usize,
    /// Elapsed time.
    pub elapsed: Duration,
    /// Estimated time remaining.
    pub eta: Option<Duration>,
    /// Current phase (warmup, measurement).
    pub phase: BenchmarkPhase,
}

/// Current phase of benchmark execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkPhase {
    Warmup,
    Measurement,
    Complete,
}

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the benchmark runner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerConfig {
    /// Root directory to search for benchmarks.
    pub benchmark_dir: PathBuf,
    /// Compiler/interpreter command for Tier 0.
    pub tier0_cmd: String,
    /// JIT baseline command for Tier 1.
    pub tier1_cmd: String,
    /// JIT optimized command for Tier 2.
    pub tier2_cmd: String,
    /// AOT compiler command for Tier 3.
    pub tier3_cmd: String,
    /// Which tiers to run (0-3).
    pub tiers: Vec<u8>,
    /// Number of warmup iterations.
    pub warmup_iterations: usize,
    /// Number of measurement iterations.
    pub measure_iterations: usize,
    /// Timeout per benchmark in milliseconds.
    pub timeout_ms: u64,
    /// Number of parallel workers.
    pub parallel: usize,
    /// Filter benchmarks by pattern.
    pub filter: Option<String>,
    /// Filter by category.
    pub category: Option<BenchmarkCategory>,
    /// Performance targets.
    pub targets: PerformanceTargets,
    /// Verbose output.
    pub verbose: bool,
    /// Warmup strategy.
    #[serde(default)]
    pub warmup_strategy: WarmupStrategy,
    /// Outlier detection method.
    #[serde(default)]
    pub outlier_method: OutlierMethod,
    /// Enable memory profiling.
    #[serde(default)]
    pub memory_profiling: bool,
    /// Confidence level for intervals (e.g., 0.95, 0.99).
    #[serde(default = "default_confidence_level")]
    pub confidence_level: f64,
}

fn default_confidence_level() -> f64 {
    0.95
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            benchmark_dir: PathBuf::from("vcs/benchmarks"),
            tier0_cmd: "verum-interpreter".to_string(),
            tier1_cmd: "verum-jit --baseline".to_string(),
            tier2_cmd: "verum-jit --optimize".to_string(),
            tier3_cmd: "verum-aot".to_string(),
            tiers: vec![3], // Default to Tier 3 (AOT)
            warmup_iterations: 100,
            measure_iterations: 1000,
            timeout_ms: 60_000,
            parallel: num_cpus::get(),
            filter: None,
            category: None,
            targets: PerformanceTargets::default(),
            verbose: false,
            warmup_strategy: WarmupStrategy::default(),
            outlier_method: OutlierMethod::default(),
            memory_profiling: false,
            confidence_level: 0.95,
        }
    }
}

// ============================================================================
// Benchmark Discovery
// ============================================================================

/// Parsed benchmark file metadata.
#[derive(Debug, Clone)]
pub struct BenchmarkSpec {
    /// File path.
    pub path: PathBuf,
    /// Benchmark name (derived from file path).
    pub name: String,
    /// Benchmark category.
    pub category: BenchmarkCategory,
    /// Benchmark type (micro, macro, baseline).
    pub benchmark_type: BenchmarkType,
    /// Expected performance threshold (from @expected-performance).
    pub expected_performance: Option<String>,
    /// Parsed expected performance.
    pub parsed_expectation: Option<PerformanceExpectation>,
    /// Tags (from @tags).
    pub tags: Vec<String>,
    /// Specified tiers (from @tier).
    pub tiers: Vec<u8>,
    /// Level (from @level).
    pub level: String,
    /// Description (from @description).
    pub description: Option<String>,
    /// Timeout in milliseconds (from @timeout).
    pub timeout_ms: Option<u64>,
    /// Comparison baseline language (from @baseline).
    pub baseline: Option<BaselineLanguage>,
}

/// Type of benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenchmarkType {
    /// Single operation benchmarks (< 1ms typical).
    Micro,
    /// Realistic workload benchmarks (< 1s typical).
    Macro,
    /// Comparison against other languages (C, Rust, Go).
    Baseline,
    /// Compilation speed benchmarks.
    Compilation,
    /// SMT/verification benchmarks.
    Smt,
    /// Memory benchmarks.
    Memory,
}

impl Default for BenchmarkType {
    fn default() -> Self {
        BenchmarkType::Micro
    }
}

/// Baseline comparison language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaselineLanguage {
    C,
    Rust,
    Go,
    Java,
    Python,
}

/// Parsed performance expectation from @expected-performance directive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PerformanceExpectation {
    /// Less than a threshold (e.g., "< 15ns").
    LessThan { value_ns: f64 },
    /// Greater than a threshold (e.g., "> 1000 ops/sec").
    GreaterThan { value_ns: f64 },
    /// Within a range (e.g., "10ns-20ns").
    Range { min_ns: f64, max_ns: f64 },
    /// Ratio comparison (e.g., "0.85-1.0x Rust performance").
    Ratio {
        min_ratio: f64,
        max_ratio: f64,
        baseline: Option<BaselineLanguage>,
    },
    /// Percentage of baseline (e.g., "within 5% of baseline").
    Percentage {
        tolerance_percent: f64,
        baseline: Option<BaselineLanguage>,
    },
}

impl PerformanceExpectation {
    /// Check if a measured value meets the expectation.
    pub fn check(&self, measured_ns: f64, baseline_ns: Option<f64>) -> bool {
        match self {
            PerformanceExpectation::LessThan { value_ns } => measured_ns < *value_ns,
            PerformanceExpectation::GreaterThan { value_ns } => measured_ns > *value_ns,
            PerformanceExpectation::Range { min_ns, max_ns } => {
                measured_ns >= *min_ns && measured_ns <= *max_ns
            }
            PerformanceExpectation::Ratio {
                min_ratio,
                max_ratio,
                ..
            } => {
                if let Some(baseline) = baseline_ns {
                    if baseline > 0.0 {
                        let ratio = baseline / measured_ns;
                        ratio >= *min_ratio && ratio <= *max_ratio
                    } else {
                        false
                    }
                } else {
                    true // No baseline to compare, pass by default
                }
            }
            PerformanceExpectation::Percentage {
                tolerance_percent, ..
            } => {
                if let Some(baseline) = baseline_ns {
                    if baseline > 0.0 {
                        let diff_percent = ((measured_ns - baseline) / baseline).abs() * 100.0;
                        diff_percent <= *tolerance_percent
                    } else {
                        false
                    }
                } else {
                    true
                }
            }
        }
    }

    /// Get threshold value in nanoseconds (for simple thresholds).
    pub fn threshold_ns(&self) -> Option<f64> {
        match self {
            PerformanceExpectation::LessThan { value_ns } => Some(*value_ns),
            PerformanceExpectation::Range { max_ns, .. } => Some(*max_ns),
            _ => None,
        }
    }
}

/// Discover benchmarks in the given directory.
pub fn discover_benchmarks(dir: &Path) -> Result<Vec<BenchmarkSpec>> {
    let mut benchmarks = Vec::new();

    for entry in WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().map(|e| e == "vr").unwrap_or(false) {
            if let Some(spec) = parse_benchmark_file(path)? {
                benchmarks.push(spec);
            }
        }
    }

    Ok(benchmarks)
}

/// Discover L4 performance specs from vcs/specs/L4-performance/.
/// These are the official performance test specifications.
pub fn discover_l4_specs(vcs_root: &Path) -> Result<Vec<BenchmarkSpec>> {
    let l4_dir = vcs_root.join("specs").join("L4-performance");
    if !l4_dir.exists() {
        return Ok(Vec::new());
    }
    discover_benchmarks(&l4_dir)
}

/// Discover all benchmarks from both vcs/benchmarks/ and vcs/specs/L4-performance/.
pub fn discover_all_benchmarks(vcs_root: &Path) -> Result<Vec<BenchmarkSpec>> {
    let mut all = Vec::new();

    // Discover from vcs/benchmarks/
    let benchmarks_dir = vcs_root.join("benchmarks");
    if benchmarks_dir.exists() {
        all.extend(discover_benchmarks(&benchmarks_dir)?);
    }

    // Discover from vcs/specs/L4-performance/
    all.extend(discover_l4_specs(vcs_root)?);

    Ok(all)
}

/// Filter benchmarks by type.
pub fn filter_by_type(specs: Vec<BenchmarkSpec>, bench_type: BenchmarkType) -> Vec<BenchmarkSpec> {
    specs
        .into_iter()
        .filter(|s| s.benchmark_type == bench_type)
        .collect()
}

/// Filter benchmarks by baseline language.
pub fn filter_by_baseline(
    specs: Vec<BenchmarkSpec>,
    baseline: BaselineLanguage,
) -> Vec<BenchmarkSpec> {
    specs
        .into_iter()
        .filter(|s| s.baseline == Some(baseline))
        .collect()
}

/// Get benchmark summary by type.
pub fn summarize_benchmarks(specs: &[BenchmarkSpec]) -> BenchmarkSummary {
    let mut summary = BenchmarkSummary::default();

    for spec in specs {
        summary.total += 1;
        match spec.benchmark_type {
            BenchmarkType::Micro => summary.micro += 1,
            BenchmarkType::Macro => summary.macro_count += 1,
            BenchmarkType::Baseline => summary.baseline += 1,
            BenchmarkType::Compilation => summary.compilation += 1,
            BenchmarkType::Smt => summary.smt += 1,
            BenchmarkType::Memory => summary.memory += 1,
        }
    }

    summary
}

/// Summary of discovered benchmarks.
#[derive(Debug, Clone, Default)]
pub struct BenchmarkSummary {
    pub total: usize,
    pub micro: usize,
    pub macro_count: usize,
    pub baseline: usize,
    pub compilation: usize,
    pub smt: usize,
    pub memory: usize,
}

/// Parse a benchmark file and extract metadata.
fn parse_benchmark_file(path: &Path) -> Result<Option<BenchmarkSpec>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read benchmark file: {}", path.display()))?;

    // Check if this is a benchmark file
    if !content.contains("@test: benchmark") {
        return Ok(None);
    }

    // Extract metadata using regex
    let test_type_re = Regex::new(r"//\s*@test:\s*(\w+)")?;
    let tier_re = Regex::new(r"//\s*@tier:\s*(.+)")?;
    let level_re = Regex::new(r"//\s*@level:\s*(\w+)")?;
    let tags_re = Regex::new(r"//\s*@tags:\s*(.+)")?;
    let perf_re = Regex::new(r"//\s*@expected-performance:\s*(.+)")?;
    let timeout_re = Regex::new(r"//\s*@timeout:\s*(\d+)")?;
    let desc_re = Regex::new(r"//\s*@description:\s*(.+)")?;
    let baseline_re = Regex::new(r"//\s*@baseline:\s*(\w+)")?;

    // Check test type
    let test_type = test_type_re
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str())
        .unwrap_or("");

    if test_type != "benchmark" {
        return Ok(None);
    }

    // Parse tiers
    let tiers = tier_re
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| parse_tiers(m.as_str()))
        .unwrap_or_else(|| vec![3]);

    // Parse level
    let level = level_re
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "L4".to_string());

    // Parse tags
    let tags: Vec<String> = tags_re
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| {
            m.as_str()
                .split(',')
                .map(|s| s.trim().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Parse expected performance
    let expected_performance = perf_re
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    // Parse the performance expectation
    let parsed_expectation = expected_performance
        .as_ref()
        .and_then(|s| parse_performance_expectation(s));

    // Parse timeout
    let timeout_ms = timeout_re
        .captures(&content)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok());

    // Parse description
    let description = desc_re
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    // Parse baseline language
    let baseline = baseline_re
        .captures(&content)
        .and_then(|c| c.get(1))
        .and_then(|m| parse_baseline_language(m.as_str()));

    // Determine category from path
    let path_str = path.to_string_lossy();
    let category = if path_str.contains("/micro/") {
        BenchmarkCategory::Micro
    } else if path_str.contains("/macro/") {
        BenchmarkCategory::Macro
    } else if path_str.contains("/compilation/") {
        BenchmarkCategory::Compilation
    } else if path_str.contains("/memory/") {
        BenchmarkCategory::Memory
    } else if path_str.contains("/runtime/") {
        BenchmarkCategory::Runtime
    } else if path_str.contains("/baselines/") || path_str.contains("/comparison/") {
        BenchmarkCategory::Baseline
    } else if path_str.contains("/cbgr-latency/") {
        BenchmarkCategory::Micro
    } else {
        BenchmarkCategory::Micro
    };

    // Determine benchmark type from path and tags
    let benchmark_type = determine_benchmark_type(&path_str, &tags);

    // Generate name from path
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(Some(BenchmarkSpec {
        path: path.to_path_buf(),
        name,
        category,
        benchmark_type,
        expected_performance,
        parsed_expectation,
        tags,
        tiers,
        level,
        description,
        timeout_ms,
        baseline,
    }))
}

/// Determine benchmark type from path and tags.
fn determine_benchmark_type(path: &str, tags: &[String]) -> BenchmarkType {
    // Check tags first
    if tags.iter().any(|t| t == "micro") {
        return BenchmarkType::Micro;
    }
    if tags.iter().any(|t| t == "macro") {
        return BenchmarkType::Macro;
    }
    if tags.iter().any(|t| t == "baseline" || t == "comparison") {
        return BenchmarkType::Baseline;
    }
    if tags.iter().any(|t| t == "compilation") {
        return BenchmarkType::Compilation;
    }
    if tags.iter().any(|t| t == "smt" || t == "verification") {
        return BenchmarkType::Smt;
    }
    if tags.iter().any(|t| t == "memory") {
        return BenchmarkType::Memory;
    }

    // Fall back to path-based detection
    if path.contains("/micro/") || path.contains("/cbgr-latency/") {
        BenchmarkType::Micro
    } else if path.contains("/macro/") {
        BenchmarkType::Macro
    } else if path.contains("/comparison/") || path.contains("/baselines/") {
        BenchmarkType::Baseline
    } else if path.contains("/compilation/") {
        BenchmarkType::Compilation
    } else if path.contains("/smt/") {
        BenchmarkType::Smt
    } else if path.contains("/memory/") {
        BenchmarkType::Memory
    } else {
        BenchmarkType::Micro
    }
}

/// Parse baseline language string.
fn parse_baseline_language(s: &str) -> Option<BaselineLanguage> {
    match s.to_lowercase().as_str() {
        "c" => Some(BaselineLanguage::C),
        "rust" => Some(BaselineLanguage::Rust),
        "go" => Some(BaselineLanguage::Go),
        "java" => Some(BaselineLanguage::Java),
        "python" => Some(BaselineLanguage::Python),
        _ => None,
    }
}

/// Parse performance expectation from @expected-performance directive.
///
/// Supports formats:
/// - "< 15ns" - less than threshold
/// - "> 100ns" - greater than threshold
/// - "10ns-20ns" - range
/// - "0.85-1.0x Rust performance" - ratio comparison
/// - "within 5% of baseline" - percentage tolerance
fn parse_performance_expectation(s: &str) -> Option<PerformanceExpectation> {
    let s = s.trim();

    // Try ratio format first: "0.85-1.0x Rust performance"
    let ratio_re =
        Regex::new(r"(\d+(?:\.\d+)?)\s*-\s*(\d+(?:\.\d+)?)\s*x\s*(\w+)?\s*(?:performance)?")
            .ok()?;
    if let Some(caps) = ratio_re.captures(s) {
        let min_ratio: f64 = caps.get(1)?.as_str().parse().ok()?;
        let max_ratio: f64 = caps.get(2)?.as_str().parse().ok()?;
        let baseline = caps
            .get(3)
            .and_then(|m| parse_baseline_language(m.as_str()));
        return Some(PerformanceExpectation::Ratio {
            min_ratio,
            max_ratio,
            baseline,
        });
    }

    // Try percentage format: "within 5% of baseline"
    let pct_re = Regex::new(r"within\s+(\d+(?:\.\d+)?)\s*%").ok()?;
    if let Some(caps) = pct_re.captures(s) {
        let tolerance: f64 = caps.get(1)?.as_str().parse().ok()?;
        return Some(PerformanceExpectation::Percentage {
            tolerance_percent: tolerance,
            baseline: None,
        });
    }

    // Try range format: "10ns-20ns" or "10-20ns" or "10ns - 20ns"
    let range_re =
        Regex::new(r"(\d+(?:\.\d+)?)\s*(ns|us|ms|s)?\s*-\s*(\d+(?:\.\d+)?)\s*(ns|us|ms|s)").ok()?;
    if let Some(caps) = range_re.captures(s) {
        let min_val: f64 = caps.get(1)?.as_str().parse().ok()?;
        let min_unit = caps.get(2).map(|m| m.as_str()).unwrap_or("ns");
        let max_val: f64 = caps.get(3)?.as_str().parse().ok()?;
        let max_unit = caps.get(4)?.as_str();

        return Some(PerformanceExpectation::Range {
            min_ns: convert_to_ns(min_val, min_unit),
            max_ns: convert_to_ns(max_val, max_unit),
        });
    }

    // Try "< Xunit" format
    let lt_re = Regex::new(r"<\s*(\d+(?:\.\d+)?)\s*(ns|us|ms|s)").ok()?;
    if let Some(caps) = lt_re.captures(s) {
        let value: f64 = caps.get(1)?.as_str().parse().ok()?;
        let unit = caps.get(2)?.as_str();
        return Some(PerformanceExpectation::LessThan {
            value_ns: convert_to_ns(value, unit),
        });
    }

    // Try "> Xunit" format
    let gt_re = Regex::new(r">\s*(\d+(?:\.\d+)?)\s*(ns|us|ms|s)").ok()?;
    if let Some(caps) = gt_re.captures(s) {
        let value: f64 = caps.get(1)?.as_str().parse().ok()?;
        let unit = caps.get(2)?.as_str();
        return Some(PerformanceExpectation::GreaterThan {
            value_ns: convert_to_ns(value, unit),
        });
    }

    None
}

/// Convert a time value to nanoseconds.
fn convert_to_ns(value: f64, unit: &str) -> f64 {
    match unit {
        "ns" => value,
        "us" => value * 1_000.0,
        "ms" => value * 1_000_000.0,
        "s" => value * 1_000_000_000.0,
        _ => value, // Assume ns by default
    }
}

/// Parse tier specification (e.g., "all", "0, 3", "compiled").
fn parse_tiers(spec: &str) -> Vec<u8> {
    let spec = spec.trim().to_lowercase();
    match spec.as_str() {
        "all" => vec![0, 1, 2, 3],
        "compiled" => vec![1, 2, 3],
        _ => spec
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect(),
    }
}

// ============================================================================
// Benchmark Execution
// ============================================================================

/// Run all discovered benchmarks.
pub fn run_benchmarks(config: &RunnerConfig) -> Result<Vec<BenchmarkResult>> {
    // Discover benchmarks
    let mut specs = discover_benchmarks(&config.benchmark_dir)?;

    // Apply filters
    if let Some(ref filter) = config.filter {
        let filter_re = Regex::new(filter)?;
        specs.retain(|s| filter_re.is_match(&s.name));
    }

    if let Some(category) = config.category {
        specs.retain(|s| s.category == category);
    }

    // Run benchmarks
    let results: Arc<Mutex<Vec<BenchmarkResult>>> = Arc::new(Mutex::new(Vec::new()));

    if config.parallel > 1 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(config.parallel)
            .build()?
            .install(|| {
                specs.par_iter().for_each(|spec| {
                    for tier in &config.tiers {
                        if spec.tiers.contains(tier) {
                            if let Ok(result) = run_single_benchmark(spec, *tier, config) {
                                results.lock().push(result);
                            }
                        }
                    }
                });
            });
    } else {
        for spec in &specs {
            for tier in &config.tiers {
                if spec.tiers.contains(tier) {
                    if let Ok(result) = run_single_benchmark(spec, *tier, config) {
                        results.lock().push(result);
                    }
                }
            }
        }
    }

    Ok(Arc::try_unwrap(results)
        .expect("Should be unique")
        .into_inner())
}

/// Run a single benchmark.
fn run_single_benchmark(
    spec: &BenchmarkSpec,
    tier: u8,
    config: &RunnerConfig,
) -> Result<BenchmarkResult> {
    let cmd = match tier {
        0 => &config.tier0_cmd,
        1 => &config.tier1_cmd,
        2 => &config.tier2_cmd,
        3 => &config.tier3_cmd,
        _ => return Err(anyhow::anyhow!("Invalid tier: {}", tier)),
    };

    let timeout = Duration::from_millis(spec.timeout_ms.unwrap_or(config.timeout_ms));

    // Warmup
    for _ in 0..config.warmup_iterations {
        execute_benchmark(cmd, &spec.path, timeout)?;
    }

    // Measure
    let mut measurements = Vec::with_capacity(config.measure_iterations);
    for _ in 0..config.measure_iterations {
        let duration = execute_benchmark(cmd, &spec.path, timeout)?;
        measurements.push(Measurement::new(duration));
    }

    // Calculate statistics
    let statistics = Statistics::from_measurements(&measurements)
        .ok_or_else(|| anyhow::anyhow!("No measurements collected"))?;

    // Get threshold from parsed expectation or fall back to old parsing
    let threshold_ns = spec
        .parsed_expectation
        .as_ref()
        .and_then(|e| e.threshold_ns())
        .or_else(|| {
            spec.expected_performance
                .as_ref()
                .and_then(|s| parse_performance_threshold(s))
        })
        .or_else(|| config.targets.get_threshold(&spec.name));

    let mut result = BenchmarkResult::new(
        format!("{}@tier{}", spec.name, tier),
        spec.category,
        statistics,
        threshold_ns,
    )
    .with_tier(tier);

    // Add metadata
    result = result
        .with_metadata("file", spec.path.display().to_string())
        .with_metadata("level", spec.level.clone())
        .with_metadata("benchmark_type", format!("{:?}", spec.benchmark_type));

    if let Some(ref desc) = spec.description {
        result = result.with_metadata("description", desc.clone());
    }

    if let Some(ref baseline) = spec.baseline {
        result = result.with_metadata("baseline", format!("{:?}", baseline));
    }

    if let Some(ref expectation) = spec.parsed_expectation {
        result = result.with_metadata("expectation", format!("{:?}", expectation));
    }

    for tag in &spec.tags {
        result = result.with_metadata("tag", tag.clone());
    }

    Ok(result)
}

/// Execute a benchmark once and return the duration.
fn execute_benchmark(cmd: &str, path: &Path, timeout: Duration) -> Result<Duration> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return Err(anyhow::anyhow!("Empty command"));
    }

    let start = Instant::now();

    let mut command = Command::new(parts[0]);
    for arg in &parts[1..] {
        command.arg(arg);
    }
    command.arg(path);
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());

    let child = command
        .spawn()
        .context("Failed to spawn benchmark process")?;

    // Wait with timeout
    let output = child
        .wait_with_output()
        .context("Failed to wait for benchmark process")?;

    let duration = start.elapsed();

    if duration > timeout {
        return Err(anyhow::anyhow!("Benchmark timed out"));
    }

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Benchmark failed with exit code: {:?}",
            output.status.code()
        ));
    }

    Ok(duration)
}

/// Parse a performance threshold string (e.g., "< 15ns", "< 100ms").
fn parse_performance_threshold(s: &str) -> Option<f64> {
    let s = s.trim();

    // Handle "< Xns" format
    let ns_re = Regex::new(r"<\s*(\d+(?:\.\d+)?)\s*ns").ok()?;
    if let Some(caps) = ns_re.captures(s) {
        return caps.get(1)?.as_str().parse().ok();
    }

    // Handle "< Xms" format
    let ms_re = Regex::new(r"<\s*(\d+(?:\.\d+)?)\s*ms").ok()?;
    if let Some(caps) = ms_re.captures(s) {
        let ms: f64 = caps.get(1)?.as_str().parse().ok()?;
        return Some(ms * 1_000_000.0);
    }

    // Handle "< Xus" format
    let us_re = Regex::new(r"<\s*(\d+(?:\.\d+)?)\s*us").ok()?;
    if let Some(caps) = us_re.captures(s) {
        let us: f64 = caps.get(1)?.as_str().parse().ok()?;
        return Some(us * 1_000.0);
    }

    None
}

// ============================================================================
// In-Process Benchmarks
// ============================================================================

/// Benchmark function type for in-process benchmarks.
pub type BenchFn = Box<dyn Fn() + Send + Sync>;

/// In-process benchmark specification.
pub struct InProcessBenchmark {
    pub name: String,
    pub category: BenchmarkCategory,
    pub setup: Option<Box<dyn Fn() + Send + Sync>>,
    pub benchmark: BenchFn,
    pub teardown: Option<Box<dyn Fn() + Send + Sync>>,
    pub iterations: usize,
    pub threshold_ns: Option<f64>,
    pub warmup_strategy: WarmupStrategy,
    pub outlier_method: OutlierMethod,
    pub memory_profiling: bool,
}

impl InProcessBenchmark {
    /// Create a new in-process benchmark.
    pub fn new(name: impl Into<String>, benchmark: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            name: name.into(),
            category: BenchmarkCategory::Micro,
            setup: None,
            benchmark: Box::new(benchmark),
            teardown: None,
            iterations: 1000,
            threshold_ns: None,
            warmup_strategy: WarmupStrategy::default(),
            outlier_method: OutlierMethod::default(),
            memory_profiling: false,
        }
    }

    /// Set the category.
    pub fn with_category(mut self, category: BenchmarkCategory) -> Self {
        self.category = category;
        self
    }

    /// Set the number of iterations.
    pub fn with_iterations(mut self, iterations: usize) -> Self {
        self.iterations = iterations;
        self
    }

    /// Set the expected threshold.
    pub fn with_threshold(mut self, threshold_ns: f64) -> Self {
        self.threshold_ns = Some(threshold_ns);
        self
    }

    /// Set the setup function.
    pub fn with_setup(mut self, setup: impl Fn() + Send + Sync + 'static) -> Self {
        self.setup = Some(Box::new(setup));
        self
    }

    /// Set the teardown function.
    pub fn with_teardown(mut self, teardown: impl Fn() + Send + Sync + 'static) -> Self {
        self.teardown = Some(Box::new(teardown));
        self
    }

    /// Set the warmup strategy.
    pub fn with_warmup_strategy(mut self, strategy: WarmupStrategy) -> Self {
        self.warmup_strategy = strategy;
        self
    }

    /// Set the outlier detection method.
    pub fn with_outlier_method(mut self, method: OutlierMethod) -> Self {
        self.outlier_method = method;
        self
    }

    /// Enable memory profiling.
    pub fn with_memory_profiling(mut self, enabled: bool) -> Self {
        self.memory_profiling = enabled;
        self
    }

    /// Run the benchmark.
    pub fn run(&self, warmup: usize) -> BenchmarkResult {
        // Setup
        if let Some(ref setup) = self.setup {
            setup();
        }

        // Warmup using strategy
        let _warmup_count = match &self.warmup_strategy {
            WarmupStrategy::Fixed(_) => {
                // Use provided warmup if Fixed strategy doesn't match
                for _ in 0..warmup {
                    std::hint::black_box((self.benchmark)());
                }
                warmup
            }
            strategy => strategy.run_warmup(&self.benchmark),
        };

        // Capture memory before
        let memory_before = if self.memory_profiling {
            Some(MemorySnapshot::capture())
        } else {
            None
        };

        // Measure
        let mut durations = Vec::with_capacity(self.iterations);
        for _ in 0..self.iterations {
            let start = Instant::now();
            std::hint::black_box((self.benchmark)());
            durations.push(start.elapsed());
        }

        // Capture memory after
        let memory_after = if self.memory_profiling {
            Some(MemorySnapshot::capture())
        } else {
            None
        };

        // Teardown
        if let Some(ref teardown) = self.teardown {
            teardown();
        }

        // Convert to nanoseconds for outlier detection
        let ns_values: Vec<f64> = durations.iter().map(|d| d.as_nanos() as f64).collect();

        // Apply outlier detection
        let outlier_result = self.outlier_method.filter(&ns_values);

        // Use filtered measurements for statistics
        let filtered_durations: Vec<Duration> = outlier_result
            .filtered
            .iter()
            .map(|&ns| Duration::from_nanos(ns as u64))
            .collect();

        // Calculate statistics
        let statistics =
            Statistics::from_durations(&filtered_durations).expect("Should have measurements");

        let mut result = BenchmarkResult::new(
            self.name.clone(),
            self.category,
            statistics,
            self.threshold_ns,
        );

        // Add outlier metadata
        if !outlier_result.outlier_indices.is_empty() {
            result = result.with_metadata(
                "outliers_removed",
                outlier_result.outlier_indices.len().to_string(),
            );
        }

        // Add memory profiling metadata
        if let (Some(before), Some(after)) = (memory_before, memory_after) {
            if let (Some(rss_before), Some(rss_after)) = (before.rss_bytes, after.rss_bytes) {
                result = result.with_metadata(
                    "memory_delta_bytes",
                    (rss_after as i64 - rss_before as i64).to_string(),
                );
            }
        }

        result
    }
}

/// Run a collection of in-process benchmarks.
pub fn run_in_process(
    benchmarks: Vec<InProcessBenchmark>,
    warmup: usize,
    parallel: bool,
) -> Vec<BenchmarkResult> {
    if parallel {
        benchmarks.into_par_iter().map(|b| b.run(warmup)).collect()
    } else {
        benchmarks.into_iter().map(|b| b.run(warmup)).collect()
    }
}

// ============================================================================
// Criterion Integration
// ============================================================================

/// Criterion-style benchmark group.
pub struct BenchmarkGroup {
    name: String,
    benchmarks: Vec<InProcessBenchmark>,
    warmup: usize,
    iterations: usize,
    warmup_strategy: WarmupStrategy,
    outlier_method: OutlierMethod,
    memory_profiling: bool,
    progress_callback: Option<Arc<ProgressCallback>>,
    completed_count: Arc<AtomicUsize>,
}

impl BenchmarkGroup {
    /// Create a new benchmark group.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            benchmarks: Vec::new(),
            warmup: 100,
            iterations: 1000,
            warmup_strategy: WarmupStrategy::default(),
            outlier_method: OutlierMethod::default(),
            memory_profiling: false,
            progress_callback: None,
            completed_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Set warmup iterations.
    pub fn warmup(mut self, warmup: usize) -> Self {
        self.warmup = warmup;
        self
    }

    /// Set measurement iterations.
    pub fn iterations(mut self, iterations: usize) -> Self {
        self.iterations = iterations;
        self
    }

    /// Set warmup strategy.
    pub fn warmup_strategy(mut self, strategy: WarmupStrategy) -> Self {
        self.warmup_strategy = strategy;
        self
    }

    /// Set outlier detection method.
    pub fn outlier_detection(mut self, method: OutlierMethod) -> Self {
        self.outlier_method = method;
        self
    }

    /// Enable memory profiling.
    pub fn memory_profiling(mut self, enabled: bool) -> Self {
        self.memory_profiling = enabled;
        self
    }

    /// Set progress callback.
    pub fn on_progress(
        mut self,
        callback: impl Fn(ProgressReport) + Send + Sync + 'static,
    ) -> Self {
        self.progress_callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Add a benchmark.
    pub fn bench(mut self, name: impl Into<String>, f: impl Fn() + Send + Sync + 'static) -> Self {
        let bench = InProcessBenchmark::new(format!("{}/{}", self.name, name.into()), f)
            .with_iterations(self.iterations)
            .with_warmup_strategy(self.warmup_strategy.clone())
            .with_outlier_method(self.outlier_method.clone())
            .with_memory_profiling(self.memory_profiling);
        self.benchmarks.push(bench);
        self
    }

    /// Add a benchmark with a threshold.
    pub fn bench_with_threshold(
        mut self,
        name: impl Into<String>,
        threshold_ns: f64,
        f: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let bench = InProcessBenchmark::new(format!("{}/{}", self.name, name.into()), f)
            .with_iterations(self.iterations)
            .with_threshold(threshold_ns)
            .with_warmup_strategy(self.warmup_strategy.clone())
            .with_outlier_method(self.outlier_method.clone())
            .with_memory_profiling(self.memory_profiling);
        self.benchmarks.push(bench);
        self
    }

    /// Run all benchmarks in the group.
    pub fn run(self) -> Vec<BenchmarkResult> {
        let total = self.benchmarks.len();
        let start = Instant::now();
        let completed = self.completed_count.clone();
        let callback = self.progress_callback.clone();
        let warmup = self.warmup;

        let results: Vec<BenchmarkResult> = self
            .benchmarks
            .into_par_iter()
            .map(|b| {
                let result = b.run(warmup);

                // Update progress
                let count = completed.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some(ref cb) = callback {
                    let elapsed = start.elapsed();
                    let avg_per_bench = elapsed.as_secs_f64() / count as f64;
                    let remaining = total - count;
                    let eta = Duration::from_secs_f64(avg_per_bench * remaining as f64);

                    cb(ProgressReport {
                        benchmark_name: result.name.clone(),
                        current_iteration: 0,
                        total_iterations: 0,
                        completed_benchmarks: count,
                        total_benchmarks: total,
                        elapsed,
                        eta: Some(eta),
                        phase: BenchmarkPhase::Complete,
                    });
                }

                result
            })
            .collect();

        results
    }
}

// ============================================================================
// Extended Statistics
// ============================================================================

/// Extended statistics with confidence intervals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendedStatistics {
    /// Base statistics.
    pub base: Statistics,
    /// 95% confidence interval for the mean.
    pub ci_95: (f64, f64),
    /// 99% confidence interval for the mean.
    pub ci_99: (f64, f64),
    /// Coefficient of variation (std_dev / mean).
    pub coefficient_of_variation: f64,
    /// Skewness of the distribution.
    pub skewness: f64,
    /// Kurtosis of the distribution.
    pub kurtosis: f64,
    /// Number of outliers detected.
    pub outliers_count: usize,
}

impl ExtendedStatistics {
    /// Calculate extended statistics from measurements.
    pub fn from_measurements(
        measurements: &[f64],
        _confidence_level: f64,
        outlier_method: &OutlierMethod,
    ) -> Option<Self> {
        if measurements.is_empty() {
            return None;
        }

        // Apply outlier detection
        let outlier_result = outlier_method.filter(measurements);
        let data = &outlier_result.filtered;

        if data.is_empty() {
            return None;
        }

        let n = data.len() as f64;
        let mean = data.iter().sum::<f64>() / n;
        let variance = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let std_dev = variance.sqrt();

        // Confidence intervals
        let stderr = std_dev / n.sqrt();
        let z_95 = 1.96;
        let z_99 = 2.576;
        let ci_95 = (mean - z_95 * stderr, mean + z_95 * stderr);
        let ci_99 = (mean - z_99 * stderr, mean + z_99 * stderr);

        // Coefficient of variation
        let cv = if mean != 0.0 { std_dev / mean } else { 0.0 };

        // Skewness
        let skewness = if std_dev > 0.0 {
            let m3 = data
                .iter()
                .map(|x| ((x - mean) / std_dev).powi(3))
                .sum::<f64>()
                / n;
            m3
        } else {
            0.0
        };

        // Kurtosis
        let kurtosis = if std_dev > 0.0 {
            let m4 = data
                .iter()
                .map(|x| ((x - mean) / std_dev).powi(4))
                .sum::<f64>()
                / n;
            m4 - 3.0 // Excess kurtosis
        } else {
            0.0
        };

        // Create base statistics
        let durations: Vec<Duration> = data
            .iter()
            .map(|&ns| Duration::from_nanos(ns as u64))
            .collect();
        let base = Statistics::from_durations(&durations)?;

        Some(Self {
            base,
            ci_95,
            ci_99,
            coefficient_of_variation: cv,
            skewness,
            kurtosis,
            outliers_count: outlier_result.outlier_indices.len(),
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tiers() {
        assert_eq!(parse_tiers("all"), vec![0, 1, 2, 3]);
        assert_eq!(parse_tiers("compiled"), vec![1, 2, 3]);
        assert_eq!(parse_tiers("0, 3"), vec![0, 3]);
        assert_eq!(parse_tiers("3"), vec![3]);
    }

    #[test]
    fn test_parse_performance_threshold() {
        assert_eq!(parse_performance_threshold("< 15ns"), Some(15.0));
        assert_eq!(parse_performance_threshold("< 100ms"), Some(100_000_000.0));
        assert_eq!(parse_performance_threshold("< 50us"), Some(50_000.0));
        assert_eq!(parse_performance_threshold("invalid"), None);
    }

    #[test]
    fn test_parse_performance_expectation_less_than() {
        let result = parse_performance_expectation("< 15ns");
        assert!(
            matches!(result, Some(PerformanceExpectation::LessThan { value_ns }) if (value_ns - 15.0).abs() < 0.001)
        );

        let result = parse_performance_expectation("< 100ms");
        assert!(
            matches!(result, Some(PerformanceExpectation::LessThan { value_ns }) if (value_ns - 100_000_000.0).abs() < 0.001)
        );
    }

    #[test]
    fn test_parse_performance_expectation_range() {
        let result = parse_performance_expectation("10ns-20ns");
        assert!(
            matches!(result, Some(PerformanceExpectation::Range { min_ns, max_ns })
            if (min_ns - 10.0).abs() < 0.001 && (max_ns - 20.0).abs() < 0.001)
        );
    }

    #[test]
    fn test_parse_performance_expectation_ratio() {
        let result = parse_performance_expectation("0.85-1.0x Rust performance");
        assert!(
            matches!(result, Some(PerformanceExpectation::Ratio { min_ratio, max_ratio, baseline })
            if (min_ratio - 0.85).abs() < 0.001
            && (max_ratio - 1.0).abs() < 0.001
            && baseline == Some(BaselineLanguage::Rust))
        );
    }

    #[test]
    fn test_parse_performance_expectation_percentage() {
        let result = parse_performance_expectation("within 5% of baseline");
        assert!(
            matches!(result, Some(PerformanceExpectation::Percentage { tolerance_percent, .. })
            if (tolerance_percent - 5.0).abs() < 0.001)
        );
    }

    #[test]
    fn test_performance_expectation_check() {
        let less_than = PerformanceExpectation::LessThan { value_ns: 100.0 };
        assert!(less_than.check(50.0, None));
        assert!(!less_than.check(150.0, None));

        let range = PerformanceExpectation::Range {
            min_ns: 10.0,
            max_ns: 20.0,
        };
        assert!(range.check(15.0, None));
        assert!(!range.check(5.0, None));
        assert!(!range.check(25.0, None));

        let ratio = PerformanceExpectation::Ratio {
            min_ratio: 0.85,
            max_ratio: 1.0,
            baseline: Some(BaselineLanguage::Rust),
        };
        // If baseline is 100ns and we measured 110ns, ratio = 100/110 = 0.909 which is in range
        assert!(ratio.check(110.0, Some(100.0)));
        // If we measured 200ns, ratio = 100/200 = 0.5 which is below 0.85
        assert!(!ratio.check(200.0, Some(100.0)));
    }

    #[test]
    fn test_determine_benchmark_type() {
        let tags_micro = vec!["micro".to_string()];
        assert_eq!(
            determine_benchmark_type("/some/path", &tags_micro),
            BenchmarkType::Micro
        );

        let tags_empty: Vec<String> = vec![];
        assert_eq!(
            determine_benchmark_type("/path/micro/test.vr", &tags_empty),
            BenchmarkType::Micro
        );
        assert_eq!(
            determine_benchmark_type("/path/macro/test.vr", &tags_empty),
            BenchmarkType::Macro
        );
        assert_eq!(
            determine_benchmark_type("/path/comparison/test.vr", &tags_empty),
            BenchmarkType::Baseline
        );
    }

    #[test]
    fn test_warmup_fixed() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let count = AtomicUsize::new(0);
        let strategy = WarmupStrategy::Fixed(10);
        let result = strategy.run_warmup(&|| {
            count.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(result, 10);
        assert_eq!(count.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn test_outlier_iqr() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 100.0]; // 100 is an outlier
        let method = OutlierMethod::Iqr { k: 1.5 };
        let result = method.filter(&data);
        assert!(!result.outlier_indices.is_empty());
        assert!(result.filtered.len() < data.len());
    }

    #[test]
    fn test_outlier_mad() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 100.0];
        let method = OutlierMethod::Mad { threshold: 3.0 };
        let result = method.filter(&data);
        assert!(!result.outlier_indices.is_empty());
    }

    #[test]
    fn test_outlier_percentile() {
        let data: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let method = OutlierMethod::Percentile {
            lower: 10.0,
            upper: 90.0,
        };
        let result = method.filter(&data);
        assert!(result.filtered.len() < data.len());
    }

    #[test]
    fn test_in_process_benchmark() {
        let bench = InProcessBenchmark::new("test", || {
            let mut sum = 0u64;
            for i in 0..100 {
                sum += i;
            }
            std::hint::black_box(sum);
        })
        .with_iterations(100)
        .with_threshold(1_000_000.0); // 1ms

        let result = bench.run(10);
        assert_eq!(result.name, "test");
        assert!(result.statistics.count > 0);
    }

    #[test]
    fn test_benchmark_group() {
        let results = BenchmarkGroup::new("test_group")
            .warmup(10)
            .iterations(50)
            .bench("add", || {
                std::hint::black_box(1 + 1);
            })
            .bench("mul", || {
                std::hint::black_box(2 * 2);
            })
            .run();

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_benchmark_group_with_outlier_detection() {
        let results = BenchmarkGroup::new("outlier_test")
            .warmup(5)
            .iterations(20)
            .outlier_detection(OutlierMethod::Iqr { k: 1.5 })
            .bench("test", || {
                std::hint::black_box(1 + 1);
            })
            .run();

        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_memory_snapshot() {
        let snapshot = MemorySnapshot::capture();
        // Just verify it doesn't panic
        let _ = snapshot.rss_bytes;
    }

    #[test]
    fn test_median() {
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0, 5.0]), 3.0);
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn test_percentile() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&data, 0.0), 1.0);
        assert_eq!(percentile(&data, 100.0), 5.0);
        assert_eq!(percentile(&data, 50.0), 3.0);
    }

    #[test]
    fn test_extended_statistics() {
        let data: Vec<f64> = (0..100).map(|i| i as f64 + 100.0).collect();
        let stats = ExtendedStatistics::from_measurements(&data, 0.95, &OutlierMethod::None);
        assert!(stats.is_some());
        let stats = stats.unwrap();
        assert!(stats.coefficient_of_variation > 0.0);
    }
}
