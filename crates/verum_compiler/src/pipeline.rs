//! Compilation Pipeline Orchestration
//!
//! Implements the End-to-End Compiler Architecture (Tier 0-2):
//!
//! **Tier 0: Frontend**
//! - Lexing (verum_lexer)
//! - Parsing (verum_fast_parser)
//! - Type checking (verum_types)
//! - CBGR analysis (verum_cbgr)
//!
//! **Tier 1: Analysis**
//! - Refinement checking (verum_smt)
//! - Context resolution
//! - Meta function expansion (meta_registry)
//!
//! **Tier 2: Backend**
//! - Module system (verum_modules)
//! - VBC codegen (verum_vbc::codegen)
//! - VBC interpretation (verum_vbc::interpreter)
//!
//! This architecture enables:
//! - Cross-file meta function resolution
//! - Compile-time code generation
//! - Direct interpretation and execution
//!
//! Implementation follows a dependency-driven roadmap: Tier 0 foundation
//! (lexer, parser, AST, types, CBGR, interpreter), then Tier 1 value-proof
//! features (protocols, refinement codegen, SMT verification, gradual verification,
//! context system, code generation, stdlib integration). The pipeline orchestrates
//! all compilation phases from source to executable.

use anyhow::{Context as AnyhowContext, Result};
use colored::Colorize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};
use verum_ast::{FileId, Item, Module, SourceFile, Span, decl::ItemKind};
use verum_common::{List, Map, Text};
use verum_diagnostics::{DiagnosticBuilder, Severity};
// VBC-first architecture imports
use verum_vbc::codegen::{VbcCodegen, CodegenConfig};
use verum_vbc::interpreter::Interpreter as VbcInterpreter;

// VBC → LLVM IR lowering (CPU compilation path)
use verum_codegen::llvm::{
    VbcToLlvmLowering, LoweringConfig as LlvmLoweringConfig,
    LoweringStats as LlvmLoweringStats,
};

// Compilation path analysis
use crate::compilation_path::{
    CompilationPath, TargetConfig as PathTargetConfig,
    analyze_function, determine_compilation_path,
};
use verum_lexer::Lexer;
use verum_modules::{
    ModuleId, ModuleLoader, ModuleInfo, ModulePath, ModuleRegistry, SharedModuleResolver,
    extract_exports_from_module, resolve_glob_reexports, resolve_specific_reexport_kinds,
};
// CoherenceChecker / ImplEntry now used only inside
// crate::pipeline::coherence (#106 Phase 6).
// LanguageProfile / ProfileChecker now used only inside
// crate::pipeline::profile_boundaries (#106 Phase 7).
use verum_fast_parser::VerumParser;
use verum_smt::{Context as SmtContext, CostTracker};
// RefinementVerifier / SubsumptionChecker / SubsumptionConfig / VerificationError /
// VerifyMode now used only inside crate::pipeline::refinement_verify (#106 Phase 4).
// Note: Gradual verification is now handled by phases::verification_phase
// See: BoundsCheckEliminator, CBGROptimizer, VerificationPipeline
use verum_common::{Maybe, Shared};
use verum_types::TypeChecker;

use crate::linker_config::ProjectConfig;
use crate::meta::MetaRegistry;
use crate::options::VerifyMode;
// IntrinsicDiagnostics / IntrinsicLint / module_utils now used only inside
// crate::pipeline::stdlib_bootstrap (#106 Phase 8).
use crate::phases::linking::{FinalLinker, LinkingConfig, ObjectFile};
use crate::phases::phase0_stdlib::{Phase0CoreCompiler, StdlibArtifacts};
use crate::phases::ExecutionTier;
use crate::phases::type_error_to_diagnostic;
use crate::session::Session;
use crate::core_cache::global_cache_or_init;
use crate::core_source::CoreSource;
use crate::core_compiler::{CoreConfig, StdlibModuleResolver};
// StdlibCompilationResult / StdlibModule now used only inside
// crate::pipeline::stdlib_bootstrap (#106 Phase 8).
use crate::hash::compute_item_hashes_from_module;
use crate::incremental_compiler::IncrementalCompiler;
use crate::staged_pipeline::{StagedPipeline, StagedConfig};

// Phase-specific submodule extractions (#106 — pipeline.rs split).
// Each submodule is a sibling file under `pipeline/` declaring an
// additional `impl<'s> CompilationPipeline<'s>` block (or a set of
// pure free helpers).  Sibling-file submodules can access this
// crate's `pub(crate)` surface via `super::*`, so private fields
// of `CompilationPipeline` remain genuinely private — only methods
// move out of this file, not access boundaries.
mod bounds_stats;
mod cbgr;
mod coherence;
mod compile_orchestration;
mod dispatch;
mod impl_axioms;
mod interpreter;
mod loading;
mod mlir;
mod native_codegen;
mod phase0;
mod profile_boundaries;
mod refinement_verify;
mod stdlib_bootstrap;
mod theorem_proofs;
mod vbc_codegen;

// ═══════════════════════════════════════════════════════════════════════════
// GLOBAL STDLIB MODULE CACHE
// ═══════════════════════════════════════════════════════════════════════════
//
// Caches parsed stdlib modules at the process level so that multiple
// CompilationPipeline instances (e.g., one per test file) don't each re-parse
// the 166+ stdlib .vr files. The cache stores Arc<Module> so they can be
// shared across threads and pipeline instances.
//
// Cache key: workspace root path (canonicalized)
// Cache value: Vec<(module_path, Arc<Module>)> + ModuleRegistry entries
// ═══════════════════════════════════════════════════════════════════════════

/// Cached parsed stdlib modules for process-level reuse.
/// Stores parsed Module ASTs and source text so subsequent pipeline instances
/// skip file I/O and parsing (the expensive part of stdlib loading).
pub(super) struct CachedStdlibModules {
    /// (module_path_string, parsed_module, source_text) triples
    pub(super) entries: Vec<(Text, Module, Text)>,
}

static GLOBAL_STDLIB_MODULES: std::sync::OnceLock<std::sync::RwLock<Option<CachedStdlibModules>>> =
    std::sync::OnceLock::new();

pub(super) fn global_stdlib_cache() -> &'static std::sync::RwLock<Option<CachedStdlibModules>> {
    GLOBAL_STDLIB_MODULES.get_or_init(|| std::sync::RwLock::new(None))
}

/// Cached fully-populated ModuleRegistry for process-level reuse.
///
/// This is the key optimization for test performance: instead of re-registering
/// ~166 stdlib modules for every test (taking ~400-600ms), we cache the fully
/// populated registry and deep_clone it for each pipeline instance.
///
/// The registry stores:
/// - All stdlib modules registered with their exports
/// - Glob re-exports resolved
/// - Module path to ID mappings
static GLOBAL_STDLIB_REGISTRY: std::sync::OnceLock<std::sync::RwLock<Option<ModuleRegistry>>> =
    std::sync::OnceLock::new();

pub(super) fn global_stdlib_registry_cache() -> &'static std::sync::RwLock<Option<ModuleRegistry>> {
    GLOBAL_STDLIB_REGISTRY.get_or_init(|| std::sync::RwLock::new(None))
}

/// Get a deep clone of the cached stdlib registry, or None if not yet populated.
///
/// This is the primary entry point for test executors to get a pre-populated
/// registry without re-registering stdlib modules.
///
/// Uses RwLock for concurrent read access — multiple compilation pipelines
/// can clone the registry simultaneously without blocking each other.
pub fn get_cached_stdlib_registry() -> Option<ModuleRegistry> {
    let cache = global_stdlib_registry_cache();
    let guard = cache.read().unwrap_or_else(|poisoned| {
        tracing::warn!("stdlib registry cache RwLock was poisoned, recovering");
        poisoned.into_inner()
    });
    guard.as_ref().map(|r| r.deep_clone())
}

/// Clear all process-level global caches to reclaim memory.
///
/// Install the canonical set of module-path aliases into the registry.
///
/// MOD-CRIT-1 (audit): without this, the type-resolver hosted a
/// hardcoded alias table at `crates/verum_types/src/infer.rs::
/// get_module_with_path_aliases`, creating an INDEPENDENT canonical-
/// path map that could drift from the loader's `module_path_to_id`.
/// All alias decisions now flow through `ModuleRegistry::path_aliases`
/// — a single source of truth owned by the registry.
///
/// The set installed here mirrors the prior hardcoded table: legacy
/// `std.*` → `core.*` aliases, semantic shorthand (`core.memory` →
/// `core.base.memory`), platform-specific resolution (`core.thread`
/// → `core.sys.{darwin,linux,windows}.thread`), and channel-vs-mpsc
/// dispatch.
///
/// User code can register additional aliases via
/// `ModuleRegistry::register_path_alias(...)` for project-local
/// overrides; the registry probes user aliases AFTER the canonical
/// path so this baseline cannot accidentally shadow user choices.
fn install_canonical_module_aliases(registry: &mut ModuleRegistry) {
    // Semantic shorthand aliases — let user code address well-known
    // modules under intuitive short names without having to memorise
    // their exact submodule path.
    registry.register_path_alias("core.memory",  "core.base.memory");
    registry.register_path_alias("core.maybe",   "core.base.maybe");
    registry.register_path_alias("core.result",  "core.base.result");
    registry.register_path_alias("core.process", "core.io.process");
    registry.register_path_alias("core.string",  "core.text.text");
    registry.register_path_alias("core.text",    "core.text.text");
    registry.register_path_alias("core.list",    "core.collections.list");
    registry.register_path_alias("core.map",     "core.collections.map");
    registry.register_path_alias("core.set",     "core.collections.set");
    // Channel module alias (tests use core.sync.mpsc but it's
    // core.async.channel under the canonical layout).
    registry.register_path_alias("core.sync.mpsc", "core.async.channel");
    // Platform-dependent thread module: resolve to host platform.
    let platform_thread: Option<&str> = if cfg!(target_os = "macos") {
        Some("core.sys.darwin.thread")
    } else if cfg!(target_os = "linux") {
        Some("core.sys.linux.thread")
    } else if cfg!(target_os = "windows") {
        Some("core.sys.windows.thread")
    } else {
        None
    };
    if let Some(canonical) = platform_thread {
        registry.register_path_alias("core.thread", canonical);
    }
}

/// Call this between test batches or when memory pressure is high.
/// The caches will be lazily re-populated on the next compilation.
pub fn clear_global_caches() {
    {
        let cache = global_stdlib_cache();
        if let Ok(mut guard) = cache.write() {
            *guard = None;
        }
    }
    {
        let cache = global_stdlib_registry_cache();
        if let Ok(mut guard) = cache.write() {
            *guard = None;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PERSISTENT STDLIB REGISTRY CACHE (DISK)
// ═══════════════════════════════════════════════════════════════════════════
//
// Caches the fully-populated ModuleRegistry to disk so that separate process
// invocations (e.g., repeated `verum run` commands) skip the ~500ms stdlib
// parse + registration. Uses blake3 content hashing for invalidation.
//
// Cache location: target/.verum-cache/stdlib/registry_<hash>.bin
// Format: bincode-serialized SerializableRegistryCache
// ═══════════════════════════════════════════════════════════════════════════

/// Serializable representation of a ModuleRegistry for disk caching.
///
/// ModuleRegistry contains `Shared<ModuleInfo>` (custom Arc) which doesn't
/// implement Serialize. This wrapper extracts the data into plain types.
#[derive(serde::Serialize, serde::Deserialize)]
struct SerializableRegistryCache {
    /// Cache format version — bump on breaking changes
    format_version: u32,
    /// Compiler version that produced this cache
    compiler_version: String,
    /// LLVM version that produced this cache — bincode-serialized layouts
    /// can depend on LLVM types via transitive `ModuleInfo` fields.
    llvm_version: String,
    /// Blake3 hash of all core/*.vr file contents
    content_hash: String,
    /// Module entries: (module_id_u32, module_info)
    modules: Vec<(u32, ModuleInfo)>,
    /// Path-to-ID mapping
    path_to_id: Vec<(String, u32)>,
}

/// Current cache format version. Bump when SerializableRegistryCache changes.
/// Bumped to 2 after adding llvm_version field.
/// Bumped to 3 after switching the on-disk format from bincode 1 to CBOR
/// (ciborium). Any pre-existing `registry.bin` from older compilers will
/// fail to parse and fall through to the full stdlib load.
const REGISTRY_CACHE_FORMAT_VERSION: u32 = 3;

/// Compute blake3 content hash of all .vr files under a directory.
pub(super) fn compute_stdlib_content_hash(stdlib_path: &Path) -> String {
    let mut hasher = blake3::Hasher::new();

    // Collect all .vr files sorted for deterministic hashing
    let mut files = Vec::new();
    collect_vr_files(stdlib_path, &mut files, 0);
    files.sort();

    for path in &files {
        // Hash relative path for location independence
        if let Ok(rel) = path.strip_prefix(stdlib_path) {
            hasher.update(rel.to_string_lossy().as_bytes());
            hasher.update(b"\x00");
        }
        // Hash file content
        if let Ok(content) = std::fs::read(path) {
            hasher.update(&content);
            hasher.update(b"\x00");
        }
    }

    hasher.finalize().to_hex().to_string()
}

/// Recursively collect .vr files (for hashing).
fn collect_vr_files(dir: &Path, files: &mut Vec<PathBuf>, depth: usize) {
    if depth >= 10 || !dir.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            if path.is_dir() {
                let dir_name = path.file_name().map(|n| n.to_string_lossy());
                if dir_name.as_deref() != Some("examples") {
                    collect_vr_files(&path, files, depth + 1);
                }
            } else if path.extension().is_some_and(|ext| ext == "vr") {
                files.push(path);
            }
        }
    }
}

/// Get the disk cache directory for stdlib registry.
fn stdlib_cache_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join("target").join(".verum-cache").join("stdlib")
}

/// Try to load a cached ModuleRegistry from disk.
///
/// Returns `Some(registry)` if a valid cache exists matching the given content hash.
pub(super) fn try_load_registry_from_disk(
    workspace_root: &Path,
    content_hash: &str,
) -> Option<ModuleRegistry> {
    let cache_dir = stdlib_cache_dir(workspace_root);
    let cache_file = cache_dir.join("registry.bin");

    if !cache_file.exists() {
        return None;
    }

    let data = std::fs::read(&cache_file).ok()?;
    // CBOR (via ciborium) instead of bincode 1 — see the note in
    // verum_compiler/Cargo.toml. Skip quietly on any parse error so a
    // stale cache from a previous compiler version falls through to the
    // full stdlib load path instead of crashing startup.
    let cached: SerializableRegistryCache =
        ciborium::de::from_reader(std::io::Cursor::new(&data)).ok()?;

    // Validate cache
    if cached.format_version != REGISTRY_CACHE_FORMAT_VERSION {
        debug!("Stdlib disk cache: format version mismatch ({} vs {})",
            cached.format_version, REGISTRY_CACHE_FORMAT_VERSION);
        return None;
    }
    if cached.compiler_version != env!("CARGO_PKG_VERSION") {
        debug!("Stdlib disk cache: compiler version mismatch ({} vs {})",
            cached.compiler_version, env!("CARGO_PKG_VERSION"));
        return None;
    }
    if cached.llvm_version != verum_codegen::llvm::LLVM_VERSION {
        debug!("Stdlib disk cache: LLVM version mismatch ({} vs {})",
            cached.llvm_version, verum_codegen::llvm::LLVM_VERSION);
        return None;
    }
    if cached.content_hash != content_hash {
        debug!("Stdlib disk cache: content hash mismatch");
        return None;
    }

    // Reconstruct ModuleRegistry from serialized data
    let mut registry = ModuleRegistry::new();
    for (id, info) in cached.modules {
        let module_id = ModuleId::new(id);
        let mut info = info;
        info.id = module_id;
        registry.register(info);
    }

    Some(registry)
}

/// Save a ModuleRegistry to disk cache.
pub(super) fn save_registry_to_disk(
    workspace_root: &Path,
    registry: &ModuleRegistry,
    content_hash: &str,
) {
    let cache_dir = stdlib_cache_dir(workspace_root);
    if std::fs::create_dir_all(&cache_dir).is_err() {
        debug!("Failed to create stdlib cache directory");
        return;
    }

    // Convert registry to serializable form. Sort by module path so the
    // cache file content is byte-stable across processes — the `Map`
    // backing `all_modules()` is a HashMap, so raw iteration leaks the
    // per-process random hasher seed into the on-disk cache. Without
    // this, the same workspace produces a different cache file on every
    // run, and downstream consumers that walk `modules` in stored
    // order assign different FunctionIds / TypeIds — the documented
    // VBC-nondeterminism bug class.
    let mut modules: Vec<(u32, ModuleInfo)> = Vec::new();
    let mut path_to_id: Vec<(String, u32)> = Vec::new();
    let mut entries: Vec<(String, u32, verum_modules::ModuleInfo)> = registry
        .all_modules()
        .map(|(id, info)| (info.path.to_string(), id.as_u32(), (**info).clone()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, id, info) in entries {
        modules.push((id, info));
        path_to_id.push((path, id));
    }

    let cached = SerializableRegistryCache {
        format_version: REGISTRY_CACHE_FORMAT_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        llvm_version: verum_codegen::llvm::LLVM_VERSION.to_string(),
        content_hash: content_hash.to_string(),
        modules,
        path_to_id,
    };

    let cache_file = cache_dir.join("registry.bin");
    let mut data: Vec<u8> = Vec::new();
    match ciborium::ser::into_writer(&cached, &mut data) {
        Ok(()) => {
            if let Err(e) = std::fs::write(&cache_file, &data) {
                debug!("Failed to write stdlib registry cache: {}", e);
            } else {
                info!("Saved stdlib registry cache ({:.1} MB) to {}",
                    data.len() as f64 / 1_048_576.0,
                    cache_file.display());
            }
        }
        Err(e) => {
            debug!("Failed to serialize stdlib registry: {}", e);
        }
    }
}

/// The current compiler pass
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilerPass {
    /// Pass 1: Scan and register all meta functions and macros
    Pass1Registration,

    /// Pass 2: Execute meta functions and expand macros
    Pass2Expansion,

    /// Pass 3: Semantic analysis (type checking, borrow checking, etc.)
    Pass3Analysis,
}

/// Compilation mode (execution strategy)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationMode {
    /// Just-in-time compilation (execute immediately)
    Jit,
    /// Ahead-of-time compilation (produce executable)
    Aot,
    /// Check only (no code generation)
    Check,
    /// Interpret (use interpreter, no LLVM)
    Interpret,
    /// MLIR-based JIT compilation (experimental)
    /// Uses AST → Verum MLIR dialect → LLVM dialect → JIT
    MlirJit,
    /// MLIR-based AOT compilation (experimental)
    /// Uses AST → Verum MLIR dialect → LLVM dialect → object file
    MlirAot,
}

/// Build mode determines how the compilation pipeline handles type registration
/// and module resolution.
///
/// This enables unified handling of both stdlib bootstrap compilation and normal
/// user code compilation within the same `CompilationPipeline`.
///
/// # Design Rationale
///
/// The key difference between stdlib and normal compilation is:
/// - **Stdlib Bootstrap**: Global type registration across ALL modules before compiling any
/// - **Normal**: Incremental per-file compilation with pre-loaded stdlib types
///
/// By abstracting this into `BuildMode`, we achieve:
/// - Single pipeline implementation (DRY principle)
/// - Clear separation of mode-specific behavior (SRP)
/// - Easy extension for future modes (OCP)
#[derive(Debug, Clone)]
pub enum BuildMode {
    /// Normal user code compilation.
    ///
    /// Loads pre-compiled stdlib from `stdlib.vbca` and compiles user code
    /// incrementally on a per-file basis.
    Normal,

    /// Stdlib bootstrap compilation.
    ///
    /// Uses global type registration across ALL stdlib modules before compiling
    /// any module. This eliminates cross-module dependency constraints.
    ///
    /// Flow:
    /// 1. Discover all modules via `StdlibModuleResolver`
    /// 2. Parse ALL modules
    /// 3. Register ALL types globally (multi-pass)
    /// 4. Compile each module to VBC
    /// 5. Output `stdlib.vbca` archive
    StdlibBootstrap {
        /// Configuration for stdlib compilation
        config: CoreConfig,
    },
}

impl Default for BuildMode {
    fn default() -> Self {
        BuildMode::Normal
    }
}

// SmtCheckResult moved to crate::pipeline::refinement_verify
// (#106 Phase 4 — pipeline.rs split).

/// Result of a check-only compilation
#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Number of files checked
    pub files_checked: usize,
    /// Number of types inferred
    pub types_inferred: usize,
    /// Number of warnings emitted
    pub warnings: usize,
    /// Number of errors emitted (total including stdlib)
    pub errors: usize,
    /// Number of errors in user code only (excluding stdlib/core)
    pub user_errors: usize,
    /// Total compilation time
    pub elapsed: std::time::Duration,
}

impl CheckResult {
    /// Create a success result
    pub fn success(
        files_checked: usize,
        types_inferred: usize,
        elapsed: std::time::Duration,
    ) -> Self {
        Self {
            files_checked,
            types_inferred,
            warnings: 0,
            errors: 0,
            user_errors: 0,
            elapsed,
        }
    }

    /// Check if compilation succeeded (no errors)
    pub fn is_ok(&self) -> bool {
        self.errors == 0
    }

    /// Get a summary string
    pub fn summary(&self) -> String {
        format!(
            "Checked {} file(s) with {} type(s) in {:.2}s - {} error(s), {} warning(s)",
            self.files_checked,
            self.types_inferred,
            self.elapsed.as_secs_f64(),
            self.errors,
            self.warnings
        )
    }
}

/// Result of test execution with captured output.
///
/// This is used by vtest to run tests and capture their output for comparison.
#[derive(Debug, Clone)]
pub struct TestExecutionResult {
    /// Captured stdout output.
    pub stdout: String,
    /// Captured stderr output.
    pub stderr: String,
    /// Exit code (0 for success, non-zero for failure/panic).
    pub exit_code: i32,
    /// Execution duration.
    pub duration: std::time::Duration,
}

impl TestExecutionResult {
    /// Check if execution was successful (exit code 0).
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    /// Check if execution panicked (exit code non-zero).
    pub fn panicked(&self) -> bool {
        self.exit_code != 0
    }

