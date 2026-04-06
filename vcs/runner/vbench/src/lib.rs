//! VBench - VCS Benchmark Runner
//!
//! A comprehensive benchmarking tool for the Verum Compliance Suite (VCS).
//! Provides performance testing, baseline comparison, regression detection,
//! and detailed reporting.
//!
//! # Overview
//!
//! VBench is designed to measure and validate Verum's performance characteristics:
//!
//! - **CBGR Latency**: Target < 15ns per check
//! - **Compilation Speed**: Target > 50K LOC/sec
//! - **Runtime Performance**: Target 0.85-0.95x native C
//! - **Memory Overhead**: Target < 5%
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                         VBENCH ARCHITECTURE                      │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                   │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │
//! │  │   Runner    │  │  Compare    │  │   Report    │              │
//! │  │  (Execute)  │  │ (Baseline)  │  │ (Generate)  │              │
//! │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘              │
//! │         │                │                │                      │
//! │         └────────────────┼────────────────┘                      │
//! │                          │                                        │
//! │  ┌─────────────┐   ┌─────▼─────┐   ┌─────────────┐              │
//! │  │  Profiling  │   │  Metrics  │   │   History   │              │
//! │  │ (Flamegraph)│   │  (Stats)  │   │  (Trends)   │              │
//! │  └─────────────┘   └───────────┘   └─────────────┘              │
//! │         │                │                │                      │
//! │         └────────────────┼────────────────┘                      │
//! │                          │                                        │
//! │                    ┌─────▼─────┐                                  │
//! │                    │    CI     │                                  │
//! │                    │ (GitHub)  │                                  │
//! │                    └───────────┘                                  │
//! │                                                                   │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example Usage
//!
//! ```no_run
//! use vbench::{RunnerConfig, run_benchmarks, generate_report, ReportFormat};
//!
//! // Configure the benchmark runner
//! let config = RunnerConfig::default();
//!
//! // Run all benchmarks
//! let results = run_benchmarks(&config).unwrap();
//!
//! // Generate a report
//! let metadata = vbench::ReportMetadata::new("Verum Benchmarks", "1.0.0");
//! let report = vbench::BenchmarkReport::new(metadata, results, vec![], vec![]);
//! let output = generate_report(&report, ReportFormat::Console).unwrap();
//! println!("{}", output);
//! ```
//!
//! # Benchmark Categories
//!
//! - **Micro**: Individual operations (< 1ms)
//!   - CBGR checks, allocation, context lookup, async spawn
//! - **Macro**: Realistic workloads (< 1s)
//!   - Sorting, JSON parsing, crypto hashing, collection ops
//! - **Compilation**: Compiler performance
//!   - Lexer, parser, type inference, codegen
//! - **SMT**: Verification time
//!   - Refinement type checking, constraint solving
//! - **Memory**: Memory usage
//!   - Heap fragmentation, stack usage, reference overhead
//!
//! # Baseline Comparison
//!
//! VBench can compare Verum performance against:
//! - C (with -O3 optimization)
//! - Rust (with release mode)
//! - Go (with standard compilation)
//!
//! # Regression Detection
//!
//! Automatically detects performance regressions by:
//! - Comparing against historical baselines
//! - Statistical significance testing (Welch's t-test)
//! - Configurable regression thresholds
//!
//! # Historical Tracking
//!
//! Track performance over time:
//! - Store results in JSON history files
//! - Trend analysis with moving averages
//! - Anomaly detection for outliers
//!
//! # CI/CD Integration
//!
//! Seamless integration with CI pipelines:
//! - GitHub Actions annotations
//! - JUnit XML output
//! - Configurable pass/fail thresholds
//! - Automatic regression detection
//!
//! # Profiling Integration
//!
//! Integration with profiling tools:
//! - Flamegraph generation
//! - perf integration (Linux)
//! - samply support (cross-platform)
//! - CPU and memory metrics

// VCS benchmark infrastructure - suppress clippy warnings for test tooling
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]

pub mod benchmarks;
pub mod ci;
pub mod compare;
pub mod history;
pub mod metrics;
pub mod profiling;
pub mod report;
pub mod runner;
pub mod stats;

