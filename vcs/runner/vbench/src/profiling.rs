//! Profiling and flamegraph integration for VCS benchmarks.
//!
//! This module provides hooks for integrating with profiling tools:
//! - Flamegraph generation
//! - CPU profiling
//! - Memory profiling
//! - Instruction counting
//!
//! # Supported Profilers
//!
//! - `perf` (Linux)
//! - `Instruments` (macOS)
//! - `samply` (cross-platform)
//! - `pprof` (Rust integration)

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::metrics::Statistics;

// ============================================================================
// Profiling Configuration
// ============================================================================

/// Configuration for profiling a benchmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilingConfig {
    /// Which profiler to use.
    pub profiler: Profiler,
    /// Output directory for profiling data.
    pub output_dir: PathBuf,
    /// Sampling frequency (Hz).
    pub frequency: u32,
    /// Duration to profile (if not using iterations).
    pub duration: Option<Duration>,
    /// Number of iterations to profile.
    pub iterations: Option<usize>,
    /// Generate flamegraph SVG.
    pub flamegraph: bool,
    /// Collapse repeated frames.
    pub collapse_recursion: bool,
    /// Filter to specific functions.
    pub filter: Option<String>,
    /// Additional profiler arguments.
    pub extra_args: Vec<String>,
}

impl Default for ProfilingConfig {
    fn default() -> Self {
        Self {
            profiler: Profiler::default(),
            output_dir: PathBuf::from("profiles"),
            frequency: 99,
            duration: None,
            iterations: Some(1000),
            flamegraph: true,
            collapse_recursion: true,
            filter: None,
            extra_args: Vec::new(),
        }
    }
}

/// Supported profilers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profiler {
    /// Linux perf
    Perf,
    /// macOS Instruments/DTrace
    Instruments,
    /// Cross-platform samply
    Samply,
    /// Built-in timing-based profiler
    Builtin,
    /// No profiler (disabled)
    None,
}

impl Default for Profiler {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::Perf
        }
        #[cfg(target_os = "macos")]
        {
            Self::Instruments
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Self::Builtin
        }
    }
}

impl Profiler {
    /// Check if this profiler is available on the current system.
    pub fn is_available(&self) -> bool {
        match self {
            Self::Perf => Command::new("perf")
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false),
            Self::Instruments => Command::new("sample")
                .arg("-h")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok(),
            Self::Samply => Command::new("samply")
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false),
            Self::Builtin => true,
            Self::None => true,
        }
    }
}

// ============================================================================
// Profiling Session
// ============================================================================

/// A profiling session for benchmarks.
pub struct ProfilingSession {
    config: ProfilingConfig,
    name: String,
    samples: Vec<Sample>,
    start_time: Option<Instant>,
}

/// A single profiling sample.
#[derive(Debug, Clone)]
pub struct Sample {
    /// Timestamp relative to session start.
    pub timestamp: Duration,
    /// Stack trace (function names, bottom to top).
    pub stack: Vec<String>,
    /// CPU time for this sample.
    pub cpu_time: Option<Duration>,
    /// Memory usage at this sample.
    pub memory_bytes: Option<u64>,
}

impl ProfilingSession {
    /// Create a new profiling session.
    pub fn new(name: &str, config: ProfilingConfig) -> Self {
        Self {
            config,
            name: name.to_string(),
            samples: Vec::new(),
            start_time: None,
        }
    }

    /// Start the profiling session.
    pub fn start(&mut self) {
        self.start_time = Some(Instant::now());
        self.samples.clear();
    }

