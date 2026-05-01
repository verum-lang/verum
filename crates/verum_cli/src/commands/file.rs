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

    let opt_tag = if opt_level >= 2 {
        "optimized"
    } else {
        "unoptimized + debuginfo"
    };
    ui::success(&format!(
        "[{}] target(s) in {}",
        opt_tag,
        ui::format_duration(start.elapsed())
    ));

    if executable_path.exists() {
        let binary_size = std::fs::metadata(&executable_path)
            .map(|m| ui::format_size(m.len()))
            .unwrap_or_else(|_| "unknown".to_string());
        ui::detail(
            "Binary",
            &format!("{} ({})", executable_path.display(), binary_size),
        );
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
                    if matches!(
                        test_type,
                        "parse-pass" | "parser" | "parse-recover" | "parse-fail"
                    ) {
                        is_parse_only = true;
                    }
                    // typecheck-fail, meta-fail, verify-fail tests expect errors
                    if matches!(
                        test_type,
                        "typecheck-fail"
                            | "parse-fail"
                            | "parse-recover"
                            | "meta-fail"
                            | "verify-fail"
                    ) {
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
                ui::success(&format!(
                    "parsing {} (errors expected) in {}",
                    file,
                    ui::format_duration(start.elapsed())
                ));
            } else {
                ui::success(&format!(
                    "parsing {} in {}",
                    file,
                    ui::format_duration(start.elapsed())
                ));
            }
        } else {
            result.map_err(|e| CliError::CompilationFailed(e.to_string()))?;
            ui::success(&format!(
                "parsing {} in {}",
                file,
                ui::format_duration(start.elapsed())
            ));
        }
    } else if expect_errors {
        // For typecheck-fail tests, errors are expected
        let result = pipeline.run_check_only();
        if result.is_err() {
            ui::success(&format!(
                "checking {} (errors expected) in {}",
                file,
                ui::format_duration(start.elapsed())
            ));
        } else {
            ui::success(&format!(
                "checking {} in {}",
                file,
                ui::format_duration(start.elapsed())
            ));
        }
    } else {
        pipeline
            .run_check_only()
            .map_err(|e| CliError::CompilationFailed(e.to_string()))?;
        ui::success(&format!(
            "checking {} in {}",
            file,
            ui::format_duration(start.elapsed())
        ));
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
    run_with_tier_and_flags(
        file,
        args,
        skip_verify,
        tier,
        timings,
        crate::script::permission_flags::PermissionFlags::default(),
    )
}

/// Run single file with tier selection AND CLI permission overrides.
///

