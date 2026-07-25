//! Compilation Profiling and Metrics Infrastructure
//!
//! This module provides comprehensive profiling and metrics collection for the
//! Verum compilation pipeline. It tracks:
//!
//! - Time spent in each compilation phase
//! - Memory allocation during compilation
//! - Slow modules and functions (bottleneck identification)
//! - Per-phase performance metrics
//! - Overall compilation statistics
//!
//! # Example Usage
//!
//! ```no_run
//! use verum_compiler::compilation_metrics::{CompilationProfileReport, ModuleMetrics};
//! use std::time::Duration;
//!
//! let mut report = CompilationProfileReport::new();
//!
//! // Track phase execution
//! report.record_phase("Lexical Parsing", Duration::from_millis(50), 1024 * 512);
//! report.record_phase("Semantic Analysis", Duration::from_millis(150), 1024 * 1024);
//!
//! // Track module compilation
//! report.add_module("main.vr", Duration::from_millis(200), 100);
//!
//! // Generate human-readable report
//! println!("{}", report.summary());
//!
//! // Export as JSON for tooling
//! let json = report.to_json().unwrap();
//! ```
//!
//! Multi-pass compilation pipeline: Parse → Meta Registry → Macro Expansion →
//! Contract Verification → Semantic Analysis → HIR → MIR → Optimization → Codegen.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use verum_common::{List, Map, Text};

/// Complete profiling report for compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationProfileReport {
    /// Metrics for each compilation phase
    pub phase_metrics: List<PhasePerformanceMetrics>,

    /// Metrics for each compiled module
    pub module_metrics: List<ModuleMetrics>,

    /// Total compilation time
    pub total_duration: Duration,

    /// Total memory allocated during compilation (bytes)
    pub total_memory_bytes: usize,

    /// Peak memory usage (bytes)
    pub peak_memory_bytes: usize,

    /// Bottlenecks detected (slowest phases/modules)
    pub bottlenecks: List<Bottleneck>,

    /// Overall statistics
    pub stats: CompilationStats,
}

/// Performance metrics for a single compilation phase
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhasePerformanceMetrics {
    /// Phase name (e.g., "Lexical Parsing", "Type Checking")
    pub phase_name: Text,

    /// Time spent in this phase
    #[serde(
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub duration: Duration,

    /// Memory allocated during this phase (bytes)
    pub memory_allocated: usize,

    /// Number of items processed (files, functions, etc.)
    pub items_processed: usize,

    /// Percentage of total compilation time
    pub time_percentage: f64,

    /// Percentage of total memory usage
    pub memory_percentage: f64,

    /// Phase-specific custom metrics
    pub custom_metrics: Map<Text, Text>,
}

/// Metrics for a single module compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleMetrics {
    /// Module name/path
    pub module_name: Text,

    /// Time to compile this module
    #[serde(
        serialize_with = "serialize_duration",
        deserialize_with = "deserialize_duration"
    )]
    pub duration: Duration,

    /// Number of functions in this module
    pub function_count: usize,

    /// Lines of code in this module
    pub lines_of_code: usize,

    /// Memory used for this module (bytes)
    pub memory_bytes: usize,

    /// Is this module a bottleneck?
    pub is_slow: bool,
}

/// Identified performance bottleneck
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    /// Type of bottleneck
    pub kind: BottleneckKind,

    /// Location (phase name or module name)
    pub location: Text,

    /// Description of the issue
    pub description: Text,

    /// Severity (percentage of total time/memory)
    pub severity: f64,

    /// Suggested optimization
    pub suggestion: Text,
}

/// Type of performance bottleneck
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BottleneckKind {
    /// Slow compilation phase
    SlowPhase,

    /// Slow module
    SlowModule,

    /// High memory usage
    HighMemory,

    /// Excessive items processed
    HighItemCount,
}

/// Overall compilation statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompilationStats {
    /// Total number of modules compiled
    pub modules_compiled: usize,

    /// Total number of functions compiled
    pub functions_compiled: usize,

    /// Total lines of code compiled
    pub total_loc: usize,

    /// Compilation speed (LOC/sec)
    pub compilation_speed_loc_per_sec: f64,

    /// Average time per module (ms)
    pub avg_time_per_module_ms: f64,

    /// Average memory per module (KB)
    pub avg_memory_per_module_kb: f64,
}

impl CompilationProfileReport {
    /// Create a new empty profiling report
    pub fn new() -> Self {
        Self {
            phase_metrics: List::new(),
            module_metrics: List::new(),
            total_duration: Duration::from_secs(0),
            total_memory_bytes: 0,
            peak_memory_bytes: 0,
            bottlenecks: List::new(),
            stats: CompilationStats::default(),
        }
    }

