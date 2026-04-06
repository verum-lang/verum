//! Performance metrics collection for VCS benchmarks.
//!
//! This module provides types and utilities for collecting, analyzing, and
//! storing performance metrics from benchmark runs. It includes statistical
//! analysis functions and threshold checking for performance regression detection.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

// ============================================================================
// Core Metric Types
// ============================================================================

/// A single timing measurement with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    /// Duration of the measurement.
    pub duration: Duration,
    /// Timestamp when the measurement was taken.
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Optional iteration count for throughput calculations.
    pub iterations: Option<u64>,
    /// Optional memory usage in bytes.
    pub memory_bytes: Option<u64>,
}

impl Measurement {
    /// Create a new measurement with the current timestamp.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            timestamp: chrono::Utc::now(),
            iterations: None,
            memory_bytes: None,
        }
    }

    /// Create a measurement with iteration count.
    pub fn with_iterations(duration: Duration, iterations: u64) -> Self {
        Self {
            duration,
            timestamp: chrono::Utc::now(),
            iterations: Some(iterations),
            memory_bytes: None,
        }
    }

    /// Add memory usage information.
    pub fn with_memory(mut self, bytes: u64) -> Self {
        self.memory_bytes = Some(bytes);
        self
    }

    /// Calculate nanoseconds per operation.
    pub fn ns_per_op(&self) -> Option<f64> {
        self.iterations.map(|iters| {
            if iters == 0 {
                0.0
            } else {
                self.duration.as_nanos() as f64 / iters as f64
            }
        })
    }

    /// Calculate operations per second.
    pub fn ops_per_sec(&self) -> Option<f64> {
        self.iterations.map(|iters| {
            let secs = self.duration.as_secs_f64();
            if secs == 0.0 {
                0.0
            } else {
                iters as f64 / secs
            }
        })
    }
}

/// Statistical summary of multiple measurements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Statistics {
    /// Number of samples.
    pub count: usize,
    /// Minimum value in nanoseconds.
    pub min_ns: f64,
    /// Maximum value in nanoseconds.
    pub max_ns: f64,
    /// Mean (average) in nanoseconds.
    pub mean_ns: f64,
    /// Median value in nanoseconds.
    pub median_ns: f64,
    /// Standard deviation in nanoseconds.
    pub std_dev_ns: f64,
    /// Coefficient of variation (std_dev / mean).
    pub cv: f64,
    /// 5th percentile in nanoseconds.
    pub p5_ns: f64,
    /// 25th percentile in nanoseconds.
    pub p25_ns: f64,
    /// 75th percentile in nanoseconds.
    pub p75_ns: f64,
    /// 95th percentile in nanoseconds.
    pub p95_ns: f64,
    /// 99th percentile in nanoseconds.
    pub p99_ns: f64,
    /// Interquartile range (p75 - p25).
    pub iqr_ns: f64,
    /// Total duration of all measurements.
    pub total_duration: Duration,
}

impl Statistics {
    /// Calculate statistics from a slice of durations.
    pub fn from_durations(durations: &[Duration]) -> Option<Self> {
        if durations.is_empty() {
            return None;
        }

        let mut ns_values: Vec<f64> = durations.iter().map(|d| d.as_nanos() as f64).collect();
        ns_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let count = ns_values.len();
        let min_ns = ns_values[0];
        let max_ns = ns_values[count - 1];
        let sum: f64 = ns_values.iter().sum();
        let mean_ns = sum / count as f64;

        // Calculate median
        let median_ns = if count % 2 == 0 {
            (ns_values[count / 2 - 1] + ns_values[count / 2]) / 2.0
        } else {
            ns_values[count / 2]
        };

        // Calculate standard deviation
        let variance: f64 =
            ns_values.iter().map(|x| (x - mean_ns).powi(2)).sum::<f64>() / count as f64;
        let std_dev_ns = variance.sqrt();

        // Coefficient of variation
        let cv = if mean_ns != 0.0 {
            std_dev_ns / mean_ns
        } else {
            0.0
        };

        // Percentiles
        let p5_ns = percentile(&ns_values, 5.0);
        let p25_ns = percentile(&ns_values, 25.0);
        let p75_ns = percentile(&ns_values, 75.0);
        let p95_ns = percentile(&ns_values, 95.0);
        let p99_ns = percentile(&ns_values, 99.0);
        let iqr_ns = p75_ns - p25_ns;

        let total_duration = durations.iter().sum();

        Some(Self {
            count,
            min_ns,
            max_ns,
            mean_ns,
            median_ns,
            std_dev_ns,
            cv,
            p5_ns,
            p25_ns,
            p75_ns,
            p95_ns,
            p99_ns,
            iqr_ns,
            total_duration,
        })
    }