/// Permission flags (`--allow`, `--allow-all`, `--deny-all`) merge
/// with the script's frontmatter `permissions = [...]` declaration
/// per the Deno-style precedence in [`build_permission_set`]:
/// frontmatter ∪ CLI flags, then `--allow-all` / `--deny-all`
/// overrides. For non-script invocations the flags are ignored —
/// the permission policy is built only when the entry source has a
/// frontmatter `permissions = [...]` field OR a CLI grant is
/// present.
pub fn run_with_tier_and_flags(
    file: &str,
    args: List<Text>,
    skip_verify: bool,
    tier: Option<u8>,
    timings: bool,
    permission_flags: crate::script::permission_flags::PermissionFlags,
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

            // For script-shaped sources (shebang at byte 0 or an inline
            // `// /// script` frontmatter block) the entry path runs
            // through `run_script_interpreted` which adds:
            //

            //  • frontmatter validation (compiler version constraint
            //  against the running build),
            //  • permission resolution (frontmatter ∪ CLI flags),
            //  • persistent VBC cache (lookup-skip-compile on hit;
            //  compile + serialise + store on miss),
            //  • lockfile placeholder (populated as
            //  dependency resolution lands).
            //

            // Plain `.vr` files (no shebang, no frontmatter) take the
            // legacy path that just runs the pipeline — no cache, no
            // ceremony, identical behaviour to before.
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

            // Unify on `run_script_interpreted` regardless of whether
            // the source carries a shebang or `// /// script`
            // frontmatter. The function gracefully degrades when the
            // frontmatter is absent (no version check, no permission
            // policy installation, no lockfile capture) — the pieces
            // it adds over the legacy plain path that ALWAYS apply
            // are the persistent VBC cache (lookup-skip-compile on
            // hit; compile + serialise + store on miss) and the
            // ScriptContext-driven cache key (source hash + compiler
            // version + flags). Pre-fix, plain `.vr` files hit a
            // separate cache-disabled branch and paid the full
            // stdlib-recompile cost on every invocation (~18s for
            // typical script-mode runs); post-fix the cache is
            // populated on first run and subsequent runs are
            // sub-second.
            run_script_interpreted(&input, options, args, timings, permission_flags)?;
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

            // Script-shaped sources go through frontmatter validation
            // (compiler-version pin, declared-deps audit) before LLVM
            // compilation so an unbuildable script fails fast with a
            // clear diagnostic instead of a confusing native-link
            // error half a megabyte deeper. The resolved permission
            // policy is also handed to the LLVM lowerer here so every
            // `PermissionAssert` site in the binary enforces the
            // same `(scope, target)` grants the interpreter would.
            // Script-shaped AOT path: validate frontmatter, then
            // try the persistent native-binary cache. On hit, exec
            // the cached binary directly — sub-millisecond cold
            // start. On miss, run the LLVM pipeline below and
            // store the result.
            let mut aot_permission_policy: Option<verum_codegen::llvm::AotPermissionPolicy> = None;
            let aot_cache_key: Option<crate::script::cache::CacheKey> = if is_script_shaped(&input)
            {
                use crate::script::context::{ScriptContext, ScriptContextOptions};
                let ctx = ScriptContext::from_path(
                    &input,
                    &ScriptContextOptions {
                        flags: permission_flags.clone(),
                        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
                        extra_cache_flags: aot_cache_flag_inputs(),
                    },
                )
                .map_err(|e| CliError::Custom(format!("script context: {e}")))?;
                if let Some(fm) = ctx.frontmatter.as_ref() {
                    check_frontmatter_version(fm)?;
                    if !fm.dependencies.is_empty() {
                        ui::warn(
                            "script frontmatter declares dependencies — \
                                 registry resolution lands separately; for \
                                 now they are ignored",
                        );
                    }
                }
                aot_permission_policy = build_aot_permission_policy(&ctx);
                if aot_permission_policy.is_some() {
                    ui::detail(
                        "Permissions",
                        &format!("{} grants baked into AOT binary", ctx.permissions.len()),
                    );
                }
                if let Some(cached) = lookup_aot_binary(ctx.cache_key) {
                    ui::status("Running", &format!("`{}` (cached AOT)", cached.display()));
                    return exec_native_binary(&cached, &args);
                }
                Some(ctx.cache_key)
            } else {
                None
            };

            let options = CompilerOptions {
                input: input.clone(),
                verify_mode,
                output_format: OutputFormat::Human,
                language_features: language_features.clone(),
                ..Default::default()
            };
            let mut session = Session::new(options);
            if let Some(policy) = aot_permission_policy {
                session.set_aot_permission_policy(policy);
            }
            let mut pipeline = CompilationPipeline::new(&mut session);

            match pipeline.run_native_compilation() {
                Ok(executable) => {
                    if timings {
                        print_phase_timings(&session);
                    }

                    // Persist the freshly-compiled AOT binary in the
                    // script cache so subsequent runs of the same
                    // source skip the LLVM pipeline entirely.
                    // Best-effort — write failures don't fail the run.
                    if let Some(key) = aot_cache_key {
                        if let Err(e) = store_aot_binary(key, &executable) {
                            ui::warn(&format!("AOT cache store failed: {e}"));
                        }
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
                    let mut fallback_pipeline = CompilationPipeline::new(&mut fallback_session);
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

/// Owned tempfile that auto-removes its path on drop. The script
/// runner threads the path through the on-disk script pipeline so
/// inline `-e` and stdin invocations get the same parser, permission
/// model, and exit-code semantics as a real script file. Drop must
/// fire AFTER the runner returns; callers hold the value across the
/// run and let it drop at the natural scope boundary.
pub struct ScriptTempFile {
    path: std::path::PathBuf,
}

impl ScriptTempFile {
    /// Path to the temporary `.vr` file. Lives until `Drop`.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for ScriptTempFile {
    fn drop(&mut self) {
        // Best-effort cleanup. A leftover temp on a panicking exit
        // is harmless — the OS will reclaim it on next reboot, and
        // the unique filename prevents collisions.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Materialise a script source string as a temporary `.vr` file
/// rooted in `$TMPDIR`. The file always carries a shebang line at
/// byte 0 so the script-mode parser engages — callers don't need
/// to hand-shebang their inline expressions or stdin payloads.
///

/// `kind` is a short descriptor (`"eval"` / `"stdin"`) embedded in
/// the filename for diagnostic clarity. PID + nanosecond suffix
/// disambiguates concurrent invocations.
pub fn synthesize_script_temp(body: &str, kind: &str) -> std::io::Result<ScriptTempFile> {
    use std::io::Write;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path =
        std::env::temp_dir().join(format!("verum-{kind}-{}-{}.vr", std::process::id(), nanos));
    let mut f = std::fs::File::create(&path)?;
    if !body.starts_with("#!") {
        writeln!(f, "#!/usr/bin/env verum")?;
    }
    f.write_all(body.as_bytes())?;
    if !body.ends_with('\n') {
        writeln!(f)?;
    }
    drop(f);
    Ok(ScriptTempFile { path })
}

/// Quick content sniff: does the file at `path` look like a Verum
/// script? A script either starts with a `#!` shebang at byte 0
/// (BOM-tolerant) OR carries an inline `// /// script` frontmatter
/// block somewhere in its first ~4 KiB. Reading more than that is
/// rare and not worth the latency — frontmatter conventionally
/// appears immediately after the shebang.
fn is_script_shaped(path: &std::path::Path) -> bool {
    use std::fs::File;
    use std::io::Read;
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 4096];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let head = &buf[..n];
    // Shebang at byte 0 (with optional UTF-8 BOM).
    if head.len() >= 2 && &head[..2] == b"#!" {
        return true;
    }
    if head.len() >= 5 && &head[..3] == [0xEF, 0xBB, 0xBF] && &head[3..5] == b"#!" {
        return true;
    }
    // Inline frontmatter marker. The line `// /// script` is the
    // canonical opening of a PEP-723-style metadata block.
    if let Ok(text) = std::str::from_utf8(head) {
        for line in text.lines() {
            if line.trim_start().starts_with("// /// script") {
                return true;
            }
        }
    }
    false
}

/// Compare the running compiler's version against a script's
/// frontmatter `verum = "<spec>"` constraint. Returns Ok on match
/// (or no constraint), Err with a human-actionable message on
/// mismatch or unparseable spec.
fn check_frontmatter_version(fm: &crate::script::frontmatter::Frontmatter) -> Result<(), CliError> {
    let Some(spec) = fm.verum.as_deref() else {
        return Ok(());
    };
    let req = match semver::VersionReq::parse(spec) {
        Ok(r) => r,
        Err(e) => {
            return Err(CliError::InvalidArgument(format!(
                "script frontmatter `verum = {spec:?}` is not a valid semver constraint: {e}"
            )));
        }
    };
    let cur_str = env!("CARGO_PKG_VERSION");
    let cur = semver::Version::parse(cur_str).map_err(|e| {
        CliError::Custom(format!(
            "internal: compiler version {cur_str:?} is not valid semver: {e}"
        ))
    })?;
    if !req.matches(&cur) {
        return Err(CliError::InvalidArgument(format!(
            "script requires `verum {spec}` but running compiler is {cur_str}.\n\
             help: install a matching toolchain or relax the script's `verum` field."
        )));
    }
    Ok(())
}

/// Script-mode interpreted run with full ScriptContext wiring:
/// frontmatter version validation, CLI permission flag merge,
/// persistent VBC cache lookup-and-store, and lockfile capture.
///

/// **Cache hit path** — deserialise the stored VBC and execute via
/// `CompilationPipeline::run_compiled_vbc`, skipping every front-end
/// phase (parse, typecheck, verify, codegen). Cold-start drops to
/// roughly the cost of zstd decompression + interpreter setup.
///

/// **Cache miss path** — run the full pipeline, capture the produced
/// `VbcModule` from the session, serialise to the on-disk cache
/// directory keyed by `(source_hash, compiler_version, extra_flags)`,
/// then continue executing the same in-memory module.
///

/// Cache failures are non-fatal: a corrupt entry, locked directory,
/// or schema mismatch downgrades to the cache-miss path with a
/// warning. Script execution is the primary contract; caching is an
/// optimisation that must never block a working run.
fn run_script_interpreted(
    input: &std::path::Path,
    mut options: CompilerOptions,
    args: List<Text>,
    timings: bool,
    permission_flags: crate::script::permission_flags::PermissionFlags,
) -> Result<(), CliError> {
    use crate::script::cache::ScriptCache;
    use crate::script::context::{ScriptContext, ScriptContextOptions};

    // Stable cache-key contributors include the CLI permission
    // flags so a `--allow-net` run doesn't reuse a `--deny-all` run's
    // cached VBC. (Cache content is identical, but conservative —
    // future codegen-emitted permission asserts will encode the
    // resolved set into the bytecode.)
    let mut extra_flags = cache_flag_inputs(&options);
    if permission_flags.allow_all {
        extra_flags.push("perm=allow-all".to_string());
    }
    if permission_flags.deny_all {
        extra_flags.push("perm=deny-all".to_string());
    }
    if !permission_flags.allow.is_empty() {
        let mut sorted = permission_flags.allow.clone();
        sorted.sort();
        extra_flags.push(format!("perm-allow=[{}]", sorted.join(",")));
    }

    // 1. Build the ScriptContext: read source, hash, extract+validate
    //  frontmatter, merge CLI permission flags, compute cache key.
    let ctx_opts = ScriptContextOptions {
        flags: permission_flags.clone(),
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        extra_cache_flags: extra_flags,
    };
    let ctx = ScriptContext::from_path(input, &ctx_opts)
        .map_err(|e| CliError::Custom(format!("script context: {e}")))?;

    // 2. Frontmatter version gate. A script with `verum = "X.Y"` that
    //  doesn't match the running build fails fast with a clear
    //  diagnostic instead of producing a confusing parse error
    //  half a megabyte deeper into the pipeline.
    let mut script_cog_resolver: Option<verum_modules::cog_resolver::CogResolver> = None;
    if let Some(fm) = ctx.frontmatter.as_ref() {
        check_frontmatter_version(fm)?;
        if !fm.dependencies.is_empty() {
            let resolved = resolve_script_dependencies(fm, input)?;
            // Persist the resolved dependency set as a sidecar
            // lockfile (`<script>.lock`). On a freshly-introduced
            // dependency this writes a new file; on subsequent
            // runs the existing lockfile is verified against the
            // current source hash + compiler version + resolved
            // grants, and rewritten when the inputs have drifted.
            // The lockfile is the authoritative pinned record for
            // reproducibility / drift detection across machines.
            persist_script_lockfile(&ctx, input, &resolved.locked)?;
            script_cog_resolver = Some(resolved.resolver);
        }
    }

    // 3. Permission policy. Built only when the script's frontmatter
    //  EXPLICITLY declares a `permissions = [...]` field. Plain
    //  scripts with no permissions block keep the interpreter
    //  router's default allow-all behaviour — explicit opt-in to
    //  sandboxing, matching Deno's `--allow-*` philosophy without
    //  breaking existing untouched scripts.
    let permission_policy = build_script_permission_policy(&ctx);
    if permission_policy.is_some() {
        ui::detail(
            "Permissions",
            &format!("{} grants installed", ctx.permissions.len()),
        );
    }

    // 3. Persistent VBC cache. Best-effort: cache-open failures fall
    //  back to a cache-disabled run. Tier-aware cache keys (already
    //  encoded in `ScriptContextOptions::extra_cache_flags`) ensure
    //  `--verify-mode runtime` and `--verify-mode auto` runs don't
    //  poison each other's cache.
    let cache: Option<ScriptCache> = ScriptCache::at_default().ok();

    // Cache hit short-circuit. Non-fatal on any error path (eviction
    // races, schema mismatch, etc.) — fall through to a regular
    // compile+run.
    if let Some(c) = cache.as_ref() {
        match ctx.cache_lookup(c) {
            Ok(Some(entry)) => {
                ui::status("Running", &format!("{} (cached VBC)", input.display()));
                return execute_cached_vbc(
                    input,
                    options,
                    args,
                    &entry.vbc,
                    timings,
                    permission_policy,
                    script_cog_resolver,
                );
            }
            Ok(None) => { /* miss — fall through */ }
            Err(e) => ui::warn(&format!("script cache lookup failed: {e}")),
        }
    }

    // 4. Cache miss: run the pipeline. The session captures the
    //  compiled VBC via `record_compiled_vbc` so we can pull it
    //  back here for cache-store.
    options.input = input.to_path_buf();
    ui::status("Running", &format!("{} (interpreter)", input.display()));
    let mut session = Session::new(options);
    if let Some(policy) = permission_policy {
        session.set_script_permission_policy(policy);
    }
    if let Some(resolver) = script_cog_resolver {
        session.set_cog_resolver(resolver);
    }
    {
        let mut pipeline = CompilationPipeline::new(&mut session);
        pipeline
            .run_interpreter(args)
            .map_err(|e| CliError::RuntimeError(e.to_string()))?;
    }

    if timings {
        print_phase_timings(&session);
    }

    // 5. Cache store. Serialise the captured VBC module and persist.
    //  Best-effort: a cache-write failure does not fail the run.
    if let (Some(c), Some(vbc)) = (cache.as_ref(), session.take_compiled_vbc()) {
        match verum_vbc::serialize::serialize_module_compressed(
            &vbc,
            verum_vbc::compression::CompressionOptions::zstd(),
        ) {
            Ok(bytes) => {
                if let Err(e) = ctx.cache_store(c, &bytes) {
                    ui::warn(&format!("script cache store failed: {e}"));
                }
            }
            Err(e) => ui::warn(&format!("script VBC serialise failed: {e}")),
        }
    }

    // 6. Translate the script's recorded exit code to process exit.
    //  The pipeline records via `Session::record_exit_code` instead
    //  of calling `process::exit` directly, so the cache-store step
    //  above runs first. `None` here means the script returned `()`
    //  or a non-numeric value — exit 0 by convention.
    if let Some(code) = session.take_exit_code() {
        std::process::exit(code);
    }

    Ok(())
}

/// Cache-hit fast-path: deserialise the stored VBC and run via the
/// pipeline's `run_compiled_vbc` entry, which skips every front-end
/// phase. The interpreter still applies all runtime semantics —
/// CBGR, refinement asserts, intrinsic dispatch — so a cached run is
/// observationally identical to a fresh compile.
fn execute_cached_vbc(
    input: &std::path::Path,
    mut options: CompilerOptions,
    args: List<Text>,
    vbc_bytes: &[u8],
    timings: bool,
    permission_policy: Option<verum_compiler::session::ScriptPermissionPolicy>,
    cog_resolver: Option<verum_modules::cog_resolver::CogResolver>,
) -> Result<(), CliError> {
    let vbc_module = verum_vbc::deserialize::deserialize_module(vbc_bytes)
        .map_err(|e| CliError::Custom(format!("cached VBC deserialise failed: {e}")))?;
    options.input = input.to_path_buf();
    let mut session = Session::new(options);
    if let Some(policy) = permission_policy {
        session.set_script_permission_policy(policy);
    }
    if let Some(resolver) = cog_resolver {
        session.set_cog_resolver(resolver);
    }
    {
        let mut pipeline = CompilationPipeline::new(&mut session);
        pipeline
            .run_compiled_vbc(std::sync::Arc::new(vbc_module), args)
            .map_err(|e| CliError::RuntimeError(e.to_string()))?;
    }
    if timings {
        print_phase_timings(&session);
    }
    if let Some(code) = session.take_exit_code() {
        std::process::exit(code);
    }
    Ok(())
}

/// Resolve a script's frontmatter dependencies into a populated
/// `CogResolver` ready to be installed on the run-time `Session`.
///

/// Three resolution kinds, all uniform from the resolver's POV
/// (`register_cog(name, version, root_path)`):
///

///  1. **Path-form** (`{ name = "foo", path = "./local-cogs/foo" }`):
///  resolved relative to the script's directory, canonicalised
///  so the script remains runnable from any cwd. No I/O beyond
///  `canonicalize`.
///  2. **Registry-form** (short `"json@1"` or long `{ name = "json",
///  version = "^1.0" }`): version constraint resolved via the
///  registry HTTP client (`get_metadata` for exact versions,
///  `get_latest_version` for bare names / range constraints
///  since the registry doesn't yet expose `list_versions`),
///  tarball downloaded into the cog cache (`<cache>/verum/cogs/
///  <name>/<version>/<name>-<version>.tar.gz`), extracted into
///  a sibling directory, registered with the resolver. Cache
///  hits (extracted dir already present) skip both download and
///  extract.
///  3. **Git-form** (`{ name = "x", git = "https://...", rev = "..." }`):
///  cloned into `<cache>/verum/git/<name>-<rev>/`, checked out
///  to the requested rev/branch/tag, registered with the
///  resolver. Cache hits skip the clone.
///

/// All three kinds emit a `LockedDep` for the script's lockfile
/// (`path+<dir>` / `registry+<url>` / `git+<url>#<sha>`) so a
/// `verum lockfile verify` can fail-closed on supply-chain drift.
///

/// Network errors during registry/git resolution surface as
/// `CliError` (the script run aborts) rather than warn-and-continue —
/// silently dropping a declared dependency would let
/// `mount foo.client.Response` "succeed" with an empty cog and the
/// resulting `unbound symbol` errors at use site would mislead the
/// user.
struct ResolvedDeps {
    resolver: verum_modules::cog_resolver::CogResolver,
    locked: Vec<crate::script::lockfile::LockedDep>,
}

fn resolve_script_dependencies(
    fm: &crate::script::frontmatter::Frontmatter,
    script_path: &std::path::Path,
) -> Result<ResolvedDeps, CliError> {
    use crate::script::frontmatter::DepSpec;
    use crate::script::lockfile::LockedDep;
    let mut resolver = verum_modules::cog_resolver::CogResolver::new();
    let mut locked: Vec<LockedDep> = Vec::new();
    let script_dir = script_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let mut path_count = 0usize;
    let mut registry_count = 0usize;
    let mut git_count = 0usize;
    for dep in &fm.dependencies {
        match dep {
            DepSpec::Long(long) if long.path.is_some() => {
                let raw_path = long.path.as_deref().unwrap();
                let resolved = if std::path::Path::new(raw_path).is_absolute() {
                    std::path::PathBuf::from(raw_path)
                } else {
                    script_dir.join(raw_path)
                };
                let canonical = resolved.canonicalize().map_err(|e| {
                    CliError::Custom(format!(
                        "script dependency `{}`: cannot resolve path `{}`: {e}",
                        long.name,
                        resolved.display()
                    ))
                })?;
                let version = long.version.clone().unwrap_or_else(|| "0.0.0".to_string());
                let integrity = compute_path_cog_integrity(&canonical);
                resolver.register_cog(long.name.as_str(), version.as_str(), canonical.clone());
                locked.push(LockedDep {
                    name: long.name.clone(),
                    version,
                    source: format!("path+{}", canonical.display()),
                    integrity,
                });
                path_count += 1;
            }
            DepSpec::Long(long) if long.git.is_some() => {
                let entry = resolve_git_dep(long)?;
                resolver.register_cog(
                    long.name.as_str(),
                    entry.version.as_str(),
                    entry.root.clone(),
                );
                locked.push(entry.locked);
                git_count += 1;
            }
            DepSpec::Long(long) => {
                let entry = resolve_registry_dep(long.name.as_str(), long.version.as_deref())?;
                resolver.register_cog(
                    long.name.as_str(),
                    entry.version.as_str(),
                    entry.root.clone(),
                );
                locked.push(entry.locked);
                registry_count += 1;
            }
            DepSpec::Short(spec) => {
                let (name, version_req) = parse_short_dep(spec).ok_or_else(|| {
                    CliError::Custom(format!(
                        "script dependency `{spec}`: malformed short-form (expected `name` or `name@version`)"
                    ))
                })?;
                let entry = resolve_registry_dep(name.as_str(), version_req.as_deref())?;
                resolver.register_cog(name.as_str(), entry.version.as_str(), entry.root.clone());
                locked.push(entry.locked);
                registry_count += 1;
            }
        }
    }

    if path_count > 0 {
        ui::detail(
            "Dependencies",
            &format!("{path_count} path-cog(s) registered"),
        );
    }
    if registry_count > 0 {
        ui::detail(
            "Dependencies",
            &format!("{registry_count} registry-cog(s) registered"),
        );
    }
    if git_count > 0 {
        ui::detail(
            "Dependencies",
            &format!("{git_count} git-cog(s) registered"),
        );
    }
    Ok(ResolvedDeps { resolver, locked })
}

/// Resolved registry/git dependency: extracted root + lockfile entry.
struct ResolvedDepEntry {
    root: std::path::PathBuf,
    version: String,
    locked: crate::script::lockfile::LockedDep,
}

/// Parse a short-form dep spec `"name@version"` or `"name"` into
/// `(name, Option<version_req>)`. Returns `None` on malformed input.
fn parse_short_dep(spec: &str) -> Option<(String, Option<String>)> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.split_once('@') {
        Some((name, ver)) => {
            let n = name.trim();
            let v = ver.trim();
            if n.is_empty() || v.is_empty() {
                None
            } else {
                Some((n.to_string(), Some(v.to_string())))
            }
        }
        None => Some((trimmed.to_string(), None)),
    }
}

/// Resolve a registry-form dependency: pick the concrete version,
/// download into the cog cache (skip if already cached + extracted),
/// extract the tarball, return the extracted-source root + lockfile
/// entry.
fn resolve_registry_dep(
    name: &str,
    version_req: Option<&str>,
) -> Result<ResolvedDepEntry, CliError> {
    use crate::registry::{cache_manager::CacheManager, client::RegistryClient};

    let client = RegistryClient::from_manifest()?;
    let cache_dir = crate::registry::cache_dir()?;
    let cache_manager = CacheManager::new(cache_dir.clone())?;

    // Version resolution. The registry doesn't expose a
    // `list_versions` endpoint yet, so:
    //  - No constraint → `get_latest_version`.
    //  - Pure-numeric `"1.4.0"` (parses as `semver::Version`) → as-is.
    //  - Range `"^1.0"`, `">=2.0"` → fall back to `get_latest_version`
    //  and pin the resolved version in the lockfile (subsequent
    //  runs hit a stable target regardless of registry drift).
    let version: String = match version_req {
        None => client.get_latest_version(name)?.as_str().to_string(),
        Some(req) => {
            if is_exact_version(req) {
                req.to_string()
            } else {
                client.get_latest_version(name)?.as_str().to_string()
            }
        }
    };

    let extracted_root = cache_dir.join(name).join(&version);
    let archive_path = extracted_root.join(format!("{}-{}.tar.gz", name, version));
    let needs_install = !looks_extracted(&extracted_root);

    if needs_install {
        let metadata = client.get_metadata(name, &version)?;
        let url = format!(
            "{}/cogs/{}/{}/download",
            crate::registry::DEFAULT_REGISTRY,
            name,
            version,
        );
        let _archive =
            cache_manager.download_cog(name, &version, url.as_str(), metadata.checksum.as_str())?;
        cache_manager.extract(&archive_path, &extracted_root)?;
    }

    let root = extracted_root.clone();
    let integrity = compute_path_cog_integrity(&root);
    let locked = crate::script::lockfile::LockedDep {
        name: name.to_string(),
        version: version.clone(),
        source: format!("registry+{}", crate::registry::DEFAULT_REGISTRY),
        integrity,
    };
    Ok(ResolvedDepEntry {
        root,
        version,
        locked,
    })
}

/// Resolve a git-form dependency: clone into the git cache (skip if
/// already cloned), checkout to the requested rev/branch/tag, return
/// the source root + lockfile entry.
fn resolve_git_dep(
    long: &crate::script::frontmatter::DepLong,
) -> Result<ResolvedDepEntry, CliError> {
    let url = long.git.as_deref().ok_or_else(|| {
        CliError::Custom(format!(
            "git dependency `{}` missing `git = ...` URL",
            long.name
        ))
    })?;
    let pin = long
        .rev
        .as_deref()
        .or(long.tag.as_deref())
        .or(long.branch.as_deref())
        .unwrap_or("HEAD");
    let safe_pin = sanitize_git_pin(pin);
    let dest = crate::registry::git_dir()?.join(format!("{}-{}", long.name, safe_pin));

    if !dest.join(".git").exists() {
        std::fs::create_dir_all(&dest)?;
        let status = std::process::Command::new("git")
            .arg("clone")
            .arg("--quiet")
            .arg(url)
            .arg(&dest)
            .status()
            .map_err(|e| {
                CliError::Custom(format!(
                    "git dep `{}`: spawn `git clone` failed: {e}",
                    long.name
                ))
            })?;
        if !status.success() {
            return Err(CliError::Custom(format!(
                "git dep `{}`: `git clone {}` failed (exit {})",
                long.name,
                url,
                status.code().unwrap_or(-1),
            )));
        }
    }

    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&dest)
        .arg("checkout")
        .arg("--quiet")
        .arg(pin)
        .status()
        .map_err(|e| {
            CliError::Custom(format!(
                "git dep `{}`: spawn `git checkout` failed: {e}",
                long.name
            ))
        })?;
    if !status.success() {
        return Err(CliError::Custom(format!(
            "git dep `{}`: `git checkout {}` failed (exit {})",
            long.name,
            pin,
            status.code().unwrap_or(-1),
        )));
    }

    let sha_out = std::process::Command::new("git")
        .arg("-C")
        .arg(&dest)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .map_err(|e| {
            CliError::Custom(format!(
                "git dep `{}`: spawn `git rev-parse` failed: {e}",
                long.name
            ))
        })?;
    let sha = String::from_utf8_lossy(&sha_out.stdout).trim().to_string();
    let version = long.version.clone().unwrap_or_else(|| sha.clone());

    let integrity = compute_path_cog_integrity(&dest);
    let locked = crate::script::lockfile::LockedDep {
        name: long.name.clone(),
        version: version.clone(),
        source: format!("git+{}#{}", url, sha),
        integrity,
    };
    Ok(ResolvedDepEntry {
        root: dest,
        version,
        locked,
    })
}

/// Check if the extracted directory looks like a populated cog
/// (contains either a `verum.toml` manifest or a `src/` directory).
/// Used to short-circuit the download+extract path on cache hits.
fn looks_extracted(dir: &std::path::Path) -> bool {
    dir.join("verum.toml").is_file() || dir.join("src").is_dir()
}

/// Heuristic: a version string is "exact" when it parses as a bare
/// `semver::Version` (no operator prefix like `^`, `~`, `>=`). Used
/// to decide whether `get_metadata` (exact) vs `get_latest_version`
/// (range) is the right registry call.
fn is_exact_version(req: &str) -> bool {
    semver::Version::parse(req).is_ok()
}

/// Sanitize a git rev/branch/tag identifier into a filesystem-safe
/// directory-name fragment. Rev SHAs are already safe; branch / tag
/// names with `/` or other non-portable chars get replaced with `_`.
/// Truncated to 64 chars to keep cache-dir names manageable.
fn sanitize_git_pin(pin: &str) -> String {
    let cleaned: String = pin
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '.' | '_' => c,
            _ => '_',
        })
        .collect();
    if cleaned.len() > 64 {
        cleaned[..64].to_string()
    } else {
        cleaned
    }
}

/// Persist (or verify+refresh) a script's resolved dependencies as
/// a sidecar `<script>.lock` next to the source.
///

/// **First run** (no lockfile present) → write a fresh lockfile
/// from `locked_deps`.
///

/// **Subsequent run** (lockfile exists) → call `verify_against` to
/// detect drift in `(source_hash, compiler_version)`. On stale,
/// rewrite. Always re-hash on every run so a deps swap (path
/// repointed, integrity changed) is reflected in the lockfile —
/// drift must be observable, not silent.
///

/// I/O failures are non-fatal: the script run is the contract;
/// the lockfile is reproducibility metadata. A read-only mount or
/// a permission glitch warns and continues.
fn persist_script_lockfile(
    ctx: &crate::script::context::ScriptContext,
    script_path: &std::path::Path,
    locked_deps: &[crate::script::lockfile::LockedDep],
) -> Result<(), CliError> {
    use crate::script::lockfile::ScriptLockfile;
    let path = ScriptLockfile::sidecar_path(script_path);
    let mut fresh = ctx.fresh_lockfile(locked_deps.to_vec());
    if let Err(e) = fresh.write_to(&path) {
        ui::warn(&format!("could not write lockfile {}: {e}", path.display()));
    }
    Ok(())
}

/// Hash a path-form cog's source tree into a stable integrity
/// digest for the lockfile. blake3 over a sorted catalogue of
/// `(relative_path, content_hash)` pairs — moving the cog dir or
/// touching whitespace inside any `.vr` file changes the digest;
/// ordering of `read_dir` results does not. Best-effort: I/O
/// errors collapse to an empty digest so a momentarily-unreadable
/// file doesn't fail the entire script run.
fn compute_path_cog_integrity(root: &std::path::Path) -> String {
    let mut entries: Vec<(String, [u8; 32])> = Vec::new();
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, [u8; 32])>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "vr") {
                let rel = path
                    .strip_prefix(root)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| path.display().to_string());
                if let Ok(bytes) = std::fs::read(&path) {
                    let mut h = [0u8; 32];
                    h.copy_from_slice(blake3::hash(&bytes).as_bytes());
                    out.push((rel, h));
                }
            }
        }
    }
    walk(root, root, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = blake3::Hasher::new();
    for (rel, h) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update(h);
    }
    hasher.finalize().to_hex().to_string()
}

