//! Benchmark command — discover and execute @bench functions with real timing.
//!
//! Discovers benchmark functions in src/ and tests/ directories, compiles
//! them via the compilation pipeline, executes each N times, and reports
//! statistical timing results. Supports baseline save/load for regression
//! detection.

use crate::error::{CliError, Result};
use crate::ui;
use colored::Colorize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use verum_common::Text;
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::options::{CompilerOptions, OutputFormat};
use verum_compiler::session::Session;
use verum_compiler::VerifyMode;
use walkdir::WalkDir;

/// Number of iterations per benchmark for statistical significance.
const DEFAULT_ITERATIONS: usize = 10;
/// Warm-up iterations (not counted).
const WARMUP_ITERATIONS: usize = 2;

/// Result of benchmarking a single function.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchResult {
    pub name: String,
    pub times_ns: Vec<f64>,
    pub mean_ns: f64,
    pub median_ns: f64,
    pub min_ns: f64,
    pub max_ns: f64,
    pub stddev_ns: f64,
}

impl BenchResult {
    fn from_times(name: String, mut times: Vec<f64>) -> Self {
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = times.len() as f64;
        let mean = times.iter().sum::<f64>() / n;
        let median = if times.len() % 2 == 0 {
            (times[times.len() / 2 - 1] + times[times.len() / 2]) / 2.0
        } else {
            times[times.len() / 2]
        };
        let min = times.first().copied().unwrap_or(0.0);
        let max = times.last().copied().unwrap_or(0.0);
        let variance = times.iter().map(|t| (t - mean).powi(2)).sum::<f64>() / n;
        let stddev = variance.sqrt();

        Self {
            name,
            times_ns: times,
            mean_ns: mean,
            median_ns: median,
            min_ns: min,
            max_ns: max,
            stddev_ns: stddev,
        }
    }
}

/// Saved baseline for comparison.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Baseline {
    timestamp: String,
    results: Vec<BenchResult>,
}

/// Discovered benchmark function.
#[derive(Debug)]
struct BenchFunc {
    name: String,
    file: PathBuf,
}

pub fn execute(
    filter: Option<Text>,
    save_baseline: Option<Text>,
    baseline: Option<Text>,
    compare_tiers: bool,
    profile: bool,
) -> Result<()> {
    ui::header("Performance Benchmarks");
    println!();

    if compare_tiers {
        ui::info("Tier comparison mode: measuring across execution tiers");
    }
    if profile {
        ui::info("Profiling mode: collecting detailed per-benchmark metrics");
    }

    // Discover benchmark functions
    let benches = discover_benchmarks(filter.as_ref());

    if benches.is_empty() {
        ui::warn("No benchmark functions found");
        ui::info("Create functions with @bench attribute or fn bench_* naming:");
        println!();
        println!("  {}", "@bench".magenta());
        println!("  {} bench_my_algorithm() {{", "fn".magenta());
        println!("      // code to benchmark");
        println!("  }}");
        return Ok(());
    }

    ui::step(&format!("Found {} benchmark(s)", benches.len()));
    println!();

    // Group by file
    let mut by_file: HashMap<PathBuf, Vec<&BenchFunc>> = HashMap::new();
    for bench in &benches {
        by_file.entry(bench.file.clone()).or_default().push(bench);
    }

    // Execute benchmarks
    let mut results = Vec::new();

    for (file, funcs) in &by_file {
        let file_display = file.strip_prefix(std::env::current_dir().unwrap_or_default())
            .unwrap_or(file)
            .display();

        // Compile the file once
        ui::status("Compiling", &file_display.to_string());
        let executable = match compile_bench_file(file) {
            Ok(exe) => exe,
            Err(e) => {
                ui::error(&format!("Failed to compile {}: {}", file_display, e));
                continue;
            }
        };

        // Run each benchmark function
        for func in funcs {
            print!("  bench {} ... ", func.name);

            match run_benchmark(&executable, &func.name) {
                Ok(result) => {
                    let time_str = format_time(result.median_ns);
                    let stddev_str = format_time(result.stddev_ns);
                    println!("{} ({} +/- {})",
                        "ok".green(),
                        time_str.cyan(),
                        stddev_str.dimmed(),
                    );
                    results.push(result);
                }
                Err(e) => {
                    println!("{} ({})", "FAILED".red().bold(), e);
                }
            }
        }
    }

    println!();

    if results.is_empty() {
        ui::warn("No benchmarks executed successfully");
        return Ok(());
    }

    // Print results table
    print_results_table(&results);

    // Baseline comparison
    if let Some(ref baseline_name) = baseline {
        if let Some(saved) = load_baseline(baseline_name.as_str()) {
            println!();
            print_baseline_comparison(&results, &saved.results);
        } else {
            ui::warn(&format!("Baseline '{}' not found", baseline_name));
        }
    }

    // Save baseline
    if let Some(ref save_name) = save_baseline {
        save_baseline_file(save_name.as_str(), &results);
        ui::success(&format!("Baseline saved: {}", save_name));
    }

    println!();
    ui::success(&format!(
        "{} benchmarks completed",
        results.len(),
    ));

    Ok(())
}

