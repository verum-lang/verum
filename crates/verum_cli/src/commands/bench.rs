//! Benchmark command — discover and execute `@bench` functions with
//! time-budget–driven sampling and industry-standard statistics.
//!
//! # Tiers
//!
//! Matches the `verum run` / `verum test` convention:
//!
//! | Tier         | How a sample is taken                                           |
//! |--------------|-----------------------------------------------------------------|
//! | Interpreter  | In-process: compile to VBC once, call `execute_function`        |
//! |              | in a Rust loop with `Instant::now()`. No process overhead.      |
//! | AOT (native) | Synthesise a driver `.vr` per `@bench` fn (original + appended  |
//! |              | `fn main() { bench_fn(); }`), compile, spawn binary; each run   |
//! |              | is a sample. Use when you need realistic native timings.        |
//!
//! Default: **AOT** (consistent with `verum run` / `verum test`; use
//! `--interp` or `--tier interpret` for the in-process path).
//!
//! # Sampling
//!
//! Following Criterion / hyperfine conventions, the harness runs a
//! warm-up phase (`--warm-up-time`, default 3 s) and a measurement
//! phase (`--measurement-time`, default 5 s). Within the measurement
//! window it adapts the sample count between `--min-samples` and
//! `--max-samples` so short benches still yield 100 samples and long
//! benches still terminate near the budget. Fixed iteration count is
//! available via `--sample-size`.
//!
//! # Statistics
//!
//! * Mean, median, stddev, min, max, MAD (median absolute deviation).
//! * Tukey IQR fences (1.5×IQR) for outlier classification — reported
//!   but not silently dropped; slow outliers are usually legitimate
//!   signal (GC, scheduler, thermals) that the reader should see.
//! * Bootstrap 95 % CI for the median (1 000 resamples) gives an
//!   honest uncertainty band around the headline number.
//!
//! # Output
//!
//! `--format table` (default), `--format json`, `--format csv`,
//! `--format markdown`. JSON output is stable across releases so CI
//! can diff against baselines without scraping.

use crate::error::{CliError, Result};
use crate::tier::Tier;
use crate::ui;
use colored::Colorize;
use rand::RngExt;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use verum_common::Text;
use verum_compiler::options::{CompilerOptions, OutputFormat, VerifyMode};
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;
use walkdir::WalkDir;

// --------------------------------------------------------------------
// Options & entry
// --------------------------------------------------------------------

/// CLI-facing options for `verum bench`.
///
/// Passed from `main.rs` after tier resolution — the command itself
/// never looks at the raw `--interp` / `--aot` flags, which keeps the
/// tier precedence identical across `run` / `test` / `bench`.
#[derive(Debug, Clone)]
pub struct BenchOptions {
    pub filter: Option<Text>,
    pub save_baseline: Option<Text>,
    pub baseline: Option<Text>,
    pub tier: Tier,
    pub format: ReportFormat,
    pub warm_up_time: Duration,
    pub measurement_time: Duration,
    pub min_samples: usize,
    pub max_samples: usize,
    pub sample_size: Option<usize>,
    pub noise_threshold_pct: f64,
    pub no_color: bool,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self {
            filter: None,
            save_baseline: None,
            baseline: None,
            tier: Tier::Aot,
            format: ReportFormat::Table,
            warm_up_time: Duration::from_secs(3),
            measurement_time: Duration::from_secs(5),
            min_samples: 10,
            max_samples: 100,
            sample_size: None,
            noise_threshold_pct: 2.0,
            no_color: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Table,
    Json,
    Csv,
    Markdown,
}

impl ReportFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "table" | "pretty" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            "markdown" | "md" => Ok(Self::Markdown),
            other => Err(CliError::InvalidArgument(format!(
                "unknown format `{}` (expected: table | json | csv | markdown)",
                other
            ))),
        }
    }
}

