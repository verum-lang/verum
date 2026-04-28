//! Compilation Session Management
//!
//! Manages state across compilation phases including source files,
//! diagnostics, caches, and compiler options.
//!
//! Compilation session: holds compiler options, diagnostics, module registry,
//! and file/module state for the duration of a single compilation run.

use anyhow::Result;
use parking_lot::RwLock;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use verum_ast::{CfgEvaluator, FileId, Module, SourceFile, TargetConfig};
use verum_common::{List, Map, Shared, Text};
use verum_diagnostics::{Diagnostic, Emitter, EmitterConfig};
use verum_modules::{ModuleId, ModuleLoader, ModuleRegistry};

use crate::compilation_metrics::CompilationProfileReport;
use crate::options::{CompilerOptions, OutputFormat};
use verum_cbgr::tier_analysis::TierAnalysisResult;
use verum_cbgr::tier_types::TierStatistics;

/// Function identifier for tier analysis caching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub u64);

/// A compilation session that tracks all state during compilation
///
/// Type-erased script-permission policy. Wraps a boxed `Fn` whose
/// signature exactly matches `PermissionRouter::set_policy`. Has a
/// `Debug` stub so the surrounding `Session` can keep its derived
/// `Debug` impl intact — the closure body itself is opaque to
/// debug printing, only its presence is reported.
pub struct ScriptPermissionPolicy(
    pub  Box<
        dyn Fn(
                verum_vbc::interpreter::permission::PermissionScope,
                u64,
            ) -> verum_vbc::interpreter::permission::PermissionDecision
            + Send
            + Sync
            + 'static,
    >,
);

impl std::fmt::Debug for ScriptPermissionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<script permission policy>")
    }
}

/// Module structure: modules organize code into hierarchical namespaces,
/// with file system mapping (lib.vr=root, foo.vr=module, foo/bar.vr=child).
#[derive(Debug)]
pub struct Session {
    /// Compiler options
    options: CompilerOptions,

    /// Source files indexed by FileId
    source_files: Shared<RwLock<Map<FileId, Shared<SourceFile>>>>,

    /// Parsed modules (cached)
    modules: Shared<RwLock<Map<FileId, Shared<Module>>>>,

    /// Module registry for multi-file projects
    /// Module registry: stores module exports for cross-file name resolution.
    module_registry: Shared<RwLock<ModuleRegistry>>,

    /// Diagnostics accumulated during compilation
    diagnostics: Shared<RwLock<List<Diagnostic>>>,

    /// Next available FileId (atomic for lock-free allocation).
    ///
    /// Wrapped in `Shared` so that ModuleLoaders created via
    /// `create_module_loader` hand out a single monotonic FileId
    /// sequence consistent with `Session::next_file_id()`. Without
    /// this, the session and its loaders would own *independent*
    /// counters that could collide on the same FileId value.
    next_file_id: Shared<AtomicU32>,

    /// Has any error been emitted? (atomic for lock-free access)
    has_errors: AtomicBool,

    /// Compilation metrics and profiling data
    metrics: Shared<RwLock<CompilationProfileReport>>,

    /// Start time of compilation (for total duration calculation)
    compilation_start: Instant,

    /// Tier analysis cache for reference tier decisions.
    ///
    /// Stores the results of tier analysis for each function, indexed by
    /// FunctionId. The codegen phase uses this cache to determine which references
    /// can be promoted from Tier 0 (~15ns) to Tier 1 (0ns).
    ///
    /// CBGR analysis results from escape analysis (tier promotion decisions).
    tier_analysis_cache: Shared<RwLock<Map<FunctionId, TierAnalysisResult>>>,

    /// Global tier statistics across all analyzed functions.
    tier_statistics: Shared<RwLock<TierStatistics>>,

    /// Configuration evaluator for @cfg conditional compilation.
    ///
    /// This evaluator is initialized from CompilerOptions and used to filter
    /// items with @cfg attributes during compilation.
    ///
    /// Conditional compilation: platform-specific and feature-gated code paths.
    cfg_evaluator: CfgEvaluator,

    /// Cross-cog module resolver (loaded from Verum.lock or Verum.toml path dependencies).
    /// When set, ModuleLoaders created by this session will dispatch cross-cog imports.
    cog_resolver: Option<verum_modules::cog_resolver::CogResolver>,

    /// Shared SMT routing statistics.
    ///
    /// Populated by verification phases that dispatch through
    /// `verum_smt::SmtBackendSwitcher`. The CLI reads these at the end
    /// of compilation to drive `verum smt-stats` (persisted to
    /// `$VERUM_STATE_DIR/smt-stats.json`). The field is always present —
    /// phases that don't use SMT simply leave it empty.
    ///
    /// `Arc` so the switcher inside each phase can hold an owned handle
    /// without moving the session.
    routing_stats: std::sync::Arc<verum_smt::routing_stats::RoutingStats>,