    /// Get the panic message if the execution panicked.
    pub fn panic_message(&self) -> Option<&str> {
        if self.panicked() {
            // Look for panic message in stderr
            if !self.stderr.is_empty() {
                Some(&self.stderr)
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// Compilation pipeline that orchestrates all compiler phases
///
/// Supports both normal user code compilation and stdlib bootstrap compilation
/// through the `BuildMode` abstraction. This unifies the compilation logic
/// while allowing mode-specific behavior for type registration and module resolution.
///
/// Module system: files map to modules (lib.vr = crate root, foo.vr = module foo,
/// foo/bar.vr = module foo.bar). Visibility defaults to private. Name resolution
/// is deterministic and unambiguous via hierarchical module paths.
///
/// Result of the unified [`CompilationPipeline::run`] dispatch.
///
/// Each variant corresponds to a distinct compilation tier: `Checked`
/// is the type-only path (codegen and linking skipped); `Built`
/// carries the path to a freshly-produced native executable.  Future
/// tiers (Tier-0 interpret, MLIR JIT, MLIR AOT) extend this enum —
/// by-value matching at the caller side ensures each new variant is
/// exhaustively handled at every call site.
#[derive(Debug, Clone)]
pub enum RunResult {
    /// `check_only = true` — type-checking succeeded, no output
    /// produced.  Embedders displaying build-completion UI should
    /// emit a "Check OK" message instead of pointing at a binary.
    Checked,
    /// AOT compilation succeeded — the path is the produced native
    /// executable on disk.
    Built(PathBuf),
}

impl RunResult {
    /// Path to the produced binary, or `None` for the check-only
    /// variant.  Convenience for callers that only want the
    /// happy-path build artifact.
    pub fn output_path(&self) -> Option<&Path> {
        match self {
            Self::Checked => None,
            Self::Built(p) => Some(p),
        }
    }

    /// Whether the pipeline ran in check-only mode (no codegen).
    pub fn is_check_only(&self) -> bool {
        matches!(self, Self::Checked)
    }
}

pub struct CompilationPipeline<'s> {
    session: &'s mut Session,

    /// Meta registry for cross-file resolution
    meta_registry: MetaRegistry,

    /// Captured version stamp (#20 / P7). Resolved once at pipeline
    /// construction so the per-meta-call sites (`MetaContext::new()`
    /// at expand_module / @const-block / per-meta-fn) hand back the
    /// same git revision + build-time millisecond stamp without
    /// invoking `git rev-parse HEAD` every time. `None` for either
    /// component when capture failed (no git tree, missing binary,
    /// reproducible-build mode); the `@version_stamp` builtin
    /// substitutes its deterministic fallback in that case.
    cached_version_stamp: (Option<verum_common::Text>, Option<u64>),

    /// Module loader for file system operations
    /// Loads modules from file system following the module-file mapping rules
    /// (foo.vr = module foo, foo/mod.vr = directory module, foo/bar.vr = child).
    module_loader: ModuleLoader,

    /// Lazy module resolver for on-demand module loading during type checking.
    /// Shared across all TypeChecker instances to enable cross-file resolution.
    /// On-demand module loading during type checking for cross-file name resolution.
    lazy_resolver: SharedModuleResolver,

    /// Cached modules (path -> AST module wrapped in Arc for zero-cost sharing)
    modules: Map<Text, Arc<Module>>,

    /// Project modules (sibling .vr files in multi-file projects).
    /// Stored separately so they survive `self.modules.clear()` after type checking.
    project_modules: Map<Text, Arc<Module>>,

    /// Current compiler pass
    current_pass: CompilerPass,

    /// Compilation mode (execution strategy: JIT, AOT, Interpret, etc.)
    mode: CompilationMode,

    /// Build mode (Normal vs StdlibBootstrap)
    ///
    /// Determines how type registration and module resolution work:
    /// - Normal: Per-file incremental compilation with pre-loaded stdlib
    /// - StdlibBootstrap: Global type registration across all modules
    build_mode: BuildMode,

    /// stdlib artifacts from Phase 0 (cached across compilations)
    stdlib_artifacts: Option<StdlibArtifacts>,

    /// Collected context names (protocols and explicit contexts) from all modules.
    /// These are populated during Pass 1.5 and used during Pass 3 to register
    /// cross-file contexts in the TypeChecker.
    /// Cross-file context resolution: context declarations (protocol-based and
    /// explicit `context` blocks) are collected from all parsed modules during
    /// Pass 1.5, then registered in the TypeChecker during Pass 3 so that
    /// `using [Context]` references can resolve across file boundaries.
    collected_contexts: List<Text>,

    /// Type registry from type checking phase
    /// Contains inferred types for expressions, enabling codegen to use
    /// type information for things like closure parameter inference.
    type_registry: Option<verum_types::TypeRegistry>,

    /// Stdlib metadata for NormalBuild mode.
    ///
    /// When set, the type checker uses pre-compiled stdlib types from embedded
    /// stdlib.vbca instead of parsing stdlib source files. This is the preferred
    /// mode for user code compilation.
    ///
    /// Pre-compiled stdlib type metadata from embedded stdlib.vbca archive.
    /// In NormalBuild mode, these types are loaded directly rather than re-parsing
    /// stdlib source, enabling fast compilation of user code.
    stdlib_metadata: Option<std::sync::Arc<verum_types::core_metadata::CoreMetadata>>,

    /// Deferred verification goals drained from the type-checker
    /// after Phase 5. Consumed by the DependentVerifier in
    /// Phase 4.4 of compile_ast_to_vbc.
    deferred_verification_goals: verum_common::List<verum_types::infer::DeferredVerificationGoal>,

    // =========================================================================
    // STDLIB BOOTSTRAP MODE FIELDS
    // =========================================================================
    // These fields are only used when build_mode == BuildMode::StdlibBootstrap

    /// Module resolver for stdlib bootstrap mode.
    /// Discovers modules and computes dependency order via topological sort.
    /// Only populated in StdlibBootstrap mode.
    stdlib_resolver: Option<StdlibModuleResolver>,

    /// Global function registry for cross-module references in stdlib bootstrap.
    /// Accumulates function info across modules (e.g., core's Some/None used by collections).
    /// Only populated in StdlibBootstrap mode.
    global_function_registry: std::collections::HashMap<String, verum_vbc::codegen::FunctionInfo>,

    /// Global protocol registry for cross-module protocol default method inheritance.
    /// Accumulates protocol info across modules (e.g., Eq's default ne method).
    /// Only populated in StdlibBootstrap mode.
    global_protocol_registry: std::collections::HashMap<String, verum_vbc::codegen::ProtocolInfo>,

    /// Compiled VBC modules in stdlib bootstrap mode.
    /// Maps module name to compiled VbcModule for archive building.
    /// Only populated in StdlibBootstrap mode.
    compiled_stdlib_modules: std::collections::HashMap<String, verum_vbc::VbcModule>,

    /// Warnings collected during stdlib compilation.
    /// Only populated in StdlibBootstrap mode.
    stdlib_warnings: List<verum_diagnostics::Diagnostic>,

    /// Errors collected during stdlib compilation.
    /// Only populated in StdlibBootstrap mode when strict_intrinsics is enabled.
    stdlib_errors: List<verum_diagnostics::Diagnostic>,

    // =========================================================================
    // INCREMENTAL COMPILATION SUPPORT
    // =========================================================================

    /// Incremental compiler for fine-grained change detection.
    /// Tracks item-level hashes to distinguish signature vs body changes,
    /// enabling minimal recompilation.
    /// Incremental compilation: tracks item-level hashes to distinguish signature
    /// changes (which invalidate dependents) from body-only changes (which don't).
    /// Phase 2 meta registry can be cached (rarely changes); Phase 3 only re-expands
    /// changed modules; Phases 4+ use standard incremental techniques.
    incremental_compiler: IncrementalCompiler,

    // =========================================================================
    // STAGED METAPROGRAMMING SUPPORT
    // =========================================================================

    /// Staged pipeline for N-level metaprogramming with fine-grained caching.
    ///
    /// Replaces the simple `expand_module()` approach with full N-level staging:
    /// - meta(N) generates meta(N-1) code
    /// - Fine-grained cache invalidation (signature vs body changes)
    /// - Dependency tracking and cascade invalidation
    /// - Cache hit/miss statistics
    ///
    /// N-level metaprogramming: meta(N) generates meta(N-1) code, with
    /// fine-grained cache invalidation (signature vs body changes),
    /// dependency tracking, and cascade invalidation.
    staged_pipeline: StagedPipeline,
}

/// Context for building CFG blocks in escape analysis.
///
/// This struct holds the state needed during CFG construction for
/// a single function, including the block ID allocator, reference
/// counter, and pending blocks to be added to the CFG.
///
/// Visibility: `pub(super)` so the extracted CBGR cluster
/// (`crate::pipeline::cbgr`) can construct + match against it via
/// `super::CfgBuildContext` (#106 Phase 9).
pub(super) struct CfgBuildContext<'a> {
    /// The CFG builder for allocating block and reference IDs.
    pub(super) builder: &'a mut verum_cbgr::CfgBuilder,
    /// Counter for allocating reference IDs.
    pub(super) ref_counter: &'a mut u64,
    /// Entry block ID for the function.
    pub(super) entry_id: verum_cbgr::analysis::BlockId,
    /// Exit block ID for the function.
    pub(super) exit_id: verum_cbgr::analysis::BlockId,
    /// Blocks built during CFG construction, to be added at the end.
    pub(super) pending_blocks: List<verum_cbgr::analysis::BasicBlock>,
    /// Closure captures detected during analysis (ref_id, is_mutable).
    pub(super) closure_captures: List<(verum_cbgr::analysis::RefId, bool)>,
}

// =====================================================================
// Script-mode parse-routing helper
// =====================================================================

/// Decide whether `source` should be parsed in **script mode** (top-level
/// statements allowed, folded into `__verum_script_main`).
///
/// Two independent triggers, OR-joined:
///
/// 1. **Shebang autodetection** — any source whose first bytes are a `#!`
///    line (BOM-tolerant: `EF BB BF #!` is accepted) is a script regardless
///    of CLI invocation form. This makes shebang exec (`./hello.vr`) work
///    without any compiler-options plumbing.
///
/// 2. **Explicit entry-source flag** — `opts.script_mode` enables script
///    mode for the entry source identified by `opts.input`. We compare via
///    canonicalised paths when both sides exist (handles `./hello.vr` vs
///    `/abs/hello.vr` vs `hello.vr`); when canonicalisation fails (file
///    deleted between load and parse), fall back to a literal match. The
///    flag only matches the entry; stdlib and imported modules ignore it,
///    keeping their library-mode parsing untouched.
///
/// The function is allocation-free on the hot path (shebang check is a
/// 5-byte slice comparison).
pub(super) fn should_parse_as_script(
    source: &str,
    opts: &crate::options::CompilerOptions,
    source_path: Option<&std::path::Path>,
) -> bool {
    // (1) Shebang autodetection — BOM-tolerant.
    let bytes = source.as_bytes();
    if bytes.len() >= 2 && &bytes[..2] == b"#!" {
        return true;
    }
    if bytes.len() >= 5 && &bytes[..3] == [0xEF, 0xBB, 0xBF] && &bytes[3..5] == b"#!" {
        return true;
    }

    // (2) Explicit entry-source flag.
    if !opts.script_mode {
        return false;
    }
    let Some(path) = source_path else {
        return false;
    };
    let entry = opts.input.as_path();
    if entry.as_os_str().is_empty() {
        return false;
    }
    if path == entry {
        return true;
    }
    // Canonicalise both sides and retry. When canonicalisation fails for
    // either side (file moved or relative path that no longer resolves),
    // the literal compare above is the only signal — return false rather
    // than over-trigger script mode on stdlib files.
    match (path.canonicalize(), entry.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod script_parse_routing_tests {
    use super::should_parse_as_script;
    use crate::options::CompilerOptions;
    use std::path::PathBuf;

    fn opts(input: &str, flag: bool) -> CompilerOptions {
        CompilerOptions {
            input: PathBuf::from(input),
            script_mode: flag,
            ..Default::default()
        }
    }

    #[test]
    fn shebang_triggers_script_mode_regardless_of_flag() {
        let o = opts("", false);
        assert!(should_parse_as_script("#!/usr/bin/env verum\nprint(1)", &o, None));
        assert!(should_parse_as_script("#!/usr/bin/env verum\nprint(1)", &o, Some(std::path::Path::new("/tmp/x.vr"))));
    }

    #[test]
    fn bom_then_shebang_is_a_script() {
        let bom_shebang = "\u{FEFF}#!/usr/bin/env verum\nprint(1)";
        let o = opts("", false);
        assert!(should_parse_as_script(bom_shebang, &o, None));
    }

    #[test]
    fn no_shebang_no_flag_is_library() {
        let o = opts("/tmp/foo.vr", false);
        assert!(!should_parse_as_script("fn main(){}", &o, Some(std::path::Path::new("/tmp/foo.vr"))));
    }

    #[test]
    fn flag_alone_requires_path_match() {
        let o = opts("/tmp/entry.vr", true);
        assert!(!should_parse_as_script("fn main(){}", &o, None));
        assert!(!should_parse_as_script("fn main(){}", &o, Some(std::path::Path::new("/tmp/other.vr"))));
        assert!(should_parse_as_script("fn main(){}", &o, Some(std::path::Path::new("/tmp/entry.vr"))));
    }

    #[test]
    fn flag_with_empty_input_matches_nothing() {
        let o = opts("", true);
        assert!(!should_parse_as_script("fn main(){}", &o, Some(std::path::Path::new("/tmp/x.vr"))));
    }
}

impl<'s> CompilationPipeline<'s> {
    /// Create a new compilation pipeline for normal user code compilation.
    ///
    /// This is the default mode that loads pre-compiled stdlib and compiles
    /// user code incrementally.
    ///
    /// Initializes ModuleLoader with the session's root path.
    ///
    /// Initializes the pipeline with module loader rooted at the session's root path,
    /// following file-to-module mapping (foo.vr = module foo, foo/mod.vr = directory module).
    pub fn new(session: &'s mut Session) -> Self {
        // Pre-compute incremental cache config before borrowing session
        let incremental = {
            let mut ic = IncrementalCompiler::new();
            if session.options().incremental {
                let cache_dir = session.options().output.parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("target").join("incremental");
                ic.set_cache_dir(cache_dir);
                let _ = ic.load_cache();
            }
            ic
        };
        let module_loader = session.create_module_loader();
        // Non-bootstrap builds still load `core/*` modules lazily on
        // demand. Both forms (`core.foo.bar` and `foo.bar`) must
        // canonicalise to the same key — otherwise the same stdlib
        // file ends up registered twice under two ModuleIds with
        // duplicate exports. Pin cog_name="core" on the lazy
        // resolver; the primary module_loader keeps its inherited
        // setting (user-code imports never use "core." as cog alias).
        let file_id_alloc = session.file_id_allocator();
        let module_id_alloc = session.module_id_allocator();
        let mut lazy_loader = ModuleLoader::new(module_loader.root_path());
        lazy_loader.set_cog_name("core");
        // Wire the same allocator handles the primary loader uses so
        // lazy-resolved modules can never clash with eagerly-loaded
        // ones on FileId or ModuleId.
        lazy_loader.set_file_id_allocator(file_id_alloc);
        lazy_loader.set_module_id_allocator(module_id_alloc);
        let lazy_resolver: SharedModuleResolver = std::sync::Arc::new(
            std::sync::Mutex::new(lazy_loader),
        );
        // Registry also canonicalises by "core" so its dedupe logic
        // matches the lazy resolver's key.
        {
            let registry_handle = session.module_registry();
            let mut reg = registry_handle.write();
            reg.set_cog_name("core");
            // MOD-CRIT-1: install the standard module-path aliases once
            // at startup so the type-resolver does not re-derive them
            // per-call. This is the single funnel point for all alias
            // decisions (loader + resolver + audit CLI all go through
            // ModuleRegistry::get_by_path_aliased).
            install_canonical_module_aliases(&mut reg);
        }
        // Capture git revision + build time once at pipeline
        // construction so per-meta-eval sites don't fork `git
        // rev-parse HEAD` every time. The capture's project-root
        // input is the parent directory of the input source file
        // (or cwd as fallback) — `capture_git_revision` already
        // tolerates empty / nonexistent paths by returning None,
        // which the `@version_stamp` builtin substitutes with the
        // documented empty-string fallback.
        let project_root_for_capture = session
            .options()
            .input
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .unwrap_or_default();
        let cached_version_stamp = (
            crate::meta::subsystems::project_info::capture_git_revision(
                &project_root_for_capture,
            ),
            crate::meta::subsystems::project_info::capture_build_time_unix_ms(),
        );

        Self {
            session,
            meta_registry: MetaRegistry::new(),
            cached_version_stamp,
            module_loader,
            lazy_resolver,
            modules: Map::new(),
            project_modules: Map::new(),
            current_pass: CompilerPass::Pass1Registration,
            mode: CompilationMode::Interpret,
            build_mode: BuildMode::Normal,
            stdlib_artifacts: None,
            collected_contexts: List::new(),
            type_registry: None,
            stdlib_metadata: None,
            deferred_verification_goals: verum_common::List::new(),
            // Stdlib bootstrap mode fields - empty for normal mode
            stdlib_resolver: None,
            global_function_registry: std::collections::HashMap::new(),
            global_protocol_registry: std::collections::HashMap::new(),
            compiled_stdlib_modules: std::collections::HashMap::new(),
            stdlib_warnings: List::new(),
            stdlib_errors: List::new(),
            incremental_compiler: incremental,
            // Staged pipeline for N-level metaprogramming with caching
            staged_pipeline: StagedPipeline::new(StagedConfig {
                max_stage: 2,  // Support meta(2), meta(1), runtime
                enable_caching: true,
                warn_unused_stages: true,
                suggest_stage_downgrade: true,
                lint_config: Default::default(),
            }),
        }
    }

    /// Create a new compilation pipeline for stdlib bootstrap compilation.
    ///
    /// This mode uses global type registration across ALL modules before
    /// compiling any module, eliminating cross-module dependency constraints.
    ///
    /// # Arguments
    ///
    /// * `session` - The compilation session
    /// * `config` - Configuration for stdlib compilation
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_compiler::{Session, CompilationPipeline, CoreConfig};
    ///
    /// let config = CoreConfig::new("stdlib")
    ///     .with_output("target/stdlib.vbca")
    ///     .with_debug_info();
    ///
    /// let mut session = Session::default();
    /// let mut pipeline = CompilationPipeline::new_core(&mut session, config);
    /// let result = pipeline.compile_core()?;
    /// ```
    pub fn new_core(session: &'s mut Session, config: CoreConfig) -> Self {
        let incremental = IncrementalCompiler::new(); // Stdlib bootstrap doesn't use incremental
        let mut module_loader = session.create_module_loader();
        // Stdlib is cog "core": any import that writes `core.foo.bar`
        // is canonicalised to `foo.bar` before filesystem lookup and
        // before the registry compares source_module on re-exports.
        // Without this both forms live as distinct entries and
        // ExportTable raises spurious "Conflicting export" warnings.
        module_loader.set_cog_name("core");
        {
            let registry_handle = session.module_registry();
            let mut reg = registry_handle.write();
            reg.set_cog_name("core");
            // MOD-CRIT-1: same alias-install as the user-pipeline path
            // (line ~804). Stdlib bootstrap also benefits from the
            // shared alias map.
            install_canonical_module_aliases(&mut reg);
        }
        let resolver = StdlibModuleResolver::new(&config.stdlib_path);
        // Create a shared lazy resolver for on-demand module loading.
        // For stdlib bootstrap, uses the stdlib path as the root.
        // Wire FileId + ModuleId allocators to the session so the
        // lazy resolver joins the same monotonic sequence.
        let file_id_alloc = session.file_id_allocator();
        let module_id_alloc = session.module_id_allocator();
        let mut lazy_loader = ModuleLoader::new(&config.stdlib_path);
        lazy_loader.set_cog_name("core");
        lazy_loader.set_file_id_allocator(file_id_alloc);
        lazy_loader.set_module_id_allocator(module_id_alloc);
        let lazy_resolver: SharedModuleResolver = std::sync::Arc::new(
            std::sync::Mutex::new(lazy_loader),
        );

        // #20 / P7 — capture once for stdlib bootstrap path too;
        // see `new()` for rationale.
        let project_root_for_capture = session
            .options()
            .input
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .unwrap_or_default();
        let cached_version_stamp = (
            crate::meta::subsystems::project_info::capture_git_revision(
                &project_root_for_capture,
            ),
            crate::meta::subsystems::project_info::capture_build_time_unix_ms(),
        );

        Self {
            session,
            meta_registry: MetaRegistry::new(),
            cached_version_stamp,
            module_loader,
            lazy_resolver,
            modules: Map::new(),
            project_modules: Map::new(),
            current_pass: CompilerPass::Pass1Registration,
            mode: CompilationMode::Interpret, // VBC interpretation for stdlib
            build_mode: BuildMode::StdlibBootstrap { config },
            stdlib_artifacts: None,
            collected_contexts: List::new(),
            type_registry: None,
            stdlib_metadata: None,
            deferred_verification_goals: verum_common::List::new(), // Not used in bootstrap mode
            // Stdlib bootstrap mode fields - initialized
            stdlib_resolver: Some(resolver),
            global_function_registry: std::collections::HashMap::new(),
            global_protocol_registry: std::collections::HashMap::new(),
            compiled_stdlib_modules: std::collections::HashMap::new(),
            stdlib_warnings: List::new(),
            stdlib_errors: List::new(),
            incremental_compiler: incremental,
            // Staged pipeline for N-level metaprogramming with caching
            // For stdlib bootstrap, use higher max_stage to support complex meta macros
            staged_pipeline: StagedPipeline::new(StagedConfig {
                max_stage: 3,  // Support meta(3), meta(2), meta(1), runtime for stdlib
                enable_caching: true,
                warn_unused_stages: false,  // Stdlib may not use all stages
                suggest_stage_downgrade: false,
                lint_config: Default::default(),
            }),
        }
    }

    /// Create a pipeline for interpreter execution
    pub fn new_interpreter(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::Interpret;
        pipeline
    }

    /// Create a pipeline for check-only mode
    pub fn new_check(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::Check;
        pipeline
    }

    /// Set stdlib metadata for NormalBuild mode.
    ///
    /// When set, the type checker uses pre-compiled stdlib types from embedded
    /// stdlib.vbca instead of parsing stdlib source files. This enables faster
    /// compilation of user code by skipping stdlib parsing.
    ///
    /// # Arguments
    ///
    /// * `metadata` - Pre-compiled stdlib types loaded from stdlib.vbca
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_compiler::core_loader;
    ///
    /// let stdlib_bytes = embedded_stdlib::get_embedded_stdlib().unwrap();
    /// let metadata = core_loader::load_core_metadata_from_bytes(stdlib_bytes)?;
    /// let mut pipeline = CompilationPipeline::new(&mut session);
    /// pipeline.set_stdlib_metadata(std::sync::Arc::new(metadata));
    /// ```
    pub fn set_stdlib_metadata(
        &mut self,
        metadata: std::sync::Arc<verum_types::core_metadata::CoreMetadata>,
    ) {
        info!(
            "Configured pipeline with stdlib metadata ({} types, {} protocols, {} functions)",
            metadata.types.len(),
            metadata.protocols.len(),
            metadata.functions.len()
        );
        self.stdlib_metadata = Some(metadata);
    }

    /// Get a reference to the session (useful for accessing diagnostics after compilation)
    pub fn session(&self) -> &Session {
        self.session
    }

    /// Get incremental compilation statistics.
    ///
    /// Returns statistics about the incremental compiler's state, including:
    /// - Number of cached modules
    /// - Number of item hashes cached
    /// - Number of files needing verification only
    ///
    /// Useful for monitoring incremental compilation efficiency.
    pub fn incremental_stats(&self) -> crate::incremental_compiler::CacheStats {
        self.incremental_compiler.stats()
    }

    /// Save incremental compilation cache to disk (call after compilation completes).
    pub fn save_incremental_cache(&self) {
        if self.session.options().incremental {
            if let Err(e) = self.incremental_compiler.save_cache() {
                tracing::warn!("Failed to save incremental cache: {}", e);
            }
        }
    }

    /// Get the incremental compiler for advanced usage.
    ///
    /// Provides direct access to the incremental compiler for:
    /// - Computing fine-grained recompilation sets
    /// - Classifying changes (signature vs body)
    /// - Managing dependency graphs
    ///
    /// # Example
    ///
    /// ```ignore
    /// let (full_recompile, verify_only) = pipeline
    ///     .incremental_compiler()
    ///     .compute_incremental_sets_fine_grained(&all_files, |path| {
    ///         // compute hashes
    ///     });
    /// ```
    pub fn incremental_compiler(&self) -> &IncrementalCompiler {
        &self.incremental_compiler
    }

    /// Get mutable access to the incremental compiler.
    ///
    /// Use this for:
    /// - Registering dependencies between modules
    /// - Updating item hashes manually
    /// - Setting cache directory for persistence
    pub fn incremental_compiler_mut(&mut self) -> &mut IncrementalCompiler {
        &mut self.incremental_compiler
    }

    /// Clear accumulated caches to reclaim memory.
    ///
    /// Call this between compilation batches (e.g., in test runners) to prevent
    /// unbounded growth of stdlib bootstrap registries. Normal-mode fields like
    /// `modules` are already cleared per-compilation; this targets the bootstrap-
    /// mode registries that accumulate across modules.
    pub fn clear_caches(&mut self) {
        self.compiled_stdlib_modules.clear();
        self.global_function_registry.shrink_to(1000);
        self.global_protocol_registry.shrink_to(100);
    }

    /// Initialize stdlib from cache.
    ///
    /// This method checks for a valid cached stdlib and loads it if available.
    /// If no cache exists or it's invalid, stdlib will be compiled on first use.
    ///
    /// This is called automatically in `BuildMode::Normal` before compilation.
    /// In `BuildMode::StdlibBootstrap`, this method has no effect.
    ///
    /// # Cache Location
    ///
    /// The cache is stored in `<project_root>/target/.verum/core_cache/`.
    ///
    /// # Cache Invalidation
    ///
    /// The cache is invalidated when:
    /// - Verum compiler version changes
    /// - Target configuration changes (os, arch)
    /// - Stdlib source files change (content hash mismatch)
    pub fn init_stdlib_from_cache(&mut self) -> Result<()> {
        // Skip for StdlibBootstrap mode - we're compiling stdlib, not using cached
        if matches!(self.build_mode, BuildMode::StdlibBootstrap { .. }) {
            debug!("Skipping stdlib cache init in StdlibBootstrap mode");
            return Ok(());
        }

        // Skip if stdlib metadata already set
        if self.stdlib_metadata.is_some() {
            debug!("Stdlib metadata already configured, skipping cache init");
            return Ok(());
        }

        let start = Instant::now();

        // Find project root for cache location
        let project_root = match self.find_workspace_root() {
            Ok(root) => root,
            Err(e) => {
                debug!("Could not find workspace root for stdlib cache: {:?}", e);
                return Ok(()); // Fall back to parsing stdlib source files
            }
        };

        // Auto-detect stdlib source
        let core_source = CoreSource::auto_detect();

        // Get target configuration (use host target)
        let target = verum_ast::cfg::TargetConfig::host();

        // Get or initialize the global stdlib cache
        let cache = global_cache_or_init(&project_root);

        // Try to get cached stdlib
        match cache.get_or_compile(&core_source, &target) {
            Ok(entry) => {
                info!(
                    "Loaded stdlib from cache ({} types, {} functions, compiled in {}ms)",
                    entry.metadata.types.len(),
                    entry.metadata.functions.len(),
                    entry.compilation_duration_ms
                );

                // Convert cached metadata to TypeChecker-compatible format
                let stdlib_meta = self.convert_cached_metadata_to_stdlib(&entry.metadata);
                self.stdlib_metadata = Some(std::sync::Arc::new(stdlib_meta));

                let elapsed = start.elapsed();
                info!("Stdlib cache initialization completed in {:.2}ms", elapsed.as_secs_f64() * 1000.0);
            }
            Err(e) => {
                warn!("Failed to load stdlib from cache: {}. Falling back to source parsing.", e);
                // Fall through - load_stdlib_modules will be called later
            }
        }

        Ok(())
    }

    /// Convert cached stdlib metadata to TypeChecker-compatible format.
    fn convert_cached_metadata_to_stdlib(
        &self,
        cached: &crate::core_cache::CachedCoreMetadata,
    ) -> verum_types::core_metadata::CoreMetadata {
        use verum_types::core_metadata::*;

        let mut metadata = CoreMetadata::default();

        // Convert types
        for cached_type in &cached.types {
            let type_desc = TypeDescriptor {
                name: Text::from(cached_type.path.split('.').next_back().unwrap_or(&cached_type.path)),
                module_path: Text::from(cached_type.path.rsplit_once('.').map(|(p, _)| p).unwrap_or("")),
                generic_params: Self::parse_generic_params_from_definition(&cached_type.definition),
                kind: match cached_type.kind.as_str() {
                    "struct" | "record" => TypeDescriptorKind::Record { fields: List::new() },
                    "variant" | "enum" => TypeDescriptorKind::Variant { cases: List::new() },
                    "protocol" | "trait" => TypeDescriptorKind::Protocol {
                        super_protocols: List::new(),
                        associated_types: List::new(),
                        required_methods: List::new(),
                        default_methods: List::new(),
                    },
                    "alias" => TypeDescriptorKind::Alias { target: Text::new() },
                    _ => TypeDescriptorKind::Record { fields: List::new() },
                },
                size: Maybe::None,
                alignment: Maybe::None,
                methods: List::new(),
                implements: List::new(),
            };
            metadata.types.insert(Text::from(cached_type.path.as_str()), type_desc);
        }

        // Convert functions
        for cached_func in &cached.functions {
            let func_desc = FunctionDescriptor {
                name: Text::from(cached_func.path.split('.').next_back().unwrap_or(&cached_func.path)),
                module_path: Text::from(cached_func.path.rsplit_once('.').map(|(p, _)| p).unwrap_or("")),
                generic_params: List::new(),
                params: List::new(),
                return_type: Text::from("()"),
                contexts: List::new(),
                is_async: false,
                is_unsafe: false,
                intrinsic_id: if cached_func.is_intrinsic {
                    Maybe::Some(0) // Placeholder - actual ID would come from intrinsic_name
                } else {
                    Maybe::None
                },
            };
            metadata.functions.insert(Text::from(cached_func.path.as_str()), func_desc);
        }

        // Set version and content hash
        metadata.version = 1;
        // Content hash is set to zeros - it will be validated by the cache layer
        metadata.content_hash = [0u8; 32];

        // Convert context declarations from the cache
        metadata.context_declarations = cached
            .context_declarations
            .iter()
            .map(|s| verum_common::Text::from(s.as_str()))
            .collect();

        metadata
    }

    /// Parse generic type parameters from a cached type definition string.
    ///
    /// Handles definitions like:
    /// - `"type List<T> is { ... }"` -> `[GenericParam { name: "T", bounds: [], default: None }]`
    /// - `"type Map<K, V> is { ... }"` -> `[GenericParam { name: "K", ... }, GenericParam { name: "V", ... }]`
    /// - `"type Point is { ... }"` -> `[]`
    fn parse_generic_params_from_definition(definition: &str) -> List<verum_types::core_metadata::GenericParam> {
        use verum_types::core_metadata::GenericParam;

        let mut params = List::new();

        // Find the generic parameter list between `<` and `>` after the type name
        let Some(open) = definition.find('<') else {
            return params;
        };
        // Only parse if `<` comes before `is` keyword (i.e., it's part of the type name, not the body)
        if let Some(is_pos) = definition.find(" is ") {
            if open > is_pos {
                return params;
            }
        }
        let Some(close) = definition.find('>') else {
            return params;
        };
        if close <= open {
            return params;
        }

        let params_str = &definition[open + 1..close];
        for param_str in params_str.split(',') {
            let trimmed = param_str.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Handle bounds: "T: Eq + Hash" -> name="T", bounds=["Eq", "Hash"]
            let (name, bounds) = if let Some(colon_pos) = trimmed.find(':') {
                let name = trimmed[..colon_pos].trim();
                let bounds_str = trimmed[colon_pos + 1..].trim();
                let bounds: List<Text> = bounds_str
                    .split('+')
                    .map(|b| Text::from(b.trim()))
                    .filter(|b| !b.is_empty())
                    .collect();
                (name, bounds)
            } else {
                (trimmed, List::new())
            };

            params.push(GenericParam {
                name: Text::from(name),
                bounds,
                default: Maybe::None,
            });
        }

        params
    }


    /// Register meta declarations from a parsed module.
    ///
    /// This extracts meta functions and macros from the module and registers them
    /// in the meta registry, enabling macro expansion and meta-fail tests.
    fn register_meta_declarations(&mut self, path: &Text, module: &Module) -> Result<()> {
        use verum_ast::decl::ItemKind;

        debug!("Registering meta declarations from: {}", path.as_str());

        for item in &module.items {
            match &item.kind {
                ItemKind::Function(func) if func.is_meta => {
                    // Register meta function
                    if let Err(e) = self
                        .meta_registry
                        .register_meta_function(path, func)
                    {
                        let diag = DiagnosticBuilder::error()
                            .message(format!("Failed to register meta function: {}", e))
                            .build();
                        self.session.emit_diagnostic(diag);
                        return Err(anyhow::anyhow!("Meta function registration failed: {}", e));
                    }
                }

                ItemKind::Function(func) if matches!(func.extern_abi, Maybe::Some(_)) => {
                    // Register extern function so meta evaluator can detect FFI calls
                    self.meta_registry.register_extern_function(
                        path,
                        &verum_common::Text::from(func.name.name.as_str()),
                    );
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

        Ok(())
    }

    /// Evaluate meta functions and @const blocks.
    ///
    /// This phase walks the module AST, finds `meta fn` declarations and `@const` blocks,
    /// lowers them to MetaExpr, and evaluates them using the MetaContext. Errors during
    /// evaluation (e.g., division by zero, missing context) are reported as diagnostics.
    fn phase_meta_evaluation(&mut self, module: &Module, module_path: &Text) -> Result<()> {
        use crate::meta::MetaError;
        use verum_ast::decl::{FunctionBody, ItemKind};
        use verum_ast::expr::ExprKind;

        debug!("Running meta evaluation phase");

        let mut errors: Vec<(String, MetaError)> = Vec::new();

        // 1. Evaluate meta fn bodies with NO parameters (fully evaluable at compile time)
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                if !func.is_meta {
                    continue;
                }
                // Only evaluate parameterless meta functions — those with parameters
                // need concrete arguments and are evaluated when called.
                if !func.params.is_empty() {
                    continue;
                }
                let fn_name = func.name.as_str().to_string();
                debug!("Evaluating meta fn: {}", fn_name);

                let body_expr = match &func.body {
                    Maybe::Some(FunctionBody::Block(block)) => {
                        // Create a block expression from the function body
                        verum_ast::expr::Expr::new(
                            ExprKind::Block(block.clone()),
                            block.span,
                        )
                    }
                    Maybe::Some(FunctionBody::Expr(expr)) => expr.clone(),
                    Maybe::None => continue, // No body (declaration only)
                };

                let mut ctx = self.fresh_meta_ctx_with_version_stamp()
                    .with_registry(std::sync::Arc::new(self.meta_registry.clone()))
                    .with_current_module(module_path.clone());

                // Enable contexts from the function's using clause
                if !func.contexts.is_empty() {
                    let context_names: Vec<verum_common::Text> = func.contexts.iter()
                        .filter_map(|c| c.path.as_ident().map(|i| verum_common::Text::from(i.as_str())))
                        .collect();
                    let parsed = crate::meta::builtins::context_requirements::EnabledContexts::parse_using_clause(&context_names);
                    ctx.enabled_contexts = parsed.enabled_contexts;
                }

                match ctx.ast_expr_to_meta_expr(&body_expr) {
                    Ok(meta_expr) => {
                        if let Err(e) = ctx.eval_meta_expr(&meta_expr) {
                            if Self::is_reportable_meta_error(&e) {
                                errors.push((fn_name, e));
                            }
                        }
                    }
                    Err(e) => {
                        if Self::is_reportable_meta_error(&e) {
                            errors.push((fn_name, e));
                        }
                    }
                }
            }
        }

        // 2. Walk all expressions to find @const blocks
        self.find_and_eval_const_blocks(module, module_path, &mut errors);

        if !errors.is_empty() {
            // Emit diagnostics for all meta evaluation errors
            for (context, error) in &errors {
                let code = error.error_code();
                let diag = DiagnosticBuilder::error()
                    .message(format!("{}: {} (in {})", code, error, context))
                    .build();
                self.session.emit_diagnostic(diag);
            }
            return Err(anyhow::anyhow!(
                "Meta evaluation failed with {} error(s)",
                errors.len()
            ));
        }

        Ok(())
    }

    /// Build a [`MetaContext`] seeded with the pipeline's cached
    /// version stamp (#20 / P7). Every per-meta-eval call site in
    /// the pipeline funnels through this helper so the
    /// `@version_stamp` family of meta builtins observes the same
    /// `git_revision` + `build_time_unix_ms` data without
    /// re-invoking `git rev-parse HEAD` on every call.
    fn fresh_meta_ctx_with_version_stamp(&self) -> crate::meta::MetaContext {
        let mut ctx = crate::meta::MetaContext::new();
        ctx.project_info.git_revision = self.cached_version_stamp.0.clone();
        ctx.project_info.build_time_unix_ms = self.cached_version_stamp.1;
        ctx
    }

    /// Check if a meta error is a "real" error that should be reported,
    /// vs an infrastructure limitation (unsupported expression, function not found, etc.).
    fn is_reportable_meta_error(e: &crate::meta::MetaError) -> bool {
        use crate::meta::MetaError;
        matches!(e,
            // M0XX: Core meta errors (type mismatches in user code)
            MetaError::TypeMismatch { .. } |
            // M1XX: Builtin errors (runtime failures in builtins)
            MetaError::AssertionFailed { .. } |
            MetaError::BuiltinEvalError { .. } |
            MetaError::TypeMismatchBuiltin { .. } |
            // M2XX: Context errors
            MetaError::MissingContext { .. } |
            MetaError::ContextCapabilityDenied { .. } |
            MetaError::ContextScopeViolation { .. } |
            // M3XX: Sandbox errors (runtime limits)
            MetaError::ForbiddenOperation { .. } |
            MetaError::MemoryLimitExceeded { .. } |
            MetaError::RecursionLimitExceeded { .. } |
            MetaError::IterationLimitExceeded { .. } |
            MetaError::TimeoutExceeded { .. } |
            MetaError::IONotAllowed { .. } |
            MetaError::UnsafeNotAllowed { .. } |
            MetaError::PathTraversalBlocked { .. } |
            // M6XX: Const evaluation errors
            MetaError::ConstOverflow { .. } |
            MetaError::DivisionByZero |
            MetaError::IndexOutOfBounds { .. } |
            // Compile errors emitted explicitly
            MetaError::CompileError(_)
        )
    }

    /// Walk module AST to find @const blocks and evaluate them.
    fn find_and_eval_const_blocks(
        &self,
        module: &Module,
        module_path: &Text,
        errors: &mut Vec<(String, crate::meta::MetaError)>,
    ) {
        use verum_ast::decl::ItemKind;
        use verum_ast::expr::ExprKind;

        // Walk all function bodies for @const blocks
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                if func.is_meta {
                    continue; // Already handled above
                }
                if let Maybe::Some(body) = &func.body {
                    let body_expr = match body {
                        verum_ast::decl::FunctionBody::Block(block) => {
                            verum_ast::expr::Expr::new(
                                ExprKind::Block(block.clone()),
                                block.span,
                            )
                        }
                        verum_ast::decl::FunctionBody::Expr(expr) => expr.clone(),
                    };
                    self.collect_const_blocks_from_expr(&body_expr, module_path, errors);
                }
            }
        }
    }

    /// Recursively search for @const blocks in an expression and evaluate them.
    fn collect_const_blocks_from_expr(
        &self,
        expr: &verum_ast::expr::Expr,
        module_path: &Text,
        errors: &mut Vec<(String, crate::meta::MetaError)>,
    ) {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::MetaFunction { name, args } if name.as_str() == "const" => {
                // @const { ... } block — evaluate arguments
                for arg in args.iter() {
                    let mut ctx = self.fresh_meta_ctx_with_version_stamp()
                        .with_registry(std::sync::Arc::new(self.meta_registry.clone()))
                        .with_current_module(module_path.clone());
                    match ctx.ast_expr_to_meta_expr(arg) {
                        Ok(meta_expr) => {
                            if let Err(e) = ctx.eval_meta_expr(&meta_expr) {
                                if Self::is_reportable_meta_error(&e) {
                                    errors.push(("@const block".to_string(), e));
                                }
                            }
                        }
                        Err(e) => {
                            if Self::is_reportable_meta_error(&e) {
                                errors.push(("@const block".to_string(), e));
                            }
                        }
                    }
                }
            }
            // Recurse into sub-expressions
            ExprKind::Block(block) => {
                for stmt in &block.stmts {
                    match &stmt.kind {
                        verum_ast::stmt::StmtKind::Expr { expr: e, .. } => {
                            self.collect_const_blocks_from_expr(e, module_path, errors);
                        }
                        verum_ast::stmt::StmtKind::Let { value: Maybe::Some(e), .. } => {
                            self.collect_const_blocks_from_expr(e, module_path, errors);
                        }
                        _ => {}
                    }
                }
                if let Maybe::Some(e) = &block.expr {
                    self.collect_const_blocks_from_expr(e, module_path, errors);
                }
            }
            ExprKind::Call { func, args, .. } => {
                self.collect_const_blocks_from_expr(func, module_path, errors);
                for arg in args.iter() {
                    self.collect_const_blocks_from_expr(arg, module_path, errors);
                }
            }
            _ => {
                // Other expressions: no recursive search needed for now
                // A full implementation would walk all sub-expressions
            }
        }
    }

    /// Expand macros in a module (Pass 2)
    fn expand_module(&mut self, path: &Text, module: &mut Module) -> Result<()> {

        debug!("Expanding macros in module: {}", path.as_str());

        // Create a macro expander visitor
        let mut expander = MacroExpander {
            registry: &self.meta_registry,
            context: self.fresh_meta_ctx_with_version_stamp(),
            module_path: path.clone(),
            expansions: List::new(),
        };

        // Walk the AST to collect macro invocations
        for item in &module.items {
            expander.collect_macro_invocations(item);
        }

        debug!(
            "  Found {} macro invocation(s) in {}",
            expander.expansions.len(),
            path.as_str()
        );

        // Execute meta functions and expand macros
        // Clone expansions to avoid borrow conflicts
        let expansions = expander.expansions.clone();
        let mut expansion_errors: List<(Text, anyhow::Error)> = List::new();

        for expansion in &expansions {
            match expander.expand_macro(expansion) {
                Ok(expanded_items) => {
                    debug!(
                        "  Expanded macro '{}' into {} item(s)",
                        expansion.macro_name.as_str(),
                        expanded_items.len()
                    );
                    // Note: In full implementation, we would insert expanded_items
                    // back into the module at the appropriate location
                    // For now, we just log the expansion
                }
                Err(e) => {
                    warn!(
                        "  Failed to expand macro '{}': {}",
                        expansion.macro_name.as_str(),
                        e
                    );
                    // Emit diagnostic
                    let diag = DiagnosticBuilder::error()
                        .message(format!("Macro expansion failed: {}", e))
                        .build();
                    self.session.emit_diagnostic(diag);
                    // Collect the error for propagation
                    expansion_errors.push((expansion.macro_name.clone(), e));
                }
            }
        }

        // Propagate first error if any expansions failed
        if let Some((macro_name, error)) = expansion_errors.into_iter().next() {
            return Err(anyhow::anyhow!("Macro expansion failed for '{}': {}", macro_name, error));
        }

        Ok(())
    }

    /// Convert a file path to a module path.
    ///
    /// Examples:
    /// - `/Users/.../src/domain/errors.vr` with src_root `/Users/.../src` -> `domain.errors`
    /// - `/Users/.../src/main.vr` with src_root `/Users/.../src` -> `main`
    /// - `/Users/.../src/services/mod.vr` with src_root `/Users/.../src` -> `services`
    ///
    /// File-to-module path mapping: strip src_root prefix, replace `/` with `.`,
    /// strip `.vr` extension, and treat `mod.vr` as the directory module name.
    fn file_path_to_module_path(
        &self,
        file_path: &std::path::Path,
        src_root: &std::path::Path,
    ) -> verum_modules::ModulePath {
        use verum_modules::ModulePath;

        // Strip the src_root prefix to get relative path
        let relative_path = file_path.strip_prefix(src_root).unwrap_or(file_path);

        // Remove .vr extension and convert path separators to dots
        let path_str = relative_path
            .with_extension("")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, ".");

        // Handle "mod" files - they represent their parent directory
        // e.g., "domain/mod" -> "domain"
        let module_path_str = if path_str.ends_with(".mod") || path_str.ends_with("/mod") {
            path_str
                .trim_end_matches(".mod")
                .trim_end_matches("/mod")
                .to_string()
        } else if path_str == "mod" {
            // Root mod.vr -> empty (root module)
            String::new()
        } else {
            path_str
        };

        if module_path_str.is_empty() {
            ModulePath::root()
        } else {
            ModulePath::from_str(&module_path_str)
        }
    }

    /// Load imported modules into the module registry for single-file checking.
    ///
    /// When checking a single file that has imports (e.g., `import super.contexts.{Database}`),
    /// we need to load and parse the imported modules so that:
    /// 1. Types and functions can be resolved during type checking
    /// 2. Context protocols can be registered for `using [...]` clauses
    ///
    /// This method:
    /// 1. Extracts all import statements from the module
    /// 2. Resolves each import path to a file path
    /// 3. Loads and parses the imported module
    /// 4. Extracts exports and contexts
    /// 5. Registers the module in the session's ModuleRegistry
    /// 6. Iteratively loads that module's imports using a work queue
    ///
    /// Cross-module resolution: imports resolved to file paths, loaded, parsed,
    /// and registered. Uses a work queue for transitive import resolution.
    ///
    /// This function uses an iterative approach with an explicit work queue
    /// to avoid stack overflow when loading deeply nested module hierarchies.
    fn load_imported_modules(
        &mut self,
        module: &Module,
        current_module_path: &str,
        src_root: &std::path::Path,
        loaded_paths: &mut std::collections::HashSet<String>,
    ) -> Result<()> {
        use verum_modules::{
            ModuleInfo, ModulePath, extract_contexts_from_module, extract_exports_from_module,
        };

        // Work queue for iterative module loading
        // Each item is (module, current_module_path)
        let mut work_queue: Vec<(Module, String)> =
            vec![(module.clone(), current_module_path.to_string())];
        let src_root = src_root.to_path_buf();

        while let Some((current_module, current_path)) = work_queue.pop() {
            // Extract import paths from the module
            for item in &current_module.items {
                if let ItemKind::Mount(import) = &item.kind {
                    // Extract the base module path from the import
                    let import_path = self.extract_import_module_path(&import.tree.kind);

                    // Determine the base directory for the import
                    // Determine if this is a stdlib import by checking if the
                    // first path segment matches a known stdlib top-level module.
                    // "core.*" and "std.*" are canonical prefixes; bare module names
                    // (e.g., "sys.*", "io.*") are shorthand for "core.sys.*", "core.io.*".
                    let first_segment = import_path.split('.').next().unwrap_or("");
                    let is_stdlib_import = matches!(first_segment,
                        "std" | "core"
                        | "sys" | "mem" | "base" | "intrinsics" | "simd" | "math"
                        | "text" | "collections"
                        | "io" | "time" | "sync"
                        | "async" | "runtime"
                        | "term"
                        | "net"
                        | "meta" | "cognitive"
                        // Actic-dual stdlib (Phase 5 E1).
                        | "action"
                    );
                    let (resolved_path, base_dir) = if is_stdlib_import {
                        // Stdlib import - map to core/ directory
                        // Both "sys.intrinsics" and "std.sys.intrinsics" resolve to core/sys/intrinsics.vr
                        let workspace_root = match self.find_workspace_root() {
                            Ok(root) => root,
                            Err(_) => {
                                debug!(
                                    "Could not find workspace root for stdlib import '{}'",
                                    import_path
                                );
                                continue;
                            }
                        };
                        // Check for core/ first (primary), then stdlib/ (legacy)
                        let core_path = workspace_root.join("core");
                        let stdlib_dir = if core_path.exists() {
                            core_path
                        } else {
                            workspace_root.join("stdlib")
                        };
                        if !stdlib_dir.exists() {
                            debug!("Stdlib directory not found at {:?} or {:?}", workspace_root.join("core"), workspace_root.join("stdlib"));
                            continue;
                        }
                        // Strip stdlib prefixes for file path resolution
                        // - std.sys.intrinsics -> sys.intrinsics
                        // - core.sys.common -> sys.common (since base_dir is already core/)
                        // - sys.intrinsics -> sys.intrinsics (no prefix to strip)
                        let canonical_path = if import_path.starts_with("std.") {
                            import_path[4..].to_string()
                        } else if import_path.starts_with("core.") {
                            // Strip "core." prefix since stdlib_dir already points to core/
                            import_path[5..].to_string()
                        } else {
                            import_path.clone()
                        };
                        (canonical_path, stdlib_dir)
                    } else {
                        // User module import - resolve relative paths
                        let resolved_path = match self
                            .resolve_import_path(&import_path, &current_path)
                        {
                            Ok(path) => path,
                            Err(e) => {
                                debug!("Failed to resolve import path '{}': {}", import_path, e);
                                continue;
                            }
                        };
                        (resolved_path, src_root.clone())
                    };

                    // Skip if already loaded
                    if loaded_paths.contains(&resolved_path) {
                        continue;
                    }
                    loaded_paths.insert(resolved_path.clone());

                    // Convert module path to file path and try to load
                    let module_path = ModulePath::from_str(&resolved_path);
                    let file_path = self.module_path_to_file_path(&module_path, &base_dir);

                    // Try different file locations (file.vr, file/mod.vr)
                    let candidates = vec![file_path.with_extension("vr"), file_path.join("mod.vr")];

                    let mut loaded = false;
                    for candidate in candidates {
                        if candidate.exists() {
                            debug!(
                                "Loading imported module: {} from {:?}",
                                resolved_path, candidate
                            );

                            // Load and parse the module
                            let source_text = match std::fs::read_to_string(&candidate) {
                                Ok(s) => s,
                                Err(e) => {
                                    debug!("Failed to read imported module {:?}: {}", candidate, e);
                                    continue;
                                }
                            };

                            // Load source into session
                            let file_id = match self
                                .session
                                .load_source_string(&source_text, candidate.clone())
                            {
                                Ok(id) => id,
                                Err(e) => {
                                    debug!("Failed to load source for {:?}: {}", candidate, e);
                                    continue;
                                }
                            };

                            // Parse the module
                            let lexer = Lexer::new(&source_text, file_id);
                            let parser = VerumParser::new();
                            let mut imported_module = match parser.parse_module(lexer, file_id) {
                                Ok(m) => m,
                                Err(errors) => {
                                    for error in errors {
                                        debug!(
                                            "Parse error in imported module {:?}: {}",
                                            candidate, error
                                        );
                                    }
                                    continue;
                                }
                            };

                            // Apply @cfg conditional compilation filtering
                            let cfg_evaluator = self.session.cfg_evaluator();
                            imported_module.items = cfg_evaluator.filter_items(&imported_module.items);

                            // Header validation at the
                            // import-on-demand parse path. The
                            // imported module's filesystem path is
                            // `candidate`; pass it to the validator
                            // so cross-file `module foo;` headers
                            // pointing to nothing surface as
                            // warnings here too.
                            let header_warnings =
                                verum_modules::loader::validate_module_headers_against_filesystem(
                                    &candidate,
                                    &imported_module,
                                );
                            for warning in &header_warnings {
                                let diag = DiagnosticBuilder::warning()
                                    .code(warning.code())
                                    .message(warning.message())
                                    .build();
                                self.session.emit_diagnostic(diag);
                            }

                            // Allocate module ID and create ModuleInfo
                            let registry = self.session.module_registry();
                            let module_id = {
                                let reg = registry.write();
                                reg.allocate_id()
                            };

                            let mut module_info = ModuleInfo::new(
                                module_id,
                                module_path.clone(),
                                imported_module.clone(),
                                file_id,
                                source_text.clone().into(),
                            );
                            module_info.header_warnings = header_warnings;

                            // Extract exports from the module's AST
                            match extract_exports_from_module(
                                &imported_module,
                                module_id,
                                &module_path,
                            ) {
                                Ok(export_table) => {
                                    module_info.exports = export_table;
                                    debug!(
                                        "Module '{}': {} exports",
                                        resolved_path,
                                        module_info.exports.len()
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        "Failed to extract exports from '{}': {:?}",
                                        resolved_path, e
                                    );
                                }
                            }

                            // Extract contexts (protocols and explicit contexts) for cross-file resolution
                            let contexts =
                                extract_contexts_from_module(&imported_module, module_id);
                            let context_count = contexts.len();
                            for ctx in contexts {
                                let name: Text = Text::from(ctx.name.as_str());
                                if !self.collected_contexts.contains(&name) {
                                    self.collected_contexts.push(name);
                                }
                            }
                            if context_count > 0 {
                                debug!(
                                    "Module '{}': {} contexts/protocols",
                                    resolved_path, context_count
                                );
                            }

                            // Register the module in the session's registry
                            {
                                let mut reg = registry.write();
                                reg.register(module_info);
                            }

                            // For user-imported modules (non-stdlib), also store in
                            // project_modules so their items get merged into the main
                            // compilation unit for VBC codegen.
                            if !is_stdlib_import {
                                let module_key = Text::from(resolved_path.as_str());
                                if !self.project_modules.contains_key(&module_key) {
                                    self.project_modules.insert(
                                        module_key,
                                        Arc::new(imported_module.clone()),
                                    );
                                }
                            }

                            // Add to work queue instead of recursive call
                            work_queue.push((imported_module, resolved_path.clone()));

                            loaded = true;
                            break;
                        }
                    }

                    if !loaded {
                        debug!(
                            "Could not find imported module '{}' (tried {:?})",
                            resolved_path, file_path
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Extract the base module path from an import tree.
    fn extract_import_module_path(&self, tree: &verum_ast::MountTreeKind) -> String {
        use verum_ast::MountTreeKind;
        use verum_ast::ty::PathSegment;

        let extract_path = |path: &verum_ast::ty::Path| -> String {
            path.segments
                .iter()
                .filter_map(|seg| {
                    match seg {
                        PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                        PathSegment::SelfValue => Some("self".to_string()),
                        PathSegment::Super => Some("super".to_string()),
                        // Relative path marker (leading dot) is treated like "self"
                        PathSegment::Relative => Some("self".to_string()),
                        _ => None,
                    }
                })
                .collect::<Vec<String>>()
                .join(".")
        };

        match tree {
            MountTreeKind::Path(path) => {
                // import module.path.item -> get module.path (parent of item)
                let full = extract_path(path);
                if let Some(dot_pos) = full.rfind('.') {
                    full[..dot_pos].to_string()
                } else {
                    full
                }
            }
            MountTreeKind::Glob(path) => extract_path(path),
            MountTreeKind::Nested { prefix, .. } => extract_path(prefix),
            // #5 / P1.5 — file-relative mount surfaces the
            // file path verbatim. The session loader resolves
            // it to a concrete file before this extractor
            // runs, so the literal path is the cleanest
            // identifier we can return.
            MountTreeKind::File { path, .. } => path.as_str().to_string(),
        }
    }

    /// Resolve relative import paths (self, super) to absolute module paths.
    ///
    /// For modules defined in mod.vr files (e.g., `contexts/mod.vr` with path `contexts`):
    /// - `.database` or `self.database` -> `contexts.database` (child module)
    ///
    /// For regular modules (e.g., `handlers/search.vr` with path `handlers.search`):
    /// - `.other` or `self.other` -> `handlers.other` (sibling module)
    ///
    /// For super imports (supports chained super):
    /// - From `handlers.search`: `super.contexts` -> `contexts` (sibling of parent)
    /// - From `services.package_service`: `super.super.domain` -> `domain` (sibling of services)
    fn resolve_import_path(
        &self,
        import_path: &str,
        current_module_path: &str,
    ) -> Result<String, verum_modules::ModuleError> {
        use verum_modules::{ModulePath, resolve_import};

        let current = ModulePath::from_str(current_module_path);
        // Use the standalone resolve_import function which properly handles
        // chained super (e.g., super.super.domain), unlike ModulePath::resolve_import
        let resolved = resolve_import(import_path, &current)?;

        Ok(resolved.to_string())
    }

    /// Convert a module path to a filesystem path (relative to src_root or stdlib_root).
    ///
    /// For stdlib paths (std.*), this strips the "std" prefix:
    /// - std.time -> time/ (when src_root is core/)
    /// - core.base.Maybe -> core/Maybe (when src_root is core/)
    ///
    /// For user paths, this maps directly:
    /// - domain.errors -> domain/errors (when src_root is src/)
    fn module_path_to_file_path(
        &self,
        module_path: &verum_modules::ModulePath,
        src_root: &std::path::Path,
    ) -> PathBuf {
        let mut path = src_root.to_path_buf();
        let segments = module_path.segments();

        // Check if this is a stdlib path by looking at the first segment
        let is_stdlib = segments
            .first()
            .map(|s| s.as_str() == "std")
            .unwrap_or(false);

        // For stdlib paths, skip the "std" prefix
        let start_idx = if is_stdlib { 1 } else { 0 };

        for i in start_idx..segments.len() {
            path = path.join(segments[i].as_str());
        }
        path
    }

    /// Register all parsed modules in the session's ModuleRegistry for cross-file resolution.
    ///
    /// This phase (1.5) runs after parsing and before expansion to:
    /// 1. Create ModuleInfo for each parsed module
    /// 2. Extract exports (public types, functions, etc.)
    /// 3. Extract contexts and protocols for cross-file context resolution
    /// 4. Register in session.module_registry
    /// 5. Enable type resolution across files
    ///
    /// Phase 1.5: builds export tables (public types, functions) and extracts
    /// contexts/protocols from each module for cross-file name and context resolution.
    fn register_modules_for_cross_file_resolution(&mut self) -> Result<()> {
        use verum_modules::{
            ModuleInfo, extract_contexts_from_module, extract_exports_from_module,
        };

        let start = Instant::now();
        let mut registered_count = 0;

        // Get the module registry from session
        let registry = self.session.module_registry();

        // Note: src_root computation was removed since path_text is already in module path format
        // (e.g., "domain.errors") after being processed by check_project/compile_project.

        // Path-sorted iteration: `self.modules` is a HashMap so the raw
        // iteration order leaks the per-process random hasher seed into
        // ModuleId allocation (`registry.allocate_id()` is a counter).
        // Non-deterministic ModuleIds in turn produce non-deterministic
        // FunctionIds at codegen time when imports resolve via the
        // registry. See module.rs:229-231 + audit memo.
        let mut sorted_modules: Vec<(&Text, &std::sync::Arc<Module>)> =
            self.modules.iter().collect();
        sorted_modules.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (path_text, module_rc) in sorted_modules {
            // The path_text is already in module path format (e.g., "domain.errors")
            // since it was converted when loading sources in check_project.
            // We just need to create a ModulePath from the string directly.
            let module_path = verum_modules::ModulePath::from_str(path_text.as_str());

            // Allocate a new ModuleId
            let module_id = {
                let reg = registry.write();
                reg.allocate_id()
            };

            // Get file_id from the module's first item span (if any), or use a default
            let file_id = module_rc
                .items
                .first()
                .map(|item| item.span.file_id)
                .unwrap_or_else(|| verum_ast::FileId::new(0));

            // Create ModuleInfo
            let mut module_info = ModuleInfo::new(
                module_id,
                module_path.clone(),
                (**module_rc).clone(),
                file_id,
                Text::new(), // Source text not stored in parsed modules
            );

            // Extract exports from the module's AST
            let module_path_str = module_path.to_string();
            match extract_exports_from_module(module_rc, module_id, &module_path) {
                Ok(mut export_table) => {
                    let exports_before = export_table.len();
                    // Add synthetic exports for stdlib built-in types
                    // These types are implemented natively in Rust but need to be visible
                    // in the type system for imports to work
                    self.add_stdlib_builtin_exports(&mut export_table, module_id, &module_path_str);
                    let exports_after = export_table.len();

                    if exports_after > exports_before {
                        info!(
                            "  Module '{}': added {} synthetic exports ({} -> {})",
                            module_path_str,
                            exports_after - exports_before,
                            exports_before,
                            exports_after
                        );
                    }

                    module_info.exports = export_table;
                    debug!(
                        "  Module '{}': {} exports",
                        module_path_str,
                        module_info.exports.len()
                    );
                }
                Err(e) => {
                    warn!(
                        "  Failed to extract exports from '{}': {:?}",
                        module_path_str, e
                    );
                }
            }

            // Extract contexts (protocols and explicit contexts) for cross-file resolution
            // Extract context/protocol declarations for cross-file `using [...]` resolution.
            let contexts = extract_contexts_from_module(module_rc, module_id);
            let context_count = contexts.len();
            for ctx in contexts {
                // Add to collected contexts for later registration in TypeChecker
                let name: Text = Text::from(ctx.name.as_str());
                if !self.collected_contexts.contains(&name) {
                    self.collected_contexts.push(name);
                }
            }
            if context_count > 0 {
                debug!(
                    "  Module '{}': {} contexts/protocols",
                    module_path_str, context_count
                );
            }

            // Register the module in the session's registry
            {
                let mut reg = registry.write();
                reg.register(module_info);
            }

            registered_count += 1;
        }

        let elapsed = start.elapsed();
        info!(
            "  Registered {} modules for cross-file resolution in {:.2}ms ({} contexts)",
            registered_count,
            elapsed.as_secs_f64() * 1000.0,
            self.collected_contexts.len()
        );

        Ok(())
    }

    /// Analyze a module (Pass 3)
    ///
    /// This performs type checking in multiple sub-passes:
    /// 1. Register cross-file contexts (protocols from other modules)
    /// 2. Register all type declarations (to handle forward references)
    /// 3. Check all functions and other items
    ///
    /// Cross-file context resolution enables `using [Context]` across files.
    /// Cross-module name resolution enables imports to resolve types from other modules.
    ///
    /// Per-module semantic analysis. Despite originally being `&mut self`,
    /// the body never writes any field of `Compiler` directly: every
    /// observable mutation flows through `Session::emit_diagnostic`
    /// (lock-free MPMC queue post-#105) or `Session::abort_if_errors`
    /// (atomic counter), and the per-call `TypeChecker` is constructed
    /// fresh and dropped before return. Pre-fix the artificial `&mut`
    /// borrow on `self` serialised the Pass-3 module loop — even on
    /// machines with 16 cores, modules in a large project were analysed
    /// strictly one at a time.
    ///
    /// The `&self` signature (#101) unblocks `module_paths.par_iter()`
    /// at the call site for a 2-4× wall-clock win on multi-module
    /// projects. Parallel correctness rests on three invariants the
    /// audit verified:
    ///
    ///   1. `TypeChecker` instances do not share mutable state — each
    ///      thread owns its checker.
    ///   2. Reads of `self.modules` / `self.collected_contexts` /
    ///      `self.stdlib_metadata` are pure HashMap / List iteration
    ///      with no concurrent writers (the loop runs after all parsing
    ///      passes have completed).
    ///   3. Diagnostic emission and error-counter polling are already
    ///      lock-free atomic operations on `Session`.
    ///
    /// `lazy_resolver` is `Arc<Mutex<dyn LazyModuleResolver>>` so
    /// concurrent late-loads serialise on a single mutex — acceptable
    /// because reachability-narrowing makes late loads rare.
    fn analyze_module(&self, path: &Text, module: &Module) -> Result<()> {
        use verum_ast::ItemKind;

        // Type check all items in the module
        // Pass the module registry for cross-file type resolution
        //
        // Mode selection:
        // - NormalBuild (stdlib_metadata = Some): Use pre-compiled stdlib types
        // - StdlibBootstrap (stdlib_metadata = None): Use builtins only
        let mut checker = match &self.stdlib_metadata {
            Some(metadata) => {
                debug!(
                    "Using stdlib metadata for type checking ({} types)",
                    metadata.types.len()
                );
                TypeChecker::new_with_core(metadata.as_ref().clone())
            }
            None => {
                // Compiling stdlib itself - use minimal context
                TypeChecker::with_minimal_context()
            }
        };

        // Register built-in types (List, Text, Int, Result, Maybe, etc.)
        // NOTE: In NormalBuild mode, these may already be loaded from stdlib metadata,
        // but register_builtins() is idempotent and ensures core intrinsics are available.
        checker.register_builtins();

        // Post-cycle-break (2026-04-24): install the SMT backend by hand.
        checker.set_smt_backend(Box::new(
            verum_smt::refinement_backend::RefinementZ3Backend::new(),
        ));

        // Enable orphan-rule checking: without a current cog name,
        // ProtocolChecker::check_orphan_rule silently returns Ok(()).
        // Use the input file's stem as the cog identifier (stable for
        // single-file builds). Manifest-based builds can override this
        // later via TypeChecker::set_current_cog directly.
        let cog_name = self.session.options().input
            .file_stem()
            .and_then(|s| s.to_str())
            .map(verum_common::Text::from)
            .unwrap_or_else(|| verum_common::Text::from("cog"));
        checker.set_current_cog(cog_name);

        // Configure type checker with module registry for cross-file resolution
        let registry = self.session.module_registry();
        checker.set_module_registry(registry.clone());

        // Configure lazy resolver for on-demand module loading
        // This enables imports to trigger module loading if not already loaded
        checker.set_lazy_resolver(self.lazy_resolver.clone());

        // Sub-pass 0: Register cross-file contexts (protocols and contexts from other modules)
        // This enables `using [Database, Auth]` to work when these are defined elsewhere
        // Register cross-file contexts so `using [Database, Auth]` resolves across files.
        for context_name in &self.collected_contexts {
            checker.register_protocol_as_context(context_name.clone());
        }
        if !self.collected_contexts.is_empty() {
            debug!(
                "  Registered {} cross-file contexts for type checking",
                self.collected_contexts.len()
            );
        }

        // The `path` parameter is already in module path format (e.g., "handlers.users")
        // after being processed by compile_project(). No need to recompute.
        // Module paths use dot-separated format (e.g., "handlers.users").
        let current_module_path_str = path.as_str().to_string();

        // Sub-pass 0: Pre-register all inline modules
        // This enables cross-module imports even when modules are declared after
        // the modules that import from them.
        // Pre-register inline modules for order-independent cross-module imports.
        for item in &module.items {
            if let ItemKind::Module(module_decl) = &item.kind {
                checker.pre_register_module_public(module_decl, "cog");
            }
        }

        // Sub-pass 1: Process imports to register imported types and functions
        // This enables cross-file type resolution for items like `import domain.errors.{RegistryError}`
        // Cross-module name resolution: process imports before type declarations.
        for item in &module.items {
            if let ItemKind::Mount(import) = &item.kind {
                if let Err(type_error) =
                    checker.process_import(import, &current_module_path_str, &registry.read())
                {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // ═══════════════════════════════════════════════════════════════════
        // PRE-PASS: Register stdlib module declarations into the type checker.
        // This processes all parsed stdlib modules through multi-pass registration,
        // making stdlib types, protocols, and implement block methods (List.push,
        // Maybe.unwrap, etc.) available for type checking user code.
        //
        // Without this, the TypeChecker only has built-in primitives and cannot
        // resolve stdlib types referenced in user code or cross-module imports.
        // ═══════════════════════════════════════════════════════════════════
        {
            // Collect all stdlib modules (those starting with "core.") to avoid
            // re-registering user modules that are already being analyzed.
            // Sort for deterministic iteration (self.modules is a HashMap):
            // shallower module keys come first so top-level stdlib functions beat
            // nested-module helpers when short names collide.
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
                    debug!("analyze_module: Registering {} stdlib modules into type checker", stdlib_entries.len());

                    // Pass S0a: Register all stdlib type names (module-scoped so
                    // fully-qualified `{mod}.{name}` keys are populated for
                    // same-named stdlib types across different modules).
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        checker.register_all_type_names(&stdlib_mod.items);
                    }

                    // Pass S0b: Resolve all stdlib type definitions
                    let mut resolution_stack = List::new();
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        for item in &stdlib_mod.items {
                            if let ItemKind::Type(type_decl) = &item.kind {
                                if let Err(e) = checker.resolve_type_definition(type_decl, &mut resolution_stack) {
                                    debug!("Stdlib type resolution error: {:?}", e);
                                }
                            }
                        }
                    }

                    // Pass S1: Register stdlib function signatures
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        for item in &stdlib_mod.items {
                            if let ItemKind::Function(func) = &item.kind {
                                if !checker.is_function_preregistered(func.name.name.as_str()) {
                                    if let Err(e) = checker.register_function_signature(func) {
                                        debug!("Stdlib function registration error: {:?}", e);
                                    }
                                }
                            }
                        }
                    }

                    // Pass S2: Register stdlib protocols
                    for (module_path, stdlib_mod) in &stdlib_entries {
                        checker.set_current_module_path(module_path.clone());
                        for item in &stdlib_mod.items {
                            if let ItemKind::Protocol(protocol_decl) = &item.kind {
                                if let Err(e) = checker.register_protocol(protocol_decl) {
                                    debug!("Stdlib protocol registration error: {:?}", e);
                                }
                            }
                        }
                    }
                }

                // Pass S3: ALWAYS register stdlib impl blocks (this registers methods
                // in inherent_methods). This must run even when metadata IS available,
                // because metadata doesn't populate inherent_methods from implement blocks.
                debug!("analyze_module: Registering stdlib impl blocks ({} modules)", stdlib_entries.len());
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    for item in &stdlib_mod.items {
                        if let ItemKind::Impl(impl_decl) = &item.kind {
                            if let Err(e) = checker.register_impl_block(impl_decl) {
                                debug!("Stdlib impl registration error: {:?}", e);
                            }
                        }
                    }
                }

                debug!("analyze_module: Stdlib registration complete");
            }

            // Restore the user-file module path so subsequent passes run in
            // the right resolution scope.
            checker.set_current_module_path(saved_module_path);
        }

        // Signal transition to user code phase
        checker.set_user_code_phase();

        // Sub-pass 2: Register all type declarations first
        // This ensures types are available when checking functions that reference them
        for item in &module.items {
            if let ItemKind::Type(type_decl) = &item.kind {
                if let Err(type_error) = checker.register_type_declaration(type_decl) {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Sub-pass 3: Register implement blocks
        // This ensures methods are available for resolution
        for item in &module.items {
            if let ItemKind::Impl(impl_decl) = &item.kind {
                if let Err(type_error) = checker.register_impl_block(impl_decl) {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Sub-pass 3.5: Protocol coherence checking (orphan rule, overlap, specialization)
        // Validates that protocol implementations follow coherence rules across the
        // entire dependency graph (user module + stdlib + project modules).
        self.check_protocol_coherence(module)?;

        // Sub-pass 4: Register function signatures (enables forward references)
        // This allows functions to call other functions defined later in the file:
        //   fn main() { helper() }  // helper is defined below
        //   fn helper() { ... }
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                if let Err(type_error) = checker.register_function_signature(func) {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Sub-pass 4.5: Register extern function signatures (FFI)
        // This allows calling FFI functions declared in extern blocks:
        //   @ffi("libSystem.B.dylib")
        //   extern { fn getpid() -> Int; }
        for item in &module.items {
            if let ItemKind::ExternBlock(extern_block) = &item.kind {
                // Register each function in the extern block
                for func in &extern_block.functions {
                    if let Err(type_error) = checker.register_function_signature(func) {
                        let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                        self.session.emit_diagnostic(diag);
                    }
                }
            }
        }

        // Sub-pass 4.6: Pre-register const declarations (enables forward references)
        // Constants defined after functions should still be visible in function bodies.
        for item in &module.items {
            if let ItemKind::Const(const_decl) = &item.kind {
                checker.pre_register_const(const_decl);
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

        // Sub-pass 5: Check all items (functions, impls, etc.)
        for item in &module.items {
            if let Err(type_error) = checker.check_item(item) {
                let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                self.session.emit_diagnostic(diag);
            }
        }

        // Abort if errors occurred
        self.session.abort_if_errors()?;

        Ok(())
    }


    // ==================== COMPILATION PHASES ====================

    /// Phase 1: Load source file
    ///
    /// If input is a directory, finds main.vr inside it.
    /// If input is a file, loads it directly.
    pub fn phase_load_source(&mut self) -> Result<FileId> {
        let input = self.session.options().input.clone();
        debug!("Loading source: {}", input.display());

        // If input is a directory, look for main.vr inside
        let actual_file = if input.is_dir() {
            let main_file = input.join("main.vr");
            if main_file.exists() {
                main_file
            } else {
                // Try to find any .vr file
                let files = self.session.discover_project_files()?;
                files.into_iter().next().ok_or_else(|| {
                    anyhow::anyhow!(
                        "No .vr files found in directory: {}. \
                         For single-file compilation, specify the .vr file directly.",
                        input.display()
                    )
                })?
            }
        } else {
            input
        };

        let file_id = self
            .session
            .load_file(&actual_file)
            .with_context(|| format!("Failed to load source file: {}", actual_file.display()))?;

        Ok(file_id)
    }

    /// Phase 2: Lexing and parsing
    pub fn phase_parse(&mut self, file_id: FileId) -> Result<Module> {
        debug!("Parsing file {:?}", file_id);

        // Check cache first
        if let Some(cached) = self.session.get_module(file_id) {
            debug!("Using cached module");
            // Still need to clone here since we can't return Shared as Module
            return Ok((*cached).clone());
        }

        let source: Shared<SourceFile> =
            self.session.get_source(file_id).context("Source file not found")?;

        // Lexing + Parsing (combined via parser)
        let start = Instant::now();

        // Decide library-mode vs script-mode parsing based on shebang
        // autodetection or the entry-source script_mode flag. See
        // `should_parse_as_script` for the full rule.
        let script = should_parse_as_script(
            source.source.as_str(),
            self.session.options(),
            source.path.as_deref(),
        );

        let parser = VerumParser::new();
        let parse_result = if script {
            parser.parse_module_script_str(source.source.as_str(), file_id)
        } else {
            let lexer = Lexer::new(&source.source, file_id);
            parser.parse_module(lexer, file_id)
        };
        let mut module = parse_result.map_err(|errors| {
            // Convert parser errors to diagnostics
            let error_count = errors.len();
            for error in errors.iter() {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("Parse error: {}", error));
                // Include error code if present (e.g., M401 for splice outside quote)
                if let Some(ref code) = error.code {
                    builder = builder.code(code.clone());
                }
                self.session.emit_diagnostic(builder.build());
            }
            // Display diagnostics before returning error
            let _ = self.session.display_diagnostics();
            anyhow::anyhow!("Parsing failed with {} error(s)", error_count)
        })?;

        // Apply @cfg conditional compilation filtering
        let cfg_evaluator = self.session.cfg_evaluator();
        let original_count = module.items.len();
        module.items = cfg_evaluator.filter_items(&module.items);
        let filtered_count = original_count - module.items.len();

        let parse_time = start.elapsed();
        debug!(
            "Parsed module with {} items ({} filtered by @cfg) in {:.2}ms",
            module.items.len(),
            filtered_count,
            parse_time.as_millis()
        );

        // MOD-MED-1 — validate module headers against
        // the filesystem. Surfaces dangling forward declarations
        // (`module foo;` with no source file) and inline-vs-
        // filesystem overlaps (`module foo { … }` alongside an
        // existing `foo/` directory). Non-blocking warnings — the
        // user fixes the dangling decl and re-runs.
        if let Some(ref file_path) = source.path {
            let warnings =
                verum_modules::loader::validate_module_headers_against_filesystem(
                    file_path,
                    &module,
                );
            for warning in warnings {
                let diag = DiagnosticBuilder::warning()
                    .code(warning.code())
                    .message(warning.message())
                    .build();
                self.session.emit_diagnostic(diag);
            }
        }

        // Record parsing metrics
        self.session.record_phase_metrics("Parsing", parse_time, 0);

        // Honour `--emit-ast`: serialise the freshly parsed module to
        // a sidecar `.ast.json` next to the input source.  The flag
        // was a config field with no readers — it has been declared
        // and defaulted on `CompilerOptions` for a long while, but
        // no compilation phase emitted anything when it was set, so
        // the documented "Emit AST in JSON format" contract was a
        // no-op.  We mirror the `emit_types`/`emit_vbc` pattern of
        // best-effort write + debug log on failure (non-fatal).
        if self.session.options().emit_ast {
            let ast_path = self.session.options().input.with_extension("ast.json");
            match serde_json::to_vec_pretty(&module) {
                Ok(data) => match std::fs::write(&ast_path, &data) {
                    Ok(()) => info!(
                        "Exported AST: {} ({} bytes)",
                        ast_path.display(),
                        data.len()
                    ),
                    Err(e) => debug!("Failed to write AST: {}", e),
                },
                Err(e) => debug!("Failed to serialise AST: {}", e),
            }
        }

        // Cache the module (session still uses its own caching mechanism)
        self.session.cache_module(file_id, module.clone());

        // Abort if errors
        self.session.abort_if_errors()?;

        Ok(module)
    }

    /// Public wrapper for type checking phase.
    ///
    /// Used by the `verum analyze` command to run type checking before
    /// CBGR analysis. Errors are returned but non-fatal for analysis purposes.
    pub fn run_type_check_phase(&mut self, module: &Module) -> Result<()> {
        self.phase_type_check(module)
    }

    /// Public wrapper for building a function's control flow graph.
    ///
    /// Used by the `verum analyze` command to run escape analysis on individual
    /// functions without going through the full compilation pipeline.
    pub fn build_function_cfg_public(
        &self,
        func: &verum_ast::decl::FunctionDecl,
    ) -> verum_cbgr::analysis::ControlFlowGraph {
        self.build_function_cfg(func)
    }

    /// Unified pre-codegen validation.
    ///
    /// Runs every language-mechanism validation that must agree
    /// between the interpreter, CPU AOT, and GPU paths:
    ///   1. `[safety]` gates (unconditional, regardless of verify_mode)
    ///   2. Type check
    ///   3. Target-profile / dependency analysis
    ///   4. SMT refinement verification (if `verify_mode.use_smt()`)
    ///   5. Context / DI validation
    ///   6. Send/Sync boundary enforcement
    ///   7. CBGR tier analysis
    ///   8. FFI boundary validation
    ///
    /// Every pipeline entry point (`run_interpreter`,
    /// `run_native_compilation`, `run_mlir_aot`, `run_for_test`, …)
    /// should call this method to guarantee identical semantics on
    /// every .vr file across every execution path. The `skip_type`
    /// flag is offered for pathological fast-paths (e.g.,
    /// verify_mode = Runtime + no user-requested gates), but even
    /// then the safety gate still fires.
    fn validate_module(&mut self, module: &Module, skip_type_check: bool) -> Result<()> {
        // Safety gate ALWAYS runs. Independent of verify_mode.
        self.phase_safety_gate(module)?;

        if skip_type_check {
            // Only the minimum gates that don't depend on typed AST.
            // Context/send_sync/ffi walkers inspect the untyped AST so
            // they're still correct here; skipping them would
            // introduce a runtime-mode bypass. Three independent &self
            // walks → fan out via rayon::scope.
            let parallel = std::env::var("VERUM_NO_PARALLEL_POST_TYPECHECK").is_err();
            if parallel {
                let ffi_result = std::sync::Mutex::new(Ok(()));
                rayon::scope(|s| {
                    s.spawn(|_| self.phase_context_validation(module));
                    s.spawn(|_| self.phase_send_sync_validation(module));
                    s.spawn(|_| {
                        *ffi_result.lock().unwrap() = self.phase_ffi_validation(module);
                    });
                });
                ffi_result.into_inner().unwrap()?;
            } else {
                self.phase_context_validation(module);
                self.phase_send_sync_validation(module);
                self.phase_ffi_validation(module)?;
            }
            return Ok(());
        }

        self.phase_type_check(module)?;

        // Post-typecheck parallel fan-out (#104). Same architectural
        // contract as `run_native_compilation`: every gate is `&self`,
        // sinks are `Session::*` `&self` methods with internal
        // synchronisation, no aliased mutable state across workers.
        let smt_enabled = self.session.options().verify_mode.use_smt();
        let parallel = std::env::var("VERUM_NO_PARALLEL_POST_TYPECHECK").is_err();

        if parallel {
            let dep_result = std::sync::Mutex::new(Ok(()));
            let verify_result = std::sync::Mutex::new(Ok(()));
            let cbgr_result = std::sync::Mutex::new(Ok(()));
            let ffi_result = std::sync::Mutex::new(Ok(()));

            rayon::scope(|s| {
                s.spawn(|_| {
                    *dep_result.lock().unwrap() = self.phase_dependency_analysis(module);
                });
                if smt_enabled {
                    s.spawn(|_| {
                        *verify_result.lock().unwrap() = self.phase_verify(module);
                    });
                }
                s.spawn(|_| self.phase_context_validation(module));
                s.spawn(|_| self.phase_send_sync_validation(module));
                s.spawn(|_| {
                    *cbgr_result.lock().unwrap() = self.phase_cbgr_analysis(module);
                });
                s.spawn(|_| {
                    *ffi_result.lock().unwrap() = self.phase_ffi_validation(module);
                });
            });

            dep_result.into_inner().unwrap()?;
            if smt_enabled {
                verify_result.into_inner().unwrap()?;
            }
            cbgr_result.into_inner().unwrap()?;
            ffi_result.into_inner().unwrap()?;
        } else {
            self.phase_dependency_analysis(module)?;
            if smt_enabled {
                self.phase_verify(module)?;
            }
            self.phase_context_validation(module);
            self.phase_send_sync_validation(module);
            self.phase_cbgr_analysis(module)?;
            self.phase_ffi_validation(module)?;
        }

        Ok(())
    }

    /// Phase 2.9: Safety feature gates (unsafe blocks, unsafe fn,
    /// `@ffi` / extern fn, per `[safety]` in verum.toml).
    ///
    /// **Runs independently of verify_mode** so `--verify runtime`
    /// cannot silently bypass the gate. Invoked by BOTH the
    /// interpreter and AOT paths before type-checking. Emits a
    /// diagnostic for each rejected construct and returns Err if any
    /// were rejected.
    fn phase_safety_gate(&self, module: &Module) -> Result<()> {
        let features = self.session.language_features();
        // Fast path: when every relevant [safety] flag is permissive,
        // skip the walker entirely — zero cost on the default
        // configuration.
        if features.unsafe_allowed() && features.safety.ffi
            && !features.safety.capability_required
            && !features.safety.forbid_stdlib_extern
        {
            return Ok(());
        }
        let policy = crate::phases::safety_gate::SafetyPolicy::from_features(
            &features.safety,
        );
        let diags = crate::phases::safety_gate::check_safety(
            std::slice::from_ref(module),
            policy,
        );
        if !diags.is_empty() {
            let n = diags.len();
            for d in diags.iter() {
                self.session.emit_diagnostic(d.clone());
            }
            return Err(anyhow::anyhow!(
                "safety gate rejected {} construct(s); see diagnostics",
                n
            ));
        }
        Ok(())
    }

    /// Stdlib-hazard lint pass.
    ///
    /// Pure AST walk; runs before type checking so the user sees
    /// W05xx warnings even when the module has type errors that
    /// would stop the pipeline at `phase_type_check`. Findings
    /// are emitted as warning-level diagnostics and never fail
    /// the build (the lint's `default_level()` is `Warn`); CI
    /// teams that want stricter enforcement set `-Dmap_get_hazard`
    /// in their lint configuration.
    fn phase_stdlib_lints(&self, module: &Module) {
        use crate::lint::{
            walk_module_for_stdlib_hazards, StdlibLintFinding,
        };
        let findings: Vec<StdlibLintFinding> =
            walk_module_for_stdlib_hazards(module);
        for finding in findings {
            let summary_text = finding.lint.summary();
            let diag = verum_diagnostics::DiagnosticBuilder::warning()
                .code(finding.lint.warning_code())
                .message(format!(
                    "{} (`{}`)",
                    summary_text,
                    finding.lint.name()
                ))
                .span(super::phases::ast_span_to_diagnostic_span(
                    finding.span,
                    None,
                ))
                .help(
                    "Prefer `get_optional(key)` for presence semantics or \
                     `get_or(key, default)` for an explicit fallback.",
                )
                .build();
            self.session.emit_diagnostic(diag);
        }
    }

    /// Phase 3: Type checking
    fn phase_type_check(&mut self, module: &Module) -> Result<()> {
        debug!("Type checking module");

        // Safety-gate pre-pass. Still invoked here (in addition to the
        // callsites in run_interpreter / run_native_compilation) so any
        // pipeline entry point that jumps directly to type_check keeps
        // the gate active. Idempotent — running twice costs one extra
        // walk on the fast path (no violations).
        self.phase_safety_gate(module)?;

        // Stdlib-hazard lint pass — AST-only, emits W05xx
        // warnings. Runs before type checking so users see the
        // hazards even when the module has unrelated type errors.
        self.phase_stdlib_lints(module);

        let start = Instant::now();

        // Mode selection:
        // - NormalBuild (stdlib_metadata = Some): Use pre-compiled stdlib types
        // - StdlibBootstrap (stdlib_metadata = None): Use builtins only
        let mut checker = match &self.stdlib_metadata {
            Some(metadata) => {
                debug!(
                    "Phase 3: Using stdlib metadata for type checking ({} types)",
                    metadata.types.len()
                );
                TypeChecker::new_with_core(metadata.as_ref().clone())
            }
            None => {
                // Compiling stdlib itself - use minimal context
                TypeChecker::with_minimal_context()
            }
        };

        // Register built-in types and functions
        // NOTE: In NormalBuild mode, these may already be loaded from stdlib metadata,
        // but register_builtins() is idempotent and ensures core intrinsics are available.
        checker.register_builtins();

        // Post-cycle-break (2026-04-24): `RefinementChecker` no longer
        // auto-constructs a Z3 backend. Install the concrete bridge from
        // `verum_smt` so refinement subsumption keeps working.
        checker.set_smt_backend(Box::new(
            verum_smt::refinement_backend::RefinementZ3Backend::new(),
        ));

        // Enable lenient contexts if the module has meta functions with using clauses.
        // Meta contexts (MetaTypes, MetaRuntime, etc.) are handled at the meta evaluation
        // level, not at the runtime context level. Without lenient mode, @const blocks
        // that call meta functions with contexts would fail type checking.
        let has_meta_contexts = module.items.iter().any(|item| {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                func.is_meta && !func.contexts.is_empty()
            } else {
                false
            }
        });
        if has_meta_contexts {
            checker.set_lenient_contexts(true);
        }

        // ==============================================================
        // Dependent type pipeline activation (Phase A.5 — surgical)
        // ==============================================================
        //
        // Historical note: before this hook was added, the entire SMT-
        // based dependent type verification path in `verum_types` and
        // `verum_smt/src/dependent.rs` was dormant code — fully
        // implemented (~3700 LoC in verum_smt alone plus the full
        // `UniverseContext` solver at `verum_types/src/context.rs:681`)
        // but never switched on from the main compiler pipeline. It only
        // ran in isolated unit tests that called `enable_dependent_types`
        // explicitly. Production builds therefore silently bypassed it.
        //
        // This activation is deliberately **opportunistic**:
        //
        //   - If the module contains ANY declaration that uses dependent-
        //     type machinery (theorem / lemma / axiom / corollary /
        //     tactic items, or refinement predicates on function
        //     parameters / return types), we enable the checker's
        //     dependent type subsystem so Pi, Sigma, Eq and universe
        //     constraints are verified through SMT.
        //
        //   - Modules that do NOT use any of the above pay zero cost —
        //     the subsystem remains disabled and compilation is
        //     bit-identical to the pre-activation behaviour.
        //
        // This preserves backward compatibility with every existing
        // `.vr` source file while making dependent types available to
        // theorem-bearing modules without requiring any user flag.
        //
        // Detection criteria (keep in sync with proof_erasure in
        // `crates/verum_vbc/src/codegen/mod.rs:3232-3250`):
        //
        //   - `ItemKind::Theorem | Lemma | Corollary | Axiom | Tactic`
        //     always require dependent type checking (proof goals are
        //     type-level propositions).
        //
        // A future refinement may also scan for explicit Pi / Sigma /
        // refinement types in function signatures and enable the
        // subsystem only when needed; for now, "any proof item present"
        // is the conservative, opt-in trigger.
        //
        // Related: `crates/verum_types/src/infer.rs:3135`
        // (`enable_dependent_types`), `:3118` (`verify_dependent_type`),
        // `:4097` (`verify_dependent_type_constraint`), call sites at
        // `:19754, :20229, :20263, :20328`.
        let has_proof_items = module.items.iter().any(|item| {
            matches!(
                &item.kind,
                verum_ast::ItemKind::Theorem(_)
                    | verum_ast::ItemKind::Lemma(_)
                    | verum_ast::ItemKind::Corollary(_)
                    | verum_ast::ItemKind::Axiom(_)
                    | verum_ast::ItemKind::Tactic(_)
            )
        });
        if has_proof_items {
            debug!("Phase 3: Enabling dependent type checking (proof items detected)");
            // Post-cycle-break (2026-04-24): `enable_dependent_types()`
            // no longer auto-constructs a verifier. Inject the SMT-backed
            // implementation directly from `verum_smt`.
            checker.enable_dependent_types();
            checker.set_dependent_checker(Box::new(
                verum_smt::dependent_backend::SmtDependentTypeChecker::new(),
            ));
        }

        // Configure type checker with module registry for cross-file resolution
        let registry = self.session.module_registry();
        checker.set_module_registry(registry.clone());

        // Configure lazy resolver for on-demand module loading
        // This enables imports to trigger module loading if not already loaded
        checker.set_lazy_resolver(self.lazy_resolver.clone());

        // Register cross-file contexts (protocols and contexts from other modules)
        // This enables `using [Database, Auth]` to work when these are defined elsewhere
        for context_name in &self.collected_contexts {
            checker.register_protocol_as_context(context_name.clone());
        }

        // Pre-register stdlib context declarations from CoreMetadata.
        // This enables `using [ComputeDevice]` etc. to resolve even
        // in single-file compilation where the declaring module
        // (gpu.vr) isn't explicitly loaded. The context names were
        // extracted during stdlib bootstrap and cached in the
        // embedded stdlib archive.
        if let Some(metadata) = &self.stdlib_metadata {
            for ctx_name in &metadata.context_declarations {
                if !self.collected_contexts.contains(ctx_name) {
                    checker.register_protocol_as_context(ctx_name.clone());
                }
            }
        }

        // Fallback: if the metadata cache doesn't have contexts
        // (e.g., old cache format), extract them directly from the
        // embedded stdlib source archive. This scans for
        // `public context Name {` patterns in .vr files.
        {
            let has_metadata_contexts = self.stdlib_metadata
                .as_ref()
                .map(|m| !m.context_declarations.is_empty())
                .unwrap_or(false);
            if !has_metadata_contexts {
                if let Some(archive) = crate::embedded_stdlib::get_embedded_stdlib() {
                    // Enable lenient context checking during pre-registration.
                    // Stdlib context method types may reference types from the
                    // same module that aren't registered yet — lenient mode
                    // defers method validation to call sites.
                    checker.set_lenient_context_checking(true);
                    let mut found_count = 0usize;
                    // Preserve the user-file module path so we can restore after.
                    let saved_ctx_path = checker.current_module_path().clone();
                    for path in archive.file_paths() {
                        if !path.ends_with(".vr") {
                            continue;
                        }
                        let content = match archive.get_file(path) {
                            Some(c) => c,
                            None => continue,
                        };
                        // Quick check: skip files without context declarations
                        if !content.contains("public context ") {
                            continue;
                        }
                        // Compute the module path for this stdlib file so that
                        // bare type references inside the context body (e.g.
                        // `LogLevel` in `fn log(level: LogLevel, msg: Text)`
                        // inside `core.context.standard.Logger`) resolve
                        // against this file's qualified-name layer first.
                        // Without this, `ast_to_type` falls back to the flat
                        // `ctx.type_defs` map where a same-named stranger
                        // (`core.base.log.LogLevel`) may be registered last
                        // and silently overwrite the expected type.
                        let mod_path = {
                            let trimmed = path.trim_end_matches(".vr");
                            // Archive paths are relative to core/, e.g.
                            // "context/standard" -> "core.context.standard".
                            let dotted = trimmed.replace('/', ".");
                            let without_core = dotted
                                .strip_prefix("core.")
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| dotted.clone());
                            let absolute = format!("core.{}", without_core);
                            // Handle `mod.vr` files (represent the parent dir).
                            if let Some(stripped) = absolute.strip_suffix(".mod") {
                                stripped.to_string()
                            } else {
                                absolute
                            }
                        };
                        checker.set_current_module_path(verum_common::Text::from(mod_path));

                        // Parse the file with the actual parser to get
                        // full ContextDecl AST nodes with method signatures.
                        let mut parser = verum_fast_parser::Parser::new(content);
                        if let Ok(module) = parser.parse_module() {
                            for item in &module.items {
                                if let verum_ast::ItemKind::Context(ctx_decl) = &item.kind {
                                    if ctx_decl.visibility == verum_ast::decl::Visibility::Public {
                                        let ctx_name = verum_common::Text::from(
                                            ctx_decl.name.name.as_str(),
                                        );
                                        // Register with FULL method signatures in
                                        // both resolver and checker. We do NOT skip
                                        // on `collected_contexts.contains(&ctx_name)`
                                        // because the collected_contexts loop above
                                        // only calls `register_protocol_as_context`
                                        // (resolver-only), leaving the context_checker
                                        // unaware of the declaration — which made
                                        // call-site `check_provided_contexts` fail
                                        // with "undefined context" even though the
                                        // resolver accepted it.
                                        //
                                        // `register_stdlib_context_full` is idempotent
                                        // enough: a second registration overwrites
                                        // the declaration with the same content.
                                        checker.register_stdlib_context_full(
                                            ctx_name,
                                            ctx_decl.clone(),
                                        );
                                        found_count += 1;
                                    }
                                }
                            }
                        }
                    }
                    checker.set_current_module_path(saved_ctx_path);
                    // Restore strict context checking for user code.
                    checker.set_lenient_context_checking(false);
                    if found_count > 0 {
                        tracing::debug!(
                            "Stdlib context pre-registration: {} contexts with full signatures from embedded archive",
                            found_count
                        );
                    }
                }
            }
        }

        // Compute the current module path for resolving relative imports (self, super)
        // For single-file checking, we need to find the project's src root by looking
        // upward from the file location. The src root is typically:
        // - A directory named "src" that contains .vr files
        // - Or a directory containing Verum.toml
        // - Or the parent of "core/" for stdlib files
        let project_root = self.session.options().input.clone();
        let src_root = if project_root.is_dir() {
            let src_dir = project_root.join("src");
            if src_dir.exists() && src_dir.is_dir() {
                src_dir
            } else {
                project_root.clone()
            }
        } else {
            // Single file: walk up the directory tree to find src root
            // The src root is typically a directory named "src" or a directory
            // that contains Verum.toml (project root's src subdirectory)
            let mut current = project_root.parent().map(|p| p.to_path_buf());
            let mut found_src = None;

            while let Some(dir) = current {
                // Check if this directory is named "src"
                if dir.file_name().map(|n| n == "src").unwrap_or(false) {
                    found_src = Some(dir.clone());
                    break;
                }
                // Check if parent has a Verum.toml (meaning this is inside a project)
                if let Some(parent) = dir.parent() {
                    if parent.join("Verum.toml").exists() || parent.join("verum.toml").exists() {
                        // Special case: if the parent directory is named "core", this
                        // is a stdlib file. Use core/'s parent as the src_root so that
                        // file_path_to_module_path produces paths like "core.async".
                        if parent.file_name().map(|n| n == "core").unwrap_or(false) {
                            if let Some(grandparent) = parent.parent() {
                                found_src = Some(grandparent.to_path_buf());
                                break;
                            }
                        }
                        // This directory (or "src" subdirectory) is the source root
                        let src_dir = parent.join("src");
                        if src_dir.exists() && src_dir.is_dir() {
                            found_src = Some(src_dir);
                        } else {
                            found_src = Some(dir.clone());
                        }
                        break;
                    }
                }
                current = dir.parent().map(|p| p.to_path_buf());
            }

            found_src.unwrap_or_else(|| {
                // Fallback: use the file's parent directory
                project_root
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(project_root.clone())
            })
        };

        // Try to compute module path from file if available
        let current_module_path_str = if let Some(item) = module.items.first() {
            if let Some(source_file) = self.session.get_source(item.span.file_id) {
                if let Some(ref file_path) = source_file.path {
                    let module_path = self.file_path_to_module_path(file_path, &src_root);
                    debug!(
                        "phase_type_check: file_path={:?}, src_root={:?}, module_path={}",
                        file_path, src_root, module_path
                    );
                    module_path.to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Load imported modules into the registry for single-file checking.
        // This ensures that imported types, functions, and contexts are available
        // for type checking. Without this, imports like `import super.contexts.{Database}`
        // would fail because the imported module wouldn't be in the registry.
        debug!(
            "Single-file type check: src_root={:?}, current_module_path={}",
            src_root, current_module_path_str
        );
        let mut loaded_paths = std::collections::HashSet::new();
        if let Err(e) = self.load_imported_modules(
            module,
            &current_module_path_str,
            &src_root,
            &mut loaded_paths,
        ) {
            warn!("Error loading imported modules: {}", e);
        }
        debug!(
            "Loaded {} imported modules, collected {} contexts",
            loaded_paths.len(),
            self.collected_contexts.len()
        );

        // Re-register cross-file contexts after loading imported modules
        // (new contexts may have been discovered)
        for context_name in &self.collected_contexts {
            checker.register_protocol_as_context(context_name.clone());
        }

        // Multi-pass type checking:
        // Pass -1: Pre-register all inline modules
        // This enables cross-module imports even when modules are declared after
        // the modules that import from them.
        // Pre-register inline modules for order-independent cross-module imports.
        for item in &module.items {
            if let verum_ast::ItemKind::Module(module_decl) = &item.kind {
                checker.pre_register_module_public(module_decl, "cog");
            }
        }

        // Pass 0: Process imports to register imported types and functions
        // This enables cross-file type resolution for imported types
        for item in &module.items {
            if let verum_ast::ItemKind::Mount(import) = &item.kind {
                if let Err(type_error) =
                    checker.process_import(import, &current_module_path_str, &registry.read())
                {
                    let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // ═══════════════════════════════════════════════════════════════════
        // PRE-PASS: Register stdlib module declarations into the type checker.
        // This processes all parsed stdlib modules through the same multi-pass
        // registration that user modules get, making stdlib types, protocols,
        // and implement block methods (List.push, Maybe.unwrap, etc.) available
        // for type checking user code.
        //
        // This is stdlib-agnostic: the compiler knows nothing about which types
        // or methods exist — it simply processes whatever .vr files were loaded.
        // ═══════════════════════════════════════════════════════════════════
        // Collect all stdlib modules (clone Arc handles to avoid borrow conflict).
        //
        // CRITICAL: Sort by key to ensure deterministic iteration order. self.modules is a
        // HashMap, so its natural iteration order is non-deterministic. When two modules
        // expose functions with the same short name (e.g. `core.base.memory::drop<T>` vs
        // `core.base.iterator.Transducer::drop<A>` (nested module)), non-deterministic
        // ordering lets different signatures "win" the top-level binding on different
        // runs, causing flaky L2 test failures.
        //
        // Shallower (fewer-dot) module keys are prioritized so top-level stdlib functions
        // beat nested-module helpers when short names collide.
        let mut stdlib_entries: Vec<_> = self.modules.iter()
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
                debug!("Registering {} stdlib modules into type checker (bootstrap mode)", stdlib_entries.len());

                // Pass S0a: Register all stdlib type names (module-scoped).
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    checker.register_all_type_names(&stdlib_mod.items);
                }

                // Pass S0b: Resolve all stdlib type definitions
                let mut resolution_stack = List::new();
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    for item in &stdlib_mod.items {
                        if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                            if let Err(e) = checker.resolve_type_definition(type_decl, &mut resolution_stack) {
                                debug!("Stdlib type resolution error: {:?}", e);
                            }
                        }
                    }
                }

                // Pass S1: Register stdlib function signatures
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

                // Pass S2: Register stdlib protocols
                for (module_path, stdlib_mod) in &stdlib_entries {
                    checker.set_current_module_path(module_path.clone());
                    for item in &stdlib_mod.items {
                        if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                            let _ = checker.register_protocol(protocol_decl);
                        }
                    }
                }
            }

            // Pass S3: ALWAYS register stdlib impl blocks (module-scoped).
            debug!("Registering stdlib impl blocks ({} modules)", stdlib_entries.len());
            for (module_path, stdlib_mod) in &stdlib_entries {
                checker.set_current_module_path(module_path.clone());
                for item in &stdlib_mod.items {
                    if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                        let _ = checker.register_impl_block(impl_decl);
                    }
                }
            }

            // Collect any unresolved placeholder names from stdlib so we can
            // exclude them from user-module placeholder verification
            let stdlib_placeholder_errors = checker.verify_no_placeholders();
            for err in &stdlib_placeholder_errors {
                debug!("Stdlib placeholder (expected): {:?}", err);
            }

            debug!("Stdlib registration complete");
        }

        // Restore the user-file module path so subsequent passes run in
        // the right resolution scope.
        checker.set_current_module_path(saved_module_path);

        // Signal transition to user code phase: variant short-name protection is relaxed
        // so user-defined types can shadow stdlib unit variants (e.g., Status.Pending).
        checker.set_user_code_phase();

        // Collect pre-existing placeholder names (from stdlib) to exclude from user verification
        let pre_existing_placeholders: std::collections::HashSet<String> = checker.verify_no_placeholders()
            .iter()
            .filter_map(|e| {
                if let verum_types::TypeError::UnresolvedPlaceholder { name, .. } = e {
                    Some(name.to_string())
                } else {
                    None
                }
            })
            .collect();

        // Two-pass type resolution for order-independent type definitions:
        //
        // This allows types to reference each other regardless of definition order:
        //   type SearchRequest is { sort_by: SortOrder };  // SortOrder used before defined
        //   type SortOrder is Relevance | Downloads;
        //
        // Pass 1a: Register all type names as placeholders
        // This makes all type names available for forward references
        checker.register_all_type_names(&module.items);

        // Pass 1b: Resolve all type definitions now that all names are known
        // This replaces placeholders with actual type definitions
        let mut resolution_stack = List::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                if let Err(e) = checker.resolve_type_definition(type_decl, &mut resolution_stack) {
                    let diag = type_error_to_diagnostic(&e, Some(self.session));
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        // Verify no placeholder types remain (indicates unresolved forward references)
        // Skip placeholders that were already unresolved from stdlib pre-registration
        for error in checker.verify_no_placeholders() {
            let is_stdlib_placeholder = if let verum_types::TypeError::UnresolvedPlaceholder { name, .. } = &error {
                pre_existing_placeholders.contains(name.as_str())
            } else {
                false
            };
            if !is_stdlib_placeholder {
                let diag = type_error_to_diagnostic(&error, Some(self.session));
                self.session.emit_diagnostic(diag);
            }
        }

        // Pass 1.9: Pre-register context declarations for forward references.
        // Without this, `using [ComputeDevice]` fails if ComputeDevice is
        // declared later in the same file. We register with full AST (not
        // just names) so method resolution works.
        for item in &module.items {
            if let verum_ast::ItemKind::Context(ctx_decl) = &item.kind {
                checker.register_stdlib_context_full(
                    verum_common::Text::from(ctx_decl.name.name.as_str()),
                    ctx_decl.clone(),
                );
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

        // Pass 3.5: Coherence checking (orphan rule, overlap detection)
        // Protocol coherence: at most one `implement Protocol for Type` per concrete
        // type in the entire dependency graph (orphan rule + overlap prevention).
        self.check_protocol_coherence(module)?;

        // Pass 3.6: Profile boundary enforcement
        // Profile boundaries: Application modules cannot depend on Systems-profile code.
        self.check_profile_boundaries(module)?;

        // Pass 4: Register function signatures (enables forward references)
        // This allows functions to call other functions defined later in the file:
        //   fn main() -> Int { fib(10) }  // fib is defined below
        //   fn fib(n: Int) -> Int { ... }
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

        // Pass 4.6: Pre-register const declarations (enables forward references)
        // Constants defined after functions should still be visible in function bodies.
        for item in &module.items {
            if let verum_ast::ItemKind::Const(const_decl) = &item.kind {
                checker.pre_register_const(const_decl);
            }
        }

        // Enable lenient context validation for files with @test annotations.
        // VCS test files use `using [Database]`, `using [Logger]`, etc. but don't
        // define these contexts — they expect a test harness to provide them at runtime.
        // In lenient mode, undefined contexts are silently accepted.
        //
        // Detection: Check source for `// @test:` comment header (VCS convention)
        // or `@test` AST attribute on any item.
        let has_test_annotation = {
            let has_ast_attr = module.items.iter().any(|item| {
                item.attributes.iter().any(|attr| attr.is_named("test"))
            });
            let has_comment_annotation = module.items.first().and_then(|item| {
                self.session.get_source(item.span.file_id)
            }).map(|sf| {
                // Check if the first few lines contain `// @test:` header
                sf.source.as_str().lines().take(10).any(|line| {
                    let trimmed = line.trim();
                    trimmed.starts_with("// @test:") || trimmed.starts_with("// @test ")
                })
            }).unwrap_or(false);
            has_ast_attr || has_comment_annotation
        };
        if has_test_annotation {
            checker.context_resolver_mut().set_lenient_contexts(true);
        }

        // Enable lenient contexts for stdlib files (core/**/*.vr).
        // Stdlib modules reference contexts from sibling modules that
        // may not be fully loaded in single-file check mode. Lenient
        // mode defers method-level validation to the call site where
        // the context is `provide`d with a concrete implementation.
        let is_stdlib_file = self.session.options().input
            .to_str()
            .map(|p| {
                p.contains("/core/") || p.contains("\\core\\")
                    || p.starts_with("core/") || p.starts_with("core\\")
            })
            .unwrap_or(false);
        if is_stdlib_file {
            checker.context_resolver_mut().set_lenient_contexts(true);
            checker.set_lenient_context_checking(true);
        }

        // For stdlib files, enable lenient mode that persists through
        // all scope changes. This flag is never reset by any operation
        // inside the type checker.
        if is_stdlib_file {
            checker.set_lenient_context_checking(true);
            checker.stdlib_single_file_mode = true;
        }

        // Pass 5: Type check all items (function bodies, impl blocks, etc.)
        for item in module.items.iter() {
            if let Err(type_error) = checker.check_item(item) {
                let diag = type_error_to_diagnostic(&type_error, Some(self.session));
                self.session.emit_diagnostic(diag);
            }
        }

        // Forward type checker diagnostics (exhaustiveness errors, warnings, etc.)
        for diag in checker.diagnostics() {
            self.session.emit_diagnostic(diag.clone());
        }
        checker.clear_diagnostics();

        // Drain deferred verification goals from the type-checker.
        // These are Type::Eq failures that the cubical bridge couldn't
        // resolve and universe constraints the local solver left
        // undecided. Store them on the pipeline for the
        // DependentVerifier phase (Phase 4.4) to consume.
        let deferred_goals = std::mem::take(&mut checker.deferred_verification_goals);
        if !deferred_goals.is_empty() {
            tracing::debug!(
                "Type checker deferred {} verification goal(s) for orchestrator",
                deferred_goals.len()
            );
        }
        self.deferred_verification_goals = deferred_goals;

        let elapsed = start.elapsed();
        let metrics = checker.metrics();

        info!(
            "Type checking completed in {:.2}ms ({} synth, {} check, {} unify)",
            elapsed.as_millis(),
            metrics.synth_count,
            metrics.check_count,
            metrics.unify_count
        );

        if self.session.options().verbose >= 2 {
            debug!("{}", metrics.report());
        }

        // Export type metadata for separate compilation (.vtyp file)
        if self.session.options().emit_types {
            let inherent = checker.get_inherent_methods();
            let methods_guard = inherent.read();
            let mut exporter = verum_types::type_exporter::TypeExporter::new(&methods_guard);
            let module_path = self.session.options().input
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "module".to_string());
            let exports = exporter.export_module(module, &module_path);
            let data = verum_types::type_exporter::serialize_module_exports(exports);
            if !data.is_empty() {
                let vtyp_path = self.session.options().input.with_extension("vtyp");
                if let Err(e) = std::fs::write(&vtyp_path, &data) {
                    debug!("Failed to write type metadata: {}", e);
                } else {
                    info!("Exported type metadata: {} ({} bytes)", vtyp_path.display(), data.len());
                }
            }
            drop(methods_guard);
        }

        // Store the type registry for later use by codegen
        // This enables closure parameter type inference without explicit annotations
        self.type_registry = Some(checker.take_type_registry());

        // Abort if errors
        self.session.abort_if_errors()?;

        Ok(())
    }

    /// Phase 3b: Dependency analysis for embedded constraints
    ///
    /// This phase validates that items are compatible with the target profile's
    /// constraints (no_alloc, no_std, embedded, cbgr_static_only, no_gpu).
    ///
    /// It runs after type checking to ensure all types are resolved before
    /// analyzing their dependency requirements.
    ///
    /// Validates items against target profile constraints (no_alloc, no_std, etc.).
    fn phase_dependency_analysis(&self, module: &Module) -> Result<()> {
        use crate::phases::dependency_analysis::DependencyAnalyzer;

        // Get target profile from compiler options
        let profile = self.session.options().to_target_profile();

        // Skip analysis if no constraints are active
        if !profile.no_alloc && !profile.no_std && !profile.embedded && !profile.cbgr_static_only && !profile.no_gpu {
            debug!("Dependency analysis skipped: no target constraints active");
            return Ok(());
        }

        debug!(
            "Running dependency analysis (profile: {}, no_alloc={}, no_std={}, embedded={})",
            profile.name, profile.no_alloc, profile.no_std, profile.embedded
        );

        let start = Instant::now();
        let mut analyzer = DependencyAnalyzer::new(profile);

        // Analyze all items in the module
        let errors = analyzer.analyze_module(module);

        let elapsed = start.elapsed();

        if !errors.is_empty() {
            // Convert TargetErrors to diagnostics
            let span_converter = |span: verum_ast::Span| -> verum_diagnostics::Span {
                self.session.convert_span(span)
            };

            for error in &errors {
                let diag = error.to_diagnostic(span_converter);
                self.session.emit_diagnostic(diag);
            }

            info!(
                "Dependency analysis completed in {:.2}ms: {} error(s)",
                elapsed.as_millis(),
                errors.len()
            );

            // Record phase metrics before aborting
            self.session
                .record_phase_metrics("Dependency Analysis", elapsed, module.items.len());

            // Abort compilation on target constraint violations
            self.session.abort_if_errors()?;
        } else {
            info!(
                "Dependency analysis completed in {:.2}ms: all items compatible with target",
                elapsed.as_millis()
            );
        }

        // Record phase metrics
        self.session
            .record_phase_metrics("Dependency Analysis", elapsed, module.items.len());

        Ok(())
    }

    /// Phase 4: Refinement verification
    fn phase_verify(&self, module: &Module) -> Result<()> {
        let _bc = verum_error::breadcrumb::enter("compiler.phase.verify", "");
        debug!("Running refinement verification");

        let start = Instant::now();
        let _smt_ctx = SmtContext::new();

        // Framework-hygiene preamble (#190): R1+R2+R3 discipline
        // for @framework / @enact annotations. Runs once per module
        // before the per-function refinement loop. R1/R2 produce
        // Warnings (recorded but non-blocking); R3 produces an
        // Error and short-circuits the verify phase.
        {
            let mut hygiene_pass = verum_verification::HygieneRecheckPass::new();
            let mut ctx = verum_verification::VerificationContext::new();
            use verum_verification::VerificationPass;
            let _ = hygiene_pass.run(module, &mut ctx);
            for d in hygiene_pass.diagnostics() {
                let builder = match d.severity {
                    verum_verification::HygieneSeverity::Error =>
                        verum_diagnostics::DiagnosticBuilder::error(),
                    verum_verification::HygieneSeverity::Warning =>
                        verum_diagnostics::DiagnosticBuilder::warning(),
                    verum_verification::HygieneSeverity::Info =>
                        verum_diagnostics::DiagnosticBuilder::warning(),
                };
                let diag = builder
                    .message(format!(
                        "framework-hygiene {}: {}",
                        d.rule,
                        d.message.as_str()
                    ))
                    .build();
                self.session.emit_diagnostic(diag);
            }
            if hygiene_pass.error_count() > 0 {
                debug!(
                    "Framework-hygiene errored ({} R3 violations); \
                     skipping refinement verification",
                    hygiene_pass.error_count()
                );
                return Ok(());
            }
        }

        // Extract functions that need verification (those with refinement types)
        let functions_to_verify: List<_> = module
            .items
            .iter()
            .filter_map(|item| {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    // Check if function has refinement types in params or return type
                    let has_refinements = func.params.iter().any(|p| {
                        if let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &p.kind {
                            self.has_refinement_type(ty)
                        } else {
                            false
                        }
                    }) || func
                        .return_type
                        .as_ref()
                        .map(|t| self.has_refinement_type(t))
                        .unwrap_or(false);

                    if has_refinements { Some(func) } else { None }
                } else {
                    None
                }
            })
            .collect();

        let num_to_verify = functions_to_verify.len();

        if num_to_verify == 0 {
            debug!("No functions with refinement types found");
            return Ok(());
        }

        info!(
            "Verifying {} function(s) with refinement types",
            num_to_verify
        );

        // ───────────────────────────────────────────────────────────
        // Parallel per-function verification (#100, Z3 + CVC5).
        //
        // Both backends are constructed per call inside
        // `verify_function_refinements` (`SmtContext::with_config(…)`
        // + `SmtRefinementVerifier::with_mode(…)`), so each rayon
        // worker gets its own thread-confined Z3 / CVC5 context. The
        // shared session-level `routing_stats` is `Arc<RoutingStats>`
        // with internally-atomic counters, safe to fan out.
        //
        // Counters and `CostTracker` are accumulated under a single
        // Mutex held only across the per-record append (microsecond-
        // scale) — ~zero contention versus the millisecond/second
        // SMT calls each worker spends.
        //
        // Opt-out: `VERUM_NO_PARALLEL_VERIFY=1` falls back to the
        // sequential loop. Useful for debugging non-deterministic
        // diagnostic ordering or pinning down a parallel-only
        // regression. Closure-cache and routing-stats are unaffected
        // — both are designed for concurrent access.
        // ───────────────────────────────────────────────────────────
        let timeout_ms = self.session.options().smt_timeout_secs * 1000;
        let timeout_secs = self.session.options().smt_timeout_secs;
        let parallel_verify = std::env::var("VERUM_NO_PARALLEL_VERIFY").is_err();

        let work: Vec<&verum_ast::decl::FunctionDecl> =
            functions_to_verify.iter().copied().collect();

        let aggregate = std::sync::Mutex::new((
            0usize, // num_verified
            0usize, // num_failed
            0usize, // num_timeout
            CostTracker::new(),
        ));

        let verify_one = |func: &verum_ast::decl::FunctionDecl| {
            debug!("Verifying function: {}", func.name.name);

            let verify_start = Instant::now();

            // K-rule preamble (#187): walk the function's refinement
            // types and run the kernel rules (currently
            // K-Refine-omega) BEFORE invoking SMT. K-rule failures
            // are hard formation errors per the trusted-base contract;
            // short-circuiting saves the SMT round and surfaces a
            // sharper diagnostic.
            let kernel_outcomes =
                verum_verification::KernelRecheck::recheck_function(func);
            let mut kernel_failure: Option<(verum_common::Text, String)> = None;
            for (label, outcome) in kernel_outcomes.iter() {
                if let Err(err) = outcome {
                    kernel_failure = Some((label.clone(), format!("{}", err)));
                    break;
                }
            }
            if let Some((label, msg)) = kernel_failure {
                let diag = verum_diagnostics::DiagnosticBuilder::error()
                    .message(format!(
                        "kernel-recheck failed for '{}': {} — {}",
                        func.name.name.as_str(),
                        label.as_str(),
                        msg,
                    ))
                    .build();
                self.session.emit_diagnostic(diag);
                let mut g = aggregate.lock().unwrap();
                g.1 += 1;
                return;
            }

            // Perform actual SMT-based refinement verification.
            let verification_result = self.verify_function_refinements(func, timeout_ms);
            let verify_elapsed = verify_start.elapsed();
            let func_name_text: verum_common::Text =
                func.name.as_str().to_string().into();

            match verification_result {
                Ok(true) => {
                    debug!(
                        "Verified function '{}' in {:.2}ms",
                        func.name.name,
                        verify_elapsed.as_millis()
                    );
                    let mut g = aggregate.lock().unwrap();
                    g.0 += 1;
                    g.3.record(verum_smt::VerificationCost::new(
                        func_name_text,
                        verify_elapsed,
                        true,
                    ));
                }
                Ok(false) => {
                    let mut g = aggregate.lock().unwrap();
                    g.1 += 1;
                    g.3.record(verum_smt::VerificationCost::new(
                        func_name_text,
                        verify_elapsed,
                        false,
                    ));
                }
                Err(e) => {
                    if verify_elapsed.as_secs() > timeout_secs {
                        warn!("Verification timeout for function: {}", func.name.name);
                        let diag = DiagnosticBuilder::new(Severity::Warning)
                            .message(format!(
                                "Verification timeout for function '{}' ({}s > {}s). Falling back to runtime checks.",
                                func.name.name,
                                verify_elapsed.as_secs(),
                                timeout_secs
                            ))
                            .build();
                        self.session.emit_diagnostic(diag);
                        let mut g = aggregate.lock().unwrap();
                        g.2 += 1;
                        g.3.record(verum_smt::VerificationCost::new(
                            func_name_text,
                            verify_elapsed,
                            false,
                        ));
                    } else {
                        warn!(
                            "Verification error for function '{}': {}",
                            func.name.name, e
                        );
                        let diag = DiagnosticBuilder::new(Severity::Error)
                            .message(format!(
                                "Verification error for function '{}': {}",
                                func.name.name, e
                            ))
                            .build();
                        self.session.emit_diagnostic(diag);
                        let mut g = aggregate.lock().unwrap();
                        g.1 += 1;
                        g.3.record(verum_smt::VerificationCost::new(
                            func_name_text,
                            verify_elapsed,
                            false,
                        ));
                    }
                }
            }
        };

        if parallel_verify && work.len() > 1 {
            use rayon::prelude::*;
            debug!(
                "  Verifying {} functions in parallel (rayon, Z3+CVC5)",
                work.len()
            );
            work.par_iter().copied().for_each(verify_one);
        } else {
            for func in &work {
                verify_one(func);
            }
        }

        let (num_verified, num_failed, num_timeout, cost_tracker) =
            aggregate.into_inner().unwrap();

        // Phase 4b: Verify theorem/lemma/axiom proofs via SMT
        self.verify_theorem_proofs(module)?;

        // Phase 4c: model-theoretic discharge of protocol axioms at
        // `implement` sites. For every `implement P for T { … }` block in
        // this module, collect P's axioms, Self-substitute into T's
        // concrete items, and discharge via explicit proof clauses or
        // auto_prove. Failures are emitted as warnings by default;
        // callers can elevate to errors via session options once their
        // stdlib surface is fully covered by explicit proofs or
        // SMT-closable obligations.
        self.verify_impl_axioms_for_module(module)?;

        let elapsed = start.elapsed();

        info!(
            "Verification complete: {} verified, {} timeout, {} failed in {:.2}s",
            num_verified,
            num_timeout,
            num_failed,
            elapsed.as_secs_f64()
        );

        if self.session.options().show_verification_costs {
            let report = cost_tracker.report();
            println!("{}", "\nVerification Cost Report:".bold());
            println!("{}", report);
        }

        // Bounds elimination analysis (verum_verification integration)
        // This identifies array accesses that can be proven safe at compile time
        if self.session.options().enable_bounds_elimination {
            self.run_bounds_elimination_analysis(module)?;
        }

        Ok(())
    }


    // verify_impl_axioms_for_module + find_protocol_decl extracted to
    // crate::pipeline::impl_axioms (#106 Phase 3 — pipeline.rs split).
    //
    // verify_theorem_proofs extracted to crate::pipeline::theorem_proofs
    // (#106 Phase 2 — pipeline.rs split).
    //
    // run_bounds_elimination_analysis + analyze_function_bounds_checks
    // + count_index_accesses + count_index_in_expr extracted to
    // crate::pipeline::bounds_stats (#106 Phase 1 — pipeline.rs split).
    //
    // verify_function_refinements + verify_return_refinement_smt +
    // verify_return_expr_smt + extract_return_values +
    // syntactic_check_refinement + extract_pattern_name +
    // has_refinement_type + SmtCheckResult extracted to
    // crate::pipeline::refinement_verify (#106 Phase 4 — pipeline.rs split).

    // check_profile_boundaries + extract_module_profile +
    // extract_profile_name + import_to_module_path +
    // get_module_profile_from_registry + type_to_text extracted to
    // crate::pipeline::profile_boundaries (#106 Phase 7).
    //
    // check_protocol_coherence + register_module_coherence_items
    // extracted to crate::pipeline::coherence (#106 Phase 6).
    //
    // extract_cfg_predicates / cfg_expr_to_predicate / expr_to_ident_string /
    // expr_to_string_literal extracted (and were previously duplicated) to
    // crate::cfg_eval — single source of truth (#106 Phase 5).

    // ==================== Phase 7: VBC Execution (Two-Tier Model v2.1) ====================
    //
    // This section handles the final execution step for VBC modules:
    // - Tier 0 (Interpreter): Direct VBC interpretation via `phase_interpret()`
    // - Tier 1 (AOT): VBC → LLVM IR → Native via `run_native_compilation()`
    //                 VBC → MLIR → GPU binaries via `run_mlir_aot()` (for @device(GPU))
    //
    // Architecture:
    // ```
    // Monomorphized VBC Module
    //       │
    //       ├─── Tier 0 ──► verum_vbc::interpreter::Interpreter
    //       │                 • ~1ms startup, ~20ns/call
    //       │                 • Full CBGR safety checks
    //       │                 • Used for: dev, REPL, debugging
    //       │
    //       └─── Tier 1 ──► CPU: VbcToLlvmLowering → LLVM IR → Native
    //                       GPU: VbcToMlirGpuLowering → MLIR → PTX/HSACO
    //                         • ~1s startup, ~1ns/call
    //                         • Proven-safe checks eliminated (0ns)
    //                         • Used for: production builds
    // ```
    //
    // Performance Characteristics:
    // | Tier        | Startup | Runtime    | Check Elimination |
    // |-------------|---------|------------|-------------------|
    // | Interpreter | ~1ms    | ~20ns/call | None (full CBGR)  |
    // | AOT (CPU)   | ~1s     | ~1ns/call  | 50-90% typical    |
    //
    // Two-tier execution: Interpreter (fast startup, full CBGR) and AOT (LLVM, 50-90% check elimination).

    /// Phase 4b: FFI Boundary Validation
    /// Phase 4c: Context System Validation
    ///
    /// Validates context usage: undeclared contexts, unprovided contexts,
    /// negative context violations (direct + transitive), and conflicts.
    /// Runs as warnings for now (errors would break existing code that
    /// doesn't yet declare all contexts).
    fn phase_context_validation(&self, module: &Module) {
        use crate::phases::context_validation::ContextValidationPhase;

        // Gate on the unified language-feature flag.
        // `[context] enabled = false` disables the whole DI/context
        // system, so there's nothing to validate.
        if !self.session.language_features().context_system_on() {
            return;
        }

        let t0 = Instant::now();
        // [context].unresolved_policy: "error" (default) → emit
        // errors; "warn" → downgrade to warnings; "allow" → suppress.
        let policy = self
            .session
            .language_features()
            .context
            .unresolved_policy
            .as_str()
            .to_string();

        let phase = ContextValidationPhase::new();
        match phase.validate_module_public(module) {
            Ok(warnings) => {
                let elapsed = t0.elapsed();
                if !warnings.is_empty() && policy != "allow" {
                    info!("Context validation: {} warnings ({:.2}ms)",
                        warnings.len(), elapsed.as_secs_f64() * 1000.0);
                    for w in warnings.iter() {
                        warn!("Context: {}", w.message());
                    }
                }
            }
            Err(errors) => {
                match policy.as_str() {
                    "allow" => {
                        // Silently suppress all context errors.
                    }
                    "warn" => {
                        for e in errors.iter() {
                            warn!("Context (downgraded to warning): {}", e.message());
                        }
                    }
                    _ => {
                        // "error" (default) — report as-is.
                        for e in errors.iter() {
                            warn!("Context: {}", e.message());
                        }
                    }
                }
            }
        }
    }

    /// Phase 4d: Send/Sync compile-time enforcement
    ///
    /// Validates that types crossing thread boundaries (spawn, Channel.send, Shared)
    /// satisfy Send/Sync bounds. Emits warnings (not errors) for now.
    fn phase_send_sync_validation(&self, module: &Module) {
        use crate::phases::send_sync_validation::SendSyncValidationPhase;

        let phase = SendSyncValidationPhase::new();
        let warnings = phase.validate_module(module);
        if !warnings.is_empty() {
            for w in warnings.iter() {
                warn!("Send/Sync: {}", w.message());
            }
        }
    }

    /// Phase 4b: FFI Boundary Validation
    ///
    /// `extern {}` blocks: warn-only (stdlib compatibility).
    /// `ffi {}` blocks: strict errors (user-written contracts must be correct).
    /// Scan a parsed module for `@device(gpu)` or `@device(GPU)` attributes on
    /// functions. Returns true if any GPU kernel annotation is found, enabling
    /// automatic GPU compilation without an explicit `--gpu` flag.
    ///
    /// This runs after Phase 2 (parsing) and before type checking so that the
    /// backend selection (CPU-only vs CPU+GPU) can be informed early.
    fn detect_gpu_kernels(module: &Module) -> bool {
        for item in module.items.iter() {
            // Check item-level attributes (outer attributes on the item)
            if Self::has_device_gpu_attr(&item.attributes) {
                return true;
            }
            // Check function-level attributes (on the FunctionDecl itself)
            if let ItemKind::Function(ref func) = item.kind {
                if Self::has_device_gpu_attr(&func.attributes) {
                    return true;
                }
            }
            // Check functions inside impl blocks
            if let ItemKind::Impl(ref impl_decl) = item.kind {
                for impl_item in impl_decl.items.iter() {
                    if Self::has_device_gpu_attr(&impl_item.attributes) {
                        return true;
                    }
                    if let verum_ast::decl::ImplItemKind::Function(ref func) = impl_item.kind {
                        if Self::has_device_gpu_attr(&func.attributes) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if a list of attributes contains `@device(gpu)` or `@device(GPU)`.
    fn has_device_gpu_attr(attrs: &List<verum_ast::Attribute>) -> bool {
        use verum_ast::expr::ExprKind;
        use verum_ast::ty::PathSegment;

        for attr in attrs.iter() {
            if attr.name.as_str() != "device" {
                continue;
            }
            // Check the first argument for "gpu" or "GPU" identifier
            if let Maybe::Some(ref args) = attr.args {
                if let Some(first_arg) = args.first() {
                    match &first_arg.kind {
                        // @device(gpu) — parsed as a path with single segment
                        ExprKind::Path(path) => {
                            if let Some(seg) = path.segments.first() {
                                if let PathSegment::Name(ident) = seg {
                                    let name = ident.name.as_str();
                                    if name.eq_ignore_ascii_case("gpu") {
                                        return true;
                                    }
                                }
                            }
                        }
                        // @device("gpu") — parsed as a string literal
                        ExprKind::Literal(lit) => {
                            if let verum_ast::literal::LiteralKind::Text(s) = &lit.kind {
                                if s.as_str().eq_ignore_ascii_case("gpu") {
                                    return true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        false
    }

    fn phase_ffi_validation(&self, module: &Module) -> Result<()> {
        use crate::phases::ffi_boundary::validate_module_ffi;
        let t0 = Instant::now();
        let result = validate_module_ffi(module, false);
        let elapsed = t0.elapsed();
        if result.functions_validated > 0 {
            info!("FFI validation: {} extern blocks, {} ffi boundaries, {} functions ({:.2}ms, {} diagnostics)",
                result.extern_blocks_validated, result.ffi_boundaries_validated,
                result.functions_validated, elapsed.as_secs_f64() * 1000.0, result.diagnostics.len());
        }
        // Warnings (from extern blocks)
        for diag in result.diagnostics.iter().filter(|d| d.severity() != Severity::Error) {
            warn!("FFI: {}", diag.message());
        }
        // Errors (from ffi blocks) — fail compilation
        let errors: Vec<_> = result.diagnostics.iter()
            .filter(|d| d.severity() == Severity::Error)
            .collect();
        if !errors.is_empty() {
            let mut msg = format!("{} FFI type safety error(s):\n", errors.len());
            for err in &errors {
                msg.push_str(&format!("  - {}\n", err.message()));
            }
            return Err(anyhow::anyhow!("{}", msg));
        }
        Ok(())
    }


    /// Count internal stdlib type registration errors.
    ///
    /// These are errors that occur when registering stdlib types, functions,
    /// protocols, and impl blocks into the type checker. They are suppressed
    /// during normal compilation but tracked for quality metrics.
    ///
    /// Returns (type_errors, func_errors, proto_errors, impl_errors, details)
    pub fn count_stdlib_type_errors(&mut self) -> (usize, usize, usize, usize, Vec<String>) {
        // Load stdlib modules if not yet loaded
        if self.modules.is_empty() {
            if let Err(e) = self.load_stdlib_modules() {
                warn!("Failed to load stdlib modules: {}", e);
                return (0, 0, 0, 0, vec![]);
            }
        }

        // Sort for deterministic iteration (self.modules is a HashMap):
        // shallower module keys come first so top-level stdlib functions beat
        // nested-module helpers when short names collide.
        let mut stdlib_entries: Vec<_> = self.modules.iter()
            .filter(|(k, _)| k.as_str().starts_with("core"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        stdlib_entries.sort_by(|(a, _), (b, _)| {
            let depth_a = a.as_str().matches('.').count();
            let depth_b = b.as_str().matches('.').count();
            depth_a.cmp(&depth_b).then_with(|| a.as_str().cmp(b.as_str()))
        });
        let stdlib_modules: Vec<_> = stdlib_entries.iter().map(|(_, v)| v.clone()).collect();

        if stdlib_modules.is_empty() {
            return (0, 0, 0, 0, vec![]);
        }

        let mut checker = verum_types::TypeChecker::with_minimal_context();
        checker.register_builtins();

        let mut type_errors = 0usize;
        let mut func_errors = 0usize;
        let mut proto_errors = 0usize;
        let mut impl_errors = 0usize;
        let mut details = Vec::new();

        // S0a: Register all stdlib type names
        for stdlib_mod in &stdlib_modules {
            checker.register_all_type_names(&stdlib_mod.items);
        }
        // S0b: Resolve stdlib type definitions
        let mut resolution_stack = verum_common::List::new();
        for stdlib_mod in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    if let Err(e) = checker.resolve_type_definition(type_decl, &mut resolution_stack) {
                        type_errors += 1;
                        details.push(format!("[TYPE] {}: {:?}", type_decl.name.name, e));
                    }
                }
            }
        }
        // S1: Register stdlib function signatures
        for stdlib_mod in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    if let Err(e) = checker.register_function_signature(func) {
                        func_errors += 1;
                        details.push(format!("[FUNC] {}: {:?}", func.name.name, e));
                    }
                }
            }
        }
        // S2: Register stdlib protocols
        for stdlib_mod in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                    if let Err(e) = checker.register_protocol(protocol_decl) {
                        proto_errors += 1;
                        details.push(format!("[PROTO] {}: {:?}", protocol_decl.name.name, e));
                    }
                }
            }
        }
        // S3: Register stdlib impl blocks
        for stdlib_mod in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                    if let Err(e) = checker.register_impl_block(impl_decl) {
                        impl_errors += 1;
                        let impl_name = match &impl_decl.kind {
                            verum_ast::decl::ImplKind::Inherent(ty) => format!("{:?}", ty),
                            verum_ast::decl::ImplKind::Protocol { protocol, for_type, .. } => {
                                format!("{:?} for {:?}", protocol, for_type)
                            }
                        };
                        details.push(format!("[IMPL] {}: {:?}", impl_name, e));
                    }
                }
            }
        }

        (type_errors, func_errors, proto_errors, impl_errors, details)
    }

    /// Count all stdlib errors including full body type checking.
    ///
    /// This runs the full type checker (including function bodies, impl blocks, etc.)
    /// on all core/ modules to identify all type errors. Returns a map of
    /// error category -> count, plus detailed error messages.
    ///
    /// Returns (total_errors, category_counts, details)
    pub fn count_stdlib_body_errors(&mut self) -> (usize, std::collections::HashMap<String, usize>, Vec<String>) {
        // Load stdlib modules if not yet loaded
        if self.modules.is_empty() {
            if let Err(e) = self.load_stdlib_modules() {
                warn!("Failed to load stdlib modules: {}", e);
                return (0, std::collections::HashMap::new(), vec![]);
            }
        }

        // Filter stdlib modules, excluding platform-specific modules for other OSes.
        // The @cfg(target_os = "X") on `module X;` declarations in mod.vr gates these
        // modules, but since file-based modules are loaded independently, we filter
        // by module path here.
        let host_os = {
            let raw = std::env::consts::OS;
            if raw == "darwin" { "macos" } else { raw }
        };
        let mut stdlib_modules: Vec<(Text, std::sync::Arc<Module>)> = self.modules.iter()
            .filter(|(k, _)| k.as_str().starts_with("core"))
            .filter(|(k, _)| {
                let mp = k.as_str();
                // Skip foreign platform modules
                if (mp.contains(".linux") && host_os != "linux") ||
                   (mp.contains(".windows") && host_os != "windows") ||
                   (mp.contains(".darwin") && host_os != "macos") ||
                   (mp.contains(".freebsd") && host_os != "freebsd") {
                    return false;
                }
                true
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        // Sort modules by path for deterministic type checking order.
        // Without this, HashMap iteration order varies between runs, causing
        // type variable bindings to differ and error counts to fluctuate.
        // Shallower (fewer-dot) module keys are prioritized so top-level stdlib
        // functions beat nested-module helpers when short names collide (e.g.
        // `core.base.memory::drop<T>` should win over
        // `core.base.iterator.Transducer::drop<A>`).
        stdlib_modules.sort_by(|(a, _), (b, _)| {
            let depth_a = a.as_str().matches('.').count();
            let depth_b = b.as_str().matches('.').count();
            depth_a.cmp(&depth_b).then_with(|| a.as_str().cmp(b.as_str()))
        });

        if stdlib_modules.is_empty() {
            return (0, std::collections::HashMap::new(), vec![]);
        }

        let mut checker = verum_types::TypeChecker::with_minimal_context();
        checker.register_builtins();

        // Enable lenient context resolution for stdlib body checking.
        // Context declarations (RandomSource, ComputeDevice, etc.) are defined
        // in core/ .vr files but may not be loaded into the context resolver.
        checker.set_lenient_contexts(true);

        // Configure type checker with module registry for cross-file resolution
        let registry = self.session.module_registry();
        checker.set_module_registry(registry.clone());

        // S0a: Register all stdlib type names
        for (_, stdlib_mod) in &stdlib_modules {
            checker.register_all_type_names(&stdlib_mod.items);
        }
        // S0b: Resolve stdlib type definitions
        let mut resolution_stack = verum_common::List::new();
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    let _ = checker.resolve_type_definition(type_decl, &mut resolution_stack);
                }
            }
        }
        // S1: Register stdlib function signatures
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    let _ = checker.register_function_signature(func);
                }
                // Register extern block (FFI) function signatures
                if let verum_ast::ItemKind::ExternBlock(extern_block) = &item.kind {
                    for func in &extern_block.functions {
                        let _ = checker.register_function_signature(func);
                    }
                }
            }
        }
        // S2: Register stdlib protocols
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                    let _ = checker.register_protocol(protocol_decl);
                }
            }
        }
        // S3: Register stdlib impl blocks
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                    let _ = checker.register_impl_block(impl_decl);
                }
            }
        }

        // S4: Process imports for each stdlib module
        // This resolves mount statements (e.g., mount super.constants.*)
        // so that imported names are available during body type checking.
        {
            let reg = registry.read();
            for (module_path, stdlib_mod) in &stdlib_modules {
                for item in &stdlib_mod.items {
                    if let verum_ast::ItemKind::Mount(import) = &item.kind {
                        let _ = checker.process_import(import, module_path.as_str(), &reg);
                    }
                }
            }
        }

        // S5: Register all const and static declarations as variables
        // This makes constants visible in function bodies.
        for (_, stdlib_mod) in &stdlib_modules {
            for item in &stdlib_mod.items {
                if let verum_ast::ItemKind::Const(const_decl) = &item.kind {
                    checker.pre_register_const(const_decl);
                }
                if let verum_ast::ItemKind::Static(static_decl) = &item.kind {
                    if let Ok(ty) = checker.ast_to_type(&static_decl.ty) {
                        checker.pre_register_static(&static_decl.name.name, ty);
                    }
                }
            }
        }

        // S6: Register platform intrinsics and module path stubs
        {
            use verum_types::{Type, TypeVar, TypeScheme};
            for &(name, pc) in &[("num_cpus",0),("getpagesize",0),("raw_read",3),("raw_write",3),("embedded_ctx_get",1),("embedded_ctx_set",2),("embedded_ctx_clear",1),("embedded_ctx_push_frame",0),("embedded_ctx_pop_frame",0),("__load_i64",1)] {
                let p: verum_common::List<Type> = (0..pc).map(|_| Type::Var(TypeVar::fresh())).collect();
                let r = Type::Var(TypeVar::fresh());
                let v: verum_common::List<TypeVar> = p.iter().filter_map(|t| if let Type::Var(v) = t { Some(*v) } else { None }).chain(std::iter::once(if let Type::Var(v) = &r { *v } else { unreachable!() })).collect();
                checker.ctx_env_insert(name, TypeScheme::poly(v, Type::function(p, r)));
            }
            for &s in &["core","darwin","linux","x86_64"] {
                let t = Type::Named { path: verum_ast::ty::Path::single(verum_ast::Ident::new(s, verum_ast::span::Span::default())), args: verum_common::List::new() };
                checker.ctx_env_insert(s, TypeScheme::mono(t));
            }
            for &n in &["GradientTape","GradientAccumulation","TIMER_STATUS","SysTlsError","ComputeDevice","RandomSource","SegmentError","CpuFeatures","SysStat","ChildSpecOpaque","Sockaddr","MemProt","MapFlags","VmAddress","ExitStatus","PathBuf","GlobalAllocator","GPUBuffer","FileDesc","Once","ProcessGroup","ExecutionEnv","ChildSpec","ContextSlots","ThreadControlBlock","Cotangent"] {
                let t = Type::Named { path: verum_ast::ty::Path::single(verum_ast::Ident::new(n, verum_ast::span::Span::default())), args: verum_common::List::new() };
                checker.ctx_define_type(n, t.clone());
                // Only register in env if not already present as a constructor function.
                // Types like FileDesc are newtypes with constructors registered at S0b.
                // Overwriting them here with TypeScheme::mono(Named) causes NotCallable errors.
                if checker.ctx_env_lookup(n).is_none() {
                    checker.ctx_env_insert(n, TypeScheme::mono(t));
                }
            }
        }

        // Now run full body type checking on functions only (not impl blocks, which
        // require additional setup).
        let mut total_errors = 0usize;
        let mut category_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut details = Vec::new();
        let mut checked_count = 0usize;
        let mut skipped_count = 0usize;

        // Create a CfgEvaluator for the current host platform.
        // This is used to skip items gated by @cfg predicates for other platforms.
        let cfg_eval = verum_ast::cfg::CfgEvaluator::new();

        for (module_path, stdlib_mod) in &stdlib_modules {
            let module_start = Instant::now();

            // Skip modules whose @cfg attributes don't match the current platform.
            if !cfg_eval.should_include(&stdlib_mod.attributes) {
                continue;
            }

            for item in &stdlib_mod.items {
                // Skip items gated by @cfg that don't match the current platform.
                if !cfg_eval.should_include(&item.attributes) {
                    continue;
                }
                // Critical fix mirroring `verum_vbc::should_compile_item`:
                // the parser puts `@cfg` on `Function.attributes` (inner
                // decl), not on `Item.attributes`.  The outer-only check
                // above silently bypasses every function-level @cfg gate.
                // Walk the inner FunctionDecl's attributes too.
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    if !cfg_eval.should_include(&func.attributes) {
                        continue;
                    }
                }

                // Only check functions (not impls - they need separate setup)
                let should_check = matches!(
                    &item.kind,
                    verum_ast::ItemKind::Function(_) |
                    verum_ast::ItemKind::Const(_) |
                    verum_ast::ItemKind::Static(_)
                );
                if !should_check {
                    continue;
                }

                // Skip if module already took too long (>10s per module = likely hanging)
                if module_start.elapsed().as_secs() > 10 {
                    skipped_count += 1;
                    continue;
                }

                checked_count += 1;
                let check_result = checker.check_item(item);

                if let Err(e) = check_result {
                    total_errors += 1;
                    let error_str = format!("{:?}", e);
                    // Categorize the error
                    let error_display = format!("{}", e);
                    let category = if error_str.contains("TypeNotFound") {
                        "TypeNotFound"
                    } else if error_str.contains("UnresolvedName") || error_str.contains("UndefinedVariable") {
                        "UnresolvedName"
                    } else if error_str.contains("TypeMismatch") || error_str.contains("Mismatch") || error_display.contains("Type mismatch") || error_display.contains("type mismatch") {
                        "TypeMismatch"
                    } else if error_str.contains("NotCallable") || error_display.contains("not a function type") {
                        "NotCallable"
                    } else if error_str.contains("FieldNotFound") || (error_display.contains("field") && error_display.contains("not found")) {
                        "FieldNotFound"
                    } else if error_str.contains("MethodNotFound") || (error_display.contains("method") && error_display.contains("not found")) {
                        "MethodNotFound"
                    } else if error_str.contains("ArityMismatch") || error_display.contains("wrong number of arguments") || error_display.contains("Function requires") || error_display.contains("Function accepts") {
                        "ArityMismatch"
                    } else if error_str.contains("UnresolvedPlaceholder") {
                        "UnresolvedPlaceholder"
                    } else if error_str.contains("NotImplemented") || error_str.contains("Unsupported") || error_display.contains("Unknown variant constructor") {
                        "NotImplemented"
                    } else if error_str.contains("unbound variable") || error_display.contains("unbound variable") {
                        "UnboundVariable"
                    } else if error_str.contains("super keyword") {
                        "SuperKeyword"
                    } else if error_str.contains("undefined context") || error_display.contains("undefined context") || error_display.contains("missing context") {
                        "UndefinedContext"
                    } else if error_display.contains("Pattern expects") || error_display.contains("Expected reference type for reference pattern") {
                        "PatternError"
                    } else if error_str.contains("infinite type") || error_display.contains("recursion") || error_display.contains("stack overflow") {
                        "InfiniteType"
                    } else if error_str.contains("invalid cast") || error_str.contains("InvalidCast") || error_display.contains("invalid cast") || error_display.contains("cannot cast") {
                        "InvalidCast"
                    } else if error_display.contains("Cannot iterate") {
                        "IterationError"
                    } else if error_display.contains("Cannot dereference") {
                        "DerefError"
                    } else if error_display.contains("Cannot access field") {
                        "FieldAccessError"
                    } else {
                        "Other"
                    };
                    *category_counts.entry(category.to_string()).or_insert(0) += 1;

                    // Extract item name for context
                    let item_name = match &item.kind {
                        verum_ast::ItemKind::Function(f) => format!("fn {}", f.name.name),
                        verum_ast::ItemKind::Impl(impl_decl) => {
                            match &impl_decl.kind {
                                verum_ast::decl::ImplKind::Inherent(ty) => format!("impl {:?}", ty),
                                verum_ast::decl::ImplKind::Protocol { protocol, for_type, .. } => {
                                    format!("impl {:?} for {:?}", protocol, for_type)
                                }
                            }
                        }
                        verum_ast::ItemKind::Const(c) => format!("const {}", c.name.name),
                        verum_ast::ItemKind::Static(s) => format!("static {}", s.name.name),
                        _ => format!("{:?}", std::mem::discriminant(&item.kind)),
                    };

                    if details.len() < 500 {
                        details.push(format!("[{}] {} in {}: {}", category, item_name, module_path.as_str(), e));
                    }
                }
            }
        }

        // Add summary stats to details
        details.push(format!("[SUMMARY] checked={}, skipped_timeout={}, errors={}", checked_count, skipped_count, total_errors));

        (total_errors, category_counts, details)
    }

    /// Get the stdlib registry (for interpreter/JIT)
    pub fn get_stdlib_registry(&self) -> Option<&crate::phases::phase0_stdlib::StdlibRegistry> {
        self.stdlib_artifacts.as_ref().map(|a| &a.registry)
    }

    /// Get the stdlib static library path (for AOT linking)
    pub fn get_stdlib_static_lib(&self) -> Option<&PathBuf> {
        self.stdlib_artifacts.as_ref().map(|a| &a.static_library)
    }

    /// Get the stdlib LLVM bitcode path (for LTO)
    pub fn get_stdlib_bitcode(&self) -> Option<&PathBuf> {
        self.stdlib_artifacts.as_ref().map(|a| &a.bitcode_library)
    }


    /// Create a pipeline for MLIR JIT mode
    pub fn new_mlir_jit(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::MlirJit;
        pipeline
    }

    /// Create a pipeline for MLIR AOT mode
    pub fn new_mlir_aot(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::MlirAot;
        pipeline
    }

    // ==================== VBC → LLVM COMPILATION ====================
    //
    // These methods implement the CPU compilation path using the new
    // VBC → LLVM IR lowering infrastructure.
    //
    // Architecture:
    //   AST → VBC (verum_vbc) → LLVM IR (verum_llvm) → Native Code
    //
    // This path is used for:
    // - Tier 1/2 JIT: Hot path optimization
    // - Tier 3 AOT: Ahead-of-time compilation to native executables

    /// Create a pipeline for VBC → LLVM JIT mode.
    ///
    /// This mode compiles Verum source through VBC to LLVM IR, then executes
    /// immediately using LLVM's JIT engine. This is the preferred path for:
    /// - Development/debugging with fast iteration
    /// - Hot path optimization (Tier 1/2)
    pub fn new_vbc_jit(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::Jit;
        pipeline
    }

    /// Create a pipeline for VBC → LLVM AOT mode.
    ///
    /// This mode compiles Verum source through VBC to LLVM IR, then generates
    /// a native executable. This is the preferred path for:
    /// - Production builds (Tier 3)
    /// - Distribution as standalone executables
    pub fn new_vbc_aot(session: &'s mut Session) -> Self {
        let mut pipeline = Self::new(session);
        pipeline.mode = CompilationMode::Aot;
        pipeline
    }

    /// Run VBC → LLVM JIT compilation and execution.
    ///
    /// This is the main entry point for the CPU JIT compilation path:
    /// 1. Parse source to AST
    /// 2. Type check
    /// 3. CBGR analysis (determines tier for each reference)
    /// 4. Compile AST to VBC
    /// 5. Lower VBC to LLVM IR
    /// 6. Execute via LLVM JIT
    ///
    /// # Returns
    ///
    /// Returns the exit code from the main function, or an error if compilation fails.
    pub fn run_vbc_jit(&mut self) -> Result<i64> {
        let start = Instant::now();
        info!("Starting VBC → LLVM JIT compilation");

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let module = self.phase_parse(file_id)?;

        // Phase 3: Type check
        self.phase_type_check(&module)?;

        // Phase 4: Refinement verification (if enabled)
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 5: CBGR analysis
        self.phase_cbgr_analysis(&module)?;

        // Phase 6: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(&module)?;

        // Phase 6.5: Compilation path analysis
        let target_config = PathTargetConfig::cpu_only(); // CPU-only for now
        self.analyze_compilation_paths(&vbc_module, &target_config)?;

        // Phase 7: Lower VBC to LLVM IR (CPU path)
        let llvm_context = verum_codegen::llvm::verum_llvm::context::Context::create();
        let (llvm_module, stats) = self.lower_vbc_to_llvm(&llvm_context, &vbc_module)?;

        info!(
            "VBC → LLVM lowering complete: {} functions, {} instructions, {:.1}% CBGR elimination",
            stats.functions_lowered,
            stats.instructions_lowered,
            stats.elimination_rate() * 100.0
        );

        // Phase 8: Execute via JIT
        let result = self.execute_llvm_jit(&llvm_module, &module)?;

        let elapsed = start.elapsed();
        info!(
            "VBC JIT execution completed in {:.2}s with exit code {}",
            elapsed.as_secs_f64(),
            result
        );

        Ok(result)
    }

    /// Run VBC → LLVM AOT compilation.
    ///
    /// This is the main entry point for the CPU AOT compilation path:
    /// 1. Parse source to AST
    /// 2. Type check
    /// 3. CBGR analysis
    /// 4. Compile AST to VBC
    /// 5. Lower VBC to LLVM IR
    /// 6. Optimize LLVM IR
    /// 7. Generate object file
    /// 8. Link into executable
    ///
    /// # Returns
    ///
    /// Returns the path to the generated executable.
    pub fn run_vbc_aot(&mut self) -> Result<PathBuf> {
        let start = Instant::now();
        info!("Starting VBC → LLVM AOT compilation");

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let module = self.phase_parse(file_id)?;

        // Phase 3: Type check
        self.phase_type_check(&module)?;

        // Phase 4: Refinement verification (if enabled)
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 5: CBGR analysis
        self.phase_cbgr_analysis(&module)?;

        // Phase 6: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(&module)?;

        // Phase 6.5: Compilation path analysis
        let target_config = PathTargetConfig::cpu_only(); // CPU-only for now
        self.analyze_compilation_paths(&vbc_module, &target_config)?;

        // Phase 7: Lower VBC to LLVM IR (CPU path)
        let llvm_context = verum_codegen::llvm::verum_llvm::context::Context::create();
        let (llvm_module, stats) = self.lower_vbc_to_llvm(&llvm_context, &vbc_module)?;

        info!(
            "VBC → LLVM lowering complete: {} functions, {} instructions, {:.1}% CBGR elimination",
            stats.functions_lowered,
            stats.instructions_lowered,
            stats.elimination_rate() * 100.0
        );

        // Phase 8: Generate native executable
        let output_path = self.generate_native_from_llvm(&llvm_module)?;

        let elapsed = start.elapsed();
        info!(
            "VBC AOT compilation completed in {:.2}s: {}",
            elapsed.as_secs_f64(),
            output_path.display()
        );

        Ok(output_path)
    }

    /// Lower a VBC module to LLVM IR.
    ///
    /// This is the core of the CPU compilation path. It translates VBC bytecode
    /// instructions to LLVM IR, applying tier-aware CBGR optimizations.
    fn lower_vbc_to_llvm<'ctx>(
        &self,
        llvm_context: &'ctx verum_codegen::llvm::verum_llvm::context::Context,
        vbc_module: &std::sync::Arc<verum_vbc::module::VbcModule>,
    ) -> Result<(verum_codegen::llvm::verum_llvm::module::Module<'ctx>, LlvmLoweringStats)> {
        let input_path = &self.session.options().input;
        let module_name = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main");

        // Configure lowering based on session options
        let opt_level = self.session.options().optimization_level;
        let config = if opt_level >= 2 {
            LlvmLoweringConfig::release(module_name)
        } else if opt_level == 0 {
            LlvmLoweringConfig::debug(module_name)
        } else {
            LlvmLoweringConfig::new(module_name).with_opt_level(opt_level)
        };

        // Wire debug info and coverage flags
        let config = config
            .with_debug_info(self.session.options().debug_info)
            .with_coverage(self.session.options().coverage);

        // Set target triple for the host
        let config = config.with_target(verum_codegen::llvm::verum_llvm::targets::TargetMachine::get_default_triple().as_str().to_string_lossy());

        // Wire the AOT permission policy into lowering. `None` is the
        // trusted-application default — `PermissionAssert` is elided.
        // `Some` makes the lowerer bake the policy into every
        // permission gate at compile time, sealing the resolved
        // grants in the binary so `--aot` runs of script-shaped
        // sources enforce identically to the interpreter.
        let config = config.with_permission_policy(self.session.aot_permission_policy());

        info!("  Lowering VBC to LLVM IR (opt level: {})", opt_level);

        // Run CBGR escape analysis on decoded VBC functions
        let escape_result = {
            use verum_vbc::cbgr_analysis::VbcEscapeAnalyzer;
            let analyzer = VbcEscapeAnalyzer::new();
            let functions: Vec<verum_vbc::VbcFunction> = vbc_module.functions.iter()
                .filter_map(|f| {
                    f.instructions.as_ref().map(|instrs| {
                        verum_vbc::VbcFunction::new(f.clone(), instrs.clone())
                    })
                })
                .collect();
            let result = analyzer.analyze(&functions);
            info!("  CBGR escape analysis: {} refs analyzed, {} promoted to tier1 ({:.1}%)",
                result.stats.total_refs,
                result.stats.promoted_to_tier1,
                result.stats.promotion_rate());
            result
        };

        let mut lowering = VbcToLlvmLowering::new(llvm_context, config);
        lowering.set_escape_analysis(escape_result);
        lowering.lower_module(vbc_module)
            .map_err(|e| anyhow::anyhow!("VBC → LLVM lowering failed: {}", e))?;

        let stats = lowering.stats().clone();
        let llvm_module = lowering.into_module();

        // Optionally dump IR for debugging
        if self.session.options().verbose > 1 {
            debug!("Generated LLVM IR:\n{}", llvm_module.print_to_string().to_string_lossy());
        }

        Ok((llvm_module, stats))
    }

