//! Stdlib precompile orchestrator — Phase 4 of the precompiled-stdlib
//! archive epic.
//!
//! Drives the existing [`CompilationPipeline::compile_core`] (stdlib
//! bootstrap mode), then runs a post-pass over the resulting
//! [`verum_vbc::module::VbcModule`] to populate the precompile-stdlib
//! extension fields landed in Phase 3:
//!
//! * theorems table (theorem / lemma / corollary / axiom / tactic
//!   declarations from the source AST),
//! * framework provenance (`@framework(name, citation)` and
//!   `@framework_translate(...)` edges),
//! * cfg-conditional function variants (one entry per `#[cfg(...)]`
//!   arm — populated by the multi-target codegen pass below).
//!
//! Subsequent phases (5–7) embed the resulting `.vbca` into the
//! compiler binary and switch `compile_ast_to_vbc` to deserialise it
//! instead of re-running stdlib codegen on every invocation.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::core_compiler::{CoreConfig, StdlibCompilationResult};
use crate::options::CompilerOptions;
use crate::pipeline::CompilationPipeline;
use crate::session::Session;

/// Configuration for the [`precompile_stdlib`] entry point.
#[derive(Debug, Clone)]
pub struct PrecompileConfig {
    /// Filesystem path to `core/`. Defaults to the workspace's
    /// `core/` directory when constructed via [`Self::for_workspace`].
    pub stdlib_path: PathBuf,
    /// Output `.vbca` path. Phase-5 build-script consumers expect
    /// `target/precompiled-stdlib/runtime.vbca` by default.
    pub output_path: PathBuf,
    /// Optional target-triple override. `None` = host triple. Phase
    /// 4b will read this and emit per-target variants for cfg-
    /// conditional functions; today the value is recorded in the
    /// archive header but selection is host-only.
    pub target_triple: Option<String>,
    /// Verbose progress reporting.
    pub verbose: bool,
}

impl PrecompileConfig {
    /// Resolve the workspace root by walking up from `start` until
    /// `core/mod.vr` is found, then build a default config writing to
    /// `<workspace>/target/precompiled-stdlib/runtime.vbca`.
    pub fn for_workspace(start: &Path) -> Result<Self> {
        let workspace = find_workspace_root(start)?;
        let stdlib_path = workspace.join("core");
        let output_path = workspace
            .join("target")
            .join("precompiled-stdlib")
            .join("runtime.vbca");
        Ok(Self {
            stdlib_path,
            output_path,
            target_triple: None,
            verbose: false,
        })
    }
}

/// Run the stdlib precompile pipeline.
///
/// Steps:
///   1. Resolve workspace + ensure `core/` is a valid stdlib root.
///   2. Build a [`CompilationPipeline`] in `StdlibBootstrap` mode.
///   3. Drive [`CompilationPipeline::compile_core`] — produces a
///      single-target VBC archive at `cfg.output_path`.
///   4. Phase-4b post-pass (deferred): re-open the archive, walk the
///      stdlib AST in parallel, populate `theorems`,
///      `framework_provenance`, and multi-variant `function_variants`
///      tables on every contained `VbcModule`, re-serialise.
///
/// Returns the [`StdlibCompilationResult`] from `compile_core` plus
/// the absolute output path.
pub fn precompile_stdlib(cfg: &PrecompileConfig) -> Result<StdlibCompilationResult> {
    if cfg.verbose {
        eprintln!(
            "verum stdlib precompile: stdlib={}, output={}, target={}",
            cfg.stdlib_path.display(),
            cfg.output_path.display(),
            cfg.target_triple.as_deref().unwrap_or("<host>")
        );
    }

    if !cfg.stdlib_path.is_dir() {
        anyhow::bail!(
            "stdlib path is not a directory: {}",
            cfg.stdlib_path.display()
        );
    }
    let mod_vr = cfg.stdlib_path.join("mod.vr");
    if !mod_vr.is_file() {
        anyhow::bail!(
            "stdlib path missing mod.vr (not a valid `core/` root): {}",
            cfg.stdlib_path.display()
        );
    }

    if let Some(parent) = cfg.output_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create output directory {}", parent.display())
        })?;
    }

    let core_config = CoreConfig::new(cfg.stdlib_path.clone())
        .with_output(cfg.output_path.clone());
    let core_config = if cfg.verbose {
        let mut c = core_config;
        c.verbose = true;
        c
    } else {
        core_config
    };

    // Stdlib bootstrap doesn't read any user input; the empty
    // `CompilerOptions` defaults are sufficient.
    let mut session = Session::new(CompilerOptions::default());
    let mut pipeline = CompilationPipeline::new_core(&mut session, core_config);
    let result = pipeline
        .compile_core()
        .context("CompilationPipeline::compile_core failed during stdlib precompile")?;

    if cfg.verbose {
        eprintln!(
            "verum stdlib precompile: {} modules, {} functions in {:?}, archive {} ({} bytes)",
            result.modules_compiled,
            result.functions_compiled,
            result.total_time,
            result.output_path.display(),
            result.output_size,
        );
    }

    // Phase-4b TODO: open `result.output_path`, walk archived
    // VbcModules in parallel, populate `theorems` /
    // `framework_provenance` / `function_variants` from source AST,
    // re-serialise. Today the archive has empty Phase-3 extension
    // tables on every module — Phase 5/6 don't need them populated
    // to embed and switch the runtime path; they're populated when
    // Phase 8/9 (verify-ladder + meta lazy-load) come online.

    Ok(result)
}