    /// Capture slot for the VBC module produced by the most recent
    /// compilation. Populated by `phase_interpret_with_args` after
    /// `compile_ast_to_vbc` succeeds, so the script-mode runner can
    /// serialise the result into the persistent script cache without
    /// re-running the pipeline.
    ///
    /// `None` until the first compile-and-run succeeds; `Some` after.
    /// `Arc` so callers receive a cheap clone of the same module the
    /// interpreter just executed — no double-allocation, no re-encode.
    last_compiled_vbc:
        Shared<RwLock<Option<std::sync::Arc<verum_vbc::module::VbcModule>>>>,

    /// Script-mode permission policy installed by the CLI runner.
    ///
    /// `Some(closure)` when the entry source is a script and the CLI
    /// has built a policy from the script's resolved `PermissionSet`
    /// (frontmatter ∪ CLI flags). The policy is a function from
    /// `(scope, target_id)` → `Allow | Deny`.
    ///
    /// The pipeline transfers this into the `VbcInterpreter`'s
    /// `PermissionRouter` immediately after constructing the
    /// interpreter, so subsequent intrinsic dispatches (raw syscalls,
    /// gated FFI calls, opt-in `check_permission` calls in stdlib)
    /// hit the script's policy on cache miss.
    ///
    /// `None` for project-mode runs and any single-file run that
    /// isn't a script — those keep the router's default allow-all
    /// behaviour, matching pre-script-mode semantics.
    ///
    /// Boxed-closure type chosen to match `PermissionRouter::set_policy`'s
    /// `F: Fn(...) + Send + Sync + 'static` bound exactly.
    script_permission_policy: Shared<RwLock<Option<ScriptPermissionPolicy>>>,

    /// Process exit code requested by the most recent execution.
    ///
    /// `None` for `()` / `nil` / non-Int returns (CLI exits 0).
    /// `Some(n)` for `Int` / `Bool` returns (CLI exits with `n`).
    ///
    /// Writing this field instead of calling `std::process::exit`
    /// from inside the pipeline lets the CLI run post-execution work
    /// — persisting the script cache, flushing telemetry, printing
    /// timings — *before* the OS terminates the process. Without
    /// this indirection a script that ends in `42` would `exit(42)`
    /// from inside the interpreter, never reaching the cache-store
    /// step, and the next invocation would re-pay the full
    /// compile cost.
    pending_exit_code: Shared<RwLock<Option<i32>>>,
}

impl Session {
    /// Create a new compilation session.
    ///
    /// Applies cross-cutting feature reconciliations once at startup
    /// (e.g., disabling SMT verification when refinement types are
    /// turned off) so downstream phases see a consistent view of the
    /// language-feature set.
    pub fn new(mut options: CompilerOptions) -> Self {
        Self::reconcile_language_features(&mut options);

        // Build TargetConfig from CompilerOptions
        let target_config = Self::build_target_config(&options);
        let cfg_evaluator = CfgEvaluator::with_config(target_config);

        Self {
            options,
            source_files: Shared::new(RwLock::new(Map::new())),
            modules: Shared::new(RwLock::new(Map::new())),
            module_registry: Shared::new(RwLock::new(ModuleRegistry::new())),
            diagnostics: Shared::new(RwLock::new(List::new())),
            next_file_id: Shared::new(AtomicU32::new(0)),
            has_errors: AtomicBool::new(false),
            metrics: Shared::new(RwLock::new(CompilationProfileReport::new())),
            compilation_start: Instant::now(),
            tier_analysis_cache: Shared::new(RwLock::new(Map::new())),
            tier_statistics: Shared::new(RwLock::new(TierStatistics::default())),
            cfg_evaluator,
            cog_resolver: None,
            routing_stats: std::sync::Arc::new(verum_smt::routing_stats::RoutingStats::new()),
            last_compiled_vbc: Shared::new(RwLock::new(None)),
            pending_exit_code: Shared::new(RwLock::new(None)),
            script_permission_policy: Shared::new(RwLock::new(None)),
        }
    }

    /// Record the VBC module produced by the most recent compilation.
    /// Called by the pipeline immediately after `compile_ast_to_vbc`
    /// succeeds. Overwrites any prior recording — the slot reflects
    /// the latest run.
    pub fn record_compiled_vbc(
        &self,
        module: std::sync::Arc<verum_vbc::module::VbcModule>,
    ) {
        *self.last_compiled_vbc.write() = Some(module);
    }

    /// Take the most recently compiled VBC module, leaving `None` in
    /// the slot. Used by the script-mode runner after a successful
    /// `run_interpreter` to serialise the result into the persistent
    /// script cache. Returns `None` if no compilation captured a VBC
    /// module — e.g., a `--check` run that never reached codegen.
    pub fn take_compiled_vbc(
        &self,
    ) -> Option<std::sync::Arc<verum_vbc::module::VbcModule>> {
        self.last_compiled_vbc.write().take()
    }