    /// Execute an LLVM module using the JIT engine.
    fn execute_llvm_jit(
        &self,
        llvm_module: &verum_codegen::llvm::verum_llvm::module::Module<'_>,
        _ast_module: &Module, // Reserved for future: extract metadata for runtime
    ) -> Result<i64> {
        info!("  Creating LLVM JIT execution engine");

        // Create JIT execution engine
        let execution_engine = llvm_module
            .create_jit_execution_engine(verum_codegen::llvm::verum_llvm::OptimizationLevel::Default)
            .map_err(|e| anyhow::anyhow!("Failed to create JIT engine: {}", e))?;

        // Look up main function
        // SAFETY: get_function requires unsafe because it can return arbitrary function pointers.
        // We're looking for known entry points that we've compiled with expected signatures.
        if let Ok(main_fn) = unsafe {
            execution_engine.get_function::<unsafe extern "C" fn() -> i64>("main")
        } {
            info!("  Executing main function via LLVM JIT");
            // SAFETY: We've compiled main with the expected signature
            let result = unsafe { main_fn.call() };
            Ok(result)
        } else {
            // Try _start as fallback
            if let Ok(start_fn) = unsafe {
                execution_engine.get_function::<unsafe extern "C" fn()>("_start")
            } {
                info!("  Executing _start function via LLVM JIT");
                // SAFETY: We've compiled _start with the expected signature
                unsafe { start_fn.call() };
                Ok(0)
            } else {
                Err(anyhow::anyhow!("No main or _start function found"))
            }
        }
    }

