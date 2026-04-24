//! Test command — discover and execute `@test` functions and whole-
//! file tests across both execution tiers.
//!
//! # Tiers
//!
//! Matches the `verum run` / `verum bench` convention:
//!
//! | Tier         | How a test is run                                           |
//! |--------------|-------------------------------------------------------------|
//! | Interpreter  | Compile file to VBC once, run main()-or-test via the        |
//! |              | interpreter in-process. Fast iteration, full diagnostics.   |
//! | AOT (native) | Build a binary per test file, spawn it; exit 0 == pass.     |
//!
//! Default: **AOT** (a test is a promise about the final artefact;
//! interpreter is available via `--interp` for fast red-green loops).
//!
//! # Options modelled on libtest / `cargo test`
//!
//! * `--filter STR` — substring match on test name
//! * `--exact` — require full match (like libtest `--exact`)
//! * `--skip PATTERN` — substring-exclude; repeatable
//! * `--include-ignored` — run all, including `@ignore`
//! * `--ignored` — run **only** `@ignore`d tests (useful to promote them)
//! * `--list` — print discovered tests and exit
//! * `--nocapture` — don't capture stdout/stderr
//! * `--test-threads N` — parallel workers; wired to rayon here (was
//!   accepted-but-ignored previously)
//! * `--format pretty | terse | json` — presentation; `json` emits one
//!   newline-delimited JSON event per test for CI ingest

use colored::Colorize;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use verum_common::{List, Text};

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::tier::Tier;
use crate::ui;
use verum_ast::{FileId, ItemKind};
use verum_compiler::options::{CompilerOptions, OutputFormat, VerifyMode};
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

// --------------------------------------------------------------------
// Public options & entry
// --------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TestOptions {
    pub filter: Option<Text>,
    pub release: bool,
    pub nocapture: bool,
    pub test_threads: Option<usize>,
    pub coverage: bool,
    pub verify: Option<Text>,
    pub tier: Tier,
    pub format: TestFormat,
    pub list: bool,
    pub include_ignored: bool,
    pub ignored_only: bool,
    pub exact: bool,
    pub skip: Vec<Text>,
}

impl Default for TestOptions {
    fn default() -> Self {
        Self {
            filter: None,
            release: false,
            nocapture: false,
            test_threads: None,
            coverage: false,
            verify: None,
            tier: Tier::Aot,
            format: TestFormat::Pretty,
            list: false,
            include_ignored: false,
            ignored_only: false,
            exact: false,
            skip: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestFormat {
    Pretty,
    Terse,
    Json,
    Junit,
    Tap,
    Sarif,
}

impl TestFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "pretty" => Ok(Self::Pretty),
            "terse" => Ok(Self::Terse),
            "json" => Ok(Self::Json),
            "junit" | "junit-xml" => Ok(Self::Junit),
            "tap" => Ok(Self::Tap),
            "sarif" => Ok(Self::Sarif),
            other => Err(CliError::InvalidArgument(format!(
                "unknown format `{}` (expected: pretty | terse | json | junit | tap | sarif)",
                other
            ))),
        }
    }
}

