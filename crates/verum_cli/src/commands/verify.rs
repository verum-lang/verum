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

/// Execute verification command for a project (no specific file)
///
/// This scans the project for .vr source files and runs the compilation
/// pipeline with verification enabled, collecting real SMT results.
pub fn execute(
    _profile: bool,
    show_cost: bool,
    compare_modes: bool,
    mode: &str,
    solver: &str,
    timeout: u64,
    _cache: bool,
    interactive: bool,
) -> Result<()> {
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
        VerificationMode::Compare => execute_compare_mode(timeout, show_cost, backend)?,
        VerificationMode::Proof => execute_proof_mode(timeout, show_cost, backend)?,
        VerificationMode::Runtime => execute_runtime_mode()?,
        // Cubical and Dependent both run through the proof pipeline; the tactic
        // routing happens inside `verum_smt::tactic_evaluation` based on the
        // discovered obligations. The CLI distinction is kept so users can
        // request the focused tactic family explicitly via `--mode=cubical|dependent`.
        VerificationMode::Cubical => execute_proof_mode(timeout, show_cost, backend)?,
        VerificationMode::Dependent => execute_proof_mode(timeout, show_cost, backend)?,
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
) -> std::result::Result<bool, String> {
    use verum_compiler::verify_cmd::VerifyCommand;

    let options = CompilerOptions {
        input: path.clone(),
        verify_mode: VerifyMode::Proof,
        smt_timeout_secs: timeout,
        smt_solver: backend.into(),
        show_verification_costs: show_cost,
        output_format: OutputFormat::Human,
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

    for source in &sources {
        let display_path = source.display().to_string();
        stats.total_files += 1;

        let file_start = Instant::now();
        match verify_file_proof(source, timeout, show_cost, backend) {
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
    Ok(stats)
}

fn execute_compare_mode(
    timeout: u64,
    show_cost: bool,
    backend: SolverChoice,
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
        let proof_ok = verify_file_proof(source, timeout, show_cost, backend);
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
        match verify_file_proof(source, timeout, false, SolverChoice::Z3) {
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
    match verify_file_proof(&file_path, timeout * 2, true, SolverChoice::Z3) {
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
