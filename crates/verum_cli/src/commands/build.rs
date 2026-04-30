// Build command implementation
// Multi-tier compilation (interpreter or AOT via LLVM) with transparent cost
// reporting for CBGR overhead, verification time, and context system costs.
// Orchestrates compilation with caching, parallelization, and semantic honesty
//
// NOTE: Migrated to use verum_compiler (the unified compiler) instead of
// the old crate::compiler module. This enables proper multi-file imports
// and full verification pipeline support.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use verum_common::{List, Text};

use verum_compiler::lint::{IntrinsicLint, LintLevel};
use verum_compiler::options::{
    CompilerOptions as NewCompilerOptions, EmitMode, LtoMode, VerifyMode,
};
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;

use crate::config::{Manifest, ReferenceMode, VerificationLevel};
use crate::error::{CliError, Result};
use crate::ui;

/// Execute the `verum build` command
/// Compile the project using the specified profile, reference mode, and
/// verification level. Supports tier 0 (interpreter) and tier 1 (AOT/LLVM).
pub fn execute(
    profile_name: Option<Text>,
    refs: Option<Text>,
    verify: Option<Text>,
    release: bool,
    target: Option<Text>,
    jobs: Option<usize>,
    _keep_temps: bool, // Reserved: compiler backend doesn't support temp retention yet
    all_features: bool,
    no_default_features: bool,
    features: Option<Text>,
    timings: bool,
    // Advanced linking options
    lto: Option<Text>,
    static_link: bool,
    strip: bool,
    strip_debug: bool,
    emit_asm: bool,
    emit_llvm: bool,
    emit_bc: bool,
    emit_types: bool,
    emit_vbc: bool,
    // Lint options
    deny_warnings: bool,
    strict_intrinsics: bool,
    deny_lint: Vec<Text>,
    warn_lint: Vec<Text>,
    allow_lint: Vec<Text>,
    forbid_lint: Vec<Text>,
    // Verification telemetry
    smt_stats: bool,
) -> Result<()> {
    let start_time = Instant::now();

    // Load manifest, then apply CLI-supplied language-feature overrides
    // (high-level flags + -Z key=value pairs) before validation.
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;
    crate::feature_overrides::apply_global(&mut manifest)?;
    manifest.validate()?;

    // Determine profile (dev/release)
    let using_release = profile_name.as_ref().map(|s| s.as_str()) == Some("release") || release;
    let profile = manifest.get_profile(using_release);

    // Build respects [codegen].tier for the compilation mode.
    // "interpret" and "check" are valid for `verum run` / `verum check`;
    // for `verum build` they're informational — build always produces
    // an AOT artifact. We warn if the user set a non-AOT tier so they
    // know the manifest value is noted but overridden.
    let codegen_tier = manifest.codegen.tier.as_str();
    if codegen_tier == "interpret" {
        crate::ui::warn(
            "[codegen].tier = \"interpret\" in verum.toml; \
             `verum build` always produces a native binary. \
             Use `verum run --interp` for interpreter mode.",
        );
    } else if codegen_tier == "check" {
        crate::ui::warn(
            "[codegen].tier = \"check\" in verum.toml; \
             `verum build` always compiles. Use `verum check` for type-check only.",
        );
    }

    // Determine verification level — full nine-strategy ladder
    // (T2.1.1 expansion) plus VFE-6/8 extensions (complexity_typed,
    // coherent_*). Each strategy's ν-coordinate is strictly ordered;
    // cli accepts the lowercase identifier matching `serde(rename_all)`.
    let verification = if let Some(v) = verify {
        match v.as_str() {
            "none"             => VerificationLevel::None,
            "runtime"          => VerificationLevel::Runtime,
            "static"           => VerificationLevel::Static,
            "fast"             => VerificationLevel::Fast,
            "formal"           => VerificationLevel::Formal,
            "proof"            => VerificationLevel::Proof,
            "thorough"         => VerificationLevel::Thorough,
            "reliable"         => VerificationLevel::Reliable,
            "certified"        => VerificationLevel::Certified,
            "synthesize"       => VerificationLevel::Synthesize,
            "complexitytyped"  => VerificationLevel::ComplexityTyped,
            "coherentstatic"   => VerificationLevel::CoherentStatic,
            "coherentruntime"  => VerificationLevel::CoherentRuntime,
            "coherent"         => VerificationLevel::Coherent,
            _ => {
                return Err(CliError::InvalidArgument(format!(
                    "Invalid verification level '{}'. \
Must be one of: none, runtime, static, fast, formal, proof, thorough, reliable, certified, synthesize, complexitytyped, coherentstatic, coherentruntime, coherent \
(see docs/verification/gradual-verification.md)",
                    v
                )));
            }
        }
    } else {
        profile.verification
    };

    // Determine reference mode
    let ref_mode = if let Some(r) = refs {
        match r.as_str() {
            "managed" => ReferenceMode::Managed,
            "checked" => ReferenceMode::Checked,
            "mixed" => ReferenceMode::Mixed,
            _ => {
                return Err(CliError::InvalidArgument(format!(
                    "Invalid reference mode '{}'. Must be: managed, checked, or mixed",
                    r
                )));
            }
        }
    } else {
        ReferenceMode::Managed
    };

    // Cargo-style compilation header
    ui::status(
        "Compiling",
        &format!(
            "{} v{} ({})",
            manifest.cog.name.as_str(),
            manifest.cog.version.as_str(),
            manifest_dir.display()
        ),
    );

    // Parse features
    let feature_list = if all_features {
        manifest.features.keys().cloned().collect()
    } else {
        let mut feats = if !no_default_features {
            manifest
                .features
                .get(&Text::from("default"))
                .cloned()
                .unwrap_or_default()
        } else {
            List::new()
        };

        if let Some(f) = features {
            let parsed: List<Text> = f
                .as_str()
                .split(',')
                .map(|s| Text::from(s.trim()))
                .collect();
            feats.extend(parsed);
        }

        feats
    };

    // Create new compiler options (using verum_compiler). The unified
    // language-feature set is built from the merged manifest (defaults
    // → verum.toml → CLI overrides) and validated up-front so invalid
    // combinations fail fast, before any pipeline phase runs.
    let language_features = crate::feature_overrides::manifest_to_features(&manifest)?;
    let mut options = NewCompilerOptions::default();
    options.language_features = language_features;
    options.input = manifest_dir.join("src");
    options.output = manifest_dir
        .join("target")
        .join(if using_release { "release" } else { "debug" })
        .join(&manifest.cog.name);
    // Ensure output directory exists
    if let Some(parent) = options.output.parent() {
        std::fs::create_dir_all(parent).map_err(CliError::Io)?;
    }
    options.optimization_level = profile.opt_level;
    options.num_threads = jobs.unwrap_or_else(num_cpus::get);
    options.incremental = profile.incremental && !using_release;
    options.verbose = if timings { 2 } else { 0 };

    // Honour the user-supplied `--jobs N` (or the default
    // num_cpus::get()) by pinning rayon's global thread pool to
    // that count. Without this call, `options.num_threads` was
    // displayed by the CLI ("Jobs: 4") but rayon's parallel work
    // — module loading (verum_modules::parallel) and contract
    // verification — used rayon's default pool size regardless.
    // `verum fmt` and `verum test` already do this initialization
    // (main.rs:2648-2653, commands/test.rs:296). Use a discarded
    // Result because `build_global()` errors only if the pool was
    // already initialized in this process — that's fine, the
    // first init wins and subsequent CLI subcommands within the
    // same process keep that count.
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(options.num_threads)
        .build_global();

    // Surface inert Profile fields. The current build path
    // consumes `verification`, `opt_level`, `incremental`, `lto`
    // (via linker_config) — but not `tier` (CompilationTier
    // selection beyond the `--release` flag), `debug_assertions`
    // (runtime debug_assertions! macro gate), `overflow_checks`
    // (panic-on-arithmetic-overflow gate), `codegen_units`
    // (parallel-compilation unit count), or `cbgr_checks`
    // (`All` / `Optimized` / `Proven` per-reference gate).
    //
    // Embedders writing `[profile.dev].overflow_checks = true`
    // or `[profile.release].cbgr_checks = "Proven"` saw zero
    // observable effect. Surface the values via tracing::debug!,
    // gated on any non-default value, so the request is audible
    // at the build entry until the pipeline integration lands.
    let prof_default = crate::config::Profile::default();
    if profile.tier != prof_default.tier
        || profile.debug_assertions != prof_default.debug_assertions
        || profile.overflow_checks != prof_default.overflow_checks
        || profile.codegen_units != prof_default.codegen_units
        || profile.cbgr_checks != prof_default.cbgr_checks
    {
        tracing::debug!(
            "build: profile fields not yet wired into NewCompilerOptions: \
             tier={:?}, debug_assertions={}, overflow_checks={}, \
             codegen_units={:?}, cbgr_checks={:?}",
            profile.tier,
            profile.debug_assertions,
            profile.overflow_checks,
            profile.codegen_units,
            profile.cbgr_checks,
        );
    }

    // Advanced linking options
    if let Some(ref lto_mode) = lto {
        if let Some(mode) = LtoMode::from_str(lto_mode.as_str()) {
            options.lto = true;
            options.lto_mode = Some(mode);
        } else {
            return Err(CliError::InvalidArgument(format!(
                "Invalid LTO mode '{}'. Valid modes: thin, full",
                lto_mode
            )));
        }
    }
    options.static_link = static_link;
    options.strip_symbols = strip;
    options.strip_debug = strip_debug;

    // Emit mode (mutually exclusive, checked in order of priority)
    if emit_asm {
        options.emit_mode = EmitMode::Assembly;
    } else if emit_llvm {
        options.emit_mode = EmitMode::LlvmIr;
    } else if emit_bc {
        options.emit_mode = EmitMode::Bitcode;
    }

    // Emit type metadata (.vtyp)
    options.emit_types = emit_types;

    // Emit VBC bytecode dump (.vbc.txt)
    options.emit_vbc = emit_vbc;

    // Set target triple for cross-compilation / @cfg evaluation.
    // Precedence: --target CLI flag > [llvm].target_triple in
    // verum.toml > host default. Pre-fix the manifest's
    // `[llvm]` block was parsed but never plumbed downstream —
    // declaring `target_triple = "x86_64-unknown-linux-gnu"`
    // in `verum.toml` had zero effect; users had to pass
    // `--target` on every invocation.
    if let Some(ref target) = target {
        options.target_triple = Some(verum_common::Text::from(target.as_str()));
    } else if let Some(ref triple) = manifest.llvm.target_triple {
        options.target_triple = Some(triple.clone());
    }

    // Wire `[llvm].target_cpu` / `[llvm].target_features` from
    // `verum.toml` into the AOT pipeline. CLI doesn't expose these
    // directly today (only `--target` is available); the manifest is
    // the user-facing knob. Fields default to `None`, in which case
    // the AOT pipeline falls back to host-CPU detection (or
    // `"generic"` / empty for WASM cross-builds).
    if options.target_cpu.is_none() && manifest.llvm.target_cpu.is_some() {
        options.target_cpu = manifest.llvm.target_cpu.clone();
    }
    if options.target_features.is_none() && manifest.llvm.target_features.is_some() {
        options.target_features = manifest.llvm.target_features.clone();
    }

    // Pass features to compiler for @cfg(feature = "...") evaluation
    options.cfg_features = feature_list
        .iter()
        .map(|f| verum_common::Text::from(f.as_str()))
        .collect();

    // Map the 9-strategy ladder + VFE-6/8 extensions to the
    // compiler's coarser `VerifyMode` until the SMT crate exposes
    // per-strategy dispatch (T2.1 — fine-grained backend wiring).
    // The mapping honours the ν-coordinate ordering:
    //   - `runtime` / `static` / `fast`: compile-time-only or trivial,
    //     collapse to `Runtime`.
    //   - `coherent_runtime`: ε-monitor emission still runtime-level.
    //   - `complexity_typed`: weak-stratum bounded arithmetic uses
    //     SMT, hence `Proof`.
    //   - `formal` and stricter (incl. `synthesize`, `coherent_*`):
    //     promote to `Proof`.
    options.verify_mode = match verification {
        VerificationLevel::None
        | VerificationLevel::Runtime
        | VerificationLevel::Static
        | VerificationLevel::Fast
        | VerificationLevel::CoherentRuntime => VerifyMode::Runtime,
        VerificationLevel::Formal
        | VerificationLevel::Proof
        | VerificationLevel::Thorough
        | VerificationLevel::Reliable
        | VerificationLevel::Certified
        | VerificationLevel::Synthesize
        | VerificationLevel::ComplexityTyped
        | VerificationLevel::CoherentStatic
        | VerificationLevel::Coherent => VerifyMode::Proof,
    };

    // Configure lint options
    options.lint_config.deny_warnings = deny_warnings;
    options.lint_config.strict_intrinsics = strict_intrinsics;

    // Apply per-lint settings (forbid has highest priority, then deny, warn, allow)
    for lint_name in &allow_lint {
        if let Some(lint) = IntrinsicLint::from_str(lint_name.as_str()) {
            options.lint_config.set_lint_level(lint, LintLevel::Allow);
        }
    }
    for lint_name in &warn_lint {
        if let Some(lint) = IntrinsicLint::from_str(lint_name.as_str()) {
            options.lint_config.set_lint_level(lint, LintLevel::Warn);
        }
    }
    for lint_name in &deny_lint {
        if let Some(lint) = IntrinsicLint::from_str(lint_name.as_str()) {
            options.lint_config.set_lint_level(lint, LintLevel::Deny);
        }
    }
    for lint_name in &forbid_lint {
        if let Some(lint) = IntrinsicLint::from_str(lint_name.as_str()) {
            options.lint_config.set_lint_level(lint, LintLevel::Forbid);
        }
    }

    if timings {
        ui::detail("Input", &format!("{}", options.input.display()));
        ui::detail("Output", &format!("{}", options.output.display()));
        ui::detail("Opt level", &format!("{}", options.optimization_level));
        ui::detail("Jobs", &format!("{}", options.num_threads));
    }

    // Create session and compilation pipeline
    let analysis_start = Instant::now();
    ui::status("Parsing", &format!("{}", manifest.cog.name.as_str()));

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    // Compile via the unified dispatch — `pipeline.run()` reads
    // `session.options().check_only` and routes to the type-only
    // path or the AOT path. Pre-fix this site always called
    // `run_native_compilation` directly, so the `check_only` flag
    // was inert for the build path; the unified dispatch makes
    // `options.check_only = true` skip codegen + linking.
    // Note: Stdlib is now embedded directly from source files in verum_compiler
    ui::status("Codegen", &format!("{} via LLVM", manifest.cog.name.as_str()));
    let run_result = pipeline
        .run()
        .map_err(|e| CliError::CompilationFailed(e.to_string()))?;
    let output = match &run_result {
        verum_compiler::pipeline::RunResult::Built(p) => {
            ui::status("Linking", &format!("{}", manifest.cog.name.as_str()));
            p.clone()
        }
        verum_compiler::pipeline::RunResult::Checked => {
            ui::success(&format!(
                "Check OK ({} v{}) — codegen skipped (check_only)",
                manifest.cog.name.as_str(),
                manifest.cog.version.as_str(),
            ));
            return Ok(());
        }
    };

    // GPU compilation path (MLIR) — auto-detected by the pipeline.
    // When the AST scanner finds @device(gpu) annotations on functions,
    // pipeline.run_native_compilation() automatically invokes run_mlir_aot()
    // to produce GPU kernel binaries alongside the CPU binary.
    // No explicit --gpu flag is required.
    let files_compiled = count_vr_files(&manifest_dir.join("src"))?
        + count_vr_files(&manifest_dir.join("core"))?;
    let output_path = output;

    // Get metrics from session - real timings tracked during compilation
    let session_metrics = session.get_build_metrics();

    // Build result struct for compatibility with existing print functions
    let result = BuildResult {
        output_path,
        files_compiled,
        files_cached: 0, // New compiler doesn't track this yet
        warnings: session.warning_count(),
        duration: analysis_start.elapsed(),
        metrics: BuildMetrics {
            parse_time: session_metrics.parse_time,
            typecheck_time: session_metrics.typecheck_time,
            codegen_time: session_metrics.codegen_time,
            optimization_time: session_metrics.optimization_time,
            link_time: session_metrics.link_time,
            total_lines: session_metrics.total_lines,
        },
    };

    let _analysis_time = analysis_start.elapsed();

    // Persist / report SMT routing telemetry when --smt-stats is on.
    // The session's RoutingStats is populated by any verification phase
    // that dispatches through SmtBackendSwitcher (see Task #42 for the
    // phase-side wiring). Even when no SMT work ran, we still write a
    // zero-filled report so `verum smt-stats` has something to show.
    if smt_stats {
        let json = session.routing_stats().as_json();
        if let Err(e) = crate::commands::smt_stats::persist_stats(&json) {
            ui::warn(&format!("Failed to persist SMT stats: {}", e));
        } else {
            ui::detail(
                "SMT stats",
                "written — run `verum smt-stats` to view",
            );
        }
    }

    // Print warnings (display count since new compiler doesn't provide individual warnings)
    if result.warnings > 0 {
        ui::warn(&format!("{} warning{} emitted", result.warnings,
            if result.warnings == 1 { "" } else { "s" }));
        // Display diagnostics from session
        if let Err(e) = session.display_diagnostics() {
            ui::debug(&format!("Failed to display diagnostics: {}", e));
        }
    }

    // Cargo-style finish line
    let profile_name = if using_release { "release" } else { "dev" };
    let opt_tag = if using_release { "optimized" } else { "unoptimized + debuginfo" };
    ui::success(&format!(
        "{} [{}] target(s) in {}",
        profile_name, opt_tag,
        ui::format_duration(start_time.elapsed())
    ));

    // Show binary path and size
    if result.output_path.exists() {
        let binary_size = std::fs::metadata(&result.output_path)
            .map(|m| ui::format_size(m.len()))
            .unwrap_or_else(|_| "unknown".to_string());
        ui::detail("Binary", &format!(
            "{} ({})",
            result.output_path.display(),
            binary_size
        ));
    }

    // Show phase timings if available
    if result.metrics.total_lines > 0 {
        let lines_per_sec = if result.duration.as_secs_f64() > 0.0 {
            result.metrics.total_lines as f64 / result.duration.as_secs_f64()
        } else {
            0.0
        };
        ui::note(&format!(
            "{} lines, {:.0} lines/sec | parse {} | typecheck {} | codegen {} | link {}",
            result.metrics.total_lines,
            lines_per_sec,
            ui::format_duration(result.metrics.parse_time),
            ui::format_duration(result.metrics.typecheck_time),
            ui::format_duration(result.metrics.codegen_time),
            ui::format_duration(result.metrics.link_time),
        ));
    }

    // CBGR cost transparency (semantic honesty)
    let cbgr_note = match ref_mode {
        ReferenceMode::Managed => "CBGR ~15ns/check (use &checked for hot paths)",
        ReferenceMode::Checked => "CBGR 0ns (statically verified)",
        ReferenceMode::Mixed => "CBGR ~5-15ns avg (escape analysis active)",
    };
    ui::note(cbgr_note);

    Ok(())
}


// ============================================================================
// Helper Types and Functions (for compatibility with existing UI)
// ============================================================================

/// Build result from the new compiler pipeline
struct BuildResult {
    output_path: PathBuf,
    files_compiled: usize,
    files_cached: usize,
    warnings: usize,
    duration: Duration,
    metrics: BuildMetrics,
}

/// Build metrics for performance reporting
#[derive(Default)]
struct BuildMetrics {
    parse_time: Duration,
    typecheck_time: Duration,
    codegen_time: Duration,
    optimization_time: Duration,
    link_time: Duration,
    total_lines: usize,
}

/// Count .vr files in a directory
fn count_vr_files(dir: &PathBuf) -> Result<usize> {
    if !dir.exists() {
        return Ok(0);
    }

    let mut count = 0;
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            // Only .vr extension is valid
            if ext == "vr" && path.is_file() {
                count += 1;
            }
        }
    }
    Ok(count)
}