    /// Calculate statistics from measurements.
    pub fn from_measurements(measurements: &[Measurement]) -> Option<Self> {
        let durations: Vec<Duration> = measurements.iter().map(|m| m.duration).collect();
        Self::from_durations(&durations)
    }

    /// Check if the mean is within a threshold (in nanoseconds).
    pub fn within_threshold(&self, threshold_ns: f64) -> bool {
        self.mean_ns <= threshold_ns
    }

    /// Calculate the margin of error at 95% confidence.
    pub fn margin_of_error_95(&self) -> f64 {
        // Using z-score of 1.96 for 95% confidence
        1.96 * self.std_dev_ns / (self.count as f64).sqrt()
    }

    /// Get confidence interval at 95%.
    pub fn confidence_interval_95(&self) -> (f64, f64) {
        let moe = self.margin_of_error_95();
        (self.mean_ns - moe, self.mean_ns + moe)
    }
}

/// Calculate percentile from sorted values.
fn percentile(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    if sorted_values.len() == 1 {
        return sorted_values[0];
    }

    let idx = (p / 100.0 * (sorted_values.len() - 1) as f64).round() as usize;
    sorted_values[idx.min(sorted_values.len() - 1)]
}

// ============================================================================
// Benchmark Result Types
// ============================================================================

/// Result of a single benchmark execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Benchmark name/identifier.
    pub name: String,
    /// Category (e.g., "micro", "macro").
    pub category: BenchmarkCategory,
    /// Execution tier (0-3).
    pub tier: Option<u8>,
    /// Statistical summary.
    pub statistics: Statistics,
    /// Target threshold in nanoseconds (if specified).
    pub threshold_ns: Option<f64>,
    /// Whether the benchmark passed its threshold.
    pub passed: bool,
    /// Optional throughput (ops/sec, bytes/sec, etc.).
    pub throughput: Option<Throughput>,
    /// Additional metadata.
    pub metadata: HashMap<String, String>,
}

impl BenchmarkResult {
    /// Create a new benchmark result.
    pub fn new(
        name: String,
        category: BenchmarkCategory,
        statistics: Statistics,
        threshold_ns: Option<f64>,
    ) -> Self {
        let passed = threshold_ns
            .map(|t| statistics.mean_ns <= t)
            .unwrap_or(true);

        Self {
            name,
            category,
            tier: None,
            statistics,
            threshold_ns,
            passed,
            throughput: None,
            metadata: HashMap::new(),
        }
    }

    /// Set the execution tier.
    pub fn with_tier(mut self, tier: u8) -> Self {
        self.tier = Some(tier);
        self
    }

    /// Set throughput information.
    pub fn with_throughput(mut self, throughput: Throughput) -> Self {
        self.throughput = Some(throughput);
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Benchmark category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkCategory {
    /// Micro-benchmarks: individual operations (~1ns - ~1ms).
    Micro,
    /// Macro-benchmarks: realistic workloads (~1ms - ~1s).
    Macro,
    /// Compilation benchmarks.
    Compilation,
    /// Runtime benchmarks.
    Runtime,
    /// Memory benchmarks.
    Memory,
    /// Baseline comparison.
    Baseline,
}

impl std::fmt::Display for BenchmarkCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Micro => write!(f, "micro"),
            Self::Macro => write!(f, "macro"),
            Self::Compilation => write!(f, "compilation"),
            Self::Runtime => write!(f, "runtime"),
            Self::Memory => write!(f, "memory"),
            Self::Baseline => write!(f, "baseline"),
        }
    }
}

/// Throughput measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Throughput {
    /// Operations per second.
    OpsPerSec(f64),
    /// Bytes per second.
    BytesPerSec(f64),
    /// Lines of code per second.
    LocPerSec(f64),
    /// Elements per second.
    ElementsPerSec(f64),
}

impl Throughput {
    /// Format throughput for display.
    pub fn format(&self) -> String {
        match self {
            Self::OpsPerSec(v) => format_throughput(*v, "op/s"),
            Self::BytesPerSec(v) => format_bytes_throughput(*v),
            Self::LocPerSec(v) => format_throughput(*v, "LOC/s"),
            Self::ElementsPerSec(v) => format_throughput(*v, "elem/s"),
        }
    }
}

fn format_throughput(value: f64, unit: &str) -> String {
    if value >= 1_000_000_000.0 {
        format!("{:.2}G {}", value / 1_000_000_000.0, unit)
    } else if value >= 1_000_000.0 {
        format!("{:.2}M {}", value / 1_000_000.0, unit)
    } else if value >= 1_000.0 {
        format!("{:.2}K {}", value / 1_000.0, unit)
    } else {
        format!("{:.2} {}", value, unit)
    }
}