    /// Stop the profiling session.
    pub fn stop(&mut self) -> Duration {
        self.start_time
            .map(|s| s.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    /// Add a sample.
    pub fn add_sample(&mut self, stack: Vec<String>) {
        if let Some(start) = self.start_time {
            self.samples.push(Sample {
                timestamp: start.elapsed(),
                stack,
                cpu_time: None,
                memory_bytes: None,
            });
        }
    }

    /// Get all samples.
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// Generate a flamegraph from collected samples.
    pub fn generate_flamegraph(&self) -> Result<String> {
        // Create collapsed stacks format
        let mut stack_counts: HashMap<String, usize> = HashMap::new();

        for sample in &self.samples {
            let stack_str = sample.stack.join(";");
            *stack_counts.entry(stack_str).or_insert(0) += 1;
        }

        let mut collapsed = String::new();
        for (stack, count) in &stack_counts {
            collapsed.push_str(&format!("{} {}\n", stack, count));
        }

        Ok(collapsed)
    }

    /// Save profiling results.
    pub fn save(&self) -> Result<PathBuf> {
        fs::create_dir_all(&self.config.output_dir)?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("{}_{}.txt", self.name, timestamp);
        let path = self.config.output_dir.join(&filename);

        let collapsed = self.generate_flamegraph()?;
        fs::write(&path, &collapsed)?;

        // Generate SVG if requested and inferno is available
        if self.config.flamegraph {
            self.generate_svg(&path, &collapsed)?;
        }

        Ok(path)
    }

    /// Generate SVG flamegraph using inferno.
    fn generate_svg(&self, collapsed_path: &Path, collapsed: &str) -> Result<()> {
        let svg_path = collapsed_path.with_extension("svg");

        // Try using inferno-flamegraph if available
        let output = Command::new("inferno-flamegraph")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();

        match output {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(collapsed.as_bytes())?;
                }
                let output = child.wait_with_output()?;
                if output.status.success() {
                    fs::write(&svg_path, output.stdout)?;
                }
            }
            Err(_) => {
                // inferno not available, create a simple text-based visualization
                let viz = self.create_text_visualization();
                let txt_path = collapsed_path.with_extension("tree.txt");
                fs::write(txt_path, viz)?;
            }
        }

        Ok(())
    }

    /// Create a simple text-based call tree visualization.
    fn create_text_visualization(&self) -> String {
        let mut stack_counts: HashMap<String, usize> = HashMap::new();

        for sample in &self.samples {
            let stack_str = sample.stack.join(" -> ");
            *stack_counts.entry(stack_str).or_insert(0) += 1;
        }

        let mut sorted: Vec<_> = stack_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        let total_samples = self.samples.len();
        let mut output = String::new();

        output.push_str(&format!("Call Tree for '{}'\n", self.name));
        output.push_str(&format!("Total samples: {}\n\n", total_samples));

        for (stack, count) in sorted.iter().take(20) {
            let pct = (*count as f64 / total_samples as f64) * 100.0;
            output.push_str(&format!("{:5.1}% ({:5}) {}\n", pct, count, stack));
        }

        output
    }
}

// ============================================================================
// Profiled Benchmark Runner
// ============================================================================

/// Run a benchmark with profiling enabled.
pub fn run_profiled<F>(
    name: &str,
    config: &ProfilingConfig,
    iterations: usize,
    mut f: F,
) -> Result<ProfiledResult>
where
    F: FnMut(),
{
    let mut session = ProfilingSession::new(name, config.clone());
    session.start();

    let mut durations = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let start = Instant::now();
        f();
        let duration = start.elapsed();
        durations.push(duration);

        // Capture a simple stack trace (current function + benchmark)
        session.add_sample(vec![name.to_string()]);
    }

    let total_duration = session.stop();

    // Calculate statistics
    let statistics = Statistics::from_durations(&durations);

    // Save profiling data
    let profile_path = if config.profiler != Profiler::None {
        Some(session.save()?)
    } else {
        None
    };

    Ok(ProfiledResult {
        name: name.to_string(),
        iterations,
        total_duration,
        statistics,
        profile_path,
        samples: session.samples.len(),
    })
}

/// Result of a profiled benchmark run.
#[derive(Debug, Clone)]
pub struct ProfiledResult {
    /// Benchmark name.
    pub name: String,
    /// Number of iterations run.
    pub iterations: usize,
    /// Total profiling duration.
    pub total_duration: Duration,
    /// Timing statistics.
    pub statistics: Option<Statistics>,
    /// Path to profile data.
    pub profile_path: Option<PathBuf>,
    /// Number of samples collected.
    pub samples: usize,
}

// ============================================================================
// External Profiler Integration
// ============================================================================