    /// Record execution of a compilation phase
    pub fn record_phase(
        &mut self,
        phase_name: impl Into<Text>,
        duration: Duration,
        memory_allocated: usize,
    ) {
        let phase_name = phase_name.into();
        self.phase_metrics.push(PhasePerformanceMetrics {
            phase_name,
            duration,
            memory_allocated,
            items_processed: 0,
            time_percentage: 0.0,
            memory_percentage: 0.0,
            custom_metrics: Map::new(),
        });

        self.total_duration += duration;
        self.total_memory_bytes += memory_allocated;
        self.peak_memory_bytes = self.peak_memory_bytes.max(memory_allocated);
    }

    /// Add a module to the profiling report
    pub fn add_module(
        &mut self,
        module_name: impl Into<Text>,
        duration: Duration,
        function_count: usize,
    ) {
        let module_name = module_name.into();
        self.module_metrics.push(ModuleMetrics {
            module_name,
            duration,
            function_count,
            lines_of_code: 0,
            memory_bytes: 0,
            is_slow: false,
        });

        self.stats.modules_compiled += 1;
        self.stats.functions_compiled += function_count;
    }

    /// Finalize the report by calculating percentages and detecting bottlenecks
    pub fn finalize(&mut self) {
        self.calculate_percentages();
        self.detect_bottlenecks();
        self.calculate_stats();
    }

    /// Calculate time and memory percentages for each phase
    fn calculate_percentages(&mut self) {
        let total_time_ms = self.total_duration.as_millis() as f64;
        let total_memory = self.total_memory_bytes as f64;

        for phase in &mut self.phase_metrics {
            let phase_time_ms = phase.duration.as_millis() as f64;
            phase.time_percentage = if total_time_ms > 0.0 {
                (phase_time_ms / total_time_ms) * 100.0
            } else {
                0.0
            };

            phase.memory_percentage = if total_memory > 0.0 {
                (phase.memory_allocated as f64 / total_memory) * 100.0
            } else {
                0.0
            };
        }
    }

    /// Detect performance bottlenecks
    fn detect_bottlenecks(&mut self) {
        const SLOW_PHASE_THRESHOLD: f64 = 20.0; // % of total time
        const SLOW_MODULE_THRESHOLD_MS: u128 = 100; // milliseconds
        const HIGH_MEMORY_THRESHOLD: f64 = 25.0; // % of total memory

        // Detect slow phases
        for phase in &self.phase_metrics {
            if phase.time_percentage >= SLOW_PHASE_THRESHOLD {
                self.bottlenecks.push(Bottleneck {
                    kind: BottleneckKind::SlowPhase,
                    location: phase.phase_name.clone(),
                    description: format!(
                        "Phase takes {:.1}% of total compilation time",
                        phase.time_percentage
                    )
                    .into(),
                    severity: phase.time_percentage,
                    suggestion: "Consider parallelization or incremental compilation".into(),
                });
            }

            if phase.memory_percentage >= HIGH_MEMORY_THRESHOLD {
                self.bottlenecks.push(Bottleneck {
                    kind: BottleneckKind::HighMemory,
                    location: phase.phase_name.clone(),
                    description: format!(
                        "Phase uses {:.1}% of total memory",
                        phase.memory_percentage
                    )
                    .into(),
                    severity: phase.memory_percentage,
                    suggestion: "Consider streaming or chunked processing".into(),
                });
            }
        }

        // Detect slow modules
        for module in &mut self.module_metrics {
            if module.duration.as_millis() >= SLOW_MODULE_THRESHOLD_MS {
                module.is_slow = true;
                self.bottlenecks.push(Bottleneck {
                    kind: BottleneckKind::SlowModule,
                    location: module.module_name.clone(),
                    description: format!(
                        "Module takes {}ms to compile",
                        module.duration.as_millis()
                    )
                    .into(),
                    severity: (module.duration.as_millis() as f64
                        / self.total_duration.as_millis() as f64)
                        * 100.0,
                    suggestion: "Consider splitting into smaller modules".into(),
                });
            }
        }
    }

    /// Calculate overall statistics
    fn calculate_stats(&mut self) {
        let total_secs = self.total_duration.as_secs_f64();

        self.stats.compilation_speed_loc_per_sec = if total_secs > 0.0 {
            self.stats.total_loc as f64 / total_secs
        } else {
            0.0
        };

        self.stats.avg_time_per_module_ms = if self.stats.modules_compiled > 0 {
            self.total_duration.as_millis() as f64 / self.stats.modules_compiled as f64
        } else {
            0.0
        };

        self.stats.avg_memory_per_module_kb = if self.stats.modules_compiled > 0 {
            (self.total_memory_bytes as f64 / 1024.0) / self.stats.modules_compiled as f64
        } else {
            0.0
        };
    }