fn format_bytes_throughput(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_000_000_000.0 {
        format!("{:.2} GB/s", bytes_per_sec / 1_000_000_000.0)
    } else if bytes_per_sec >= 1_000_000.0 {
        format!("{:.2} MB/s", bytes_per_sec / 1_000_000.0)
    } else if bytes_per_sec >= 1_000.0 {
        format!("{:.2} KB/s", bytes_per_sec / 1_000.0)
    } else {
        format!("{:.2} B/s", bytes_per_sec)
    }
}

// ============================================================================
// Performance Thresholds
// ============================================================================

/// Target performance thresholds from VCS spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceTargets {
    /// CBGR check latency target (default: 15ns).
    pub cbgr_check_ns: f64,
    /// Type inference target (default: 100ms per 10K LOC).
    pub type_inference_ms_per_10k_loc: f64,
    /// Compilation speed target (default: 50K LOC/sec).
    pub compilation_loc_per_sec: f64,
    /// Runtime performance vs C (default: 0.85-0.95x).
    pub runtime_vs_c_min: f64,
    pub runtime_vs_c_max: f64,
    /// Memory overhead target (default: < 5%).
    pub memory_overhead_percent: f64,
    /// Custom thresholds by benchmark name.
    pub custom: HashMap<String, f64>,
}

impl Default for PerformanceTargets {
    fn default() -> Self {
        Self {
            cbgr_check_ns: 15.0,
            type_inference_ms_per_10k_loc: 100.0,
            compilation_loc_per_sec: 50_000.0,
            runtime_vs_c_min: 0.85,
            runtime_vs_c_max: 0.95,
            memory_overhead_percent: 5.0,
            custom: HashMap::new(),
        }
    }
}

impl PerformanceTargets {
    /// Get threshold for a benchmark by name.
    pub fn get_threshold(&self, name: &str) -> Option<f64> {
        // Check custom thresholds first
        if let Some(&threshold) = self.custom.get(name) {
            return Some(threshold);
        }

        // Check known benchmark patterns
        if name.contains("cbgr") || name.contains("CBGR") {
            return Some(self.cbgr_check_ns);
        }

        None
    }

    /// Add a custom threshold.
    pub fn with_threshold(mut self, name: impl Into<String>, threshold_ns: f64) -> Self {
        self.custom.insert(name.into(), threshold_ns);
        self
    }
}

// ============================================================================
// Timer Utilities
// ============================================================================

/// High-precision timer for benchmark measurements.
pub struct Timer {
    start: Option<Instant>,
    measurements: Vec<Duration>,
}

impl Timer {
    /// Create a new timer.
    pub fn new() -> Self {
        Self {
            start: None,
            measurements: Vec::new(),
        }
    }

    /// Start the timer.
    pub fn start(&mut self) {
        self.start = Some(Instant::now());
    }

    /// Stop the timer and record the measurement.
    pub fn stop(&mut self) -> Option<Duration> {
        if let Some(start) = self.start.take() {
            let duration = start.elapsed();
            self.measurements.push(duration);
            Some(duration)
        } else {
            None
        }
    }

    /// Get all recorded measurements.
    pub fn measurements(&self) -> &[Duration] {
        &self.measurements
    }

    /// Get statistics for all measurements.
    pub fn statistics(&self) -> Option<Statistics> {
        Statistics::from_durations(&self.measurements)
    }

    /// Clear all measurements.
    pub fn clear(&mut self) {
        self.start = None;
        self.measurements.clear();
    }

    /// Measure a closure N times and return statistics.
    pub fn measure<F, R>(&mut self, iterations: usize, mut f: F) -> Option<Statistics>
    where
        F: FnMut() -> R,
    {
        self.clear();
        for _ in 0..iterations {
            self.start();
            std::hint::black_box(f());
            self.stop();
        }
        self.statistics()
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

/// Measure execution time of a closure.
pub fn measure_once<F, R>(f: F) -> (Duration, R)
where
    F: FnOnce() -> R,
{
    let start = Instant::now();
    let result = f();
    let duration = start.elapsed();
    (duration, result)
}

/// Measure execution time of a closure multiple times.
pub fn measure_n<F, R>(iterations: usize, mut f: F) -> Vec<Duration>
where
    F: FnMut() -> R,
{
    (0..iterations)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(f());
            start.elapsed()
        })
        .collect()
}

/// Black box to prevent compiler optimizations.
pub fn black_box<T>(x: T) -> T {
    std::hint::black_box(x)
}

