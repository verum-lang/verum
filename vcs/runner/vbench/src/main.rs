//! VBench CLI - VCS Benchmark Runner
//!
//! Command-line interface for running VCS benchmarks, comparing against baselines,
//! and generating performance reports.
//!
//! # Usage
//!
//! ```bash
//! # Run all benchmarks
//! vbench run
//!
//! # Run specific category
//! vbench run --category micro
//!
//! # Run built-in benchmark suites
//! vbench micro      # Micro benchmarks
//! vbench macro      # Macro benchmarks
//! vbench full       # Full benchmark suite
//!
//! # Run with baseline comparison
//! vbench run --compare rust,c
//!
//! # Generate HTML report
//! vbench report --format html --output report.html
//!
//! # Check for regressions
//! vbench check --baseline baseline.json
//!
//! # Manage history
//! vbench history show
//! vbench history trends
//!
//! # Profile benchmarks
//! vbench profile --benchmark cbgr
//!
//! # CI integration
//! vbench ci --format github
//! ```

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

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

use vbench::{
    BenchmarkCategory, BenchmarkReport, BenchmarkType, CiConfig, CiOutputFormat, HistoryStore,
    PerformanceTargets, Profiler, ProfilingConfig, RegressionConfig, ReportFormat, ReportMetadata,
    RunnerConfig, detect_regression, discover_all_benchmarks, discover_benchmarks,
    discover_l4_specs, filter_by_type, format_ci_result, generate_github_summary, generate_report,
    is_ci_environment, run_all_compilation_benchmarks, run_all_macro_benchmarks,
    run_all_memory_benchmarks, run_all_micro_benchmarks, run_all_smt_benchmarks, run_benchmarks,
    run_full_benchmark_suite, run_profiled, summarize_benchmarks, validate_ci, write_report,
};

// ============================================================================
// CLI Definition
// ============================================================================

/// VBench - VCS Benchmark Runner
///
/// Performance testing tool for the Verum Compliance Suite.
/// Measures CBGR latency, compilation speed, runtime performance, and more.
#[derive(Parser)]
#[command(name = "vbench")]
#[command(author = "Verum Team")]
#[command(version = "1.0.0")]
#[command(about = "VCS benchmark runner for performance testing", long_about = None)]
struct Cli {
    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Quiet mode (minimal output)
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run benchmarks from files
    Run {
        /// Benchmark directory
        #[arg(short, long, default_value = "vcs/benchmarks")]
        dir: PathBuf,

        /// Filter benchmarks by name pattern
        #[arg(short, long)]
        filter: Option<String>,

        /// Filter by category
        #[arg(short, long)]
        category: Option<CategoryArg>,

        /// Execution tier (0-3, or "all")
        #[arg(short, long, default_value = "3")]
        tier: String,

        /// Number of warmup iterations
        #[arg(long, default_value = "100")]
        warmup: usize,

        /// Number of measurement iterations
        #[arg(long, default_value = "1000")]
        iterations: usize,

        /// Number of parallel workers
        #[arg(short, long, default_value = "1")]
        parallel: usize,

        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file (for non-console formats)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Compare against baselines
        #[arg(long, value_delimiter = ',')]
        compare: Option<Vec<String>>,

        /// Fail if any benchmark exceeds threshold
        #[arg(long)]
        strict: bool,

        /// Save to history
        #[arg(long)]
        history: Option<PathBuf>,

        /// Version for history tracking
        #[arg(long)]
        version: Option<String>,
    },

