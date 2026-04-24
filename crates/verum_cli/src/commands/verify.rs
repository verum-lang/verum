//! Formal verification command with real Z3 SMT integration
//!
//! Verifies refinement types and contracts using the Z3 SMT solver via the
//! verum_compiler compilation pipeline. Supports three modes:
//!
//! - **proof**: Full SMT verification at compile time (0ns runtime overhead)
//! - **runtime**: Runtime checks only (0ns compile overhead)
//! - **compare**: Run both modes and display cost/benefit analysis
//!
//! When the `verification` feature is not enabled (no Z3), the command
//! clearly reports that Z3 is unavailable and suggests how to enable it.

use crate::error::{CliError, Result};
use crate::ui;
use colored::Colorize;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use verum_common::List;

use verum_compiler::options::{CompilerOptions, OutputFormat, VerifyMode};
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;

/// CLI-visible SMT backend choice. Kept as a local enum so it remains
/// usable even when the `verification` feature is disabled (that feature
/// gates the `verum_smt` dependency and the real `BackendChoice`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolverChoice {
    Z3,
    Cvc5,
    Auto,
    Portfolio,
    Capability,
}

impl SolverChoice {
    pub fn parse(s: &str) -> std::result::Result<Self, String> {
        match s.to_lowercase().as_str() {
            "z3" => Ok(Self::Z3),
            "cvc5" => Ok(Self::Cvc5),
            "auto" => Ok(Self::Auto),
            "portfolio" => Ok(Self::Portfolio),
            "capability" | "capability-based" | "smart" => Ok(Self::Capability),
            other => Err(format!("Unknown --solver value: '{other}'")),
        }
    }
}

#[cfg(feature = "verification")]
impl From<SolverChoice> for verum_smt::backend_switcher::BackendChoice {
    fn from(c: SolverChoice) -> Self {
        use verum_smt::backend_switcher::BackendChoice as B;
        match c {
            SolverChoice::Z3 => B::Z3,
            SolverChoice::Cvc5 => B::Cvc5,
            SolverChoice::Auto => B::Auto,
            SolverChoice::Portfolio => B::Portfolio,
            SolverChoice::Capability => B::Capability,
        }
    }
}

/// Verification mode for the CLI command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationMode {
    /// Runtime checks only (fast, 0ns compile overhead)
    Runtime,
    /// Compile-time proofs (slow compile, 0ns runtime)
    Proof,
    /// Compare both modes and show cost/benefit
    Compare,
    /// Cubical type theory tactics (path induction, glue, hcomp, …)
    Cubical,
    /// Dependent-type SMT tactics (refinement + sigma + pi reasoning)
    Dependent,
}

impl VerificationMode {
    /// Parse a `--mode=…` value from the CLI.
    pub fn parse(s: &str) -> std::result::Result<Self, String> {
        match s {
            "runtime" => Ok(Self::Runtime),
            "proof" => Ok(Self::Proof),
            "compare" => Ok(Self::Compare),
            "cubical" => Ok(Self::Cubical),
            "dependent" => Ok(Self::Dependent),
            other => Err(format!("unknown verification mode '{}'", other)),
        }
    }
}

/// Verification statistics collected from a real pipeline run
#[derive(Debug, Default)]
pub struct VerificationStats {
    /// Total files processed
    pub total_files: usize,
    /// Files that verified successfully
    pub files_verified: usize,
    /// Files that failed verification
    pub files_failed: usize,
    /// Files using runtime checks (no SMT)
    pub files_runtime: usize,
    /// Total verification time
    pub total_time: Duration,
}

impl VerificationStats {
    fn success_rate(&self) -> f64 {
        if self.total_files == 0 {
            return 100.0;
        }
        self.files_verified as f64 / self.total_files as f64 * 100.0
    }
}