pub fn execute(opts: TestOptions) -> Result<()> {
    let start = Instant::now();
    let quiet = matches!(
        opts.format,
        TestFormat::Json | TestFormat::Junit | TestFormat::Tap | TestFormat::Sarif
    );

    // Manifest + feature overrides (honour -Z test.*, [test].timeout_secs, ...)
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;
    crate::feature_overrides::apply_global(&mut manifest)?;

    let language_features = crate::feature_overrides::manifest_to_features(&manifest)?;

    if !quiet {
        ui::output("");
        ui::status(
            "Testing",
            &format!(
                "{} v{} ({})",
                manifest.cog.name.as_str(),
                manifest.cog.version,
                opts.tier.as_str(),
            ),
        );
        ui::output("");
    }

    // Discovery
    let test_files = find_test_files(&manifest_dir)?;
    if test_files.is_empty() {
        if !quiet {
            ui::warn("No test files found in tests/");
        }
        return Ok(());
    }

    let mut all: Vec<Test> = Vec::new();
    for f in &test_files {
        for t in discover_tests(f)? {
            all.push(t);
        }
    }

    // Filter: include / exact / skip
    let filtered: Vec<Test> = all
        .into_iter()
        .filter(|t| matches_filter(&t.name, &opts.filter, opts.exact))
        .filter(|t| !opts.skip.iter().any(|p| t.name.as_str().contains(p.as_str())))
        .collect();

    // Ignore resolution:
    //   --ignored       → only ignored
    //   --include-ignored → everything
    //   default          → skip ignored
    let active: Vec<&Test> = filtered
        .iter()
        .filter(|t| {
            if opts.ignored_only {
                t.ignored
            } else if opts.include_ignored {
                true
            } else {
                !t.ignored
            }
        })
        .collect();

    // --list: print and exit
    if opts.list {
        match opts.format {
            TestFormat::Json => {
                for t in &filtered {
                    println!(
                        "{}",
                        serde_json::json!({
                            "name": t.name.as_str(),
                            "file": t.file.display().to_string(),
                            "ignored": t.ignored,
                        })
                    );
                }
            }
            _ => {
                for t in &filtered {
                    let tag = if t.ignored { " (ignored)".dimmed() } else { "".normal() };
                    ui::output(&format!("{}{}", t.name, tag));
                }
                ui::output(&format!("\n{} tests", filtered.len()));
            }
        }
        return Ok(());
    }

    // Effective config for a single test-run
    let cfg = TestRunCfg {
        timeout_secs: manifest.test.timeout_secs,
        deny_warnings: manifest.test.deny_warnings,
        coverage: opts.coverage || manifest.test.coverage,
        nocapture: opts.nocapture,
        language_features,
        tier: opts.tier,
    };

    let total = filtered.len();
    let ignored_count = filtered.iter().filter(|t| t.ignored).count();

    if !quiet {
        ui::output(&format!(
            "running {} test{} (tier={}, parallel={})",
            active.len(),
            if active.len() == 1 { "" } else { "s" },
            opts.tier.as_str(),
            manifest.test.parallel,
        ));
    }

    let test_target_dir = manifest_dir.join("target").join("test");
    std::fs::create_dir_all(&test_target_dir).ok();

    // Thread pool: wire --test-threads so it actually takes effect.
    let pool: Option<rayon::ThreadPool> = if manifest.test.parallel {
        let n = opts.test_threads.unwrap_or_else(num_cpus::get).max(1);
        Some(
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .map_err(|e| CliError::Custom(format!("rayon: {}", e)))?,
        )
    } else {
        None
    };

    let run_one = |t: &Test| (t.name.clone(), run_single_test(t, &test_target_dir, &cfg));

    let results: Vec<(Text, TestResult)> = match &pool {
        Some(p) => p.install(|| active.par_iter().map(|t| run_one(t)).collect()),
        None => active.iter().map(|t| run_one(t)).collect(),
    };

    // Present each result in the chosen format
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failures: Vec<TestFailure> = Vec::new();
    for (name, result) in &results {
        let t = active.iter().find(|t| &t.name == name).unwrap();
        present_result(&opts, t, result, &mut passed, &mut failed, &mut failures);
    }
    // Non-active (only exists when we're NOT in --ignored-only mode):
    // their names should still appear once, marked ignored, when we
    // are in the default mode (skip ignored).
    if !opts.ignored_only && !opts.include_ignored {
        for t in filtered.iter().filter(|t| t.ignored) {
            match opts.format {
                TestFormat::Json => println!(
                    "{}",
                    serde_json::json!({
                        "event": "test",
                        "name": t.name.as_str(),
                        "outcome": "ignored",
                    })
                ),
                TestFormat::Terse => { /* keep dots-only output clean */ }
                TestFormat::Pretty => ui::output(&format!(
                    "test {} ... {}",
                    t.name,
                    "ignored".yellow()
                )),
                // Aggregate formats emit per-test entries at summary time.
                TestFormat::Junit | TestFormat::Tap | TestFormat::Sarif => {}
            }
        }
    }

    let total_duration = start.elapsed();

    // Pretty-print failures detail
    if !quiet && !failures.is_empty() {
        ui::output("");
        ui::output(&format!("{}", "failures:".bold()));
        ui::output("");
        for f in &failures {
            ui::output(&format!("  --- {} ---", f.name));
            if !f.error.is_empty() {
                ui::output(&format!("  {}", f.error));
            }
            for (label, body) in &[("stdout", &f.stdout), ("stderr", &f.stderr)] {
                if body.is_empty() {
                    continue;
                }
                ui::output(&format!("  {}:", label));
                for line in body.lines().take(20) {
                    ui::output(&format!("    {}", line));
                }
                let n = body.lines().count();
                if n > 20 {
                    ui::output(&format!("    ... ({} more lines)", n - 20));
                }
            }
            ui::output("");
        }
    }

    // Summary
    match opts.format {
        TestFormat::Junit => emit_junit(&results, &active, ignored_count, total_duration),
        TestFormat::Tap => emit_tap(&results, &active, ignored_count),
        TestFormat::Sarif => emit_sarif(&results, &active),
        TestFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "event": "summary",
                    "total": total,
                    "passed": passed,
                    "failed": failed,
                    "ignored": ignored_count,
                    "duration_ms": total_duration.as_millis() as u64,
                })
            );
        }
        TestFormat::Terse => {
            let verdict = if failed > 0 { "FAILED".red().bold() } else { "ok".green().bold() };
            ui::output(&format!(
                "\ntest result: {}. {} passed; {} failed; {} ignored; finished in {}",
                verdict,
                passed,
                failed,
                ignored_count,
                format_duration(total_duration),
            ));
        }
        TestFormat::Pretty => {
            ui::output("");
            let verdict = if failed > 0 { "FAILED".red().bold() } else { "ok".green().bold() };
            ui::output(&format!(
                "test result: {}. {} passed; {} failed; {} ignored; {} total; finished in {}",
                verdict,
                passed,
                failed,
                ignored_count,
                total,
                format_duration(total_duration),
            ));
            ui::output("");
        }
    }

    if cfg.coverage && !quiet {
        ui::output(&format!("{}", "coverage:".bold()));
        ui::output(&format!("  Functions instrumented: {}", total));
        ui::output(&format!(
            "  Coverage data written to {}/coverage/",
            test_target_dir.display()
        ));
        ui::output("  Use `llvm-cov report` to generate detailed reports");
    }

    if failed > 0 {
        Err(CliError::TestsFailed { passed, failed })
    } else {
        Ok(())
    }
}

