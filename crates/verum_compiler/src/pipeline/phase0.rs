//! Phase 0: stdlib AOT-prep + workspace-root discovery.
//!
//! Extracted from `pipeline.rs` (#106 Phase 17). Houses the
//! one-shot "compile verum_std → static library + FFI exports +
//! symbol registry" preparation step that runs once per build,
//! plus the workspace-root finder that locates `Verum.toml` /
//! `Cargo.toml`-rooted projects.
//!
//! Methods:
//!
//!   * `phase0_stdlib_preparation` — primary entry; required for
//!     AOT/JIT modes, skipped for Interpret/Check.
//!   * `add_stdlib_builtin_exports` — registers core verum-std
//!     symbols into the FFI export table.
//!   * `find_workspace_root` — searches CWD ancestors for the
//!     workspace marker file.
//!   * `find_workspace_from_path` — variant that takes an
//!     explicit start path.

use std::path::{Path, PathBuf};

use anyhow::Result;
use tracing::{debug, info};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    // ==================== PHASE 0: stdlib COMPILATION ====================

    /// Phase 0: stdlib Compilation & Preparation
    ///
    /// This phase runs once per build and compiles the Verum standard library
    /// from Rust source to static library, generating FFI exports and symbol
    /// registries for consumption by all execution tiers.
    ///
    /// Outputs are cached and reused across compilations unless verum_std
    /// source files change.
    ///
    /// **Mode-specific behavior:**
    /// - `Interpret` mode: SKIPPED - interpreter uses Rust native execution
    /// - `Check` mode: SKIPPED - type checking uses built-in type definitions
    /// - `Aot` mode: REQUIRED - static library for native linking
    /// - `Jit` mode: REQUIRED - symbol registry for JIT compilation
    ///
    /// Phase 0: Compile verum_std to static lib, generate C-compatible FFI exports,
    /// build symbol registry, prepare LLVM bitcode for LTO, cache monomorphized generics.
    fn phase0_stdlib_preparation(&mut self) -> Result<()> {
        // Check if we already have cached artifacts
        if self.stdlib_artifacts.is_some() {
            debug!("Phase 0: Using cached stdlib artifacts");
            return Ok(());
        }

        // Skip Phase 0 for modes that don't need compiled stdlib
        // Interpreter uses Rust native execution, not C linking
        // Check mode only needs type definitions, not runtime library
        match self.mode {
            CompilationMode::Interpret => {
                debug!("Phase 0: Skipped for interpreter mode (uses Rust native execution)");
                return Ok(());
            }
            CompilationMode::Check => {
                debug!("Phase 0: Skipped for check mode (uses built-in type definitions)");
                return Ok(());
            }
            CompilationMode::Aot | CompilationMode::Jit => {
                // Continue with Phase 0 compilation
            }
            CompilationMode::MlirJit | CompilationMode::MlirAot => {
                debug!("Phase 0: Skipped for MLIR mode (uses MLIR-based stdlib)");
                return Ok(());
            }
        }

        info!("Phase 0: Compiling stdlib (first run or cache invalid)");
        let start = Instant::now();

        // Determine workspace root
        // Try to find Cargo.toml in parent directories
        let workspace_root = self.find_workspace_root()?;

        // Create Phase 0 compiler
        let stdlib_path = workspace_root.join("stdlib");
        let cache_dir = workspace_root.join("target/verum_cache/stdlib");

        let phase0 = Phase0CoreCompiler::new(stdlib_path, cache_dir);

        // Compile stdlib
        let artifacts = phase0
            .compile_core()
            .context("Phase 0 stdlib compilation failed")?;

        let elapsed = start.elapsed();

        info!(
            "Phase 0 completed in {:.2}s ({} functions registered)",
            elapsed.as_secs_f64(),
            artifacts.registry.functions.len()
        );

        if self.session.options().verbose >= 2 {
            info!("  Static library: {}", artifacts.static_library.display());
            info!("  LLVM bitcode: {}", artifacts.bitcode_library.display());
            info!(
                "  FFI exports: {} symbols",
                artifacts.ffi_exports.symbol_mappings.len()
            );
            info!(
                "  Monomorphized: {} instantiations",
                artifacts.monomorphization_cache.instantiations.len()
            );
        }

        // Cache artifacts for subsequent compilations
        self.stdlib_artifacts = Some(artifacts);

        Ok(())
    }

    /// Add synthetic exports for stdlib modules when AST-extracted exports are insufficient.
    ///
    /// Exports should be derived from actually compiled modules (the .vr source files).
    /// This function is now a no-op: all stdlib exports come from the actual module AST
    /// via `extract_exports_from_module`. Hardcoded synthetic exports were removed because
    /// they duplicated information that should live in the .vr source files and could
    /// drift out of sync with the actual stdlib definitions.
    fn add_stdlib_builtin_exports(
        &self,
        _export_table: &mut verum_modules::ExportTable,
        _module_id: verum_modules::ModuleId,
        _module_path: &str,
    ) {
        // All exports are now derived from the actual .vr source files.
        // If a stdlib module needs to export a type or function, it must be
        // declared in the corresponding .vr file and will be extracted by
        // extract_exports_from_module().
    }

    /// Find the workspace root directory using multiple strategies.
    ///
    /// Strategies (in priority order):
    /// 1. `VERUM_WORKSPACE_ROOT` environment variable (set by CLI/tests)
    /// 2. Walk up from the verum binary location (reliable for installed binaries)
    /// 3. Walk up from input file directory (original behavior)
    /// 4. Walk up from current working directory
    ///
    /// This ensures reliable workspace detection regardless of where the
    /// compilation is invoked from (test directories, CI/CD, etc.).
    fn find_workspace_root(&self) -> Result<PathBuf> {
        // Strategy 1: Environment variable (highest priority, used by tests)
        if let Ok(workspace_root) = std::env::var("VERUM_WORKSPACE_ROOT") {
            let path = PathBuf::from(&workspace_root);
            if path.exists() && (path.join("core").exists() || path.join("stdlib").exists()) {
                debug!("Using VERUM_WORKSPACE_ROOT: {}", path.display());
                return Ok(path);
            }
        }

        // Strategy 2: Walk up from the verum binary's actual location
        // This works reliably for installed binaries in target/debug or target/release
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(workspace) = self.find_workspace_from_path(&exe_path) {
                debug!(
                    "Found workspace from binary location: {}",
                    workspace.display()
                );
                return Ok(workspace);
            }
        }

        // Strategy 3: Walk up from input file's parent directory
        let input_path = &self.session.options().input;
        if let Some(workspace) = self.find_workspace_from_path(input_path) {
            debug!("Found workspace from input file: {}", workspace.display());
            return Ok(workspace);
        }

        // Strategy 4: Walk up from current working directory
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(workspace) = self.find_workspace_from_path(&cwd) {
                debug!("Found workspace from CWD: {}", workspace.display());
                return Ok(workspace);
            }
        }

        // All strategies failed
        Err(anyhow::anyhow!(
            "Could not find Verum workspace root. \
             Set VERUM_WORKSPACE_ROOT environment variable or run from within the workspace. \
             The workspace must contain core/Cargo.toml"
        ))
    }

    /// Helper: Walk up the directory tree from a starting path to find workspace root.
    ///
    /// A valid workspace root is identified by one of (in priority order):
    /// 1. A directory containing `core/mod.vr` (stdlib source tree — most reliable)
    /// 2. A directory containing `Verum.toml` with a `core/` sibling
    /// 3. A directory containing `Cargo.toml` with `[workspace]` and `core/` (dev mode)
    fn find_workspace_from_path(&self, start_path: &Path) -> Option<PathBuf> {
        // Canonicalize to get absolute path (resolve symlinks)
        let abs_path = start_path.canonicalize().ok()?;

        // Start from the path itself or its parent if it's a file
        let mut current = if abs_path.is_file() {
            abs_path.parent()?.to_path_buf()
        } else {
            abs_path
        };

        // Walk up the directory tree
        loop {
            // Primary: directory with core/mod.vr is a Verum workspace root
            if current.join("core").join("mod.vr").exists() {
                return Some(current);
            }

            // Secondary: Verum.toml with core/ directory
            if current.join("Verum.toml").exists()
                && (current.join("core").exists() || current.join("stdlib").exists())
            {
                return Some(current);
            }

            // Tertiary: Cargo.toml with [workspace] (Rust dev mode)
            let cargo_toml = current.join("Cargo.toml");
            if cargo_toml.exists() {
                if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                    if content.contains("[workspace]") {
                        if current.join("core").exists() || current.join("stdlib").exists() {
                            return Some(current);
                        }
                    }
                }
            }

            // Move to parent directory
            match current.parent() {
                Some(parent) if parent != current => {
                    current = parent.to_path_buf();
                }
                _ => {
                    // Reached filesystem root without finding workspace
                    break;
                }
            }
        }

        None
    }
}