    /// Generate a human-readable summary report
    pub fn summary(&self) -> Text {
        let mut output = String::new();

        output.push_str("=== Compilation Profile Report ===\n\n");

        // Overall stats
        output.push_str(&format!(
            "Total Time: {:.2}s\n",
            self.total_duration.as_secs_f64()
        ));
        output.push_str(&format!(
            "Total Memory: {:.2} MB\n",
            self.total_memory_bytes as f64 / (1024.0 * 1024.0)
        ));
        output.push_str(&format!(
            "Peak Memory: {:.2} MB\n",
            self.peak_memory_bytes as f64 / (1024.0 * 1024.0)
        ));
        output.push_str(&format!("Modules: {}\n", self.stats.modules_compiled));
        output.push_str(&format!("Functions: {}\n", self.stats.functions_compiled));
        output.push_str(&format!(
            "Compilation Speed: {:.0} LOC/sec\n\n",
            self.stats.compilation_speed_loc_per_sec
        ));

        // Phase breakdown
        output.push_str("=== Phase Breakdown ===\n\n");
        for phase in &self.phase_metrics {
            output.push_str(&format!(
                "{}: {:.0}ms ({:.1}%) | {:.2} MB ({:.1}%)\n",
                phase.phase_name,
                phase.duration.as_millis(),
                phase.time_percentage,
                phase.memory_allocated as f64 / (1024.0 * 1024.0),
                phase.memory_percentage
            ));
        }

        // Bottlenecks
        if !self.bottlenecks.is_empty() {
            output.push_str("\n=== Performance Bottlenecks ===\n\n");
            for bottleneck in &self.bottlenecks {
                output.push_str(&format!(
                    "[{:?}] {}: {}\n  → {}\n",
                    bottleneck.kind,
                    bottleneck.location,
                    bottleneck.description,
                    bottleneck.suggestion
                ));
            }
        }

        // Slow modules
        let slow_modules: List<_> = self.module_metrics.iter().filter(|m| m.is_slow).collect();

        if !slow_modules.is_empty() {
            output.push_str("\n=== Slow Modules ===\n\n");
            for module in slow_modules {
                output.push_str(&format!(
                    "{}: {}ms ({} functions)\n",
                    module.module_name,
                    module.duration.as_millis(),
                    module.function_count
                ));
            }
        }

        output.into()
    }

    /// Export report as JSON
    pub fn to_json(&self) -> Result<Text, serde_json::Error> {
        serde_json::to_string_pretty(self).map(|s| s.into())
    }

    /// Import report from JSON
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

impl Default for CompilationProfileReport {
    fn default() -> Self {
        Self::new()
    }
}

// Helper functions for Duration serialization
fn serialize_duration<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_u64(duration.as_millis() as u64)
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let millis = u64::deserialize(deserializer)?;
    Ok(Duration::from_millis(millis))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compilation_report_basic() {
        let mut report = CompilationProfileReport::new();

        report.record_phase("Parsing", Duration::from_millis(100), 1024 * 1024);
        report.record_phase("Type Checking", Duration::from_millis(200), 2 * 1024 * 1024);

        report.add_module("main.vr", Duration::from_millis(150), 10);

        report.finalize();

        assert_eq!(report.phase_metrics.len(), 2);
        assert_eq!(report.module_metrics.len(), 1);
        assert_eq!(report.stats.modules_compiled, 1);
        assert_eq!(report.stats.functions_compiled, 10);
    }

    #[test]
    fn test_phase_percentages() {
        let mut report = CompilationProfileReport::new();

        report.record_phase("Phase A", Duration::from_millis(50), 1024);
        report.record_phase("Phase B", Duration::from_millis(50), 1024);

        report.finalize();

        // Each phase should be 50% of total time
        for phase in &report.phase_metrics {
            assert!((phase.time_percentage - 50.0).abs() < 0.1);
        }
    }

    #[test]
    fn test_bottleneck_detection() {
        let mut report = CompilationProfileReport::new();

        // Add a slow phase (>20% of total time)
        report.record_phase("Slow Phase", Duration::from_millis(250), 1024);
        report.record_phase("Fast Phase", Duration::from_millis(50), 512);

        report.finalize();

        // Should detect the slow phase as a bottleneck
        assert!(!report.bottlenecks.is_empty());
        let has_slow_phase_bottleneck = report
            .bottlenecks
            .iter()
            .any(|b| b.kind == BottleneckKind::SlowPhase);
        assert!(has_slow_phase_bottleneck);
    }

    #[test]
    fn test_json_serialization() {
        let mut report = CompilationProfileReport::new();
        report.record_phase("Test", Duration::from_millis(100), 1024);
        report.finalize();

        let json = report.to_json().unwrap();
        let deserialized = CompilationProfileReport::from_json(json.as_str()).unwrap();

        assert_eq!(deserialized.phase_metrics.len(), 1);
        assert_eq!(deserialized.phase_metrics[0].phase_name, Text::from("Test"));
    }
}