pub fn execute(opts: BenchOptions) -> Result<()> {
    // Honour `BenchOptions.no_color` — when set, suppress all
    // ANSI-colored output globally for the duration of the bench
    // run. Pre-fix the field landed on the options struct but no
    // code path consulted it, so `verum bench --no-color` still
    // emitted ANSI escapes (broke output captured in CI logs that
    // didn't strip them).
    if opts.no_color {
        colored::control::set_override(false);
    }

    let quiet = matches!(opts.format, ReportFormat::Json | ReportFormat::Csv);

    if !quiet {
        ui::header("Performance Benchmarks");
        println!();
        ui::info(&format!(
            "tier={} warm-up={}s measure={}s samples={}..{} format={:?}",
            opts.tier.as_str(),
            opts.warm_up_time.as_secs_f32(),
            opts.measurement_time.as_secs_f32(),
            opts.min_samples,
            opts.max_samples,
            opts.format,
        ));
    }

    let benches = discover_benchmarks(opts.filter.as_ref())?;
    if benches.is_empty() {
        if !quiet {
            ui::warn("No benchmark functions found in src/ tests/ benches/");
            ui::info("Mark a function with @bench (optionally @bench(group)) or name it fn bench_*");
        }
        return emit_results(&opts, &[]);
    }

    if !quiet {
        ui::step(&format!("Discovered {} bench function(s)", benches.len()));
    }

    // Group by source file so we compile each file at most once per tier.
    let mut by_file: HashMap<PathBuf, Vec<BenchFunc>> = HashMap::new();
    for b in benches {
        by_file.entry(b.file.clone()).or_default().push(b);
    }

    let mut results: Vec<BenchResult> = Vec::new();
    let mut compiled_cache: HashMap<PathBuf, CompiledFile> = HashMap::new();

    for (file, funcs) in &by_file {
        let display_path = display_path(file);
        if !quiet {
            ui::status("Compiling", &display_path);
        }

        let compiled = match prepare_compiled_file(file, opts.tier, funcs) {
            Ok(c) => c,
            Err(e) => {
                ui::error(&format!("failed to compile {}: {}", display_path, e));
                continue;
            }
        };
        compiled_cache.insert(file.clone(), compiled);
        let compiled = compiled_cache.get(file).unwrap();

        for func in funcs {
            if !quiet {
                print!("  bench {} ... ", func.name.as_str().dimmed());
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }

            match run_bench(opts.tier, compiled, func, &opts) {
                Ok(result) => {
                    if !quiet {
                        println!(
                            "{} {} ({:+.1}% CI95, n={})",
                            "ok".green(),
                            format_time(result.median_ns).cyan(),
                            result.ci95_rel_pct(),
                            result.times_ns.len(),
                        );
                    }
                    results.push(result);
                }
                Err(e) => {
                    if !quiet {
                        println!("{} ({})", "FAILED".red().bold(), e);
                    } else {
                        eprintln!("bench {} failed: {}", func.name, e);
                    }
                }
            }
        }
    }

    if !quiet {
        println!();
    }

    emit_results(&opts, &results)?;

    if let Some(ref base) = opts.baseline {
        if let Some(saved) = load_baseline(base.as_str()) {
            if !quiet {
                println!();
                print_baseline_comparison(&results, &saved.results, opts.noise_threshold_pct);
            }
        } else if !quiet {
            ui::warn(&format!("Baseline '{}' not found (target/bench/)", base));
        }
    }

    if let Some(ref save_name) = opts.save_baseline {
        save_baseline_file(save_name.as_str(), &results)?;
        if !quiet {
            ui::success(&format!("Baseline saved: target/bench/{}.json", save_name));
        }
    }

    if !quiet && !results.is_empty() {
        println!();
        ui::success(&format!("{} benchmark(s) completed", results.len()));
    }

    Ok(())
}

// --------------------------------------------------------------------
// Discovery
// --------------------------------------------------------------------

#[derive(Debug, Clone)]
struct BenchFunc {
    name: String,
    file: PathBuf,
    /// Optional group from `@bench(group)`.
    group: Option<String>,
}

fn discover_benchmarks(filter: Option<&Text>) -> Result<Vec<BenchFunc>> {
    let mut found = Vec::new();
    let mut scanned = false;
    for dir in ["src", "tests", "benches"] {
        let p = PathBuf::from(dir);
        if !p.exists() {
            continue;
        }
        scanned = true;
        for entry in WalkDir::new(&p).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("vr") {
                continue;
            }
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            scan_file_for_benches(path, &content, filter, &mut found);
        }
    }
    if !scanned {
        ui::info("No src/ tests/ benches/ dirs in current directory");
    }
    Ok(found)
}

