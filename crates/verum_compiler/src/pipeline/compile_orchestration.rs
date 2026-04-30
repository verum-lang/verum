//! Multi-file compilation orchestration.
//!
//! Extracted from `pipeline.rs` (#106 Phase 14). Houses the four
//! public entry points that drive compilation across discovered
//! source files:
//!
//!   * `compile_string` — single-source convenience for testing
//!     and simple use cases.
//!   * `compile_multi_pass` — three-pass architecture
//!     (parse → analyze → verify) over a Map of pre-parsed
//!     sources, with rayon-parallel per-module type checking
//!     and verification.
//!   * `compile_project` — discovers + parses + multi-passes
//!     all .vr files in the project tree.
//!   * `check_project` — type-check-only project flow that
//!     returns a CheckResult with diagnostics + statistics
//!     (used by IDEs, CI/CD, dev workflows).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as AnyhowContext, Result};
use tracing::{debug, info, warn};

use verum_ast::{decl::ItemKind, FileId, Module};
use verum_common::{List, Map, Shared, Text};
use verum_diagnostics::DiagnosticBuilder;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;
use verum_modules::{
    ModuleId, ModuleInfo, extract_exports_from_module, resolve_glob_reexports,
    resolve_specific_reexport_kinds,
};
use verum_types::TypeChecker;

use crate::hash::compute_item_hashes_from_module;
use crate::phases::type_error_to_diagnostic;

use super::{
    CheckResult, CompilationMode, CompilationPipeline, CompilerPass, save_registry_to_disk,
    should_parse_as_script,
};

impl<'s> CompilationPipeline<'s> {
    /// Compile a string of source code (simple API)
    ///
    /// This is a convenience method for testing and simple use cases.
    /// It compiles the given source code as a single module.
    pub fn compile_string(&mut self, source: &str) -> Result<()> {
        let start = Instant::now();

        info!("Compiling source string ({} bytes)", source.len());

        // Load stdlib modules first (enables std.* imports and type resolution)
        self.load_stdlib_modules()?;

        // Create a temporary file ID for the source
        let temp_path = PathBuf::from("<string>");
        let file_id = self
            .session
            .load_source_string(source, temp_path.clone())
            .context("Failed to load source string")?;

        // Parse
        let module = self.phase_parse(file_id)?;

        // Type check
        self.phase_type_check(&module)?;

        // Dependency analysis (validates against target constraints)
        self.phase_dependency_analysis(&module)?;

        // Verify refinements if enabled
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // CBGR analysis
        self.phase_cbgr_analysis(&module)?;

        // Execution (if mode requires it)
        match self.mode {
            CompilationMode::Interpret => {
                self.phase_interpret(&module)?;
            }
            CompilationMode::Check => {
                info!("Check-only mode: skipping interpretation");
            }
            CompilationMode::Jit | CompilationMode::Aot => {
                info!("VBC → LLVM compilation: use run_vbc_jit/run_vbc_aot methods");
            }
            CompilationMode::MlirJit | CompilationMode::MlirAot => {
                info!("MLIR compilation: use run_mlir_jit/run_mlir_aot methods");
            }
        }

        let elapsed = start.elapsed();
        info!("Compilation completed in {:.2}s", elapsed.as_secs_f64());

        Ok(())
    }

    // compile_core + parse_stdlib_module_files + register_stdlib_types_globally
    // + compile_core_module_from_ast + is_forward_reference_to_later_module
    // + merge_stdlib_vbc_modules + build_stdlib_archive extracted to
    // crate::pipeline::stdlib_bootstrap (#106 Phase 8 — pipeline.rs split).

