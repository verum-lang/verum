//! File-based compilation commands
//!
//! This module provides single-file operations that work independently
//! of Verum projects. These commands are useful for quick scripts,
//! testing, and REPL-style development.
//!
//! Single-file compilation commands integrated into the main verum CLI.

use anyhow::Result;
use colored::Colorize;
use std::path::PathBuf;
use verum_common::{List, Text};

use crate::error::CliError;
use crate::ui;

use verum_compiler::{
    options::{CompilerOptions, OutputFormat, VerifyMode},
    pipeline::CompilationPipeline,
    profile_cmd::ProfileCommand,
    repl::Repl,
    session::Session,
    verify_cmd::VerifyCommand,
};

/// Parse verify mode from string.
///
/// Accepts the three core verify modes (`auto`, `runtime`, `proof`) plus
/// the focused tactic-family aliases `cubical` and `dependent`. The
/// tactic-family aliases route through the proof pipeline at the
/// `VerifyMode` layer (the underlying tactic dispatch happens inside
/// `verum_smt::tactic_evaluation` based on the obligation shape, not the
/// CLI mode); the CLI just acknowledges the user's intent so the
/// invocation doesn't error out.
fn parse_verify_mode(mode: &str) -> Result<VerifyMode, CliError> {
    match mode.to_lowercase().as_str() {
        "auto" => Ok(VerifyMode::Auto),
        "runtime" => Ok(VerifyMode::Runtime),
        "proof" | "cubical" | "dependent" | "compare" => Ok(VerifyMode::Proof),
        _ => Err(CliError::InvalidArgument(format!(
            "Invalid verify mode: {}. Must be one of: auto, runtime, proof, cubical, dependent, compare",
            mode
        ))),
    }
}