    /// Generate a native executable from an LLVM module.
    fn generate_native_from_llvm(
        &self,
        llvm_module: &verum_codegen::llvm::verum_llvm::module::Module<'_>,
    ) -> Result<PathBuf> {
        use verum_codegen::llvm::verum_llvm::targets::{
            InitializationConfig, Target, TargetMachine, RelocMode, CodeModel, FileType,
        };

        // Initialize LLVM targets ONCE per process.
        {
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| {
                let _ = Target::initialize_native(&InitializationConfig::default());
            });
        }

        // Get input path and determine output paths
        let input_path = &self.session.options().input;
        let project_root = self.get_project_root(input_path);

        let profile = if self.session.options().optimization_level >= 2 {
            "release"
        } else {
            "debug"
        };

        let target_dir = project_root.join("target");
        let profile_dir = target_dir.join(profile);
        let build_dir = target_dir.join("build");

        std::fs::create_dir_all(&profile_dir)?;
        std::fs::create_dir_all(&build_dir)?;

        let module_name = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main");

        let output_path = if self.session.options().output.to_str().unwrap_or("").is_empty() {
            profile_dir.join(if cfg!(windows) {
                format!("{}.exe", module_name)
            } else {
                module_name.to_string()
            })
        } else {
            self.session.options().output.clone()
        };