// --------------------------------------------------------------------
// Filter helpers
// --------------------------------------------------------------------

fn matches_filter(name: &Text, filter: &Option<Text>, exact: bool) -> bool {
    match filter {
        None => true,
        Some(f) if exact => name.as_str() == f.as_str(),
        Some(f) => name.as_str().contains(f.as_str()),
    }
}

// --------------------------------------------------------------------
// Presentation
// --------------------------------------------------------------------

fn present_result(
    opts: &TestOptions,
    test: &Test,
    result: &TestResult,
    passed: &mut usize,
    failed: &mut usize,
    failures: &mut Vec<TestFailure>,
) {
    match opts.format {
        TestFormat::Json => present_json(test, result),
        TestFormat::Terse => present_terse(result),
        TestFormat::Pretty => present_pretty(test, result, opts.nocapture),
        // Aggregate formats don't emit per-test lines — everything goes
        // into one buffer emitted at summary time. We still have to
        // fall through so pass/fail counters and failure collection run.
        TestFormat::Junit | TestFormat::Tap | TestFormat::Sarif => {}
    }
    match result {
        TestResult::Pass { .. } => *passed += 1,
        TestResult::Fail { stdout, stderr, exit_code, error, .. } => {
            *failed += 1;
            failures.push(TestFailure {
                name: test.name.clone(),
                stdout: stdout.clone(),
                stderr: stderr.clone(),
                exit_code: *exit_code,
                error: error.clone(),
            });
        }
        TestResult::CompileError { error, .. } => {
            *failed += 1;
            failures.push(TestFailure {
                name: test.name.clone(),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                error: format!("compilation failed: {}", error),
            });
        }
    }
}

fn present_json(test: &Test, result: &TestResult) {
    let (outcome, duration, error): (&str, Duration, Option<&str>) = match result {
        TestResult::Pass { duration, .. } => ("ok", *duration, None),
        TestResult::Fail { duration, error, .. } => ("failed", *duration, Some(error.as_str())),
        TestResult::CompileError { duration, error } => ("compile-error", *duration, Some(error.as_str())),
    };
    let mut obj = serde_json::json!({
        "event": "test",
        "name": test.name.as_str(),
        "outcome": outcome,
        "duration_ms": duration.as_millis() as u64,
    });
    if let Some(e) = error {
        obj["error"] = serde_json::Value::String(e.to_string());
    }
    println!("{}", obj);
}

fn present_terse(result: &TestResult) {
    use std::io::Write;
    let dot = match result {
        TestResult::Pass { .. } => ".".green().to_string(),
        TestResult::Fail { .. } => "F".red().to_string(),
        TestResult::CompileError { .. } => "E".red().to_string(),
    };
    print!("{}", dot);
    let _ = std::io::stdout().flush();
}

fn present_pretty(test: &Test, result: &TestResult, nocapture: bool) {
    let (status, duration, stdout, stderr): (String, Duration, String, String) = match result {
        TestResult::Pass { duration, stdout, stderr } => (
            "ok".green().to_string(),
            *duration,
            stdout.clone(),
            stderr.clone(),
        ),
        TestResult::Fail { duration, stdout, stderr, .. } => (
            "FAILED".red().bold().to_string(),
            *duration,
            stdout.clone(),
            stderr.clone(),
        ),
        TestResult::CompileError { duration, .. } => (
            "FAILED".red().bold().to_string(),
            *duration,
            String::new(),
            String::new(),
        ),
    };
    ui::output(&format!(
        "test {} ... {} ({})",
        test.name,
        status,
        format_duration(duration)
    ));
    if nocapture {
        for body in [&stdout, &stderr] {
            if !body.is_empty() {
                for line in body.lines() {
                    ui::output(&format!("  {}", line));
                }
            }
        }
    }
}

// --------------------------------------------------------------------
// Execution model
// --------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestRunCfg {
    timeout_secs: u64,
    deny_warnings: bool,
    coverage: bool,
    nocapture: bool,
    language_features: verum_compiler::language_features::LanguageFeatures,
    tier: Tier,
}

