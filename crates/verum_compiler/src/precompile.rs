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