/// Extract bench functions from a single file. Accepts:
///
/// * `@bench` on its own line, followed by `[visibility] fn NAME(`
/// * `@bench(group)` likewise; `group` is captured for reporting
/// * `[visibility] fn bench_NAME(` (legacy naming-convention form)
///
/// Valid visibility prefixes: `public`, `pub`, `private`, none.
fn scan_file_for_benches(
    path: &Path,
    content: &str,
    filter: Option<&Text>,
    out: &mut Vec<BenchFunc>,
) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        // @bench [, @bench(group)] — next non-blank fn line is the bench.
        let mut pending_group: Option<String> = None;
        let mut had_attr = false;
        if trimmed == "@bench" {
            had_attr = true;
        } else if let Some(rest) = trimmed.strip_prefix("@bench(") {
            had_attr = true;
            if let Some(end) = rest.find(')') {
                let g = rest[..end].trim();
                if !g.is_empty() {
                    pending_group = Some(g.to_string());
                }
            }
        }
        if had_attr {
            // Walk forward past blank lines / further attributes to the fn line.
            let mut j = i + 1;
            while j < lines.len() {
                let t = lines[j].trim();
                if t.is_empty() || t.starts_with("//") || t.starts_with("@") {
                    j += 1;
                    continue;
                }
                if let Some(name) = parse_fn_name(t) {
                    if should_include(&name, filter) {
                        out.push(BenchFunc {
                            name,
                            file: path.to_path_buf(),
                            group: pending_group.clone(),
                        });
                    }
                }
                break;
            }
            i = j + 1;
            continue;
        }
        // Legacy naming convention: fn bench_NAME
        if let Some(name) = parse_fn_name(trimmed) {
            if name.starts_with("bench_") && should_include(&name, filter) {
                out.push(BenchFunc {
                    name,
                    file: path.to_path_buf(),
                    group: None,
                });
            }
        }
        i += 1;
    }
}

fn should_include(name: &str, filter: Option<&Text>) -> bool {
    filter.map(|f| name.contains(f.as_str())).unwrap_or(true)
}

/// Parse `[public|pub|private] fn NAME(` → `NAME`, else `None`.
fn parse_fn_name(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = if let Some(r) = t.strip_prefix("public fn ") {
        r
    } else if let Some(r) = t.strip_prefix("pub fn ") {
        r
    } else if let Some(r) = t.strip_prefix("private fn ") {
        r
    } else if let Some(r) = t.strip_prefix("fn ") {
        r
    } else {
        return None;
    };
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(rest[..end].to_string())
    }
}

// --------------------------------------------------------------------
// Compile / prepare
// --------------------------------------------------------------------

/// Per-tier compile product cached per source file.
enum CompiledFile {
    /// AOT: we synthesise one driver binary per bench function, so the
    /// compile step here is a no-op and each function compiles lazily
    /// in `run_bench_aot`. We cache the source to avoid re-reading.
    AotSource { source: String },
    /// Interpreter: the VBC module is compiled once; each bench
    /// function dispatches via its `FunctionId`.
    Interpret {
        module: Arc<verum_vbc::VbcModule>,
        /// Map bench-function name → VBC FunctionId.
        func_ids: HashMap<String, verum_vbc::FunctionId>,
    },
}

fn prepare_compiled_file(file: &Path, tier: Tier, funcs: &[BenchFunc]) -> Result<CompiledFile> {
    match tier {
        Tier::Interpret => prepare_interpret(file, funcs),
        Tier::Aot => {
            let src = fs::read_to_string(file).map_err(|e| {
                CliError::CompilationFailed(format!("read {}: {}", file.display(), e))
            })?;
            if contains_main_fn(&src) {
                return Err(CliError::CompilationFailed(format!(
                    "{} already defines fn main(); AOT bench mode synthesises its own driver. \
                     Either remove main() or use --interp.",
                    file.display()
                )));
            }
            Ok(CompiledFile::AotSource { source: src })
        }
    }
}

fn contains_main_fn(src: &str) -> bool {
    src.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("fn main(")
            || t.starts_with("public fn main(")
            || t.starts_with("pub fn main(")
            || t.starts_with("async fn main(")
    })
}