enum TestResult {
    Pass {
        duration: Duration,
        stdout: String,
        stderr: String,
    },
    Fail {
        duration: Duration,
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
        error: String,
    },
    CompileError {
        duration: Duration,
        error: String,
    },
}

struct TestFailure {
    name: Text,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    error: String,
}

fn run_single_test(test: &Test, target_dir: &Path, cfg: &TestRunCfg) -> TestResult {
    if let Some(prop) = &test.property {
        return run_test_property(test, prop, cfg);
    }
    match cfg.tier {
        Tier::Aot => run_test_aot(test, target_dir, cfg),
        Tier::Interpret => run_test_interpret(test, cfg),
    }
}

/// Property-test path: compile once via VBC, loop the PBT runner.
/// Routes through the interpreter irrespective of --tier because the
/// property runner needs per-iteration Value construction that the
/// native-binary path can't do (each sample would require respawning).
fn run_test_property(
    test: &Test,
    prop: &crate::commands::property::PropertyFunc,
    _cfg: &TestRunCfg,
) -> TestResult {
    use crate::commands::property::{
        load_regression_db, record_regression, run_property, save_regression_db,
        seeds_for, RunnerConfig, Seed,
    };
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

    let start = Instant::now();

    // Compile file → VBC (same shape as run_test_interpret).
    let source = match std::fs::read_to_string(&test.file) {
        Ok(s) => s,
        Err(e) => {
            return TestResult::CompileError {
                duration: start.elapsed(),
                error: format!("read: {}", e),
            };
        }
    };
    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let lexer = Lexer::new(&source, file_id);
    let ast = match parser.parse_module(lexer, file_id) {
        Ok(m) => m,
        Err(errs) => {
            let joined = errs
                .iter()
                .map(|e| format!("{}", e))
                .collect::<Vec<_>>()
                .join("\n");
            return TestResult::CompileError {
                duration: start.elapsed(),
                error: format!("parse: {}", joined),
            };
        }
    };
    let config = CodegenConfig::new("test");
    let mut codegen = VbcCodegen::with_config(config);
    let module = match codegen.compile_module(&ast) {
        Ok(m) => m,
        Err(e) => {
            return TestResult::CompileError {
                duration: start.elapsed(),
                error: format!("codegen: {:?}", e),
            };
        }
    };
    let module = std::sync::Arc::new(module);

    // Replay regression DB seeds first, then draw fresh ones.
    let mut db = load_regression_db();
    let replay_seeds = seeds_for(&db, test.name.as_str());

    let default_runs = prop.runs_override.unwrap_or(100);
    let pinned = prop.seed_override;

    // Replay pass (one run each, pinned seed). If a stored seed now
    // PASSES, the bug it originally captured has been fixed — drop
    // it from the DB so the regression set always reflects current
    // failures. Matches Hypothesis's "database pruning" behaviour.
    let mut pruned_hex: Vec<String> = Vec::new();
    for s in &replay_seeds {
        let cfg = RunnerConfig {
            runs: 1,
            max_shrinks: 500,
            seed: *s,
            pinned_seed: true,
        };
        let outcome = run_property(&module, prop, &cfg);
        if let Some(f) = outcome.failure {
            return TestResult::Fail {
                duration: start.elapsed(),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                error: format!(
                    "regression replay seed {} still fails: shrunk=({}) :: {}",
                    f.seed.to_hex(),
                    f.shrunk_inputs.join(", "),
                    f.message
                ),
            };
        } else {
            pruned_hex.push(s.to_hex());
        }
    }
    if !pruned_hex.is_empty() {
        let name = test.name.as_str().to_string();
        db.entries.retain(|e| !(e.test == name && pruned_hex.contains(&e.seed)));
        let _ = save_regression_db(&db);
    }

    // Fresh-sample pass. Seed picked from wall time if not pinned by the
    // @property(seed = 0x...) override.
    let seed = pinned.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        Seed(nanos ^ 0x9E37_79B9_7F4A_7C15)
    });
    let cfg = RunnerConfig {
        runs: default_runs,
        max_shrinks: 500,
        seed,
        pinned_seed: pinned.is_some(),
    };
    let outcome = run_property(&module, prop, &cfg);
    if let Some(f) = outcome.failure {
        let msg = format!(
            "property failed after {} iterations\n  seed: {}\n  original: ({})\n  shrunk: ({}) [{} shrink steps]\n  error: {}\n  replay: verum test --filter '{}' -Z test.property_seed={}",
            outcome.iterations,
            f.seed.to_hex(),
            f.original_inputs.join(", "),
            f.shrunk_inputs.join(", "),
            f.shrink_steps,
            f.message,
            test.name,
            f.seed.to_hex(),
        );
        // Persist failing seed so future runs replay it first.
        record_regression(
            &mut db,
            test.name.as_str(),
            f.seed,
            &format!("({})", f.shrunk_inputs.join(", ")),
        );
        let _ = save_regression_db(&db);
        return TestResult::Fail {
            duration: start.elapsed(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            error: msg,
        };
    }

    TestResult::Pass {
        duration: start.elapsed(),
        stdout: format!("{} iterations ok", outcome.iterations),
        stderr: String::new(),
    }
}