/// Run a command under perf profiling (Linux).
pub fn profile_with_perf(
    command: &str,
    args: &[&str],
    output_path: &Path,
    frequency: u32,
) -> Result<()> {
    let perf_data = output_path.with_extension("perf.data");

    let status = Command::new("perf")
        .args([
            "record",
            "-F",
            &frequency.to_string(),
            "-g",
            "-o",
            perf_data.to_str().unwrap(),
            "--",
            command,
        ])
        .args(args)
        .status()
        .context("Failed to run perf record")?;

    if !status.success() {
        return Err(anyhow::anyhow!("perf record failed"));
    }

    // Generate flamegraph-compatible output
    let script_output = Command::new("perf")
        .args(["script", "-i", perf_data.to_str().unwrap()])
        .output()
        .context("Failed to run perf script")?;

    let stacks = output_path.with_extension("stacks");
    fs::write(&stacks, script_output.stdout)?;

    Ok(())
}

/// Run a command under samply profiling.
pub fn profile_with_samply(command: &str, args: &[&str], output_path: &Path) -> Result<()> {
    let status = Command::new("samply")
        .args(["record", "-o", output_path.to_str().unwrap(), "--", command])
        .args(args)
        .status()
        .context("Failed to run samply")?;

    if !status.success() {
        return Err(anyhow::anyhow!("samply record failed"));
    }

    Ok(())
}

/// Run a command under macOS sample profiling.
#[cfg(target_os = "macos")]
pub fn profile_with_sample(pid: u32, duration_secs: u32, output_path: &Path) -> Result<()> {
    let status = Command::new("sample")
        .args([
            &pid.to_string(),
            &duration_secs.to_string(),
            "-file",
            output_path.to_str().unwrap(),
        ])
        .status()
        .context("Failed to run sample")?;

    if !status.success() {
        return Err(anyhow::anyhow!("sample failed"));
    }

    Ok(())
}

// ============================================================================
// Flamegraph Generation
// ============================================================================

/// Configuration for flamegraph generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlamegraphConfig {
    /// Title for the flamegraph.
    pub title: String,
    /// Width in pixels.
    pub width: u32,
    /// Minimum width for frames (pixels).
    pub min_width: f64,
    /// Color scheme.
    pub colors: ColorScheme,
    /// Invert the graph (icicle graph).
    pub inverted: bool,
    /// Collapse recursion.
    pub collapse_recursion: bool,
}

impl Default for FlamegraphConfig {
    fn default() -> Self {
        Self {
            title: "VBench Flamegraph".to_string(),
            width: 1200,
            min_width: 0.1,
            colors: ColorScheme::Hot,
            inverted: false,
            collapse_recursion: true,
        }
    }
}

/// Color scheme for flamegraphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorScheme {
    Hot,
    Mem,
    Io,
    Blue,
    Green,
    Purple,
    Orange,
}

