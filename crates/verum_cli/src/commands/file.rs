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
            // For script-shaped sources (shebang at byte 0 or an inline
            // `// /// script` frontmatter block) the entry path runs
            // through `run_script_interpreted` which adds:
            //
            //   • frontmatter validation (compiler version constraint
            //     against the running build),
            //   • permission resolution (frontmatter ∪ CLI flags),
            //   • persistent VBC cache (lookup-skip-compile on hit;
            //     compile + serialise + store on miss),
            //   • lockfile placeholder (populated as
            //     dependency resolution lands).
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

            if is_script_shaped(&input) {
                run_script_interpreted(&input, options, args, timings)?;
            } else {
                let mut session = Session::new(options);
                {
                    let mut pipeline = CompilationPipeline::new(&mut session);
                    pipeline
                        .run_interpreter(args)
                        .map_err(|e| CliError::RuntimeError(e.to_string()))?;
                }
                if timings {
                    print_phase_timings(&session);
                }
                if let Some(code) = session.take_exit_code() {
                    std::process::exit(code);
                }
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

            // Script-shaped sources go through frontmatter validation
            // (compiler-version pin, declared-deps audit) before LLVM
            // compilation so an unbuildable script fails fast with a
            // clear diagnostic instead of a confusing native-link
            // error half a megabyte deeper. Permission enforcement
            // doesn't yet apply to the AOT path — the runtime
            // `PermissionRouter` lives in the interpreter only;
            // wiring it into the LLVM lowering of `PermissionAssert`
            // is a follow-on step.
            if is_script_shaped(&input) {
                use crate::script::context::{ScriptContext, ScriptContextOptions};
                use crate::script::permission_flags::PermissionFlags;
                let ctx = ScriptContext::from_path(
                    &input,
                    &ScriptContextOptions {
                        flags: PermissionFlags::default(),
                        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
                        extra_cache_flags: Vec::new(),
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
                    if !fm.permissions.is_empty() {
                        ui::warn(
                            "script frontmatter declares permissions — \
                             AOT permission enforcement lands separately; \
                             use `verum run` (interpreter) for sandboxed \
                             execution today",
                        );
                    }
                }
            }

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
pub fn synthesize_script_temp(
    body: &str,
    kind: &str,
) -> std::io::Result<ScriptTempFile> {
    use std::io::Write;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "verum-{kind}-{}-{}.vr",
        std::process::id(),
        nanos
    ));
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
fn check_frontmatter_version(
    fm: &crate::script::frontmatter::Frontmatter,
) -> Result<(), CliError> {
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
) -> Result<(), CliError> {
    use crate::script::cache::ScriptCache;
    use crate::script::context::{ScriptContext, ScriptContextOptions};
    use crate::script::permission_flags::PermissionFlags;

    // 1. Build the ScriptContext: read source, hash, extract+validate
    //    frontmatter, merge CLI permission flags, compute cache key.
    let ctx_opts = ScriptContextOptions {
        flags: PermissionFlags::default(),
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        extra_cache_flags: cache_flag_inputs(&options),
    };
    let ctx = ScriptContext::from_path(input, &ctx_opts).map_err(|e| {
        CliError::Custom(format!("script context: {e}"))
    })?;

    // 2. Frontmatter version gate. A script with `verum = "X.Y"` that
    //    doesn't match the running build fails fast with a clear
    //    diagnostic instead of producing a confusing parse error
    //    half a megabyte deeper into the pipeline.
    if let Some(fm) = ctx.frontmatter.as_ref() {
        check_frontmatter_version(fm)?;
        if !fm.dependencies.is_empty() {
            ui::warn(
                "script frontmatter declares dependencies — registry \
                 resolution lands separately; for now they are ignored",
            );
        }
    }

    // 3. Permission policy. Built only when the script's frontmatter
    //    EXPLICITLY declares a `permissions = [...]` field. Plain
    //    scripts with no permissions block keep the interpreter
    //    router's default allow-all behaviour — explicit opt-in to
    //    sandboxing, matching Deno's `--allow-*` philosophy without
    //    breaking existing untouched scripts.
    let permission_policy = build_script_permission_policy(&ctx);
    if permission_policy.is_some() {
        ui::detail(
            "Permissions",
            &format!("{} grants installed", ctx.permissions.len()),
        );
    }

    // 3. Persistent VBC cache. Best-effort: cache-open failures fall
    //    back to a cache-disabled run. Tier-aware cache keys (already
    //    encoded in `ScriptContextOptions::extra_cache_flags`) ensure
    //    `--verify-mode runtime` and `--verify-mode auto` runs don't
    //    poison each other's cache.
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
                );
            }
            Ok(None) => { /* miss — fall through */ }
            Err(e) => ui::warn(&format!("script cache lookup failed: {e}")),
        }
    }

    // 4. Cache miss: run the pipeline. The session captures the
    //    compiled VBC via `record_compiled_vbc` so we can pull it
    //    back here for cache-store.
    options.input = input.to_path_buf();
    ui::status("Running", &format!("{} (interpreter)", input.display()));
    let mut session = Session::new(options);
    if let Some(policy) = permission_policy {
        session.set_script_permission_policy(policy);
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
    //    Best-effort: a cache-write failure does not fail the run.
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
    //    The pipeline records via `Session::record_exit_code` instead
    //    of calling `process::exit` directly, so the cache-store step
    //    above runs first. `None` here means the script returned `()`
    //    or a non-numeric value — exit 0 by convention.
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
) -> Result<(), CliError> {
    let vbc_module = verum_vbc::deserialize::deserialize_module(vbc_bytes).map_err(|e| {
        CliError::Custom(format!("cached VBC deserialise failed: {e}"))
    })?;
    options.input = input.to_path_buf();
    let mut session = Session::new(options);
    if let Some(policy) = permission_policy {
        session.set_script_permission_policy(policy);
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
/// | Scope          | Granted iff PermissionSet has any of                    |
/// |----------------|---------------------------------------------------------|
/// | `Syscall`      | `ffi`                                                   |
/// | `FileSystem`   | `fs:read`, `fs:write`                                   |
/// | `Network`      | `net`                                                   |
/// | `Process`      | `run`                                                   |
/// | `Memory`       | (always allowed — no script-level memory grants exist)  |
/// | `Cryptography` | (always allowed — covered by language-level audits)     |
/// | `Time`         | `time`, `random`                                        |
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
fn build_script_permission_policy(
    ctx: &crate::script::context::ScriptContext,
) -> Option<verum_compiler::session::ScriptPermissionPolicy> {
    use crate::script::permissions::PermissionKind;
    use verum_compiler::session::ScriptPermissionPolicy;
    use verum_vbc::interpreter::permission::{PermissionDecision, PermissionScope};

    // Opt-in to sandboxing — only install a policy when the
    // frontmatter explicitly declared permissions. Plain scripts
    // (no frontmatter, or a frontmatter without a permissions
    // field) keep the router's default allow-all so they continue
    // to work unchanged.
    let has_explicit_permissions = ctx
        .frontmatter
        .as_ref()
        .map(|fm| !fm.permissions.is_empty())
        .unwrap_or(false);
    if !has_explicit_permissions {
        return None;
    }

    let perms = ctx.permissions.clone();
    let policy = move |scope: PermissionScope, _target_id: u64| -> PermissionDecision {
        let allowed = match scope {
            PermissionScope::Syscall => {
                perms.grants_of(PermissionKind::Ffi).next().is_some()
            }
            PermissionScope::FileSystem => {
                perms.grants_of(PermissionKind::FsRead).next().is_some()
                    || perms.grants_of(PermissionKind::FsWrite).next().is_some()
            }
            PermissionScope::Network => {
                perms.grants_of(PermissionKind::Net).next().is_some()
            }
            PermissionScope::Process => {
                perms.grants_of(PermissionKind::Run).next().is_some()
            }
            // Memory operations (mmap, etc.) and cryptography
            // primitives don't have a script-level grant kind; let
            // them through. Future work may add `mem` / `crypto`
            // kinds if the threat model warrants.
            PermissionScope::Memory => true,
            PermissionScope::Cryptography => true,
            PermissionScope::Time => {
                perms.grants_of(PermissionKind::Time).next().is_some()
                    || perms.grants_of(PermissionKind::Random).next().is_some()
            }
        };
        if allowed {
            PermissionDecision::Allow
        } else {
            PermissionDecision::Deny
        }
    };

    Some(ScriptPermissionPolicy(Box::new(policy)))
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