        // Create target machine
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple)
            .map_err(|e| anyhow::anyhow!("Failed to get target: {}", e))?;

        let opt_level = match self.session.options().optimization_level {
            0 => verum_codegen::llvm::verum_llvm::OptimizationLevel::None,
            1 => verum_codegen::llvm::verum_llvm::OptimizationLevel::Less,
            2 => verum_codegen::llvm::verum_llvm::OptimizationLevel::Default,
            _ => verum_codegen::llvm::verum_llvm::OptimizationLevel::Aggressive,
        };

        let target_machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                opt_level,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| anyhow::anyhow!("Failed to create target machine"))?;

        // Write object file
        let obj_path = build_dir.join(format!("{}.o", module_name));
        info!("  Writing object file to {}", obj_path.display());

        target_machine
            .write_to_file(llvm_module, FileType::Object, &obj_path)
            .map_err(|e| anyhow::anyhow!("Failed to write object file: {}", e))?;

        // Generate runtime stubs
        let runtime_stubs_path = self.generate_runtime_stubs(&build_dir)?;
        let runtime_obj = self.compile_c_file(&runtime_stubs_path, &build_dir)?;

        // Link into executable
        info!("  Linking executable to {}", output_path.display());
        self.link_executable(&[obj_path, runtime_obj], &output_path)?;

        Ok(output_path)
    }

    /// Analyze VBC module to determine compilation paths for each function.
    ///
    /// This phase analyzes the VBC bytecode to determine whether functions
    /// should be compiled via the CPU path (LLVM IR) or GPU path (MLIR).
    ///
    /// # Arguments
    ///
    /// * `vbc_module` - The VBC module to analyze
    /// * `target_config` - Target configuration (GPU availability, thresholds, etc.)
    ///
    /// # Returns
    ///
    /// Returns Ok(()) if all functions can be compiled, or an error if GPU
    /// compilation is required but unavailable.
    fn analyze_compilation_paths(
        &self,
        vbc_module: &std::sync::Arc<verum_vbc::module::VbcModule>,
        target_config: &PathTargetConfig,
    ) -> Result<()> {
        use tracing::{debug, warn};

        let mut cpu_count = 0usize;
        let mut gpu_count = 0usize;
        let mut hybrid_count = 0usize;
        let mut total_tensor_ops = 0usize;
        let mut total_gpu_ops = 0usize;

        for func_desc in &vbc_module.functions {
            let func_name = vbc_module
                .strings
                .get(func_desc.name)
                .unwrap_or("<unknown>");

            // Analyze the function
            let analysis = match analyze_function(func_desc, vbc_module) {
                Ok(a) => a,
                Err(e) => {
                    debug!(
                        "  Function '{}': analysis skipped ({})",
                        func_name,
                        e
                    );
                    // Skip functions that can't be analyzed (e.g., no bytecode)
                    cpu_count += 1;
                    continue;
                }
            };

            // Determine compilation path
            let path = determine_compilation_path(&analysis, target_config);

            // Track statistics
            total_tensor_ops += analysis.tensor_op_count;
            total_gpu_ops += analysis.gpu_op_count;

            match &path {
                CompilationPath::Cpu => {
                    cpu_count += 1;
                    debug!(
                        "  Function '{}': CPU path ({} instructions, {} tensor ops)",
                        func_name, analysis.instruction_count, analysis.tensor_op_count
                    );
                }
                CompilationPath::Gpu => {
                    gpu_count += 1;
                    debug!(
                        "  Function '{}': GPU path ({} GPU ops, {} tensor ops)",
                        func_name, analysis.gpu_op_count, analysis.tensor_op_count
                    );

                    // Currently, GPU path requires MLIR which isn't wired for VBC yet
                    if !target_config.has_gpu {
                        warn!(
                            "Function '{}' requires GPU but no GPU target available, falling back to CPU",
                            func_name
                        );
                    }
                }
                CompilationPath::Hybrid { gpu_regions } => {
                    hybrid_count += 1;
                    debug!(
                        "  Function '{}': Hybrid path ({} CPU + {} GPU regions)",
                        func_name,
                        analysis.instruction_count - analysis.gpu_op_count,
                        gpu_regions.len()
                    );
                }
            }
        }

        info!(
            "Compilation path analysis: {} CPU, {} GPU, {} hybrid functions ({} tensor ops, {} GPU ops total)",
            cpu_count, gpu_count, hybrid_count, total_tensor_ops, total_gpu_ops
        );

        // For now, we only support CPU path - error on GPU-only functions
        if gpu_count > 0 && !target_config.has_gpu {
            warn!(
                "{} functions require GPU compilation but will use CPU fallback",
                gpu_count
            );
        }

        Ok(())
    }
}