// Re-export main types from each module
pub use benchmarks::{
    CompilationSpeedResult, FatRef, MemoryOverheadReport, SmtConstraint, TARGET_CBGR_CHECK_NS,
    TARGET_COMPILATION_LOC_PER_SEC, TARGET_MEMORY_OVERHEAD_PERCENT, TARGET_RUNTIME_VS_C_MAX,
    TARGET_RUNTIME_VS_C_MIN, TARGET_TYPE_INFERENCE_MS_10K_LOC, ThinRef, TypeInferenceEnv,
    calculate_memory_overhead, measure_compilation_speed, run_all_compilation_benchmarks,
    run_all_macro_benchmarks, run_all_memory_benchmarks, run_all_micro_benchmarks,
    run_all_smt_benchmarks, run_allocation_benchmarks, run_async_benchmarks, run_cbgr_benchmarks,
    run_collection_benchmarks, run_compilation_benchmarks, run_compilation_speed_benchmarks,
    run_context_benchmarks, run_crypto_benchmarks, run_full_benchmark_suite, run_memory_benchmarks,
    run_parsing_benchmarks, run_runtime_benchmarks, run_smt_benchmarks, run_sorting_benchmarks,
    run_sync_benchmarks, run_type_inference_benchmarks,
};
pub use ci::{
    BenchmarkStatus, CiConfig, CiMessage, CiOutputFormat, CiProvider, CiResult, CiSummary,
    MessageLevel, Status, detect_ci_provider, format_ci_result, generate_default_ci_config,
    generate_github_summary, is_ci_environment, load_ci_config, save_ci_config, validate_ci,
};
pub use compare::{
    BaselineLanguage, BaselineManager, BaselineResult, BenchmarkHistory as CompareHistory,
    ComparisonAssessment, ComparisonResult, EffectSizeInterpretation, HistoricalDataPoint,
    RegressionConfig, RegressionResult, SignificanceResult, SignificanceTest, bootstrap_test,
    detect_regression, detect_regression_with_raw, mann_whitney_u_test, welch_t_test,
};
pub use history::{
    Anomaly, AnomalyDetector, AnomalyKind, Baseline, BenchmarkHistory, HistoricalPoint,
    HistoricalPointBuilder, HistoricalStats, HistoryStore, HistorySummary, StoreMetadata,
    TrendAnalysis, TrendSeverity, exponential_moving_average, simple_moving_average,
    weighted_moving_average,
};
pub use metrics::{
    BenchmarkCategory, BenchmarkResult, Measurement, MemorySnapshot, PerformanceTargets,
    Statistics, Throughput, Timer, black_box, measure_n, measure_once,
};
pub use profiling::{
    ColorScheme, CpuMetrics, FlamegraphConfig, MemoryMetrics, ProfiledResult, Profiler,
    ProfilingConfig, ProfilingSession, Sample, TrackedAllocator, generate_flamegraph_svg,
    run_profiled,
};
pub use report::{
    BenchmarkReport, ReportFormat, ReportMetadata, ReportSummary, generate_report, write_report,
};
pub use runner::{
    BaselineLanguage as RunnerBaselineLanguage,
    BenchmarkGroup,
    BenchmarkPhase,
    BenchmarkSpec,
    BenchmarkSummary,
    // New L4 specs support
    BenchmarkType,
    ExtendedStatistics,
    InProcessBenchmark,
    MemoryProfile,
    MemorySnapshot as RunnerMemorySnapshot,
    OutlierMethod,
    OutlierResult,
    PerformanceExpectation,
    ProgressReport,
    RunnerConfig,
    WarmupStrategy,
    discover_all_benchmarks,
    discover_benchmarks,
    discover_l4_specs,
    filter_by_baseline,
    filter_by_type,
    run_benchmarks,
    run_in_process,
    summarize_benchmarks,
};
pub use stats::{
    BootstrapComparisonResult, BootstrapResult, ChangeConfidence, ComparisonSummary,
    DescriptiveStats, DistributionAnalysis, DistributionType, EffectSize as StatsEffectSize,
    OutlierAnalysis, OutlierMethod as StatsOutlierMethod, RegressionAnalysis,
    SignificanceResult as StatsSignificanceResult, analyze_distribution, analyze_regression,
    bootstrap_ci, bootstrap_compare, detect_outliers_iqr, detect_outliers_mad,
    mann_whitney_u_test as stats_mann_whitney_u_test, paired_t_test, remove_outliers,
    welch_t_test as stats_welch_t_test,
};

// ============================================================================
// Convenience Types and Functions
// ============================================================================

/// Result type for vbench operations.
pub type Result<T> = anyhow::Result<T>;

/// Default performance targets from the VCS specification.
pub fn default_targets() -> PerformanceTargets {
    PerformanceTargets::default()
}

/// Quick benchmark for a single function.
///
/// # Example
///
/// ```
/// use vbench::quick_bench;
///
/// let result = quick_bench("my_function", 1000, || {
///     // Function to benchmark
///     let sum: u64 = (0..100).sum();
///     std::hint::black_box(sum);
/// });
///
/// println!("Mean: {:.2}ns", result.statistics.mean_ns);
/// ```
pub fn quick_bench<F>(name: &str, iterations: usize, f: F) -> BenchmarkResult
where
    F: Fn() + Send + Sync + 'static,
{
    InProcessBenchmark::new(name, f)
        .with_iterations(iterations)
        .run(100)
}

/// Create a benchmark group with default settings.
///
/// # Example
///
/// ```
/// use vbench::bench_group;
///
/// let results = bench_group("arithmetic")
///     .bench("add", || { let _ = std::hint::black_box(1 + 1); })
///     .bench("mul", || { let _ = std::hint::black_box(2 * 3); })
///     .run();
/// ```
pub fn bench_group(name: &str) -> BenchmarkGroup {
    BenchmarkGroup::new(name)
}