    /// Run multi-pass compilation on multiple source files.
    ///
    /// This is the main entry point for the multi-pass architecture.
    /// It processes all source files through three distinct passes.
    pub fn compile_multi_pass(&mut self, sources: &Map<Text, Text>) -> Result<()> {
        let start = Instant::now();

        info!(
            "Starting multi-pass compilation for {} source(s)",
            sources.len()
        );

        // PHASE 0: stdlib Compilation & Preparation (runs once, cached)
        self.phase0_stdlib_preparation()?;

        // PASS 1: Parse and Register
        self.current_pass = CompilerPass::Pass1Registration;
        info!("Pass 1: Registration phase");

        for (path, source) in sources.iter() {
            debug!("  Registering meta declarations from: {}", path.as_str());
            let module = self.parse_and_register(path, source)?;
            self.modules.insert(path.clone(), Arc::new(module));
        }

        // Check for circular dependencies
        if let Err(e) = self.meta_registry.check_circular_dependencies() {
            return Err(anyhow::anyhow!("Circular dependency detected: {}", e));
        }

        info!(
            "  Registered {} meta functions and {} macros",
            self.meta_registry.all_meta_functions().len(),
            self.meta_registry.all_macros().len()
        );

        // PASS 1.5: Register modules in session registry for cross-file resolution
        // This builds export tables and allows types to be resolved across files
        info!("Pass 1.5: Module registration for cross-file resolution");
        self.register_modules_for_cross_file_resolution()?;

        // PASS 2: Staged Compilation with Caching
        // Uses StagedPipeline for N-level metaprogramming with fine-grained caching.
        // Falls back to simple expand_module() on errors for robustness.
        // N-level staging: meta(N) generates meta(N-1) code with dependency tracking.
        self.current_pass = CompilerPass::Pass2Expansion;
        info!("Pass 2: Staged compilation with caching");

        // Collect module paths to avoid borrowing issues
        let module_paths: List<Text> = self.modules.keys().cloned().collect();
        let mut total_cache_hits = 0u64;
        let mut total_meta_executions = 0u64;

        for path in &module_paths {
            debug!("  Staged compilation for: {}", path.as_str());

            if let Some(module_rc) = self.modules.get(path).cloned() {
                let module = (*module_rc).clone();

                // Reset staged pipeline for this module (keeps config, clears state)
                self.staged_pipeline.reset();

                // Import meta functions from main registry into staged pipeline
                self.staged_pipeline.import_from_registry(&self.meta_registry, &module);

                // Execute staged compilation
                match self.staged_pipeline.compile(module) {
                    Ok(result) => {
                        // Emit diagnostics from staged compilation
                        for diag in result.diagnostics.iter() {
                            self.session.emit_diagnostic(diag.clone());
                        }

                        // Accumulate statistics
                        let stats = &result.stats;
                        total_cache_hits += stats.cache_hits.values().copied().sum::<u32>() as u64;
                        total_meta_executions += stats.total_meta_executions as u64;

                        debug!(
                            "    Staged compilation: {} stages, {} meta executions, {} cache hits",
                            stats.stages_processed,
                            stats.total_meta_executions,
                            stats.cache_hits.values().copied().sum::<u32>()
                        );

                        // Update module with expanded version (runtime_code from staged result)
                        self.modules.insert(path.clone(), Arc::new(result.runtime_code));
                    }
                    Err(e) => {
                        // Fallback to simple expansion on staged compilation error
                        warn!(
                            "  Staged compilation failed for {}, falling back to simple expansion: {}",
                            path.as_str(),
                            e
                        );

                        // Clone module again for fallback (original was consumed)
                        if let Some(module_rc) = self.modules.get(path) {
                            let mut fallback_module = (**module_rc).clone();
                            if let Err(expand_err) = self.expand_module(path, &mut fallback_module) {
                                warn!("  Fallback expansion also failed: {}", expand_err);
                                return Err(expand_err);
                            }
                            self.modules.insert(path.clone(), Arc::new(fallback_module));
                        }
                    }
                }
            }
        }

        info!(
            "  Staged compilation complete: {} modules, {} cache hits, {} meta executions",
            module_paths.len(),
            total_cache_hits,
            total_meta_executions
        );

        // PASS 2.5: Compute Item Hashes for Incremental Compilation
        // This enables fine-grained change detection (signature vs body changes).
        // Incremental: item-level hashes distinguish signature vs body changes.
        for path in &module_paths {
            if let Some(module_rc) = self.modules.get(path) {
                let hashes = compute_item_hashes_from_module(module_rc.as_ref());
                let file_path = PathBuf::from(path.as_str());
                self.incremental_compiler.update_item_hashes(file_path, hashes);
            }
        }
        debug!(
            "  Computed item hashes for {} modules (incremental: {} cached)",
            module_paths.len(),
            self.incremental_compiler.stats().item_hashes_cached
        );

        // PASS 2.75: Compute Fine-Grained Incremental Sets
        // This determines which modules need full recompilation vs. verification-only.
        // - Signature changes → full recompilation of dependents
        // - Body-only changes → verification-only of dependents (skip type checking)
        // Signature changes trigger full recompilation; body-only changes allow skip.
        let file_paths: Vec<PathBuf> = module_paths
            .iter()
            .map(|p| PathBuf::from(p.as_str()))
            .collect();

        // Compute which modules need full recompilation vs. verification-only
        let (full_recompile_set, verify_only_set) = self
            .incremental_compiler
            .compute_incremental_sets_fine_grained(&file_paths, |path| {
                // Find the module for this path and compute its hashes
                let path_text = Text::from(path.to_string_lossy().to_string());
                self.modules.get(&path_text).map(|module_rc| {
                    compute_item_hashes_from_module(module_rc.as_ref())
                })
            });

        // Convert back to Text paths for lookup
        let full_recompile: HashSet<Text> = full_recompile_set
            .iter()
            .map(|p| Text::from(p.to_string_lossy().to_string()))
            .collect();
        let verify_only: HashSet<Text> = verify_only_set
            .iter()
            .map(|p| Text::from(p.to_string_lossy().to_string()))
            .collect();

        if !verify_only.is_empty() {
            info!(
                "  Incremental: {} modules for full analysis, {} modules for verification-only",
                full_recompile.len(),
                verify_only.len()
            );
            debug!(
                "  Verification-only modules: {:?}",
                verify_only.iter().map(|t| t.as_str()).collect::<Vec<_>>()
            );
        }

        // PASS 3: Semantic Analysis (parallel, #101)
        //
        // Skip analysis for verification-only modules (they reuse cached
        // type check results). This is the key optimization: when only the
        // implementation of a dependency changed (not its signature), we
        // don't need to re-type-check dependent modules.
        //
        // Parallelism rationale: each call to `analyze_module` constructs
        // its own `TypeChecker` and never writes back to `Compiler` state
        // — every "mutation" routes through `Session::emit_diagnostic`
        // (lock-free SegQueue post-#105) or `Session::abort_if_errors`
        // (atomic counter). The shared `module_registry` and
        // `lazy_resolver` are `Arc<{RwLock,Mutex}<…>>`, so inter-thread
        // access is the lock primitives' problem, not ours. Reads of
        // `self.modules` / `self.collected_contexts` are pure HashMap /
        // List iteration with no concurrent writers in this phase.
        //
        // Opt-out: `VERUM_NO_PARALLEL_ANALYZE=1` falls back to the
        // sequential loop. Useful for debugging non-deterministic
        // diagnostic ordering or pinning down a parallel-only regression.
        self.current_pass = CompilerPass::Pass3Analysis;
        info!("Pass 3: Analysis phase");

        let parallel_analyze = std::env::var("VERUM_NO_PARALLEL_ANALYZE").is_err();

        // Pre-collect (path, module) pairs in topological order so we
        // can hand the parallel iterator a self-contained work list and
        // keep the hot-path closure free of HashMap lookups.
        let analysis_workset: Vec<(Text, Arc<Module>)> = module_paths
            .iter()
            .filter(|path| !verify_only.contains(*path))
            .filter_map(|path| {
                self.modules
                    .get(path)
                    .map(|module_rc| (path.clone(), Arc::clone(module_rc)))
            })
            .collect();

        if parallel_analyze && analysis_workset.len() > 1 {
            use rayon::prelude::*;
            debug!(
                "  Analyzing {} modules in parallel (rayon)",
                analysis_workset.len()
            );
            // `try_for_each` short-circuits on the first Err — preserves
            // the sequential loop's "bail on first failure" semantics.
            // Diagnostics from other threads still flow into the session
            // queue, so the post-pass `abort_if_errors` reports the
            // complete set rather than a single first-failure.
            analysis_workset
                .par_iter()
                .try_for_each(|(path, module)| -> Result<()> {
                    debug!("  Analyzing: {}", path.as_str());
                    self.analyze_module(path, module)
                })?;
        } else {
            for (path, module) in &analysis_workset {
                debug!("  Analyzing: {}", path.as_str());
                self.analyze_module(path, module)?;
            }
        }

        // Phase 4: Refinement verification (if enabled)
        // IMPORTANT: Run verification for BOTH full recompile AND verification-only modules.
        // Even if types didn't change, implementation changes may violate contracts.
        if self.session.options().verify_mode.use_smt() {
            // Combine full recompile and verify-only sets for verification
            let modules_to_verify: HashSet<&Text> = module_paths
                .iter()
                .filter(|p| full_recompile.contains(*p) || verify_only.contains(*p))
                .collect();

            if !modules_to_verify.is_empty() {
                info!(
                    "Pass 4: Verification phase ({} modules, {} verify-only)",
                    modules_to_verify.len(),
                    verify_only.len()
                );
            }

            // Parallel per-module verification (#100). `phase_verify`
            // is `&self` post-#100 audit and constructs its SMT
            // contexts (Z3 + CVC5) per call, so each rayon worker
            // owns its solver state. Inner per-function loop is also
            // parallel; the two layers compose into work-stealing
            // across both axes when project has multiple modules.
            let verify_workset: Vec<(Text, Arc<Module>)> = module_paths
                .iter()
                .filter(|path| modules_to_verify.contains(*path))
                .filter_map(|path| {
                    self.modules
                        .get(path)
                        .map(|module_rc| (path.clone(), Arc::clone(module_rc)))
                })
                .collect();

            let parallel_verify_outer =
                std::env::var("VERUM_NO_PARALLEL_VERIFY").is_err();

            if parallel_verify_outer && verify_workset.len() > 1 {
                use rayon::prelude::*;
                debug!(
                    "  Verifying {} modules in parallel (rayon outer)",
                    verify_workset.len()
                );
                verify_workset
                    .par_iter()
                    .try_for_each(|(path, module)| -> Result<()> {
                        debug!("  Verifying: {}", path.as_str());
                        self.phase_verify(module)
                    })?;
            } else {
                for (path, module) in &verify_workset {
                    debug!("  Verifying: {}", path.as_str());
                    self.phase_verify(module)?;
                }
            }
        }

        let elapsed = start.elapsed();
        info!(
            "Multi-pass compilation completed in {:.2}s",
            elapsed.as_secs_f64()
        );

        Ok(())
    }