// ==================== MACRO EXPANSION ====================

/// Types of macro/meta function arguments
#[derive(Debug, Clone)]
enum InvocationArgs {
    /// Traditional macro args (unparsed token tree from macro!())
    MacroArgs(verum_ast::expr::MacroArgs),
    /// Meta function args (parsed expressions from @meta())
    MetaArgs(List<verum_ast::expr::Expr>),
}

/// A macro or meta function invocation found in the AST
#[derive(Debug, Clone)]
struct MacroInvocation {
    /// Name of the macro/meta function being invoked
    macro_name: Text,
    /// Arguments to the invocation
    args: InvocationArgs,
    /// Span of the invocation
    span: Span,
}

/// Visitor that collects and expands macro invocations
struct MacroExpander<'a> {
    /// Reference to the meta registry
    registry: &'a MetaRegistry,
    /// Meta execution context
    context: crate::meta::MetaContext,
    /// Current module path
    module_path: Text,
    /// Collected macro invocations
    expansions: List<MacroInvocation>,
}

impl<'a> MacroExpander<'a> {
    /// Collect macro invocations from an item
    fn collect_macro_invocations(&mut self, item: &Item) {
        use verum_ast::visitor::Visitor;
        self.visit_item(item);
    }

    /// Expand a macro or meta function invocation
    fn expand_macro(&mut self, invocation: &MacroInvocation) -> Result<List<Item>> {
        use crate::meta::ConstValue;

        let module_path_std = Text::from(self.module_path.as_str());
        let macro_name_text = Text::from(invocation.macro_name.as_str());


        // Try to resolve as a user-defined meta function first (@meta_name())
        // This handles functions declared with `meta fn name() -> TokenStream { ... }`
        if let Some(meta_fn) = self.registry.resolve_meta_call(&module_path_std, &macro_name_text) {
            debug!(
                "Expanding user-defined meta function '@{}'",
                invocation.macro_name.as_str()
            );

            // Convert args to ConstValue based on the invocation type
            let args = match &invocation.args {
                InvocationArgs::MetaArgs(exprs) => {
                    // Meta function calls have parsed expressions as arguments
                    // For now, we convert them to ConstValue::Expr for the meta function to use
                    exprs.iter()
                        .map(|e| ConstValue::Expr(e.clone()))
                        .collect::<Vec<_>>()
                }
                InvocationArgs::MacroArgs(macro_args) => {
                    // Traditional macro calls have unparsed token trees
                    vec![ConstValue::Text(macro_args.tokens.clone())]
                }
            };

            // Execute the meta function
            let result = self
                .context
                .execute_user_meta_fn(&meta_fn, args)
                .map_err(|e| {
                    anyhow::anyhow!("Meta function execution failed: {}", e)
                })?;

            // For expression-level meta functions, just return empty items
            // The expansion should happen inline where the expression was
            // For now, we log the result and return empty
            debug!(
                "Meta function '{}' returned: {:?}",
                invocation.macro_name.as_str(),
                result.type_name()
            );

            // Return empty items - the expansion is used inline, not as new items
            return Ok(List::new());
        }

        // Otherwise, try to resolve as a traditional macro (macro!())
        let macro_def = match self
            .registry
            .resolve_macro(&module_path_std, &macro_name_text)
        {
            Maybe::Some(def) => def,
            Maybe::None => {
                // Neither a meta function nor a macro was found
                // This might be a built-in meta function like @cfg, @const, etc.
                // Those are handled elsewhere, so we skip them here
                debug!(
                    "Skipping unknown/built-in meta function: {}",
                    invocation.macro_name.as_str()
                );
                return Ok(List::new());
            }
        };

        debug!(
            "Expanding macro '{}' using expander '{}'",
            invocation.macro_name.as_str(),
            macro_def.expander.as_str()
        );

        // Look up the expander meta function
        let meta_fn = match self
            .registry
            .resolve_meta_call(&macro_def.module, &macro_def.expander)
        {
            Some(func) => func,
            None => {
                return Err(anyhow::anyhow!(
                    "Meta function '{}' not found for macro expansion",
                    macro_def.expander.as_str()
                ));
            }
        };

        // Convert macro arguments to ConstValue
        let args = match &invocation.args {
            InvocationArgs::MacroArgs(macro_args) => {
                vec![ConstValue::Text(macro_args.tokens.clone())]
            }
            InvocationArgs::MetaArgs(exprs) => {
                vec![ConstValue::Text(format!("{:?}", exprs).into())]
            }
        };

        // Execute the meta function
        let result = self
            .context
            .execute_user_meta_fn(&meta_fn, args)
            .map_err(|e| anyhow::anyhow!("Meta function execution failed: {}", e))?;

        // Convert result back to AST items
        // The result should be ConstValue::Items(List<ConstValue::Expr>)
        match result {
            ConstValue::Items(items) => {
                // Convert items to AST items by parsing each ConstValue
                debug!("Generated {} items from macro expansion", items.len());

                let mut ast_items = List::new();
                for (idx, const_val) in items.iter().enumerate() {
                    match const_val {
                        ConstValue::Expr(expr) => {
                            // Convert the expression to a token stream, then parse as an item
                            use crate::quote::ToTokens;
                            let token_stream = expr.into_token_stream();

                            match token_stream.parse_as_item() {
                                Ok(item) => {
                                    ast_items.push(item);
                                }
                                Err(e) => {
                                    return Err(anyhow::anyhow!(
                                        "Failed to parse item {} from macro expansion: {}",
                                        idx,
                                        e
                                    ));
                                }
                            }
                        }
                        ConstValue::Text(code) => {
                            // Parse text as an item directly
                            let file_id = invocation.span.file_id;
                            match crate::quote::TokenStream::from_str(code.as_str(), file_id) {
                                Ok(token_stream) => match token_stream.parse_as_item() {
                                    Ok(item) => {
                                        ast_items.push(item);
                                    }
                                    Err(e) => {
                                        return Err(anyhow::anyhow!(
                                            "Failed to parse text item {} from macro expansion: {}",
                                            idx,
                                            e
                                        ));
                                    }
                                },
                                Err(e) => {
                                    return Err(anyhow::anyhow!(
                                        "Failed to tokenize text item {} from macro expansion: {}",
                                        idx,
                                        e
                                    ));
                                }
                            }
                        }
                        _ => {
                            return Err(anyhow::anyhow!(
                                "Invalid item type in macro expansion at index {}: expected Expr or Text, found {}",
                                idx,
                                const_val.type_name()
                            ));
                        }
                    }
                }

                debug!(
                    "Successfully converted {} items from macro expansion",
                    ast_items.len()
                );
                Ok(ast_items)
            }
            ConstValue::Expr(expr) => {
                // Single expression - try to parse it as an item
                debug!("Generated single expression from macro expansion");

                use crate::quote::ToTokens;
                let token_stream = expr.into_token_stream();

                match token_stream.parse_as_item() {
                    Ok(item) => {
                        let mut result_list = List::new();
                        result_list.push(item);
                        debug!("Successfully parsed expression as item");
                        Ok(result_list)
                    }
                    Err(e) => {
                        // If it can't be parsed as an item, it might be meant for expression context
                        // In that case, we should probably error out since we're in item context
                        Err(anyhow::anyhow!(
                            "Macro expansion returned an expression that cannot be used as an item: {}",
                            e
                        ))
                    }
                }
            }
            ConstValue::Text(code) => {
                // Text result - parse it as code
                debug!("Generated text from macro expansion, parsing as items");

                let file_id = invocation.span.file_id;
                match crate::quote::TokenStream::from_str(code.as_str(), file_id) {
                    Ok(token_stream) => {
                        // Try to parse as a single item
                        match token_stream.parse_as_item() {
                            Ok(item) => {
                                let mut result_list = List::new();
                                result_list.push(item);
                                debug!("Successfully parsed text as item");
                                Ok(result_list)
                            }
                            Err(e) => Err(anyhow::anyhow!(
                                "Failed to parse generated text as item: {}",
                                e
                            )),
                        }
                    }
                    Err(e) => Err(anyhow::anyhow!("Failed to tokenize generated text: {}", e)),
                }
            }
            _ => Err(anyhow::anyhow!(
                "Macro expansion returned unexpected type: {}. Expected Items, Expr, or Text",
                result.type_name()
            )),
        }
    }
}