// ============================================================================
// Built-in Benchmarks (re-exported from benchmarks module)
// ============================================================================
// See benchmarks module for comprehensive benchmark suites:
// - run_cbgr_benchmarks()
// - run_allocation_benchmarks()
// - run_context_benchmarks()
// - run_sync_benchmarks()
// - run_async_benchmarks()
// - run_sorting_benchmarks()
// - run_parsing_benchmarks()
// - run_crypto_benchmarks()
// - run_collection_benchmarks()
// - run_compilation_benchmarks()
// - run_smt_benchmarks()
// - run_memory_benchmarks()
// - run_runtime_benchmarks()
// - run_all_micro_benchmarks()
// - run_all_macro_benchmarks()
// - run_all_compilation_benchmarks()
// - run_all_smt_benchmarks()
// - run_all_memory_benchmarks()
// - run_full_benchmark_suite()

// ============================================================================
// Configuration
// ============================================================================

/// Load configuration from a TOML file.
pub fn load_config(path: &std::path::Path) -> Result<RunnerConfig> {
    let content = std::fs::read_to_string(path)?;
    let config: RunnerConfig = toml::from_str(&content)?;
    Ok(config)
}

/// Save configuration to a TOML file.
pub fn save_config(config: &RunnerConfig, path: &std::path::Path) -> Result<()> {
    let content = toml::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quick_bench() {
        let result = quick_bench("test", 100, || {
            let _ = black_box(1 + 1);
        });

        assert_eq!(result.name, "test");
        // Count may be significantly less than 100 due to:
        // - Outlier removal (IQR filtering)
        // - Timing variations on CI/slow machines
        // Just verify we got some samples
        assert!(
            result.statistics.count >= 50 && result.statistics.count <= 100,
            "Expected count between 50-100, got {}",
            result.statistics.count
        );
    }

    #[test]
    fn test_bench_group() {
        let results = bench_group("test_group")
            .warmup(10)
            .iterations(50)
            .bench("op1", || {
                let _ = black_box(1);
            })
            .bench("op2", || {
                let _ = black_box(2);
            })
            .run();

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_cbgr_benchmarks() {
        let results = run_cbgr_benchmarks();
        assert!(!results.is_empty());

        // Check that CBGR tier0 benchmarks have thresholds
        let tier0_results: Vec<_> = results
            .iter()
            .filter(|r| r.name.contains("tier0"))
            .collect();
        assert!(!tier0_results.is_empty());
    }

    #[test]
    fn test_allocation_benchmarks() {
        let results = run_allocation_benchmarks();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_all_micro_benchmarks() {
        let results = run_all_micro_benchmarks();
        assert!(results.len() > 10); // Should have many micro benchmarks
    }

    #[test]
    fn test_report_generation() {
        let results = vec![quick_bench("test", 10, || {
            let _ = black_box(1);
        })];
        let metadata = ReportMetadata::new("Test", "1.0.0");
        let report = BenchmarkReport::new(metadata, results, vec![], vec![]);

        // Test all formats
        for format in [
            ReportFormat::Console,
            ReportFormat::Json,
            ReportFormat::Html,
            ReportFormat::Csv,
            ReportFormat::Markdown,
        ] {
            let output = generate_report(&report, format);
            assert!(output.is_ok(), "Failed to generate {:?} report", format);
        }
    }

    #[test]
    fn test_ci_validation() {
        let results = vec![quick_bench("test", 10, || {
            let _ = black_box(1);
        })];
        let metadata = ReportMetadata::new("Test", "1.0.0");
        let report = BenchmarkReport::new(metadata, results, vec![], vec![]);

        let ci_config = CiConfig::default();
        let ci_result = validate_ci(&report, &ci_config);

        assert!(ci_result.summary.total == 1);
    }

    #[test]
    fn test_history_store() {
        let mut store = HistoryStore::new();
        let results = vec![quick_bench("test", 10, || {
            let _ = black_box(1);
        })];

        store.add_results(&results, "1.0.0", Some("abc123"));

        assert!(store.get("test").is_some());
        assert_eq!(store.get("test").unwrap().points.len(), 1);
    }

    #[test]
    fn test_profiling_session() {
        let config = ProfilingConfig {
            profiler: Profiler::None,
            ..Default::default()
        };

        let result = run_profiled("test", &config, 10, || {
            let _ = std::hint::black_box(1 + 1);
        });

        assert!(result.is_ok());
    }

    #[test]
    fn test_thin_ref_size() {
        // ThinRef should be 16 bytes: ptr(8) + generation(4) + epoch_caps(4)
        assert_eq!(std::mem::size_of::<ThinRef<u64>>(), 16);
    }

    #[test]
    fn test_fat_ref_size() {
        // FatRef should be 24 bytes: ptr(8) + generation(4) + epoch_caps(4) + len(8)
        assert_eq!(std::mem::size_of::<FatRef<u64>>(), 24);
    }
}