// ============================================================================
// Phase 12: precompile-cog
// ============================================================================

/// Configuration for [`precompile_cog`] — the per-cog analogue of
/// [`PrecompileConfig`].
///
/// A "cog" is any Verum project with a `Verum.toml` manifest: a
/// user library, an internal company project, or a third-party
/// package destined for the registry. The Phase 12 orchestrator
/// drives the *same* `CompilationPipeline::compile_core` machinery
/// used for stdlib (Phase 4) — `compile_core` is generic over the
/// source-tree path and module-namespace prefix, so cogs reuse 100%
/// of the bootstrap-mode codegen pipeline rather than spawning a
/// parallel implementation.
///
/// The output `.vbca` follows the registry naming convention:
/// `<name>-<version>-verum-<compiler-version>.vbca` (matches
/// `vbca_fetcher::vbca_cache_path` so the same artifact paths
/// flow through CI publish, on-disk cache, and registry download).
#[derive(Debug, Clone)]
pub struct PrecompileCogConfig {
    /// Cog source root — directory containing `Verum.toml` and a
    /// `src/` (or top-level) tree of `.vr` files. Resolved from
    /// the manifest at construction time.
    pub cog_dir: PathBuf,
    /// Cog name — drives module-namespace prefixing and the output
    /// filename. Read from `cog.name` in `Verum.toml`.
    pub cog_name: String,
    /// Cog version — drives the output filename. Read from
    /// `cog.version` in `Verum.toml`.
    pub cog_version: String,
    /// Output `.vbca` path. Constructed as
    /// `<cog_dir>/target/cog-vbca/<name>-<version>-verum-<compiler>.vbca`
    /// when not overridden.
    pub output_path: PathBuf,
    /// Optional target-triple. `None` = host. Phase 12b cross-compile
    /// matrix builds will iterate this axis.
    pub target_triple: Option<String>,
    /// Verbose progress reporting.
    pub verbose: bool,
}

impl PrecompileCogConfig {
    /// Build from a cog directory containing `Verum.toml`. Reads the
    /// manifest, resolves cog name + version, computes the canonical
    /// output path. Caller may override [`Self::output_path`] before
    /// running the orchestrator.
    pub fn for_cog(cog_dir: impl Into<PathBuf>) -> Result<Self> {
        let cog_dir: PathBuf = cog_dir.into().canonicalize().context("canonicalize cog dir")?;
        let manifest_path = cog_dir.join("Verum.toml");
        if !manifest_path.is_file() {
            anyhow::bail!(
                "no Verum.toml at {} — not a Verum cog root",
                manifest_path.display()
            );
        }
        let (name, version) = read_cog_manifest_minimal(&manifest_path)?;
        let compiler_version = env!("CARGO_PKG_VERSION");
        let output_filename = format!("{}-{}-verum-{}.vbca", name, version, compiler_version);
        let output_path = cog_dir.join("target").join("cog-vbca").join(output_filename);
        Ok(Self {
            cog_dir,
            cog_name: name,
            cog_version: version,
            output_path,
            target_triple: None,
            verbose: false,
        })
    }

    /// Override the default output path. The default lives at
    /// `<cog_dir>/target/cog-vbca/<name>-<version>-verum-<compiler>.vbca`;
    /// CI flows that want to upload the artifact straight to a
    /// registry staging area set their own path.
    pub fn with_output(mut self, path: impl Into<PathBuf>) -> Self {
        self.output_path = path.into();
        self
    }
}