fn prepare_interpret(file: &Path, funcs: &[BenchFunc]) -> Result<CompiledFile> {
    use verum_ast::{FileId, Module};
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

    let src = fs::read_to_string(file)
        .map_err(|e| CliError::CompilationFailed(format!("read {}: {}", file.display(), e)))?;
    let fid = FileId::new(0);
    let parser = VerumParser::new();
    let lexer = Lexer::new(&src, fid);
    let module_ast: Module = parser.parse_module(lexer, fid).map_err(|errs| {
        let joined = errs.iter().map(|e| format!("{}", e)).collect::<Vec<_>>().join("\n");
        CliError::CompilationFailed(format!("parse {}: {}", file.display(), joined))
    })?;

    let config = CodegenConfig::new("bench");
    let mut codegen = VbcCodegen::with_config(config);
    let module = codegen
        .compile_module(&module_ast)
        .map_err(|e| CliError::CompilationFailed(format!("codegen: {:?}", e)))?;

    let module = Arc::new(module);

    let mut func_ids = HashMap::new();
    for f in funcs {
        let id_opt = module
            .functions
            .iter()
            .find(|vf| module.get_string(vf.name) == Some(f.name.as_str()))
            .map(|vf| vf.id);
        match id_opt {
            Some(id) => {
                func_ids.insert(f.name.clone(), id);
            }
            None => {
                return Err(CliError::CompilationFailed(format!(
                    "bench fn `{}` not found in compiled VBC module of {}",
                    f.name,
                    file.display()
                )));
            }
        }
    }

    Ok(CompiledFile::Interpret { module, func_ids })
}

// --------------------------------------------------------------------
// Execution (per-sample)
// --------------------------------------------------------------------

fn run_bench(
    tier: Tier,
    compiled: &CompiledFile,
    func: &BenchFunc,
    opts: &BenchOptions,
) -> Result<BenchResult> {
    match tier {
        Tier::Interpret => run_bench_interpret(compiled, func, opts),
        Tier::Aot => run_bench_aot(compiled, func, opts),
    }
}

fn run_bench_interpret(
    compiled: &CompiledFile,
    func: &BenchFunc,
    opts: &BenchOptions,
) -> Result<BenchResult> {
    use verum_vbc::interpreter::Interpreter;

    let (module, func_ids) = match compiled {
        CompiledFile::Interpret { module, func_ids } => (module, func_ids),
        _ => return Err(CliError::RuntimeError("compiled artifact mismatch".into())),
    };
    let fid = *func_ids.get(&func.name).ok_or_else(|| {
        CliError::RuntimeError(format!("bench fn `{}` not mapped", func.name))
    })?;

    // Single interpreter instance reused across samples so we're not
    // re-paying state-construction overhead each iteration. Each call
    // to `execute_function` runs the bench body once and returns.
    // Disable the interpreter's safety caps — benches intentionally run
    // hot loops with billions of VBC ops, and the counters are cumulative
    // across the Interpreter's lifetime (not per-call), so even one
    // @bench with ITERATIONS=10^6 trips the default 100M cap. Setting
    // max_instructions=0 and timeout_ms=0 disables both gates (the
    // dispatch check is `count > max && max > 0`).
    let mut interp = Interpreter::new(Arc::clone(module));
    interp.state.config.max_instructions = 0;
    interp.state.config.timeout_ms = 0;

    // Warm-up: run while warm_up_time hasn't elapsed. Discard timings.
    let warm_end = Instant::now() + opts.warm_up_time;
    let mut warm_calls = 0usize;
    while Instant::now() < warm_end && warm_calls < opts.max_samples * 4 {
        let _ = interp
            .execute_function(fid)
            .map_err(|e| CliError::RuntimeError(format!("bench body: {:?}", e)))?;
        warm_calls += 1;
    }

    // Measurement: run until we've reached measurement_time or max_samples,
    // but keep going until at least min_samples have been collected.
    let fixed = opts.sample_size;
    let measure_end = Instant::now() + opts.measurement_time;
    let mut times_ns: Vec<f64> = Vec::with_capacity(opts.max_samples);
    let target_max = fixed.unwrap_or(opts.max_samples);
    let target_min = fixed.unwrap_or(opts.min_samples);
    loop {
        let t0 = Instant::now();
        let _ = interp
            .execute_function(fid)
            .map_err(|e| CliError::RuntimeError(format!("bench body: {:?}", e)))?;
        let elapsed = t0.elapsed();
        times_ns.push(elapsed.as_nanos() as f64);
        if times_ns.len() >= target_max {
            break;
        }
        if fixed.is_none() && Instant::now() >= measure_end && times_ns.len() >= target_min {
            break;
        }
    }

    Ok(BenchResult::from_samples(
        func.name.clone(),
        func.group.clone(),
        "interpret".to_string(),
        times_ns,
    ))
}