fn run_test_aot(test: &Test, target_dir: &Path, cfg: &TestRunCfg) -> TestResult {
    let start = Instant::now();

    let stem = test.file.file_stem().and_then(|s| s.to_str()).unwrap_or("test");
    let binary_name = format!("test_{}", stem);
    let output_path = target_dir.join(&binary_name);

    let mut lint_config = verum_compiler::lint::LintConfig::default();
    if cfg.deny_warnings {
        lint_config.deny_warnings = true;
    }
    let options = CompilerOptions {
        input: test.file.clone(),
        output: output_path.clone(),
        verify_mode: VerifyMode::Runtime,
        output_format: OutputFormat::Human,
        coverage: cfg.coverage,
        lint_config,
        language_features: cfg.language_features.clone(),
        ..Default::default()
    };
    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let executable = match pipeline.run_native_compilation() {
        Ok(exe) => exe,
        Err(e) => {
            return TestResult::CompileError {
                duration: start.elapsed(),
                error: e.to_string(),
            };
        }
    };

    let child = Command::new(&executable)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return TestResult::Fail {
                duration: start.elapsed(),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                error: format!("spawn failure: {}", e),
            };
        }
    };

    let output = if cfg.timeout_secs == 0 {
        child.wait_with_output()
    } else {
        let deadline = Instant::now() + Duration::from_secs(cfg.timeout_secs);
        let poll = Duration::from_millis(25);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break child.wait_with_output(),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        return TestResult::Fail {
                            duration: start.elapsed(),
                            stdout: String::new(),
                            stderr: String::new(),
                            exit_code: None,
                            error: format!(
                                "timed out after {}s",
                                cfg.timeout_secs
                            ),
                        };
                    }
                    std::thread::sleep(poll);
                }
                Err(e) => {
                    return TestResult::Fail {
                        duration: start.elapsed(),
                        stdout: String::new(),
                        stderr: String::new(),
                        exit_code: None,
                        error: format!("poll failure: {}", e),
                    };
                }
            }
        }
    };
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            return TestResult::Fail {
                duration: start.elapsed(),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                error: format!("wait failure: {}", e),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let duration = start.elapsed();
    if output.status.success() {
        TestResult::Pass { duration, stdout, stderr }
    } else {
        let code = output.status.code();
        let error = code
            .map(|c| format!("process exited with code {}", c))
            .unwrap_or_else(|| "process terminated by signal".to_string());
        TestResult::Fail { duration, stdout, stderr, exit_code: code, error }
    }
}

fn run_test_interpret(test: &Test, _cfg: &TestRunCfg) -> TestResult {
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    use verum_vbc::interpreter::Interpreter;

    let start = Instant::now();

    let source = match std::fs::read_to_string(&test.file) {
        Ok(s) => s,
        Err(e) => {
            return TestResult::CompileError {
                duration: start.elapsed(),
                error: format!("read: {}", e),
            };
        }
    };

    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let lexer = Lexer::new(&source, file_id);
    let ast = match parser.parse_module(lexer, file_id) {
        Ok(m) => m,
        Err(errs) => {
            let joined = errs.iter().map(|e| format!("{}", e)).collect::<Vec<_>>().join("\n");
            return TestResult::CompileError {
                duration: start.elapsed(),
                error: format!("parse: {}", joined),
            };
        }
    };

    let config = CodegenConfig::new("test");
    let mut codegen = VbcCodegen::with_config(config);
    let module = match codegen.compile_module(&ast) {
        Ok(m) => m,
        Err(e) => {
            return TestResult::CompileError {
                duration: start.elapsed(),
                error: format!("codegen: {:?}", e),
            };
        }
    };
    let module = Arc::new(module);

    // Pick the function to run. Priority:
    //   1. Function whose name matches the test name (for per-@test tests)
    //   2. `main`
    let fn_name_tail: &str = if let Some(fn_name) = &test.fn_name {
        fn_name.as_str()
    } else {
        test.name
            .as_str()
            .rsplit_once("::")
            .map(|(_, n)| n)
            .unwrap_or_else(|| test.name.as_str())
    };
    let fid_opt = module
        .functions
        .iter()
        .find(|vf| module.get_string(vf.name) == Some(fn_name_tail))
        .or_else(|| module.functions.iter().find(|vf| module.get_string(vf.name) == Some("main")))
        .map(|vf| vf.id);
    let fid = match fid_opt {
        Some(id) => id,
        None => {
            return TestResult::CompileError {
                duration: start.elapsed(),
                error: format!("test entry point `{}` or `main` not found", fn_name_tail),
            };
        }
    };

    let mut interp = Interpreter::new(module);
    // Disable tier-0 safety caps — test runners frequently push past
    // the 100M instruction / 30s timeout defaults, especially for
    // @property-style tests that iterate internally.
    interp.state.config.max_instructions = 0;
    interp.state.config.timeout_ms = 0;

    let outcome = if let Some(args) = &test.case_args {
        // @test_case path: convert literal args → VBC Values, call directly.
        let vbc_args: std::result::Result<Vec<_>, _> = args
            .iter()
            .map(|tv| tv.to_vbc_value(&mut interp))
            .collect();
        match vbc_args {
            Ok(vs) => crate::commands::property::call_parametrised(&mut interp, fid, &vs),
            Err(e) => {
                return TestResult::CompileError {
                    duration: start.elapsed(),
                    error: format!("encode @test_case args: {}", e),
                };
            }
        }
    } else {
        interp.execute_function(fid)
    };
    let duration = start.elapsed();
    let stdout = interp.state.get_stdout().to_string();
    match outcome {
        Ok(v) => {
            let exit = if v.is_int() { v.as_i64() as i32 } else { 0 };
            if exit == 0 {
                TestResult::Pass { duration, stdout, stderr: String::new() }
            } else {
                TestResult::Fail {
                    duration,
                    stdout,
                    stderr: String::new(),
                    exit_code: Some(exit),
                    error: format!("exit code {}", exit),
                }
            }
        }
        Err(e) => TestResult::Fail {
            duration,
            stdout,
            stderr: String::new(),
            exit_code: None,
            error: format!("runtime: {:?}", e),
        },
    }
}

