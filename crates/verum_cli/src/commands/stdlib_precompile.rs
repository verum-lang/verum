//! `verum stdlib precompile` — Phase 4 of the precompiled-stdlib
//! archive epic. Wires the existing
//! [`verum_compiler::precompile::precompile_stdlib`] orchestrator
//! through the CLI surface so build scripts (Phase 5) and CI can
//! refresh the embedded stdlib archive without an explicit
//! `cargo xtask` step.
//!
//! Not a `cargo xtask` because the underlying pipeline lives in
//! `verum_compiler` already — exposing it through the existing
//! `verum` binary keeps the public surface minimal and the
//! reuse-don't-fork principle intact.

use std::path::PathBuf;

use verum_compiler::precompile::{PrecompileConfig, precompile_stdlib};

use crate::error::CliError;
use crate::ui;

/// Entry point invoked by the dispatcher in `main.rs`.
pub fn run(
    stdlib_path: Option<PathBuf>,
    out: Option<PathBuf>,
    target: Option<String>,
    verbose: bool,
) -> Result<(), CliError> {
    let cwd = std::env::current_dir().map_err(|e| {
        CliError::Custom(format!("failed to read cwd for workspace resolution: {e}"))
    })?;

    let mut cfg = PrecompileConfig::for_workspace(&cwd).map_err(|e| {
        CliError::Custom(format!(
            "failed to resolve workspace root from cwd: {e}. Pass --stdlib-path explicitly."
        ))
    })?;

    if let Some(p) = stdlib_path {
        cfg.stdlib_path = p;
    }
    if let Some(o) = out {
        cfg.output_path = o;
    }
    cfg.target_triple = target;
    cfg.verbose = verbose;

    ui::step("Precompiling stdlib to VBC archive");
    ui::detail("Source", &cfg.stdlib_path.display().to_string());
    ui::detail("Output", &cfg.output_path.display().to_string());
    if let Some(t) = cfg.target_triple.as_deref() {
        ui::detail("Target", t);
    }

    let result = precompile_stdlib(&cfg)
        .map_err(|e| CliError::Custom(format!("stdlib precompile failed: {e:?}")))?;

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