// ============================================================================
// Memory Metrics
// ============================================================================

/// Memory usage snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    /// Heap allocation in bytes.
    pub heap_bytes: u64,
    /// Stack usage in bytes (estimated).
    pub stack_bytes: Option<u64>,
    /// Resident set size in bytes.
    pub rss_bytes: Option<u64>,
    /// Virtual memory size in bytes.
    pub virtual_bytes: Option<u64>,
}

impl MemorySnapshot {
    /// Create a snapshot of current memory usage.
    #[cfg(target_os = "macos")]
    pub fn capture() -> Option<Self> {
        use std::process::Command;

        // Use ps command to get memory info on macOS
        let output = Command::new("ps")
            .args(["-o", "rss,vsz", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        if lines.len() < 2 {
            return None;
        }

        let parts: Vec<&str> = lines[1].split_whitespace().collect();
        if parts.len() < 2 {
            return None;
        }

        let rss_kb: u64 = parts[0].parse().ok()?;
        let vsz_kb: u64 = parts[1].parse().ok()?;

        Some(Self {
            heap_bytes: 0, // Would need allocator tracking
            stack_bytes: None,
            rss_bytes: Some(rss_kb * 1024),
            virtual_bytes: Some(vsz_kb * 1024),
        })
    }

    #[cfg(target_os = "linux")]
    pub fn capture() -> Option<Self> {
        use std::fs;

        let status = fs::read_to_string("/proc/self/status").ok()?;
        let mut rss_bytes = None;
        let mut virtual_bytes = None;

        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                if let Some(kb) = line.split_whitespace().nth(1) {
                    rss_bytes = kb.parse::<u64>().ok().map(|kb| kb * 1024);
                }
            } else if line.starts_with("VmSize:") {
                if let Some(kb) = line.split_whitespace().nth(1) {
                    virtual_bytes = kb.parse::<u64>().ok().map(|kb| kb * 1024);
                }
            }
        }

        Some(Self {
            heap_bytes: 0,
            stack_bytes: None,
            rss_bytes,
            virtual_bytes,
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn capture() -> Option<Self> {
        None
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statistics_calculation() {
        let durations: Vec<Duration> = vec![
            Duration::from_nanos(10),
            Duration::from_nanos(20),
            Duration::from_nanos(30),
            Duration::from_nanos(40),
            Duration::from_nanos(50),
        ];

        let stats = Statistics::from_durations(&durations).unwrap();

        assert_eq!(stats.count, 5);
        assert_eq!(stats.min_ns, 10.0);
        assert_eq!(stats.max_ns, 50.0);
        assert_eq!(stats.mean_ns, 30.0);
        assert_eq!(stats.median_ns, 30.0);
    }

    #[test]
    fn test_measurement_ops_per_sec() {
        let measurement = Measurement::with_iterations(Duration::from_secs(1), 1_000_000);

        assert_eq!(measurement.ops_per_sec(), Some(1_000_000.0));
        assert_eq!(measurement.ns_per_op(), Some(1000.0));
    }

    #[test]
    fn test_timer() {
        let mut timer = Timer::new();
        timer.start();
        std::thread::sleep(Duration::from_millis(1));
        let duration = timer.stop().unwrap();

        assert!(duration >= Duration::from_millis(1));
        assert_eq!(timer.measurements().len(), 1);
    }

    #[test]
    fn test_performance_targets() {
        let targets = PerformanceTargets::default().with_threshold("custom_bench", 100.0);

        assert_eq!(targets.get_threshold("cbgr_check"), Some(15.0));
        assert_eq!(targets.get_threshold("custom_bench"), Some(100.0));
        assert_eq!(targets.get_threshold("unknown"), None);
    }

    #[test]
    fn test_threshold_check() {
        let stats = Statistics {
            count: 100,
            min_ns: 10.0,
            max_ns: 20.0,
            mean_ns: 14.0,
            median_ns: 14.0,
            std_dev_ns: 2.0,
            cv: 0.14,
            p5_ns: 11.0,
            p25_ns: 12.0,
            p75_ns: 16.0,
            p95_ns: 18.0,
            p99_ns: 19.0,
            iqr_ns: 4.0,
            total_duration: Duration::from_nanos(1400),
        };

        assert!(stats.within_threshold(15.0));
        assert!(!stats.within_threshold(13.0));
    }

    #[test]
    fn test_throughput_format() {
        assert_eq!(Throughput::OpsPerSec(1_500_000.0).format(), "1.50M op/s");
        assert_eq!(
            Throughput::BytesPerSec(1_000_000_000.0).format(),
            "1.00 GB/s"
        );
    }
}