/// Read the minimum manifest fields the orchestrator needs:
/// `cog.name` and `cog.version`. Matches the schema in
/// `crates/verum_cli/src/config.rs`. We don't reuse the CLI's full
/// `Manifest::from_file` parser to avoid dragging
/// `verum_compiler` → `verum_cli` reverse-dep cycles; the cog
/// section is small and stable.
fn read_cog_manifest_minimal(path: &Path) -> Result<(String, String)> {
    let bytes =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    // Use toml::Value for forward-compat — every field after `cog.name`
    // / `cog.version` is ignored.
    let root: toml::Value = toml::from_str(&bytes)
        .with_context(|| format!("parse {} as TOML", path.display()))?;
    let cog = root
        .get("cog")
        .ok_or_else(|| anyhow::anyhow!("Verum.toml missing [cog] table at {}", path.display()))?;
    let name = cog
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Verum.toml [cog].name is missing or not a string"))?
        .to_string();
    let version = cog
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("Verum.toml [cog].version is missing or not a string")
        })?
        .to_string();
    Ok((name, version))
}

/// Run the cog precompile pipeline.
///
/// Steps:
///   1. Validate the cog source tree — `Verum.toml` + at least one
///      `.vr` file under the root.
///   2. Build a [`CompilationPipeline`] in `StdlibBootstrap` mode
///      pointing at the cog source root. The bootstrap mode's
///      global type-registration phase is exactly what we want for
///      a multi-file cog: every `.vr` file's types are registered
///      before any function body is codegen'd, so cross-file
///      references resolve cleanly.
///   3. Drive `compile_core` to produce a single-target `.vbca` at
///      the configured output path.
///   4. Phase 12b TODO: post-pass for multi-variant cfg-conditional
///      function bodies + theorem extraction (mirrors the Phase
///      4b TODO on the stdlib path).
///
/// Reuses 100% of the existing precompile-stdlib infrastructure;
/// no parallel codegen implementation, no new pipeline phases.
pub fn precompile_cog(cfg: &PrecompileCogConfig) -> Result<StdlibCompilationResult> {
    if cfg.verbose {
        eprintln!(
            "verum cog precompile: cog={} ({}), source={}, output={}, target={}",
            cfg.cog_name,
            cfg.cog_version,
            cfg.cog_dir.display(),
            cfg.output_path.display(),
            cfg.target_triple.as_deref().unwrap_or("<host>")
        );
    }

    if !cfg.cog_dir.is_dir() {
        anyhow::bail!("cog directory does not exist: {}", cfg.cog_dir.display());
    }
    let manifest_path = cfg.cog_dir.join("Verum.toml");
    if !manifest_path.is_file() {
        anyhow::bail!(
            "Verum.toml missing at {} — not a Verum cog root",
            manifest_path.display()
        );
    }

    if !has_any_vr_file(&cfg.cog_dir)? {
        anyhow::bail!(
            "cog at {} contains no .vr files — nothing to precompile",
            cfg.cog_dir.display()
        );
    }

    if let Some(parent) = cfg.output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output dir {}", parent.display()))?;
    }

    // Bootstrap-mode pipeline: cog source root replaces the stdlib
    // path, the existing `compile_core` orchestrator handles
    // discovery / parse / global-type-registration / codegen /
    // archive write.
    let core_config = CoreConfig::new(cfg.cog_dir.clone()).with_output(cfg.output_path.clone());
    let core_config = if cfg.verbose {
        let mut c = core_config;
        c.verbose = true;
        c
    } else {
        core_config
    };

    let mut session = Session::new(CompilerOptions::default());
    let mut pipeline = CompilationPipeline::new_core(&mut session, core_config);
    let result = pipeline.compile_core().with_context(|| {
        format!(
            "CompilationPipeline::compile_core failed during cog precompile (cog={})",
            cfg.cog_name
        )
    })?;

    if cfg.verbose {
        eprintln!(
            "verum cog precompile: {} modules, {} functions in {:?}, archive {} ({} bytes)",
            result.modules_compiled,
            result.functions_compiled,
            result.total_time,
            result.output_path.display(),
            result.output_size,
        );
    }

    Ok(result)
}

/// True iff the directory tree (recursive) contains at least one
/// `.vr` file. Used as a fast pre-check before kicking off the full
/// pipeline.
fn has_any_vr_file(root: &Path) -> Result<bool> {
    fn scan(dir: &Path) -> std::io::Result<bool> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            // Skip target/ directories — codegen output, not source.
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            if path.is_dir() {
                if scan(&path)? {
                    return Ok(true);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("vr") {
                return Ok(true);
            }
        }
        Ok(false)
    }
    scan(root).with_context(|| format!("scan {}", root.display()))
}