    /// Compile a multi-file project by discovering all .vr files.
    ///
    /// This method uses the ModuleLoader to:
    /// 1. Discover all .vr files in the project directory
    /// 2. Load and parse each module
    /// 3. Register modules in the session's ModuleRegistry
    /// 4. Run multi-pass compilation
    ///
    /// Discovers .vr files following module-file mapping (foo.vr = module foo,
    /// foo/mod.vr = directory module). Registers in session's ModuleRegistry,
    /// then runs multi-pass compilation.
    ///
    /// Note: For deep recursion scenarios, ensure RUST_MIN_STACK is set
    /// appropriately (e.g., 16MB) in the build/test environment.
    pub fn compile_project(&mut self) -> Result<()> {
        let start = Instant::now();

        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;

        info!("Discovering project files...");
        let project_files = self.session.discover_project_files()?;

        if project_files.is_empty() {
            warn!("No .vr files found in project directory");
            return Ok(());
        }

        info!("Found {} .vr file(s)", project_files.len());

        // Load and parse all modules using ModuleLoader.
        //
        // File I/O is naturally parallel: each `read_to_string` is an
        // independent syscall, and OS schedulers exploit kernel-level
        // parallelism (epoll, io_uring) to overlap multiple reads
        // even on a single CPU. rayon's `par_iter` work-stealing
        // composes with the OS's read-ahead so wall-clock scales with
        // the slower of (cores, disk-throughput).
        //
        // Opt-out via `VERUM_NO_PARALLEL_READ=1` for diagnostic
        // ordering reproducibility in CI / regression triage.
        let parallel_read = std::env::var("VERUM_NO_PARALLEL_READ").is_err();
        let root_path = self.module_loader.root_path().to_path_buf();

        let to_module_entry = |file_path: &PathBuf| -> Result<(Text, Text)> {
            debug!("Loading module: {}", file_path.display());
            let source_text = std::fs::read_to_string(file_path)
                .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

            let mut module_path_str = file_path
                .strip_prefix(&root_path)
                .unwrap_or(file_path)
                .with_extension("")
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, ".");

            // Handle "mod" files — they represent their parent directory.
            // e.g., "domain.mod" -> "domain", "services.mod" -> "services"
            if module_path_str.ends_with(".mod") {
                module_path_str = module_path_str.trim_end_matches(".mod").to_string();
            } else if module_path_str == "mod" {
                module_path_str = String::new();
            }

            Ok((Text::from(module_path_str), Text::from(source_text)))
        };

