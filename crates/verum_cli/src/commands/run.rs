//! Run command - builds and executes the project
//!
//! # Two-Tier Architecture
//!
//! The execution tiers are:
//! - Tier 0: VBC Interpreter (direct VBC execution, full diagnostics)
//! - Tier 1: AOT compilation via LLVM (native executable)
//!
//! Use `verum run --tier 1` or `--tier aot` for native execution.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;
use verum_common::{List, Text};

use crate::config::{CompilationTier, Manifest};
use crate::error::{CliError, Result};
use crate::ui;

/// Execute the `verum run` command
pub fn execute(
    tier: Option<u8>,
    profile: Option<Text>,
    release: bool,
    example: Option<Text>,
    bin: Option<Text>,
    args: List<Text>,
) -> Result<()> {
    // Determine compilation tier. Apply CLI feature overrides so
    // `--tier`, `-Z codegen.tier=...`, etc. are respected here as well.
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;
    crate::feature_overrides::apply_global(&mut manifest)?;

    // Tier resolution priority:
    //   1. Explicit CLI tier (from --tier/--interp/--aot via run_with_tier)
    //   2. [codegen].tier from verum.toml (new unified config system)
    //   3. [profile.dev/release].tier (legacy per-profile config)
    let compilation_tier = if let Some(t) = tier {
        CompilationTier::from_u8(t)
            .ok_or_else(|| CliError::InvalidArgument(format!("Invalid tier {}. Must be 0-1", t)))?
    } else {
        // Check [codegen].tier first (unified config takes precedence).
        match manifest.codegen.tier.as_str() {
            "interpret" => CompilationTier::Interpreter,
            "aot" => CompilationTier::Aot,
            "check" => {
                return Err(CliError::InvalidArgument(
                    "[codegen].tier = \"check\" is for `verum check`, not `verum run`".into(),
                ));
            }
            _ => {
                // Fall back to legacy [profile.dev/release].tier
                let using_release =
                    profile.as_ref().map(|s| s.as_str()) == Some("release") || release;
                manifest.get_profile(using_release).tier
            }
        }
    };

    let mode = if release { "release" } else { "debug" };
    let bin_name = if let Some(b) = bin {
        b
    } else if let Some(e) = example {
        e
    } else {
        manifest.cog.name.clone()
    };

    // Execute based on compilation tier.
    //
    // Tier 0 (interpreter) skips the AOT build entirely and routes
    // through the canonical `CompilationPipeline::run_interpreter`
    // path so it applies the same safety_gate / type_check /
    // cbgr_analysis phases that production `verum build` enforces.
    // Earlier this command rebuilt VbcCodegen + Interpreter in
    // place — that bypassed every static-analysis phase and made
    // `verum run` divergent from `verum build` (and from vtest,
    // which already uses the canonical path).
    //
    // Tier 1 (AOT) builds first then exec's the binary.
    match compilation_tier {
        CompilationTier::Interpreter => {
            run_vbc_interpreted(&manifest_dir, mode, &bin_name, &args)
        }
        CompilationTier::Aot => {
            // Build first (only the AOT path needs a binary on disk).
            super::build::execute(
                profile.clone(),
                None, // refs
                None, // verify
                release,
                None,  // target
                None,  // jobs
                false, // keep_temps
                false, // all_features
                false, // no_default_features
                None,  // features
                false, // timings
                None,  // lto
                false, // static_link
                false, // strip
                false, // strip_debug
                false, // emit_asm
                false, // emit_llvm
                false, // emit_bc
                false, // emit_types
                false, // emit_vbc
                false, // deny_warnings
                false, // strict_intrinsics
                Vec::new(), // deny_lint
                Vec::new(), // warn_lint
                Vec::new(), // allow_lint
                Vec::new(), // forbid_lint
                false,      // smt_stats
            )?;
            run_native(&manifest_dir, mode, &bin_name, &args)
        }
    }
}