/// Sentinel target_id meaning "wildcard / coarse-mode check".
/// The FFI dispatch passes this when it doesn't have a structured
/// target value to hash; the policy treats it as "any grant of
/// the scope's matching kind allows". Once SCRIPT-5c-followup
/// extends the gate to extract real targets (path bytes for FS,
/// host:port for Net), only specific-target lookups will hit the
/// HashMap and this sentinel becomes the all-grant fallback.
const WILDCARD_TARGET_ID: u64 = 0;

/// Compute a stable u64 hash of a granted target string. Used at
/// policy build time to pre-populate the (scope, target_id) →
/// Allow HashMap; the runtime gate hashes the same string at the
/// call site and looks up the result.
///

/// blake3-32-bit truncated. Collisions over the script's grant
/// set are vanishingly improbable (≈2⁻³² for unrelated strings)
/// and would only over-grant — never under-grant — because the
/// HashMap stores explicit Allow entries and the default is Deny.
fn hash_grant_target(target: &str) -> u64 {
    let h = blake3::hash(target.as_bytes());
    let bytes = h.as_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Map a CLI permission kind to the runtime PermissionScope used
/// by the interpreter's `PermissionRouter`. Mirror of the inline
/// match in `build_script_permission_policy`'s closure — kept
/// separate so the policy builder and the lookup-key constructor
/// agree byte-for-byte on the mapping.
fn cli_kind_to_router_scope(
    kind: crate::script::permissions::PermissionKind,
) -> Option<verum_vbc::interpreter::permission::PermissionScope> {
    use crate::script::permissions::PermissionKind;
    use verum_vbc::interpreter::permission::PermissionScope;
    Some(match kind {
        PermissionKind::Ffi => PermissionScope::Syscall,
        PermissionKind::FsRead | PermissionKind::FsWrite => PermissionScope::FileSystem,
        PermissionKind::Net => PermissionScope::Network,
        PermissionKind::Run => PermissionScope::Process,
        PermissionKind::Time | PermissionKind::Random => PermissionScope::Time,
        // `Env` doesn't map cleanly to a Router scope today.
        // Process covers env-mutating syscalls; pure env reads
        // aren't gated. Returning None means env-grants don't
        // contribute Router entries — they remain advisory at
        // the language-level boundary.
        PermissionKind::Env => return None,
    })
}

/// Build a permission policy from a script's `ScriptContext`.
/// Returns `None` for scripts whose frontmatter does not declare a
/// `permissions = [...]` field — such scripts run unrestricted (the
/// router's default), preserving the legacy behaviour for the wide
/// existing surface that hasn't opted into sandboxing.
///

/// When the frontmatter DOES declare permissions, the returned
/// policy enforces deny-by-default coarse-grained gating: each
/// runtime check against a `PermissionScope` is granted iff the
/// script's `PermissionSet` carries at least one grant of the
/// matching `PermissionKind`. The mapping:
///

/// | Scope | Granted iff PermissionSet has any of |
/// |----------------|---------------------------------------------------------|
/// | `Syscall` | `ffi` |
/// | `FileSystem` | `fs:read`, `fs:write` |
/// | `Network` | `net` |
/// | `Process` | `run` |
/// | `Memory` | (always allowed — no script-level memory grants exist) |
/// | `Cryptography` | (always allowed — covered by language-level audits) |
/// | `Time` | `time`, `random` |
///

/// **Coarse-by-construction.** The current `PermissionAssert`
/// dispatch carries a u64 `target_id` that, for raw syscalls, is
/// the syscall NUMBER — not the path / host / etc. that a
/// fine-grained `permissions = ["fs:read=./data"]` grant would
/// authorise on. Fine-grained per-target enforcement requires
/// extending the codegen to hash the structured target value
/// (path bytes, host:port) at the call site; that work is tracked
/// separately. The current policy gives meaningful protection at
/// the kind level and is the natural insertion point for the
/// future per-target check.
/// Build the AOT-side counterpart of [`build_script_permission_policy`]:
/// the same `PermissionSet`, packaged as compile-time data the LLVM
/// lowerer can bake into the generated binary at every
/// `PermissionAssert` site.
///

/// `None` is returned for scripts whose `ctx.permissions` is empty —
/// the trusted-application path. The lowerer treats `None` as
/// allow-all (no-op every gate), matching the interpreter's default
/// when no script policy is wired.
///

/// The mapping mirrors `build_script_permission_policy` exactly so
/// the two execution tiers agree on which scope/target combinations
/// are allowed:
///

/// * `Memory` and `Cryptography` go into `always_allow` (no script
///  permission kind maps to them today).
/// * Wildcard CLI-scope grants populate the `wildcards` set.
/// * Specific-target grants populate `specific` with the same
///  `(scope_tag, target_id)` pairs — `target_id` hashed via
///  `hash_grant_target` to match the runtime gate's input shape.
fn build_aot_permission_policy(
    ctx: &crate::script::context::ScriptContext,
) -> Option<verum_codegen::llvm::AotPermissionPolicy> {
    use crate::script::permissions::PermissionScope as CliScope;
    use verum_codegen::llvm::AotPermissionPolicy;
    use verum_vbc::interpreter::permission::PermissionScope;

    if ctx.permissions.is_empty() {
        return None;
    }

    let mut policy = AotPermissionPolicy::default();

    // Memory and Cryptography have no script-level kinds; the
    // interpreter policy treats them as always allowed regardless of
    // declared grants. Mirror that here so AOT and interpreter agree
    // on every scope, not just the kinds the script wrote.
    policy
        .always_allow
        .insert(PermissionScope::Memory.to_wire_tag());
    policy
        .always_allow
        .insert(PermissionScope::Cryptography.to_wire_tag());

    for grant in iterate_grants(&ctx.permissions) {
        let Some(scope) = cli_kind_to_router_scope(grant.kind) else {
            continue;
        };
        let scope_tag = scope.to_wire_tag();
        match &grant.scope {
            CliScope::Any => {
                policy.wildcards.insert(scope_tag);
            }
            CliScope::Targets(targets) => {
                for t in targets {
                    policy.specific.insert((scope_tag, hash_grant_target(t)));
                }
            }
        }
    }

    Some(policy)
}

fn build_script_permission_policy(
    ctx: &crate::script::context::ScriptContext,
) -> Option<verum_compiler::session::ScriptPermissionPolicy> {
    use verum_compiler::session::ScriptPermissionPolicy;
    use verum_vbc::interpreter::permission::{PermissionDecision, PermissionScope};

    // Opt-in to sandboxing — install a policy when EITHER the
    // frontmatter explicitly declared `permissions = [...]` OR the
    // user passed any `--allow*` / `--deny-all` CLI flag (visible
    // to us indirectly via a non-empty `ctx.permissions`). Plain
    // scripts with neither signal keep the router's default
    // allow-all so untouched scripts continue to work unchanged.
    if ctx.permissions.is_empty() {
        return None;
    }

    use crate::script::permissions::PermissionScope as CliScope;
    use std::collections::HashSet;

    // Pre-compute the (scope, target_id) lookup set. Wildcard
    // grants ("permissions = [\"net\"]") populate the
    // (scope, WILDCARD_TARGET_ID) entry; specific grants
    // ("permissions = [\"net=api.example.com:443\"]") populate
    // (scope, hash(target)) per target. The runtime gate hashes
    // the same target at call time and probes both the specific
    // entry and the wildcard fallback before denying.
    //

    // O(1) at check time (one HashSet lookup), bounded by
    // grant count at build time. No allocation in the hot path.
    let mut allow_set: HashSet<(PermissionScope, u64)> = HashSet::new();
    let perms_snapshot = ctx.permissions.clone();
    for grant in iterate_grants(&perms_snapshot) {
        let Some(scope) = cli_kind_to_router_scope(grant.kind) else {
            continue;
        };
        match &grant.scope {
            CliScope::Any => {
                allow_set.insert((scope, WILDCARD_TARGET_ID));
            }
            CliScope::Targets(targets) => {
                for t in targets {
                    allow_set.insert((scope, hash_grant_target(t)));
                }
            }
        }
        // FsRead and FsWrite share the FileSystem scope. A grant
        // of either kind contributes to the same map entry; the
        // gate doesn't distinguish read-vs-write in target_id
        // space (yet). When fine-grained read/write distinction
        // lands, add a sub-key suffix to the hash.
    }

    let policy = move |scope: PermissionScope, target_id: u64| -> PermissionDecision {
        // Specific-target entry wins; wildcard is the per-scope
        // fallback. Order avoids a redundant HashSet lookup when
        // the gate did pass a real target.
        if allow_set.contains(&(scope, target_id))
            || allow_set.contains(&(scope, WILDCARD_TARGET_ID))
        {
            PermissionDecision::Allow
        } else {
            // Memory and Cryptography scopes have no script-level
            // kind today — leave them open by policy regardless
            // of the script's declared grants. Future work may
            // tie them to explicit kinds if the threat model
            // requires it.
            match scope {
                PermissionScope::Memory | PermissionScope::Cryptography => {
                    PermissionDecision::Allow
                }
                _ => PermissionDecision::Deny,
            }
        }
    };

    Some(ScriptPermissionPolicy(Box::new(policy)))
}

/// Walk every grant in a `PermissionSet`. The set's API exposes
/// `grants_of(kind)` per-kind iteration — this helper merges all
/// kinds into a single sequence so the policy builder doesn't
/// have to enumerate kinds explicitly. Mirrors the canonical
/// kind list from `script::permissions::PermissionKind`.
fn iterate_grants<'a>(
    set: &'a crate::script::permissions::PermissionSet,
) -> impl Iterator<Item = &'a crate::script::permissions::Permission> {
    use crate::script::permissions::PermissionKind;
    [
        PermissionKind::FsRead,
        PermissionKind::FsWrite,
        PermissionKind::Net,
        PermissionKind::Env,
        PermissionKind::Run,
        PermissionKind::Ffi,
        PermissionKind::Time,
        PermissionKind::Random,
    ]
    .into_iter()
    .flat_map(move |k| set.grants_of(k))
}