impl<'a> verum_ast::visitor::Visitor for MacroExpander<'a> {
    fn visit_expr(&mut self, expr: &verum_ast::expr::Expr) {
        use verum_ast::expr::ExprKind;
        use verum_ast::visitor::walk_expr;

        match &expr.kind {
            // Check if this is a traditional macro call (name!())
            ExprKind::MacroCall { path, args } => {
                // Extract macro name from path
                if let Some(ident) = path.as_ident() {
                    let macro_name = Text::from(ident.as_str());

                    debug!("Found macro invocation: {}", macro_name.as_str());

                    // Record this invocation
                    self.expansions.push(MacroInvocation {
                        macro_name,
                        args: InvocationArgs::MacroArgs(args.clone()),
                        span: expr.span,
                    });
                }
            }

            // Check if this is a meta function call (@name())
            // User-defined meta functions use this syntax
            ExprKind::MetaFunction { name, args } => {
                let meta_name = Text::from(name.name.as_str());

                debug!("Found meta function invocation: @{}", meta_name.as_str());

                // Record this invocation for expansion
                // Note: We check if this is a user-defined meta function in expand_macro
                self.expansions.push(MacroInvocation {
                    macro_name: meta_name,
                    args: InvocationArgs::MetaArgs(args.clone()),
                    span: expr.span,
                });
            }

            _ => {}
        }

        // Continue walking
        walk_expr(self, expr);
    }
}