/// Generate a simple SVG flamegraph from collapsed stacks.
pub fn generate_flamegraph_svg(collapsed: &str, config: &FlamegraphConfig) -> Result<String> {
    // Parse collapsed format: "func1;func2;func3 count"
    let mut stack_counts: Vec<(Vec<&str>, usize)> = Vec::new();

    for line in collapsed.lines() {
        let parts: Vec<&str> = line.rsplitn(2, ' ').collect();
        if parts.len() == 2 {
            if let Ok(count) = parts[0].parse::<usize>() {
                let stack: Vec<&str> = parts[1].split(';').collect();
                stack_counts.push((stack, count));
            }
        }
    }

    if stack_counts.is_empty() {
        return Ok(String::new());
    }

    // Calculate total samples
    let total: usize = stack_counts.iter().map(|(_, c)| c).sum();

    // Build frame tree
    let mut frames: HashMap<String, FrameData> = HashMap::new();

    for (stack, count) in &stack_counts {
        for (depth, func) in stack.iter().enumerate() {
            let key = format!("{};{}", depth, func);
            let frame = frames.entry(key).or_insert(FrameData {
                name: func.to_string(),
                depth,
                samples: 0,
            });
            frame.samples += count;
        }
    }

    // Generate simple SVG
    let height_per_frame = 16.0;
    let max_depth = frames.values().map(|f| f.depth).max().unwrap_or(0) + 1;
    let svg_height = (max_depth as f64 * height_per_frame) + 50.0;

    let width = config.width;
    let height = svg_height as u32;

    let mut svg = String::new();
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">"#,
        width, height, width, height
    ));
    svg.push_str(
        r#"
<style>
  .frame { fill: #f8b878; stroke: #d08050; stroke-width: 0.5; }
  .frame:hover { fill: #ffcc88; }
  .label { font: 12px monospace; fill: #000; }
  .title { font: 14px sans-serif; fill: #000; }
</style>
"#,
    );
    svg.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#f0f0f0\"/>",
        width, height
    ));
    svg.push_str(&format!(
        r#"<text x="10" y="20" class="title">{}</text>"#,
        config.title
    ));
    svg.push_str(&format!(
        r#"<text x="10" y="35" class="label">Total samples: {}</text>"#,
        total
    ));
    svg.push('\n');

    // Sort frames by depth and samples
    let mut sorted_frames: Vec<_> = frames.values().collect();
    sorted_frames.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| b.samples.cmp(&a.samples))
    });

    let y_base = svg_height - 20.0;

    for frame in sorted_frames.iter().take(100) {
        let width_ratio = frame.samples as f64 / total as f64;
        let frame_width = (config.width as f64 - 20.0) * width_ratio;

        if frame_width < config.min_width {
            continue;
        }

        let y = y_base - ((frame.depth + 1) as f64 * height_per_frame);
        let x = 10.0; // Simplified: all at same x

        svg.push_str(&format!(
            r#"<rect class="frame" x="{:.1}" y="{:.1}" width="{:.1}" height="15">
<title>{} ({} samples, {:.1}%)</title>
</rect>
"#,
            x,
            y,
            frame_width.min(config.width as f64 - 20.0),
            frame.name,
            frame.samples,
            width_ratio * 100.0
        ));

        // Add label if frame is wide enough
        if frame_width > 50.0 {
            svg.push_str(&format!(
                r#"<text class="label" x="{:.1}" y="{:.1}">{}</text>
"#,
                x + 2.0,
                y + 12.0,
                truncate(&frame.name, ((frame_width - 4.0) / 7.0) as usize)
            ));
        }
    }

    svg.push_str("</svg>");

    Ok(svg)
}

/// Frame data for flamegraph generation.
struct FrameData {
    name: String,
    depth: usize,
    samples: usize,
}

/// Truncate a string to a maximum length.
fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else if max_len > 3 {
        &s[..max_len - 3]
    } else {
        &s[..max_len]
    }
}

// ============================================================================
// CPU Metrics (Linux)
// ============================================================================

/// CPU performance counters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuMetrics {
    /// Number of CPU cycles.
    pub cycles: u64,
    /// Number of instructions executed.
    pub instructions: u64,
    /// Instructions per cycle (IPC).
    pub ipc: f64,
    /// Cache misses.
    pub cache_misses: u64,
    /// Branch misses.
    pub branch_misses: u64,
}

/// Collect CPU metrics for a function (Linux only with perf support).
#[cfg(target_os = "linux")]
pub fn collect_cpu_metrics<F, R>(f: F) -> (R, Option<CpuMetrics>)
where
    F: FnOnce() -> R,
{
    // Try to use perf stat
    let result = f();

    // Note: In a real implementation, this would use perf_event_open
    // or the perf crate to collect actual hardware counters.
    // This is a placeholder that returns None.
    (result, None)
}

#[cfg(not(target_os = "linux"))]
pub fn collect_cpu_metrics<F, R>(f: F) -> (R, Option<CpuMetrics>)
where
    F: FnOnce() -> R,
{
    (f(), None)
}

// ============================================================================
// Memory Metrics
// ============================================================================

/// Memory allocation metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryMetrics {
    /// Total bytes allocated.
    pub bytes_allocated: u64,
    /// Number of allocations.
    pub allocation_count: u64,
    /// Total bytes deallocated.
    pub bytes_deallocated: u64,
    /// Number of deallocations.
    pub deallocation_count: u64,
    /// Peak memory usage.
    pub peak_bytes: u64,
    /// Current memory usage.
    pub current_bytes: u64,
}