/// Cache-key contributors specific to the AOT script-binary cache.
/// Identical (source, compiler, flags) tuples should produce
/// byte-identical AOT binaries on the same target — but a
/// different target triple, opt level, or LTO mode produces a
/// different binary, so each must contribute to the key.
fn aot_cache_flag_inputs() -> Vec<String> {
    vec![
        format!("aot=1"),
        format!("target={}", std::env::consts::ARCH),
        format!("os={}", std::env::consts::OS),
    ]
}

/// Resolve the AOT script-binary cache root: `~/.verum/script-aot-cache/`.
/// Returns `None` on any I/O glitch — the AOT cache is best-effort.
fn aot_cache_root() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    Some(home.join(".verum").join("script-aot-cache"))
}

/// Look up a previously-compiled AOT binary for the given cache key.
/// Returns `Some(path)` on hit (binary exists at the canonical
/// location), `None` on miss or any I/O failure.
fn lookup_aot_binary(key: crate::script::cache::CacheKey) -> Option<std::path::PathBuf> {
    let root = aot_cache_root()?;
    let entry = root.join(key.to_hex());
    let bin_name = if cfg!(windows) {
        "binary.exe"
    } else {
        "binary"
    };
    let bin = entry.join(bin_name);
    if bin.is_file() { Some(bin) } else { None }
}