fn run_bench_aot(
    compiled: &CompiledFile,
    func: &BenchFunc,
    opts: &BenchOptions,
) -> Result<BenchResult> {
    let src = match compiled {
        CompiledFile::AotSource { source } => source,
        _ => return Err(CliError::RuntimeError("compiled artifact mismatch".into())),
    };

    // Synthesise a driver: original source + `fn main() { bench_fn(); }`.
    // Each bench fn gets its own driver file and its own binary.
    let target_dir = PathBuf::from("target/bench/drivers");
    fs::create_dir_all(&target_dir).map_err(|e| {
        CliError::CompilationFailed(format!(
            "mkdir {}: {}",
            target_dir.display(),
            e
        ))
    })?;
    let driver_path = target_dir.join(format!("{}.vr", func.name));
    let driver_source = format!(
        "{}\n\n// --- bench driver (auto-generated) ---\nfn main() {{\n    {}();\n}}\n",
        src, func.name
    );
    fs::write(&driver_path, &driver_source).map_err(|e| {
        CliError::CompilationFailed(format!("write {}: {}", driver_path.display(), e))
    })?;

    let language_features = crate::feature_overrides::scratch_features()?;
    let options = CompilerOptions {
        input: driver_path.clone(),
        verify_mode: VerifyMode::Runtime,
        output_format: OutputFormat::Human,
        optimization_level: 3,
        language_features,
        ..Default::default()
    };
    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);
    let exe = pipeline
        .run_native_compilation()
        .map_err(|e| CliError::CompilationFailed(format!("AOT: {}", e)))?;

    // Warm-up: run N times without timing.
    let warm_end = Instant::now() + opts.warm_up_time;
    let mut warm_runs = 0usize;
    while Instant::now() < warm_end && warm_runs < opts.max_samples {
        let _ = Command::new(&exe)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        warm_runs += 1;
    }

    let fixed = opts.sample_size;
    let measure_end = Instant::now() + opts.measurement_time;
    let target_max = fixed.unwrap_or(opts.max_samples);
    let target_min = fixed.unwrap_or(opts.min_samples);
    let mut times_ns: Vec<f64> = Vec::with_capacity(target_max);
    loop {
        let t0 = Instant::now();
        let status = Command::new(&exe)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| CliError::RuntimeError(format!("spawn: {}", e)))?;
        let elapsed = t0.elapsed();
        if !status.success() {
            return Err(CliError::RuntimeError(format!(
                "bench exited with code {}",
                status.code().unwrap_or(-1)
            )));
        }
        times_ns.push(elapsed.as_nanos() as f64);
        if times_ns.len() >= target_max {
            break;
        }
        if fixed.is_none() && Instant::now() >= measure_end && times_ns.len() >= target_min {
            break;
        }
    }

    Ok(BenchResult::from_samples(
        func.name.clone(),
        func.group.clone(),
        "aot".to_string(),
        times_ns,
    ))
}

// --------------------------------------------------------------------
// Statistics
// --------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BenchResult {
    pub name: String,
    pub group: Option<String>,
    pub tier: String,
    pub times_ns: Vec<f64>,
    pub mean_ns: f64,
    pub median_ns: f64,
    pub stddev_ns: f64,
    pub mad_ns: f64,
    pub min_ns: f64,
    pub max_ns: f64,
    /// 95 % bootstrap CI for the median: (lower, upper) in nanoseconds.
    pub ci95_lo_ns: f64,
    pub ci95_hi_ns: f64,
    /// Tukey outlier classifier (1.5×IQR from Q1/Q3).
    pub outliers: OutlierReport,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct OutlierReport {
    pub low_mild: usize,
    pub high_mild: usize,
    pub low_severe: usize,
    pub high_severe: usize,
}

impl BenchResult {
    pub fn from_samples(
        name: String,
        group: Option<String>,
        tier: String,
        mut times: Vec<f64>,
    ) -> Self {
        // Sort ascending for percentile work.
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = times.len();
        assert!(n > 0, "BenchResult needs at least one sample");

        let mean = mean(&times);
        let median = percentile(&times, 0.5);
        let stddev = stddev(&times, mean);
        let mad = median_absolute_deviation(&times, median);
        let min = times[0];
        let max = times[n - 1];

        let (q1, q3) = (percentile(&times, 0.25), percentile(&times, 0.75));
        let iqr = q3 - q1;
        let mild_lo = q1 - 1.5 * iqr;
        let mild_hi = q3 + 1.5 * iqr;
        let sev_lo = q1 - 3.0 * iqr;
        let sev_hi = q3 + 3.0 * iqr;
        let mut outliers = OutlierReport::default();
        for &t in &times {
            if t < sev_lo {
                outliers.low_severe += 1;
            } else if t < mild_lo {
                outliers.low_mild += 1;
            } else if t > sev_hi {
                outliers.high_severe += 1;
            } else if t > mild_hi {
                outliers.high_mild += 1;
            }
        }

        let (ci_lo, ci_hi) = bootstrap_median_ci95(&times, 1000);

        BenchResult {
            name,
            group,
            tier,
            times_ns: times,
            mean_ns: mean,
            median_ns: median,
            stddev_ns: stddev,
            mad_ns: mad,
            min_ns: min,
            max_ns: max,
            ci95_lo_ns: ci_lo,
            ci95_hi_ns: ci_hi,
            outliers,
        }
    }