        let entries: Vec<(Text, Text)> = if parallel_read && project_files.len() > 1 {
            use rayon::prelude::*;
            project_files
                .par_iter()
                .map(to_module_entry)
                .collect::<Result<Vec<_>>>()?
        } else {
            project_files
                .iter()
                .map(to_module_entry)
                .collect::<Result<Vec<_>>>()?
        };

        let mut sources = Map::new();
        for (k, v) in entries {
            sources.insert(k, v);
        }

        // Run multi-pass compilation on all discovered modules
        self.compile_multi_pass(&sources)?;

        let elapsed = start.elapsed();
        info!(
            "Project compilation completed in {:.2}s",
            elapsed.as_secs_f64()
        );

        Ok(())
    }

    /// Type-check a multi-file project without code generation.
    ///
    /// This method discovers all .vr files in the project directory and runs
    /// multi-pass type checking without generating any code. It's useful for:
    /// - IDE integration (quick feedback)
    /// - CI/CD pipelines (validation only)
    /// - Development workflows (check before run)
    ///
    /// Returns a CheckResult with diagnostics and statistics.
    ///
    /// Runs check-only mode: discovers modules, parses, type checks, but does
    /// not generate code. Returns diagnostics and statistics.
    ///
    /// Note: For deep recursion scenarios (type inference, import resolution,
    /// module dependency analysis), ensure RUST_MIN_STACK is set appropriately
    /// (e.g., 16MB) in the build/test environment.
    pub fn check_project(&mut self) -> Result<CheckResult> {
        let start = Instant::now();

        info!("Starting project-wide type checking...");

        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;

        // 1. Discover all .vr files in the project
        info!("Discovering project files...");
        let project_files = self.session.discover_project_files()?;

        if project_files.is_empty() {
            warn!("No .vr files found in project directory");
            return Ok(CheckResult::success(0, 0, start.elapsed()));
        }

        info!("Found {} .vr file(s) to check", project_files.len());

        // 2. Load all source files
        let mut sources = Map::new();
        // E_MODULE_PATH_COLLISION detector: track which file produced each
        // module path. Two filesystem rules (file-form `foo.vr` vs
        // directory-form `foo/mod.vr`) can both reach the same module
        // path; the silent-overwrite at `sources.insert(...)` would let
        // the second file win without warning, leaving the first file's
        // declarations unreachable.
        let mut path_to_source: std::collections::BTreeMap<String, PathBuf> =
            std::collections::BTreeMap::new();

        for file_path in &project_files {
            debug!("Loading module: {}", file_path.display());

            // Read source file
            let source_text = std::fs::read_to_string(file_path)
                .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

            // Convert path to module path
            let mut module_path_str = file_path
                .strip_prefix(self.module_loader.root_path())
                .unwrap_or(file_path)
                .with_extension("")
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, ".");

            // Handle "mod" files - they represent their parent directory
            // e.g., "domain.mod" -> "domain", "services.mod" -> "services"
            if module_path_str.ends_with(".mod") {
                module_path_str = module_path_str.trim_end_matches(".mod").to_string();
            } else if module_path_str == "mod" {
                // Root mod.vr -> empty string (root module)
                module_path_str = String::new();
            }

            if let Some(prev) = path_to_source.get(&module_path_str) {
                eprintln!(
                    "error<E_MODULE_PATH_COLLISION>: module path '{}' resolves to two source files",
                    if module_path_str.is_empty() { "<root>" } else { module_path_str.as_str() },
                );
                eprintln!("  using:    {}", prev.display());
                eprintln!("  ignoring: {}", file_path.display());
                eprintln!(
                    "  hint: pick exactly one of the file form (`<name>.vr`) \
                     or the directory form (`<name>/mod.vr`); having both makes \
                     declarations in the loser invisible at use sites and is \
                     classified as `E_MODULE_PATH_COLLISION`"
                );
                continue;
            }
            path_to_source.insert(module_path_str.clone(), file_path.clone());

            sources.insert(Text::from(module_path_str), Text::from(source_text));
        }

        // Track the initial diagnostic count
        let initial_error_count = self.session.error_count();
        let initial_warning_count = self.session.warning_count();

        // 3. PHASE 0: stdlib Compilation & Preparation
        // CRITICAL: Skip stdlib compilation for check-only mode.
        // Type checking doesn't need compiled stdlib - it only needs type signatures
        // which are available from the AST and built-in type definitions.
        // This avoids expensive subprocess calls and Cargo compilation.
        if self.mode != CompilationMode::Check {
            if let Err(e) = self.phase0_stdlib_preparation() {
                debug!(
                    "Skipping stdlib preparation (not required for this mode): {}",
                    e
                );
            }
        } else {
            debug!(
                "Phase 0: Skipped for check-only mode (type signatures available from built-ins)"
            );
        }

        // 4. PASS 1: Parse and Register
        self.current_pass = CompilerPass::Pass1Registration;
        info!("Pass 1: Registration phase");

        for (path, source) in sources.iter() {
            debug!("  Registering meta declarations from: {}", path.as_str());

            // Load source as a string (files are already loaded in sources map)
            let virtual_path = PathBuf::from(path.as_str());
            let file_id = self
                .session
                .load_source_string(source.as_str(), virtual_path)?;

            // Parse the module
            let lexer = Lexer::new(source.as_str(), file_id);
            let parser = VerumParser::new();
            let module_result = parser.parse_module(lexer, file_id);

            match module_result {
                Ok(mut module) => {
                    // Apply @cfg conditional compilation filtering
                    let cfg_evaluator = self.session.cfg_evaluator();
                    module.items = cfg_evaluator.filter_items(&module.items);

                    // Register meta functions and macros
                    for item in &module.items {
                        match &item.kind {
                            ItemKind::Function(func) if func.is_meta => {
                                // Register meta function
                                if let Err(e) = self
                                    .meta_registry
                                    .register_meta_function(&Text::from(path.as_str()), func)
                                {
                                    let diag = DiagnosticBuilder::error()
                                        .message(format!("Failed to register meta function: {}", e))
                                        .build();
                                    self.session.emit_diagnostic(diag);
                                }
                            }
                            ItemKind::Meta(_meta_decl) => {
                                // Register macro
                                debug!("  Found macro declaration (registration pending)");
                            }
                            _ => {
                                // Other items don't need registration
                            }
                        }
                    }

                    // Header validation at every
                    // user-source parse_module site. The
                    // virtual_path is a PathBuf reflecting the
                    // logical source path; pass it through to the
                    // validator so dangling forward-decls and
                    // inline-vs-filesystem overlaps surface here too.
                    let header_warnings =
                        verum_modules::loader::validate_module_headers_against_filesystem(
                            &PathBuf::from(path.as_str()),
                            &module,
                        );
                    for warning in header_warnings {
                        let diag = DiagnosticBuilder::warning()
                            .code(warning.code())
                            .message(warning.message())
                            .build();
                        self.session.emit_diagnostic(diag);
                    }

                    self.modules.insert(path.clone(), Arc::new(module));
                }
                Err(errors) => {
                    // Emit parse errors as diagnostics but continue with other modules
                    for error in errors {
                        let diag = DiagnosticBuilder::error()
                            .message(format!("Parse error: {}", error))
                            .build();
                        self.session.emit_diagnostic(diag);
                    }
                    debug!("  Skipping module {} due to parse errors", path.as_str());
                }
            }
        }

        // Check for circular dependencies
        if let Err(e) = self.meta_registry.check_circular_dependencies() {
            return Err(anyhow::anyhow!("Circular dependency detected: {}", e));
        }

        info!(
            "  Registered {} meta functions and {} macros",
            self.meta_registry.all_meta_functions().len(),
            self.meta_registry.all_macros().len()
        );

        // 4.5 PASS 1.5: Register modules for cross-file resolution
        // This builds export tables and extracts contexts/protocols for cross-file resolution
        // Build export tables (public items) and extract contexts/protocols
        // for cross-file resolution. Enables `using [Context]` across file boundaries.
        info!("Pass 1.5: Module registration for cross-file resolution");
        self.register_modules_for_cross_file_resolution()?;

        // 5. PASS 2: Staged Compilation with Caching
        // Uses StagedPipeline for N-level metaprogramming with fine-grained caching.
        // Falls back to simple expand_module() on errors for robustness.
        // N-level staging: meta(N) generates meta(N-1) code with dependency tracking.
        self.current_pass = CompilerPass::Pass2Expansion;
        info!("Pass 2: Staged compilation with caching");

        // Collect module paths to avoid borrowing issues
        let module_paths: List<Text> = self.modules.keys().cloned().collect();
        let mut total_cache_hits = 0u64;
        let mut total_meta_executions = 0u64;

        for path in &module_paths {
            // Skip stdlib modules - they don't need macro expansion in check mode
            if path.as_str().starts_with("core") {
                continue;
            }
            debug!("  Staged compilation for: {}", path.as_str());

            if let Some(module_rc) = self.modules.get(path).cloned() {
                let module = (*module_rc).clone();

                // Reset staged pipeline for this module (keeps config, clears state)
                self.staged_pipeline.reset();

                // Import meta functions from main registry into staged pipeline
                self.staged_pipeline.import_from_registry(&self.meta_registry, &module);

                // Execute staged compilation
                match self.staged_pipeline.compile(module) {
                    Ok(result) => {
                        // Emit diagnostics from staged compilation
                        for diag in result.diagnostics.iter() {
                            self.session.emit_diagnostic(diag.clone());
                        }

                        // Accumulate statistics
                        let stats = &result.stats;
                        total_cache_hits += stats.cache_hits.values().copied().sum::<u32>() as u64;
                        total_meta_executions += stats.total_meta_executions as u64;

                        debug!(
                            "    Staged compilation: {} stages, {} meta executions, {} cache hits",
                            stats.stages_processed,
                            stats.total_meta_executions,
                            stats.cache_hits.values().copied().sum::<u32>()
                        );

                        // Update module with expanded version (runtime_code from staged result)
                        self.modules.insert(path.clone(), Arc::new(result.runtime_code));
                    }
                    Err(e) => {
                        // Fallback to simple expansion on staged compilation error
                        warn!(
                            "  Staged compilation failed for {}, falling back to simple expansion: {}",
                            path.as_str(),
                            e
                        );

                        // Clone module again for fallback (original was consumed)
                        if let Some(module_rc) = self.modules.get(path) {
                            let mut fallback_module = (**module_rc).clone();
                            if let Err(expand_err) = self.expand_module(path, &mut fallback_module) {
                                warn!("  Fallback expansion also failed: {}", expand_err);
                                return Err(expand_err);
                            }
                            self.modules.insert(path.clone(), Arc::new(fallback_module));
                        }
                    }
                }
            }
        }

        info!(
            "  Staged compilation complete: {} modules, {} cache hits, {} meta executions",
            module_paths.len(),
            total_cache_hits,
            total_meta_executions
        );

        // PASS 2.5: Compute Item Hashes for Incremental Compilation
        // This enables fine-grained change detection (signature vs body changes).
        // Incremental: item-level hashes distinguish signature vs body changes.
        for path in &module_paths {
            if let Some(module_rc) = self.modules.get(path) {
                let hashes = compute_item_hashes_from_module(module_rc.as_ref());
                let file_path = PathBuf::from(path.as_str());
                self.incremental_compiler.update_item_hashes(file_path, hashes);
            }
        }
        debug!(
            "  Computed item hashes for {} modules (incremental: {} cached)",
            module_paths.len(),
            self.incremental_compiler.stats().item_hashes_cached
        );

        // 6. PASS 3: Type Checking with Import Resolution
        self.current_pass = CompilerPass::Pass3Analysis;
        info!("Pass 3: Type checking phase");

        // Track total types inferred across all modules
        let mut total_types_inferred = 0;

        // Create shared inherent_methods map for order-independent method resolution
        // This enables methods registered in one module's implement blocks to be
        // visible to all other modules, regardless of compilation order.
        // Order-independent method resolution: methods in implement blocks are shared
        // across all modules regardless of compilation order.
        let shared_inherent_methods = Shared::new(parking_lot::RwLock::new(
            Map::new(),
        ));

        for path in &module_paths {
            // Skip stdlib modules from full type checking - they only need their
            // declarations registered (done via stdlib_modules below), not their
            // bodies checked. Checking stdlib bodies produces false errors because
            // the stdlib has known internal issues that don't affect user code.
            if path.as_str().starts_with("core") {
                continue;
            }

            debug!("  Type checking: {}", path.as_str());
            // Clone Arc (cheap) to release borrow before calling mutable method
            let module_rc = self.modules.get(path).map(Arc::clone);
            if let Some(module) = module_rc {
                // Create type checker with shared inherent_methods for cross-module visibility
                let mut checker = TypeChecker::with_shared_methods(shared_inherent_methods.clone());

                // Register built-in types and functions
                checker.register_builtins();

                // Configure type checker with module registry for cross-file resolution
                // This enables imports to resolve types from other modules
                let registry = self.session.module_registry();
                checker.set_module_registry(registry.clone());

                // Configure lazy resolver for on-demand module loading
                // This enables imports to trigger module loading if not already loaded
                checker.set_lazy_resolver(self.lazy_resolver.clone());

                // Register cross-file contexts (protocols and contexts from other modules)
                // This enables `using [Database, Auth]` to work when defined elsewhere
                // Register cross-file contexts so `using [Database, Auth]` resolves across files.
                for context_name in &self.collected_contexts {
                    checker.register_protocol_as_context(context_name.clone());
                }

                // Multi-pass type checking:
                // Pass 0: Process imports to register imported types and functions
                // This must happen before type declarations to handle cross-file types
                // Cross-module name resolution: process imports before type declarations.
                for item in &module.items {
                    if let verum_ast::ItemKind::Mount(import) = &item.kind {
                        if let Err(type_error) =
                            checker.process_import(import, path.as_str(), &registry.read())
                        {
                            let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                            self.session.emit_diagnostic(diag);
                        }
                    }
                }

                // Pass 0: Pre-register all inline modules
                // This enables cross-module imports even when modules are declared after
                // the modules that import from them.
                // Pre-register inline modules for order-independent cross-module imports.
                for item in &module.items {
                    if let verum_ast::ItemKind::Module(module_decl) = &item.kind {
                        checker.pre_register_module_public(module_decl, "cog");
                    }
                }

                // ═══════════════════════════════════════════════════════════════
                // PRE-PASS: Register stdlib module declarations into type checker.
                // Without this, the TypeChecker only has built-in primitives and
                // cannot resolve stdlib types referenced in user code.
                // ═══════════════════════════════════════════════════════════════
                // Sort for deterministic iteration (self.modules is a HashMap).
                // Shallower (fewer-dot) module keys come first so top-level stdlib
                // functions beat nested-module helpers when short names collide.
                let mut stdlib_entries: Vec<_> = self.modules.iter()
                    .filter(|(k, _)| k.as_str().starts_with("core"))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                stdlib_entries.sort_by(|(a, _), (b, _)| {
                    let depth_a = a.as_str().matches('.').count();
                    let depth_b = b.as_str().matches('.').count();
                    depth_a.cmp(&depth_b).then_with(|| a.as_str().cmp(b.as_str()))
                });

                // Preserve the user-file module path so we can restore it after
                // stdlib registration transiently rebinds it per-module.
                let saved_module_path = checker.current_module_path().clone();

                if !stdlib_entries.is_empty() {
                    if self.stdlib_metadata.is_none() {
                        // S0a: Register stdlib type names (module-scoped so the
                        // fully-qualified `{mod}.{name}` key is populated and
                        // same-named stdlib types don't collide on flat lookup).
                        for (module_path, stdlib_mod) in &stdlib_entries {
                            checker.set_current_module_path(module_path.clone());
                            checker.register_all_type_names(&stdlib_mod.items);
                        }
                        // S0b: Resolve stdlib type definitions
                        let mut resolution_stack = List::new();
                        for (module_path, stdlib_mod) in &stdlib_entries {
                            checker.set_current_module_path(module_path.clone());
                            for item in &stdlib_mod.items {
                                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                                    let _ = checker.resolve_type_definition(type_decl, &mut resolution_stack);
                                }
                            }
                        }
                        // S1: Register stdlib function signatures
                        for (module_path, stdlib_mod) in &stdlib_entries {
                            checker.set_current_module_path(module_path.clone());
                            for item in &stdlib_mod.items {
                                if let verum_ast::ItemKind::Function(func) = &item.kind {
                                    if !checker.is_function_preregistered(func.name.name.as_str()) {
                                        let _ = checker.register_function_signature(func);
                                    }
                                }
                            }
                        }
                        // S2: Register stdlib protocols
                        for (module_path, stdlib_mod) in &stdlib_entries {
                            checker.set_current_module_path(module_path.clone());
                            for item in &stdlib_mod.items {
                                if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                                    let _ = checker.register_protocol(protocol_decl);
                                }
                            }
                        }
                    }

                    // S3: ALWAYS register stdlib impl blocks (module-scoped so
                    // method signatures reference the *declaring* module's
                    // same-named types).
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        for item in &stdlib_mod.items {
                            if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                                let _ = checker.register_impl_block(impl_decl);
                            }
                        }
                    }
                }

                // Restore the user-file module path so subsequent passes
                // (user type / impl / function registration) run in the right
                // resolution scope.
                checker.set_current_module_path(saved_module_path);

                // Signal transition to user code phase
                checker.set_user_code_phase();

                // Pass 1: Register type declarations
                for item in &module.items {
                    if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                        if let Err(e) = checker.register_type_declaration(type_decl) {
                            let diag = type_error_to_diagnostic(&e, Some(self.session));
                            self.session.emit_diagnostic(diag);
                        }
                    }
                }

                // Pass 2: Register protocol declarations
                for item in &module.items {
                    if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                        if let Err(e) = checker.register_protocol(protocol_decl) {
                            let diag = type_error_to_diagnostic(&e, Some(self.session));
                            self.session.emit_diagnostic(diag);
                        }
                    }
                }

                // Pass 3: Register protocol implementations
                for item in &module.items {
                    if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                        if let Err(e) = checker.register_impl_block(impl_decl) {
                            let diag = type_error_to_diagnostic(&e, Some(self.session));
                            self.session.emit_diagnostic(diag);
                        }
                    }
                }

                // Pass 4: Register function signatures (enables forward references)
                for item in &module.items {
                    if let verum_ast::ItemKind::Function(func) = &item.kind {
                        if let Err(e) = checker.register_function_signature(func) {
                            let diag = type_error_to_diagnostic(&e, Some(self.session));
                            self.session.emit_diagnostic(diag);
                        }
                    }
                }

                // Pass 4.5: Register extern function signatures (FFI)
                for item in &module.items {
                    if let verum_ast::ItemKind::ExternBlock(extern_block) = &item.kind {
                        for func in &extern_block.functions {
                            if let Err(e) = checker.register_function_signature(func) {
                                let diag = type_error_to_diagnostic(&e, Some(self.session));
                                self.session.emit_diagnostic(diag);
                            }
                        }
                    }
                }

                // Pass 4.6: Pre-register const and static declarations (enables forward references)
                // Constants and statics defined after functions should still be visible in function bodies.
                for item in &module.items {
                    if let verum_ast::ItemKind::Const(const_decl) = &item.kind {
                        checker.pre_register_const(const_decl);
                    }
                    if let verum_ast::ItemKind::Static(static_decl) = &item.kind {
                        // Pre-register static variable type for forward reference resolution
                        if let Ok(ty) = checker.ast_to_type(&static_decl.ty) {
                            checker.pre_register_static(&static_decl.name.name, ty);
                        }
                    }
                }

                // Enable lenient context validation for files with @test annotations.
                let has_test_annotation = module.items.iter().any(|item| {
                    item.attributes.iter().any(|attr| attr.is_named("test"))
                }) || module.items.first().and_then(|item| {
                    self.session.get_source(item.span.file_id)
                }).map(|sf| {
                    sf.source.as_str().lines().take(10).any(|line| {
                        let trimmed = line.trim();
                        trimmed.starts_with("// @test:") || trimmed.starts_with("// @test ")
                    })
                }).unwrap_or(false);
                if has_test_annotation {
                    checker.context_resolver_mut().set_lenient_contexts(true);
                }

                // Pass 5: Type check all items
                for item in &module.items {
                    if let Err(type_error) = checker.check_item(item) {
                        let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                        self.session.emit_diagnostic(diag);
                    }
                }

                // Accumulate metrics
                let metrics = checker.metrics();
                total_types_inferred += metrics.synth_count + metrics.check_count;

                debug!(
                    "  Completed {} ({} synth, {} check, {} unify)",
                    path.as_str(),
                    metrics.synth_count,
                    metrics.check_count,
                    metrics.unify_count
                );
            }
        }

        let elapsed = start.elapsed();

        // Calculate diagnostics
        let final_error_count = self.session.error_count();
        let final_warning_count = self.session.warning_count();
        let new_errors = final_error_count - initial_error_count;
        let new_warnings = final_warning_count - initial_warning_count;

        // user_errors counts only errors from project files, not from
        // stdlib. Stdlib errors are tracked SEPARATELY in
        // `self.stdlib_errors` (a Vec local to the pipeline) and
        // intentionally never enter the session's diagnostic pool —
        // see `phase0_stdlib::run_phase0_stdlib_for_module` and the
        // `phases/phase0_stdlib.rs` paths that push to
        // `self.stdlib_errors` without calling `emit_diagnostic`.
        //
        // So `final_error_count - initial_error_count` already counts
        // ONLY user-file errors (the session sees just those). The
        // previous hardcoded `user_errors: 0` was a bug that silently
        // swallowed every user diagnostic: `check.rs:66` reads
        // `result.user_errors` to gate diagnostic display, so even
        // when `result.errors > 0`, the detailed error text never
        // reached the terminal because user_errors was 0.
        let user_errors = new_errors;

        // Create result
        let result = CheckResult {
            files_checked: project_files.len(),
            types_inferred: total_types_inferred,
            warnings: new_warnings,
            errors: new_errors,
            user_errors,
            elapsed,
        };

        info!(
            "Type checking completed: {} file(s), {} type(s) in {:.2}s",
            result.files_checked,
            result.types_inferred,
            elapsed.as_secs_f64()
        );

        if result.errors > 0 {
            warn!("Type checking found {} error(s)", result.errors);
        } else {
            info!("Type checking succeeded with no errors");
        }

        Ok(result)
    }
}
