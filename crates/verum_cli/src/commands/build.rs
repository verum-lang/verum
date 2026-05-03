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
    // Windows PE subsystem override (`console` / `gui`). When
    // `None`, the manifest `[build].windows_subsystem` value is
    // used (defaulting to `console`). The CLI flag has higher
    // precedence than the manifest, mirroring how `--target` and
    // `--lto` already shadow their manifest counterparts.
    windows_subsystem_cli: Option<Text>,
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

    // #119 / #110 manifest gate — when `[build].strict_codegen = true`
    // is set in `verum.toml`, surface it via the same
    // `VERUM_STRICT_CODEGEN` env-var pathway used by the CLI flag and
    // CI invocations. Resolution order, highest precedence first:
    //   1. Existing env var (e.g. CI sets it directly).
    //   2. CLI `--strict-codegen` flag (sets the env var on the
    //      caller side before reaching us).
    //   3. This manifest key (sets the env var if neither above did).
    //   4. Default `false` (legacy lenient behaviour).
    // We only set the env var if it's not already present, so the CLI
    // flag and direct env-var settings both win uniformly.
    if manifest.build.strict_codegen && std::env::var("VERUM_STRICT_CODEGEN").is_err() {
        // SAFETY: writing to the process environment is safe in single-
        // threaded CLI startup; the flag is set before the pipeline
        // constructs any threads. Edition-2024 unsafe-edition compatible.
        unsafe {
            std::env::set_var("VERUM_STRICT_CODEGEN", "1");
        }
    }

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
            "none" => VerificationLevel::None,
            "runtime" => VerificationLevel::Runtime,
            "static" => VerificationLevel::Static,
            "fast" => VerificationLevel::Fast,
            "formal" => VerificationLevel::Formal,
            "proof" => VerificationLevel::Proof,
            "thorough" => VerificationLevel::Thorough,
            "reliable" => VerificationLevel::Reliable,
            "certified" => VerificationLevel::Certified,
            "synthesize" => VerificationLevel::Synthesize,
            "complexitytyped" => VerificationLevel::ComplexityTyped,
            "coherentstatic" => VerificationLevel::CoherentStatic,
            "coherentruntime" => VerificationLevel::CoherentRuntime,
            "coherent" => VerificationLevel::Coherent,
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

    // Wire `Profile.debug_assertions` into `CompilerOptions.
    // debug_assertions_override` so the manifest's
    // `[profile.<name>].debug_assertions` reaches the cfg evaluator
    // and gates `@cfg(debug_assertions)` accurately. Pre-fix this
    // field was tracing-only — embedders writing
    // `[profile.dev].debug_assertions = false` (turn the flag OFF
    // despite opt-level=0) saw zero observable effect at the
    // `@cfg(debug_assertions)` gate. Set the override only when
    // the manifest value differs from the auto-derive
    // (`optimization_level == 0`) so callers who don't explicitly
    // configure the field get the unchanged auto-detect behaviour.
    let auto_debug_assertions = options.optimization_level == 0;
    if profile.debug_assertions != auto_debug_assertions {
        options.debug_assertions_override = Some(profile.debug_assertions);
    }

    // Surface still-inert Profile fields. The current build path
    // does not consume `tier` (CompilationTier selection beyond the
    // `--release` flag), `overflow_checks` (panic-on-arithmetic-
    // overflow gate — needs MIR/VBC codegen integration),
    // `codegen_units` (parallel-compilation unit count — needs LLVM
    // backend wiring), or `cbgr_checks` (`All` / `Optimized` /
    // `Proven` per-reference gate — needs CBGR pipeline plumbing).
    //

    // `debug_assertions` IS now wired above; the remaining fields
    // are surfaced via tracing::debug! when set to non-default
    // values so the request is audible at the build entry until
    // the pipeline integration lands.
    let prof_default = crate::config::Profile::default();
    if profile.tier != prof_default.tier
        || profile.overflow_checks != prof_default.overflow_checks
        || profile.codegen_units != prof_default.codegen_units
        || profile.cbgr_checks != prof_default.cbgr_checks
    {
        tracing::debug!(
            "build: profile fields not yet wired into NewCompilerOptions: \
             tier={:?}, overflow_checks={}, codegen_units={:?}, cbgr_checks={:?}",
            profile.tier,
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

    // Windows subsystem resolution. Highest precedence first:
    //  1. CLI `--windows-subsystem` flag (this branch).
    //  2. Manifest `[build].windows_subsystem` if explicitly set.
    //  3. Source-level `@gui` / `@console` attribute on `fn main`
    //  (resolved by the compiler pipeline once the AST is parsed
    //  via `pipeline::resolve_windows_subsystem_from_attrs`).
    //  4. Default Console (the linker defaults when
    //  `options.windows_subsystem` stays None).
    //

    // The CLI flag and an explicit manifest setting are
    // higher-precedence than the source attribute — they represent
    // overrides for a specific build / project — so they're applied
    // here, blocking the attribute-driven step from firing. When
    // both are absent, `options.windows_subsystem` stays `None` and
    // the codegen pipeline scans the AST for `@gui` / `@console`.
    if let Some(ref s) = windows_subsystem_cli {
        match crate::config::WindowsSubsystem::parse(s.as_str()) {
            Some(sub) => {
                options.windows_subsystem = Some(verum_common::Text::from(sub.as_link_flag()));
            }
            None => {
                return Err(CliError::InvalidArgument(format!(
                    "--windows-subsystem expects `console` or `gui`, got `{}`",
                    s
                )));
            }
        }
    } else if let Some(manifest_sub) = manifest.build.windows_subsystem {
        // Manifest-explicit value (Some(Console) / Some(Gui)) wins
        // over the source attribute.
        options.windows_subsystem = Some(verum_common::Text::from(manifest_sub.as_link_flag()));
    } else {
        // Source-attribute resolution. Scan the entry source file
        // for `@gui` / `@console` attached to `fn main`. The scan is
        // textual (comment-stripping + regex-style), not a full AST
        // walk — sufficient for the leaf-level pattern
        //

        //  @gui
        //  fn main() { ... }
        //

        // and avoids parsing the file twice (once here, once during
        // the actual compile pipeline). Edge cases — attribute
        // hidden inside a macro expansion, attribute applied via
        // `@gui fn main` on the same line — are still caught because
        // the scan tolerates arbitrary whitespace + comments between
        // the attribute and the `fn main` token.
        if let Some(sub) = scan_main_subsystem_attribute(&manifest_dir) {
            options.windows_subsystem = Some(verum_common::Text::from(sub));
        }
        // else: leave None — default (Console) applies at link time.
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
    //  - `runtime` / `static` / `fast`: compile-time-only or trivial,
    //  collapse to `Runtime`.
    //  - `coherent_runtime`: ε-monitor emission still runtime-level.
    //  - `complexity_typed`: weak-stratum bounded arithmetic uses
    //  SMT, hence `Proof`.
    //  - `formal` and stricter (incl. `synthesize`, `coherent_*`):
    //  promote to `Proof`.
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
    ui::status(
        "Codegen",
        &format!("{} via LLVM", manifest.cog.name.as_str()),
    );
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
    let files_compiled =
        count_vr_files(&manifest_dir.join("src"))? + count_vr_files(&manifest_dir.join("core"))?;
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

    // Persist / report SMT routing telemetry. See
    // `smt_stats_decision` for the load-bearing contract; the build
    // path branches on the typed enum it returns. Two surfaces gate
    // the persist (CLI `--smt-stats` and manifest
    // `[verify].persist_stats`) — either source asking for
    // persistence is sufficient unless `[verify].enable_telemetry =
    // false` short-circuits both. Closes the inert-defense pattern
    // at config.rs where the [verify] telemetry knobs were
    // populated from manifest but had no consumer (#301).
    match smt_stats_decision(
        smt_stats,
        manifest.verify.persist_stats,
        manifest.verify.enable_telemetry,
    ) {
        SmtStatsDecision::Persist => {
            let json = session.routing_stats().as_json();
            if let Err(e) = crate::commands::smt_stats::persist_stats(&json) {
                ui::warn(&format!("Failed to persist SMT stats: {}", e));
            } else {
                ui::detail("SMT stats", "written — run `verum smt-stats` to view");
            }
        }
        SmtStatsDecision::CliOverridden => {
            ui::warn(
                "--smt-stats requested but [verify].enable_telemetry = false in \
                 manifest; skipping disk persist (set enable_telemetry = true \
                 or remove the manifest override)",
            );
        }
        SmtStatsDecision::Skip => {
            // No source asked for persistence — silent skip.
        }
    }

    // Print warnings (display count since new compiler doesn't provide individual warnings)
    if result.warnings > 0 {
        ui::warn(&format!(
            "{} warning{} emitted",
            result.warnings,
            if result.warnings == 1 { "" } else { "s" }
        ));
        // Display diagnostics from session
        if let Err(e) = session.display_diagnostics() {
            ui::debug(&format!("Failed to display diagnostics: {}", e));
        }
    }

    // Cargo-style finish line
    let profile_name = if using_release { "release" } else { "dev" };
    let opt_tag = if using_release {
        "optimized"
    } else {
        "unoptimized + debuginfo"
    };
    ui::success(&format!(
        "{} [{}] target(s) in {}",
        profile_name,
        opt_tag,
        ui::format_duration(start_time.elapsed())
    ));

    // Show binary path and size
    if result.output_path.exists() {
        let binary_size = std::fs::metadata(&result.output_path)
            .map(|m| ui::format_size(m.len()))
            .unwrap_or_else(|_| "unknown".to_string());
        ui::detail(
            "Binary",
            &format!("{} ({})", result.output_path.display(), binary_size),
        );
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

/// Scan the project's entry source for `@gui` / `@console` on `fn main`.
///

/// Returns the literal `link.exe` `/SUBSYSTEM:` flag value
/// (`"WINDOWS"` for `@gui`, `"CONSOLE"` for `@console`) when the
/// attribute is found, or `None` when neither attribute is present
/// or the entry file doesn't exist.
///

/// The scan is textual — it strips line/block comments and looks for
/// the attribute token immediately followed (modulo whitespace) by
/// the `fn main` declaration. This avoids re-parsing the file via
/// the full AST pipeline (which would happen during the actual
/// compile anyway), keeping the resolution cheap.
///

/// Searched files (in order):
///  1. `src/main.vr` — the conventional application entry.
///  2. `main.vr` at the manifest root — alternative project layout.
///

/// Robust to:
///  * `// line comments` between the attribute and `fn main`
///  * `/* block comments */` between them
///  * Arbitrary whitespace / newlines
///  * Multiple attributes (`@gui\n@inline\nfn main`) — last
///  subsystem-affecting attribute wins, matching the natural
///  reading order.
fn scan_main_subsystem_attribute(manifest_dir: &std::path::Path) -> Option<&'static str> {
    let candidates = [
        manifest_dir.join("src").join("main.vr"),
        manifest_dir.join("main.vr"),
    ];
    let entry_path = candidates.iter().find(|p| p.exists())?;
    let raw = std::fs::read_to_string(entry_path).ok()?;
    let stripped = strip_verum_comments(&raw);

    // Locate the `fn main` token. We accept any whitespace before
    // the `fn`, but the identifier must be exactly `main` followed
    // by `(` or whitespace+`(`.
    let main_idx = find_fn_main_token(&stripped)?;

    // Walk backwards from `fn main` over whitespace + adjacent
    // attributes. Only attributes IMMEDIATELY preceding `fn main`
    // (separated only by whitespace, not by other top-level items)
    // count. An `@gui` attached to a helper function is NOT a
    // subsystem hint for `main`. Last subsystem-affecting attribute
    // among the contiguous prefix wins.
    let prefix = &stripped[..main_idx];
    let mut last_match: Option<&'static str> = None;
    let mut cursor = prefix.trim_end();
    loop {
        // Try to peel one attribute off the end.
        let Some(at_pos) = cursor.rfind('@') else {
            break;
        };
        let attr_slice = &cursor[at_pos..];
        // Identifier: characters after `@` until first non-ident char.
        let ident_end = attr_slice[1..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .map(|i| i + 1)
            .unwrap_or(attr_slice.len());
        let ident = &attr_slice[1..ident_end];
        // Skip the optional `(...)` argument list of the attribute.
        let mut body_end = ident_end;
        if attr_slice.as_bytes().get(body_end) == Some(&b'(') {
            // Consume balanced parens.
            let mut depth: i32 = 0;
            let bytes = attr_slice.as_bytes();
            while body_end < bytes.len() {
                match bytes[body_end] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        body_end += 1;
                        if depth == 0 {
                            break;
                        }
                        continue;
                    }
                    _ => {}
                }
                body_end += 1;
            }
            // Unbalanced paren — bail; this isn't a clean attribute.
            if depth != 0 {
                break;
            }
        }
        // CRITICAL: anything between the attribute body's end and the
        // current cursor end MUST be only whitespace. If there's a
        // `fn helper() {}` or any other code in between, the `@attr`
        // belongs to that earlier construct, NOT to `fn main`. This
        // is the fix for `ignores_at_gui_on_non_main_function` — the
        // pre-fix scanner accepted `@gui` ANYWHERE in the prefix.
        let after_body = &attr_slice[body_end..];
        if !after_body.chars().all(|c| c.is_whitespace()) {
            break;
        }
        match ident {
            "gui" => last_match = last_match.or(Some("WINDOWS")),
            "console" => last_match = last_match.or(Some("CONSOLE")),
            _ => {}
        }
        // Move cursor to before this attribute and continue.
        cursor = cursor[..at_pos].trim_end();
        if cursor.is_empty() || !cursor.contains('@') {
            break;
        }
    }
    last_match
}

/// Strip `// line comments` and `/* block comments */` from Verum
/// source text. Replaces stripped regions with spaces so byte
/// offsets line up with the original — useful for diagnostics
/// downstream, even though the textual scan only consumes the
/// stripped output. Doesn't handle string literals containing `//`
/// — acceptable since attribute placement is at top-level scope
/// where the stripper is safe.
fn strip_verum_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        // Line comment.
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        // Block comment.
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            out.push(' ');
            out.push(' ');
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                out.push(if bytes[i] == b'\n' { '\n' } else { ' ' });
                i += 1;
            }
            if i + 1 < bytes.len() {
                out.push(' ');
                out.push(' ');
                i += 2;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Find the byte offset of the `fn` keyword in `fn main(...)` (top-level).
/// Returns `None` if no such declaration is present.
fn find_fn_main_token(src: &str) -> Option<usize> {
    let mut i = 0;
    let bytes = src.as_bytes();
    while i + 6 < bytes.len() {
        // Look for "fn" preceded by start-of-source / non-ident.
        if bytes[i] == b'f' && bytes[i + 1] == b'n' {
            let prev_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            if prev_ok {
                // Skip whitespace after `fn`.
                let mut j = i + 2;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                // Identifier == "main"?
                if j + 4 <= bytes.len() && &bytes[j..j + 4] == b"main" {
                    let after = j + 4;
                    let after_ok = after >= bytes.len()
                        || !(bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_');
                    if after_ok {
                        // Confirm `(` after main (allowing whitespace).
                        let mut k = after;
                        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                            k += 1;
                        }
                        if k < bytes.len() && (bytes[k] == b'(' || bytes[k] == b'<') {
                            return Some(i);
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
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

/// Outcome of evaluating the SMT-stats persistence policy (#301).
///

/// Three CLI VerifyConfig fields are merged into a single typed
/// decision so the build path branches once and the contract is
/// pin-testable without driving the whole build pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmtStatsDecision {
    /// Persist routing stats to disk + emit the success line.
    /// Reached when telemetry is enabled AND either source (CLI
    /// `--smt-stats` or manifest `[verify].persist_stats`) asked
    /// for persistence.
    Persist,
    /// CLI explicitly asked for persistence but the manifest
    /// disabled telemetry — skip the persist + emit a warning so
    /// the user sees their `--smt-stats` request was dropped.
    CliOverridden,
    /// No source asked for persistence — silent skip.
    Skip,
}

/// Evaluate the SMT-stats persistence policy. Pure function;
/// extracted so the OR-then-AND lattice across CLI and manifest
/// surfaces is regression-pinned.
///

/// Truth table (`telemetry_enabled = true` reduces to `cli ||
/// manifest_persist`; `telemetry_enabled = false` short-circuits):
///

/// | cli_smt_stats | manifest.persist_stats | manifest.enable_telemetry | decision |
/// |---------------|------------------------|---------------------------|---------------|
/// | true | * | true | Persist |
/// | false | true | true | Persist |
/// | false | false | true | Skip |
/// | true | * | false | CliOverridden |
/// | false | * | false | Skip |
pub(crate) fn smt_stats_decision(
    cli_smt_stats: bool,
    manifest_persist_stats: bool,
    manifest_enable_telemetry: bool,
) -> SmtStatsDecision {
    if !manifest_enable_telemetry {
        return if cli_smt_stats {
            SmtStatsDecision::CliOverridden
        } else {
            SmtStatsDecision::Skip
        };
    }
    if cli_smt_stats || manifest_persist_stats {
        SmtStatsDecision::Persist
    } else {
        SmtStatsDecision::Skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: make a temp dir with `src/main.vr` containing the given source.
    fn write_main_vr(src: &str) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("mkdir src");
        std::fs::write(src_dir.join("main.vr"), src).expect("write main.vr");
        tmp
    }

    // ----- #301: smt_stats_decision pin tests --------------------------

    #[test]
    fn smt_stats_cli_only_persists_when_telemetry_enabled() {
        // `--smt-stats` alone with telemetry enabled (default) → Persist.
        assert_eq!(
            smt_stats_decision(true, false, true),
            SmtStatsDecision::Persist
        );
    }

    #[test]
    fn smt_stats_manifest_only_persists_when_telemetry_enabled() {
        // Manifest `persist_stats = true` alone → Persist (no CLI flag).
        assert_eq!(
            smt_stats_decision(false, true, true),
            SmtStatsDecision::Persist
        );
    }

    #[test]
    fn smt_stats_or_combines_cli_and_manifest() {
        // OR-combination: either source asking for persistence → Persist.
        assert_eq!(
            smt_stats_decision(true, true, true),
            SmtStatsDecision::Persist
        );
    }

    #[test]
    fn smt_stats_neither_source_yields_skip() {
        assert_eq!(
            smt_stats_decision(false, false, true),
            SmtStatsDecision::Skip
        );
    }

    #[test]
    fn smt_stats_telemetry_disabled_short_circuits_cli() {
        // CLI explicitly asked but manifest disabled telemetry →
        // CliOverridden (warning, no persist). This is the
        // load-bearing pin: without #301 the CLI flag would
        // silently win and disk persist regardless of manifest.
        assert_eq!(
            smt_stats_decision(true, false, false),
            SmtStatsDecision::CliOverridden
        );
        assert_eq!(
            smt_stats_decision(true, true, false),
            SmtStatsDecision::CliOverridden
        );
    }

    #[test]
    fn smt_stats_telemetry_disabled_silent_skip_when_no_cli() {
        // Telemetry disabled + no CLI request → silent Skip
        // (no warning since the user didn't ask for anything).
        assert_eq!(
            smt_stats_decision(false, false, false),
            SmtStatsDecision::Skip
        );
        assert_eq!(
            smt_stats_decision(false, true, false),
            SmtStatsDecision::Skip
        );
    }

    #[test]
    fn finds_at_gui_above_fn_main() {
        let tmp = write_main_vr("@gui\nfn main() { print(\"hi\"); }\n");
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), Some("WINDOWS"));
    }

    #[test]
    fn finds_at_console_above_fn_main() {
        let tmp = write_main_vr("@console\nfn main() {}\n");
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), Some("CONSOLE"));
    }

    #[test]
    fn returns_none_when_no_attribute() {
        let tmp = write_main_vr("fn main() {}\n");
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), None);
    }

    #[test]
    fn ignores_attribute_in_line_comment() {
        let tmp = write_main_vr("// @gui (commented out)\nfn main() {}\n");
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), None);
    }

    #[test]
    fn ignores_attribute_in_block_comment() {
        let tmp = write_main_vr("/* @gui ignored */\nfn main() {}\n");
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), None);
    }

    #[test]
    fn finds_attribute_with_other_attrs_between() {
        // Multiple attributes: the subsystem-relevant one wins.
        let tmp = write_main_vr("@gui\n@inline\nfn main() {}\n");
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), Some("WINDOWS"));
    }

    #[test]
    fn ignores_at_gui_on_non_main_function() {
        let tmp = write_main_vr("@gui\nfn helper() {}\nfn main() {}\n");
        // Helper picks up @gui — but main is the entry point, not helper.
        // Our scanner walks back from main; @gui is on helper, not main.
        // So the result should be None (or, depending on layout, still
        // pick up @gui if it's textually adjacent). This test pins the
        // documented semantic: only attributes immediately preceding
        // `fn main` count.
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), None);
    }

    #[test]
    fn finds_at_gui_with_blank_lines_between() {
        let tmp = write_main_vr("@gui\n\n\nfn main() {}\n");
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), Some("WINDOWS"));
    }

    #[test]
    fn returns_none_when_no_main_vr() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // No src/ dir at all.
        assert_eq!(scan_main_subsystem_attribute(tmp.path()), None);
    }

    #[test]
    fn windows_subsystem_parse_aliases() {
        use crate::config::WindowsSubsystem;
        assert_eq!(
            WindowsSubsystem::parse("console"),
            Some(WindowsSubsystem::Console)
        );
        assert_eq!(
            WindowsSubsystem::parse("CLI"),
            Some(WindowsSubsystem::Console)
        );
        assert_eq!(
            WindowsSubsystem::parse("Terminal"),
            Some(WindowsSubsystem::Console)
        );
        assert_eq!(WindowsSubsystem::parse("gui"), Some(WindowsSubsystem::Gui));
        assert_eq!(
            WindowsSubsystem::parse("Windows"),
            Some(WindowsSubsystem::Gui)
        );
        assert_eq!(WindowsSubsystem::parse("nonsense"), None);
    }

    #[test]
    fn windows_subsystem_link_flags() {
        use crate::config::WindowsSubsystem;
        assert_eq!(WindowsSubsystem::Console.as_link_flag(), "CONSOLE");
        assert_eq!(WindowsSubsystem::Gui.as_link_flag(), "WINDOWS");
    }
}