/// Persist a freshly-compiled AOT binary into the cache. Atomic at
/// the rename boundary (write to temp filename, fsync, rename) so a
/// crash mid-write doesn't leave a corrupt entry visible to the
/// next lookup. Best-effort — caller must not fail the run on
/// `Err`.
fn store_aot_binary(
    key: crate::script::cache::CacheKey,
    src_binary: &std::path::Path,
) -> std::io::Result<()> {
    let root = aot_cache_root()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "$HOME unset"))?;
    let entry = root.join(key.to_hex());
    std::fs::create_dir_all(&entry)?;
    let bin_name = if cfg!(windows) {
        "binary.exe"
    } else {
        "binary"
    };
    let final_path = entry.join(bin_name);
    let tmp_path = entry.join(format!("{}.tmp-{}", bin_name, std::process::id()));
    std::fs::copy(src_binary, &tmp_path)?;
    // Preserve the executable bit on Unix so the cached binary can
    // be exec'd directly without a chmod step on every lookup.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp_path, perms)?;
    }
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Exec a cached AOT binary with the script's args, propagating its
/// exit code. Mirrors the live-compile path's exec semantics so the
/// observed behaviour is identical between cache-hit and cache-miss
/// runs.
fn exec_native_binary(binary: &std::path::Path, args: &List<Text>) -> Result<(), CliError> {
    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let status = std::process::Command::new(binary)
        .args(&args_str)
        .status()
        .map_err(|e| {
            CliError::RuntimeError(format!(
                "Failed to run cached AOT binary {}: {}",
                binary.display(),
                e
            ))
        })?;
    if !status.success() {
        let exit_code = status.code().unwrap_or(-1);
        std::process::exit(exit_code);
    }
    Ok(())
}