/// A simple memory tracking allocator wrapper.
pub struct TrackedAllocator {
    bytes_allocated: std::sync::atomic::AtomicU64,
    bytes_deallocated: std::sync::atomic::AtomicU64,
    allocation_count: std::sync::atomic::AtomicU64,
    deallocation_count: std::sync::atomic::AtomicU64,
    peak_bytes: std::sync::atomic::AtomicU64,
}

impl TrackedAllocator {
    /// Create a new tracked allocator.
    pub const fn new() -> Self {
        Self {
            bytes_allocated: std::sync::atomic::AtomicU64::new(0),
            bytes_deallocated: std::sync::atomic::AtomicU64::new(0),
            allocation_count: std::sync::atomic::AtomicU64::new(0),
            deallocation_count: std::sync::atomic::AtomicU64::new(0),
            peak_bytes: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Get current metrics.
    pub fn metrics(&self) -> MemoryMetrics {
        use std::sync::atomic::Ordering::Relaxed;

        let allocated = self.bytes_allocated.load(Relaxed);
        let deallocated = self.bytes_deallocated.load(Relaxed);

        MemoryMetrics {
            bytes_allocated: allocated,
            allocation_count: self.allocation_count.load(Relaxed),
            bytes_deallocated: deallocated,
            deallocation_count: self.deallocation_count.load(Relaxed),
            peak_bytes: self.peak_bytes.load(Relaxed),
            current_bytes: allocated.saturating_sub(deallocated),
        }
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.bytes_allocated.store(0, Relaxed);
        self.bytes_deallocated.store(0, Relaxed);
        self.allocation_count.store(0, Relaxed);
        self.deallocation_count.store(0, Relaxed);
        self.peak_bytes.store(0, Relaxed);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profiler_availability() {
        // Built-in should always be available
        assert!(Profiler::Builtin.is_available());
        assert!(Profiler::None.is_available());
    }

    #[test]
    fn test_profiling_session() {
        let config = ProfilingConfig::default();
        let mut session = ProfilingSession::new("test", config);

        session.start();

        for i in 0..10 {
            session.add_sample(vec![format!("func_{}", i % 3)]);
        }

        session.stop();

        assert_eq!(session.samples().len(), 10);
    }

    #[test]
    fn test_generate_flamegraph() {
        let config = ProfilingConfig::default();
        let mut session = ProfilingSession::new("test", config);

        session.start();
        session.add_sample(vec!["main".to_string(), "foo".to_string()]);
        session.add_sample(vec!["main".to_string(), "bar".to_string()]);
        session.add_sample(vec!["main".to_string(), "foo".to_string()]);
        session.stop();

        let collapsed = session.generate_flamegraph().unwrap();
        assert!(collapsed.contains("main;foo"));
        assert!(collapsed.contains("main;bar"));
    }

    #[test]
    fn test_run_profiled() {
        let config = ProfilingConfig {
            profiler: Profiler::None, // Disable actual profiling for test
            ..Default::default()
        };

        let result = run_profiled("test", &config, 10, || {
            let _: u64 = (0..100).sum();
        });

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.name, "test");
        assert_eq!(result.iterations, 10);
    }

    #[test]
    fn test_flamegraph_config() {
        let config = FlamegraphConfig::default();
        assert_eq!(config.width, 1200);
        assert!(config.collapse_recursion);
    }

    #[test]
    fn test_flamegraph_svg_generation() {
        let collapsed = "main;foo 10\nmain;bar 5\nmain;foo;baz 3\n";
        let config = FlamegraphConfig::default();

        let svg = generate_flamegraph_svg(collapsed, &config).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("VBench Flamegraph"));
    }

    #[test]
    fn test_memory_metrics() {
        let allocator = TrackedAllocator::new();
        let metrics = allocator.metrics();

        assert_eq!(metrics.bytes_allocated, 0);
        assert_eq!(metrics.allocation_count, 0);
    }
}