    /// Record an OS exit code requested by the most recent execution.
    /// Pipeline code paths that surface a script's tail value (or a
    /// `fn main() -> Int` return) call this; the CLI driver reads it
    /// after post-run housekeeping and translates to
    /// `std::process::exit`.
    pub fn record_exit_code(&self, code: i32) {
        *self.pending_exit_code.write() = Some(code);
    }

    /// Take the pending exit code, leaving `None` in the slot.
    /// Returns `None` for runs that ended with `()` / `nil` /
    /// non-numeric return — the CLI maps those to exit 0.
    pub fn take_exit_code(&self) -> Option<i32> {
        self.pending_exit_code.write().take()
    }

    /// Install a script-mode permission policy. Called by the CLI
    /// runner immediately after building the `Session` and before
    /// the pipeline reaches `phase_interpret_with_args`. The policy
    /// is transferred into the `VbcInterpreter`'s `PermissionRouter`
    /// after the interpreter is constructed; subsequent intrinsic
    /// dispatches consult the script's grants on cache miss.
    ///
    /// Replacing an existing policy is supported but rare — the
    /// expected lifecycle is at-most-once per script run.
    pub fn set_script_permission_policy(&self, policy: ScriptPermissionPolicy) {
        *self.script_permission_policy.write() = Some(policy);
    }

    /// Take the script-mode permission policy, leaving `None` in the
    /// slot. The pipeline calls this after constructing the
    /// interpreter so it can transfer the policy into the router.
    /// Subsequent calls return `None` until a new policy is
    /// installed — there is no replay.
    pub fn take_script_permission_policy(&self) -> Option<ScriptPermissionPolicy> {
        self.script_permission_policy.write().take()
    }

    /// Set the cross-cog resolver for external package imports.
    pub fn set_cog_resolver(&mut self, resolver: verum_modules::cog_resolver::CogResolver) {
        self.cog_resolver = Some(resolver);
    }

    /// Borrow the cross-cog resolver, if one is installed. Used by the
    /// pipeline's `load_external_cog_modules` phase to walk every
    /// registered cog's filesystem root and pre-register its modules
    /// in the session's module registry — same machinery as
    /// `load_project_modules`, sourced from the resolver instead of
    /// the manifest.
    pub fn cog_resolver(&self) -> Option<&verum_modules::cog_resolver::CogResolver> {
        self.cog_resolver.as_ref()
    }

    /// Convenience accessor for the unified language-feature set.
    ///
    /// Equivalent to `self.options().language_features`, but callers that
    /// only need to query features shouldn't have to drag in the full
    /// `CompilerOptions` import.
    pub fn language_features(
        &self,
    ) -> &crate::language_features::LanguageFeatures {
        &self.options.language_features
    }

    /// Apply feature-driven reconciliations that cross-cut the options
    /// struct. Called from `Session::new*` constructors so that no
    /// caller can bypass them.
    ///
    /// Current reconciliations:
    ///   1. If `types.refinement` is disabled, the refinement-type SMT
    ///      path is a no-op — downgrade `verify_mode` to `Runtime` so
    ///      the pipeline doesn't spin up a solver for nothing.
    ///   2. If `codegen.proof_erasure` is disabled, `debug.show_erased_proofs`
    ///      becomes moot but is otherwise harmless (no action).
    fn reconcile_language_features(opts: &mut CompilerOptions) {
        // 1. Refinement off → no SMT solver needed.
        if !opts.language_features.refinement_typing_on()
            && opts.verify_mode.use_smt()
        {
            opts.verify_mode = crate::options::VerifyMode::Runtime;
        }
        // 2. [codegen].debug_info → CompilerOptions.debug_info boolean.
        match opts.language_features.codegen.debug_info.as_str() {
            "none" => opts.debug_info = false,
            "line" | "full" => opts.debug_info = true,
            _ => {}
        }
        // 3. [runtime].panic → recorded for codegen. "abort" uses
        //    abort() (core dump), "unwind" uses _exit(1) (clean exit).
        //    The value is read by LLVM codegen from the session's
        //    language_features when emitting panic blocks.
    }

    /// Access the shared SMT routing statistics handle.
    ///
    /// Phase code clones this `Arc` when constructing an
    /// `SmtBackendSwitcher`, so all verification work in a session
    /// shares a single stats collector. The CLI calls
    /// `.as_json()` / `.report()` on the returned handle after
    /// compilation completes.
    pub fn routing_stats(&self) -> &std::sync::Arc<verum_smt::routing_stats::RoutingStats> {
        &self.routing_stats
    }

    /// Replace the routing-stats handle (used by test harnesses that
    /// need to inject a pre-populated collector).
    pub fn set_routing_stats(
        &mut self,
        stats: std::sync::Arc<verum_smt::routing_stats::RoutingStats>,
    ) {
        self.routing_stats = stats;
    }