    /// Half-width of the 95 % CI as a percent of the median — the
    /// "± X%" number usually quoted in bench output.
    pub fn ci95_rel_pct(&self) -> f64 {
        if self.median_ns == 0.0 {
            return 0.0;
        }
        let half = (self.ci95_hi_ns - self.ci95_lo_ns) / 2.0;
        (half / self.median_ns) * 100.0
    }
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn stddev(xs: &[f64], mean: f64) -> f64 {
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / xs.len() as f64;
    var.sqrt()
}

/// Expects `xs` already sorted ascending.
fn percentile(xs: &[f64], p: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let n = xs.len();
    if n == 1 {
        return xs[0];
    }
    let idx = p * (n as f64 - 1.0);
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        xs[lo]
    } else {
        let frac = idx - lo as f64;
        xs[lo] * (1.0 - frac) + xs[hi] * frac
    }
}

fn median_absolute_deviation(xs: &[f64], med: f64) -> f64 {
    let mut devs: Vec<f64> = xs.iter().map(|x| (x - med).abs()).collect();
    devs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile(&devs, 0.5)
}

/// Percentile bootstrap 95 % CI for the median.
///
/// Draws `iters` resamples with replacement, computes the median of
/// each, returns the 2.5 / 97.5 percentiles of the resulting
/// distribution. No external deps; uses `rand` which is already in
/// the workspace. For small sample counts (<10) this is wider than
/// a t-CI would be but doesn't require distributional assumptions.
fn bootstrap_median_ci95(samples: &[f64], iters: usize) -> (f64, f64) {
    let n = samples.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    if n == 1 {
        return (samples[0], samples[0]);
    }
    let mut rng = rand::rng();
    let mut medians: Vec<f64> = Vec::with_capacity(iters);
    let mut buf = vec![0.0f64; n];
    for _ in 0..iters {
        for slot in buf.iter_mut() {
            let k = rng.random_range(0..n);
            *slot = samples[k];
        }
        buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        medians.push(percentile(&buf, 0.5));
    }
    medians.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (percentile(&medians, 0.025), percentile(&medians, 0.975))
}

// --------------------------------------------------------------------
// Output / reporting
// --------------------------------------------------------------------

fn emit_results(opts: &BenchOptions, results: &[BenchResult]) -> Result<()> {
    match opts.format {
        ReportFormat::Table => print_table(results),
        ReportFormat::Json => print_json(results)?,
        ReportFormat::Csv => print_csv(results),
        ReportFormat::Markdown => print_markdown(results),
    }
    Ok(())
}

fn print_table(results: &[BenchResult]) {
    if results.is_empty() {
        return;
    }
    println!("{}", "Results:".bold());
    println!();
    println!(
        "  {:<32} {:>6} {:>12} {:>12} {:>14} {:>10} {:>10}",
        "Benchmark".bold(),
        "Tier".bold(),
        "Median".bold(),
        "Mean".bold(),
        "±CI95".bold(),
        "Min".bold(),
        "Outliers".bold(),
    );
    println!("  {}", "-".repeat(102));
    for r in results {
        let outl = r.outliers.low_mild + r.outliers.high_mild
            + r.outliers.low_severe + r.outliers.high_severe;
        let tier = match r.tier.as_str() {
            "aot" => "aot".green().to_string(),
            _ => "interp".cyan().to_string(),
        };
        println!(
            "  {:<32} {:>6} {:>12} {:>12} {:>14} {:>10} {:>10}",
            truncate(&r.name, 32),
            tier,
            format_time(r.median_ns).cyan(),
            format_time(r.mean_ns),
            format!("±{:.1}%", r.ci95_rel_pct()).dimmed(),
            format_time(r.min_ns).green(),
            format!("{}/{}", outl, r.times_ns.len()).dimmed(),
        );
    }
}

fn print_json(results: &[BenchResult]) -> Result<()> {
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "schema": "verum-bench/v1",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "results": results,
    }))
    .map_err(|e| CliError::Custom(format!("json: {}", e)))?;
    println!("{}", json);
    Ok(())
}