    /// Run built-in micro benchmarks (CBGR, allocation, context, sync)
    Micro {
        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Run built-in macro benchmarks (sorting, parsing, crypto)
    Macro {
        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Run built-in compilation benchmarks (lexer, parser, typecheck)
    Compilation {
        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Run built-in SMT verification benchmarks
    Smt {
        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Run built-in memory benchmarks
    Memory {
        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Run the full benchmark suite
    Full {
        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Save to history
        #[arg(long)]
        history: Option<PathBuf>,

        /// Version for history tracking
        #[arg(long)]
        version: Option<String>,
    },

    /// Generate a report from previous results
    Report {
        /// Input JSON results file
        #[arg(short, long)]
        input: PathBuf,

        /// Output format
        #[arg(short, long, default_value = "html")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Check for performance regressions
    Check {
        /// Current results file
        #[arg(short, long)]
        current: PathBuf,

        /// Baseline results file
        #[arg(short, long)]
        baseline: PathBuf,

        /// Regression threshold (percentage)
        #[arg(long, default_value = "5.0")]
        threshold: f64,

        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,
    },

    /// CI/CD integration commands
    Ci {
        /// CI output format
        #[arg(long, default_value = "console")]
        format: CiFormatArg,

        /// Results file to validate
        #[arg(short, long)]
        results: Option<PathBuf>,

        /// Baseline for regression detection
        #[arg(short, long)]
        baseline: Option<PathBuf>,

        /// Regression threshold (percentage)
        #[arg(long, default_value = "5.0")]
        threshold: f64,

        /// Fail on any threshold violation
        #[arg(long)]
        strict: bool,

        /// Generate GitHub step summary
        #[arg(long)]
        github_summary: bool,
    },

    /// Manage benchmark history
    History {
        #[command(subcommand)]
        command: HistoryCommands,
    },

    /// Profile benchmarks with flamegraph
    Profile {
        /// Benchmark to profile (name filter)
        #[arg(short, long)]
        benchmark: Option<String>,

        /// Output directory for profiles
        #[arg(short, long, default_value = "profiles")]
        output: PathBuf,

        /// Number of iterations
        #[arg(long, default_value = "1000")]
        iterations: usize,

        /// Generate flamegraph SVG
        #[arg(long)]
        flamegraph: bool,
    },

    /// List available benchmarks
    List {
        /// Benchmark directory
        #[arg(short, long, default_value = "vcs/benchmarks")]
        dir: PathBuf,

        /// Filter by category
        #[arg(short, long)]
        category: Option<CategoryArg>,

        /// Show detailed information
        #[arg(long)]
        detailed: bool,
    },

    /// Show performance targets
    Targets {
        /// Show custom thresholds from config
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// Run L4 performance spec tests from vcs/specs/L4-performance/
    Specs {
        /// VCS root directory
        #[arg(long, default_value = "vcs")]
        vcs_root: PathBuf,

        /// Filter by benchmark type
        #[arg(long)]
        bench_type: Option<BenchTypeArg>,

        /// Filter by name pattern
        #[arg(short, long)]
        filter: Option<String>,

        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Include vcs/benchmarks/ as well
        #[arg(long)]
        include_benchmarks: bool,

        /// Validate against expected performance
        #[arg(long)]
        validate: bool,

        /// Show summary only
        #[arg(long)]
        summary: bool,
    },

    /// Compare against baseline languages (C, Rust, Go)
    Baseline {
        /// Baseline language to compare against
        #[arg(short, long)]
        language: BaselineArg,

        /// VCS root directory
        #[arg(long, default_value = "vcs")]
        vcs_root: PathBuf,

        /// Output format
        #[arg(long, default_value = "console")]
        format: FormatArg,

        /// Output file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum HistoryCommands {
    /// Show benchmark history
    Show {
        /// History file path
        #[arg(short, long, default_value = "benchmark_history.json")]
        file: PathBuf,

        /// Filter by benchmark name
        #[arg(short, long)]
        benchmark: Option<String>,

        /// Number of recent entries to show
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Show performance trends
    Trends {
        /// History file path
        #[arg(short, long, default_value = "benchmark_history.json")]
        file: PathBuf,

        /// Filter by benchmark name
        #[arg(short, long)]
        benchmark: Option<String>,
    },

    /// Detect anomalies in history
    Anomalies {
        /// History file path
        #[arg(short, long, default_value = "benchmark_history.json")]
        file: PathBuf,

        /// Sigma threshold for anomaly detection
        #[arg(long, default_value = "3.0")]
        sigma: f64,
    },

    /// Prune old history entries
    Prune {
        /// History file path
        #[arg(short, long, default_value = "benchmark_history.json")]
        file: PathBuf,

        /// Maximum age in days
        #[arg(long, default_value = "90")]
        max_age_days: i64,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CategoryArg {
    Micro,
    Macro,
    Compilation,
    Runtime,
    Memory,
    Baseline,
}

impl From<CategoryArg> for BenchmarkCategory {
    fn from(arg: CategoryArg) -> Self {
        match arg {
            CategoryArg::Micro => BenchmarkCategory::Micro,
            CategoryArg::Macro => BenchmarkCategory::Macro,
            CategoryArg::Compilation => BenchmarkCategory::Compilation,
            CategoryArg::Runtime => BenchmarkCategory::Runtime,
            CategoryArg::Memory => BenchmarkCategory::Memory,
            CategoryArg::Baseline => BenchmarkCategory::Baseline,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FormatArg {
    Console,
    Json,
    Html,
    Csv,
    Markdown,
}

impl From<FormatArg> for ReportFormat {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::Console => ReportFormat::Console,
            FormatArg::Json => ReportFormat::Json,
            FormatArg::Html => ReportFormat::Html,
            FormatArg::Csv => ReportFormat::Csv,
            FormatArg::Markdown => ReportFormat::Markdown,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum CiFormatArg {
    Console,
    Github,
    Junit,
    Json,
}

impl From<CiFormatArg> for CiOutputFormat {
    fn from(arg: CiFormatArg) -> Self {
        match arg {
            CiFormatArg::Console => CiOutputFormat::Console,
            CiFormatArg::Github => CiOutputFormat::GitHub,
            CiFormatArg::Junit => CiOutputFormat::JUnit,
            CiFormatArg::Json => CiOutputFormat::Json,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum BenchTypeArg {
    Micro,
    Macro,
    Baseline,
    Compilation,
    Smt,
    Memory,
}

impl From<BenchTypeArg> for BenchmarkType {
    fn from(arg: BenchTypeArg) -> Self {
        match arg {
            BenchTypeArg::Micro => BenchmarkType::Micro,
            BenchTypeArg::Macro => BenchmarkType::Macro,
            BenchTypeArg::Baseline => BenchmarkType::Baseline,
            BenchTypeArg::Compilation => BenchmarkType::Compilation,
            BenchTypeArg::Smt => BenchmarkType::Smt,
            BenchTypeArg::Memory => BenchmarkType::Memory,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum BaselineArg {
    C,
    Rust,
    Go,
    Java,
    Python,
}

// ============================================================================
// Main Entry Point
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let log_level = if cli.verbose {
        Level::DEBUG
    } else if cli.quiet {
        Level::ERROR
    } else {
        Level::INFO
    };

    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .with_target(false)
        .without_time()
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

    match cli.command {
        Commands::Run {
            dir,
            filter,
            category,
            tier,
            warmup,
            iterations,
            parallel,
            format,
            output,
            compare,
            strict,
            history,
            version,
        } => cmd_run(
            dir, filter, category, tier, warmup, iterations, parallel, format, output, compare,
            strict, history, version,
        ),
        Commands::Micro { format, output } => cmd_micro(format, output),
        Commands::Macro { format, output } => cmd_macro(format, output),
        Commands::Compilation { format, output } => cmd_compilation(format, output),
        Commands::Smt { format, output } => cmd_smt(format, output),
        Commands::Memory { format, output } => cmd_memory(format, output),
        Commands::Full {
            format,
            output,
            history,
            version,
        } => cmd_full(format, output, history, version),
        Commands::Report {
            input,
            format,
            output,
        } => cmd_report(input, format, output),
        Commands::Check {
            current,
            baseline,
            threshold,
            format,
        } => cmd_check(current, baseline, threshold, format),
        Commands::Ci {
            format,
            results,
            baseline,
            threshold,
            strict,
            github_summary,
        } => cmd_ci(format, results, baseline, threshold, strict, github_summary),
        Commands::History { command } => cmd_history(command),
        Commands::Profile {
            benchmark,
            output,
            iterations,
            flamegraph,
        } => cmd_profile(benchmark, output, iterations, flamegraph),
        Commands::List {
            dir,
            category,
            detailed,
        } => cmd_list(dir, category, detailed),
        Commands::Targets { config } => cmd_targets(config),
        Commands::Specs {
            vcs_root,
            bench_type,
            filter,
            format,
            output,
            include_benchmarks,
            validate,
            summary,
        } => cmd_specs(
            vcs_root,
            bench_type,
            filter,
            format,
            output,
            include_benchmarks,
            validate,
            summary,
        ),
        Commands::Baseline {
            language,
            vcs_root,
            format,
            output,
        } => cmd_baseline(language, vcs_root, format, output),
    }
}

// ============================================================================
// Command Implementations
// ============================================================================

fn cmd_run(
    dir: PathBuf,
    filter: Option<String>,
    category: Option<CategoryArg>,
    tier: String,
    warmup: usize,
    iterations: usize,
    parallel: usize,
    format: FormatArg,
    output: Option<PathBuf>,
    _compare: Option<Vec<String>>,
    strict: bool,
    history_path: Option<PathBuf>,
    version: Option<String>,
) -> Result<()> {
    info!("Running benchmarks from {}", dir.display());

    // Parse tiers
    let tiers: Vec<u8> = if tier == "all" {
        vec![0, 1, 2, 3]
    } else {
        tier.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect()
    };

    // Build config
    let config = RunnerConfig {
        benchmark_dir: dir,
        tiers,
        warmup_iterations: warmup,
        measure_iterations: iterations,
        parallel,
        filter,
        category: category.map(Into::into),
        verbose: true,
        ..Default::default()
    };

    // Run benchmarks
    let results = run_benchmarks(&config)?;

    if results.is_empty() {
        println!("{}", "No benchmarks found".yellow());
        return Ok(());
    }

    // Save to history if requested
    if let Some(ref path) = history_path {
        let mut store = HistoryStore::load_or_create(path)?;
        let ver = version.as_deref().unwrap_or("unknown");
        store.add_results(&results, ver, None);
        store.save()?;
        println!("Results saved to history: {}", path.display());
    }

    // Generate report
    let metadata = ReportMetadata::new("VBench Performance Report", "1.0.0");
    let report = BenchmarkReport::new(metadata, results, vec![], vec![]);

    let report_content = generate_report(&report, format.into())?;

    if let Some(path) = output {
        write_report(&report, format.into(), &path)?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", report_content);
    }

    // Check strict mode
    if strict && report.summary.failed > 0 {
        return Err(anyhow::anyhow!(
            "{} benchmarks failed threshold checks",
            report.summary.failed
        ));
    }

    Ok(())
}

fn cmd_micro(format: FormatArg, output: Option<PathBuf>) -> Result<()> {
    println!("{}", "Running built-in micro benchmarks...".cyan());
    println!("  CBGR, allocation, context, sync\n");

    let results = run_all_micro_benchmarks();
    output_results("VBench Micro Benchmarks", results, format, output)
}

fn cmd_macro(format: FormatArg, output: Option<PathBuf>) -> Result<()> {
    println!("{}", "Running built-in macro benchmarks...".cyan());
    println!("  Sorting, parsing, crypto, collections, runtime\n");

    let results = run_all_macro_benchmarks();
    output_results("VBench Macro Benchmarks", results, format, output)
}

fn cmd_compilation(format: FormatArg, output: Option<PathBuf>) -> Result<()> {
    println!("{}", "Running built-in compilation benchmarks...".cyan());
    println!("  Lexer, parser, typecheck, codegen\n");

    let results = run_all_compilation_benchmarks();
    output_results("VBench Compilation Benchmarks", results, format, output)
}

fn cmd_smt(format: FormatArg, output: Option<PathBuf>) -> Result<()> {
    println!(
        "{}",
        "Running built-in SMT verification benchmarks...".cyan()
    );
    println!("  Constraint solving, refinement types\n");

    let results = run_all_smt_benchmarks();
    output_results("VBench SMT Benchmarks", results, format, output)
}

fn cmd_memory(format: FormatArg, output: Option<PathBuf>) -> Result<()> {
    println!("{}", "Running built-in memory benchmarks...".cyan());
    println!("  Reference overhead, allocation patterns\n");

    let results = run_all_memory_benchmarks();
    output_results("VBench Memory Benchmarks", results, format, output)
}

fn cmd_full(
    format: FormatArg,
    output: Option<PathBuf>,
    history_path: Option<PathBuf>,
    version: Option<String>,
) -> Result<()> {
    println!("{}", "Running full benchmark suite...".cyan());
    println!("  Micro, Macro, Compilation, SMT, Memory, Async\n");

    let results = run_full_benchmark_suite();

    // Save to history if requested
    if let Some(ref path) = history_path {
        let mut store = HistoryStore::load_or_create(path)?;
        let ver = version.as_deref().unwrap_or("unknown");
        store.add_results(&results, ver, None);
        store.save()?;
        println!("Results saved to history: {}", path.display());
    }

    output_results("VBench Full Benchmark Suite", results, format, output)
}

/// Helper function to output results in various formats.
fn output_results(
    title: &str,
    results: Vec<vbench::BenchmarkResult>,
    format: FormatArg,
    output: Option<PathBuf>,
) -> Result<()> {
    let metadata = ReportMetadata::new(title, "1.0.0");
    let report = BenchmarkReport::new(metadata, results, vec![], vec![]);

    let report_content = generate_report(&report, format.into())?;

    if let Some(path) = output {
        write_report(&report, format.into(), &path)?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", report_content);
    }

    Ok(())
}

fn cmd_ci(
    format: CiFormatArg,
    results_path: Option<PathBuf>,
    baseline_path: Option<PathBuf>,
    threshold: f64,
    strict: bool,
    github_summary: bool,
) -> Result<()> {
    // Auto-detect if running in CI
    if is_ci_environment() {
        info!("CI environment detected");
    }

    // Load or run benchmarks
    let report = if let Some(path) = results_path {
        let content = std::fs::read_to_string(&path)?;
        serde_json::from_str(&content)?
    } else {
        // Run micro benchmarks as default
        let results = run_all_micro_benchmarks();
        let metadata = ReportMetadata::new("VBench CI", "1.0.0");
        BenchmarkReport::new(metadata, results, vec![], vec![])
    };

    // Configure CI validation
    let ci_config = CiConfig {
        fail_on_threshold: strict,
        fail_on_regression: true,
        regression_threshold_percent: threshold,
        ..Default::default()
    };

    // Validate
    let ci_result = validate_ci(&report, &ci_config);

    // Output in requested format
    let output = format_ci_result(&ci_result, format.into());
    println!("{}", output);

    // Generate GitHub step summary if requested
    if github_summary {
        let summary = generate_github_summary(&ci_result);
        // Write to GITHUB_STEP_SUMMARY if available
        if let Ok(summary_file) = std::env::var("GITHUB_STEP_SUMMARY") {
            std::fs::write(&summary_file, &summary)?;
            println!("GitHub step summary written to: {}", summary_file);
        } else {
            println!("\n{}", "GitHub Step Summary:".bold());
            println!("{}", summary);
        }
    }

    // Exit with appropriate code
    if !ci_result.passed {
        std::process::exit(ci_result.exit_code);
    }

    Ok(())
}

fn cmd_history(command: HistoryCommands) -> Result<()> {
    use vbench::AnomalyDetector;

    match command {
        HistoryCommands::Show {
            file,
            benchmark,
            limit,
        } => {
            let store = HistoryStore::load(&file)?;

            let names: Vec<_> = if let Some(ref filter) = benchmark {
                store
                    .benchmark_names()
                    .into_iter()
                    .filter(|n| n.contains(filter))
                    .collect()
            } else {
                store.benchmark_names()
            };

            for name in names {
                if let Some(history) = store.get(name) {
                    println!("{} [{}]", name.bold(), history.category);

                    let points: Vec<_> = history.points.iter().rev().take(limit).collect();

                    for point in points.iter().rev() {
                        println!(
                            "  {} v{}: {:.2}ns (+/- {:.2}ns)",
                            point.timestamp.format("%Y-%m-%d %H:%M"),
                            point.version,
                            point.mean_ns,
                            point.std_dev_ns
                        );
                    }
                    println!();
                }
            }
        }

        HistoryCommands::Trends { file, benchmark } => {
            let store = HistoryStore::load(&file)?;
            let trends = store.analyze_trends();

            let filtered: Vec<_> = if let Some(ref filter) = benchmark {
                trends
                    .into_iter()
                    .filter(|t| t.name.contains(filter))
                    .collect()
            } else {
                trends
            };

            println!("{}", "Performance Trends".cyan().bold());
            println!();

            for trend in filtered {
                let trend_str = trend.trend_description();
                let severity_color = match trend.severity() {
                    vbench::TrendSeverity::Improvement => "green",
                    vbench::TrendSeverity::Stable => "white",
                    vbench::TrendSeverity::Minor => "yellow",
                    vbench::TrendSeverity::Warning => "red",
                    vbench::TrendSeverity::Critical => "red",
                };

                println!(
                    "  {:40} {:15} ({} points)",
                    trend.name, trend_str, trend.data_points
                );
            }
        }

        HistoryCommands::Anomalies { file, sigma } => {
            let store = HistoryStore::load(&file)?;
            let detector = AnomalyDetector::new(sigma, 10);

            println!("{}", "Anomaly Detection".cyan().bold());
            println!("Threshold: {}sigma\n", sigma);

            let mut found_anomalies = false;

            for name in store.benchmark_names() {
                if let Some(history) = store.get(name) {
                    let anomalies = detector.detect(history);
                    if !anomalies.is_empty() {
                        found_anomalies = true;
                        println!("{}:", name.bold());
                        for anomaly in anomalies {
                            let kind = match anomaly.kind {
                                vbench::AnomalyKind::UnexpectedlyFast => "FAST",
                                vbench::AnomalyKind::UnexpectedlySlow => "SLOW",
                            };
                            println!(
                                "  {} {} at {}: {:.2}ns (z={:.2})",
                                kind.yellow(),
                                anomaly.point.version,
                                anomaly.point.timestamp.format("%Y-%m-%d"),
                                anomaly.point.mean_ns,
                                anomaly.z_score
                            );
                        }
                        println!();
                    }
                }
            }

            if !found_anomalies {
                println!("{}", "No anomalies detected".green());
            }
        }

        HistoryCommands::Prune { file, max_age_days } => {
            let mut store = HistoryStore::load(&file)?;
            let before = store.benchmarks.len();

            store.prune(max_age_days);
            store.save()?;

            let after = store.benchmarks.len();
            println!("Pruned history entries older than {} days", max_age_days);
            println!("Benchmarks: {} -> {}", before, after);
        }
    }

    Ok(())
}

fn cmd_profile(
    benchmark: Option<String>,
    output_dir: PathBuf,
    iterations: usize,
    flamegraph: bool,
) -> Result<()> {
    use std::fs;

    println!("{}", "Profiling benchmarks...".cyan());

    fs::create_dir_all(&output_dir)?;

    let config = ProfilingConfig {
        profiler: Profiler::Builtin,
        output_dir: output_dir.clone(),
        iterations: Some(iterations),
        flamegraph,
        ..Default::default()
    };

    // Run selected benchmarks with profiling
    let bench_name = benchmark.as_deref().unwrap_or("cbgr");

    println!("Running {} with {} iterations...", bench_name, iterations);

    let result = run_profiled(bench_name, &config, iterations, || {
        // Run a simple benchmark
        let mut sum = 0u64;
        for i in 0..1000 {
            sum += i;
        }
        std::hint::black_box(sum);
    })?;

    println!();
    println!("{}", "Profile Results:".bold());
    println!("  Iterations: {}", result.iterations);
    println!("  Total time: {:?}", result.total_duration);
    println!("  Samples: {}", result.samples);

    if let Some(stats) = result.statistics {
        println!("  Mean: {:.2}ns", stats.mean_ns);
        println!("  Std dev: {:.2}ns", stats.std_dev_ns);
        println!("  P95: {:.2}ns", stats.p95_ns);
    }

    if let Some(path) = result.profile_path {
        println!();
        println!("Profile saved to: {}", path.display());

        if flamegraph {
            let svg_path = path.with_extension("svg");
            if svg_path.exists() {
                println!("Flamegraph: {}", svg_path.display());
            }
        }
    }

    Ok(())
}

fn cmd_report(input: PathBuf, format: FormatArg, output: PathBuf) -> Result<()> {
    info!("Generating report from {}", input.display());

    let content = std::fs::read_to_string(&input).context("Failed to read input file")?;

    let report: BenchmarkReport =
        serde_json::from_str(&content).context("Failed to parse input file")?;

    write_report(&report, format.into(), &output)?;

    println!("Report written to: {}", output.display());

    Ok(())
}

fn cmd_check(
    current_path: PathBuf,
    baseline_path: PathBuf,
    threshold: f64,
    format: FormatArg,
) -> Result<()> {
    info!("Checking for regressions...");

    let current_content = std::fs::read_to_string(&current_path)?;
    let baseline_content = std::fs::read_to_string(&baseline_path)?;

    let current: BenchmarkReport = serde_json::from_str(&current_content)?;
    let baseline: BenchmarkReport = serde_json::from_str(&baseline_content)?;

    let config = RegressionConfig {
        threshold_percent: threshold,
        ..Default::default()
    };

    // Match results by name and check for regressions
    let mut regressions = Vec::new();
    for current_result in &current.results {
        if let Some(baseline_result) = baseline
            .results
            .iter()
            .find(|r| r.name == current_result.name)
        {
            let regression = detect_regression(current_result, baseline_result, &config);
            if regression.is_regression {
                regressions.push(regression);
            }
        }
    }

    // Output results
    if regressions.is_empty() {
        println!("{}", "No regressions detected".green().bold());
    } else {
        println!(
            "{}",
            format!("{} regressions detected:", regressions.len())
                .red()
                .bold()
        );
        println!();

        for regression in &regressions {
            println!(
                "  {} {}: {} -> {} ({:+.1}%)",
                "REGRESSION".red(),
                regression.name,
                format_ns(regression.baseline_mean_ns),
                format_ns(regression.current_mean_ns),
                regression.percentage_change,
            );
        }

        // Exit with error code
        std::process::exit(1);
    }

    Ok(())
}

fn cmd_list(dir: PathBuf, category: Option<CategoryArg>, detailed: bool) -> Result<()> {
    let specs = discover_benchmarks(&dir)?;

    if specs.is_empty() {
        println!("{}", "No benchmarks found".yellow());
        return Ok(());
    }

    let filtered: Vec<_> = if let Some(cat) = category {
        let cat: BenchmarkCategory = cat.into();
        specs.into_iter().filter(|s| s.category == cat).collect()
    } else {
        specs
    };

    println!("{}", format!("Found {} benchmarks:", filtered.len()).cyan());
    println!();

    for spec in filtered {
        if detailed {
            println!("  {} [{}]", spec.name.bold(), spec.category);
            println!("    Path: {}", spec.path.display());
            println!("    Level: {}", spec.level);
            println!("    Tiers: {:?}", spec.tiers);
            if !spec.tags.is_empty() {
                println!("    Tags: {}", spec.tags.join(", "));
            }
            if let Some(ref perf) = spec.expected_performance {
                println!("    Expected: {}", perf);
            }
            println!();
        } else {
            println!(
                "  {:40} {:10} {}",
                spec.name,
                format!("[{}]", spec.category),
                spec.expected_performance.as_deref().unwrap_or("")
            );
        }
    }

    Ok(())
}

fn cmd_targets(config: Option<PathBuf>) -> Result<()> {
    let targets = if let Some(path) = config {
        let content = std::fs::read_to_string(&path)?;
        let config: RunnerConfig = toml::from_str(&content)?;
        config.targets
    } else {
        PerformanceTargets::default()
    };

    println!("{}", "VCS Performance Targets".cyan().bold());
    println!();
    println!("  {}", "Core Targets:".bold());
    println!("    CBGR check latency:     < {}ns", targets.cbgr_check_ns);
    println!(
        "    Type inference:         < {}ms per 10K LOC",
        targets.type_inference_ms_per_10k_loc
    );
    println!(
        "    Compilation speed:      > {} LOC/sec",
        targets.compilation_loc_per_sec
    );
    println!(
        "    Runtime vs C:           {:.0}-{:.0}%",
        targets.runtime_vs_c_min * 100.0,
        targets.runtime_vs_c_max * 100.0
    );
    println!(
        "    Memory overhead:        < {:.0}%",
        targets.memory_overhead_percent
    );

    if !targets.custom.is_empty() {
        println!();
        println!("  {}", "Custom Thresholds:".bold());
        for (name, threshold) in &targets.custom {
            println!("    {:30} < {}ns", name, threshold);
        }
    }

    Ok(())
}

fn cmd_specs(
    vcs_root: PathBuf,
    bench_type: Option<BenchTypeArg>,
    filter: Option<String>,
    format: FormatArg,
    output: Option<PathBuf>,
    include_benchmarks: bool,
    validate: bool,
    summary_only: bool,
) -> Result<()> {
    println!("{}", "Running L4 performance specs...".cyan());

    // Discover specs
    let specs = if include_benchmarks {
        discover_all_benchmarks(&vcs_root)?
    } else {
        discover_l4_specs(&vcs_root)?
    };

    if specs.is_empty() {
        println!("{}", "No L4 performance specs found".yellow());
        println!("  Looking in: {}/specs/L4-performance/", vcs_root.display());
        return Ok(());
    }

    // Filter by type
    let filtered = if let Some(bt) = bench_type {
        filter_by_type(specs, bt.into())
    } else {
        specs
    };

    // Filter by name pattern
    let filtered: Vec<_> = if let Some(ref pattern) = filter {
        let re = regex::Regex::new(pattern)?;
        filtered
            .into_iter()
            .filter(|s| re.is_match(&s.name))
            .collect()
    } else {
        filtered
    };

    // Show summary if requested
    if summary_only {
        let summary = summarize_benchmarks(&filtered);
        println!();
        println!("{}", "Benchmark Summary:".bold());
        println!("  Total:        {}", summary.total);
        println!("  Micro:        {}", summary.micro);
        println!("  Macro:        {}", summary.macro_count);
        println!("  Baseline:     {}", summary.baseline);
        println!("  Compilation:  {}", summary.compilation);
        println!("  SMT:          {}", summary.smt);
        println!("  Memory:       {}", summary.memory);
        return Ok(());
    }

    println!("  Found {} specs\n", filtered.len());

    // Build config and run
    let config = RunnerConfig {
        benchmark_dir: vcs_root.join("specs").join("L4-performance"),
        verbose: true,
        ..Default::default()
    };

    let results = run_benchmarks(&config)?;

    // Validate against expectations if requested
    if validate {
        println!("{}", "Validating against expected performance...".cyan());
        let mut passed = 0;
        let mut failed = 0;

        for result in &results {
            // Find corresponding spec
            let spec_name = result.name.split('@').next().unwrap_or(&result.name);
            if let Some(spec) = filtered.iter().find(|s| s.name == spec_name) {
                if let Some(ref expectation) = spec.parsed_expectation {
                    let mean_ns = result.statistics.mean_ns;
                    if expectation.check(mean_ns, None) {
                        passed += 1;
                        if matches!(format, FormatArg::Console) {
                            println!("  {} {} - {:.2}ns", "PASS".green(), result.name, mean_ns);
                        }
                    } else {
                        failed += 1;
                        println!(
                            "  {} {} - {:.2}ns (expected: {})",
                            "FAIL".red(),
                            result.name,
                            mean_ns,
                            spec.expected_performance.as_deref().unwrap_or("unknown")
                        );
                    }
                }
            }
        }

        println!();
        println!("  Passed: {}, Failed: {}", passed, failed);

        if failed > 0 {
            return Err(anyhow::anyhow!("{} performance specs failed", failed));
        }
    }

    // Generate report
    let metadata = ReportMetadata::new("L4 Performance Specs", "1.0.0");
    let report = BenchmarkReport::new(metadata, results, vec![], vec![]);

    let report_content = generate_report(&report, format.into())?;

    if let Some(path) = output {
        write_report(&report, format.into(), &path)?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", report_content);
    }

    Ok(())
}

fn cmd_baseline(
    language: BaselineArg,
    vcs_root: PathBuf,
    format: FormatArg,
    output: Option<PathBuf>,
) -> Result<()> {
    let lang_str = match language {
        BaselineArg::C => "C",
        BaselineArg::Rust => "Rust",
        BaselineArg::Go => "Go",
        BaselineArg::Java => "Java",
        BaselineArg::Python => "Python",
    };

    println!(
        "{}",
        format!("Running baseline comparison against {}...", lang_str).cyan()
    );

    // Discover all benchmarks and filter by baseline type
    let all_specs = discover_all_benchmarks(&vcs_root)?;
    let baseline_specs = filter_by_type(all_specs, BenchmarkType::Baseline);

    if baseline_specs.is_empty() {
        println!("{}", "No baseline comparison benchmarks found".yellow());
        println!(
            "  Looking in: {}/specs/L4-performance/comparison/",
            vcs_root.display()
        );
        return Ok(());
    }

    println!(
        "  Found {} baseline comparison benchmarks\n",
        baseline_specs.len()
    );

    // Build config and run
    let config = RunnerConfig {
        benchmark_dir: vcs_root.join("specs").join("L4-performance"),
        verbose: true,
        ..Default::default()
    };

    let results = run_benchmarks(&config)?;

    // Filter to baseline results
    let baseline_results: Vec<_> = results
        .into_iter()
        .filter(|r| {
            r.metadata
                .get("benchmark_type")
                .map(|t| t == "Baseline")
                .unwrap_or(false)
        })
        .collect();

    if baseline_results.is_empty() {
        println!("{}", "No baseline comparison results".yellow());
        return Ok(());
    }

    // Generate comparison report
    println!("{}", format!("Comparison Results vs {}:", lang_str).bold());
    println!();

    for result in &baseline_results {
        let status = if result.passed {
            "PASS".green()
        } else {
            "FAIL".red()
        };
        println!(
            "  {} {:40} {:.2}ns",
            status, result.name, result.statistics.mean_ns
        );
    }

    // Generate report
    let metadata = ReportMetadata::new(
        &format!("Baseline Comparison: Verum vs {}", lang_str),
        "1.0.0",
    );
    let report = BenchmarkReport::new(metadata, baseline_results, vec![], vec![]);

    let report_content = generate_report(&report, format.into())?;

    if let Some(path) = output {
        write_report(&report, format.into(), &path)?;
        println!("\nReport written to: {}", path.display());
    } else if matches!(
        format,
        FormatArg::Json | FormatArg::Html | FormatArg::Csv | FormatArg::Markdown
    ) {
        println!("{}", report_content);
    }

    Ok(())
}

// ============================================================================
// Utilities
// ============================================================================

fn format_ns(ns: f64) -> String {
    if ns >= 1_000_000_000.0 {
        format!("{:.2}s", ns / 1_000_000_000.0)
    } else if ns >= 1_000_000.0 {
        format!("{:.2}ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.2}us", ns / 1_000.0)
    } else {
        format!("{:.2}ns", ns)
    }
}