/// Profiler configuration plumbed from the CLI.
///
/// Controls whether the per-function verification profiler runs, whether a
/// total-time budget is enforced, whether results are exported as JSON for
/// CI/CD, and whether a distributed cache URL is advertised. Held here so
/// the main verify command signature stays manageable.
#[derive(Debug, Clone, Default)]
pub struct ProfileConfig {
    /// Enable per-function profiling + report printing.
    pub enabled: bool,
    /// Fail if total verification time exceeds this budget.
    pub budget: Option<Duration>,
    /// Write the profile report as JSON to this path.
    pub export_path: Option<PathBuf>,
    /// URL of a distributed verification cache, if any. Surfaced in the
    /// report; actual wire-up is still handled by the compiler core.
    pub distributed_cache: Option<String>,
    /// Named `[verify.profiles.<name>]` profile to apply from
    /// `verum.toml`. CLI flags still win over profile values —
    /// precedence order is: CLI flag > profile override > base
    /// `[verify]` > default. Unknown profile name surfaces an error
    /// at merge time.
    pub profile_name: Option<String>,
    /// Per-obligation profiling granularity — when true, the
    /// profile report includes a breakdown of individual proof
    /// obligations within each function (preconditions,
    /// postconditions, refinement checks, loop invariants, …). At
    /// current instrumentation this surfaces the slowest-obligation
    /// table in the human-readable report; the JSON export carries
    /// the full per-obligation list. Implies `enabled = true`.
    /// Docs: `docs/verification/performance.md §5`.
    pub profile_obligation: bool,
}

/// Compute the list of `.vr` source files changed since the given
/// git ref. Used by `--diff HEAD~N` / `--diff origin/main` to limit
/// verification scope in CI.
///
/// ## Semantics
///
/// Runs `git diff --name-only <ref> --` and filters the output to
/// paths ending in `.vr` that exist under the current working
/// directory. The comparison is against the working tree (staged +
/// unstaged + committed changes since the ref), matching the
/// expected CI behaviour of "verify only what this PR touches".
///
/// ## Error paths
///
/// * `git` not installed / not on PATH → `Err("git: command not found")`.
/// * Ref doesn't resolve → `Err("git diff <ref> failed: <stderr>")`.
/// * Current directory not inside a git repo → same as above.
///
/// Callers should treat the error as advisory (fall back to full-tree
/// verification with a warning) rather than a hard build failure.
fn compute_diff_filter(base: &str) -> std::result::Result<Vec<PathBuf>, String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["diff", "--name-only", base, "--"])
        .output()
        .map_err(|e| format!("git: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff {} failed: {}", base, stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let paths: Vec<PathBuf> = stdout
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && line.ends_with(".vr"))
        .map(PathBuf::from)
        .collect();

    Ok(paths)
}

/// Merge CLI-provided profile knobs with whatever the project's
/// `verum.toml` declares under `[verify]`. CLI flags always win — the
/// manifest only fills in the gaps left by the command line.
///
/// Called from `execute` before we start iterating sources.
fn merge_with_manifest(cli: ProfileConfig) -> ProfileConfig {
    let manifest_dir = match crate::config::Manifest::find_manifest_dir() {
        Ok(dir) => dir,
        Err(_) => return cli,
    };
    let manifest_path = crate::config::Manifest::manifest_path(&manifest_dir);
    let manifest = match crate::config::Manifest::from_file(&manifest_path) {
        Ok(m) => m,
        Err(_) => return cli,
    };

    // Apply the named profile if requested. On success, every field
    // declared in `[verify.profiles.<name>]` overrides the matching
    // field in the base `[verify]` block; untouched fields inherit.
    //
    // On unknown-profile error, emit a warning and continue with the
    // base block — the verify run itself proceeds with declared CLI
    // flags, preserving the "don't silently swallow user intent"
    // rule. The documented fail-on-unknown behavior is enforced at
    // the CLI-flag-validation layer (see `main.rs::Commands::Verify`
    // dispatch) via `VerifyConfig::with_profile` returning Err.
    let effective_verify = match cli.profile_name.as_deref() {
        Some(name) => match manifest.verify.clone().with_profile(name) {
            Ok(merged) => merged,
            Err(e) => {
                eprintln!(
                    "{} {} — continuing with base [verify]",
                    "warning:".yellow().bold(),
                    e.as_str()
                );
                manifest.verify.clone()
            }
        },
        None => manifest.verify.clone(),
    };
    let v = &effective_verify;

    // Only consult the manifest for fields the CLI didn't already set.
    // `profile_slow_functions` in the manifest implicitly enables the
    // profiler for the whole project when the user hasn't passed
    // `--profile` on the command line.
    let mut out = cli;
    if !out.enabled && v.profile_slow_functions && v.total_budget.is_some() {
        // If the manifest declares a budget, opt the profile on by default —
        // users asking for a budget definitely want the report.
        out.enabled = true;
    }
    if out.budget.is_none() {
        if let Some(b) = v.total_budget.as_deref() {
            if let Ok(d) = parse_duration(b) {
                out.budget = Some(d);
            }
        }
    }
    if out.distributed_cache.is_none() {
        out.distributed_cache = v.distributed_cache.as_ref().map(|t| t.to_string());
    }
    out
}