/// Run VBC interpreted (Tier 0).
///
/// Routes through the canonical `CompilationPipeline::run_interpreter`,
/// which applies the same load_stdlib_modules / load_project_modules /
/// safety_gate / type_check / dependency_analysis / verify (if SMT) /
/// cbgr_analysis / phase_interpret_with_args sequence that
/// `verum build` and `vtest` already share.
///
/// **History**: this function previously rebuilt `VbcCodegen` +
/// `Interpreter` in place from a manually-merged AST, bypassing
/// every static-analysis phase.  That made `verum run --tier 0`
/// divergent from `verum build` (no safety_gate / type_check /
/// cbgr_analysis) and divergent from `vtest` (which has used the
/// canonical pipeline since #178).  Consolidated 2026-04-27
/// (task #35) so the three entry points all hit the same code
/// path — fixes in the pipeline reach every consumer immediately.
fn run_vbc_interpreted(
    manifest_dir: &PathBuf,
    _mode: &str,
    _bin_name: &Text,
    args: &[Text],
) -> Result<()> {
    use verum_compiler::options::{CompilerOptions, VerifyMode};
    use verum_compiler::pipeline::CompilationPipeline;
    use verum_compiler::session::Session;

    // Resolve entry point.  Prefer src/main.vr (cog convention),
    // fall back to a top-level main.vr for script-style projects.
    let main_file = {
        let primary = manifest_dir.join("src").join("main.vr");
        if primary.exists() {
            primary
        } else {
            let secondary = manifest_dir.join("main.vr");
            if secondary.exists() {
                secondary
            } else {
                return Err(CliError::Custom(format!(
                    "Entry point not found: {}. Create src/main.vr with a fn main() function.",
                    primary.display()
                )));
            }
        }
    };

    ui::status("Interpreting", &format!("{}", main_file.display()));

    let start = Instant::now();

    let options = CompilerOptions {
        input: main_file,
        // `verum run` defaults to Auto verify_mode (matches the
        // CompilerOptions default — runtime + SMT heuristic).
        // Manifest-driven verify_mode propagation is a separate
        // plumbing follow-up to keep this consolidation strictly
        // behaviour-equivalent for the validated subset.
        verify_mode: VerifyMode::Auto,
        ..Default::default()
    };
    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_interpreter(&mut session);

    // Forward CLI args (List<Text>) to main() if its signature accepts them;
    // pipeline.phase_interpret_with_args handles the no-args fallback when
    // main() takes 0 parameters.
    let args_list: List<Text> = args.iter().cloned().collect();

    pipeline
        .run_interpreter(args_list)
        .map_err(|e| CliError::Custom(format!("Runtime error: {}", e)))?;

    let elapsed = start.elapsed();
    ui::note(&format!("executed in {}", ui::format_duration(elapsed)));

    Ok(())
}

/// Run native executable (Tier 1: AOT)
fn run_native(
    manifest_dir: &PathBuf,
    mode: &str,
    bin_name: &Text,
    args: &[Text],
) -> Result<()> {
    let exe_path = manifest_dir
        .join("target")
        .join(mode)
        .join(bin_name.as_str());

    #[cfg(target_os = "windows")]
    let exe_path = exe_path.with_extension("exe");

    if !exe_path.exists() {
        return Err(CliError::Custom(format!(
            "Executable not found: {}. Run 'verum build' first.",
            exe_path.display()
        )));
    }

    ui::status("Running", &format!("`{}`", exe_path.display()));

    let start = Instant::now();

    let mut cmd = Command::new(&exe_path);
    for arg in args {
        cmd.arg(arg.as_str());
    }

    let status = cmd.status().map_err(|e| {
        CliError::Custom(format!("Failed to execute {}: {}", exe_path.display(), e))
    })?;

    let elapsed = start.elapsed();

    if !status.success() {
        let code = status.code().unwrap_or(1);
        ui::error(&format!("process exited with code: {}", code));
        std::process::exit(code);
    }

    ui::note(&format!("completed in {}", ui::format_duration(elapsed)));
    Ok(())
}

// `load_source_modules` / `parse_source_file` removed in task #35:
// the canonical CompilationPipeline does its own multi-file
// discovery via `Session.discover_project_files()` +
// `phase_load_source` / `phase_parse`, so the duplicated
// walkdir+VerumParser scaffolding here was redundant.