    /// Register external cog dependencies from lockfile data.
    ///
    /// Each entry is (name, version, root_path). This is a convenience method
    /// so CLI code doesn't need to depend on verum_modules directly.
    pub fn register_cog_dependencies(&mut self, deps: Vec<(String, String, std::path::PathBuf)>) {
        if deps.is_empty() { return; }
        let mut resolver = verum_modules::cog_resolver::CogResolver::new();
        for (name, version, root_path) in deps {
            resolver.register_cog(name, version, root_path);
        }
        self.cog_resolver = Some(resolver);
    }

    /// Create a new compilation session with a pre-populated module registry.
    ///
    /// This is an optimization for test performance: instead of re-registering
    /// ~166 stdlib modules for every test (~500ms), tests can pass a deep_clone
    /// of a cached registry (~1ms).
    ///
    /// # Arguments
    ///
    /// * `options` - Compiler options for this session
    /// * `registry` - Pre-populated module registry (typically a deep_clone of the stdlib cache)
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Get cached registry (returns None if not yet populated)
    /// if let Some(registry) = get_cached_stdlib_registry() {
    ///     let session = Session::with_registry(options, registry);
    ///     // session now has stdlib modules pre-loaded
    /// }
    /// ```
    pub fn with_registry(mut options: CompilerOptions, registry: ModuleRegistry) -> Self {
        Self::reconcile_language_features(&mut options);

        let target_config = Self::build_target_config(&options);
        let cfg_evaluator = CfgEvaluator::with_config(target_config);

        Self {
            options,
            source_files: Shared::new(RwLock::new(Map::new())),
            modules: Shared::new(RwLock::new(Map::new())),
            module_registry: Shared::new(RwLock::new(registry)),
            diagnostics: Shared::new(RwLock::new(List::new())),
            next_file_id: Shared::new(AtomicU32::new(0)),
            has_errors: AtomicBool::new(false),
            metrics: Shared::new(RwLock::new(CompilationProfileReport::new())),
            compilation_start: Instant::now(),
            tier_analysis_cache: Shared::new(RwLock::new(Map::new())),
            tier_statistics: Shared::new(RwLock::new(TierStatistics::default())),
            cfg_evaluator,
            cog_resolver: None,
            routing_stats: std::sync::Arc::new(verum_smt::routing_stats::RoutingStats::new()),
            last_compiled_vbc: Shared::new(RwLock::new(None)),
            pending_exit_code: Shared::new(RwLock::new(None)),
            script_permission_policy: Shared::new(RwLock::new(None)),
        }
    }

    /// Build a TargetConfig from CompilerOptions
    ///
    /// Parses target_triple or detects host platform, then applies
    /// custom features and flags from options.
    fn build_target_config(options: &CompilerOptions) -> TargetConfig {
        // Start with host platform detection or parse target triple
        let mut config = if let Some(ref triple) = options.target_triple {
            TargetConfig::for_target(triple.as_str())
        } else {
            TargetConfig::host()
        };

        // Apply debug_assertions based on optimization level
        config.debug_assertions = options.optimization_level == 0;

        // Apply test mode
        config.test = options.test_mode;

        // Add enabled features
        for feature in &options.cfg_features {
            config.features.push(feature.clone());
        }

        // Add custom key-value pairs
        for (key, value) in &options.cfg_custom {
            config.custom.insert(key.clone(), value.clone());
        }

        config
    }

    /// Get compiler options
    pub fn options(&self) -> &CompilerOptions {
        &self.options
    }

    /// Get mutable compiler options (for pipeline-detected flags like GPU kernels)
    pub fn options_mut(&mut self) -> &mut CompilerOptions {
        &mut self.options
    }

    /// Get the cfg evaluator for conditional compilation.
    ///
    /// Use this to check if items with @cfg attributes should be included
    /// in compilation based on the current target configuration.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let item = // ... item with @cfg(unix) attribute
    /// if session.cfg_evaluator().should_include(&item) {
    ///     // Process the item
    /// }
    /// ```
    pub fn cfg_evaluator(&self) -> &CfgEvaluator {
        &self.cfg_evaluator
    }

    /// Get the current target configuration
    pub fn target_config(&self) -> &TargetConfig {
        self.cfg_evaluator.config()
    }

    /// Load a source file and return its FileId
    pub fn load_file(&self, path: &Path) -> Result<FileId> {
        // Check if already loaded
        let sources = self.source_files.read();
        for (id, source) in sources.iter() {
            if let Some(ref existing_path) = source.path {
                if existing_path == path {
                    return Ok(*id);
                }
            }
        }
        drop(sources);

        // Allocate new FileId
        let file_id = self.allocate_file_id();

        // Create SourceFile from path
        let source_file = SourceFile::from_path(file_id, path.to_path_buf())?;

        // Register with global registry for error diagnostics
        verum_common::register_source_file(file_id, path.display().to_string(), source_file.source.as_str());

        // Store it
        self.source_files
            .write()
            .insert(file_id, Shared::new(source_file));

        Ok(file_id)
    }