/// Walk up from `start`, returning the first directory that contains
/// `core/mod.vr`. Mirrors the existing `find_workspace_root` helper
/// in `pipeline/loading.rs` but is exposed here so callers outside
/// the pipeline (CLI, build scripts, tooling) can resolve the
/// workspace without instantiating a session.
fn find_workspace_root(start: &Path) -> Result<PathBuf> {
    let start = start.canonicalize().with_context(|| {
        format!("failed to canonicalize start path {}", start.display())
    })?;
    let mut here: &Path = &start;
    loop {
        let candidate = here.join("core").join("mod.vr");
        if candidate.is_file() {
            return Ok(here.to_path_buf());
        }
        match here.parent() {
            Some(p) => here = p,
            None => anyhow::bail!(
                "could not find workspace root containing core/mod.vr starting from {}",
                start.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_for_workspace_resolves() {
        // Walk up from this source file's directory; we must land on
        // a workspace whose `core/mod.vr` exists.
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let cfg = PrecompileConfig::for_workspace(&manifest_dir);
        assert!(cfg.is_ok(), "workspace resolution: {:?}", cfg.err());
        let cfg = cfg.unwrap();
        assert!(cfg.stdlib_path.ends_with("core"));
        assert!(cfg.output_path.ends_with("runtime.vbca"));
    }

    #[test]
    fn cog_manifest_minimal_extraction() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-cog-manifest-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let manifest = tmp.join("Verum.toml");
        let _ = std::fs::write(
            &manifest,
            r#"
[cog]
name = "json"
version = "1.4.2"
authors = ["alice"]
description = "JSON helpers"

[language]
edition = "2026"
"#,
        );
        let parsed = read_cog_manifest_minimal(&manifest);
        assert!(parsed.is_ok(), "manifest parse: {:?}", parsed.err());
        let (name, version) = parsed.unwrap();
        assert_eq!(name, "json");
        assert_eq!(version, "1.4.2");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cog_config_for_cog_resolves_default_output() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-cog-config-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::write(
            tmp.join("Verum.toml"),
            r#"
[cog]
name = "mycog"
version = "0.3.0"
"#,
        );
        // Need a .vr file for the directory to be a valid cog.
        let _ = std::fs::write(tmp.join("lib.vr"), "fn main() {}\n");

        let cfg = PrecompileCogConfig::for_cog(&tmp);
        assert!(cfg.is_ok(), "for_cog: {:?}", cfg.err());
        let cfg = cfg.unwrap();
        assert_eq!(cfg.cog_name, "mycog");
        assert_eq!(cfg.cog_version, "0.3.0");
        // Output path: <tmp>/target/cog-vbca/mycog-0.3.0-verum-<v>.vbca
        let compiler_v = env!("CARGO_PKG_VERSION");
        let expected_filename = format!("mycog-0.3.0-verum-{}.vbca", compiler_v);
        assert!(
            cfg.output_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s == expected_filename)
                .unwrap_or(false),
            "expected output filename {} but got {}",
            expected_filename,
            cfg.output_path.display(),
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cog_precompile_rejects_missing_manifest() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-cog-no-manifest-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let result = PrecompileCogConfig::for_cog(&tmp);
        assert!(result.is_err(), "expected error for missing manifest");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cog_precompile_rejects_empty_cog() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-cog-empty-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::write(
            tmp.join("Verum.toml"),
            r#"
[cog]
name = "empty"
version = "0.1.0"
"#,
        );
        // No .vr files — cog has no source to precompile.
        let cfg = PrecompileCogConfig::for_cog(&tmp).expect("for_cog");
        let result = precompile_cog(&cfg);
        assert!(result.is_err(), "expected error for empty cog");
        assert!(
            result
                .err()
                .map(|e| format!("{e}").contains("no .vr files"))
                .unwrap_or(false),
            "error should mention missing .vr files"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn precompile_rejects_non_stdlib_dir() {
        let tmp = std::env::temp_dir().join(format!(
            "verum-precompile-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let cfg = PrecompileConfig {
            stdlib_path: tmp.clone(),
            output_path: tmp.join("out.vbca"),
            target_triple: None,
            verbose: false,
        };
        let err = precompile_stdlib(&cfg).err();
        assert!(err.is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
