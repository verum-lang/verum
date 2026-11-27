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

use std::path::{Path, PathBuf};
use std::num::NonZeroUsize;

use verum_compiler::precompile::{PrecompileConfig, precompile_stdlib};

use crate::error::CliError;
use crate::ui;

pub fn check(
    modules: &[verum_common::Text],
    stdlib_path: Option<PathBuf>,
    threads: Option<NonZeroUsize>,
) -> Result<(), CliError> {
    use verum_compiler::{CompilerOptions, CoreConfig, Session, pipeline::CompilationPipeline};

    let cfg = resolve_config(stdlib_path, None, None, threads, false)?;
    let mut options = CompilerOptions::default();
    if let Some(workers) = threads {
        options.num_threads = workers.get();
    }
    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_core(&mut session, CoreConfig::new(cfg.stdlib_path));
    pipeline.check_core_sources(modules)
        .map_err(|error| CliError::Custom(format!("{error:#}")))?;
    for module in modules {
        ui::detail("Checked", module);
    }
    Ok(())
}

/// Entry point invoked by the dispatcher in `main.rs`.
pub fn run(
    stdlib_path: Option<PathBuf>,
    out: Option<PathBuf>,
    target: Option<String>,
    threads: Option<NonZeroUsize>,
    verbose: bool,
) -> Result<(), CliError> {
    let cfg = resolve_config(stdlib_path, out, target, threads, verbose)?;

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

/// Explicit source selection does not require a surrounding Verum workspace.
/// Workspace discovery remains the authority only when no source was supplied.
fn resolve_config(
    stdlib_path: Option<PathBuf>,
    out: Option<PathBuf>,
    target: Option<String>,
    threads: Option<NonZeroUsize>,
    verbose: bool,
) -> Result<PrecompileConfig, CliError> {
    let mut cfg = match stdlib_path {
        Some(source) => PrecompileConfig::for_source(source, Path::new(".")),
        None => {
            let cwd = std::env::current_dir().map_err(|e| {
                CliError::Custom(format!("failed to read cwd for workspace resolution: {e}"))
            })?;
            PrecompileConfig::for_workspace(&cwd).map_err(|e| {
                CliError::Custom(format!(
                    "failed to resolve workspace root from cwd: {e}. Pass --stdlib-path explicitly."
                ))
            })?
        }
    };
    if let Some(output) = out {
        cfg.output_path = output;
    }
    cfg.target_triple = target;
    cfg.threads = threads;
    cfg.verbose = verbose;
    Ok(cfg)
}

#[cfg(test)]
#[path = "../../tests/commands/stdlib_precompile.rs"]
mod tests;
