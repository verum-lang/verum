//! `verum cog precompile` — Phase 12 of the precompiled-stdlib
//! archive epic.
//!
//! Wires the existing
//! [`verum_compiler::precompile::precompile_cog`] orchestrator
//! through the CLI surface so cog authors and CI flows can produce
//! `.vbca` artefacts ready for registry distribution. The cog
//! orchestrator reuses 100% of the stdlib precompile pipeline
//! ([`verum_compiler::precompile::precompile_stdlib`]) — same
//! global type registration, same per-module codegen, same archive
//! writer; the only differences are the source-tree path and the
//! output filename convention.

use std::path::PathBuf;

use verum_compiler::precompile::{PrecompileCogConfig, precompile_cog};

use crate::error::CliError;
use crate::ui;

/// Entry point invoked by the dispatcher in `main.rs`.
///
/// Resolves the cog directory (defaulting to the current working
/// directory when not specified), reads `Verum.toml` to pick up
/// the cog's name + version, and drives the precompile pipeline.
/// On success the output `.vbca` lands at the canonical
/// registry-naming path
/// `<cog>/target/cog-vbca/<name>-<version>-verum-<compiler>.vbca`
/// unless overridden via `--out`.
pub fn run(
    cog_dir: Option<PathBuf>,
    out: Option<PathBuf>,
    target: Option<String>,
    verbose: bool,
) -> Result<(), CliError> {
    let cog_dir = match cog_dir {
        Some(p) => p,
        None => std::env::current_dir().map_err(|e| {
            CliError::Custom(format!("failed to read cwd for cog directory: {e}"))
        })?,
    };

    let mut cfg = PrecompileCogConfig::for_cog(&cog_dir).map_err(|e| {
        CliError::Custom(format!(
            "failed to resolve cog at {}: {e}",
            cog_dir.display()
        ))
    })?;

    if let Some(o) = out {
        cfg.output_path = o;
    }
    cfg.target_triple = target;
    cfg.verbose = verbose;

    ui::step(&format!(
        "Precompiling cog {} {} to VBC archive",
        cfg.cog_name, cfg.cog_version
    ));
    ui::detail("Source", &cfg.cog_dir.display().to_string());
    ui::detail("Output", &cfg.output_path.display().to_string());
    if let Some(t) = cfg.target_triple.as_deref() {
        ui::detail("Target", t);
    }

    let result = precompile_cog(&cfg)
        .map_err(|e| CliError::Custom(format!("cog precompile failed: {e:?}")))?;

    ui::detail(
        "Modules compiled",
        &format!("{}", result.modules_compiled),
    );
    ui::detail(
        "Functions compiled",
        &format!("{}", result.functions_compiled),
    );
    ui::detail(
        "Archive size",
        &format!("{} bytes", result.output_size),
    );
    ui::detail(
        "Duration",
        &format!("{:.2}s", result.total_time.as_secs_f64()),
    );

    Ok(())
}