/// Reset all mutable global state between test executions.
///
/// Called by the test executor before each test to prevent state leakage
/// between tests in batch runs. Clears VBC value side-tables, exhaustiveness
/// cache, and other per-compilation global state, while preserving expensive
/// stdlib caches.
pub fn reset_test_isolation() {
    verum_vbc::reset_global_value_tables();
    verum_types::exhaustiveness::clear_global_cache();
}

// ---------------------------------------------------------------------------
// Inline tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod resolve_super_path_tests {
    use super::CompilationPipeline;

    fn resolve(src: &str, mount: &str) -> String {
        // resolve_super_path is an associated fn — call it via the type.
        CompilationPipeline::resolve_super_path(src, mount)
    }

    #[test]
    fn no_super_returns_path_unchanged() {
        assert_eq!(resolve("core.sys.time_ops", "core.foo.bar"), "core.foo.bar");
    }

    #[test]
    fn no_super_short_path_unchanged() {
        assert_eq!(resolve("core.sys.time_ops", "base.memory"), "base.memory");
    }

    #[test]
    fn single_super_resolves_to_sibling() {
        // The motivating case for #163: core.sys.time_ops mounts
        // super.raw → core.sys.raw.
        assert_eq!(resolve("core.sys.time_ops", "super.raw"), "core.sys.raw");
    }

    #[test]
    fn single_super_with_subpath_resolves_through_sibling() {
        assert_eq!(
            resolve("core.sys.time_ops", "super.raw.foo"),
            "core.sys.raw.foo",
        );
    }

    #[test]
    fn double_super_drops_two_components() {
        assert_eq!(
            resolve("core.sys.time_ops", "super.super.collections.List"),
            "core.collections.List",
        );
    }

    #[test]
    fn lone_super_yields_parent_path() {
        // `mount super` (no subpath) refers to the parent module
        // itself — uncommon but legal grammar.
        assert_eq!(resolve("core.sys.time_ops", "super"), "core.sys");
    }

    #[test]
    fn super_at_root_returns_input_unchanged() {
        // `super` from a top-level `core` module would walk past the
        // root.  We don't try to invent a sentinel — return the path
        // as-is so the progressive-prefix walk fails to match (correct
        // behaviour for malformed input).
        assert_eq!(resolve("core", "super.foo"), "super.foo");
    }

    #[test]
    fn excessive_super_returns_input_unchanged() {
        // 4-super-deep from `core.sys.time_ops` (3 components) walks
        // past the root.
        assert_eq!(
            resolve("core.sys.time_ops", "super.super.super.super.x"),
            "super.super.super.super.x",
        );
    }

    #[test]
    fn exactly_root_super_returns_input_unchanged() {
        // 3-super-deep from `core.sys.time_ops` (3 components) walks
        // exactly to the root and yields an empty parent — also
        // malformed; treat as out-of-range.  Returning "x" verbatim
        // would let it match unrelated top-level modules in the
        // progressive-prefix walk, which is wrong.
        assert_eq!(
            resolve("core.sys.time_ops", "super.super.super.x"),
            "super.super.super.x",
        );
    }
}
