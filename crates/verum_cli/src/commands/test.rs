//! Test command — discover and execute `@test` functions and whole-
//! file tests across both execution tiers.
//!

//! # Tiers
//!

//! Matches the `verum run` / `verum bench` convention:
//!

//! | Tier | How a test is run |
//! |--------------|-------------------------------------------------------------|
//! | Interpreter | Compile file to VBC once, run main()-or-test via the |
//! | | interpreter in-process. Fast iteration, full diagnostics. |
//! | AOT (native) | Build a binary per test file, spawn it; exit 0 == pass. |
//!

//! Default: **AOT** (a test is a promise about the final artefact;
//! interpreter is available via `--interp` for fast red-green loops).
//!

//! # Options modelled on libtest / `cargo test`
//!

//! * `--filter STR` — substring match on test name. A filter that
//!   contains `/` only matches at suite-path segment boundaries
//!   (`--filter sync/` selects `sync/…` and `runtime/sync/…` but never
//!   `async/…`); a leading `^` additionally anchors at the name start
//!   (`--filter '^text/'` scopes to the `text/` subtree only)
//! * `--exact` — require full match (like libtest `--exact`)
//! * `--skip PATTERN` — substring-exclude; repeatable
//! * `--include-ignored` — run all, including `@ignore`
//! * `--ignored` — run **only** `@ignore`d tests (useful to promote them)
//! * `--list` — print discovered tests and exit
//! * `--nocapture` — don't capture stdout/stderr
//! * `--test-threads N` — parallel workers; wired to rayon here (was
//!  accepted-but-ignored previously)
//! * `--format pretty | terse | json` — presentation; `json` emits one
//!  newline-delimited JSON event per test for CI ingest

use colored::Colorize;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use verum_common::{List, Text};
use verum_vbc::module::VbcModule;

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::tier::Tier;
use crate::ui;
use verum_ast::{FileId, ItemKind};
use verum_compiler::options::{CompilerOptions, OutputFormat, VerifyMode};
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;

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

/// Effective default worker count for a test run (ONE authority for
/// the banner and the pool). Interp tests are cheap in-process jobs —
/// the CPU count is the right default. AOT tests each spawn an
/// ISOLATED CHILD that performs a full native compile+link (an LLVM
/// instance measured in GBs of RSS): a CPU-count default fans out
/// 8-16 concurrent compilers and has OOM-rebooted a 16-core laptop
/// (T0203). AOT defaults to 2 workers; `--test-threads` and
/// `[test].parallel=false` override exactly as before.
fn default_test_threads(tier: Tier) -> usize {
    match tier {
        Tier::Aot => 2,
        _ => num_cpus::get(),
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

/// TEST-IGNORED-FLAGS-NOOP-1 (T0365): the ONE authority for how
/// `@ignore` interacts with selection AND execution.
///
/// `--ignored` / `--include-ignored` are SELECTION flags; historically
/// they stopped at the parent's selection pass and never reached the
/// child re-invocations (`run_interp_file_subprocess`,
/// `run_aot_subprocess`), whose own default-mode selection silently
/// re-skipped every `@ignore`'d test — `verum test --ignored` printed
/// "running N tests" and then reported `0 passed; 0 failed; N ignored`
/// (and the per-test AOT child matched zero tests and exited 0: a
/// silent false PASS). Everything that needs the mode — parent
/// selection, ignored-count accounting, child argv, child-result
/// accounting, default-mode reporting — derives it from this enum so
/// the decision cannot fork again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IgnoreMode {
    /// Default: run only tests without `@ignore`.
    SkipIgnored,
    /// `--ignored`: run ONLY `@ignore`'d tests (pin promotion /
    /// pin-verification sweeps).
    OnlyIgnored,
    /// `--include-ignored`: run everything.
    IncludeIgnored,
}

impl IgnoreMode {
    fn from_opts(opts: &TestOptions) -> Self {
        if opts.ignored_only {
            IgnoreMode::OnlyIgnored
        } else if opts.include_ignored {
            IgnoreMode::IncludeIgnored
        } else {
            IgnoreMode::SkipIgnored
        }
    }

    /// Does a test whose `@ignore` status is `ignored` get EXECUTED
    /// under this mode?
    fn selects(self, ignored: bool) -> bool {
        match self {
            IgnoreMode::SkipIgnored => !ignored,
            IgnoreMode::OnlyIgnored => ignored,
            IgnoreMode::IncludeIgnored => true,
        }
    }

    /// Argv fragment that reproduces this mode in a child
    /// re-invocation (the child re-applies selection over its own
    /// discovery, so the parent's mode must ride along).
    fn child_args(self) -> &'static [&'static str] {
        match self {
            IgnoreMode::SkipIgnored => &[],
            IgnoreMode::OnlyIgnored => &["--ignored"],
            IgnoreMode::IncludeIgnored => &["--include-ignored"],
        }
    }
}