/// Build single file to executable
pub fn build(
    file: &str,
    output: Option<&str>,
    opt_level: u8,
    verify_mode: &str,
    timeout: u64,
    show_costs: bool,
    emit_vbc: bool,
) -> Result<(), CliError> {
    let start = std::time::Instant::now();

    let input = PathBuf::from(file);
    if !input.exists() {
        return Err(CliError::FileNotFound(file.to_string()));
    }

    ui::status("Compiling", &format!("{} (AOT)", file));

    let verify_mode = parse_verify_mode(verify_mode)?;

    // If no output specified, the pipeline will use target/<profile>/<name>
    // If output is specified, use it as-is
    let output_path = output.map(PathBuf::from).unwrap_or_default();

    // Inherit CLI feature overrides so single-file AOT build fires
    // the same gates as `verum build` / `verum run`.
    let language_features = crate::feature_overrides::scratch_features()?;
    let options = CompilerOptions {
        input: input.clone(),
        output: output_path.clone(),
        verify_mode,
        smt_timeout_secs: timeout,
        show_verification_costs: show_costs,
        optimization_level: opt_level,
        output_format: OutputFormat::Human,
        emit_vbc,
        language_features,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    // Build native executable instead of interpreting
    let executable_path = pipeline
        .run_native_compilation()
        .map_err(|e| CliError::CompilationFailed(e.to_string()))?;

    let opt_tag = if opt_level >= 2 { "optimized" } else { "unoptimized + debuginfo" };
    ui::success(&format!(
        "[{}] target(s) in {}",
        opt_tag,
        ui::format_duration(start.elapsed())
    ));

    if executable_path.exists() {
        let binary_size = std::fs::metadata(&executable_path)
            .map(|m| ui::format_size(m.len()))
            .unwrap_or_else(|_| "unknown".to_string());
        ui::detail("Binary", &format!(
            "{} ({})",
            executable_path.display(),
            binary_size
        ));
    }

    Ok(())
}

/// Check single file without compilation
pub fn check(file: &str, continue_on_error: bool, parse_only: bool) -> Result<(), CliError> {
    let start = std::time::Instant::now();

    let input = PathBuf::from(file);
    if !input.exists() {
        return Err(CliError::FileNotFound(file.to_string()));
    }

    // Auto-detect test type annotations for parse-only mode, expected errors, and skip
    let (parse_only, expect_errors, skip_reason) = {
        if let Ok(content) = std::fs::read_to_string(&input) {
            let mut is_parse_only = parse_only;
            let mut expects_errors = false;
            let mut skip: Option<String> = None;
            for line in content.lines().take(15) {
                let trimmed = line.trim();
                if trimmed.starts_with("// @test:") {
                    let test_type = trimmed.trim_start_matches("// @test:").trim();
                    if matches!(test_type, "parse-pass" | "parser" | "parse-recover" | "parse-fail") {
                        is_parse_only = true;
                    }
                    // typecheck-fail, meta-fail, verify-fail tests expect errors
                    if matches!(test_type, "typecheck-fail" | "parse-fail" | "parse-recover" | "meta-fail" | "verify-fail") {
                        expects_errors = true;
                    }
                }
                if trimmed.starts_with("// @expect:") {
                    let expect = trimmed.trim_start_matches("// @expect:").trim();
                    if matches!(expect, "errors" | "fail" | "error") {
                        expects_errors = true;
                    }
                }
                if trimmed.starts_with("// @skip:") {
                    let reason = trimmed.trim_start_matches("// @skip:").trim();
                    skip = Some(reason.to_string());
                }
            }
            (is_parse_only, expects_errors, skip)
        } else {
            (parse_only, false, None)
        }
    };

    // Handle @skip annotation
    if let Some(reason) = skip_reason {
        ui::status("Skipping", &format!("{} ({})", file, reason));
        return Ok(());
    }

    if parse_only {
        ui::status("Parsing", file);
    } else {
        ui::status("Checking", file);
    }

    // Build LanguageFeatures from any installed CLI overrides so
    // `verum check file.vr -Z safety.unsafe_allowed=false` fires the
    // same gates as `verum run` / `verum build`.
    let language_features = crate::feature_overrides::scratch_features()?;
    let options = CompilerOptions {
        input,
        output_format: OutputFormat::Human,
        continue_on_error,
        language_features,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    if parse_only {
        let result = pipeline.run_parse_only();
        if expect_errors {
            // For parse-recover/parse-fail tests with @expect: errors,
            // parse errors are expected — success means errors were found
            if result.is_err() {
                ui::success(&format!("parsing {} (errors expected) in {}", file, ui::format_duration(start.elapsed())));
            } else {
                ui::success(&format!("parsing {} in {}", file, ui::format_duration(start.elapsed())));
            }
        } else {
            result.map_err(|e| CliError::CompilationFailed(e.to_string()))?;
            ui::success(&format!("parsing {} in {}", file, ui::format_duration(start.elapsed())));
        }
    } else if expect_errors {
        // For typecheck-fail tests, errors are expected
        let result = pipeline.run_check_only();
        if result.is_err() {
            ui::success(&format!("checking {} (errors expected) in {}", file, ui::format_duration(start.elapsed())));
        } else {
            ui::success(&format!("checking {} in {}", file, ui::format_duration(start.elapsed())));
        }
    } else {
        pipeline
            .run_check_only()
            .map_err(|e| CliError::CompilationFailed(e.to_string()))?;
        ui::success(&format!("checking {} in {}", file, ui::format_duration(start.elapsed())));
    }
    Ok(())
}

/// Run single file (interpret or compile and execute)
pub fn run(file: &str, args: List<Text>, skip_verify: bool) -> Result<(), CliError> {
    run_with_tier(file, args, skip_verify, None, false)
}

/// Run single file with tier selection
///
/// Tier selection:
/// - Tier 0 (interpreter): Direct interpretation, instant start
/// - Tier 1 (aot): AOT compilation via LLVM, production quality
pub fn run_with_tier(
    file: &str,
    args: List<Text>,
    skip_verify: bool,
    tier: Option<u8>,
    timings: bool,
) -> Result<(), CliError> {
    let tier_num = match tier {
        Some(0) | None => 0,
        Some(1) => 1,
        Some(t) => {
            return Err(CliError::InvalidArgument(format!(
                "Invalid tier '{}'. Valid tiers: 0 (interpreter), 1 (aot)",
                t
            )));
        }
    };

    let input = PathBuf::from(file);
    if !input.exists() {
        return Err(CliError::FileNotFound(file.to_string()));
    }

    // Resolve effective language features from CLI overrides (if any).
    // Even in single-file mode (no verum.toml), the user can supply
    // `-Z safety.unsafe_allowed=false` etc. on the command line and
    // the installed global override set applies. This ensures feature
    // gates fire identically in Tier 0 (interpreter) AND Tier 1 (AOT).
    let language_features = crate::feature_overrides::scratch_features()?;

    match tier_num {
        0 => {
            // Tier 0: Direct interpretation via pipeline.
            //
            // Mode policy (Verum execution-mode contract):
            //   1. Interpreter — `.vr` file declares `fn main()`; run via VBC.
            //   2. AOT (Tier 1) — same but compiled to native via LLVM.
            //   3. Script — `.vr` file MUST start with a `#!` shebang line;
            //      top-level statements are then folded into a synthesised
            //      `__verum_script_main` wrapper. Files without shebang AND
            //      without `fn main()` are rejected at entry-detection time
            //      with a help message pointing at both options.
            //
            // The single-file CLI entry intentionally does NOT force
            // `script_mode = true`; the pipeline's `should_parse_as_script`
            // helper drives mode selection from the file's content (shebang
            // byte-prefix). Stdin and inline-`-e` sources without a path
            // opt the flag on explicitly when those entry points land.
            let options = CompilerOptions {
                input: input.clone(),
                verify_mode: if skip_verify {
                    VerifyMode::Runtime
                } else {
                    VerifyMode::Auto
                },
                output_format: OutputFormat::Human,
                language_features: language_features.clone(),
                ..Default::default()
            };
            let mut session = Session::new(options);
            let mut pipeline = CompilationPipeline::new(&mut session);
            pipeline
                .run_interpreter(args)
                .map_err(|e| CliError::RuntimeError(e.to_string()))?;

            if timings {
                print_phase_timings(&session);
            }
        }
        1 => {
            // Tier 1: AOT compilation to native binary then execute.
            // Mode is content-driven (shebang autodetect, no flag) — see
            // Tier-0 comment.
            let verify_mode = if skip_verify {
                VerifyMode::Runtime
            } else {
                VerifyMode::Auto
            };
            let options = CompilerOptions {
                input: input.clone(),
                verify_mode,
                output_format: OutputFormat::Human,
                language_features: language_features.clone(),
                ..Default::default()
            };
            let mut session = Session::new(options);
            let mut pipeline = CompilationPipeline::new(&mut session);

            match pipeline.run_native_compilation() {
                Ok(executable) => {
                    if timings {
                        print_phase_timings(&session);
                    }

                    ui::status("Running", &format!("`{}`", executable.display()));

                    // Execute the native binary
                    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    let status = std::process::Command::new(&executable)
                        .args(&args_str)
                        .status()
                        .map_err(|e| {
                            CliError::RuntimeError(format!("Failed to run executable: {}", e))
                        })?;

                    if !status.success() {
                        // Propagate the child program's exit code so this
                        // wrapper is transparent to callers (test runners,
                        // shells using $?). Treating any non-zero as a
                        // wrapper error masked the user's exit value with
                        // a constant 1, which broke vtest's @expected-exit
                        // contract.
                        let exit_code = status.code().unwrap_or(-1);
                        std::process::exit(exit_code);
                    }
                }
                Err(aot_err) => {
                    // If the error came from a feature gate (safety,
                    // unsafe, FFI, etc.) — do NOT fall back. A gate
                    // rejection is a user-intent check, not a build
                    // system hiccup, and silently falling back would
                    // defeat the gate.
                    let err_str = aot_err.to_string();
                    if err_str.contains("safety gate")
                        || err_str.contains("[safety]")
                        || err_str.contains("[meta]")
                        || err_str.contains("[context]")
                    {
                        return Err(CliError::CompilationFailed(err_str));
                    }

                    // Graceful fallback: AOT failed for an unrelated
                    // reason (LLVM glitch, toolchain issue) — retry
                    // with the interpreter. Preserve language_features
                    // so the interpreter applies the same gates.
                    ui::warn(&format!(
                        "AOT compilation failed: {}. Falling back to interpreter.",
                        aot_err
                    ));
                    let fallback_options = CompilerOptions {
                        input: input.clone(),
                        verify_mode,
                        output_format: OutputFormat::Human,
                        language_features: language_features.clone(),
                        ..Default::default()
                    };
                    let mut fallback_session = Session::new(fallback_options);
                    let mut fallback_pipeline =
                        CompilationPipeline::new(&mut fallback_session);
                    fallback_pipeline
                        .run_interpreter(args)
                        .map_err(|e| CliError::RuntimeError(e.to_string()))?;
                }
            }
        }
        _ => unreachable!(),
    }

    Ok(())
}

/// Print compilation phase timings from session metrics
fn print_phase_timings(session: &Session) {
    let phases = session.get_phase_timings();
    if phases.is_empty() {
        return;
    }

    eprintln!("\n  Compilation Timings:");
    eprintln!("  ────────────────────────────────────");

    let mut total = std::time::Duration::ZERO;
    for (name, duration) in &phases {
        total += *duration;
        eprintln!("  {:<19}{:>8.1}ms", format!("{}:", name), duration.as_secs_f64() * 1000.0);
    }

    eprintln!("  ────────────────────────────────────");
    eprintln!("  {:<19}{:>8.1}ms", "Total:", total.as_secs_f64() * 1000.0);
    eprintln!();
}

/// Verify refinement types in single file
pub fn verify(
    file: &str,
    mode: &str,
    show_costs: bool,
    timeout: u64,
    solver: &str,
    function: Option<&str>,
) -> Result<(), CliError> {
    ui::step(&format!("Verifying {}", file));

    let input = PathBuf::from(file);
    if !input.exists() {
        return Err(CliError::FileNotFound(file.to_string()));
    }

    let verify_mode = parse_verify_mode(mode)?;
    let language_features = crate::feature_overrides::scratch_features()?;
    // Always validate the --solver input so typos error out regardless of
    // the `verification` feature. The parsed choice is only forwarded to the
    // compiler when the feature is enabled; otherwise the compiler's Z3
    // default is used.
    let _backend = crate::commands::verify::SolverChoice::parse(solver).map_err(|e| {
        CliError::VerificationFailed(format!(
            "{e}. Accepted values: z3, cvc5, auto, portfolio, capability"
        ))
    })?;

    #[cfg(feature = "verification")]
    let smt_solver_choice: verum_smt::backend_switcher::BackendChoice = _backend.into();
    #[cfg(not(feature = "verification"))]
    let smt_solver_choice = Default::default();

    let options = CompilerOptions {
        input,
        verify_mode,
        smt_timeout_secs: timeout,
        smt_solver: smt_solver_choice,
        show_verification_costs: show_costs,
        output_format: OutputFormat::Human,
        language_features,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let verify_cmd = VerifyCommand::new(&mut session);

    verify_cmd
        .run(function)
        .map_err(|e| CliError::VerificationFailed(e.to_string()))?;

    ui::success("Verification complete");
    Ok(())
}

/// Profile CBGR overhead in single file
pub fn profile(
    file: &str,
    memory: bool,
    hot_threshold: f64,
    output: Option<&str>,
    suggest: bool,
) -> Result<(), CliError> {
    ui::step(&format!("Profiling {}", file));

    let input = PathBuf::from(file);
    if !input.exists() {
        return Err(CliError::FileNotFound(file.to_string()));
    }

    let language_features = crate::feature_overrides::scratch_features()?;
    let options = CompilerOptions {
        input,
        profile_memory: memory,
        hot_path_threshold: hot_threshold,
        output_format: OutputFormat::Human,
        language_features,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut profile_cmd = ProfileCommand::new(&mut session);

    let output_path = output.map(PathBuf::from);
    let output_ref = output_path.as_deref();

    profile_cmd
        .run(output_ref, suggest)
        .map_err(|e| CliError::ProfilingFailed(e.to_string()))?;

    ui::success("Profiling complete");
    Ok(())
}

/// Interactive REPL with optional file preload
pub fn repl(preload: Option<&str>, skip_verify: bool) -> Result<(), CliError> {
    ui::step("Starting REPL");

    let language_features = crate::feature_overrides::scratch_features()?;
    let options = CompilerOptions {
        verify_mode: if skip_verify {
            VerifyMode::Runtime
        } else {
            VerifyMode::Auto
        },
        output_format: OutputFormat::Human,
        language_features,
        ..Default::default()
    };

    let session = Session::new(options);
    let mut repl = Repl::new(session);

    if let Some(preload_path) = preload {
        let path = PathBuf::from(preload_path);
        if !path.exists() {
            return Err(CliError::FileNotFound(preload_path.to_string()));
        }
        repl.preload(&path)
            .map_err(|e| CliError::ReplError(e.to_string()))?;
    }

    repl.run().map_err(|e| CliError::ReplError(e.to_string()))?;

    Ok(())
}

/// Display compiler information
pub fn info(features: bool, llvm: bool, all: bool) -> Result<(), CliError> {
    println!("{}", "Verum Compiler Information".bold());
    println!("{}", "=".repeat(50));
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!("Repository: {}", env!("CARGO_PKG_REPOSITORY"));
    println!();

    if features || all {
        println!("{}", "Features:".bold());
        println!("  {} Refinement types with SMT verification", "✓".green());
        println!("  {} CBGR memory management (<15ns overhead)", "✓".green());
        println!("  {} Bidirectional type checking", "✓".green());
        println!("  {} Stream comprehensions", "✓".green());
        println!("  {} Context system (DI)", "✓".green());
        println!();
    }

    if llvm || all {
        println!("{}", "LLVM Backend:".bold());
        #[cfg(feature = "llvm")]
        println!("  Version: {}", "21.1 (via inkwell)");
        #[cfg(not(feature = "llvm"))]
        println!("  Status: {}", "Not built with LLVM support".yellow());
        println!();
    }

    println!("{}", "Components:".bold());
    println!("  Lexer: verum_lexer v{}", env!("CARGO_PKG_VERSION"));
    println!("  Parser: verum_parser v{}", env!("CARGO_PKG_VERSION"));
    println!("  Type Checker: verum_types v{}", env!("CARGO_PKG_VERSION"));
    println!("  SMT Solver: Z3 (via verum_smt)");
    println!("  CBGR Runtime: verum_cbgr v{}", env!("CARGO_PKG_VERSION"));
    println!();

    println!("{}", "Usage:".bold());
    println!("  Project commands: verum build, verum run, verum test");
    println!("  Single file commands: verum run <file.vr>, verum check <file.vr>");
    println!("  For help: verum --help");

    Ok(())
}