// --------------------------------------------------------------------
// Discovery
// --------------------------------------------------------------------

struct Test {
    name: Text,
    file: PathBuf,
    ignored: bool,
    /// When Some, this is a property-based test — the runner generates
    /// random inputs for each parameter and calls the function N times.
    property: Option<crate::commands::property::PropertyFunc>,
    /// When Some, this test was expanded from a @test_case(args...)
    /// attribute — the runner should call the function with these
    /// literal args instead of no-args. The original fn name (without
    /// the `[N]` suffix) is used to resolve the VBC FunctionId.
    case_args: Option<Vec<crate::commands::property::TreeValue>>,
    /// Underlying fn name (without `[N]` suffix) — needed for @test_case
    /// expansions to still find their target in the compiled VBC module.
    fn_name: Option<String>,
}

fn discover_tests(file: &Path) -> Result<List<Test>> {
    let source = std::fs::read_to_string(file)?;

    // AST-based first: parse the file and look for @test / fn main().
    let file_id = FileId::new(0);
    let lexer = Lexer::new(&source, file_id);
    let parser = VerumParser::new();

    if let Ok(module) = parser.parse_module(lexer, file_id) {
        let mut tests = List::new();
        let module_name = file.file_stem().unwrap().to_str().unwrap();
        let has_test_attrs = module.items.iter().any(|item| {
            matches!(&item.kind, ItemKind::Function(f) if f.attributes.iter().any(|a| a.name.as_str() == "test"))
        });
        // Pass 1: property-based tests (@property).
        let property_funcs = crate::commands::property::discover_properties_in_module(
            &module, module_name, file,
        );
        let has_property_attrs = !property_funcs.is_empty();

        if has_test_attrs || has_property_attrs {
            for item in &module.items {
                if let ItemKind::Function(func) = &item.kind {
                    let is_test = func.attributes.iter().any(|a| a.name.as_str() == "test");
                    let is_property = func.attributes.iter().any(|a| a.name.as_str() == "property");
                    if !is_test && !is_property {
                        continue;
                    }
                    let is_ignored = func.attributes.iter().any(|a| {
                        a.name.as_str() == "ignore" || a.name.as_str() == "ignored"
                    });
                    let property = if is_property {
                        property_funcs
                            .iter()
                            .find(|p| p.name == func.name.as_str())
                            .cloned()
                    } else {
                        None
                    };
                    // @test_case expansion: one Test per invocation.
                    let cases = parse_test_cases(&func.attributes);
                    if !cases.is_empty() {
                        for (idx, args) in cases.into_iter().enumerate() {
                            tests.push(Test {
                                name: format!("{}::{}[{}]", module_name, func.name, idx).into(),
                                file: file.to_path_buf(),
                                ignored: is_ignored,
                                property: property.clone(),
                                case_args: Some(args),
                                fn_name: Some(func.name.to_string()),
                            });
                        }
                    } else {
                        tests.push(Test {
                            name: format!("{}::{}", module_name, func.name).into(),
                            file: file.to_path_buf(),
                            ignored: is_ignored,
                            property,
                            case_args: None,
                            fn_name: Some(func.name.to_string()),
                        });
                    }
                }
            }
        } else {
            // Whole-file test — must have main()
            let has_main = module.items.iter().any(|item| {
                matches!(&item.kind, ItemKind::Function(f) if f.name.as_str() == "main")
            });
            if has_main {
                let is_ignored = source.lines().take(10).any(|l| {
                    let t = l.trim();
                    t.contains("@ignore") || t.contains("@ignored")
                });
                tests.push(Test {
                    name: module_name.into(),
                    file: file.to_path_buf(),
                    ignored: is_ignored,
                    property: None,
                    case_args: None,
                    fn_name: None,
                });
            }
        }
        return Ok(tests);
    }

    // Fallback: text scan (preserves legacy behaviour when the parser
    // can't handle the file — e.g. it uses an extension still WIP).
    let mut tests = List::new();
    for (i, line) in source.lines().enumerate() {
        let l = line.trim();
        if l.starts_with("@test") || l.starts_with("#[test]") {
            if let Some(next) = source.lines().nth(i + 1) {
                if let Some(name) = extract_fn_name(next) {
                    let ignored = l.contains("ignore");
                    tests.push(Test {
                        name: format!(
                            "{}::{}",
                            file.file_stem().unwrap().to_str().unwrap(),
                            name
                        )
                        .into(),
                        file: file.to_path_buf(),
                        ignored,
                        property: None,
                        case_args: None,
                        fn_name: None,
                    });
                }
            }
        }
    }
    if tests.is_empty() {
        let has_main = source.lines().any(|l| {
            let t = l.trim();
            t.starts_with("fn main(") || t.starts_with("async fn main(")
        });
        if has_main {
            let module_name = file.file_stem().unwrap().to_str().unwrap();
            tests.push(Test {
                name: module_name.into(),
                file: file.to_path_buf(),
                ignored: false,
                property: None,
                case_args: None,
                fn_name: None,
            });
        }
    }
    Ok(tests)
}