    /// Load source code from a string (for testing and REPL)
    pub fn load_source_string(&self, source_code: &str, virtual_path: PathBuf) -> Result<FileId> {
        // Allocate new FileId
        let file_id = self.allocate_file_id();

        // Create SourceFile from string
        let mut source_file = SourceFile::new(
            file_id,
            virtual_path.to_string_lossy().to_string(),
            source_code.to_string(),
        );
        source_file.path = Some(virtual_path.clone());

        // Register with global registry for error diagnostics
        verum_common::register_source_file(file_id, virtual_path.display().to_string(), source_code);

        // Store it
        self.source_files
            .write()
            .insert(file_id, Shared::new(source_file));

        Ok(file_id)
    }

    /// Get source file by ID
    pub fn get_source(&self, file_id: FileId) -> Option<Shared<SourceFile>> {
        self.source_files.read().get(&file_id).cloned()
    }

    /// Add a parsed module to the cache
    pub fn cache_module(&self, file_id: FileId, module: Module) {
        self.modules.write().insert(file_id, Shared::new(module));
    }

    /// Get cached module
    pub fn get_module(&self, file_id: FileId) -> Option<Shared<Module>> {
        self.modules.read().get(&file_id).cloned()
    }

    /// Emit a diagnostic
    pub fn emit_diagnostic(&self, diagnostic: Diagnostic) {
        let is_error = diagnostic.severity() == verum_diagnostics::Severity::Error;

        self.diagnostics.write().push(diagnostic);

        if is_error {
            // Use atomic store for lock-free error flag update
            self.has_errors.store(true, Ordering::Relaxed);
        }
    }

    /// Emit multiple diagnostics (batched for efficiency)
    pub fn emit_diagnostics(&self, diagnostics: List<Diagnostic>) {
        if diagnostics.is_empty() {
            return;
        }

        // Check if any diagnostic is an error before taking the lock
        let has_error = diagnostics
            .iter()
            .any(|d| d.severity() == verum_diagnostics::Severity::Error);

        // Single write lock for all diagnostics (batch insert)
        {
            let mut diags = self.diagnostics.write();
            for diag in diagnostics {
                diags.push(diag);
            }
        }

        if has_error {
            self.has_errors.store(true, Ordering::Relaxed);
        }
    }

    /// Get all diagnostics
    pub fn diagnostics(&self) -> List<Diagnostic> {
        self.diagnostics.read().clone()
    }

    /// Clear all diagnostics
    pub fn clear_diagnostics(&self) {
        self.diagnostics.write().clear();
        self.has_errors.store(false, Ordering::Relaxed);
    }

    /// Check if any errors have been emitted
    pub fn has_errors(&self) -> bool {
        self.has_errors.load(Ordering::Relaxed)
    }

