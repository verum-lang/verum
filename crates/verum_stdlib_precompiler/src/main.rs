//! T4 of the single-path archive-driven epic.
//!
//! Standalone binary that produces `target/precompiled-stdlib/runtime.vbca`
//! and `target/precompiled-stdlib/runtime.core_metadata`.  Same as
//! `verum stdlib precompile` but in a binary that does NOT depend
//! on the embedded archive — `verum_cli` does, so its build can't
//! invoke itself for the precompile step (chicken-and-egg).
//!
//! Invoked by `verum_compiler/build.rs` (T3) when the
//! checksum-cached archive is stale.  Caller passes the workspace
//! root as the first argument; the binary writes to
//! `<workspace>/target/precompiled-stdlib/runtime.{vbca,core_metadata}`
//! and exits with status 0 on success.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("verum_stdlib_precompiler: {:?}", e);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let workspace_root = match args.next() {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()
            .context("no workspace root supplied and CWD lookup failed")?,
    };

    if !workspace_root.join("core").join("mod.vr").is_file() {
        anyhow::bail!(
            "expected `core/mod.vr` under workspace root {} — pass the verum workspace path as the first argument",
            workspace_root.display()
        );
    }

    let cfg = verum_compiler::precompile::PrecompileConfig::for_workspace(&workspace_root)?;
    let result = verum_compiler::precompile::precompile_stdlib(&cfg)
        .context("precompile_stdlib failed")?;

    eprintln!(
        "verum_stdlib_precompiler: {} modules, {} functions in {:?}, {} bytes",
        result.modules_compiled,
        result.functions_compiled,
        result.total_time,
        result.output_size,
    );
    Ok(())
}