fn extract_fn_name(line: &str) -> Option<Text> {
    let t = line.trim();
    for pref in ["public fn ", "pub fn ", "private fn ", "fn "] {
        if let Some(rest) = t.strip_prefix(pref) {
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            if end > 0 {
                return Some(Text::from(&rest[..end]));
            }
        }
    }
    None
}

fn find_test_files(project_dir: &Path) -> Result<List<PathBuf>> {
    let tests_dir = project_dir.join("tests");
    if !tests_dir.exists() {
        return Ok(List::new());
    }
    let mut files = List::new();
    for entry in walkdir::WalkDir::new(tests_dir).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("vr") {
            files.push(path.to_path_buf());
        }
    }
    files.sort();
    Ok(files)
}

// --------------------------------------------------------------------
// Formatting helpers
// --------------------------------------------------------------------

fn format_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        format!("{:.2}ms", ms)
    } else if ms < 1000.0 {
        format!("{:.0}ms", ms)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}



/// Parse `@test_case(arg, arg, ...)` attributes on a function into a
/// list of argument vectors ready for call_with_args. Returns empty
/// vec if no @test_case attributes are present.
///
/// Supported argument literals: Int, Bool, Text, Float. Anything else
/// is silently dropped — keeps the attribute surface simple and avoids
/// inventing type coercions at discover time.
fn parse_test_cases(
    attrs: &[verum_ast::Attribute],
) -> Vec<Vec<crate::commands::property::TreeValue>> {
    use crate::commands::property::TreeValue;
    use verum_ast::ExprKind;
    let mut cases = Vec::new();
    for a in attrs {
        if a.name.as_str() != "test_case" {
            continue;
        }
        let args = match &a.args {
            verum_common::Maybe::Some(a) => a,
            _ => continue,
        };
        let mut case: Vec<TreeValue> = Vec::new();
        for e in args.iter() {
            if let Some(tv) = expr_to_tree_value(e) {
                case.push(tv);
            }
        }
        if !case.is_empty() {
            cases.push(case);
        }
    }
    cases
}