    /// Get number of errors
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .read()
            .iter()
            .filter(|d| d.severity() == verum_diagnostics::Severity::Error)
            .count()
    }

    /// Get number of warnings
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .read()
            .iter()
            .filter(|d| d.severity() == verum_diagnostics::Severity::Warning)
            .count()
    }

    /// Render and display all diagnostics
    pub fn display_diagnostics(&self) -> Result<()> {
        let config = EmitterConfig {
            format: match self.options.output_format {
                OutputFormat::Human => verum_diagnostics::OutputFormat::Human,
                OutputFormat::Json => verum_diagnostics::OutputFormat::Json,
            },
            colors: self.options.use_color(),
            show_source: true,
            context_lines: 2,
        };

        let mut emitter = Emitter::new(config);
        let diagnostics = self.diagnostics();
        // Write diagnostics to stderr (not stdout) so they can be captured by test runners
        let stderr = io::stderr();
        let mut handle = stderr.lock();

        for diagnostic in diagnostics {
            emitter.emit(&diagnostic, &mut handle)?;
        }

        Ok(())
    }

    /// Format all diagnostics to a string (for testing)
    pub fn format_diagnostics(&self) -> String {
        let config = EmitterConfig {
            format: verum_diagnostics::OutputFormat::Human,
            colors: false,
            show_source: true,
            context_lines: 2,
        };

        let mut emitter = Emitter::new(config);
        let diagnostics = self.diagnostics();
        let mut output = Vec::new();

        for diagnostic in diagnostics {
            let _ = emitter.emit(&diagnostic, &mut output);
        }

        String::from_utf8_lossy(&output).to_string()
    }

    /// Abort compilation if errors exist
    pub fn abort_if_errors(&self) -> Result<()> {
        if self.has_errors() {
            self.display_diagnostics()?;
            anyhow::bail!("compilation failed with {} error(s)", self.error_count());
        }
        Ok(())
    }

    /// Get source file content for a span
    pub fn source_content(&self, file_id: FileId) -> Option<Text> {
        self.get_source(file_id).map(|s| s.source.clone())
    }

    /// Allocate a new FileId
    fn allocate_file_id(&self) -> FileId {
        // Use atomic fetch_add for lock-free file ID allocation
        let id = self.next_file_id.fetch_add(1, Ordering::Relaxed);
        FileId::new(id)
    }

    /// Get all source files
    pub fn all_sources(&self) -> List<Shared<SourceFile>> {
        self.source_files
            .read()
            .iter()
            .map(|(_, v)| v.clone())
            .collect::<List<_>>()
            .into()
    }

    /// Get statistics about the session
    pub fn stats(&self) -> SessionStats {
        SessionStats {
            num_files: self.source_files.read().len(),
            num_modules: self.modules.read().len(),
            num_errors: self.error_count(),
            num_warnings: self.warning_count(),
        }
    }

    /// Create a module loader for the session.
    ///
    /// Uses the input directory (or file's parent) as the root path for module resolution.
    ///
    /// Module loader initialized from session root path for file-to-module mapping.
    pub fn create_module_loader(&self) -> ModuleLoader {
        // When input is a directory, use it directly as the root
        // When input is a file, use its parent directory
        let root_path = if self.options.input.is_dir() {
            self.options.input.clone()
        } else if let Some(parent) = self.options.input.parent() {
            parent.to_path_buf()
        } else {
            PathBuf::from(".")
        };
        let mut loader = ModuleLoader::new(root_path);
        // Wire loader to the session's unified FileId and ModuleId
        // allocators. Without this, each loader owns its own counters
        // and IDs from independent loaders collide.
        loader.set_file_id_allocator(self.next_file_id.clone());
        loader.set_module_id_allocator(
            self.module_registry.read().id_allocator(),
        );
        // Attach cross-cog resolver if available (from Verum.lock)
        if let Some(ref resolver) = self.cog_resolver {
            loader.set_cog_resolver(resolver.clone());
        }
        loader
    }

    /// Hand out the Session's ModuleId allocator handle so secondary
    /// loaders (e.g. pipeline-side lazy_resolver) can join the same
    /// monotonic sequence.
    pub fn module_id_allocator(&self) -> Shared<AtomicU32> {
        self.module_registry.read().id_allocator()
    }

    /// Hand out the Session's FileId allocator handle.
    pub fn file_id_allocator(&self) -> Shared<AtomicU32> {
        self.next_file_id.clone()
    }

    /// Get access to the module registry.
    ///
    /// Module registry: stores module exports for cross-file name resolution.
    pub fn module_registry(&self) -> Shared<RwLock<ModuleRegistry>> {
        self.module_registry.clone()
    }

    /// Register a module in the module registry.
    ///
    /// Module registry: stores module exports for cross-file name resolution.
    pub fn register_module(&self, module_info: verum_modules::ModuleInfo) -> ModuleId {
        self.module_registry.write().register(module_info)
    }

    /// Get a module by ID from the registry.
    ///
    /// Module registry: stores module exports for cross-file name resolution.
    pub fn get_module_by_id(&self, id: ModuleId) -> Option<Shared<verum_modules::ModuleInfo>> {
        self.module_registry.read().get(id).into()
    }

    /// Get a module by path from the registry.
    ///
    /// Module registry: stores module exports for cross-file name resolution.
    pub fn get_module_by_path(&self, path: &str) -> Option<Shared<verum_modules::ModuleInfo>> {
        self.module_registry.read().get_by_path(path).into()
    }

    /// Discover all .vr files in a directory tree.
    ///
    /// This enables multi-file project compilation.
    ///
    /// Module loader initialized from session root path for file-to-module mapping.
    pub fn discover_project_files(&self) -> Result<List<PathBuf>> {
        // When input is a directory (like 'src/'), search inside it directly
        // When input is a file, search from its parent directory
        let root_path = if self.options.input.is_dir() {
            self.options.input.clone()
        } else if let Some(parent) = self.options.input.parent() {
            parent.to_path_buf()
        } else {
            PathBuf::from(".")
        };

        let mut verum_files = List::new();
        self.discover_files_recursive(&root_path, &mut verum_files)?;
        Ok(verum_files)
    }

    /// Recursively discover .vr files in a directory.
    /// SAFETY: Uses depth limit to prevent stack overflow on deep directory structures
    fn discover_files_recursive(&self, dir: &Path, files: &mut List<PathBuf>) -> Result<()> {
        const MAX_DEPTH: usize = 100; // Prevent stack overflow
        self.discover_files_recursive_impl(dir, files, 0, MAX_DEPTH)
    }

    /// Internal implementation with depth tracking
    fn discover_files_recursive_impl(
        &self,
        dir: &Path,
        files: &mut List<PathBuf>,
        current_depth: usize,
        max_depth: usize,
    ) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }

        // Depth limit to prevent stack overflow
        if current_depth >= max_depth {
            tracing::debug!(
                "Maximum directory depth ({}) reached at {:?}",
                max_depth,
                dir
            );
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip symlinks to prevent infinite loops
            if path.is_symlink() {
                continue;
            }

            if path.is_dir() {
                // Recursively search subdirectories with incremented depth
                self.discover_files_recursive_impl(&path, files, current_depth + 1, max_depth)?;
            } else if let Some(ext) = path.extension() {
                if ext == "vr" {
                    files.push(path);
                }
            }
        }

        Ok(())
    }

    /// Convert AST Span (byte offsets) to Diagnostic Span (line/column).
    ///
    /// This method performs efficient conversion using the cached line start
    /// information in SourceFile. If the source file is not found, it returns
    /// a placeholder span to ensure diagnostics can still be displayed.
    ///
    /// # Performance
    ///
    /// - O(log n) lookup via binary search on line starts
    /// - < 1ms per conversion (typically ~100ns)
    /// - No allocations for cache hits
    ///
    /// # Arguments
    ///
    /// * `ast_span` - The byte-offset span from AST
    ///
    /// # Returns
    ///
    /// A LineColSpan with 1-indexed line/column numbers for diagnostic display.
    /// Returns placeholder "<unknown>:1:1" if source file not found.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let file_id = session.load_file(Path::new("test.vr"))?;
    /// let ast_span = Span::new(0, 10, file_id);
    /// let diag_span = session.convert_span(ast_span);
    /// assert_eq!(diag_span.file, "test.vr");
    /// ```
    pub fn convert_span(&self, ast_span: verum_ast::Span) -> verum_diagnostics::Span {
        // Handle dummy/synthetic spans gracefully
        if ast_span.is_dummy() {
            return verum_diagnostics::Span::new("<generated>", 1, 1, 1);
        }

        // Look up the source file
        let source_files = self.source_files.read();
        let source_file = match source_files.get(&ast_span.file_id) {
            Some(sf) => sf,
            None => {
                // Fallback: source file not in session (shouldn't happen in normal flow)
                return verum_diagnostics::Span::new(
                    &format!("<unknown file {:?}>", ast_span.file_id),
                    1,
                    1,
                    1,
                );
            }
        };

        // Get the display name - use path if available, otherwise use name
        let display_name: String = if let Some(ref path) = source_file.path {
            path.display().to_string()
        } else {
            source_file.name.clone().into_string()
        };

        // Convert using SourceFile's efficient line lookup
        match source_file.span_to_line_col(ast_span) {
            Some(mut line_col_span) => {
                // Update the file name to use the full path
                line_col_span.file = display_name.into();
                line_col_span
            }
            None => {
                // Fallback: span doesn't match file (shouldn't happen)
                verum_diagnostics::Span::new(&display_name, 1, 1, 1)
            }
        }
    }

    // ==================== METRICS API ====================

    /// Record a compilation phase execution with timing and memory info
    ///
    /// This method is used to track performance of individual compilation phases
    /// for profiling and optimization purposes.
    ///
    /// # Arguments
    ///
    /// * `phase_name` - Name of the phase (e.g., "Lexing", "Parsing", "Type Checking")
    /// * `duration` - Time taken to execute the phase
    /// * `memory_allocated` - Memory allocated during this phase (in bytes)
    pub fn record_phase_metrics(
        &self,
        phase_name: &str,
        duration: Duration,
        memory_allocated: usize,
    ) {
        self.metrics
            .write()
            .record_phase(Text::from(phase_name), duration, memory_allocated);
    }

    /// Get individual phase timings as (name, duration) pairs.
    pub fn get_phase_timings(&self) -> Vec<(String, Duration)> {
        let metrics = self.metrics.read();
        metrics
            .phase_metrics
            .iter()
            .map(|p| (p.phase_name.to_string(), p.duration))
            .collect()
    }

    /// Record module compilation metrics
    ///
    /// Tracks individual module compilation for identifying slow modules.
    pub fn record_module_metrics(
        &self,
        module_name: &str,
        duration: Duration,
        function_count: usize,
    ) {
        self.metrics
            .write()
            .add_module(Text::from(module_name), duration, function_count);
    }

    /// Get the total compilation duration since session start
    pub fn total_duration(&self) -> Duration {
        self.compilation_start.elapsed()
    }

    /// Finalize metrics and get the complete profiling report
    ///
    /// This should be called at the end of compilation to:
    /// - Calculate percentages
    /// - Detect bottlenecks
    /// - Generate summary statistics
    ///
    /// Returns a cloned copy of the finalized metrics report.
    pub fn finalize_metrics(&self) -> CompilationProfileReport {
        let mut metrics = self.metrics.write();
        metrics.total_duration = self.compilation_start.elapsed();
        metrics.finalize();
        metrics.clone()
    }

    /// Get current (unfinalized) metrics
    ///
    /// Returns a snapshot of the current metrics without finalization.
    /// Useful for progress reporting during long compilations.
    pub fn current_metrics(&self) -> CompilationProfileReport {
        self.metrics.read().clone()
    }

    /// Get phase-specific metrics for populating BuildMetrics
    ///
    /// Returns durations for common phases (parse, typecheck, codegen, etc.)
    /// for backward compatibility with CLI's BuildMetrics struct.
    pub fn get_build_metrics(&self) -> BuildMetrics {
        let metrics = self.metrics.read();
        let mut parse_time = Duration::ZERO;
        let mut typecheck_time = Duration::ZERO;
        let mut codegen_time = Duration::ZERO;
        let mut optimization_time = Duration::ZERO;
        let mut link_time = Duration::ZERO;
        let mut total_lines = 0;

        // Extract phase-specific durations
        for phase in &metrics.phase_metrics {
            let name = phase.phase_name.as_str().to_lowercase();
            if name.contains("pars") || name.contains("lex") || name.contains("stdlib")
                || name.contains("project module") || name.contains("dependency")
            {
                parse_time += phase.duration;
            } else if name.contains("type") || name.contains("semantic")
                || name.contains("cbgr") || name.contains("verif")
            {
                typecheck_time += phase.duration;
            } else if name.contains("codegen") || name.contains("code gen") {
                codegen_time += phase.duration;
            } else if name.contains("optim") || name.contains("mono") {
                optimization_time += phase.duration;
            } else if name.contains("link") {
                link_time += phase.duration;
            }
        }

        // Calculate total lines from modules
        for module in &metrics.module_metrics {
            total_lines += module.lines_of_code;
        }

        BuildMetrics {
            parse_time,
            typecheck_time,
            codegen_time,
            optimization_time,
            link_time,
            total_lines,
        }
    }

    // ==================== TIER ANALYSIS CACHE API ====================

    /// Store tier analysis result in the session cache.
    ///
    /// Called by the tier analysis phase to cache tier decisions for later
    /// use by the codegen phase.
    ///
    /// # Arguments
    ///
    /// * `function_id` - Unique identifier for the function
    /// * `result` - The analysis result containing tier decisions for all references
    ///
    /// CBGR analysis results from escape analysis (tier promotion decisions).
    pub fn cache_tier_analysis(&self, function_id: FunctionId, result: TierAnalysisResult) {
        self.tier_analysis_cache.write().insert(function_id, result);
    }

    /// Get cached tier analysis result for a function.
    ///
    /// Called by the codegen phase to retrieve tier decisions for references.
    ///
    /// # Arguments
    ///
    /// * `function_id` - Unique identifier for the function
    ///
    /// # Returns
    ///
    /// The cached analysis result if available, None otherwise.
    pub fn get_tier_analysis(&self, function_id: FunctionId) -> Option<TierAnalysisResult> {
        self.tier_analysis_cache.read().get(&function_id).cloned()
    }

    /// Check if tier analysis exists for a function.
    pub fn has_tier_analysis(&self, function_id: FunctionId) -> bool {
        self.tier_analysis_cache.read().contains_key(&function_id)
    }

    /// Merge tier statistics from another analysis.
    ///
    /// Called after analyzing functions to accumulate statistics across
    /// all functions in the compilation unit.
    pub fn merge_tier_statistics(&self, stats: &TierStatistics) {
        self.tier_statistics.write().merge(stats);
    }

    /// Get a copy of the current tier statistics.
    pub fn tier_statistics(&self) -> TierStatistics {
        self.tier_statistics.read().clone()
    }

    /// Get all cached tier analysis results.
    ///
    /// Returns a cloned map of all function analysis results. Useful for
    /// bulk codegen operations.
    pub fn all_tier_analyses(&self) -> Map<FunctionId, TierAnalysisResult> {
        self.tier_analysis_cache.read().clone()
    }

    /// Clear the tier analysis cache.
    ///
    /// Useful for incremental compilation when functions are recompiled.
    pub fn clear_tier_cache(&self) {
        self.tier_analysis_cache.write().clear();
        *self.tier_statistics.write() = TierStatistics::default();
    }

    /// Get the number of functions with cached tier analysis.
    pub fn tier_cache_size(&self) -> usize {
        self.tier_analysis_cache.read().len()
    }
}

/// Build metrics for backward compatibility with CLI
#[derive(Debug, Clone, Default)]
pub struct BuildMetrics {
    /// Time spent parsing source files
    pub parse_time: Duration,

    /// Time spent type checking
    pub typecheck_time: Duration,

    /// Time spent generating code (LLVM IR / bytecode)
    pub codegen_time: Duration,

    /// Time spent on optimization passes
    pub optimization_time: Duration,

    /// Time spent linking (for AOT mode)
    pub link_time: Duration,

    /// Total lines of code compiled
    pub total_lines: usize,
}

/// Statistics about a compilation session
#[derive(Debug, Clone)]
pub struct SessionStats {
    /// Number of source files loaded
    pub num_files: usize,

    /// Number of modules parsed
    pub num_modules: usize,

    /// Number of errors
    pub num_errors: usize,

    /// Number of warnings
    pub num_warnings: usize,
}

impl SessionStats {
    /// Display statistics
    pub fn display(&self) -> Text {
        format!(
            "Session: {} files, {} modules, {} errors, {} warnings",
            self.num_files, self.num_modules, self.num_errors, self.num_warnings
        ).into()
    }
}