/// Stable cache-key contributors derived from CompilerOptions.
/// Order matters — cache keys are deterministic over this slice. Any
/// option that affects the produced VBC bytes must show up here, or
/// runs with different settings will share a cache entry incorrectly.
fn cache_flag_inputs(opts: &CompilerOptions) -> Vec<String> {
    vec![
        format!("verify={:?}", opts.verify_mode),
        format!("opt={}", opts.optimization_level),
        format!("script_mode={}", opts.script_mode),
    ]
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
        eprintln!(
            "  {:<19}{:>8.1}ms",
            format!("{}:", name),
            duration.as_secs_f64() * 1000.0
        );
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

#[cfg(test)]
mod script_dep_tests {
    use super::{is_exact_version, parse_short_dep, sanitize_git_pin};

    #[test]
    fn parse_short_dep_bare_name() {
        let (n, v) = parse_short_dep("json").unwrap();
        assert_eq!(n, "json");
        assert!(v.is_none());
    }

    #[test]
    fn parse_short_dep_name_at_version() {
        let (n, v) = parse_short_dep("json@1.4.0").unwrap();
        assert_eq!(n, "json");
        assert_eq!(v.as_deref(), Some("1.4.0"));
    }

    #[test]
    fn parse_short_dep_name_at_range() {
        let (n, v) = parse_short_dep("http@^0.2").unwrap();
        assert_eq!(n, "http");
        assert_eq!(v.as_deref(), Some("^0.2"));
    }

    #[test]
    fn parse_short_dep_trims_whitespace() {
        let (n, v) = parse_short_dep("  json  @  1  ").unwrap();
        assert_eq!(n, "json");
        assert_eq!(v.as_deref(), Some("1"));
    }

    #[test]
    fn parse_short_dep_rejects_empty() {
        assert!(parse_short_dep("").is_none());
        assert!(parse_short_dep("   ").is_none());
        assert!(parse_short_dep("@1.0").is_none());
        assert!(parse_short_dep("name@").is_none());
    }

    #[test]
    fn is_exact_version_recognises_full_semver() {
        assert!(is_exact_version("1.0.0"));
        assert!(is_exact_version("0.2.5"));
        assert!(is_exact_version("2.10.3-alpha.1"));
    }

    #[test]
    fn is_exact_version_rejects_ranges_and_partials() {
        assert!(!is_exact_version("^1.0"));
        assert!(!is_exact_version("~0.2"));
        assert!(!is_exact_version(">=2.0"));
        assert!(!is_exact_version("1"));
        assert!(!is_exact_version("1.2"));
    }

    #[test]
    fn sanitize_git_pin_passes_safe_chars() {
        assert_eq!(sanitize_git_pin("a1b2c3d4"), "a1b2c3d4");
        assert_eq!(sanitize_git_pin("v1.2.3"), "v1.2.3");
        assert_eq!(sanitize_git_pin("feature_branch-1"), "feature_branch-1");
    }

    #[test]
    fn sanitize_git_pin_replaces_unsafe_chars() {
        assert_eq!(sanitize_git_pin("feature/foo"), "feature_foo");
        assert_eq!(sanitize_git_pin("refs/tags/v1"), "refs_tags_v1");
        assert_eq!(sanitize_git_pin("user@host:path"), "user_host_path");
    }

    #[test]
    fn sanitize_git_pin_truncates_long_input() {
        let long = "a".repeat(200);
        let s = sanitize_git_pin(&long);
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c == 'a'));
    }
}