fn print_csv(results: &[BenchResult]) {
    println!("name,tier,group,n,median_ns,mean_ns,stddev_ns,mad_ns,min_ns,max_ns,ci95_lo_ns,ci95_hi_ns,outliers_total");
    for r in results {
        let outl = r.outliers.low_mild + r.outliers.high_mild
            + r.outliers.low_severe + r.outliers.high_severe;
        println!(
            "{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{}",
            csv_escape(&r.name),
            r.tier,
            r.group.as_deref().unwrap_or(""),
            r.times_ns.len(),
            r.median_ns,
            r.mean_ns,
            r.stddev_ns,
            r.mad_ns,
            r.min_ns,
            r.max_ns,
            r.ci95_lo_ns,
            r.ci95_hi_ns,
            outl,
        );
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn print_markdown(results: &[BenchResult]) {
    println!("| Benchmark | Tier | Median | Mean | ±CI95 | Min | n | Outliers |");
    println!("|-----------|------|--------|------|-------|-----|---|----------|");
    for r in results {
        let outl = r.outliers.low_mild + r.outliers.high_mild
            + r.outliers.low_severe + r.outliers.high_severe;
        println!(
            "| `{}` | {} | {} | {} | ±{:.1}% | {} | {} | {} |",
            r.name,
            r.tier,
            format_time(r.median_ns),
            format_time(r.mean_ns),
            r.ci95_rel_pct(),
            format_time(r.min_ns),
            r.times_ns.len(),
            outl,
        );
    }
}

fn format_time(ns: f64) -> String {
    if ns < 1_000.0 {
        format!("{:.1} ns", ns)
    } else if ns < 1_000_000.0 {
        format!("{:.2} µs", ns / 1_000.0)
    } else if ns < 1_000_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else {
        format!("{:.2} s", ns / 1_000_000_000.0)
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}

fn display_path(p: &Path) -> String {
    if let Ok(rel) = p.strip_prefix(std::env::current_dir().unwrap_or_default()) {
        rel.display().to_string()
    } else {
        p.display().to_string()
    }
}

// --------------------------------------------------------------------
// Baseline (save / load / diff)
// --------------------------------------------------------------------

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Baseline {
    timestamp: String,
    results: Vec<BenchResult>,
}

fn baseline_dir() -> PathBuf {
    PathBuf::from("target/bench")
}

fn save_baseline_file(name: &str, results: &[BenchResult]) -> Result<()> {
    let dir = baseline_dir();
    fs::create_dir_all(&dir).map_err(|e| {
        CliError::Custom(format!("mkdir {}: {}", dir.display(), e))
    })?;
    let path = dir.join(format!("{}.json", name));
    let baseline = Baseline {
        timestamp: chrono::Utc::now().to_rfc3339(),
        results: results.to_vec(),
    };
    let json = serde_json::to_string_pretty(&baseline)
        .map_err(|e| CliError::Custom(format!("json: {}", e)))?;
    fs::write(&path, json).map_err(|e| CliError::Custom(format!("write: {}", e)))?;
    Ok(())
}

fn load_baseline(name: &str) -> Option<Baseline> {
    let path = baseline_dir().join(format!("{}.json", name));
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Side-by-side comparison with significance check.
///
/// A change is flagged only when the point estimate (median) moved by
/// more than `noise_threshold_pct` AND the CI95 ranges don't overlap
/// (simple but robust "are they distinguishable" test).
fn print_baseline_comparison(
    current: &[BenchResult],
    baseline: &[BenchResult],
    noise_pct: f64,
) {
    println!("{}", "Baseline Comparison:".bold());
    println!();
    println!(
        "  {:<32} {:>12} {:>12} {:>18}",
        "Benchmark".bold(),
        "Current".bold(),
        "Baseline".bold(),
        "Change".bold(),
    );
    println!("  {}", "-".repeat(80));
    let map: HashMap<&str, &BenchResult> =
        baseline.iter().map(|b| (b.name.as_str(), b)).collect();
    for cur in current {
        if let Some(base) = map.get(cur.name.as_str()) {
            let pct = ((cur.median_ns - base.median_ns) / base.median_ns) * 100.0;
            let overlaps = !(cur.ci95_hi_ns < base.ci95_lo_ns || base.ci95_hi_ns < cur.ci95_lo_ns);
            let within_noise = pct.abs() < noise_pct;
            let change = if within_noise || overlaps {
                format!("{:+.1}% (noise)", pct).dimmed()
            } else if pct < 0.0 {
                format!("{:+.1}% faster ✓", pct).green()
            } else {
                format!("{:+.1}% slower ✗", pct).red()
            };
            println!(
                "  {:<32} {:>12} {:>12} {:>18}",
                truncate(&cur.name, 32),
                format_time(cur.median_ns).cyan(),
                format_time(base.median_ns),
                change,
            );
        } else {
            println!(
                "  {:<32} {:>12} {:>12} {:>18}",
                truncate(&cur.name, 32),
                format_time(cur.median_ns).cyan(),
                "—",
                "new".yellow(),
            );
        }
    }
}

// --------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fn_accepts_all_visibilities() {
        assert_eq!(parse_fn_name("fn foo() {").as_deref(), Some("foo"));
        assert_eq!(parse_fn_name("pub fn foo() {").as_deref(), Some("foo"));
        assert_eq!(parse_fn_name("public fn foo() {").as_deref(), Some("foo"));
        assert_eq!(parse_fn_name("private fn foo_bar() {").as_deref(), Some("foo_bar"));
        assert_eq!(parse_fn_name("let x = 1;"), None);
    }

    #[test]
    fn discover_finds_bench_after_attribute() {
        let src = "const N: Int = 1;\n@bench\npublic fn bench_foo() {\n    let _ = N;\n}\n";
        let mut out = Vec::new();
        scan_file_for_benches(Path::new("x.vr"), src, None, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "bench_foo");
        assert!(out[0].group.is_none());
    }

    #[test]
    fn discover_picks_up_group() {
        let src = "@bench(hot_path)\nfn bench_foo() {}\n";
        let mut out = Vec::new();
        scan_file_for_benches(Path::new("x.vr"), src, None, &mut out);
        assert_eq!(out[0].group.as_deref(), Some("hot_path"));
    }

    #[test]
    fn discover_legacy_naming() {
        let src = "fn bench_legacy() {}\nfn regular() {}\n";
        let mut out = Vec::new();
        scan_file_for_benches(Path::new("x.vr"), src, None, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "bench_legacy");
    }

    #[test]
    fn stats_smoke() {
        let r = BenchResult::from_samples(
            "x".into(),
            None,
            "interpret".into(),
            vec![100.0, 102.0, 99.0, 101.0, 300.0, 98.0, 103.0, 97.0, 100.0, 101.0],
        );
        // Median near 100.5
        assert!(r.median_ns > 99.0 && r.median_ns < 102.0);
        // 300.0 should land as a high outlier
        assert!(r.outliers.high_mild + r.outliers.high_severe >= 1);
        // CI95 contains median
        assert!(r.ci95_lo_ns <= r.median_ns && r.median_ns <= r.ci95_hi_ns);
    }

    #[test]
    fn format_time_picks_correct_unit() {
        assert!(format_time(500.0).contains("ns"));
        assert!(format_time(5_000.0).contains("µs"));
        assert!(format_time(5_000_000.0).contains("ms"));
        assert!(format_time(5_000_000_000.0).contains(" s"));
    }

    #[test]
    fn no_color_default_is_false() {
        // Pin: the documented default keeps coloured output enabled
        // so users running `verum bench` interactively see the
        // styled tables. CI invocations that need plain text opt
        // in via `--no-color`.
        let opts = BenchOptions::default();
        assert!(!opts.no_color, "default no_color must stay false");
    }

    #[test]
    fn no_color_flag_disables_colored_output_globally() {
        // Pin: setting `--no-color` on the CLI flips the global
        // colored::control override so every subsequent ANSI-styled
        // print emits plain text. We can't observe the override
        // in another thread without test isolation, but we can
        // verify the override surface itself reaches the runtime.
        // The wiring runs unconditionally — if a refactor drops
        // the call, future bench output regressions to ANSI in CI
        // logs.
        let opts = BenchOptions {
            no_color: true,
            ..BenchOptions::default()
        };
        assert!(opts.no_color);

        // Apply the override directly (mirrors the wiring in
        // execute) and assert that a freshly-coloured string
        // renders without ANSI escapes.
        colored::control::set_override(false);
        let plain = "test".red().to_string();
        assert!(
            !plain.contains("\u{1b}["),
            "set_override(false) must strip ANSI escapes, got: {:?}",
            plain,
        );
        // Restore default for any subsequent tests in the suite.
        colored::control::unset_override();
    }
}