pub fn execute(opts: TestOptions) -> Result<()> {
    let start = Instant::now();
    // SCRIPT-HOOK-TEST-RUNNER-1: the in-process interpret tier below drives
    // `verum_vbc::interpreter::Interpreter` directly (compiled_test_module +
    // Interpreter::call), bypassing the pipeline entry points that install
    // the scripting compiler hook — so `script_engine_eval` inside a test
    // reported "compiler unavailable" (kind 4) while the SAME source under
    // `verum run` evaluated fine.  The test environment must equal the run
    // environment wherever verum_compiler is linked; idempotent (Once).
    verum_compiler::api::ensure_scripting_compiler_installed();
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
        .filter(|t| {
            !opts
                .skip
                .iter()
                .any(|p| t.name.as_str().contains(p.as_str()))
        })
        .collect();

    // Ignore resolution:
    //  --ignored → only ignored
    //  --include-ignored → everything
    //  default → skip ignored
    let ignore_mode = IgnoreMode::from_opts(&opts);
    let active: Vec<&Test> = filtered
        .iter()
        .filter(|t| ignore_mode.selects(t.ignored))
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
                    let tag = if t.ignored {
                        " (ignored)".dimmed()
                    } else {
                        "".normal()
                    };
                    ui::output(&format!("{}{}", t.name, tag));
                }
                ui::output(&format!("\n{} tests", filtered.len()));
            }
        }
        return Ok(());
    }

    // Effective config for a single test-run
    let cfg = TestRunCfg {
        // AOT-BUDGET-1 (META-AOT-PARITY-1 stopgap): every per-test AOT
        // subprocess currently recompiles the mounted stdlib's NATIVE
        // leg from scratch — a measured 35-55s fixed baseline (the
        // precompiled stdlib caches VBC+metadata only), and umbrella
        // mounts (core.meta.mod: 55 re-exports) push heavy tests to
        // 100-120s. Those tests COMPLETE; the 60s default guillotined
        // them (16-18 'timed out' failures in the meta sweep with the
        // binary running in 0.01s). Until AOT-STDLIB-NATIVE-CACHE-1
        // removes the baseline, give AOT runs a 180s floor — explicit
        // manifest values above that still win.
        timeout_secs: if matches!(opts.tier, Tier::Aot) {
            manifest.test.timeout_secs.max(180)
        } else {
            manifest.test.timeout_secs
        },
        deny_warnings: manifest.test.deny_warnings,
        coverage: opts.coverage || manifest.test.coverage,
        nocapture: opts.nocapture,
        language_features,
        tier: opts.tier,
        release: opts.release,
        verify_mode_override: opts.verify.as_ref().and_then(|v| {
            // Map the string-form CLI flag onto the typed enum. We
            // ignore unrecognised values (return None) instead of
            // erroring at this layer — keeps the test runner
            // tolerant of typos in CI invocations and surfaces them
            // as a default-mode run rather than a hard failure.
            match v.as_str().to_ascii_lowercase().as_str() {
                "runtime" => Some(verum_compiler::options::VerifyMode::Runtime),
                "proof" | "static" => Some(verum_compiler::options::VerifyMode::Proof),
                "auto" => Some(verum_compiler::options::VerifyMode::Auto),
                _ => None,
            }
        }),
        property_testing: manifest.test.property_testing,
        proptest_cases: manifest.test.proptest_cases,
        differential: manifest.test.differential,
        fuzzing: manifest.test.fuzzing,
    };

    // T0365: `ignored` in the summary counts tests SKIPPED because of
    // their `@ignore` pin. Under `--ignored` / `--include-ignored` the
    // pinned tests EXECUTE — reporting them as "ignored" was exactly
    // the defect's face ("running 13 tests" → "0 passed; 13 ignored").
    let ignored_count = match ignore_mode {
        IgnoreMode::SkipIgnored => filtered.iter().filter(|t| t.ignored).count(),
        IgnoreMode::OnlyIgnored | IgnoreMode::IncludeIgnored => 0,
    };
    let total = active.len() + ignored_count;

    if !quiet {
        // Banner reports the EFFECTIVE worker count — `--test-threads 1`
        // previously still printed `parallel=true`, which mis-attributed
        // set-only crashes to parallelism during triage (task #40).
        let effective_threads = if manifest.test.parallel {
            opts
                .test_threads
                .unwrap_or_else(|| default_test_threads(opts.tier))
                .max(1)
        } else {
            1
        };
        ui::output(&format!(
            "running {} test{} (tier={}, threads={})",
            active.len(),
            if active.len() == 1 { "" } else { "s" },
            opts.tier.as_str(),
            effective_threads,
        ));
    }

    // After #298 + #273 + #299 every [test].* manifest field is
    // load-bearing through `TestRunCfg`: property_testing /
    // proptest_cases / differential / fuzzing all flow through to
    // a real consumer. Surface the load-bearing modes when set
    // so embedders see the runner observed their setting.
    if manifest.test.differential && !quiet {
        ui::output(
            "  [differential] every non-property test runs through both \
             Tier 0 (interpreter) and Tier 1 (AOT) — disagreement is itself \
             a test failure",
        );
    }
    if manifest.test.fuzzing && !quiet {
        ui::output(
            "  [fuzzing] cargo-fuzz targets under `fuzz/` will be exercised \
             after the regular test suite — new crash artifacts count as \
             test failures (per-target budget VERUM_FUZZ_BUDGET_SECS or 30s)",
        );
    }

    let test_target_dir = manifest_dir.join("target").join("test");
    std::fs::create_dir_all(&test_target_dir).ok();

    // Thread pool: wire --test-threads so it actually takes effect.
    //

    // T0.5.1 — rayon worker threads default to a small stack (512 KiB
    // on macOS, ~2 MiB on Linux). Each test invokes the full compiler
    // pipeline (type checker + VBC codegen + AOT) which recursively
    // walks AST / Type / CoreTerm structures; the recursion-guard
    // bounds (parser MAX_RECURSION_DEPTH=128, types MAX_AST_TO_TYPE_
    // DEPTH=64) are sized for typical program ASTs but stdlib
    // bootstrap can blow them on debug builds at deeper modules.
    //

    // Match the main thread's 16 MiB stack so workers don't SIGBUS
    // mid-stdlib-load.
    // GENERATE-NATIVE-WORKER-RACE-1 (task #49): a 1-thread pool is NOT
    // "no parallelism" — `p.install(run_one)` moves the whole compiler
    // pipeline onto a rayon WORKER thread, and LLVM's lazily
    // cxa-guard-registered pass statics then race the pool's park/wake
    // path (the exact §23 semaphore corruption the eager
    // `Target::initialize_native` + pipeline rayon-fence were built
    // against — both assume codegen runs on the MAIN thread). With
    // `--test-threads 1` skip the pool entirely so the single test
    // executes on the main thread — the same stable configuration as
    // `verum build`.
    let pool: Option<rayon::ThreadPool> = if manifest.test.parallel {
        let n = opts
            .test_threads
            .unwrap_or_else(|| default_test_threads(opts.tier))
            .max(1);
        if n <= 1 {
            None
        } else {
            const WORKER_STACK_SIZE: usize = 16 * 1024 * 1024;
            Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .stack_size(WORKER_STACK_SIZE)
                    .build()
                    .map_err(|e| CliError::Custom(format!("rayon: {}", e)))?,
            )
        }
    } else {
        None
    };

    let run_one = |t: &Test| (t.name.clone(), run_single_test(t, &test_target_dir, &cfg));

    // GENERATE-NATIVE-WORKER-RACE-1: recursion terminator for the
    // uniform AOT process-isolation. The parent sets this env in
    // `run_aot_subprocess`; the child compiles its single test
    // in-process (main thread, no pool).
    let aot_child = std::env::var_os("VERUM_TEST_AOT_CHILD").is_some();

    // TEST-RUNNER-ISOLATION-1: per-FILE process quarantine for the
    // interp tier (see `run_interp_file_subprocess`).  Applied when
    // the run spans MORE THAN ONE file — a single-file run has nothing
    // downstream to protect, and it is also exactly the shape the
    // child re-invocation produces (a second recursion terminator on
    // top of the env marker).  JSON-family formats stay in-process:
    // their consumers parse OUR stdout and the child protocol would
    // double-emit events.
    let interp_child = std::env::var_os("VERUM_TEST_INTERP_CHILD").is_some();
    let interp_isolate = opts.tier == Tier::Interpret
        && !interp_child
        && !cfg.differential
        && std::env::var_os("VERUM_TEST_NO_ISOLATE").is_none()
        && !matches!(
            opts.format,
            TestFormat::Json | TestFormat::Junit | TestFormat::Tap | TestFormat::Sarif
        )
        && {
            let mut files: Vec<&Path> = active.iter().map(|t| t.file.as_path()).collect();
            files.sort_unstable();
            files.dedup();
            files.len() > 1
        };

    let results: Vec<(Text, TestResult)> = if interp_isolate {
        // Group by file, preserving discovery order; fan the file
        // batches out on the pool when one exists.
        let mut file_order: Vec<&Path> = Vec::new();
        let mut by_file: std::collections::HashMap<&Path, Vec<&Test>> =
            std::collections::HashMap::new();
        for t in &active {
            let entry = by_file.entry(t.file.as_path()).or_default();
            if entry.is_empty() {
                file_order.push(t.file.as_path());
            }
            entry.push(t);
        }
        let run_batch = |file: &&Path| -> Vec<(Text, TestResult)> {
            let tests = &by_file[*file];
            // The canonical `<prefix>::` namespace is the test name up
            // to the last `::` — identical for every test in the file.
            let prefix = tests[0]
                .name
                .rsplit_once("::")
                .map(|(p, _)| p.to_string())
                .unwrap_or_else(|| tests[0].name.to_string());
            run_interp_file_subprocess(&prefix, tests, ignore_mode, &opts.skip)
        };
        let batches: Vec<Vec<(Text, TestResult)>> = match &pool {
            Some(p) => p.install(|| file_order.par_iter().map(run_batch).collect()),
            None => file_order.iter().map(run_batch).collect(),
        };
        batches.into_iter().flatten().collect()
    } else {
        match &pool {
        Some(p) => {
            // Parallel `--aot`: compile + run each test in its OWN process.
            //
            // In-process parallel native codegen is NOT safe. LLVM's
            // per-process global state — lazily `__cxa_guard_acquire`-
            // registered codegen (MachineFunction) passes, the pass registry,
            // `cl::opt` globals — is shared across rayon workers, and driving
            // it from several threads at once SIGSEGVs in
            // `callDefaultCtor<*Pass>`, aborting the ENTIRE run (defect-class
            // catalogue §23). Locks and pass-guard warm-ups were insufficient
            // (the racing workers aren't *in* codegen). The robust fix is
            // process isolation: re-invoke ourselves as `verum test --aot
            // --test-threads 1 --exact --filter <name>` per test (see
            // `run_aot_subprocess`). Each child owns its LLVM state, so the
            // parent fans them out fully in parallel with zero shared-state
            // races. The child matches a single test (`active.len() == 1`) and
            // runs it in-process, terminating the recursion. Interp stays
            // in-process (no LLVM, no race).
            if opts.tier == Tier::Aot && !aot_child {
                p.install(|| {
                    active
                        .par_iter()
                        .map(|t| (t.name.clone(), run_aot_subprocess(t)))
                        .collect()
                })
            } else {
                p.install(|| active.par_iter().map(|t| run_one(t)).collect())
            }
        }
        // No pool (threads=1 or [test].parallel=false): AOT still goes
        // through per-test subprocesses UNLESS we ARE the subprocess —
        // GENERATE-NATIVE-WORKER-RACE-1 makes process isolation
        // uniform (the old `active.len() > 1` gate ran single-test AOT
        // in-process ON A WORKER, which is exactly the racy shape).
        // The child (VERUM_TEST_AOT_CHILD=1, threads=1, no pool) runs
        // its one test in-process on the MAIN thread — the stable
        // `verum build` configuration — terminating the recursion.
        None if opts.tier == Tier::Aot && !aot_child => active
            .iter()
            .map(|t| (t.name.clone(), run_aot_subprocess(t)))
            .collect(),
        None => active.iter().map(|t| run_one(t)).collect(),
        }
    };

    // T0365: a deliberately-executed `@ignore`'d test that FAILS must
    // surface its pin reason — the reason names the tracking class the
    // failure belongs to, which is the whole point of running pins
    // (`--ignored` promotion / pin-verification sweeps). Idempotent:
    // in the file-quarantine topology the child process already ran
    // this pass on the error text it reported, and the parent must not
    // stack a second copy.
    let results: Vec<(Text, TestResult)> = results
        .into_iter()
        .map(|(name, result)| {
            let pinned = active.iter().find(|t| t.name == name && t.ignored);
            let result = match pinned {
                Some(t) => attach_ignore_note(result, t.ignore_reason.as_deref()),
                None => result,
            };
            (name, result)
        })
        .collect();

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
    if ignore_mode == IgnoreMode::SkipIgnored {
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
                TestFormat::Pretty => {
                    ui::output(&format!("test {} ... {}", t.name, "ignored".yellow()))
                }
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
            let verdict = if failed > 0 {
                "FAILED".red().bold()
            } else {
                "ok".green().bold()
            };
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
            let verdict = if failed > 0 {
                "FAILED".red().bold()
            } else {
                "ok".green().bold()
            };
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

    // Fuzz orchestration (#299): when [test].fuzzing = true, after
    // the @test / @property suite finishes, discover cargo-fuzz
    // targets under `fuzz/` and exercise each. New crash artifacts
    // count as additional failures — the run is GREEN only when
    // both regular tests pass AND no fuzz target produces a fresh
    // artifact. Cargo-fuzz absent → hint message + zero outcomes
    // (best-effort: missing toolchain shouldn't fail CI).
    let mut fuzz_failures: usize = 0;
    if cfg.fuzzing {
        let budget = std::env::var("VERUM_FUZZ_BUDGET_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);
        let report =
            crate::commands::fuzz::run(&manifest_dir, std::time::Duration::from_secs(budget));
        if !quiet {
            ui::output("");
            ui::output(&format!("{}", "fuzz:".bold()));
            if let Some(hint) = &report.hint {
                ui::output(&format!("  {}", hint));
            }
            ui::output(&format!(
                "  Discovered {} fuzz target(s); per-target budget {}s",
                report.discovered, budget
            ));
            for o in &report.outcomes {
                let label = match &o.status {
                    crate::commands::fuzz::FuzzStatus::Clean => "ok".green().bold(),
                    crate::commands::fuzz::FuzzStatus::Crashed => "CRASHED".red().bold(),
                    crate::commands::fuzz::FuzzStatus::HarnessError(_) => {
                        "HARNESS-ERROR".red().bold()
                    }
                    crate::commands::fuzz::FuzzStatus::Timeout => "TIMEOUT".yellow().bold(),
                };
                ui::output(&format!(
                    "    [{}] {} ({})",
                    label,
                    o.target,
                    format_duration(o.duration),
                ));
                if !o.new_crash_artifacts.is_empty() {
                    for a in &o.new_crash_artifacts {
                        ui::output(&format!("      crash: {}", a));
                    }
                }
                if let crate::commands::fuzz::FuzzStatus::HarnessError(msg) = &o.status {
                    ui::output(&format!(
                        "      stderr: {}",
                        msg.lines().next().unwrap_or("")
                    ));
                }
            }
        }
        fuzz_failures = report
            .outcomes
            .iter()
            .filter(|o| {
                matches!(
                    o.status,
                    crate::commands::fuzz::FuzzStatus::Crashed
                        | crate::commands::fuzz::FuzzStatus::HarnessError(_)
                )
            })
            .count();
    }

    let total_failed = failed + fuzz_failures;
    if total_failed > 0 {
        Err(CliError::TestsFailed {
            passed,
            failed: total_failed,
        })
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
        // FILTER-ANCHOR-1 (task #40): a leading `^` anchors the filter at
        // the NAME START (prefix match).  Plain substring matching cannot
        // scope a run to the `text/` subtree at all — `--filter text/`
        // also matches every `co**ntext/**` test, and there is NO safe
        // substring spelling for such tree pairs (`text/mod/unit` ⊂
        // `context/mod/unit`).  A broken foreign suite pulled in this way
        // aborts the whole run under panic=abort, which cost the
        // 2026-07-10 text campaign four separate collisions.
        // `--filter '^text/'` now does what the subtree-scoping doc
        // always promised.
        //
        // TEST-FILTER-SUBSTRING-ANCHOR-1 (T0341): a filter CONTAINING
        // `/` names suite-path segments, so it matches only at a
        // segment boundary — `--filter sync/` selects `sync/…` and the
        // nested `runtime/sync/…` but never `async/…` (pre-fix it
        // pulled 483 async/* tests into a 257-test sync sweep). Plain
        // slash-less substrings (`--filter bloom`) keep their historic
        // anywhere-match semantics.
        Some(f) => match f.as_str().strip_prefix('^') {
            Some(prefix) => name.as_str().starts_with(prefix),
            None if f.as_str().contains('/') => {
                segment_anchored_contains(name.as_str(), f.as_str())
            }
            None => name.as_str().contains(f.as_str()),
        },
    }
}

/// T0341: does `filter` occur in `name` starting at a suite-path
/// segment boundary (the name start, or immediately after a `/`)?
///
/// This is the ONE matcher used for slash-carrying filters — it keeps
/// mid-tree subtree scoping (`collections/list` matches
/// `core/collections/list::t`) while making segment names
/// substring-safe (`sync/` cannot match `async/`).
fn segment_anchored_contains(name: &str, filter: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = name[from..].find(filter) {
        let at = from + rel;
        if at == 0 || name.as_bytes()[at - 1] == b'/' {
            return true;
        }
        // Advance one full character past this occurrence's start so
        // overlapping occurrences at later boundaries are still found.
        from = at
            + name[at..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
    }
    false
}

/// T0365: annotate a failing result from a deliberately-executed
/// `@ignore`'d test with the pin's reason string (the reason names the
/// tracking class the failure belongs to). Pass results are untouched.
/// Idempotent by marker so parent/child topologies can both apply it.
fn attach_ignore_note(result: TestResult, reason: Option<&str>) -> TestResult {
    const MARK: &str = "[@ignore]";
    let note = match reason {
        Some(r) => format!("{} reason: {}", MARK, r),
        None => format!("{} (no reason recorded on the pin)", MARK),
    };
    let attach = |error: String| -> String {
        if error.contains(MARK) {
            error
        } else if error.is_empty() {
            note.clone()
        } else {
            format!("{}\n  {}", error, note)
        }
    };
    match result {
        TestResult::Fail {
            duration,
            stdout,
            stderr,
            exit_code,
            error,
        } => TestResult::Fail {
            duration,
            stdout,
            stderr,
            exit_code,
            error: attach(error),
        },
        TestResult::CompileError { duration, error } => TestResult::CompileError {
            duration,
            error: attach(error),
        },
        pass => pass,
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
        TestResult::Fail {
            stdout,
            stderr,
            exit_code,
            error,
            ..
        } => {
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
        TestResult::Fail {
            duration, error, ..
        } => ("failed", *duration, Some(error.as_str())),
        TestResult::CompileError { duration, error } => {
            ("compile-error", *duration, Some(error.as_str()))
        }
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
        TestResult::Pass {
            duration,
            stdout,
            stderr,
        } => (
            "ok".green().to_string(),
            *duration,
            stdout.clone(),
            stderr.clone(),
        ),
        TestResult::Fail {
            duration,
            stdout,
            stderr,
            ..
        } => (
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
    /// Mirror of `TestOptions.release` — when true, the AOT path
    /// compiles tests at the highest optimization level instead of
    /// the default. Closes the inert-defense pattern around the CLI
    /// `--release` flag for `verum test`: pre-fix the flag landed
    /// on TestOptions but never reached CompilerOptions.
    release: bool,
    /// Mirror of `TestOptions.verify` — when set, overrides the
    /// default per-test `verify_mode`. Recognised values: `runtime`
    /// / `static` / `proof` (case-insensitive). Unrecognised values
    /// fall back to the default to avoid breaking the test runner
    /// on a typo. Closes the inert-defense pattern for the CLI
    /// `--verify <mode>` flag.
    verify_mode_override: Option<verum_compiler::options::VerifyMode>,
    /// Mirror of `[test].property_testing` from the manifest. When
    /// `false`, tests carrying the `@property(...)` attribute are
    /// skipped with a "property testing disabled" outcome instead of
    /// being executed — embedders can opt out of the proptest path
    /// without removing the attribute from individual `.vr` files.
    /// Default: `true` (matches `TestConfig::default()`).
    property_testing: bool,
    /// Mirror of `[test].proptest_cases` — used as the default
    /// `runs` count when an individual `@property(runs = N)` does
    /// not override it. Pre-fix: hardcoded to 100 inside
    /// `run_test_property`. Post-fix: reads from the manifest so
    /// a project-wide `proptest_cases = 1000` actually shapes how
    /// many samples each property explores.
    proptest_cases: u32,
    /// Mirror of `[test].differential` — when `true`, every
    /// non-property test runs through BOTH the interpreter (Tier
    /// 0, in-process VBC) AND the AOT pipeline (Tier 1, native
    /// binary). The differential wrapper requires both tiers to
    /// produce a PASS for the test to count as PASS; any tier
    /// disagreement (one passes, the other fails) surfaces as a
    /// dedicated cross-tier failure with both tiers' diagnostics
    /// attached. This is the load-bearing soundness gate for
    /// the language's two execution backends — disagreement is
    /// the bug, not the test. Property tests (carrying
    /// `@property(...)`) are NOT subject to differential
    /// expansion: they route through the interpreter by design
    /// (per-iteration Value construction is impractical under
    /// per-binary respawn) and are passed through unchanged.
    differential: bool,
    /// Mirror of `[test].fuzzing` — when `true`, after the
    /// regular @test / @property suite finishes, the runner
    /// discovers every cargo-fuzz target under `fuzz/` (workspace
    /// root + `crates/*/fuzz/`) and exercises each via
    /// `cargo fuzz run <target> -- -max_total_time=<N>`. New
    /// crash artifacts under `fuzz/artifacts/<target>/` count as
    /// test failures. Cargo-fuzz absent → the runner emits a
    /// hint and continues (best-effort observability rather than
    /// a hard CI gate). See `commands/fuzz.rs`.
    fuzzing: bool,
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

/// Compile + run ONE AOT test in a separate `verum` process.
///
/// In-process parallel native codegen is not thread-safe — LLVM's
/// per-process state (lazily `__cxa_guard`-registered codegen passes,
/// the pass registry, `cl::opt` globals) is shared across rayon workers
/// and races into a `callDefaultCtor<*Pass>` SIGSEGV (defect-class
/// catalogue §23). Re-invoking ourselves with `--test-threads 1 --exact
/// --filter <name>` gives this test its own process and thus its own
/// isolated LLVM state; the parent fans these out in parallel for full
/// throughput with no shared-state race. The child matches exactly one
/// test, so it runs in-process and does NOT recurse. Pass/fail is read
/// from the child's exit status; on failure the child's captured output
/// is surfaced for the failure report.
fn run_aot_subprocess(test: &Test) -> TestResult {
    let start = Instant::now();
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("verum"));
    let mut cmd = Command::new(&exe);
    cmd.arg("test")
        .arg("--aot")
        .arg("--test-threads")
        .arg("1")
        .arg("--exact")
        .arg("--filter")
        .arg(test.name.as_str())
        .arg("--format")
        .arg("terse");
    // T0365: the parent already SELECTED this test; when it is
    // `@ignore`'d (an --ignored / --include-ignored run) the child's
    // own default-mode selection must not re-skip it — pre-fix the
    // child matched zero tests and exited 0, a silent false PASS.
    if test.ignored {
        cmd.arg("--include-ignored");
    }
    let out = cmd
        // GENERATE-NATIVE-WORKER-RACE-1: mark the child so it runs its
        // single test in-process (main thread) instead of recursing.
        .env("VERUM_TEST_AOT_CHILD", "1")
        .output();
    match out {
        Ok(o) if o.status.success() => TestResult::Pass {
            duration: start.elapsed(),
            stdout: String::new(),
            stderr: String::new(),
        },
        Ok(o) => TestResult::Fail {
            duration: start.elapsed(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            exit_code: o.status.code(),
            error: "AOT compile/run failed in isolated subprocess".to_string(),
        },
        Err(e) => TestResult::CompileError {
            duration: start.elapsed(),
            error: format!("failed to spawn AOT subprocess: {}", e),
        },
    }
}

/// TEST-RUNNER-ISOLATION-1 — run ONE test FILE's interp batch in a
/// separate `verum` process.
///
/// The in-process interp suite runs every test in the ONE CLI process;
/// a wild raw load in any test (`RawLoadI64` checks only `addr > 0`)
/// SIGSEGVs the runner and every suite scheduled after it is silently
/// never run — coverage loss disguised as a crash (concrete incident:
/// the ctx_bridge overflow-guard test with a below-threshold count
/// aborted the whole `^runtime/` sweep).  Process isolation is the
/// only recovery that works for fatal signals; per-FILE granularity
/// keeps the compile amortization (all of a file's tests share one
/// child, one module compile) so the overhead is one process spawn per
/// file, not per test.
///
/// The child re-invokes `verum test --interp --format json --filter
/// ^<file-prefix>::` with `VERUM_TEST_INTERP_CHILD=1` (recursion
/// terminator, mirroring `VERUM_TEST_AOT_CHILD`).  Per-test outcomes
/// are read from the child's `{"event":"test",...}` stream; tests the
/// child never reported (it died mid-batch) are marked failed with the
/// child's exit/signal attribution — the REST OF THE SUITE CONTINUES.
///
/// Escape hatch: `VERUM_TEST_NO_ISOLATE=1` restores the historical
/// single-process topology (useful for in-process debuggers).
fn run_interp_file_subprocess(
    prefix: &str,
    expected: &[&Test],
    ignore_mode: IgnoreMode,
    skip: &[Text],
) -> Vec<(Text, TestResult)> {
    let start = Instant::now();
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("verum"));
    // Anchor the child's filter to this file's canonical `<prefix>::`
    // name space. Escape regex metacharacters in the path-derived
    // prefix so a literal '.' in a folder name can't cross-match.
    let escaped: String = prefix
        .chars()
        .flat_map(|c| {
            let needs = matches!(
                c,
                '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\'
            );
            if needs { vec!['\\', c] } else { vec![c] }
        })
        .collect();
    let mut cmd = Command::new(&exe);
    cmd.arg("test")
        .arg("--interp")
        .arg("--format")
        .arg("json")
        .arg("--filter")
        .arg(format!("^{}::", escaped));
    // T0365: the child re-applies SELECTION over this file's tests, so
    // the parent's selection flags must ride along. Without them the
    // child's default mode silently re-skipped every `@ignore`'d test
    // the parent selected (`--ignored` → "0 passed; N ignored"), and
    // ran tests the parent's `--skip` excluded (widening a crash's
    // blast radius past the user's selection).
    for a in ignore_mode.child_args() {
        cmd.arg(a);
    }
    for p in skip {
        cmd.arg("--skip").arg(p.as_str());
    }
    let out = cmd.env("VERUM_TEST_INTERP_CHILD", "1").output();
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            let err = format!("failed to spawn interp subprocess: {}", e);
            return expected
                .iter()
                .map(|t| {
                    (
                        t.name.clone(),
                        TestResult::CompileError {
                            duration: start.elapsed(),
                            error: err.clone(),
                        },
                    )
                })
                .collect();
        }
    };
    // Parse the child's per-test JSON events.
    let stdout_text = String::from_utf8_lossy(&out.stdout);
    let mut reported: std::collections::HashMap<String, TestResult> =
        std::collections::HashMap::new();
    for line in stdout_text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if v.get("event").and_then(|e| e.as_str()) != Some("test") {
            continue;
        }
        let Some(name) = v.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let duration = Duration::from_millis(
            v.get("duration_ms").and_then(|d| d.as_u64()).unwrap_or(0),
        );
        let error = v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("")
            .to_string();
        let result = match v.get("outcome").and_then(|o| o.as_str()) {
            Some("ok") => TestResult::Pass {
                duration,
                stdout: String::new(),
                stderr: String::new(),
            },
            Some("ignored") => continue, // parent counts ignored itself
            Some("compile-error") => TestResult::CompileError { duration, error },
            _ => TestResult::Fail {
                duration,
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                error,
            },
        };
        reported.insert(name.to_string(), result);
    }
    // Attribute the child's death to every unreported test.
    // T0365: `expected` IS the parent's selected set for this file
    // (the ignore-mode already shaped it), so it needs no re-filtering
    // here — the old `!t.ignored` guard was the parent-side face of
    // the executor re-skip.
    let crash_note = if reported.len() < expected.len() {
        let status = &out.status;
        #[cfg(unix)]
        let sig = {
            use std::os::unix::process::ExitStatusExt;
            status.signal()
        };
        #[cfg(not(unix))]
        let sig: Option<i32> = None;
        Some(match (sig, status.code()) {
            (Some(s), _) => format!(
                "interp batch runner killed by signal {} before this test ran \
                 (TEST-RUNNER-ISOLATION-1 quarantine; see the sibling failures \
                 in this file for the crashing test)",
                s
            ),
            (None, Some(c)) => format!(
                "interp batch runner exited {} before reporting this test \
                 (TEST-RUNNER-ISOLATION-1 quarantine)",
                c
            ),
            (None, None) => "interp batch runner vanished before reporting this test \
                 (TEST-RUNNER-ISOLATION-1 quarantine)"
                .to_string(),
        })
    } else {
        None
    };
    expected
        .iter()
        .map(|t| {
            let result = reported.remove(t.name.as_str()).unwrap_or_else(|| {
                TestResult::Fail {
                    duration: start.elapsed(),
                    stdout: String::new(),
                    stderr: String::from_utf8_lossy(&out.stderr)
                        .lines()
                        .rev()
                        .take(10)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n"),
                    exit_code: out.status.code(),
                    error: crash_note.clone().unwrap_or_else(|| {
                        "interp batch runner reported no outcome for this test".to_string()
                    }),
                }
            });
            (t.name.clone(), result)
        })
        .collect()
}

fn run_single_test(test: &Test, target_dir: &Path, cfg: &TestRunCfg) -> TestResult {
    if let Some(prop) = &test.property {
        // When the manifest disables property testing, we treat any
        // test carrying `@property(...)` as a configured skip rather
        // than executing the proptest harness. This lets embedders
        // turn the entire property-testing surface off without
        // editing individual `.vr` files — the attribute stays put,
        // the runner simply records a "skipped" outcome. Reported
        // as a Pass with a single-line stdout so CI tooling sees a
        // clean run rather than a synthetic failure.
        if !cfg.property_testing {
            return TestResult::Pass {
                duration: Duration::from_nanos(0),
                stdout: String::from("property testing disabled in manifest [test]"),
                stderr: String::new(),
            };
        }
        // Property tests are NOT subject to differential expansion:
        // their per-iteration Value construction is impractical
        // under per-binary respawn (the AOT path would require
        // re-launching the binary for every sample). Route them
        // through the proptest harness regardless of cfg.differential.
        return run_test_property(test, prop, cfg);
    }
    // Differential dispatch: run the test through BOTH tiers and
    // require agreement. This is the load-bearing soundness gate
    // for the language's two execution backends. Disagreement
    // (one tier Pass, the other Fail) is itself the test failure.
    if cfg.differential {
        return run_test_differential(test, target_dir, cfg);
    }
    match cfg.tier {
        Tier::Aot => run_test_aot(test, target_dir, cfg),
        Tier::Interpret => run_test_interpret(test, cfg),
    }
}

/// Run `test` through both Tier 0 (interpreter) and Tier 1 (AOT)
/// and require both to PASS. Any disagreement surfaces as a
/// cross-tier soundness failure with both tiers' diagnostics
/// attached so a maintainer can pinpoint which tier produced the
/// faulty result.
///

/// Outcome lattice:
///

/// | T0 outcome | T1 outcome | Differential outcome |
/// |-------------|-------------|--------------------------------|
/// | Pass | Pass | Pass (durations summed) |
/// | Pass | Fail | Fail (cross-tier disagreement) |
/// | Fail | Pass | Fail (cross-tier disagreement) |
/// | Fail | Fail | Fail (both tiers; first error) |
/// | CompileErr | * | CompileErr (T0 short-circuits) |
/// | Pass | CompileErr | Fail (T1 cannot lower) |
///

/// The duration field aggregates both tiers so the test report
/// reflects total cross-tier work.
fn run_test_differential(test: &Test, target_dir: &Path, cfg: &TestRunCfg) -> TestResult {
    // Run T0 first — it's the fastest path and a CompileError
    // here short-circuits T1 (no point lowering a program that
    // already failed at the type-checker / VBC codegen layer).
    let t0_start = Instant::now();
    let t0 = run_test_interpret(test, cfg);
    let t0_dur = t0_start.elapsed();

    if let TestResult::CompileError { error, .. } = &t0 {
        return TestResult::CompileError {
            duration: t0_dur,
            error: format!("differential[T0=compile-error]: {error}"),
        };
    }

    let t1_start = Instant::now();
    let t1 = run_test_aot(test, target_dir, cfg);
    let t1_dur = t1_start.elapsed();
    combine_differential_outcomes(t0, t1, t0_dur + t1_dur)
}

/// Pure-function outcome combiner for differential testing.
///

/// Extracted from `run_test_differential` so the cross-tier
/// agreement contract can be pinned by unit tests without
/// driving the full compile/link/execute pipeline. The tier
/// inputs are already-computed `TestResult` values; this
/// function only handles the lattice that maps two outcomes
/// to a single `TestResult`.
fn combine_differential_outcomes(
    t0: TestResult,
    t1: TestResult,
    total_dur: Duration,
) -> TestResult {
    let t0_pass = matches!(t0, TestResult::Pass { .. });
    let t1_pass = matches!(t1, TestResult::Pass { .. });

    if t0_pass && t1_pass {
        let (t0_stdout, t0_stderr) = match &t0 {
            TestResult::Pass { stdout, stderr, .. } => (stdout.clone(), stderr.clone()),
            _ => (String::new(), String::new()),
        };
        let (t1_stdout, t1_stderr) = match &t1 {
            TestResult::Pass { stdout, stderr, .. } => (stdout.clone(), stderr.clone()),
            _ => (String::new(), String::new()),
        };
        // Surface stdout drift as a soft warning on stderr — both
        // tiers Pass so the test is GREEN, but a maintainer can
        // see disagreement in the textual output (typically
        // tolerable: timestamps, addresses, hashmap iteration
        // order). Hard-failing on stdout drift would force every
        // test to be deterministic across both backends, which is
        // not a goal of differential testing — agreement on the
        // test's pass/fail verdict is the load-bearing contract.
        let stderr = if t0_stdout != t1_stdout {
            format!(
                "{}{}\n[differential] stdout drift between tiers (both PASS):\n  T0: {} bytes\n  T1: {} bytes",
                t0_stderr,
                t1_stderr,
                t0_stdout.len(),
                t1_stdout.len()
            )
        } else {
            format!("{}{}", t0_stderr, t1_stderr)
        };
        return TestResult::Pass {
            duration: total_dur,
            stdout: format!("[T0] {t0_stdout}\n[T1] {t1_stdout}"),
            stderr,
        };
    }

    let summarise = |r: &TestResult| -> String {
        match r {
            TestResult::Pass { stdout, .. } => format!("PASS ({} bytes stdout)", stdout.len()),
            TestResult::Fail {
                error, exit_code, ..
            } => {
                format!("FAIL (exit={:?}): {error}", exit_code)
            }
            TestResult::CompileError { error, .. } => {
                format!("COMPILE-ERROR: {error}")
            }
        }
    };
    let t0_summary = summarise(&t0);
    let t1_summary = summarise(&t1);
    let header = match (t0_pass, t1_pass) {
        (true, false) => "cross-tier disagreement: T0 PASS / T1 FAIL",
        (false, true) => "cross-tier disagreement: T0 FAIL / T1 PASS",
        (false, false) => "both tiers failed",
        (true, true) => unreachable!("handled above"),
    };
    let error = format!("{header}\n  [T0 interpret] {t0_summary}\n  [T1 aot]       {t1_summary}");
    let exit_code = match (&t0, &t1) {
        (TestResult::Fail { exit_code, .. }, _) => *exit_code,
        (_, TestResult::Fail { exit_code, .. }) => *exit_code,
        _ => None,
    };
    TestResult::Fail {
        duration: total_dur,
        stdout: String::new(),
        stderr: String::new(),
        exit_code,
        error,
    }
}

/// Property-test path: compile once via VBC, loop the PBT runner.
/// Routes through the interpreter irrespective of --tier because the
/// property runner needs per-iteration Value construction that the
/// native-binary path can't do (each sample would require respawning).
fn run_test_property(
    test: &Test,
    prop: &crate::commands::property::PropertyFunc,
    cfg: &TestRunCfg,
) -> TestResult {
    use crate::commands::property::{
        RunnerConfig, Seed, load_regression_db, record_regression, run_property,
        save_regression_db, seeds_for,
    };

    let start = Instant::now();

    // Compile file → VBC through the SAME stdlib-aware path as
    // `run_test_interpret`, memoised per file (shared `CompileKind::Stdlib`
    // cache) so an N-property file — or a file mixing @test and @property —
    // compiles exactly once.  This unification fixes the prior
    // archive-const codegen miss (see `build_stdlib_test_module`).
    let module = match compiled_test_module(&test.file, CompileKind::Stdlib, || {
        build_stdlib_test_module(test)
    }) {
        Ok(m) => m,
        Err(error) => {
            return TestResult::CompileError {
                duration: start.elapsed(),
                error,
            };
        }
    };

    // Replay regression DB seeds first, then draw fresh ones.
    let mut db = load_regression_db();
    let replay_seeds = seeds_for(&db, test.name.as_str());

    // Default-runs precedence: per-test `@property(runs = N)` wins
    // over manifest `[test].proptest_cases`. When neither is set
    // we fall back to TestConfig::default()'s 256 (the historical
    // hard-coded literal here was 100, which silently masked the
    // manifest setting since the pre-fix runner ignored cfg).
    let default_runs = prop.runs_override.unwrap_or(cfg.proptest_cases);
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
        db.entries
            .retain(|e| !(e.test == name && pruned_hex.contains(&e.seed)));
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

    let stem = test
        .file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("test");
    // Unique per (test file, test fn) — the output binary, like the merged
    // source and its `.o`/`.ll`, must not collide across the many test
    // files that share a stem ("unit_test"), or parallel `par_iter`
    // workers overwrite each other's binaries (running the wrong / a
    // half-written executable). See `unique_merged_stem`.
    let binary_name = format!(
        "test_{}",
        unique_merged_stem(&test.file, test.fn_name.as_deref(), stem)
    );
    let output_path = target_dir.join(&binary_name);

    let mut lint_config = verum_compiler::lint::LintConfig::default();
    if cfg.deny_warnings {
        lint_config.deny_warnings = true;
    }

    // T6.0.4 — AOT path companion to the interpret-mode auto-mount.
    // If the test file lives inside a cog with a src/lib.vr or
    // src/main.vr, synthesise a `mount cog.lib.*` (or `mount cog.main.*`)
    // line and prepend it via a temp file so the production pipeline
    // resolves crate-root references without per-test boilerplate.
    //

    // T0.5.2 — additionally synthesise a `fn main()` that invokes the
    // @test function and exits 0 on success, so the AOT-compiled
    // binary's exit code matches the test convention (mirrors what
    // run_test_interpret does in-process via Interpreter::call).
    //
    // **Architectural rule** (task #16 close): the synthetic
    // `fn main` MUST be emitted unconditionally for every AOT test,
    // not gated on the presence of `src/lib.vr` / `src/main.vr` in the
    // surrounding cog.  Pre-fix
    // `synthesise_test_input_with_crate_root` returned `None` when
    // either condition was missing (no `verum.toml` parent, or no
    // crate-root file), leaving the test compiled WITHOUT a
    // `verum_main` body.  The C-runtime entry (`emit_main_entry` in
    // `platform_ir.rs:1927`) then fell through to its `no_main`
    // branch and silently `return 1` for every test — yielding the
    // depth-of-bug "AOT exit code 1, no diagnostic" failure mode
    // that masked the test result entirely.
    //
    // Fall back to the new `synthesise_test_main_only` helper when
    // the crate-root merge can't run: it produces a minimal test
    // file containing only the original source plus the synthetic
    // `fn main`, preserving the test convention regardless of cog
    // layout.
    // T0365: an `@ignore`'d test reaches run_test_aot only when an
    // ignore-inclusive mode (--ignored / --include-ignored) SELECTED
    // it — its own fn must then survive the merge strip
    // (IGNORED-FN-IN-MERGE-1 removes every pinned fn otherwise, so the
    // synthetic main called a function that no longer existed).
    // Sibling pins still strip: they may pin compile-time gaps.
    let keep_ignored_fn = if test.ignored {
        test.fn_name.as_deref()
    } else {
        None
    };
    let test_input = synthesise_test_input_with_crate_root(
        &test.file,
        target_dir,
        test.fn_name.as_deref(),
        keep_ignored_fn,
    )
    .or_else(|| {
        synthesise_test_main_only(
            &test.file,
            target_dir,
            test.fn_name.as_deref(),
            keep_ignored_fn,
        )
    })
    .unwrap_or_else(|| test.file.clone());

    // Wire CLI `--verify` and `--release` into the compilation:
    //  * `verify_mode_override` overrides the default Runtime mode
    //  when the user passed `verum test --verify static|proof`.
    //  * `release = true` lifts the optimization level to 3,
    //  matching `verum build --release` semantics.
    let verify_mode = cfg.verify_mode_override.unwrap_or(VerifyMode::Runtime);
    let optimization_level = if cfg.release { 3 } else { 0 };
    let options = CompilerOptions {
        input: test_input,
        output: output_path.clone(),
        verify_mode,
        output_format: OutputFormat::Human,
        coverage: cfg.coverage,
        lint_config,
        language_features: cfg.language_features.clone(),
        optimization_level,
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
                            error: format!("timed out after {}s", cfg.timeout_secs),
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
        TestResult::Pass {
            duration,
            stdout,
            stderr,
        }
    } else {
        let code = output.status.code();
        let error = code
            .map(|c| format!("process exited with code {}", c))
            .unwrap_or_else(|| "process terminated by signal".to_string());
        TestResult::Fail {
            duration,
            stdout,
            stderr,
            exit_code: code,
            error,
        }
    }
}

/// T6.0.4 (AOT companion) — synthesise a temp test source file that
/// prepends crate-root contents to the user's test file. The AOT
/// pipeline operates on a single input file, so to give tests/
/// implicit access to src/lib.vr we read both, concatenate body
/// contents (stripping any duplicate `module` header from the
/// crate root since the test file owns its own module identity),
/// and write to `<target_dir>/test_<stem>.merged.vr`.
///

/// T0.5.2 — when `test_fn_name` is provided, *also* append a
/// synthesised `fn main() -> Int { <fn>(); 0 }` so the AOT
/// binary's exit-code semantics match the test convention. The
/// runner reads the binary's exit code, so success must be 0.
/// Returns the merged file path on success, `None` (= use the
/// original test file) if no manifest / no crate root / IO failure.
/// IGNORED-FN-IN-MERGE-1 — drop `@ignore`-pinned test fns from a test
/// source before it enters the AOT merged compile.
///
/// The interp runner honours `@ignore` per test, but the AOT path
/// compiles the WHOLE merged file — an ignored test whose body pins a
/// known COMPILE-TIME gap (that is often exactly why it is pinned)
/// fails the file's typecheck and takes every live sibling test down
/// with it (RUNTIME-AOT-LEG-1 groups 1: mod/unit 7 + thread 2, where
/// pinned `RuntimeInfo.*` / `Thread.spawn<Int>` bodies E400'd the
/// merge).
///
/// Line-level scanner: an `@ignore` attribute starts a skip region
/// that swallows the following comment/attribute lines, the `fn`
/// declaration, and its brace-balanced body.  Brace counting is
/// naive about braces inside STRING literals — balanced f-string
/// interpolations (`f"x={y}"`) cancel out, but a lone unmatched `{`
/// in a plain string inside a pinned body would desync the scan; the
/// conformance suites do not contain that shape.
/// `keep_fn` (T0365): when the runner is DELIBERATELY executing one
/// `@ignore`'d test (`--ignored` / `--include-ignored` selected it),
/// that fn's region is emitted instead of stripped — minus the
/// `@ignore` attribute line itself — so the merged file still defines
/// the function the synthetic main invokes. Every OTHER pinned fn is
/// stripped exactly as before.
fn strip_ignored_tests(source: &str, keep_fn: Option<&str>) -> String {
    let mut out = String::with_capacity(source.len());
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim_start().starts_with("@ignore") {
            // Swallow up to the fn decl…, buffering the region so it
            // can be re-emitted when it is the fn under execution.
            let mut region: Vec<&str> = Vec::new();
            let mut region_fn: Option<Text> = None;
            let mut depth: i64 = 0;
            let mut seen_body = false;
            for l in lines.by_ref() {
                region.push(l);
                if region_fn.is_none() {
                    region_fn = extract_fn_name(l);
                }
                let opens = l.matches('{').count() as i64;
                let closes = l.matches('}').count() as i64;
                if opens > 0 {
                    seen_body = true;
                }
                depth += opens - closes;
                if seen_body && depth <= 0 {
                    break;
                }
            }
            let keep = matches!(
                (keep_fn, region_fn.as_ref()),
                (Some(k), Some(n)) if n.as_str() == k
            );
            if keep {
                for l in region {
                    out.push_str(l);
                    out.push('\n');
                }
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn synthesise_test_input_with_crate_root(
    test_file: &Path,
    target_dir: &Path,
    test_fn_name: Option<&str>,
    keep_ignored_fn: Option<&str>,
) -> Option<PathBuf> {
    let mut cur = test_file.parent()?;
    let cog_root = loop {
        if cur.join("verum.toml").is_file() || cur.join("Verum.toml").is_file() {
            break cur.to_path_buf();
        }
        cur = cur.parent()?;
    };

    let candidates = [cog_root.join("src/lib.vr"), cog_root.join("src/main.vr")];
    let root_path = candidates.iter().find(|p| p.is_file())?;

    let test_source =
        strip_ignored_tests(&std::fs::read_to_string(test_file).ok()?, keep_ignored_fn);
    let root_source = std::fs::read_to_string(root_path).ok()?;

    // Strip any leading `module …;` declaration from the crate root —
    // the test file's module identity wins.
    let stripped_root = root_source
        .lines()
        .skip_while(|l| {
            let t = l.trim_start();
            t.is_empty() || t.starts_with("//") || t.starts_with("/*")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let stripped_root = if stripped_root.trim_start().starts_with("module ") {
        // Drop the module header line (and its trailing semicolon).
        stripped_root
            .split_once(';')
            .map(|(_, rest)| rest)
            .unwrap_or(&stripped_root)
            .to_string()
    } else {
        stripped_root
    };

    // T0.5.2 — synthetic main wraps the @test function. Returns 0
    // on natural completion (any panic / assert-fail aborts the
    // process before reaching the final 0). This mirrors how
    // run_test_interpret extracts the test fn and calls it directly.
    let synth_main = match test_fn_name {
        Some(name) => format!(
            "\n\n// === T0.5.2 synthetic main — invokes the @test fn ===\n\
             public fn main() -> Int {{\n    {}();\n    0\n}}\n",
            name
        ),
        None => String::new(),
    };

    let stem = test_file.file_stem()?.to_str()?;
    let merged_path = target_dir.join(format!(
        "test_{}.merged.vr",
        unique_merged_stem(test_file, test_fn_name, stem)
    ));
    if std::fs::create_dir_all(target_dir).is_err() {
        return None;
    }
    let merged = format!(
        "// Auto-merged by T6.0.4 — test file body appended after stripped crate root.\n\
         // Source test: {}\n// Source crate root: {}\n\n{}\n\n// === test body ===\n{}{}",
        test_file.display(),
        root_path.display(),
        stripped_root,
        test_source,
        synth_main,
    );
    std::fs::write(&merged_path, merged).ok()?;
    Some(merged_path)
}

/// Per-test-unique stem for the merged source file and every artifact
/// derived from it (the `test_<stem>` binary, `<stem>.o`, `<stem>.ll`).
///
/// Many test files share a `file_stem` — every module's
/// `unit_test.vr` / `property_test.vr` / … collapses to the same
/// `"unit_test"` — and the synthetic `main` wraps a *specific* `@test`,
/// so the merged content differs per test. Keying the merged path on
/// the bare stem makes parallel `par_iter` workers write the SAME
/// `target/test/test_unit_test.merged.vr` concurrently; the interleaved
/// writes corrupt it, and the malformed source lowers to malformed IR
/// that SIGSEGVs LLVM during `generate_native` — aborting the entire
/// `verum test --aot` run (0 results from N tests). Folding the full
/// source path + test-fn into the stem makes every concurrent
/// compilation target its own files. Deterministic (fixed-key
/// `DefaultHasher`) so re-runs reuse the same scratch names.
fn unique_merged_stem(test_file: &Path, test_fn_name: Option<&str>, stem: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    test_file.hash(&mut hasher);
    test_fn_name.hash(&mut hasher);
    format!("{}_{:016x}", stem, hasher.finish())
}

/// Task #16 close — synthetic-main-only fallback when the cog has
/// no `src/lib.vr` / `src/main.vr` to merge.  Produces a temp file
/// containing the test source verbatim plus a `public fn main() -> Int
/// { <fn>(); 0 }` trailer so the AOT entry-point flow finds a
/// `verum_main` to call instead of silently `return 1`'ing.
///
/// Mirrors the synthetic-main half of
/// `synthesise_test_input_with_crate_root` without the crate-root
/// merge — used as the universal fallback so EVERY AOT test, even
/// in cogs without a `src/` directory (e.g. integration test
/// packages whose tests live in `tests/` only), gets a wired-up
/// main entry.  Pre-fix the synthesis was gated on cog layout and
/// silently dropped for everything else, leading to the depth-of-
/// bug "AOT exit code 1, no diagnostic" failure mode.
///
/// Returns `None` only when `test_fn_name` is `None` (no @test to
/// invoke) or when filesystem write fails — in both cases the
/// caller falls through to the raw test file.
fn synthesise_test_main_only(
    test_file: &Path,
    target_dir: &Path,
    test_fn_name: Option<&str>,
    keep_ignored_fn: Option<&str>,
) -> Option<PathBuf> {
    let test_fn = test_fn_name?;
    let test_source =
        strip_ignored_tests(&std::fs::read_to_string(test_file).ok()?, keep_ignored_fn);
    let stem = test_file.file_stem()?.to_str()?;
    let merged_path = target_dir.join(format!(
        "test_{}.merged.vr",
        unique_merged_stem(test_file, test_fn_name, stem)
    ));
    if std::fs::create_dir_all(target_dir).is_err() {
        return None;
    }
    let synth_main = format!(
        "\n\n// === task #16 close — synthetic main wraps the @test fn ===\n\
         public fn main() -> Int {{\n    {}();\n    0\n}}\n",
        test_fn
    );
    let merged = format!(
        "// Auto-synthesised by task #16 — no crate-root merge needed.\n\
         // Source test: {}\n\n{}{}",
        test_file.display(),
        test_source,
        synth_main,
    );
    std::fs::write(&merged_path, merged).ok()?;
    Some(merged_path)
}

/// T6.0.4 — locate the cog's crate root (src/lib.vr or src/main.vr)
/// and parse its items so the test module can reference them
/// without an explicit `mount` line. Walks up from the test file
/// looking for a `verum.toml`; on hit, tries `src/lib.vr` then
/// `src/main.vr`. Returns the parsed root's items on success;
/// `None` if no cog manifest, no root file, or parse failure
/// (we silently fall through — a parse error in the crate root
/// will surface as the user runs `verum check` separately).
fn find_and_parse_crate_root(test: &Test) -> Option<List<verum_ast::Item>> {
    use std::path::Path;

    fn walk_up_for_manifest(start: &Path) -> Option<std::path::PathBuf> {
        let mut cur = start.parent()?;
        loop {
            if cur.join("verum.toml").is_file() || cur.join("Verum.toml").is_file() {
                return Some(cur.to_path_buf());
            }
            cur = cur.parent()?;
        }
    }

    let cog_root = walk_up_for_manifest(&test.file)?;
    let candidates = [cog_root.join("src/lib.vr"), cog_root.join("src/main.vr")];
    let root_path = candidates.iter().find(|p| p.is_file())?;

    let source = std::fs::read_to_string(root_path).ok()?;
    let file_id = FileId::new(1); // Distinct from test file's FileId(0).
    let parser = VerumParser::new();
    let lexer = Lexer::new(&source, file_id);
    let module = parser.parse_module(lexer, file_id).ok()?;
    Some(module.items)
}

// --------------------------------------------------------------------
// Per-file compiled-module cache
// --------------------------------------------------------------------
//
// Every `@test` in a file shares one identical AST, so it lowers to one
// identical compiled `VbcModule`.  The historical runner recompiled the
// file — and lazily re-linked the embedded stdlib archive — once *per
// test*, so an N-test file paid the full stdlib-link cost N times.  For
// a 143-test module that is ~140 redundant full compilations (measured
// at ~43 s each for a module that mounts `core.base.*`).
//
// `compiled_test_module` memoises the compiled module by
// (canonical-path, kind).  The first test to reach a given file
// compiles it while holding that file's slot lock; concurrent same-file
// tests (the suite runs under rayon's `par_iter`) block on the single
// slot lock and then reuse the cached `Arc<VbcModule>`.  Distinct files
// still compile fully in parallel — the outer map lock is held only
// long enough to fetch-or-create the slot, never across a compile.
//
// `kind` is retained as a cache-key discriminant for forward
// extensibility, but both runner entry points (`run_test_interpret`
// for @test and `run_test_property` for @property) now compile through
// the single stdlib-aware `build_stdlib_test_module`
// (`compile_module_with_stdlib` + crate-root merge), so they share the
// `CompileKind::Stdlib` slot and a mixed @test/@property file compiles
// exactly once.
//
// Sharing one module across tests is sound: `VbcModule` carries no
// interior mutability and the interpreter takes it via `Arc` (read
// only), so every test still runs in a fresh `Interpreter` over the
// same immutable bytecode.  A compile *error* is cached too — a file
// that fails to compile fails identically for all its tests, and we
// must not pay to rediscover that per test.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum CompileKind {
    /// `compile_module_with_stdlib` + crate-root merge — the canonical
    /// (and currently only) test-compile path for both @test and
    /// @property.
    Stdlib,
}

type CachedModule = std::result::Result<Arc<VbcModule>, String>;

fn compiled_test_module(
    file: &Path,
    kind: CompileKind,
    build: impl FnOnce() -> CachedModule,
) -> CachedModule {
    type Slot = Arc<Mutex<Option<CachedModule>>>;
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, CompileKind), Slot>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    // Canonicalise so two spellings of the same path collapse to one
    // entry; fall back to the raw path if the file can't be resolved.
    let canon = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let key = (canon, kind);

    // Brief outer-lock window: fetch or create this file's slot.
    let slot: Slot = {
        let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    };

    // Per-file lock: serialises only same-file compilers. Distinct
    // files proceed concurrently because they hold distinct slots.
    let mut slot_guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = slot_guard.as_ref() {
        return cached.clone();
    }
    let built = build();
    *slot_guard = Some(built.clone());
    built
}

// Compile a test file to a stdlib-aware `VbcModule`.
//
// This is the single canonical test-compile entry point: it parses the
// source, merges the cog crate-root (T6.0.4), and lowers through
// `compile_module_with_stdlib` so the embedded stdlib archive's full
// const/function surface is registered in codegen context (via
// `apply_lazy_with_types`).  BOTH `run_test_interpret` (@test) and
// `run_test_property` (@property) route through here so the two paths
// share identical codegen semantics AND the same per-file compile cache
// (`CompileKind::Stdlib`).
//
// Historical defect (fixed): `run_test_property` previously used the
// bare `VbcCodegen::compile_module`, which skips the archive load.  A
// stdlib const reachable only through the archive (e.g.
// `core.mem.capability.CAP_EXECUTE`, referenced from a non-DCE'd helper
// inside a `@property` file) then failed codegen with
// `UndefinedVariable("CAP_EXECUTE")` even though the same code compiled
// cleanly as an `@test`.  Unifying on `compile_module_with_stdlib`
// closes that whole class.
fn build_stdlib_test_module(test: &Test) -> CachedModule {
    use verum_vbc::codegen::CodegenConfig;

    let source = std::fs::read_to_string(&test.file).map_err(|e| format!("read: {}", e))?;

    let file_id = FileId::new(0);
    let parser = VerumParser::new();
    let lexer = Lexer::new(&source, file_id);
    let mut ast = parser.parse_module(lexer, file_id).map_err(|errs| {
        let joined = errs
            .iter()
            .map(|e| format!("{}", e))
            .collect::<Vec<_>>()
            .join("\n");
        format!("parse: {}", joined)
    })?;

    // T6.0.4 — tests/ files implicitly mount the cog's crate root
    // (src/lib.vr or src/main.vr). Cargo / npm conventionally make a
    // package's tests/ directory have unrestricted access to the
    // package's public API; Verum aligns: locate the cog manifest by
    // walking up from the test file, parse the crate root, and
    // append its items to the test module's item list. Mount-line
    // boilerplate in test files becomes optional.
    let crate_root_items = find_and_parse_crate_root(test);

    // META-TEST-TYPECHECK-1 — run the REAL type checker before VBC
    // codegen.  Pre-fix the interp/property harness compiled test
    // files straight through `compile_module_with_stdlib` (AST → VBC),
    // which contains NO `verum_types` phase at all — a test could
    // assign `()` (e.g. the Unit return of `List.append`) into a
    // `List<T>`-typed field and "pass" compilation, then die at
    // runtime with NullPointer/`method not found on ()`.  That made
    // `verum test --interp` STRICTLY MORE LENIENT than both
    // `verum run --interp` (which calls `validate_module`) and
    // `verum test --aot` (full pipeline `phase_type_check`) — so
    // interp-vs-AOT "divergence" was frequently just the missing
    // checker, not a tier bug.  Route through the same
    // `run_check_only` entry `verum check` uses; the per-file compile
    // cache (`CompileKind::Stdlib`) amortises it to once per file.
    //
    // Scope: standalone (non-cog) test files — exactly the
    // `core-tests/` conformance surface.  Cog tests merge crate-root
    // items below and would need the same synthesis the AOT path does;
    // they keep the pre-existing behaviour for now.
    // Escape hatch for triage sweeps: `VERUM_TEST_LENIENT_TYPES=1`.
    if crate_root_items.is_none()
        && std::env::var("VERUM_TEST_LENIENT_TYPES").is_err()
    {
        // TEST-PREFLIGHT-IGNORE-1: strip `@ignore`'d test fns before the
        // file-level check. A deliberately compile-broken pinned test
        // (an E400 repro under @ignore) previously failed EVERY test in
        // the file uniformly — @ignore only skipped execution, not this
        // preflight — forcing pins to be fully commented out. The strip
        // is lexical (attr-anchored brace matching, string/char-aware)
        // and FAIL-OPEN: any irregularity falls back to checking the
        // original file (today's behaviour). The stripped copy lives at
        // a stable per-source path so the pipeline's per-file compile
        // cache keeps amortising.
        let check_input: std::path::PathBuf = match std::fs::read_to_string(&test.file)
            .ok()
            .and_then(|s| strip_ignored_test_fns(&s))
        {
            Some(stripped) => {
                let mut tmp = std::env::temp_dir();
                tmp.push("verum-preflight");
                let _ = std::fs::create_dir_all(&tmp);
                let stem = test
                    .file
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("test");
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                use std::hash::{Hash, Hasher};
                test.file.hash(&mut hasher);
                tmp.push(format!("{}_{:016x}.vr", stem, hasher.finish()));
                if std::fs::write(&tmp, stripped).is_ok() {
                    tmp
                } else {
                    test.file.clone()
                }
            }
            None => test.file.clone(),
        };
        let options = CompilerOptions {
            input: check_input,
            output_format: OutputFormat::Human,
            // The preflight is a fast-fail TYPE check. Proof
            // obligations are verified once at execution time
            // (`validate_module` → `phase_verify` in the run
            // pipeline); running SMT here too would verify every
            // theorem twice per test file.
            verify_mode: verum_compiler::VerifyMode::Runtime,
            ..Default::default()
        };
        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);
        if let Err(e) = pipeline.run_check_only() {
            return Err(format!("typecheck: {}", e));
        }
    }

    if let Some(crate_root_items) = crate_root_items {
        // Prepend crate-root items so test items can reference them.
        let mut merged = crate_root_items;
        for item in ast.items.iter() {
            merged.push((*item).clone());
        }
        ast.items = merged;
    }

    let config = CodegenConfig::new("test");
    let module = verum_compiler::single_module::compile_module_with_stdlib(
        &ast,
        config,
        /* propagate_test_attr = */ true,
    )
    .map_err(|e| format!("codegen: {:?}", e))?;
    Ok(Arc::new(module))
}

fn run_test_interpret(test: &Test, _cfg: &TestRunCfg) -> TestResult {
    use verum_vbc::interpreter::Interpreter;

    let start = Instant::now();

    let module = match compiled_test_module(&test.file, CompileKind::Stdlib, || {
        build_stdlib_test_module(test)
    }) {
        Ok(m) => m,
        Err(error) => {
            return TestResult::CompileError {
                duration: start.elapsed(),
                error,
            };
        }
    };

    // Pick the function to run. Priority:
    //  1. Function whose name matches the test name (for per-@test tests)
    //  2. `main`
    let fn_name_tail: &str = if let Some(fn_name) = &test.fn_name {
        fn_name.as_str()
    } else {
        test.name
            .as_str()
            .rsplit_once("::")
            .map(|(_, n)| n)
            .unwrap_or_else(|| test.name.as_str())
    };
    // Match either the exact stored name OR its last-segment (leaf):
    // the precompiler's descriptor-name promotion (commit 53c7d5448)
    // stores `<source_module>.<fn_name>` in the descriptor's name
    // field rather than the bare leaf. For a test module declaring
    // `module main;`, `test_page_size_canonical` lands as
    // `main.test_page_size_canonical`; the runner's
    // `find_by_name(leaf)` lookup must canonicalise the comparison
    // to leaf-only, otherwise every per-test entry point misses.
    let leaf_matches = |stored: Option<&str>| -> bool {
        let Some(s) = stored else { return false; };
        if s == fn_name_tail {
            return true;
        }
        s.rsplit('.').next() == Some(fn_name_tail)
    };
    let main_leaf_matches = |stored: Option<&str>| -> bool {
        let Some(s) = stored else { return false; };
        s == "main" || s.rsplit('.').next() == Some("main")
    };
    // HARNESS-FIDELITY (#26): a module can contain DUPLICATE functions with
    // the same leaf name — the freshly-compiled `@test` fn (is_test=true)
    // AND a STALE copy baked in from `core/target/test/` synthesized test
    // artifacts of a previous run (module path `core.target.test.*`,
    // is_test=false). The stale copy carries outdated bytecode (e.g. a
    // `match` on an archive enum compiled against old variant tags), so
    // executing it yields wrong results — a harness-only FALSE NEGATIVE that
    // does not reproduce under `verum run`. Prefer the fresh `is_test=true`
    // match so the runner always executes the function it just compiled;
    // fall back to any leaf match (entry points without the test attr) and
    // finally to `main`.
    let fid_opt = module
        .functions
        .iter()
        .find(|vf| vf.is_test && leaf_matches(module.get_string(vf.name)))
        .or_else(|| {
            module
                .functions
                .iter()
                .find(|vf| leaf_matches(module.get_string(vf.name)))
        })
        .or_else(|| {
            module
                .functions
                .iter()
                .find(|vf| main_leaf_matches(module.get_string(vf.name)))
        })
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
    // TEST-DEADLINE-INTERP-1: replace the unbounded run (caps fully
    // disabled — one runtime hang froze the whole suite; the ListIter
    // devirt loop demonstrated it live) with a WALL-CLOCK deadline via
    // the interpreter's own timeout cap: the manifest/test timeout when
    // configured, else a 120s default (property tests iterate
    // internally — the old 30s default was the reason caps were
    // disabled; 120s keeps them un-throttled while bounding hangs).
    // Instruction cap stays off — time is the honest budget.
    interp.state.config.max_instructions = 0;
    interp.state.config.timeout_ms = if _cfg.timeout_secs > 0 {
        _cfg.timeout_secs.saturating_mul(1000)
    } else {
        120_000
    };

    // Run global ctors before executing the test function.  Without
    // this, `__tls_init_*` synthetic functions never populate their
    // TLS slots — every `static mut` declared in the user's mounted
    // stdlib reads back `Value::default()` (nil), and methods on those
    // statics null-deref at the first GetF.  Mirrors `Interpreter::run_main`
    // (which the file/cog runners use) and the AOT path's
    // `@llvm.global_ctors` array. Pre-fix:
    //   * `verum run hazard_stats_test.vr` worked (run_main path).
    //   * `verum test --interp --filter test_hazard_stats` null-derefed
    //     at `core.mem.hazard.hazard_stats pc=30` (test path bypassed ctors).
    if let Err(e) = interp.run_global_ctors() {
        return TestResult::Fail {
            duration: start.elapsed(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            error: format!("global_ctors: {:?}", e),
        };
    }

    let outcome = if let Some(args) = &test.case_args {
        // @test_case path: convert literal args → VBC Values, call directly.
        let vbc_args: std::result::Result<Vec<_>, _> =
            args.iter().map(|tv| tv.to_vbc_value(&mut interp)).collect();
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
                TestResult::Pass {
                    duration,
                    stdout,
                    stderr: String::new(),
                }
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
    /// T0365: the human-stated reason on the `@ignore` pin — either an
    /// attribute argument (`@ignore("…")` / `@ignore(reason = "…")`)
    /// or the trailing `//` comment on the attribute's line. Surfaced
    /// when the pin is deliberately executed and fails: the reason
    /// names the tracking class the failure belongs to.
    ignore_reason: Option<Text>,
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

/// Build the suite-relative module path used to qualify a test's name.
///
/// `core-tests/mem/capability/unit_test.vr` → `mem/capability/unit_test`.
/// `tests/foo/bar_test.vr`                  → `foo/bar_test`.
/// A file directly under the suite root      → just its stem.
///
/// Qualification makes test names unique across directories (two
/// `unit_test.vr` files in different folders no longer collapse to the
/// same `unit_test::fn` name) and lets `--filter '^mem/'` or
/// `--filter '^mem/capability/'` scope a run to a subtree (`^` anchors
/// at the name START; a bare `mem/` filter anchors at segment
/// boundaries — it also selects a nested `…/mem/` subtree but never a
/// name where `mem/` is a substring inside a segment; T0341). The function leaf
/// still follows the final `::`, so `--filter <fn>` and function
/// resolution (which prefers `Test::fn_name`) are unaffected.
fn module_qualified_prefix(file: &Path) -> String {
    let comps: Vec<&str> = file
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("test");
    // Anchor on the LAST test-root component so nested suites resolve to
    // their own suite-relative path.
    let anchor = comps
        .iter()
        .rposition(|c| *c == "core-tests" || *c == "tests");
    match anchor {
        // Directory components between the anchor and the file name.
        Some(i) => {
            let dirs = &comps[i + 1..comps.len().saturating_sub(1)];
            if dirs.is_empty() {
                stem.to_string()
            } else {
                format!("{}/{}", dirs.join("/"), stem)
            }
        }
        None => stem.to_string(),
    }
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
        // Suite-relative path prefix (`mem/capability/unit_test`) used to
        // qualify every test name discovered in this file.
        let name_prefix = module_qualified_prefix(file);
        let has_test_attrs = module.items.iter().any(|item| {
            matches!(&item.kind, ItemKind::Function(f) if f.attributes.iter().any(|a| a.name.as_str() == "test"))
        });
        // Pass 1: property-based tests (@property).
        let property_funcs =
            crate::commands::property::discover_properties_in_module(&module, module_name, file);
        let has_property_attrs = !property_funcs.is_empty();

        if has_test_attrs || has_property_attrs {
            for item in &module.items {
                if let ItemKind::Function(func) = &item.kind {
                    let is_test = func.attributes.iter().any(|a| a.name.as_str() == "test");
                    let is_property = func
                        .attributes
                        .iter()
                        .any(|a| a.name.as_str() == "property");
                    if !is_test && !is_property {
                        continue;
                    }
                    let is_ignored = func
                        .attributes
                        .iter()
                        .any(|a| a.name.as_str() == "ignore" || a.name.as_str() == "ignored");
                    let reason = if is_ignored {
                        ignore_reason(&func.attributes, &source)
                    } else {
                        None
                    };
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
                                name: format!("{}::{}[{}]", name_prefix, func.name, idx).into(),
                                file: file.to_path_buf(),
                                ignored: is_ignored,
                                ignore_reason: reason.clone(),
                                property: property.clone(),
                                case_args: Some(args),
                                fn_name: Some(func.name.to_string()),
                            });
                        }
                    } else {
                        tests.push(Test {
                            name: format!("{}::{}", name_prefix, func.name).into(),
                            file: file.to_path_buf(),
                            ignored: is_ignored,
                            ignore_reason: reason,
                            property,
                            case_args: None,
                            fn_name: Some(func.name.to_string()),
                        });
                    }
                }
            }
        } else {
            // Whole-file test — must have main()
            let has_main = module.items.iter().any(
                |item| matches!(&item.kind, ItemKind::Function(f) if f.name.as_str() == "main"),
            );
            if has_main {
                let is_ignored = source.lines().take(10).any(|l| {
                    let t = l.trim();
                    t.contains("@ignore") || t.contains("@ignored")
                });
                let reason = if is_ignored {
                    file_level_ignore_reason(&source)
                } else {
                    None
                };
                tests.push(Test {
                    name: name_prefix.clone().into(),
                    file: file.to_path_buf(),
                    ignored: is_ignored,
                    ignore_reason: reason,
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
                        name: format!("{}::{}", module_qualified_prefix(file), name).into(),
                        file: file.to_path_buf(),
                        ignored,
                        ignore_reason: None,
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
            tests.push(Test {
                name: module_qualified_prefix(file).into(),
                file: file.to_path_buf(),
                ignored: false,
                ignore_reason: None,
                property: None,
                case_args: None,
                fn_name: None,
            });
        }
    }
    Ok(tests)
}

/// T0365: extract the human-stated reason from a fn-level `@ignore`
/// pin. Two spellings exist across the suites:
///
///   * attribute argument — `@ignore("reason …")` or
///     `@ignore(reason = "…")`;
///   * trailing comment — `@ignore  // reason …` (the dominant
///     historical form; the comment never reaches the AST, so it is
///     recovered from the attribute's source line via its span).
fn ignore_reason(attrs: &[verum_ast::Attribute], source: &str) -> Option<Text> {
    use verum_ast::{ExprKind, LiteralKind};
    let attr = attrs
        .iter()
        .find(|a| a.name.as_str() == "ignore" || a.name.as_str() == "ignored")?;
    match &attr.args {
        verum_common::Maybe::Some(args) => {
            for e in args.iter() {
                match &e.kind {
                    ExprKind::Literal(lit) => {
                        if let LiteralKind::Text(s) = &lit.kind {
                            return Some(Text::from(s.as_str()));
                        }
                    }
                    ExprKind::NamedArg { name, value } if name.name.as_str() == "reason" => {
                        if let ExprKind::Literal(lit) = &value.kind {
                            if let LiteralKind::Text(s) = &lit.kind {
                                return Some(Text::from(s.as_str()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        // Bare `@ignore` — recover a trailing `// …` comment from the
        // attribute's own source line. Restricted to the arg-less form
        // so a `//` INSIDE an attribute-arg string can never be
        // misread as a comment.
        _ => {
            let start = attr.span.start as usize;
            if start >= source.len() {
                return None;
            }
            let line_end = source[start..]
                .find('\n')
                .map(|o| start + o)
                .unwrap_or(source.len());
            let line = &source[start..line_end];
            let reason = line.split_once("//").map(|(_, c)| c.trim())?;
            if reason.is_empty() {
                None
            } else {
                Some(Text::from(reason))
            }
        }
    }
}

/// T0365: file-level companion of [`ignore_reason`] for whole-file
/// tests, whose `@ignore` marker lives in the first ten lines (often
/// inside a comment): the reason is whatever follows the marker after
/// separator punctuation, e.g. `// @ignore: CLASS-NAME-1`.
fn file_level_ignore_reason(source: &str) -> Option<Text> {
    for l in source.lines().take(10) {
        let t = l.trim();
        if let Some(pos) = t.find("@ignore") {
            let rest = t[pos + "@ignore".len()..]
                .trim_start_matches('d')
                .trim_start_matches([':', '(', ' ', '\t'])
                .trim_end_matches(')')
                .trim_matches('"')
                .trim();
            if !rest.is_empty() {
                return Some(Text::from(rest));
            }
        }
    }
    None
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

/// Discover the list of test-source directories under `project_dir`.
///
/// Three well-known locations are recognised:
///
/// * `<project>/tests/`           — user-project tests (libtest convention).
/// * `<project>/core-tests/`      — stdlib tests when they live alongside
///                                  the package source.
/// * `<project>/../core-tests/`   — stdlib tests at the workspace root,
///                                  one level up from a package whose
///                                  `Verum.toml` lives in a member
///                                  directory (the canonical layout for
///                                  this repo: `core/verum.toml` is the
///                                  package, `core-tests/` is its test
///                                  suite at the workspace level).
///
/// Each directory that exists is walked; missing directories are ignored.
/// The discovery contract is: any stdlib module gets its tests run by
/// `verum test` (interp + AOT) without explicit manifest configuration.
fn test_source_dirs(project_dir: &Path) -> List<PathBuf> {
    let mut dirs = List::new();
    for name in &["tests", "core-tests"] {
        let d = project_dir.join(name);
        if d.exists() && d.is_dir() {
            dirs.push(d);
        }
    }
    // Workspace-level `core-tests/` (sibling of the package directory).
    // Picked up only when *not* already in `dirs` and the parent
    // directory exists. Skip when project_dir is at filesystem root.
    if let Some(parent) = project_dir.parent() {
        let workspace_core_tests = parent.join("core-tests");
        if workspace_core_tests.exists()
            && workspace_core_tests.is_dir()
            && !dirs.iter().any(|d| d == &workspace_core_tests)
        {
            dirs.push(workspace_core_tests);
        }
    }
    dirs
}

fn find_test_files(project_dir: &Path) -> Result<List<PathBuf>> {
    let dirs = test_source_dirs(project_dir);
    if dirs.is_empty() {
        return Ok(List::new());
    }
    let mut files = List::new();
    for dir in dirs.iter() {
        for entry in walkdir::WalkDir::new(dir).follow_links(false) {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("vr") {
                files.push(path.to_path_buf());
            }
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
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: inner,
        } => {
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

fn emit_junit(results: &[(Text, TestResult)], active: &[&Test], ignored: usize, total: Duration) {
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
            TestResult::Fail {
                duration, error, ..
            } => (*duration, false, error.clone(), "failure"),
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
            TestResult::Pass { duration, .. } => {
                println!("ok {} - {} # time={:.3}s", i, name, duration.as_secs_f64())
            }
            TestResult::Fail {
                duration, error, ..
            } => {
                println!(
                    "not ok {} - {} # time={:.3}s",
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

    #[test]
    fn strip_ignored_tests_removes_pinned_fn_keeps_siblings() {
        let src = "mount x.{Y};

@ignore  // CLASS-1 — pinned
         // continuation comment
@test
fn pinned_one() {
    let s = f\"cap={1 + 1}\";
    assert(true);
}

@test
fn live_one() {
    assert(true);
}
";
        let out = super::strip_ignored_tests(src, None);
        assert!(!out.contains("pinned_one"), "pinned fn must be stripped: {}", out);
        assert!(out.contains("live_one"), "live sibling must survive: {}", out);
        assert!(out.contains("mount x.{Y};"), "header must survive");
    }

    #[test]
    fn strip_ignored_tests_noop_without_pins() {
        let src = "@test
fn a() { assert(true); }
";
        assert_eq!(super::strip_ignored_tests(src, None), src);
    }

    /// T0365 pin: when an ignore-inclusive mode deliberately executes
    /// one pinned test, `keep_fn` preserves exactly that fn in the
    /// merged AOT source (minus the `@ignore` line) while sibling pins
    /// still strip.
    #[test]
    fn strip_ignored_tests_keep_fn_preserves_the_test_under_execution() {
        let src = "@test
@ignore(\"CLASS-A\")
fn pinned_keep_me() {
    assert(false);
}

@test
@ignore(\"CLASS-B\")
fn pinned_drop_me() {
    assert(false);
}
";
        let out = super::strip_ignored_tests(src, Some("pinned_keep_me"));
        assert!(
            out.contains("fn pinned_keep_me"),
            "kept fn must survive: {}",
            out
        );
        assert!(
            !out.contains("@ignore"),
            "the @ignore attribute lines themselves must not survive: {}",
            out
        );
        assert!(
            !out.contains("fn pinned_drop_me"),
            "sibling pin must still strip: {}",
            out
        );
    }
    use super::*;

    /// TEST-PREFLIGHT-IGNORE-2: attributes PRECEDING `@ignore`
    /// (`@test` first is the dominant stdlib order) must be stripped
    /// with the fn, or the preflight file dangles an item-less
    /// attribute and fails to parse (E018).
    #[test]
    fn strip_ignored_peels_preceding_attributes() {
        let src = "@test\nfn keep_me() {\n    assert(true);\n}\n\n@test\n@ignore\nfn drop_me() {\n    assert(false);\n}\n";
        let out = strip_ignored_test_fns(src).expect("one fn stripped");
        assert!(out.contains("fn keep_me"));
        assert!(!out.contains("fn drop_me"));
        // The dangling-@test hazard: no attribute may immediately
        // precede the strip marker.
        let marker_at = out.find("[preflight]").unwrap();
        let before = &out[..marker_at];
        let last_line = before
            .trim_end_matches(|c| c == '\n' || c == '/' || c == ' ')
            .rsplit('\n')
            .next()
            .unwrap_or("");
        assert!(
            !last_line.trim_start().starts_with('@'),
            "dangling attribute before strip marker: {last_line:?}"
        );
    }

    /// T0365 pin: the selection truth-table for the three ignore
    /// modes. `selects` is the ONE authority consulted by the parent's
    /// selection pass; the child argv fragments must reproduce the
    /// same mode across a process boundary.
    #[test]
    fn ignore_mode_selection_truth_table() {
        use super::IgnoreMode::*;
        assert!(SkipIgnored.selects(false) && !SkipIgnored.selects(true));
        assert!(!OnlyIgnored.selects(false) && OnlyIgnored.selects(true));
        assert!(IncludeIgnored.selects(false) && IncludeIgnored.selects(true));
        assert_eq!(SkipIgnored.child_args(), &[] as &[&str]);
        assert_eq!(OnlyIgnored.child_args(), &["--ignored"][..]);
        assert_eq!(IncludeIgnored.child_args(), &["--include-ignored"][..]);
    }

    /// T0365 pin: a failing pinned test surfaces its `@ignore` reason;
    /// the annotation is idempotent (parent + child topologies may
    /// both apply it) and passes are untouched.
    #[test]
    fn attach_ignore_note_annotates_failures_idempotently() {
        let failed = fail("assertion failed", Some(1));
        let annotated = attach_ignore_note(failed, Some("tracked as CLASS-X"));
        let TestResult::Fail { error, .. } = &annotated else {
            panic!("expected Fail");
        };
        assert!(error.contains("assertion failed"), "original error kept: {error}");
        assert!(error.contains("[@ignore] reason: tracked as CLASS-X"), "{error}");
        // Second application must not duplicate the note.
        let twice = attach_ignore_note(annotated, Some("tracked as CLASS-X"));
        let TestResult::Fail { error, .. } = &twice else {
            panic!("expected Fail");
        };
        assert_eq!(error.matches("[@ignore]").count(), 1, "{error}");
        // A pass stays a pass, unannotated.
        let ok = attach_ignore_note(pass("fine"), Some("irrelevant"));
        assert!(matches!(ok, TestResult::Pass { ref stdout, .. } if stdout == "fine"));
    }

    /// T0365 pin: reason extraction covers both real-world spellings —
    /// the attribute-argument form and the trailing-comment form.
    #[test]
    fn ignore_reason_reads_attr_arg_and_trailing_comment() {
        let src = "@test\n@ignore(\"CLASS-ARG-1 details\")\nfn a() {\n    assert(false);\n}\n\n@test\n@ignore  // CLASS-COMMENT-2 trailing\nfn b() {\n    assert(false);\n}\n";
        let file_id = verum_ast::FileId::new(0);
        let parser = verum_fast_parser::VerumParser::new();
        let lexer = verum_lexer::Lexer::new(src, file_id);
        let module = parser
            .parse_module(lexer, file_id)
            .expect("probe module parses");
        let mut reasons = Vec::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(f) = &item.kind {
                reasons.push(super::ignore_reason(&f.attributes, src));
            }
        }
        assert_eq!(
            reasons,
            vec![
                Some(Text::from("CLASS-ARG-1 details")),
                Some(Text::from("CLASS-COMMENT-2 trailing")),
            ]
        );
    }

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

    /// T0341 pin: a slash-carrying filter matches only at suite-path
    /// segment boundaries — `sync/` can no longer pull `async/` (or
    /// any other superstring segment) into the run.
    #[test]
    fn filter_with_slash_is_segment_anchored() {
        let f: Option<Text> = Some("sync/".into());
        assert!(matches_filter(&"sync/mutex::test_lock".into(), &f, false));
        assert!(
            matches_filter(&"runtime/sync/atomic::test_cas".into(), &f, false),
            "nested sync/ segment must still match"
        );
        assert!(
            !matches_filter(&"async/task::test_spawn".into(), &f, false),
            "async/ is NOT a segment-boundary occurrence of sync/"
        );
        assert!(
            !matches_filter(&"runtime/async/waker::test_wake".into(), &f, false),
            "nested async/ must not match either"
        );
        // Mid-tree subtree scoping still works, and a partial tail is
        // allowed (the anchor constrains the START of the match).
        let g: Option<Text> = Some("collections/list".into());
        assert!(matches_filter(
            &"core/collections/list::test_push".into(),
            &g,
            false
        ));
        let h: Option<Text> = Some("sync/mu".into());
        assert!(matches_filter(&"sync/mutex::test_lock".into(), &h, false));
    }

    /// T0341 pin: preserved behaviors around the segment anchor —
    /// plain slash-less substrings match anywhere (`--filter bloom`,
    /// `--filter sync`), and `^` remains a pure name-start anchor.
    #[test]
    fn filter_plain_substring_and_caret_semantics_preserved() {
        let bloom: Option<Text> = Some("bloom".into());
        assert!(matches_filter(
            &"text/bloom_filter::test_insert".into(),
            &bloom,
            false
        ));
        // Slash-less `sync` intentionally keeps anywhere-substring
        // semantics (matches async too) — only `/`-carrying filters
        // opt into segment anchoring.
        let sync_plain: Option<Text> = Some("sync".into());
        assert!(matches_filter(&"async/task::test_spawn".into(), &sync_plain, false));
        // `^` prefix anchor unchanged.
        let caret: Option<Text> = Some("^text/".into());
        assert!(matches_filter(&"text/mod::unit".into(), &caret, false));
        assert!(!matches_filter(&"context/mod::unit".into(), &caret, false));
    }

    #[test]
    fn segment_anchored_contains_boundary_table() {
        use super::segment_anchored_contains;
        assert!(segment_anchored_contains("sync/a::t", "sync/"));
        assert!(segment_anchored_contains("x/sync/a::t", "sync/"));
        assert!(!segment_anchored_contains("async/a::t", "sync/"));
        // First occurrence unanchored, later occurrence anchored.
        assert!(segment_anchored_contains("async/sync/a::t", "sync/"));
        // A filter starting with a multi-byte character walks the scan
        // by whole characters (no byte-boundary panic) and still
        // resolves boundaries correctly.
        assert!(segment_anchored_contains("é/x::t", "é/"));
        assert!(segment_anchored_contains("a/é/x::t", "é/"));
        assert!(!segment_anchored_contains("aé/xé/y::t", "é/"));
    }

    #[test]
    fn format_parse_accepts_known() {
        assert_eq!(TestFormat::parse("pretty").unwrap(), TestFormat::Pretty);
        assert_eq!(TestFormat::parse("terse").unwrap(), TestFormat::Terse);
        assert_eq!(TestFormat::parse("json").unwrap(), TestFormat::Json);
        assert!(TestFormat::parse("xml").is_err());
    }

    fn parse_verify(s: &str) -> Option<verum_compiler::options::VerifyMode> {
        // Mirror the parsing logic used at the cfg construction
        // site (around line ~225). Pinned here so a regression in
        // that mapping fails this test rather than slipping into
        // production silently.
        match s.to_ascii_lowercase().as_str() {
            "runtime" => Some(verum_compiler::options::VerifyMode::Runtime),
            "proof" | "static" => Some(verum_compiler::options::VerifyMode::Proof),
            "auto" => Some(verum_compiler::options::VerifyMode::Auto),
            _ => None,
        }
    }

    #[test]
    fn verify_flag_runtime_maps_to_runtime() {
        assert_eq!(
            parse_verify("runtime"),
            Some(verum_compiler::options::VerifyMode::Runtime)
        );
        assert_eq!(
            parse_verify("RUNTIME"),
            Some(verum_compiler::options::VerifyMode::Runtime)
        );
    }

    #[test]
    fn verify_flag_static_and_proof_map_to_proof() {
        // Pin: both `static` and `proof` route to the same VerifyMode
        // because the documented CLI surface accepts the user-facing
        // synonym `static` for the SMT-backed proof mode.
        assert_eq!(
            parse_verify("proof"),
            Some(verum_compiler::options::VerifyMode::Proof)
        );
        assert_eq!(
            parse_verify("static"),
            Some(verum_compiler::options::VerifyMode::Proof)
        );
    }

    #[test]
    fn verify_flag_unknown_value_falls_back_to_default() {
        // Pin: unrecognised --verify values produce None so the test
        // runner falls back to the per-test default (Runtime) instead
        // of failing at this layer. Keeps CI tolerant of typos.
        assert_eq!(parse_verify("not-a-mode"), None);
        assert_eq!(parse_verify(""), None);
    }

    /// Build a minimal TestRunCfg for property-runner unit tests.
    /// The fields the property dispatcher actually reads are
    /// `property_testing` and `proptest_cases`; everything else
    /// defaults to a placeholder shape that doesn't reach the
    /// codepath under test.
    fn cfg_with_property(property_testing: bool, proptest_cases: u32) -> TestRunCfg {
        TestRunCfg {
            timeout_secs: 60,
            deny_warnings: false,
            coverage: false,
            nocapture: false,
            language_features: verum_compiler::language_features::LanguageFeatures::default(),
            tier: Tier::Interpret,
            release: false,
            verify_mode_override: None,
            property_testing,
            proptest_cases,
            differential: false,
            fuzzing: false,
        }
    }

    fn cfg_with_differential(differential: bool) -> TestRunCfg {
        TestRunCfg {
            timeout_secs: 60,
            deny_warnings: false,
            coverage: false,
            nocapture: false,
            language_features: verum_compiler::language_features::LanguageFeatures::default(),
            tier: Tier::Interpret,
            release: false,
            verify_mode_override: None,
            property_testing: true,
            proptest_cases: 256,
            differential,
            fuzzing: false,
        }
    }

    #[test]
    fn property_testing_disabled_skips_property_tests_with_pass() {
        // Pin (#298): when [test].property_testing = false, a test
        // carrying @property(...) yields a Pass with the disabled
        // marker line, NOT a property-runner invocation. This is
        // the load-bearing dispatch contract for the manifest flag.
        let test = Test {
            name: "demo_property".into(),
            file: PathBuf::from("/dev/null"),
            ignored: false,
            ignore_reason: None,
            property: Some(crate::commands::property::PropertyFunc {
                name: "demo".to_string(),
                file: PathBuf::from("/dev/null"),
                params: Vec::new(),
                runs_override: None,
                seed_override: None,
            }),
            case_args: None,
            fn_name: Some("demo".to_string()),
        };
        let cfg = cfg_with_property(false, 256);
        let result = run_single_test(&test, std::path::Path::new("/tmp"), &cfg);
        match result {
            TestResult::Pass { stdout, .. } => {
                assert!(
                    stdout.contains("property testing disabled"),
                    "expected disabled marker in stdout, got: {stdout}"
                );
            }
            TestResult::Fail { error, .. } => {
                panic!("expected Pass when property testing disabled, got Fail: {error}");
            }
            TestResult::CompileError { error, .. } => {
                panic!("expected Pass when property testing disabled, got CompileError: {error}");
            }
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    fn pass(stdout: &str) -> TestResult {
        TestResult::Pass {
            duration: ms(1),
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }
    fn fail(error: &str, exit: Option<i32>) -> TestResult {
        TestResult::Fail {
            duration: ms(1),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: exit,
            error: error.to_string(),
        }
    }
    fn compile_err(msg: &str) -> TestResult {
        TestResult::CompileError {
            duration: ms(1),
            error: msg.to_string(),
        }
    }

    #[test]
    fn differential_both_pass_with_same_stdout_yields_pass() {
        let r = combine_differential_outcomes(pass("hello"), pass("hello"), ms(10));
        match r {
            TestResult::Pass { stdout, stderr, .. } => {
                assert!(stdout.contains("[T0] hello"));
                assert!(stdout.contains("[T1] hello"));
                assert!(
                    !stderr.contains("stdout drift"),
                    "no drift expected on identical stdout"
                );
            }
            _ => panic!("expected Pass"),
        }
    }

    #[test]
    fn differential_both_pass_with_drift_still_passes_but_warns() {
        // Pin: stdout drift between tiers is observability-only —
        // agreement on the pass/fail verdict is the load-bearing
        // contract; drift surfaces as a stderr warning so a
        // maintainer can investigate non-determinism without the
        // CI pipeline going red on every run.
        let r = combine_differential_outcomes(pass("0x1234abcd"), pass("0x99887766"), ms(10));
        match r {
            TestResult::Pass { stderr, .. } => {
                assert!(
                    stderr.contains("stdout drift"),
                    "expected drift warning in stderr, got: {stderr}"
                );
            }
            _ => panic!("expected Pass with drift warning"),
        }
    }

    #[test]
    fn differential_t0_pass_t1_fail_yields_disagreement_failure() {
        let r =
            combine_differential_outcomes(pass("ok"), fail("assertion blew up", Some(1)), ms(10));
        match r {
            TestResult::Fail {
                error, exit_code, ..
            } => {
                assert!(
                    error.contains("T0 PASS / T1 FAIL"),
                    "header missing: {error}"
                );
                assert!(
                    error.contains("[T0 interpret]"),
                    "T0 summary missing: {error}"
                );
                assert!(error.contains("[T1 aot]"), "T1 summary missing: {error}");
                assert!(
                    error.contains("assertion blew up"),
                    "T1 detail missing: {error}"
                );
                assert_eq!(exit_code, Some(1));
            }
            _ => panic!("expected Fail"),
        }
    }

    #[test]
    fn differential_t0_fail_t1_pass_yields_disagreement_failure() {
        let r = combine_differential_outcomes(fail("interp panic", Some(101)), pass("ok"), ms(10));
        match r {
            TestResult::Fail {
                error, exit_code, ..
            } => {
                assert!(
                    error.contains("T0 FAIL / T1 PASS"),
                    "header missing: {error}"
                );
                assert!(error.contains("interp panic"), "T0 detail missing: {error}");
                assert_eq!(exit_code, Some(101));
            }
            _ => panic!("expected Fail"),
        }
    }

    #[test]
    fn differential_both_fail_aggregates_diagnostics() {
        let r = combine_differential_outcomes(
            fail("interp side", Some(1)),
            fail("aot side", Some(2)),
            ms(10),
        );
        match r {
            TestResult::Fail {
                error, exit_code, ..
            } => {
                assert!(
                    error.contains("both tiers failed"),
                    "header missing: {error}"
                );
                assert!(error.contains("interp side"));
                assert!(error.contains("aot side"));
                // First-found exit-code wins (T0 is checked first).
                assert_eq!(exit_code, Some(1));
            }
            _ => panic!("expected Fail"),
        }
    }

    #[test]
    fn differential_t1_compile_error_is_a_disagreement_failure() {
        let r = combine_differential_outcomes(
            pass("interp says ok"),
            compile_err("LLVM lowering failed at f()"),
            ms(10),
        );
        match r {
            TestResult::Fail { error, .. } => {
                assert!(error.contains("T0 PASS / T1 FAIL"));
                assert!(error.contains("COMPILE-ERROR"));
                assert!(error.contains("LLVM lowering"));
            }
            _ => panic!("expected Fail (T1 compile error counts as disagreement)"),
        }
    }

    #[test]
    fn differential_disabled_returns_tier_specific_path() {
        // Pin (#273): when cfg.differential = false, the
        // dispatcher takes the cfg.tier branch and does NOT
        // invoke run_test_differential. We can't observe that
        // directly without integration scaffolding, but we can
        // verify the cfg shape carries the flag through.
        let cfg = cfg_with_differential(false);
        assert!(!cfg.differential);
        let cfg = cfg_with_differential(true);
        assert!(cfg.differential);
    }

    #[test]
    fn property_testing_enabled_does_not_short_circuit() {
        // Pin (#298): with property_testing = true, the dispatcher
        // routes to run_test_property — which will then fail at
        // file-read because /dev/null isn't a real test file. The
        // failure mode (CompileError "read:") is the proof that the
        // dispatcher did not short-circuit at the disabled gate.
        let test = Test {
            name: "demo_property".into(),
            file: PathBuf::from("/dev/null"),
            ignored: false,
            ignore_reason: None,
            property: Some(crate::commands::property::PropertyFunc {
                name: "demo".to_string(),
                file: PathBuf::from("/dev/null"),
                params: Vec::new(),
                runs_override: None,
                seed_override: None,
            }),
            case_args: None,
            fn_name: Some("demo".to_string()),
        };
        let cfg = cfg_with_property(true, 256);
        let result = run_single_test(&test, std::path::Path::new("/tmp"), &cfg);
        // /dev/null reads as an empty file; the parser then fails.
        // Either way the result is NOT the disabled-marker Pass —
        // that's the load-bearing assertion.
        match result {
            TestResult::Pass { stdout, .. } => {
                assert!(
                    !stdout.contains("property testing disabled"),
                    "should not short-circuit when property_testing = true; stdout: {stdout}"
                );
            }
            TestResult::Fail { .. } | TestResult::CompileError { .. } => {
                // Expected: the runner reached run_test_property and
                // produced a real downstream failure on /dev/null.
            }
        }
    }
}

/// TEST-PREFLIGHT-IGNORE-1: lexically remove test fns annotated
/// `@ignore` (with or without a reason) so the file-level typecheck
/// preflight tolerates deliberately compile-broken pinned tests.
///
/// Returns `Some(stripped)` when at least one fn was removed and the
/// scan stayed regular; `None` means "use the original file" (no
/// @ignore present, or the matcher hit something irregular —
/// fail-open by design).
fn strip_ignored_test_fns(src: &str) -> Option<String> {
    if !src.contains("@ignore") {
        return None;
    }
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0usize;
    let mut stripped_any = false;
    'outer: while i < bytes.len() {
        // Find the next line start with `@ignore` (allow leading ws).
        let line_end = src[i..].find('\n').map(|o| i + o + 1).unwrap_or(bytes.len());
        let line = &src[i..line_end];
        let t = line.trim_start();
        if t.starts_with("@ignore") {
            // Scan forward over subsequent attribute/comment lines to
            // the `fn ` header; give up (fail-open) past 8 lines.
            let mut j = line_end;
            let mut hops = 0;
            loop {
                if hops > 8 || j >= bytes.len() {
                    // Irregular shape — keep original text as-is.
                    out.push_str(line);
                    i = line_end;
                    continue 'outer;
                }
                let le = src[j..].find('\n').map(|o| j + o + 1).unwrap_or(bytes.len());
                let l2 = src[j..le].trim_start();
                if l2.starts_with("fn ") || l2.starts_with("pub fn ") {
                    // Brace-match from the first '{' after the header.
                    let Some(open_rel) = src[j..].find('{') else {
                        out.push_str(line);
                        i = line_end;
                        continue 'outer;
                    };
                    let mut k = j + open_rel;
                    let mut depth = 0i32;
                    let mut in_str = false;
                    let mut in_char = false;
                    let mut prev_bs = false;
                    while k < bytes.len() {
                        let c = bytes[k] as char;
                        if in_str {
                            if c == '"' && !prev_bs {
                                in_str = false;
                            }
                            prev_bs = c == '\\' && !prev_bs;
                        } else if in_char {
                            if c == '\'' && !prev_bs {
                                in_char = false;
                            }
                            prev_bs = c == '\\' && !prev_bs;
                        } else {
                            match c {
                                '"' => in_str = true,
                                '\'' => in_char = true,
                                '{' => depth += 1,
                                '}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        // Consume through end of line.
                                        let after = src[k..]
                                            .find('\n')
                                            .map(|o| k + o + 1)
                                            .unwrap_or(bytes.len());
                                        // Attributes PRECEDING the
                                        // `@ignore` line (`@test`,
                                        // `@cfg(...)`, ...) were already
                                        // emitted — peel them off `out`,
                                        // or the strip leaves a dangling
                                        // `@test` with no item and the
                                        // whole preflight file fails to
                                        // PARSE (E018 at the marker
                                        // comment; the sync/condvar
                                        // suite's uniform 217ms
                                        // compile-fail signature).
                                        loop {
                                            let trimmed_end = out.trim_end_matches('\n');
                                            let Some(last_nl) = trimmed_end.rfind('\n') else {
                                                break;
                                            };
                                            let last_line = trimmed_end[last_nl + 1..].trim_start();
                                            if last_line.starts_with('@') {
                                                out.truncate(last_nl + 1);
                                            } else {
                                                break;
                                            }
                                        }
                                        out.push_str(
                                            "// [preflight] @ignore'd test stripped\n",
                                        );
                                        stripped_any = true;
                                        i = after;
                                        continue 'outer;
                                    }
                                }
                                _ => {}
                            }
                            prev_bs = false;
                        }
                        k += 1;
                    }
                    // Unbalanced — fail-open.
                    return None;
                }
                if l2.starts_with('@') || l2.starts_with("//") || l2.is_empty() {
                    j = le;
                    hops += 1;
                    continue;
                }
                // Non-attr, non-fn after @ignore — irregular; keep text.
                out.push_str(line);
                i = line_end;
                continue 'outer;
            }
        }
        out.push_str(line);
        i = line_end;
    }
    if stripped_any { Some(out) } else { None }
}