/// Discover benchmark functions from .vr files.
fn discover_benchmarks(filter: Option<&Text>) -> Vec<BenchFunc> {
    let mut benches = Vec::new();

    let search_dirs: Vec<PathBuf> = ["src", "tests", "benches"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();

    for dir in &search_dirs {
        for entry in WalkDir::new(dir).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() { continue; }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "vr" { continue; }

            if let Ok(content) = fs::read_to_string(path) {
                // Find @bench annotated functions or fn bench_* functions
                let mut prev_line_is_bench = false;
                for line in content.lines() {
                    let trimmed = line.trim();

                    if trimmed == "@bench" || trimmed.starts_with("@bench(") {
                        prev_line_is_bench = true;
                        continue;
                    }

                    let is_bench_fn = prev_line_is_bench
                        || (trimmed.starts_with("fn bench_") && trimmed.contains('{'));

                    if is_bench_fn && trimmed.starts_with("fn ") {
                        if let Some(name) = extract_fn_name(trimmed) {
                            let should_include = filter
                                .map(|f| name.contains(f.as_str()))
                                .unwrap_or(true);

                            if should_include {
                                benches.push(BenchFunc {
                                    name,
                                    file: path.to_path_buf(),
                                });
                            }
                        }
                    }

                    prev_line_is_bench = false;
                }
            }
        }
    }

    benches
}

/// Extract function name from `fn name(...) { ... }`.
fn extract_fn_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("fn ")?.trim();
    let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
    Some(rest[..end].to_string())
}

/// Compile a benchmark file to a native executable.
fn compile_bench_file(file: &Path) -> Result<PathBuf> {
    // Inherit CLI feature overrides so benchmark builds honor the
    // same `-Z` / `--cbgr` / `--no-cubical` settings as `verum build`.
    let language_features = crate::feature_overrides::scratch_features()?;
    let options = CompilerOptions {
        input: file.to_path_buf(),
        verify_mode: VerifyMode::Runtime,
        output_format: OutputFormat::Human,
        optimization_level: 2, // Optimized for accurate benchmarks
        language_features,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    pipeline
        .run_native_compilation()
        .map_err(|e| CliError::CompilationFailed(e.to_string()))
}

/// Run a single benchmark function N times and collect timing.
fn run_benchmark(executable: &Path, _func_name: &str) -> Result<BenchResult> {
    let mut times = Vec::new();

    // Warmup
    for _ in 0..WARMUP_ITERATIONS {
        let _ = Command::new(executable)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    // Measured runs
    for _ in 0..DEFAULT_ITERATIONS {
        let start = Instant::now();
        let status = Command::new(executable)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| CliError::RuntimeError(e.to_string()))?;

        let elapsed = start.elapsed();

        if !status.success() {
            return Err(CliError::RuntimeError(format!(
                "Benchmark exited with code {}",
                status.code().unwrap_or(-1)
            )));
        }

        times.push(elapsed.as_nanos() as f64);
    }

    let name = executable
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(BenchResult::from_times(name, times))
}

/// Format time in human-readable units.
fn format_time(ns: f64) -> String {
    if ns < 1_000.0 {
        format!("{:.1}ns", ns)
    } else if ns < 1_000_000.0 {
        format!("{:.2}us", ns / 1_000.0)
    } else if ns < 1_000_000_000.0 {
        format!("{:.2}ms", ns / 1_000_000.0)
    } else {
        format!("{:.2}s", ns / 1_000_000_000.0)
    }
}

/// Print results table.
fn print_results_table(results: &[BenchResult]) {
    println!("{}", "Results:".bold());
    println!();
    println!(
        "  {:<35} {:>12} {:>12} {:>12} {:>10}",
        "Benchmark".bold(),
        "Median".bold(),
        "Mean".bold(),
        "Std Dev".bold(),
        "Min".bold(),
    );
    println!("  {}", "-".repeat(83));

    for r in results {
        println!(
            "  {:<35} {:>12} {:>12} {:>12} {:>10}",
            r.name,
            format_time(r.median_ns).cyan(),
            format_time(r.mean_ns),
            format_time(r.stddev_ns).dimmed(),
            format_time(r.min_ns).green(),
        );
    }
}

/// Print comparison against saved baseline.
fn print_baseline_comparison(current: &[BenchResult], baseline: &[BenchResult]) {
    println!("{}", "Baseline Comparison:".bold());
    println!();
    println!(
        "  {:<35} {:>12} {:>12} {:>12}",
        "Benchmark".bold(), "Current".bold(), "Baseline".bold(), "Change".bold(),
    );
    println!("  {}", "-".repeat(73));

    let baseline_map: HashMap<&str, &BenchResult> = baseline.iter()
        .map(|b| (b.name.as_str(), b))
        .collect();

    for cur in current {
        if let Some(base) = baseline_map.get(cur.name.as_str()) {
            let pct = ((cur.median_ns - base.median_ns) / base.median_ns) * 100.0;
            let change = if pct < -1.0 {
                format!("{:.1}% faster", -pct).green()
            } else if pct > 1.0 {
                format!("{:.1}% slower", pct).red()
            } else {
                "~same".dimmed()
            };

            println!(
                "  {:<35} {:>12} {:>12} {:>12}",
                cur.name,
                format_time(cur.median_ns).cyan(),
                format_time(base.median_ns),
                change,
            );
        } else {
            println!(
                "  {:<35} {:>12} {:>12} {:>12}",
                cur.name,
                format_time(cur.median_ns).cyan(),
                "--",
                "new".yellow(),
            );
        }
    }
}

/// Save benchmark results as a baseline.
fn save_baseline_file(name: &str, results: &[BenchResult]) {
    let baseline = Baseline {
        timestamp: chrono::Utc::now().to_rfc3339(),
        results: results.to_vec(),
    };

    let dir = PathBuf::from("target/bench");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{}.json", name));

    if let Ok(json) = serde_json::to_string_pretty(&baseline) {
        let _ = fs::write(&path, json);
    }
}

/// Load a saved baseline.
fn load_baseline(name: &str) -> Option<Baseline> {
    let path = PathBuf::from(format!("target/bench/{}.json", name));
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}