fn expr_to_tree_value(e: &verum_ast::Expr) -> Option<crate::commands::property::TreeValue> {
    use crate::commands::property::TreeValue;
    use verum_ast::{ExprKind, LiteralKind, UnOp};
    match &e.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Int(il) => Some(TreeValue::Int {
                value: il.value as i64,
                lo: i64::MIN,
                hi: i64::MAX,
            }),
            LiteralKind::Bool(b) => Some(TreeValue::Bool(*b)),
            LiteralKind::Float(fl) => Some(TreeValue::Float(fl.value)),
            LiteralKind::Text(s) => Some(TreeValue::Text {
                value: s.to_string(),
                max_len: u32::MAX,
            }),
            _ => None,
        },
        ExprKind::Unary { op: UnOp::Neg, expr: inner } => {
            if let ExprKind::Literal(lit) = &inner.kind {
                match &lit.kind {
                    LiteralKind::Int(il) => Some(TreeValue::Int {
                        value: -(il.value as i64),
                        lo: i64::MIN,
                        hi: i64::MAX,
                    }),
                    LiteralKind::Float(fl) => Some(TreeValue::Float(-fl.value)),
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}


// ----------------------------------------------------------------
// Aggregate CI-output emitters (JUnit XML / TAP v13 / SARIF 2.1.0)
// ----------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn emit_junit(
    results: &[(Text, TestResult)],
    active: &[&Test],
    ignored: usize,
    total: Duration,
) {
    let n = results.len() + ignored;
    let failures = results
        .iter()
        .filter(|(_, r)| !matches!(r, TestResult::Pass { .. }))
        .count();
    println!(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    println!(
        r#"<testsuites tests="{}" failures="{}" time="{:.3}">"#,
        n,
        failures,
        total.as_secs_f64()
    );
    println!(
        r#"  <testsuite name="verum" tests="{}" failures="{}" skipped="{}" time="{:.3}">"#,
        n,
        failures,
        ignored,
        total.as_secs_f64()
    );
    for (name, r) in results {
        let (elapsed, ok, err, kind) = match r {
            TestResult::Pass { duration, .. } => (*duration, true, String::new(), ""),
            TestResult::Fail { duration, error, .. } => (*duration, false, error.clone(), "failure"),
            TestResult::CompileError { duration, error } => {
                (*duration, false, error.clone(), "error")
            }
        };
        let _ = active; // silence unused warning when active goes away
        println!(
            r#"    <testcase classname="verum" name="{}" time="{:.3}">"#,
            xml_escape(name.as_str()),
            elapsed.as_secs_f64()
        );
        if !ok {
            println!(
                r#"      <{} message="{}"><![CDATA[{}]]></{}>"#,
                kind,
                xml_escape(&err.lines().next().unwrap_or("")),
                err.replace("]]>", "]]]]><![CDATA[>"),
                kind,
            );
        }
        println!("    </testcase>");
    }
    println!("  </testsuite>");
    println!("</testsuites>");
}

fn emit_tap(results: &[(Text, TestResult)], _active: &[&Test], ignored: usize) {
    let n = results.len() + ignored;
    println!("TAP version 13");
    println!("1..{}", n);
    let mut i: usize = 1;
    for (name, r) in results {
        match r {
            TestResult::Pass { duration, .. } => println!(
                "ok {} - {} # time={:.3}s",
                i,
                name,
                duration.as_secs_f64()
            ),
            TestResult::Fail { duration, error, .. } => {
                println!("not ok {} - {} # time={:.3}s", i, name, duration.as_secs_f64());
                println!("  ---");
                for line in error.lines() {
                    println!("  message: {}", line);
                }
                println!("  ...");
            }
            TestResult::CompileError { duration, error } => {
                println!(
                    "not ok {} - {} # time={:.3}s (compile-error)",
                    i,
                    name,
                    duration.as_secs_f64()
                );
                println!("  ---");
                for line in error.lines() {
                    println!("  message: {}", line);
                }
                println!("  ...");
            }
        }
        i += 1;
    }
}

fn emit_sarif(results: &[(Text, TestResult)], _active: &[&Test]) {
    let rules = serde_json::json!([{
        "id": "verum-test",
        "name": "VerumTestFailure",
        "shortDescription": {"text": "A Verum test failed"},
        "fullDescription": {"text": "Emitted by `verum test` for each failing test."},
        "defaultConfiguration": {"level": "error"},
    }]);
    let mut sarif_results = Vec::new();
    for (name, r) in results {
        let (ok, msg): (bool, String) = match r {
            TestResult::Pass { .. } => (true, String::new()),
            TestResult::Fail { error, .. } => (false, error.clone()),
            TestResult::CompileError { error, .. } => (false, error.clone()),
        };
        if ok {
            continue;
        }
        sarif_results.push(serde_json::json!({
            "ruleId": "verum-test",
            "level": "error",
            "message": {"text": msg},
            "locations": [{
                "logicalLocations": [{"name": name.as_str()}],
            }],
        }));
    }
    let doc = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "verum",
                    "informationUri": "https://verum-lang.dev",
                    "rules": rules,
                }
            },
            "results": sarif_results,
        }],
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_substring() {
        let f: Option<Text> = Some("foo".into());
        assert!(matches_filter(&"abc_foo_xyz".into(), &f, false));
        assert!(!matches_filter(&"abc".into(), &f, false));
    }

    #[test]
    fn filter_exact() {
        let f: Option<Text> = Some("abc".into());
        assert!(matches_filter(&"abc".into(), &f, true));
        assert!(!matches_filter(&"abcd".into(), &f, true));
    }

    #[test]
    fn format_parse_accepts_known() {
        assert_eq!(TestFormat::parse("pretty").unwrap(), TestFormat::Pretty);
        assert_eq!(TestFormat::parse("terse").unwrap(), TestFormat::Terse);
        assert_eq!(TestFormat::parse("json").unwrap(), TestFormat::Json);
        assert!(TestFormat::parse("xml").is_err());
    }
}