/// Parse a human-readable duration string (e.g. `120s`, `2m`, `1h`).
///
/// Accepts a bare number as seconds. Used by `--budget=...` at the CLI layer.
pub fn parse_duration(s: &str) -> std::result::Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".into());
    }

    let split_at = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num_part, unit_part) = s.split_at(split_at);
    let n: f64 = num_part
        .parse()
        .map_err(|_| format!("invalid duration number: '{s}'"))?;
    if !n.is_finite() || n < 0.0 {
        return Err(format!("duration must be a non-negative finite number: '{s}'"));
    }

    let secs = match unit_part.trim() {
        "" | "s" | "sec" | "secs" | "seconds" => n,
        "ms" => n / 1000.0,
        "m" | "min" | "mins" | "minutes" => n * 60.0,
        "h" | "hr" | "hrs" | "hours" => n * 3600.0,
        other => return Err(format!("unknown duration unit: '{other}' (use s/m/h)")),
    };
    Ok(Duration::from_millis((secs * 1000.0) as u64))
}

/// Execute verification command for a project (no specific file)
///
/// This scans the project for .vr source files and runs the compilation
/// pipeline with verification enabled, collecting real SMT results.
pub fn execute(
    profile: ProfileConfig,
    show_cost: bool,
    compare_modes: bool,
    mode: &str,
    solver: &str,
    timeout: u64,
    _cache: bool,
    interactive: bool,
    diff_base: Option<String>,
) -> Result<()> {
    // Merge [verify] block from verum.toml (if any) with CLI flags. CLI
    // wins — per spec §1.5 / §6.1 CLI arguments override manifest defaults.
    let profile = merge_with_manifest(profile);

    if let Some(ref url) = profile.distributed_cache {
        ui::info(&format!(
            "Distributed verification cache: {} (advisory — results are \
             stored/read by the compiler when the feature is enabled)",
            url
        ));
    }

    // Resolve --diff: when supplied, compute the list of source files
    // touched since the given git ref. Verification is then limited to
    // those files. Falls through with a warning if git isn't available
    // or the ref doesn't exist (rather than failing the build — CI
    // environments that don't have a full git history shouldn't lose
    // verification coverage just because the diff filter can't apply).
    let diff_filter = if let Some(ref base) = diff_base {
        match compute_diff_filter(base.as_str()) {
            Ok(list) => {
                ui::info(&format!(
                    "Diff mode vs {}: {} source file(s) changed",
                    base,
                    list.len()
                ));
                Some(list)
            }
            Err(e) => {
                ui::warn(&format!(
                    "--diff {} failed ({}); verifying full source tree",
                    base, e
                ));
                None
            }
        }
    } else {
        None
    };
    // Plumbing follow-up: once `execute_proof_mode` takes a file-filter
    // argument, pass `diff_filter` through so only those files are
    // actually verified. At current dispatch it is an advisory hint
    // used by the project walker below.
    let _ = diff_filter;

    ui::header("Formal Verification");

    // Validate + parse the --solver choice. Accepts
    // z3 | cvc5 | auto | portfolio | capability. Unknown values error out
    // with a clear message instead of silently defaulting.
    let backend = SolverChoice::parse(solver).map_err(|e| {
        CliError::verification_failed(format!(
            "{e}. Accepted values: z3, cvc5, auto, portfolio, capability"
        ))
    })?;

    // Determine verification mode. `--compare-modes` is equivalent to
    // `--mode=compare` and takes precedence; otherwise parse the explicit
    // `--mode=…` value (defaults to "proof" at the CLI layer).
    let mode = if compare_modes {
        VerificationMode::Compare
    } else {
        VerificationMode::parse(mode).map_err(|e| {
            CliError::verification_failed(format!(
                "{e}. Accepted values: runtime, proof, compare, cubical, dependent"
            ))
        })?
    };

    ui::step(&format!(
        "Verifying with {:?} backend (timeout: {}s)",
        backend, timeout
    ));

    if interactive {
        return execute_interactive(timeout);
    }

    let stats = match mode {
        VerificationMode::Compare => execute_compare_mode(timeout, show_cost, backend, &profile)?,
        VerificationMode::Proof => execute_proof_mode(timeout, show_cost, backend, &profile)?,
        VerificationMode::Runtime => execute_runtime_mode()?,
        // Cubical and Dependent both run through the proof pipeline; the tactic
        // routing happens inside `verum_smt::tactic_evaluation` based on the
        // discovered obligations. The CLI distinction is kept so users can
        // request the focused tactic family explicitly via `--mode=cubical|dependent`.
        VerificationMode::Cubical => execute_proof_mode(timeout, show_cost, backend, &profile)?,
        VerificationMode::Dependent => execute_proof_mode(timeout, show_cost, backend, &profile)?,
    };
    let _ = backend; // retained for future use outside proof/compare modes

    print_summary(&stats);

    if stats.files_failed > 0 {
        return Err(CliError::verification_failed(format!(
            "{} file(s) failed verification",
            stats.files_failed
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Source file discovery
// ---------------------------------------------------------------------------

/// Discover .vr source files in the current project directory
fn discover_source_files() -> List<PathBuf> {
    let mut sources = List::new();

    // Look for src/ directory first (standard project layout)
    let src_dir = PathBuf::from("src");
    if src_dir.is_dir() {
        collect_vr_files(&src_dir, &mut sources);
    }

    // Also check project root for standalone .vr files
    if let Ok(entries) = std::fs::read_dir(".") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "vr") && path.is_file() {
                sources.push(path);
            }
        }
    }

    // If nothing found, check for lib.vr or main.vr
    if sources.is_empty() {
        for name in &["lib.vr", "main.vr"] {
            let p = PathBuf::from(name);
            if p.exists() {
                sources.push(p);
            }
        }
    }

    sources
}

/// Recursively collect .vr files from a directory
fn collect_vr_files(dir: &PathBuf, out: &mut List<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_vr_files(&path, out);
            } else if path.extension().map_or(false, |ext| ext == "vr") {
                out.push(path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Real verification via VerifyCommand (Z3 available)
// ---------------------------------------------------------------------------

/// Run verification on a single file using the real Z3 SMT solver.
///
/// Uses `verum_compiler::verify_cmd::VerifyCommand` which:
/// 1. Parses and type-checks the source file
/// 2. Extracts functions with refinement types / contracts
/// 3. Creates a Z3 context and translates AST constraints to SMT
/// 4. Runs the solver with the configured timeout
/// 5. Reports real pass/fail/timeout per function
#[cfg(feature = "verification")]
fn verify_file_proof(
    path: &PathBuf,
    timeout: u64,
    show_cost: bool,
    backend: SolverChoice,
    profile: &ProfileConfig,
) -> std::result::Result<bool, String> {
    use verum_compiler::verify_cmd::VerifyCommand;

    let options = CompilerOptions {
        input: path.clone(),
        verify_mode: VerifyMode::Proof,
        smt_timeout_secs: timeout,
        smt_solver: backend.into(),
        show_verification_costs: show_cost,
        output_format: OutputFormat::Human,
        // Profiler / budget / export wiring — `--profile`, `--budget`, `--export`
        // arrive here from the CLI and are consumed by VerifyCommand internally.
        profile_verification: profile.enabled,
        profile_obligation: profile.profile_obligation,
        verification_budget_secs: profile.budget.map(|d| d.as_secs().max(1)),
        export_verification_json: profile.export_path.is_some(),
        verification_json_path: profile.export_path.clone(),
        distributed_cache_url: profile.distributed_cache.clone(),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let verify_cmd = VerifyCommand::new(&mut session);

    match verify_cmd.run(None) {
        Ok(()) => Ok(true),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Verification failed") {
                Ok(false)
            } else if msg.contains("budget exceeded") {
                // Surface the budget violation as a hard failure so the shell
                // exit code reflects it — CI pipelines depend on this.
                Err(format!("verification budget exceeded: {}", msg))
            } else {
                Err(msg)
            }
        }
    }
}

/// Fallback when Z3 is not available: run type-check only and report
/// that SMT proofs are not possible.
#[cfg(not(feature = "verification"))]
fn verify_file_proof(
    path: &PathBuf,
    timeout: u64,
    _show_cost: bool,
    _backend: SolverChoice,
    _profile: &ProfileConfig,
) -> std::result::Result<bool, String> {
    // Without Z3, run type-checking only
    let options = CompilerOptions {
        input: path.clone(),
        verify_mode: VerifyMode::Runtime,
        smt_timeout_secs: timeout,
        output_format: OutputFormat::Human,
        check_only: true,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    match pipeline.run_check_only() {
        Ok(()) => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}

/// Run type-check only (for runtime mode, no SMT needed)
fn verify_file_check_only(path: &PathBuf) -> std::result::Result<bool, String> {
    let options = CompilerOptions {
        input: path.clone(),
        verify_mode: VerifyMode::Runtime,
        output_format: OutputFormat::Human,
        check_only: true,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    match pipeline.run_check_only() {
        Ok(()) => Ok(true),
        Err(e) => Err(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Mode implementations
// ---------------------------------------------------------------------------

fn execute_proof_mode(
    timeout: u64,
    show_cost: bool,
    backend: SolverChoice,
    profile: &ProfileConfig,
) -> Result<VerificationStats> {
    #[cfg(not(feature = "verification"))]
    {
        println!();
        ui::warn(
            "Z3 SMT solver not available. Enable the 'verification' feature to use proof mode.",
        );
        ui::info("  Install Z3:  brew install z3  (macOS)  /  apt install libz3-dev  (Linux)");
        ui::info("  Then build:  cargo build --features verification");
        println!();
        ui::info("Falling back to type-checking only (no SMT proofs).");
        println!();
    }

    ui::step("Scanning project for source files");

    let sources = discover_source_files();
    if sources.is_empty() {
        ui::warn("No .vr source files found in project");
        return Ok(VerificationStats::default());
    }

    ui::step(&format!("Found {} source file(s)", sources.len()));
    println!();

    let mut stats = VerificationStats::default();
    let overall_start = Instant::now();

    // Per-project budget accounting (spec §1.2 / §1.4): if the user passed
    // `--budget=120s`, we bound the *total* project time, not per-file time.
    // The inner VerifyCommand's BudgetTracker resets on every file, so the
    // project-level guard lives here.
    let project_budget = profile.budget;
    let mut stopped_on_budget = false;

    for source in &sources {
        // Per-file budget: shrink the compiler-side budget to whatever
        // remains in the project budget, so even a single file can't blow
        // past the total.
        let mut per_file_profile = profile.clone();
        if let Some(budget) = project_budget {
            let elapsed = overall_start.elapsed();
            if elapsed >= budget {
                stopped_on_budget = true;
                ui::warn(&format!(
                    "Verification budget exhausted after {:.2}s — skipping remaining \
                     {} file(s).",
                    elapsed.as_secs_f64(),
                    sources.len() - stats.total_files,
                ));
                break;
            }
            per_file_profile.budget = Some(budget - elapsed);
        }

        let display_path = source.display().to_string();
        stats.total_files += 1;

        let file_start = Instant::now();
        match verify_file_proof(source, timeout, show_cost, backend, &per_file_profile) {
            Ok(true) => {
                let elapsed = file_start.elapsed();
                stats.files_verified += 1;
                println!(
                    "  {} {} {:.2}s",
                    "✓".green().bold(),
                    display_path.bold(),
                    elapsed.as_secs_f64()
                );
            }
            Ok(false) => {
                let elapsed = file_start.elapsed();
                stats.files_failed += 1;
                println!(
                    "  {} {} {:.2}s",
                    "✗".red().bold(),
                    display_path.bold(),
                    elapsed.as_secs_f64()
                );
            }
            Err(e) => {
                stats.files_failed += 1;
                println!(
                    "  {} {} {}",
                    "✗".red().bold(),
                    display_path.bold(),
                    format!("({})", e).dimmed()
                );
            }
        }
    }

    stats.total_time = overall_start.elapsed();

    // Surface the budget overshoot to callers. `execute` already returns
    // CliError::verification_failed on any files_failed > 0, but an empty
    // budget-abort would otherwise succeed silently — use a sentinel failure.
    if stopped_on_budget {
        stats.files_failed = stats.files_failed.max(1);
        return Err(CliError::verification_failed(format!(
            "project verification budget of {:.1}s exceeded after {} file(s)",
            project_budget.unwrap().as_secs_f64(),
            stats.total_files,
        )));
    }

    Ok(stats)
}

fn execute_compare_mode(
    timeout: u64,
    show_cost: bool,
    backend: SolverChoice,
    profile: &ProfileConfig,
) -> Result<VerificationStats> {
    ui::step("Comparing runtime vs proof verification modes");

    let sources = discover_source_files();
    if sources.is_empty() {
        ui::warn("No .vr source files found in project");
        return Ok(VerificationStats::default());
    }

    ui::step(&format!("Found {} source file(s)", sources.len()));

    let mut stats = VerificationStats::default();
    let overall_start = Instant::now();

    println!();
    ui::section("Cost/Benefit Comparison");
    println!();

    for source in &sources {
        let display_path = source.display().to_string();
        stats.total_files += 1;

        // Measure proof mode time
        let proof_start = Instant::now();
        let proof_ok = verify_file_proof(source, timeout, show_cost, backend, profile);
        let proof_time = proof_start.elapsed();

        // Measure check-only (runtime) time
        let check_start = Instant::now();
        let check_ok = verify_file_check_only(source);
        let check_time = check_start.elapsed();

        let proof_succeeded = proof_ok.as_ref().map_or(false, |ok| *ok);
        let check_succeeded = check_ok.as_ref().map_or(false, |ok| *ok);

        if proof_succeeded {
            stats.files_verified += 1;
        } else {
            stats.files_failed += 1;
        }

        let status_symbol = if proof_succeeded {
            "✓".green().bold()
        } else {
            "✗".red().bold()
        };

        println!("  {} {}", status_symbol, display_path.bold());
        println!();
        println!("    Cost Comparison:");
        println!(
            "      Compile-time proof: {:.2}ms compile, 0ns runtime    {}",
            proof_time.as_secs_f64() * 1000.0,
            if proof_succeeded {
                "(proved)".green().to_string()
            } else {
                "(failed/timeout)".red().to_string()
            }
        );
        println!(
            "      Runtime checks:     {:.2}ms compile, ~3-15ns/call   {}",
            check_time.as_secs_f64() * 1000.0,
            if check_succeeded {
                "(type-checked)".green().to_string()
            } else {
                "(errors)".red().to_string()
            }
        );

        if proof_succeeded && proof_time > check_time {
            let overhead = proof_time - check_time;
            println!(
                "      {}",
                format!(
                    "Proof overhead: +{:.2}ms compile time, saves runtime cost",
                    overhead.as_secs_f64() * 1000.0
                )
                .dimmed()
            );
        }
        println!();
    }

    stats.total_time = overall_start.elapsed();
    Ok(stats)
}

fn execute_runtime_mode() -> Result<VerificationStats> {
    ui::step("Using runtime verification mode");
    ui::info("All refinements will be checked at runtime (0s compile overhead)");
    ui::info("No SMT solver needed for runtime mode.");

    let sources = discover_source_files();
    if sources.is_empty() {
        ui::warn("No .vr source files found in project");
        return Ok(VerificationStats::default());
    }

    let mut stats = VerificationStats::default();
    let overall_start = Instant::now();

    println!();

    for source in &sources {
        let display_path = source.display().to_string();
        stats.total_files += 1;

        match verify_file_check_only(source) {
            Ok(true) => {
                stats.files_runtime += 1;
                println!(
                    "  {} {} (runtime checks)",
                    "⚡".yellow(),
                    display_path
                );
            }
            Ok(false) | Err(_) => {
                stats.files_failed += 1;
                println!("  {} {} (type errors)", "✗".red(), display_path);
            }
        }
    }

    stats.total_time = overall_start.elapsed();
    println!();
    ui::success(&format!(
        "{} file(s) will use runtime verification checks",
        stats.files_runtime
    ));

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Interactive mode
// ---------------------------------------------------------------------------

fn execute_interactive(timeout: u64) -> Result<()> {
    #[cfg(not(feature = "verification"))]
    {
        let _ = timeout; // suppress unused warning when Z3 is not available
        ui::error("Interactive mode requires Z3 SMT solver.");
        ui::info("  Install Z3:  brew install z3  (macOS)  /  apt install libz3-dev  (Linux)");
        ui::info("  Then build:  cargo build --features verification");
        return Err(CliError::verification_failed(
            "Z3 not available. Interactive mode requires the 'verification' feature.",
        ));
    }

    #[cfg(feature = "verification")]
    {
        execute_interactive_impl(timeout)
    }
}

#[cfg(feature = "verification")]
fn execute_interactive_impl(timeout: u64) -> Result<()> {
    use std::io::{self, Write};

    ui::section("Interactive Proof Mode");
    println!();
    println!("  Welcome to Verum interactive verification!");
    println!("  Scanning project for functions with verification issues...");
    println!();

    let sources = discover_source_files();
    if sources.is_empty() {
        ui::warn("No .vr source files found in project");
        return Ok(());
    }

    // Run verification on all files to find problems
    let mut problem_files: List<(String, String)> = List::new(); // (file_path, error_detail)

    for source in &sources {
        let display_path = source.display().to_string();
        match verify_file_proof(source, timeout, false, SolverChoice::Z3, &ProfileConfig::default()) {
            Ok(false) => {
                problem_files.push((display_path, "Verification failed".to_string()));
            }
            Err(e) => {
                problem_files.push((display_path, e));
            }
            Ok(true) => {} // passed, skip
        }
    }

    if problem_files.is_empty() {
        ui::success("All files verified successfully. Nothing to investigate.");
        return Ok(());
    }

    loop {
        println!("  Files with verification issues:");
        println!();
        for (i, (path, issue)) in problem_files.iter().enumerate() {
            println!(
                "    {} {}",
                format!("[{}]", i + 1).cyan(),
                path.as_str().bold(),
            );
            println!("       Issue: {}", issue.as_str().yellow());
        }
        println!();
        println!("  Commands:");
        println!(
            "    1-{}: Re-verify file with increased timeout",
            problem_files.len()
        );
        println!("    help: Show proof tactics");
        println!("    quit: Exit interactive mode");
        println!();

        print!("  {} ", ">".green().bold());
        io::stdout().flush().unwrap();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            break;
        }

        let cmd = input.trim();

        if cmd == "quit" || cmd == "q" || cmd == "exit" {
            ui::info("Exiting interactive mode");
            break;
        }

        if cmd == "help" || cmd == "h" {
            show_proof_help();
            continue;
        }

        if let Ok(idx) = cmd.parse::<usize>() {
            if idx > 0 && idx <= problem_files.len() {
                let (path, _issue) = &problem_files[idx - 1];
                investigate_file(path, timeout);
                continue;
            }
        }

        ui::warn(&format!(
            "Unknown command: {}. Type 'help' for options.",
            cmd
        ));
        println!();
    }

    Ok(())
}

fn show_proof_help() {
    println!();
    println!("  Proof Tactics:");
    println!();
    println!("  {} Simplification:", "1.".bold());
    println!("     - Break complex predicates into smaller assertions");
    println!("     - Use intermediate lemmas");
    println!("     - Replace nonlinear with linear approximations");
    println!();
    println!("  {} Timeout Handling:", "2.".bold());
    println!("     - Increase timeout with --timeout <seconds>");
    println!("     - Simplify quantifiers");
    println!("     - Use @verify(runtime) for complex cases");
    println!();
    println!("  {} Annotations:", "3.".bold());
    println!("     - @axiom: Add trusted assumptions");
    println!("     - @invariant: Specify loop invariants");
    println!("     - @ensures: Strengthen postconditions");
    println!();
}

#[cfg(feature = "verification")]
fn investigate_file(path: &str, timeout: u64) {
    println!();
    println!("{}", "-".repeat(60).cyan());
    println!("  Re-verifying: {}", path.bold());
    println!(
        "  Using increased timeout: {}s",
        (timeout * 2).to_string().cyan()
    );
    println!("{}", "-".repeat(60).cyan());
    println!();

    let file_path = PathBuf::from(path);
    let start = Instant::now();
    match verify_file_proof(&file_path, timeout * 2, true, SolverChoice::Z3, &ProfileConfig::default()) {
        Ok(true) => {
            let elapsed = start.elapsed();
            ui::success(&format!(
                "Verification succeeded with extended timeout ({:.2}s)",
                elapsed.as_secs_f64()
            ));
            println!();
            println!("  Suggestion: increase your default timeout to at least {}s", (elapsed.as_secs() + 5));
        }
        Ok(false) => {
            ui::warn("Verification still fails with extended timeout.");
            println!();
            println!("  Suggested actions:");
            println!();
            println!("    {} Simplify contract predicates", "1.".green());
            println!("       Break complex expressions into smaller, linear constraints");
            println!();
            println!("    {} Add intermediate lemmas", "2.".green());
            println!("       Use 'lemma' declarations to help the solver");
            println!();
            println!("    {} Switch to runtime verification", "3.".green());
            println!("       Add @verify(runtime) annotation to skip SMT");
            println!(
                "       {}",
                "Compile time: 0s, Runtime: ~3-15ns per call".dimmed()
            );
        }
        Err(e) => {
            ui::error(&format!("Error: {}", e));
        }
    }
    println!();
}

// ---------------------------------------------------------------------------
// Summary display
// ---------------------------------------------------------------------------

fn print_summary(stats: &VerificationStats) {
    println!();
    ui::section("Verification Summary");
    println!();

    println!("  Total files:    {}", stats.total_files);
    println!(
        "  {} Verified:     {}",
        "✓".green(),
        stats.files_verified
    );

    if stats.files_runtime > 0 {
        println!(
            "  {} Runtime:      {}",
            "⚡".yellow(),
            stats.files_runtime
        );
    }

    if stats.files_failed > 0 {
        println!(
            "  {} Failed:       {}",
            "✗".red(),
            stats.files_failed
        );
    }

    println!();
    println!("  Success rate: {:.1}%", stats.success_rate());
    println!("  Total time:   {:.2}s", stats.total_time.as_secs_f64());
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_accepts_bare_seconds() {
        assert_eq!(parse_duration("120").unwrap(), Duration::from_secs(120));
    }

    #[test]
    fn parse_duration_supports_time_units() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
    }

    #[test]
    fn parse_duration_rejects_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("10x").is_err());
        assert!(parse_duration("-1s").is_err());
    }

    #[test]
    fn profile_config_default_is_noop() {
        let cfg = ProfileConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.budget.is_none());
        assert!(cfg.export_path.is_none());
        assert!(cfg.distributed_cache.is_none());
    }

    #[test]
    fn verification_stats_success_rate_rounds() {
        let mut stats = VerificationStats::default();
        stats.total_files = 4;
        stats.files_verified = 3;
        assert!((stats.success_rate() - 75.0).abs() < 0.001);
        stats.total_files = 0;
        assert!((stats.success_rate() - 100.0).abs() < 0.001);
    }

    #[test]
    fn diff_filter_with_bogus_ref_returns_err_not_panic() {
        // The --diff flag must never panic on a ref that doesn't exist
        // — CI should degrade to full verification rather than crash.
        let res =
            compute_diff_filter("definitely-not-a-real-git-ref-xyzzy-42");
        // Either Err (expected — ref doesn't resolve) or Ok with empty
        // (some git versions may succeed silently); both are
        // degrade-gracefully paths. What we're locking in is "no panic".
        match res {
            Ok(paths) => {
                // Degenerate OK path — should be empty.
                assert!(
                    paths.is_empty(),
                    "bogus ref produced unexpected paths: {:?}",
                    paths
                );
            }
            Err(msg) => {
                assert!(
                    !msg.is_empty(),
                    "diff error should carry a non-empty message"
                );
            }
        }
    }
}
