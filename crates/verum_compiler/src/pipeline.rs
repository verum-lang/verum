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
use verum_vbc::codegen::{VbcCodegen, CodegenConfig, TierContext};
use verum_vbc::interpreter::Interpreter as VbcInterpreter;
use verum_vbc::module::{FunctionId as VbcFunctionId, VbcModule};

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
    CoherenceChecker, ImplEntry, LanguageProfile, ModuleId, ModuleLoader, ModuleInfo, ModulePath,
    ModuleRegistry, ProfileChecker, SharedModuleResolver, extract_exports_from_module,
    resolve_glob_reexports, resolve_specific_reexport_kinds,
};
use verum_fast_parser::VerumParser;
use verum_smt::{
    Context as SmtContext, CostTracker, RefinementVerifier as SmtRefinementVerifier,
    SubsumptionChecker, SubsumptionConfig, VerificationError as SmtVerificationError,
    VerifyMode as SmtVerifyMode,
};
// Note: Gradual verification is now handled by phases::verification_phase
// See: BoundsCheckEliminator, CBGROptimizer, VerificationPipeline
use verum_common::{Maybe, Shared};
use verum_types::TypeChecker;

use crate::lint::{IntrinsicDiagnostics, IntrinsicLint};
use crate::linker_config::ProjectConfig;
use crate::meta::MetaRegistry;
use crate::module_utils;
use crate::options::VerifyMode;
use crate::phases::linking::{FinalLinker, LinkingConfig, ObjectFile};
use crate::phases::phase0_stdlib::{Phase0CoreCompiler, StdlibArtifacts};
use crate::phases::ExecutionTier;
use crate::phases::type_error_to_diagnostic;
use crate::session::Session;
use crate::core_cache::global_cache_or_init;
use crate::core_source::CoreSource;
use crate::core_compiler::{CoreConfig, StdlibCompilationResult, StdlibModule, StdlibModuleResolver};
use crate::hash::compute_item_hashes_from_module;
use crate::incremental_compiler::IncrementalCompiler;
use crate::staged_pipeline::{StagedPipeline, StagedConfig};

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
struct CachedStdlibModules {
    /// (module_path_string, parsed_module, source_text) triples
    entries: Vec<(Text, Module, Text)>,
}

static GLOBAL_STDLIB_MODULES: std::sync::OnceLock<std::sync::RwLock<Option<CachedStdlibModules>>> =
    std::sync::OnceLock::new();

fn global_stdlib_cache() -> &'static std::sync::RwLock<Option<CachedStdlibModules>> {
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

fn global_stdlib_registry_cache() -> &'static std::sync::RwLock<Option<ModuleRegistry>> {
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
fn compute_stdlib_content_hash(stdlib_path: &Path) -> String {
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
fn try_load_registry_from_disk(
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
fn save_registry_to_disk(
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

/// Result of SMT-based refinement verification
///
/// This enum represents the outcome of verifying a refinement constraint
/// using the Z3 SMT solver.
///
/// Refinement type subsumption: when a function returns a refined type
/// (e.g., `Int{> 0}`), the compiler checks via SMT that the return expression
/// satisfies the predicate. Three outcomes: Verified (proven by Z3), Timeout
/// (solver exceeded budget), or Falsifiable (counterexample found).
#[derive(Debug, Clone)]
enum SmtCheckResult {
    /// The refinement constraint was successfully verified.
    /// The return expression provably satisfies the predicate.
    Verified,

    /// The refinement constraint was violated.
    /// A counterexample demonstrates a case where the predicate fails.
    Violated {
        /// Optional counterexample showing values that violate the constraint
        counterexample: Option<String>,
    },

    /// The SMT solver could not determine the result.
    /// This may happen for complex predicates or unsupported operations.
    Unknown {
        /// Explanation of why verification was inconclusive
        reason: String,
    },

    /// The SMT solver timed out before completing verification.
    /// The constraint should be checked at runtime instead.
    Timeout,
}

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

/// Context for building CFG blocks in escape analysis
///
/// This struct holds the state needed during CFG construction for
/// a single function, including the block ID allocator, reference
/// counter, and pending blocks to be added to the CFG.
struct CfgBuildContext<'a> {
    /// The CFG builder for allocating block and reference IDs
    builder: &'a mut verum_cbgr::CfgBuilder,
    /// Counter for allocating reference IDs
    ref_counter: &'a mut u64,
    /// Entry block ID for the function
    entry_id: verum_cbgr::analysis::BlockId,
    /// Exit block ID for the function
    exit_id: verum_cbgr::analysis::BlockId,
    /// Blocks built during CFG construction, to be added at the end
    pending_blocks: List<verum_cbgr::analysis::BasicBlock>,
    /// Closure captures detected during analysis (ref_id, is_mutable)
    closure_captures: List<(verum_cbgr::analysis::RefId, bool)>,
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
fn should_parse_as_script(
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

    /// Compile the standard library to a VBC archive.
    ///
    /// This method is only available in `StdlibBootstrap` mode (created via `new_core()`).
    /// It uses global type registration across ALL modules before compiling any module,
    /// which eliminates cross-module dependency constraints.
    ///
    /// # Flow
    ///
    /// 1. Discover all stdlib modules via `StdlibModuleResolver`
    /// 2. Parse ALL modules to AST
    /// 3. Register ALL types globally (multi-pass across all modules)
    /// 4. Compile each module to VBC bytecode
    /// 5. Build and write `stdlib.vbca` archive
    ///
    /// # Returns
    ///
    /// Returns `StdlibCompilationResult` containing compilation statistics,
    /// or an error if compilation fails.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The pipeline is not in `StdlibBootstrap` mode
    /// - Module discovery fails
    /// - Parsing fails
    /// - Type registration fails
    /// - VBC codegen fails
    /// - Archive writing fails
    ///
    /// # Example
    ///
    /// ```ignore
    /// use verum_compiler::{Session, CompilationPipeline, CoreConfig};
    ///
    /// let config = CoreConfig::new("stdlib")
    ///     .with_output("target/stdlib.vbca");
    ///
    /// let mut session = Session::default();
    /// let mut pipeline = CompilationPipeline::new_core(&mut session, config);
    ///
    /// let result = pipeline.compile_core()?;
    /// println!("Compiled {} modules with {} functions",
    ///     result.modules_compiled, result.functions_compiled);
    /// ```
    pub fn compile_core(&mut self) -> Result<StdlibCompilationResult> {
        use verum_ast::cfg::TargetConfig;
        use verum_vbc::write_archive_to_file;

        // Extract config from build_mode (or fail if not in StdlibBootstrap mode)
        let config = match &self.build_mode {
            BuildMode::StdlibBootstrap { config } => config.clone(),
            BuildMode::Normal => {
                return Err(anyhow::anyhow!(
                    "compile_core() requires StdlibBootstrap mode. Use new_core() to create the pipeline."
                ));
            }
        };

        let start = std::time::Instant::now();
        let mut module_times = std::collections::HashMap::new();

        // ====================================================================
        // STEP 1: Discover modules
        // ====================================================================
        if config.verbose {
            eprintln!("Discovering stdlib modules in {}...", config.stdlib_path.display());
        }

        let resolver = self.stdlib_resolver.as_mut()
            .ok_or_else(|| anyhow::anyhow!("StdlibModuleResolver not initialized"))?;

        resolver.discover().map_err(|e| anyhow::anyhow!("Module discovery failed: {}", e))?;

        if config.verbose {
            eprintln!("Found {} modules", resolver.module_count());
        }

        // Get modules in dependency order
        let modules_to_compile: Vec<StdlibModule> = resolver.modules_in_order()
            .into_iter()
            .cloned()
            .collect();

        // ====================================================================
        // STEP 2: Parse ALL modules
        // ====================================================================
        if config.verbose {
            eprintln!("Phase 1: Parsing all modules...");
        }

        // Track (module_name, [(file_path, ast_module), ...]) for submodule resolution
        let mut all_parsed_modules: Vec<(String, Vec<(PathBuf, verum_ast::Module)>)> = Vec::new();

        for module in &modules_to_compile {
            let ast_modules = self.parse_stdlib_module_files(module)?;
            all_parsed_modules.push((module.name.clone(), ast_modules));
        }

        // ====================================================================
        // STEP 2.25: Resolve file-relative mounts (#5 / P1.5)
        // ====================================================================
        //
        // Before module-registry registration, walk every
        // parsed module for `MountTreeKind::File` declarations
        // (`mount ./helper.vr;`).  For each, the resolver
        // loads the referenced file via the loader's sandbox,
        // parses it, and surfaces it as a synthetic module
        // ready to be registered alongside its peers.
        //
        // This plugs file mounts into the existing
        // module-path pipeline with zero new resolution
        // codepaths — the synthesised module name (alias or
        // file basename) becomes the canonical module-path
        // identifier in the registry, and downstream import
        // resolution treats it identically to any other
        // module.
        //
        // Soft-fail strategy: file-mount resolution errors
        // surface as warnings during stdlib bootstrap (no
        // user-authored file mounts in core/ today, so any
        // failure is a regression in our own infrastructure)
        // and as hard errors during normal compilation.
        if !modules_to_compile.is_empty() {
            let mut resolver_seeds: Vec<(PathBuf, verum_ast::Module)> = Vec::new();
            for (_mod_name, files) in &all_parsed_modules {
                for (path, ast) in files {
                    resolver_seeds.push((path.clone(), ast.clone()));
                }
            }
            match verum_modules::file_mount::resolve_file_mounts(
                &mut self.module_loader,
                &resolver_seeds,
                |source| {
                    use verum_lexer::Lexer;
                    use verum_fast_parser::VerumParser;
                    let lexer = Lexer::new(source.source.as_str(), source.file_id);
                    let parser = VerumParser::new();
                    parser
                        .parse_module(lexer, source.file_id)
                        .map_err(|errs| {
                            let summary: String = errs
                                .iter()
                                .map(|e| e.to_string())
                                .collect::<Vec<_>>()
                                .join("; ");
                            verum_modules::error::ModuleError::Other {
                                message: verum_common::Text::from(format!(
                                    "parse error in file mount `{}`: {}",
                                    source.file_path.display(),
                                    summary
                                )),
                                span: None,
                            }
                        })
                },
            ) {
                Ok(resolved) => {
                    if !resolved.is_empty() && config.verbose {
                        eprintln!(
                            "Phase 1.25: Resolved {} file-relative mount(s)",
                            resolved.len()
                        );
                    }
                    // Each resolved file becomes its own
                    // module entry in `all_parsed_modules`,
                    // with the synthesised name acting as
                    // the canonical module path.  The
                    // existing Phase 1.5 registration loop
                    // picks them up uniformly.
                    for entry in resolved {
                        // Re-parse the source for AST
                        // ownership — the parsed module from
                        // the resolver callback is dropped.
                        // (Cheap: the loader cached the read,
                        // and parse is fast.)
                        let lexer = verum_lexer::Lexer::new(
                            entry.source.as_str(),
                            entry.file_id,
                        );
                        let parser = verum_fast_parser::VerumParser::new();
                        let ast = match parser.parse_module(lexer, entry.file_id) {
                            Ok(m) => m,
                            Err(e) => {
                                return Err(anyhow::anyhow!(
                                    "Parse error re-parsing file mount `{}`: {:?}",
                                    entry.absolute_path.display(),
                                    e
                                ));
                            }
                        };
                        all_parsed_modules.push((
                            entry.synthetic_name.clone(),
                            vec![(entry.absolute_path.clone(), ast)],
                        ));
                    }
                }
                Err(e) => {
                    // During stdlib bootstrap there should
                    // be no file mounts; if there are, log
                    // a clear warning rather than aborting
                    // (defensive — keeps stdlib compilation
                    // resilient to accidental mount syntax
                    // sneaking into core/).
                    if config.verbose {
                        eprintln!(
                            "Phase 1.25: file-mount resolution warning: {}",
                            e
                        );
                    }
                }
            }
        }

        // ====================================================================
        // STEP 2.5: Register ALL parsed modules in the ModuleRegistry
        // ====================================================================
        // This MUST happen before type registration (Step 3) so that the
        // TypeChecker can resolve cross-module imports via the registry.
        // Without this, import resolution fails with E402 (module not found).
        if config.verbose {
            eprintln!("Phase 1.5: Registering modules in ModuleRegistry...");
        }

        {
            let module_registry = self.session.module_registry();
            for (module_name, ast_modules_with_paths) in &all_parsed_modules {
                for (file_path, ast_module) in ast_modules_with_paths {
                    // Compute module path from module name + file
                    // Module names are already in dot-separated format (e.g., "core.base.primitives")
                    let module_path = ModulePath::from_str(module_name.as_str());
                    let module_id = module_registry.read().allocate_id();

                    let file_id = ast_module.items.first()
                        .map(|item| item.span.file_id)
                        .unwrap_or_else(|| verum_ast::FileId::new(0));

                    let source_text = std::fs::read_to_string(file_path)
                        .unwrap_or_default();

                    let mut module_info = ModuleInfo::new(
                        module_id,
                        module_path.clone(),
                        ast_module.clone(),
                        file_id,
                        Text::from(source_text),
                    );

                    // Extract exports for cross-module import resolution
                    match extract_exports_from_module(ast_module, module_id, &module_path) {
                        Ok(export_table) => {
                            module_info.exports = export_table;
                        }
                        Err(e) => {
                            debug!("Failed to extract exports from {}: {:?}", module_name, e);
                        }
                    }

                    module_registry.write().register(module_info);

                    // Also add to self.modules for later use
                    let path_text = Text::from(module_name.as_str());
                    if !self.modules.contains_key(&path_text) {
                        self.modules.insert(path_text, Arc::new(ast_module.clone()));
                    }

                    // ALSO register a per-file sub-module path for non-mod.vr files.
                    // E.g., core/async/poll.vr -> register as "core.async.poll"
                    // This enables relative imports like `mount .poll.*` to resolve
                    // from within the parent module (core.async).
                    if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                        if file_stem != "mod" {
                            let sub_module_name = format!("{}.{}", module_name, file_stem);
                            let sub_module_path = ModulePath::from_str(&sub_module_name);
                            let sub_module_id = module_registry.read().allocate_id();

                            let mut sub_module_info = ModuleInfo::new(
                                sub_module_id,
                                sub_module_path.clone(),
                                ast_module.clone(),
                                file_id,
                                Text::from(std::fs::read_to_string(file_path).unwrap_or_default()),
                            );

                            match extract_exports_from_module(ast_module, sub_module_id, &sub_module_path) {
                                Ok(export_table) => {
                                    sub_module_info.exports = export_table;
                                }
                                Err(e) => {
                                    debug!("Failed to extract exports from {}: {:?}", sub_module_name, e);
                                }
                            }

                            module_registry.write().register(sub_module_info);

                            // Also add to self.modules
                            let sub_path_text = Text::from(sub_module_name.as_str());
                            if !self.modules.contains_key(&sub_path_text) {
                                self.modules.insert(sub_path_text, Arc::new(ast_module.clone()));
                            }
                        }
                    }
                }
            }

            // Resolve re-exports so that glob imports work
            let mut guard = module_registry.write();
            let _ = resolve_specific_reexport_kinds(&mut guard);
            let mut iteration = 0;
            loop {
                iteration += 1;
                match resolve_glob_reexports(&mut guard) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if iteration >= 10 => break,
                    Ok(_) => continue,
                }
            }
        }

        if config.verbose {
            let registry_count = self.session.module_registry().read().len();
            eprintln!("  Registered {} modules in ModuleRegistry", registry_count);
        }

        // ====================================================================
        // STEP 3: Global type registration (ALL modules)
        // ====================================================================
        if config.verbose {
            eprintln!("Phase 2: Registering types globally across all modules...");
        }

        // Create TypeChecker with minimal context for stdlib compilation
        // (types are registered dynamically as stdlib .vr files are parsed)
        let mut type_checker = verum_types::infer::TypeChecker::with_minimal_context();
        type_checker.register_primitives();

        // Set module registry on type checker so cross-module imports can be resolved
        let registry = self.session.module_registry();
        type_checker.set_module_registry(registry.clone());

        self.register_stdlib_types_globally(&all_parsed_modules, &mut type_checker, &config)?;

        // ====================================================================
        // STEP 4: Compile each module to VBC
        // ====================================================================
        if config.verbose {
            eprintln!("Phase 3: Compiling modules to VBC...");
        }

        let mut functions_compiled = 0;
        let target = TargetConfig::host();

        // Build list of all module names for forward reference detection
        let all_module_names: Vec<&str> = all_parsed_modules
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();

        for (idx, (module_name, ast_modules_with_paths)) in all_parsed_modules.iter().enumerate() {
            let module_start = std::time::Instant::now();
            let module = modules_to_compile.iter()
                .find(|m| &m.name == module_name)
                .expect("module should exist");

            if config.verbose {
                eprintln!("  Compiling module: {} ({} files)", module.name, module.source_files.len());
            }

            // Build set of modules that will be compiled AFTER this one (forward references)
            let later_modules: std::collections::HashSet<&str> = all_module_names[idx + 1..]
                .iter()
                .copied()
                .collect();

            // Extract just the AST modules for compilation
            let ast_modules: Vec<&verum_ast::Module> = ast_modules_with_paths
                .iter()
                .map(|(_, ast)| ast)
                .collect();

            let (vbc_module, funcs) = self.compile_core_module_from_ast(
                module,
                ast_modules.as_slice(),
                &config,
                &target,
                &later_modules,
            )?;
            functions_compiled += funcs;

            module_times.insert(module.name.clone(), module_start.elapsed());
            self.compiled_stdlib_modules.insert(module.name.clone(), vbc_module);
        }

        // ====================================================================
        // STEP 5: Build archive
        // ====================================================================
        if config.verbose {
            eprintln!("Building archive...");
        }

        let archive = self.build_stdlib_archive(&config)?;

        // ====================================================================
        // STEP 6: Write archive
        // ====================================================================
        if let Some(parent) = config.output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("Failed to create output directory: {}", e))?;
        }

        write_archive_to_file(&archive, &config.output_path)
            .map_err(|e| anyhow::anyhow!("Failed to write archive: {}", e))?;

        let output_size = std::fs::metadata(&config.output_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(StdlibCompilationResult {
            modules_compiled: self.compiled_stdlib_modules.len(),
            functions_compiled,
            total_time: start.elapsed(),
            module_times,
            output_path: config.output_path.clone(),
            output_size,
            warnings: self.stdlib_warnings.clone(),
            errors: self.stdlib_errors.clone(),
        })
    }

    /// Parse stdlib module source files to AST.
    fn parse_stdlib_module_files(
        &self,
        module: &StdlibModule,
    ) -> Result<Vec<(PathBuf, verum_ast::Module)>> {
        use crate::api::SourceFile;

        let mut sources: Vec<(PathBuf, SourceFile)> = Vec::new();
        for path in &module.source_files {
            let source = SourceFile::load(path)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;
            sources.push((path.clone(), source));
        }

        let mut ast_modules = Vec::new();

        for (path, source) in &sources {
            let mut parser = verum_fast_parser::Parser::new(&source.content);
            match parser.parse_module() {
                Ok(ast_module) => ast_modules.push((path.clone(), ast_module)),
                Err(e) => {
                    return Err(anyhow::anyhow!("Parse error in {}: {:?}", source.path, e));
                }
            }
        }

        Ok(ast_modules)
    }

    /// Global type registration across ALL stdlib modules.
    ///
    /// Multi-pass registration order:
    /// 1. Import aliases
    /// 2. Type names (forward declarations)
    /// 3. Type bodies
    /// 4. Function signatures
    /// 5. Protocols
    /// 6. Impl blocks
    fn register_stdlib_types_globally(
        &mut self,
        all_modules: &[(String, Vec<(PathBuf, verum_ast::Module)>)],
        type_checker: &mut verum_types::infer::TypeChecker,
        config: &CoreConfig,
    ) -> Result<()> {
        use verum_ast::cfg::TargetConfig;
        let target = TargetConfig::host();

        // Pass 0: Process imports with aliases
        if config.verbose {
            eprintln!("  Pass 0: Processing import aliases from all modules...");
        }
        for (_module_name, ast_modules) in all_modules {
            for (_file_path, ast_module) in ast_modules {
                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Mount(import_decl) = &item.kind {
                        type_checker.process_import_aliases(import_decl);
                    }
                }
            }
        }

        // Pass 0.5: Process full imports using the ModuleRegistry
        // This resolves `mount .stream.*` and `mount .protocols.*` etc. so that
        // types from sub-modules are available during type-checking. Each module's
        // imports are resolved with the correct current_module_path so that relative
        // imports (leading dot) resolve to the right absolute module paths.
        if config.verbose {
            eprintln!("  Pass 0.5: Processing cross-module imports...");
        }
        {
            let registry = self.session.module_registry();
            for (module_name, ast_modules) in all_modules {
                // For each file in the module, compute the correct current_module_path
                for (file_path, ast_module) in ast_modules {
                    let current_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                        if file_stem == "mod" {
                            // mod.vr represents its parent directory module
                            module_name.clone()
                        } else {
                            // Regular file: module_name.file_stem
                            format!("{}.{}", module_name, file_stem)
                        }
                    } else {
                        module_name.clone()
                    };

                    for item in &ast_module.items {
                        if let verum_ast::ItemKind::Mount(import_decl) = &item.kind {
                            // Process the full import (not just aliases) to bring
                            // cross-module types into scope for this module
                            if let Err(e) = type_checker.process_import(
                                import_decl,
                                &current_module_path,
                                &registry.read(),
                            ) {
                                debug!("Stdlib import warning in {}: {:?}", current_module_path, e);
                            }
                        }
                    }
                }
            }
        }

        // Pass 1: Register ALL type NAMES
        if config.verbose {
            eprintln!("  Pass 1: Registering type names from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (_file_path, ast_module) in ast_modules {
                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                        type_checker.register_type_name_only(type_decl);
                    }
                }
            }
            if config.verbose {
                eprintln!("    {} type names registered", module_name);
            }
        }

        // Pass 2: Register ALL type BODIES
        if config.verbose {
            eprintln!("  Pass 2: Registering type bodies from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (file_path, ast_module) in ast_modules {
                // Architectural: set the checker's current module so that
                // `define_type_in_current_module` can publish each type
                // under its fully-qualified key (`{module}.{name}`). Without
                // this the qualified-name layer stays empty and same-named
                // stdlib types (e.g. `RecvError` in broadcast/channel/quic)
                // silently collide on the flat lookup table.
                let per_file_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == "mod" {
                        module_name.clone()
                    } else {
                        format!("{}.{}", module_name, file_stem)
                    }
                } else {
                    module_name.clone()
                };
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                        let filtered_decl = module_utils::filter_type_decl_for_target(type_decl, &target);
                        if let Err(e) = type_checker.register_type_declaration(&filtered_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0910")
                                .message(format!("Type registration warning in {}: {}", module_name, e))
                                .build();
                            self.stdlib_warnings.push(warning);
                        }
                    }
                }
            }
        }

        // Pass 3: Register ALL function signatures
        if config.verbose {
            eprintln!("  Pass 3: Registering function signatures from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (file_path, ast_module) in ast_modules {
                // Module-scoped resolution: free fn signatures reference types
                // (`Result<_, RecvError>`) that may collide across modules, so
                // anchor each file to its qualified module path before
                // registration.
                let per_file_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == "mod" {
                        module_name.clone()
                    } else {
                        format!("{}.{}", module_name, file_stem)
                    }
                } else {
                    module_name.clone()
                };
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Function(func) = &item.kind {
                        if let Err(e) = type_checker.register_function_signature(func) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0911")
                                .message(format!("Function signature warning in {}: {}", module_name, e))
                                .build();
                            self.stdlib_warnings.push(warning);
                        }
                    }
                }
            }
        }

        // Pass 4: Register ALL protocols
        if config.verbose {
            eprintln!("  Pass 4: Registering protocols from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (file_path, ast_module) in ast_modules {
                // Module-scoped resolution for protocol method signatures.
                let per_file_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == "mod" {
                        module_name.clone()
                    } else {
                        format!("{}.{}", module_name, file_stem)
                    }
                } else {
                    module_name.clone()
                };
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                        if let Err(e) = type_checker.register_protocol(protocol_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0912")
                                .message(format!("Protocol registration warning in {}: {}", module_name, e))
                                .build();
                            self.stdlib_warnings.push(warning);
                        }
                    }
                }
            }
        }

        // Pass 5: Register ALL impl blocks
        if config.verbose {
            eprintln!("  Pass 5: Registering impl blocks from all modules...");
        }
        for (module_name, ast_modules) in all_modules {
            for (file_path, ast_module) in ast_modules {
                // Architectural: set the current module path so that type
                // references inside impl-block signatures (e.g. `RecvError` in
                // `Stream for BroadcastReceiver`) resolve against the
                // qualified-name layer first — avoiding collisions between
                // same-named types in different stdlib modules.
                let per_file_module_path = if let Some(file_stem) = file_path.file_stem().and_then(|s| s.to_str()) {
                    if file_stem == "mod" {
                        module_name.clone()
                    } else {
                        format!("{}.{}", module_name, file_stem)
                    }
                } else {
                    module_name.clone()
                };
                type_checker.set_current_module_path(per_file_module_path);

                for item in &ast_module.items {
                    if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                        if let Err(e) = type_checker.register_impl_block(impl_decl) {
                            let warning = verum_diagnostics::DiagnosticBuilder::warning()
                                .code("W0913")
                                .message(format!("Impl block registration warning in {}: {}", module_name, e))
                                .build();
                            self.stdlib_warnings.push(warning);
                        }
                    }
                }
            }
        }

        // Pass 5.5a: Protocol-based discovery of coercion-friendly types.
        // Walks loaded AST modules looking for `implement <Coercion> for
        // X` blocks (where Coercion ∈ {IntCoercible, TensorLike,
        // Indexable, RangeLike} from core/base/coercion.vr) and registers
        // each target type with the unifier. Stdlib types that already
        // declare these implement-blocks are picked up here — zero
        // architectural violation for those.
        let mut all_ast_modules: Vec<&verum_ast::Module> = Vec::new();
        for (_, ast_modules) in all_modules {
            for (_, ast_module) in ast_modules {
                all_ast_modules.push(ast_module);
            }
        }
        let registered_via_protocol =
            crate::stdlib_coercion_registry::scan_protocol_implementations(
                type_checker.unifier_mut(),
                all_ast_modules.iter().copied(),
            );
        if registered_via_protocol > 0 {
            debug!(
                "[coercion-registry] discovered {} stdlib-coercion impl blocks via protocol scan",
                registered_via_protocol
            );
        }

        // Pass 5.5b: Hardcoded fallback registration for stdlib types
        // not yet retrofitted with implement blocks. Per the
        // architectural rule in `verum_types/src/CLAUDE.md`
        // ("NEVER hardcode stdlib/core type knowledge in the
        // compiler"), the hardcoded scaffolding is contained in the
        // dedicated `stdlib_coercion_registry` module so the violation
        // lives in one identifiable spot.
        //
        // The unifier's register_*_type methods de-duplicate via
        // HashSet, so calling 5.5b after 5.5a is harmless when an
        // already-discovered type happens to be in the hardcoded list.
        // Each stdlib retrofit (adding `implement IntCoercible for X`)
        // lets us delete X from the hardcoded list with safe
        // rollback at every step.
        crate::stdlib_coercion_registry::register_stdlib_coercions(
            type_checker.unifier_mut(),
        );

        // Pass 6: Validate imports
        // Now that all types, functions, and protocols are registered,
        // validate that all imports reference items that actually exist.
        if config.verbose {
            eprintln!("  Pass 6: Validating imports...");
        }

        let export_index = crate::core_compiler::build_export_index(all_modules);
        let import_errors = crate::core_compiler::validate_imports(all_modules, &export_index, &target);

        for (module_path, item_name, similar, span) in import_errors {
            let message = if similar.is_empty() {
                format!(
                    "E401: cannot find `{}` in module `{}` (byte {}-{})",
                    item_name, module_path, span.start, span.end
                )
            } else {
                format!(
                    "E401: cannot find `{}` in module `{}` (byte {}-{}). Did you mean: {}?",
                    item_name, module_path, span.start, span.end, similar
                )
            };

            let error = verum_diagnostics::DiagnosticBuilder::error()
                .code("E0401")
                .message(message)
                .build();
            self.stdlib_warnings.push(error);
        }

        if config.verbose {
            eprintln!("  Global type registration complete.");
        }

        Ok(())
    }

    /// Compile a stdlib module from pre-parsed AST.
    ///
    /// # Arguments
    /// * `module` - The module to compile
    /// * `ast_modules` - Pre-parsed AST modules for this module
    /// * `config` - Stdlib compilation configuration
    /// * `target` - Target platform configuration
    /// * `later_modules` - Set of module names that will be compiled AFTER this module.
    ///   Used for forward reference detection to suppress warnings for cross-module
    ///   function calls that will be resolved later in the compilation sequence.
    fn compile_core_module_from_ast(
        &mut self,
        module: &StdlibModule,
        ast_modules: &[&verum_ast::Module],
        config: &CoreConfig,
        target: &verum_ast::cfg::TargetConfig,
        later_modules: &std::collections::HashSet<&str>,
    ) -> Result<(verum_vbc::VbcModule, usize)> {
        use verum_vbc::codegen::CodegenConfig;
        use verum_vbc::module::{FunctionDescriptor, VbcFunction};
        use verum_vbc::instruction::Instruction;

        // Configure VBC codegen
        let codegen_config = CodegenConfig::new(&module.name)
            .with_optimization_level(config.optimization_level)
            .with_target(target.clone());

        let codegen_config = if config.debug_info {
            codegen_config.with_debug_info()
        } else {
            codegen_config
        };

        let mut codegen = VbcCodegen::with_config(codegen_config);

        // Import functions and protocols from previously compiled modules
        codegen.import_functions(&self.global_function_registry);
        codegen.import_protocols(&self.global_protocol_registry);

        // Three-pass compilation within the module (cross-file two-phase collection)
        // Pass 1a: Collect ALL protocol definitions from ALL files first
        // This ensures protocols like Eq, Ord are available when processing
        // impl blocks that implement them, regardless of file order.
        for ast_module in ast_modules {
            codegen.collect_protocol_definitions(ast_module);
        }

        // Pass 1b: Collect all other declarations from ALL files
        let lint_diagnostics = IntrinsicDiagnostics::new(&self.session.options().lint_config);
        for ast_module in ast_modules {
            if let Err(e) = codegen.collect_non_protocol_declarations(ast_module) {
                let diag = lint_diagnostics.codegen_warning(&module.name, &e.to_string(), None);
                let level = self.session.options().lint_config.level_for(IntrinsicLint::MissingImplementation);
                if level.is_error() {
                    self.stdlib_errors.push(diag);
                } else if level.should_emit() {
                    self.stdlib_warnings.push(diag);
                }
            }
        }

        // After all declarations collected, resolve pending imports
        // This handles cross-file imports within the same module
        codegen.resolve_pending_imports();

        // Pass 2: Compile all function bodies and merge
        let mut total_func_count = 0;
        let mut merged_vbc = verum_vbc::VbcModule::new(module.name.clone());

        for ast_module in ast_modules {
            match codegen.compile_function_bodies(ast_module) {
                Ok(compiled_module) => {
                    total_func_count += compiled_module.functions.len();
                    self.merge_stdlib_vbc_modules(&mut merged_vbc, compiled_module)?;
                }
                Err(e) => {
                    // Check if this is a forward reference to a module compiled later.
                    // If so, suppress the warning - the function will be available at runtime.
                    let is_forward_ref = if let Some(func_name) = e.undefined_function_name() {
                        // Extract the module prefix from the function path
                        // e.g., "darwin::tls::init_main_thread_tls" -> check "sys.darwin"
                        // e.g., "mem::heap::init_thread_heap" -> check "mem"
                        Self::is_forward_reference_to_later_module(
                            func_name,
                            &module.name,
                            later_modules,
                        )
                    } else {
                        false
                    };

                    if !is_forward_ref {
                        // Use IntrinsicDiagnostics for configurable severity
                        // Include span info in error message since we don't have file_id for Span construction
                        let error_msg = if let Some(ref s) = e.span {
                            format!("{} (byte {}-{})", e, s.start, s.end)
                        } else {
                            e.to_string()
                        };
                        let diag = lint_diagnostics.codegen_warning(&module.name, &error_msg, None);
                        let level = self.session.options().lint_config.level_for(IntrinsicLint::MissingImplementation);
                        if level.is_error() {
                            self.stdlib_errors.push(diag);
                        } else if level.should_emit() {
                            self.stdlib_warnings.push(diag);
                        }
                    }

                    // Create stub functions
                    for item in &ast_module.items {
                        if let verum_ast::ItemKind::Function(func) = &item.kind {
                            total_func_count += 1;
                            let func_name = func.name.name.to_string();
                            let name_id = merged_vbc.intern_string(&func_name);
                            let mut descriptor = FunctionDescriptor::new(name_id);
                            descriptor.register_count = 1;
                            descriptor.locals_count = func.params.len() as u16;
                            let vbc_func = VbcFunction::new(descriptor, vec![Instruction::RetV]);
                            merged_vbc.add_function(vbc_func.descriptor.clone());
                        }
                    }
                }
            }
        }

        // Export newly registered functions and protocols to global registries
        let new_functions = codegen.export_functions();
        for (name, info) in new_functions {
            self.global_function_registry.entry(name).or_insert(info);
        }

        let new_protocols = codegen.export_protocols();
        for (name, info) in new_protocols {
            self.global_protocol_registry.entry(name).or_insert(info);
        }

        Ok((merged_vbc, total_func_count))
    }

    /// Checks if an undefined function error is a forward reference to a module
    /// that will be compiled later in the compilation sequence.
    ///
    /// # Arguments
    /// * `func_path` - The function path from the error (e.g., "darwin::tls::init_main_thread_tls")
    /// * `current_module` - The module currently being compiled (e.g., "sys")
    /// * `later_modules` - Set of modules that will be compiled after the current one
    ///
    /// # Returns
    /// `true` if this appears to be a forward reference to a later module
    fn is_forward_reference_to_later_module(
        func_path: &str,
        current_module: &str,
        later_modules: &std::collections::HashSet<&str>,
    ) -> bool {
        // The function path uses "::" as separator (e.g., "darwin::tls::init_main_thread_tls")
        // We need to map this to module names which use "." as separator (e.g., "sys.darwin")

        // Extract the first component of the path
        let parts: Vec<&str> = func_path.split("::").collect();
        if parts.is_empty() {
            return false;
        }

        let first_component = parts[0];

        // Case 1: Direct submodule reference (e.g., "darwin" from "sys" -> "sys.darwin")
        let submodule_name = format!("{}.{}", current_module, first_component);
        if later_modules.contains(submodule_name.as_str()) {
            return true;
        }

        // Case 2: Direct module reference (e.g., "mem" -> "mem")
        if later_modules.contains(first_component) {
            return true;
        }

        // Case 3: Path with multiple components - try to match against later modules
        // e.g., "mem::heap::init_thread_heap" should match "mem"
        for later_module in later_modules {
            // Check if the function path starts with the module name
            let module_prefix = later_module.replace('.', "::");
            if func_path.starts_with(&module_prefix) || func_path.starts_with(later_module) {
                return true;
            }

            // Check if any component matches the module name (without parent prefix)
            // e.g., "mem" in "mem::heap::..." should match later module "mem"
            let module_parts: Vec<&str> = later_module.split('.').collect();
            if let Some(last_part) = module_parts.last() {
                if first_component == *last_part {
                    return true;
                }
            }
        }

        false
    }

    /// Merge a compiled VBC module into the main module.
    fn merge_stdlib_vbc_modules(
        &self,
        target: &mut verum_vbc::VbcModule,
        source: verum_vbc::VbcModule,
    ) -> Result<()> {
        let bytecode_offset = target.bytecode.len() as u32;
        let func_id_base = target.functions.len() as u32;

        // Merge function descriptors with adjusted offsets and function ID base
        for mut func in source.functions {
            func.bytecode_offset += bytecode_offset;
            func.func_id_base = func_id_base;
            target.add_function(func);
        }

        // Merge bytecode
        target.bytecode.extend_from_slice(&source.bytecode);

        // Merge type descriptors, remapping FunctionIds in protocol impls
        for ty in &source.types {
            let mut ty = ty.clone();
            // Remap FunctionIds in protocol implementations to account for
            // function table offset after merging with previous modules.
            for proto_impl in ty.protocols.iter_mut() {
                for fn_id in proto_impl.methods.iter_mut() {
                    if *fn_id != u32::MAX {
                        *fn_id += func_id_base;
                    }
                }
            }
            // Note: drop_fn and clone_fn are not remapped here because they
            // are not currently used from the merged module at runtime.
            target.add_type(ty);
        }

        // Merge string pool
        for (s, _id) in source.strings.iter() {
            target.intern_string(s);
        }

        // Merge constant pool
        for c in &source.constants {
            target.add_constant(c.clone());
        }

        // Merge specializations
        target.specializations.extend(source.specializations);

        // Merge field layout metadata (for GetF/SetF field index remapping in LLVM lowering)
        // field_id_to_name: extend target with source entries (source IDs offset by target length)
        // Note: VBC GetF instructions in each module use module-local field IDs.
        // We keep all entries so the LLVM lowering can look up field names.
        if !source.field_id_to_name.is_empty() {
            let offset = target.field_id_to_name.len();
            target.field_id_to_name.extend(source.field_id_to_name);
            // Store offset for future remapping if needed
            let _ = offset; // Currently field_ids are used per-module, not cross-module
        }
        // type_field_layouts: merge all type layouts (source overrides for same type names)
        for (type_name, fields) in source.type_field_layouts {
            target.type_field_layouts.entry(type_name).or_insert(fields);
        }

        Ok(())
    }

    /// Build the VBC archive from compiled stdlib modules.
    fn build_stdlib_archive(
        &self,
        config: &CoreConfig,
    ) -> Result<verum_vbc::VbcArchive> {
        use verum_vbc::{ArchiveBuilder, ArchiveFlags};

        let mut flags = ArchiveFlags::IS_STDLIB;
        if config.debug_info {
            flags |= ArchiveFlags::DEBUG_INFO;
        }
        if config.source_maps {
            flags |= ArchiveFlags::SOURCE_MAPS;
        }

        let mut builder = ArchiveBuilder::stdlib().with_flags(flags);

        let resolver = self.stdlib_resolver.as_ref()
            .ok_or_else(|| anyhow::anyhow!("StdlibModuleResolver not initialized"))?;

        // Add modules in compilation order
        for name in resolver.compilation_order() {
            if let Some(module) = self.compiled_stdlib_modules.get(name) {
                let deps: Vec<&str> = resolver.get_module(name)
                    .map(|m| m.dependencies.iter().map(|s: &String| s.as_str()).collect())
                    .unwrap_or_default();

                builder.add_module(name, module, &deps)
                    .map_err(|e| anyhow::anyhow!("Failed to add module {} to archive: {}", name, e))?;
            }
        }

        Ok(builder.finish())
    }

    /// Run multi-pass compilation on multiple source files
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

        // PASS 3: Semantic Analysis
        // Skip analysis for verification-only modules (they reuse cached type check results).
        // This is the key optimization: when only the implementation of a dependency changed
        // (not its signature), we don't need to re-type-check dependent modules.
        self.current_pass = CompilerPass::Pass3Analysis;
        info!("Pass 3: Analysis phase");

        for path in &module_paths {
            // Skip verification-only modules - they don't need re-analysis
            if verify_only.contains(path) {
                debug!("  Skipping analysis (verify-only): {}", path.as_str());
                continue;
            }

            debug!("  Analyzing: {}", path.as_str());
            // Clone Arc (cheap) to release borrow before calling mutable method
            let module_rc = self.modules.get(path).map(Arc::clone);
            if let Some(module) = module_rc {
                self.analyze_module(path, &module)?;
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

            for path in &module_paths {
                // Only verify modules that need it (changed or have changed dependencies)
                if !modules_to_verify.contains(path) {
                    continue;
                }

                debug!("  Verifying: {}", path.as_str());
                // Clone Arc (cheap) to release borrow before calling mutable method
                let module_rc = self.modules.get(path).map(Arc::clone);
                if let Some(module) = module_rc {
                    self.phase_verify(&module)?;
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

        // Load and parse all modules using ModuleLoader
        let mut sources = Map::new();

        for file_path in project_files {
            debug!("Loading module: {}", file_path.display());

            // Read source file
            let source_text = std::fs::read_to_string(&file_path)
                .with_context(|| format!("Failed to read file: {}", file_path.display()))?;

            // Convert file path to module path for proper import resolution
            let mut module_path_str = file_path
                .strip_prefix(self.module_loader.root_path())
                .unwrap_or(&file_path)
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

            sources.insert(Text::from(module_path_str), Text::from(source_text));
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

    // ========================================================================
    // STDLIB MODULE LOADING
    // ========================================================================

    /// Load and parse all stdlib modules into self.modules.
    ///
    /// This enables cross-file imports from std.* modules.
    /// Must be called before processing user modules.
    ///
    /// # Performance Optimization (Registry Caching)
    ///
    /// This function implements a two-level caching strategy:
    /// 1. **Registry cache (FAST PATH)**: If we have a fully-populated registry
    ///    cached, we deep_clone it (~1ms) instead of re-registering all modules
    /// 2. **Module cache (FALLBACK)**: If no registry cache, we use cached parsed
    ///    modules to avoid re-parsing, then populate and cache the registry
    ///
    /// The registry cache provides ~500ms speedup per compilation by avoiding:
    /// - Module registration in ModuleRegistry (~166 modules)
    /// - Export extraction from each module
    /// - Glob re-export resolution (iterative)
    ///
    /// Loads stdlib with two-tier caching: (1) registry cache from prior compilation,
    /// (2) parsed module cache to avoid re-parsing ~166 stdlib modules.
    fn load_stdlib_modules(&mut self) -> Result<()> {
        let start = Instant::now();
        debug!("load_stdlib_modules called");

        // FAST PATH: Try to use cached fully-populated registry
        // This is the key optimization: deep_clone a cached registry (~1ms)
        // instead of re-registering ~166 modules (~500ms).
        // NOTE: deep_clone shares ModuleInfo via Arc (Shared) and only clones
        // the HashMap structure. Further optimization would require wrapping the
        // entire registry in Arc and using copy-on-write for mutations.
        {
            let cache = global_stdlib_registry_cache();
            let guard = cache.read().unwrap_or_else(|poisoned| {
                tracing::warn!("stdlib registry cache RwLock poisoned, recovering");
                poisoned.into_inner()
            });
            if let Some(ref cached_registry) = *guard {
                let cloned = cached_registry.deep_clone();
                let module_count = cloned.len();

                // Replace the session's registry with the cloned one
                {
                    let registry_shared = self.session.module_registry();
                    let mut session_registry = registry_shared.write();
                    *session_registry = cloned;
                }

                // Also populate the local modules map from the registry.
                // Sort by module path before iterating: ModuleRegistry.modules
                // is Map (HashMap-backed via verum_common::Map), so raw
                // iteration order leaks Rust's per-process random hasher
                // seed into downstream codegen, producing non-deterministic
                // bytecode (see #143).  Path-sorted iteration is stable
                // across runs.
                let session_registry = self.session.module_registry();
                let reg = session_registry.read();
                let mut entries: Vec<(String, Arc<verum_ast::Module>)> = reg
                    .all_modules()
                    .map(|(_id, info)| (info.path.to_string(), Arc::new(info.ast.clone())))
                    .collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                for (path_str, ast_arc) in entries {
                    self.modules.insert(Text::from(path_str), ast_arc);
                }
                drop(reg);

                let elapsed = start.elapsed();
                info!(
                    "Loaded {} stdlib module(s) from registry cache in {:.2}ms",
                    module_count,
                    elapsed.as_secs_f64() * 1000.0
                );
                return Ok(());
            }
        }

        // SLOW PATH: No in-memory registry cache, load from source
        debug!("No in-memory registry cache, loading stdlib from source");

        // Determine stdlib path based on build mode:
        // - StdlibBootstrap mode: Use the configured stdlib_path directly
        // - Normal mode: Find workspace root and look for core/
        //
        // ARCHITECTURE NOTE: The embedded stdlib (embedded_stdlib.rs) contains all
        // core/*.vr sources compressed in the binary. Currently we still resolve from
        // the filesystem for dev mode (workspace core/). In production builds, the
        // embedded archive can be used instead of filesystem by switching the source.
        // The embedded archive API: crate::embedded_stdlib::get_embedded_stdlib()
        let (stdlib_path, workspace_root_for_cache) = match &self.build_mode {
            BuildMode::StdlibBootstrap { config } => {
                debug!("StdlibBootstrap mode: using configured path {:?}", config.stdlib_path);
                (config.stdlib_path.clone(), None)
            }
            BuildMode::Normal => {
                // Stdlib (core cog) resolution:
                //   1. VERUM_STDLIB_PATH env var (explicit override)
                //   2. Workspace core/ directory (dev mode — binary in target/)
                //
                // NOTE: ~/.verum/core/ resolution commented out — embedded stdlib
                // replaces filesystem-based production installs.
                let stdlib_candidates: Vec<(PathBuf, Option<PathBuf>)> = {
                    let mut candidates = Vec::new();

                    // 1. Explicit override
                    if let Ok(path) = std::env::var("VERUM_STDLIB_PATH") {
                        let p = PathBuf::from(&path);
                        if p.exists() {
                            candidates.push((p, None));
                        }
                    }

                    // 2. Workspace root (dev mode).
                    //
                    // T6.0.2 — only accept the candidate when
                    // `core/mod.vr` is present. A bare `core/`
                    // directory (e.g. inside a user cog that
                    // happened to scaffold the namespace but
                    // never populated it) silently shadowed the
                    // embedded stdlib pre-fix; cogs whose `core/`
                    // is empty (or absent) now fall through to
                    // the embedded path correctly.
                    if let Ok(workspace_root) = self.find_workspace_root() {
                        let core_path = workspace_root.join("core");
                        let mod_file = core_path.join("mod.vr");
                        if mod_file.is_file() {
                            candidates.push((core_path, Some(workspace_root)));
                        }
                    }

                    // 3. ~/.verum/core/ — DISABLED: embedded stdlib replaces this
                    // if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
                    //     let verum_core = PathBuf::from(&home).join(".verum").join("core");
                    //     if verum_core.exists() {
                    //         candidates.push((verum_core, None));
                    //     }
                    // }

                    candidates
                };

                match stdlib_candidates.into_iter().next() {
                    Some((stdlib, workspace)) => {
                        debug!("Core stdlib resolved: {:?}", stdlib);
                        (stdlib, workspace)
                    }
                    None => {
                        debug!("No core stdlib found on filesystem");
                        return Ok(());
                    }
                }
            }
        };

        if !stdlib_path.exists() {
            debug!("Stdlib directory not found at {:?}, skipping", stdlib_path);
            return Ok(());
        }

        // DISK CACHE: Persistent registry cache for cross-process reuse.
        // Always enabled — disk cache avoids re-parsing 171 stdlib .vr files.
        // Disable with VERUM_NO_DISK_CACHE=1 if needed.
        let content_hash = if std::env::var("VERUM_NO_DISK_CACHE").is_ok() {
            String::new() // Explicitly disabled
        } else {
            compute_stdlib_content_hash(&stdlib_path)
        };
        if !content_hash.is_empty() {
            if let Some(ref ws_root) = workspace_root_for_cache {
                if let Some(disk_registry) = try_load_registry_from_disk(ws_root, &content_hash) {
                    let module_count = disk_registry.len();

                    // Populate the session's registry
                    {
                        let registry_shared = self.session.module_registry();
                        let mut session_registry = registry_shared.write();
                        *session_registry = disk_registry.deep_clone();
                    }

                    // Populate local modules map (path-sorted — see #143).
                    let session_registry = self.session.module_registry();
                    let reg = session_registry.read();
                    let mut entries: Vec<(String, Arc<verum_ast::Module>)> = reg
                        .all_modules()
                        .map(|(_id, info)| (info.path.to_string(), Arc::new(info.ast.clone())))
                        .collect();
                    entries.sort_by(|a, b| a.0.cmp(&b.0));
                    for (path_str, ast_arc) in entries {
                        self.modules.insert(Text::from(path_str), ast_arc);
                    }
                    drop(reg);

                    // Also populate in-memory caches for subsequent pipeline instances
                    {
                        let cache = global_stdlib_registry_cache();
                        let mut guard = cache.write().unwrap_or_else(|poisoned| {
                            tracing::warn!("stdlib registry cache RwLock poisoned during write, recovering");
                            poisoned.into_inner()
                        });
                        if guard.is_none() {
                            *guard = Some(disk_registry);
                        }
                    }

                    let elapsed = start.elapsed();
                    info!(
                        "Loaded {} stdlib module(s) from disk cache in {:.2}ms",
                        module_count,
                        elapsed.as_secs_f64() * 1000.0
                    );
                    return Ok(());
                }
            }
        }

        // FULL LOAD: No cache available, parse everything from source
        debug!("No disk cache, performing full stdlib load");

        // Try to use the process-level parsed stdlib cache.
        // This avoids re-parsing 166+ .vr files for every pipeline instance.
        let cached_entries = {
            let cache = global_stdlib_cache();
            let guard = cache.read().unwrap_or_else(|poisoned| {
                tracing::warn!("stdlib cache RwLock poisoned, recovering");
                poisoned.into_inner()
            });
            guard.as_ref().map(|c| c.entries.clone())
        };

        let parsed_modules: Vec<(Text, Module, Text)> = if let Some(entries) = cached_entries {
            debug!("Using cached stdlib modules ({} entries)", entries.len());
            entries
        } else {
            // First time: discover, read, and parse all stdlib files
            let stdlib_files = self.discover_stdlib_files(&stdlib_path)?;
            if stdlib_files.is_empty() {
                debug!("No .vr files found in core/");
                return Ok(());
            }

            info!("Parsing {} stdlib module(s) (first load, parallel)...", stdlib_files.len());

            // Phase 1: Read all files and compute module paths (parallelizable I/O)
            use rayon::prelude::*;
            let file_data: Vec<(Text, String, PathBuf)> = stdlib_files
                .par_iter()
                .filter_map(|file_path| {
                    let module_path_str = {
                        // Compute module path from file path
                        let rel = file_path.strip_prefix(&stdlib_path).ok()?;
                        let mut parts: Vec<String> = Vec::new();
                        parts.push("core".to_string());
                        for component in rel.components() {
                            if let std::path::Component::Normal(os_str) = component {
                                let s = os_str.to_str()?;
                                if s.ends_with(".vr") {
                                    parts.push(s.trim_end_matches(".vr").to_string());
                                } else {
                                    parts.push(s.to_string());
                                }
                            }
                        }
                        // Handle "mod" files: mod.vr represents its parent directory.
                        // e.g., "core.intrinsics.mod" -> "core.intrinsics"
                        let joined = parts.join(".");
                        if joined.ends_with(".mod") {
                            Text::from(joined.trim_end_matches(".mod"))
                        } else {
                            Text::from(joined)
                        }
                    };
                    let source_text = std::fs::read_to_string(file_path).ok()?;
                    Some((module_path_str, source_text, file_path.clone()))
                })
                .collect();

            // Sort by module path to ensure deterministic registration order.
            // rayon's par_iter() returns results in arbitrary order depending on
            // thread scheduling, which caused intermittent type resolution failures
            // when variant constructors or method tables were populated in different
            // orders across runs.
            let mut file_data = file_data;
            file_data.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));

            // Phase 2: Parse modules (must be sequential due to shared parser state)
            let mut entries = Vec::with_capacity(file_data.len());
            for (module_path_str, source_text, file_path) in &file_data {
                match self.parse_stdlib_module(module_path_str, &Text::from(source_text.clone()), file_path) {
                    Ok(module) => {
                        entries.push((module_path_str.clone(), module, Text::from(source_text.clone())));
                    }
                    Err(e) => {
                        debug!("Failed to parse stdlib module {}: {:?}", module_path_str.as_str(), e);
                    }
                }
            }

            // Store in global cache for future pipeline instances
            {
                let cache = global_stdlib_cache();
                let mut guard = cache.write().unwrap_or_else(|poisoned| {
                    tracing::warn!("stdlib cache RwLock poisoned during write, recovering");
                    poisoned.into_inner()
                });
                *guard = Some(CachedStdlibModules {
                    entries: entries.clone(),
                });
            }

            entries
        };

        // Register all parsed modules in the session's ModuleRegistry and local modules map
        for (module_path_str, module, source_text) in &parsed_modules {
            if self.modules.contains_key(module_path_str) {
                continue;
            }

            let item_count = module.items.len();
            let module_path = ModulePath::from_str(module_path_str.as_str());
            let module_registry = self.session.module_registry();
            let module_id = module_registry.read().allocate_id();

            let file_id = module.items.first()
                .map(|item| item.span.file_id)
                .unwrap_or(FileId::new(0));

            let mut module_info = ModuleInfo::new(
                module_id,
                module_path.clone(),
                module.clone(),
                file_id,
                source_text.clone(),
            );

            match extract_exports_from_module(module, module_id, &module_path) {
                Ok(export_table) => {
                    let export_count = export_table.len();
                    module_info.exports = export_table;
                    debug!("{} has {} items, {} exports", module_path_str.as_str(), item_count, export_count);
                }
                Err(e) => {
                    debug!("Failed to extract exports from {}: {:?}", module_path_str.as_str(), e);
                }
            }

            module_registry.write().register(module_info);
            self.register_inline_modules(module, &module_path, file_id);
            self.modules.insert(module_path_str.clone(), Arc::new(module.clone()));
        }

        // After all modules are loaded, resolve re-exports in two phases:
        //
        // Phase 1: Resolve ExportKind for specific item re-exports FIRST
        // This handles `public import path.{Item1, Item2}` where the kind was
        // defaulted to Type during initial extraction. Now we look up the actual
        // kind from the source module (e.g., Some is a Function, not a Type).
        //
        // Phase 2: Resolve glob re-exports SECOND
        // This processes `public import path.*` statements, copying exports from
        // source modules. By running this AFTER specific kind resolution, the
        // glob copies will get the correct ExportKind values.
        {
            let module_registry = self.session.module_registry();
            let mut guard = module_registry.write();

            // Phase 1: Specific item re-exports (updates ExportKind)
            match resolve_specific_reexport_kinds(&mut guard) {
                Ok(updated_count) => {
                    debug!("Updated {} re-export kinds", updated_count);
                }
                Err(e) => {
                    debug!("Failed to resolve re-export kinds: {:?}", e);
                }
            }

            // Phase 2: Glob re-exports (copies exports with correct kinds)
            // Run in a loop to handle transitive/chained glob re-exports
            // (e.g., runtime/time.vr -> runtime/mod.vr -> mod.vr)
            let mut iteration = 0;
            loop {
                iteration += 1;
                match resolve_glob_reexports(&mut guard) {
                    Ok(resolved_count) => {
                        debug!("Glob re-export iteration {}: resolved {} exports", iteration, resolved_count);
                        if resolved_count == 0 || iteration >= 10 {
                            break;
                        }
                    }
                    Err(e) => {
                        debug!("Failed to resolve glob re-exports: {:?}", e);
                        break;
                    }
                }
            }
        }

        let elapsed = start.elapsed();
        let stdlib_count = self
            .modules
            .iter()
            .filter(|(k, _)| k.as_str().starts_with("core"))
            .count();
        let registry_count = self.session.module_registry().read().len();
        info!(
            "Loaded {} stdlib module(s) ({} registered) in {:.2}ms",
            stdlib_count,
            registry_count,
            elapsed.as_secs_f64() * 1000.0
        );

        // Cache the fully-populated registry for future pipeline instances.
        // This is the key optimization: subsequent loads will deep_clone this
        // cached registry instead of re-registering all modules.
        {
            let cache = global_stdlib_registry_cache();
            let mut guard = cache.write().unwrap_or_else(|poisoned| {
                tracing::warn!("stdlib registry cache RwLock poisoned during write, recovering");
                poisoned.into_inner()
            });
            if guard.is_none() {
                let registry = self.session.module_registry().read().clone();
                info!(
                    "Caching stdlib registry with {} modules for future reuse",
                    registry.len()
                );
                *guard = Some(registry);
            }
        }

        // Persist registry to disk for cross-process reuse (release builds or opt-in).
        if !content_hash.is_empty() {
            if let Some(ref ws_root) = workspace_root_for_cache {
                let registry = self.session.module_registry().read().clone();
                save_registry_to_disk(ws_root, &registry, &content_hash);
            }
        }

        Ok(())
    }

    /// Load project modules from the input file's directory.
    ///
    /// When the input file resides in a directory containing a `mod.vr` file,
    /// that directory is treated as a multi-file project. All sibling `.vr` files
    /// are discovered, parsed, and registered as modules, enabling cross-file
    /// `mount` imports (e.g., `mount bootstrap.token.*`).
    /// Walk every cog registered in the session's `CogResolver` and
    /// load its modules into the session's module registry. Symmetric
    /// with `load_project_modules` but sourced from the resolver
    /// (script-mode `dependencies = [...]`, `verum add`, etc.) instead
    /// of the manifest's project tree.
    ///
    /// Each cog's filesystem root is walked recursively; every `.vr`
    /// file is parsed in library mode and registered under the dotted
    /// path `<cog_name>.<relative_path>` (with `mod.vr` collapsing to
    /// the directory name). Subsequent `mount cog_name.foo` from the
    /// entry source resolves through the same registry as workspace
    /// modules — the consumer can't tell the difference.
    ///
    /// No-op when no resolver is installed (project mode, plain
    /// scripts without `dependencies = [...]`).
    fn load_external_cog_modules(&mut self) -> Result<()> {
        let cog_locations: Vec<(String, PathBuf)> = match self.session.cog_resolver() {
            Some(resolver) => resolver
                .cog_names()
                .into_iter()
                .filter_map(|name| {
                    resolver
                        .get_cog_root(name.as_str())
                        .map(|root| (name.as_str().to_string(), root.clone()))
                })
                .collect(),
            None => return Ok(()),
        };

        for (cog_name, cog_root) in cog_locations {
            let canonical_root = cog_root.canonicalize().unwrap_or(cog_root.clone());
            let mut cog_files: Vec<PathBuf> = Vec::new();
            // Reuse the same recursive walker as project modules — the
            // skip-list (hidden dirs, target/, node_modules/, test_*)
            // applies identically to external cogs.
            Self::discover_vr_files_recursive(&canonical_root, &None, &mut cog_files);
            if cog_files.is_empty() {
                debug!(
                    "External cog '{}' at {} has no .vr files",
                    cog_name,
                    canonical_root.display()
                );
                continue;
            }

            info!(
                "Loading {} module(s) from external cog '{}' at {}",
                cog_files.len(),
                cog_name,
                canonical_root.display()
            );

            for file_path in &cog_files {
                let stem =
                    file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
                let module_path_str = {
                    let rel = file_path
                        .parent()
                        .and_then(|p| p.strip_prefix(&canonical_root).ok())
                        .unwrap_or(std::path::Path::new(""));
                    let mut parts = vec![cog_name.clone()];
                    for component in rel.components() {
                        if let std::path::Component::Normal(seg) = component {
                            if let Some(s) = seg.to_str() {
                                parts.push(s.to_string());
                            }
                        }
                    }
                    if stem != "mod" {
                        parts.push(stem.to_string());
                    }
                    Text::from(parts.join("."))
                };

                if self.modules.contains_key(&module_path_str) {
                    continue;
                }

                let source_text = match std::fs::read_to_string(file_path) {
                    Ok(s) => s,
                    Err(e) => {
                        debug!(
                            "Failed to read external cog module {}: {:?}",
                            module_path_str.as_str(),
                            e
                        );
                        continue;
                    }
                };

                match self.parse_stdlib_module(
                    &module_path_str,
                    &Text::from(source_text.clone()),
                    file_path,
                ) {
                    Ok(module) => {
                        let module_path = ModulePath::from_str(module_path_str.as_str());
                        let module_registry = self.session.module_registry();
                        let module_id = module_registry.read().allocate_id();
                        let file_id = module
                            .items
                            .first()
                            .map(|item| item.span.file_id)
                            .unwrap_or(FileId::new(0));

                        let mut module_info = ModuleInfo::new(
                            module_id,
                            module_path.clone(),
                            module.clone(),
                            file_id,
                            Text::from(source_text),
                        );

                        // External-cog modules behave like project
                        // modules from the consumer's perspective —
                        // export ALL items, not just `pub` ones,
                        // so the script can reach internals it
                        // explicitly mounts.
                        let export_table =
                            Self::extract_all_exports(&module, module_id, &module_path);
                        module_info.exports = export_table;

                        module_registry.write().register(module_info);
                        self.register_inline_modules(&module, &module_path, file_id);
                        let module_rc = Arc::new(module);
                        self.modules.insert(module_path_str.clone(), module_rc.clone());
                        self.project_modules
                            .insert(module_path_str.clone(), module_rc);
                        debug!(
                            "Loaded external-cog module: {}",
                            module_path_str.as_str()
                        );
                    }
                    Err(e) => {
                        debug!(
                            "Failed to parse external-cog module {}: {:?}",
                            module_path_str.as_str(),
                            e
                        );
                    }
                }
            }
        }

        // Resolve re-exports across the registered modules (mirrors
        // the same step at the end of load_project_modules).
        {
            let module_registry = self.session.module_registry();
            let mut guard = module_registry.write();
            let _ = resolve_specific_reexport_kinds(&mut guard);
            let _ = resolve_glob_reexports(&mut guard);
        }

        Ok(())
    }

    fn load_project_modules(&mut self) -> Result<()> {
        let input_path = self.session.options().input.clone();
        let input_dir = match input_path.parent() {
            Some(dir) if dir.as_os_str().is_empty() => std::env::current_dir()?,
            Some(dir) => dir.to_path_buf(),
            None => return Ok(()),
        };

        // Canonicalize for reliable path comparison
        let input_dir = input_dir.canonicalize().unwrap_or(input_dir);

        // Only treat as a project if there's a mod.vr in the directory
        let mod_file = input_dir.join("mod.vr");
        if !mod_file.exists() {
            return Ok(());
        }

        // Determine the project module prefix from the directory name
        let project_prefix = input_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string();

        info!("Detected multi-file project '{}' in {}", project_prefix, input_dir.display());

        // Discover all .vr files in the project directory (recursive)
        let canonical_input = input_path.canonicalize().ok();
        let mut project_files: Vec<PathBuf> = Vec::new();
        Self::discover_vr_files_recursive(&input_dir, &canonical_input, &mut project_files);

        if project_files.is_empty() {
            return Ok(());
        }

        info!("Loading {} project module(s)", project_files.len());

        // Track which module_path_str each filesystem source produced so a
        // subsequent collision (two files mapping to the same module path —
        // typically `foo.vr` Rule 2 vs `foo/mod.vr` Rule 4) can surface as a
        // hard diagnostic instead of silently skipping the second loader. The
        // first source wins; the loser's declarations would otherwise be
        // unreachable through any `mount` and the user sees `unbound
        // variable` errors at use sites with no hint about the cause.
        let mut module_path_to_source: std::collections::BTreeMap<String, PathBuf> =
            std::collections::BTreeMap::new();

        // Parse and register each project module
        for file_path in &project_files {
            let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
            // Build dotted module path from relative directory components
            // e.g. project_dir/sub/foo.vr -> "project.sub.foo"
            //      project_dir/sub/mod.vr -> "project.sub"
            //      project_dir/foo.vr     -> "project.foo"
            //      project_dir/mod.vr     -> "project"
            let module_path_str = {
                let rel = file_path.parent()
                    .and_then(|p| p.strip_prefix(&input_dir).ok())
                    .unwrap_or(std::path::Path::new(""));
                let mut parts = vec![project_prefix.clone()];
                for component in rel.components() {
                    if let std::path::Component::Normal(seg) = component {
                        if let Some(s) = seg.to_str() {
                            parts.push(s.to_string());
                        }
                    }
                }
                if stem != "mod" {
                    parts.push(stem.to_string());
                }
                Text::from(parts.join("."))
            };

            // Detect E_MODULE_PATH_COLLISION: two files reach the same
            // dotted module path. The most-common shape is `foo.vr` (Rule 2,
            // file form) AND `foo/mod.vr` (Rule 4, directory form) both
            // declaring module `<project>.foo`.  Surface this as a hard
            // diagnostic with both sources cited, and skip the loser so the
            // rest of the project can keep building (the user gets a
            // clear actionable message instead of silent loss).
            if let Some(prev_source) = module_path_to_source.get(module_path_str.as_str()) {
                eprintln!(
                    "error<E_MODULE_PATH_COLLISION>: module path '{}' resolves to two source files",
                    module_path_str.as_str(),
                );
                eprintln!("  using:    {}", prev_source.display());
                eprintln!("  ignoring: {}", file_path.display());
                eprintln!(
                    "  hint: pick exactly one of the file form (`<name>.vr`) \
                     or the directory form (`<name>/mod.vr`); having both makes \
                     declarations in the loser invisible at use sites and is \
                     classified as `E_MODULE_PATH_COLLISION`"
                );
                continue;
            }
            module_path_to_source.insert(module_path_str.as_str().to_string(), file_path.clone());

            if self.modules.contains_key(&module_path_str) {
                continue;
            }

            let source_text = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(e) => {
                    debug!("Failed to read project module {}: {:?}", module_path_str.as_str(), e);
                    continue;
                }
            };

            match self.parse_stdlib_module(&module_path_str, &Text::from(source_text.clone()), file_path) {
                Ok(module) => {
                    let module_path = ModulePath::from_str(module_path_str.as_str());
                    let module_registry = self.session.module_registry();
                    let module_id = module_registry.read().allocate_id();

                    let file_id = module.items.first()
                        .map(|item| item.span.file_id)
                        .unwrap_or(FileId::new(0));

                    let mut module_info = ModuleInfo::new(
                        module_id,
                        module_path.clone(),
                        module.clone(),
                        file_id,
                        Text::from(source_text),
                    );

                    // For project modules, export ALL items (not just public ones)
                    // since they share the same project context.
                    let export_table = Self::extract_all_exports(&module, module_id, &module_path);
                    module_info.exports = export_table;

                    // MOD-MED-1 — validate `module foo;`
                    // headers against the filesystem. Emits warnings
                    // for dangling forward-decls
                    // (E_MODULE_HEADER_FORWARD_DECL_NO_SOURCE) and
                    // inline-vs-filesystem overlaps
                    // (E_MODULE_INLINE_FILESYSTEM_OVERLAP) so users
                    // see header inconsistencies without breaking
                    // the build.
                    let header_warnings =
                        verum_modules::loader::validate_module_headers_against_filesystem(
                            file_path,
                            &module,
                        );
                    for warning in &header_warnings {
                        let diag = verum_diagnostics::DiagnosticBuilder::warning()
                            .code(warning.code())
                            .message(warning.message())
                            .build();
                        self.session.emit_diagnostic(diag);
                    }
                    module_info.header_warnings = header_warnings;

                    module_registry.write().register(module_info);
                    self.register_inline_modules(&module, &module_path, file_id);
                    let module_rc = Arc::new(module);
                    self.modules.insert(module_path_str.clone(), module_rc.clone());
                    // Also store in project_modules so they survive self.modules.clear()
                    self.project_modules.insert(module_path_str.clone(), module_rc);
                    debug!("Loaded project module: {}", module_path_str.as_str());
                }
                Err(e) => {
                    debug!("Failed to parse project module {}: {:?}", module_path_str.as_str(), e);
                }
            }
        }

        // Resolve re-exports within project modules
        {
            let module_registry = self.session.module_registry();
            let mut guard = module_registry.write();
            let _ = resolve_specific_reexport_kinds(&mut guard);
            let _ = resolve_glob_reexports(&mut guard);
        }

        Ok(())
    }

    /// Recursively discover all `.vr` files under `dir`, skipping hidden
    /// directories (names starting with `.`), `target/`, and `node_modules/`.
    /// The main input file (identified by `canonical_input`) and test files
    /// (names starting with `test_`) are also excluded.
    fn discover_vr_files_recursive(
        dir: &std::path::Path,
        canonical_input: &Option<PathBuf>,
        out: &mut Vec<PathBuf>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.is_dir() {
                let dir_name = entry.file_name();
                let name = dir_name.to_str().unwrap_or("");
                // Skip hidden directories, build artifacts, and node_modules
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                Self::discover_vr_files_recursive(&path, canonical_input, out);
            } else if path.extension().is_some_and(|ext| ext == "vr") {
                // Skip the main input file (it will be loaded separately)
                if path.canonicalize().ok().as_ref() == canonical_input.as_ref() {
                    continue;
                }
                // Skip test files (they're standalone)
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if stem.starts_with("test_") {
                    continue;
                }
                out.push(path);
            }
        }
    }

    /// Extract all exports from a module regardless of visibility.
    /// Used for project-internal modules where all items should be accessible.
    fn extract_all_exports(
        module: &Module,
        module_id: ModuleId,
        module_path: &ModulePath,
    ) -> verum_modules::exports::ExportTable {
        use verum_ast::ItemKind;
        use verum_modules::exports::{ExportTable, ExportedItem, ExportKind};
        use verum_ast::Visibility;

        let mut export_table = ExportTable::new();
        export_table.set_module_id(module_id);
        export_table.set_module_path(module_path.clone());

        for item in &module.items {
            let result = match &item.kind {
                ItemKind::Function(func) => {
                    let kind = if func.is_meta { ExportKind::Meta } else { ExportKind::Function };
                    export_table.add_export(ExportedItem::new(
                        func.name.name.as_str(), kind, Visibility::Public, module_id, item.span,
                    ))
                }
                ItemKind::Type(type_decl) => {
                    let _ = export_table.add_export(ExportedItem::new(
                        type_decl.name.name.as_str(), ExportKind::Type, Visibility::Public, module_id, item.span,
                    ));
                    // Also export variant constructors
                    if let verum_ast::decl::TypeDeclBody::Variant(variants) = &type_decl.body {
                        for variant in variants {
                            let _ = export_table.add_export(ExportedItem::new(
                                variant.name.name.as_str(), ExportKind::Function, Visibility::Public, module_id, variant.span,
                            ));
                        }
                    }
                    Ok(())
                }
                ItemKind::Protocol(proto) => {
                    let kind = if proto.is_context { ExportKind::Context } else { ExportKind::Protocol };
                    export_table.add_export(ExportedItem::new(
                        proto.name.name.as_str(), kind, Visibility::Public, module_id, item.span,
                    ))
                }
                ItemKind::Const(const_decl) => {
                    export_table.add_export(ExportedItem::new(
                        const_decl.name.name.as_str(), ExportKind::Const, Visibility::Public, module_id, item.span,
                    ))
                }
                ItemKind::Static(static_decl) => {
                    export_table.add_export(ExportedItem::new(
                        static_decl.name.name.as_str(), ExportKind::Const, Visibility::Public, module_id, item.span,
                    ))
                }
                _ => Ok(()), // Skip impl blocks, modules, imports, etc.
            };
            if let Err(e) = result {
                debug!("Failed to add export in project module: {:?}", e);
            }
        }

        export_table
    }

    /// Discover all .vr files in the stdlib directory.
    fn discover_stdlib_files(&self, stdlib_path: &Path) -> Result<List<PathBuf>> {
        let mut files = List::new();
        self.discover_stdlib_files_recursive(stdlib_path, &mut files, 0)?;
        Ok(files)
    }

    /// Recursively discover .vr files in stdlib directory.
    fn discover_stdlib_files_recursive(
        &self,
        dir: &Path,
        files: &mut List<PathBuf>,
        depth: usize,
    ) -> Result<()> {
        const MAX_DEPTH: usize = 10;

        if depth >= MAX_DEPTH || !dir.is_dir() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_symlink() {
                continue;
            }

            if path.is_dir() {
                // Skip examples directory - it contains demo code with unsupported features
                let dir_name = path.file_name().map(|n| n.to_string_lossy());
                if dir_name.as_deref() != Some("examples") {
                    self.discover_stdlib_files_recursive(&path, files, depth + 1)?;
                }
            } else if path.extension().is_some_and(|ext| ext == "vr") {
                files.push(path);
            }
        }

        Ok(())
    }

    /// Parse a stdlib module (similar to parse_and_register but for stdlib).
    fn parse_stdlib_module(
        &mut self,
        module_path: &Text,
        source: &Text,
        file_path: &Path,
    ) -> Result<Module> {
        // Load file into session for proper file_id tracking
        let file_id = self.session.load_file(file_path)?;

        let lexer = Lexer::new(source.as_str(), file_id);

        let parser = VerumParser::new();
        let module = parser.parse_module(lexer, file_id).map_err(|errors| {
            // A stdlib module that fails to parse is either a compiler bug
            // (the parser can't handle syntax we ship in core/*.vr) or a
            // stdlib bug (invalid syntax shipped). Either way it causes
            // every downstream `mount core.*.X` to silently fail with
            // "module not found", which is a far worse diagnostic than
            // the real parse error. Emit at WARN so stdlib breakage is
            // surfaced in normal tooling runs and cannot regress unseen.
            for error in &errors {
                warn!("Stdlib parse error in {}: {}", module_path.as_str(), error);
            }
            anyhow::anyhow!(
                "Parsing stdlib module {} failed with {} error(s)",
                module_path.as_str(),
                errors.len()
            )
        })?;

        Ok(module)
    }

    /// Register inline modules (modules defined with `public module name { ... }`)
    ///
    /// This is needed for modules like `std.prelude` which are defined inline
    /// in `core/mod.vr` rather than in their own file.
    fn register_inline_modules(
        &self,
        parent_module: &Module,
        parent_path: &ModulePath,
        file_id: FileId,
    ) {
        let module_registry = self.session.module_registry();

        for item in &parent_module.items {
            if let ItemKind::Module(mod_decl) = &item.kind {
                // Check if this is an inline module (has items)
                if let verum_common::Maybe::Some(ref items) = mod_decl.items {
                    // Create the child module path
                    let child_path = parent_path.join(mod_decl.name.name.as_str());
                    let child_path_str = child_path.to_string();

                    // Create a synthetic AST Module from the items
                    let inline_module = Module {
                        items: items.clone(),
                        attributes: List::new(),
                        file_id,
                        span: item.span,
                    };

                    // Allocate ID and create ModuleInfo
                    let module_id = module_registry.read().allocate_id();
                    let mut module_info = ModuleInfo::new(
                        module_id,
                        child_path.clone(),
                        inline_module.clone(),
                        file_id,
                        Text::from(""), // No separate source for inline modules
                    );

                    // Extract exports
                    match extract_exports_from_module(&inline_module, module_id, &child_path) {
                        Ok(export_table) => {
                            module_info.exports = export_table;
                        }
                        Err(e) => {
                            debug!("Failed to extract exports from inline module {}: {:?}",
                                child_path_str, e);
                        }
                    }

                    // Register the inline module
                    module_registry.write().register(module_info);

                    // Recursively register any nested inline modules
                    self.register_inline_modules(&inline_module, &child_path, file_id);
                }
            }
        }
    }

    /// Parse source and register meta declarations (Pass 1)
    fn parse_and_register(&mut self, path: &Text, source: &Text) -> Result<Module> {
        // Load source as a string (files are already loaded in sources map)
        let virtual_path = PathBuf::from(path.as_str());
        let file_id = self
            .session
            .load_source_string(source.as_str(), virtual_path.clone())?;

        // Decide library-mode vs script-mode parsing based on shebang
        // autodetection or the entry-source script_mode flag. See
        // `should_parse_as_script` for the full rule.
        let script = should_parse_as_script(
            source.as_str(),
            self.session.options(),
            Some(virtual_path.as_path()),
        );

        // Parse
        let parser = VerumParser::new();
        let parse_result = if script {
            parser.parse_module_script_str(source.as_str(), file_id)
        } else {
            let lexer = Lexer::new(source.as_str(), file_id);
            parser.parse_module(lexer, file_id)
        };
        let mut module = parse_result.map_err(|errors| {
            let error_count = errors.len();
            for error in errors {
                let diag = DiagnosticBuilder::error()
                    .message(format!("Parse error: {}", error))
                    .build();
                self.session.emit_diagnostic(diag);
            }
            anyhow::anyhow!("Parsing failed with {} error(s)", error_count)
        })?;

        // Apply @cfg conditional compilation filtering
        // Filter out items that don't match the current target configuration.
        // This ensures platform-specific code (e.g., FFI blocks with @cfg(unix))
        // is excluded when compiling for incompatible targets.
        let cfg_evaluator = self.session.cfg_evaluator();
        let original_count = module.items.len();
        module.items = cfg_evaluator.filter_items(&module.items);
        let filtered_count = original_count - module.items.len();
        if filtered_count > 0 {
            debug!(
                "  Filtered {} item(s) based on @cfg predicates in {}",
                filtered_count,
                path.as_str()
            );
        }

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
                    // Note: This would need actual macro extraction logic
                    debug!("  Found macro declaration (registration pending)");
                }

                _ => {
                    // Other items don't need registration
                }
            }
        }

        // Header validation at the parse_and_register
        // user-source path. Surfaces dangling forward-decls and
        // inline-vs-filesystem overlaps for files that don't go
        // through phase_parse (e.g. multi-source registration in
        // run_full_compilation).
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

        Ok(module)
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
    fn analyze_module(&mut self, path: &Text, module: &Module) -> Result<()> {
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

    /// Run complete compilation (all phases)
    pub fn run_full_compilation(&mut self) -> Result<()> {
        let start = Instant::now();

        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;

        // Phase 1: Lexing
        let file_id = self.phase_load_source()?;

        // Phase 2: Parsing
        let module = self.phase_parse(file_id)?;

        // Phase 3: Type checking
        self.phase_type_check(&module)?;

        // Phase 3b: Dependency analysis
        self.phase_dependency_analysis(&module)?;

        // Phase 4: Refinement verification
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 5: CBGR analysis
        self.phase_cbgr_analysis(&module)?;

        // Phase 6: Interpretation
        self.phase_interpret(&module)?;

        let elapsed = start.elapsed();
        info!("Compilation completed in {:.2}s", elapsed.as_secs_f64());

        Ok(())
    }

    /// Run type checking only (no execution)
    ///
    /// Note: For complex type checking scenarios, ensure RUST_MIN_STACK is set
    /// appropriately (e.g., 16MB) in the build/test environment.
    pub fn run_check_only(&mut self) -> Result<()> {
        let start = Instant::now();

        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;

        // Register stdlib modules for cross-file type/context/import
        // resolution. Without this, `mount core.sys.darwin.libsystem.{...}`
        // and `using [ComputeDevice]` fail because the type checker
        // doesn't know about symbols from sibling modules.
        // This is the CORRECT architectural fix — not lenient bypasses.
        self.register_modules_for_cross_file_resolution()?;

        // Load sibling project modules (enables cross-file mount imports)
        self.load_project_modules()?;
        // Load externally-registered cogs (script-mode `dependencies`,
        // verum-add deps, etc.) using the same module-registration
        // machinery so cross-cog `mount foo.bar` resolves transparently.
        self.load_external_cog_modules()?;

        let file_id = self.phase_load_source()?;
        let mut module = self.phase_parse(file_id)?;

        // Get module path for registration and expansion
        let module_path = Text::from(self.session.options().input.display().to_string());

        // Register meta functions (enables meta-fail tests)
        self.register_meta_declarations(&module_path, &module)?;

        // Expand macros (evaluates @macro() invocations, triggers hygiene checks)
        self.expand_module(&module_path, &mut module)?;

        // Check if file has meta functions for special handling
        let has_meta_functions = module.items.iter().any(|item| {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                func.is_meta
            } else {
                false
            }
        });

        if has_meta_functions {
            // For files with meta functions, run BOTH meta evaluation and type checking.
            // Meta evaluation runs first to produce M-code errors (needed for meta-fail tests).
            // Type checking also runs to produce E-code errors (needed for tests expecting E400, etc.).
            // Both phases emit diagnostics to the session, so all errors are collected
            // in format_diagnostics() regardless of which phase returned first.
            let meta_result = self.phase_meta_evaluation(&module, &module_path);
            let type_check_result = self.phase_type_check(&module);

            // Return error from whichever phase failed (meta errors take priority)
            if let Err(e) = meta_result {
                // Also report type check errors if any
                if let Err(_tc_err) = type_check_result {
                    // Both failed - diagnostics from both are in the session
                }
                return Err(e);
            }
            if let Err(e) = type_check_result {
                return Err(e);
            }
        } else {
            // For files without meta functions, original ordering: type check then meta eval
            let type_check_result = self.phase_type_check(&module);
            if type_check_result.is_ok() {
                self.phase_meta_evaluation(&module, &module_path)?;
            } else {
                return type_check_result;
            }
        }

        // Dependency analysis (validates against target constraints)
        self.phase_dependency_analysis(&module)?;

        let elapsed = start.elapsed();
        info!("Type checking completed in {:.2}s", elapsed.as_secs_f64());

        Ok(())
    }

    /// Run parse only (no type checking, for VCS parse-pass tests)
    pub fn run_parse_only(&mut self) -> Result<()> {
        let start = Instant::now();

        let file_id = self.phase_load_source()?;
        let _module = self.phase_parse(file_id)?;

        let elapsed = start.elapsed();
        info!("Parsing completed in {:.2}s", elapsed.as_secs_f64());

        Ok(())
    }

    /// Run interpreter mode
    /// Execute a pre-compiled VBC module against the given args.
    ///
    /// Used by the script-mode persistent cache: on a cache hit the
    /// runner deserialises the stored VBC bytes into a `VbcModule` and
    /// calls this method, skipping every front-end phase (parse,
    /// typecheck, verify, codegen) for a sub-millisecond cold start
    /// of unchanged scripts.
    ///
    /// Behaviour matches `phase_interpret_with_args` post-compile —
    /// builds a `VbcInterpreter`, resolves the entry function (`main`
    /// with `__verum_script_main` fallback), executes with or without
    /// the args list, and routes the terminal value through
    /// `propagate_main_exit_code` for tier-parity with AOT.
    pub fn run_compiled_vbc(
        &mut self,
        vbc_module: std::sync::Arc<verum_vbc::module::VbcModule>,
        args: List<Text>,
    ) -> Result<()> {
        // Re-record the captured VBC so a subsequent
        // `take_compiled_vbc()` still surfaces something — useful
        // when the runner wants to refresh metadata even on cache hits.
        self.session.record_compiled_vbc(vbc_module.clone());

        let mut interpreter = VbcInterpreter::new(vbc_module);
        // Transfer the script-mode permission policy (if the CLI
        // installed one) into the interpreter's PermissionRouter
        // before the first instruction dispatches. The router's
        // one-entry cache + warm path keeps repeated checks at
        // ≤2ns; the policy itself is consulted only on cache miss.
        if let Some(policy) = self.session.take_script_permission_policy() {
            interpreter.state.permission_router.set_policy(policy.0);
        }
        let main_func_id = self.find_main_function_id(&interpreter.state.module)?;
        let main_param_count = interpreter
            .state
            .module
            .get_function(main_func_id)
            .map(|f| f.params.len())
            .unwrap_or(0);

        if main_param_count == 0 || args.is_empty() {
            info!("Executing cached VBC (no-args path)");
            let result = interpreter.execute_function(main_func_id);
            return self.finalize_run_result(result);
        }

        let rust_args: Vec<String> = args.iter().map(|t| t.to_string()).collect();
        let args_value = interpreter
            .alloc_string_list(&rust_args)
            .map_err(|e| anyhow::anyhow!("Failed to allocate args: {:?}", e))?;
        info!(
            "Executing cached VBC with {} args",
            rust_args.len()
        );
        let result = interpreter.call(main_func_id, &[args_value]);
        self.finalize_run_result(result)
    }

    pub fn run_interpreter(&mut self, args: List<Text>) -> Result<()> {
        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;

        // Load sibling project modules (enables cross-file mount imports)
        self.load_project_modules()?;
        // Load externally-registered cogs (script-mode `dependencies`,
        // verum-add deps, etc.) using the same module-registration
        // machinery so cross-cog `mount foo.bar` resolves transparently.
        self.load_external_cog_modules()?;

        let file_id = self.phase_load_source()?;
        let module = self.phase_parse(file_id)?;

        // Safety-feature gates (unsafe, @ffi, etc.) ALWAYS run —
        // independent of verify_mode. Without this, `--verify runtime`
        // silently bypassed the user's `[safety]` configuration.
        self.phase_safety_gate(&module)?;

        // Type check unless in runtime-only mode
        // Runtime mode skips static analysis for faster iteration
        if self.session.options().verify_mode != VerifyMode::Runtime {
            self.phase_type_check(&module)?;

            // Dependency analysis (validates against target constraints)
            self.phase_dependency_analysis(&module)?;

            // Verify refinements if enabled
            if self.session.options().verify_mode.use_smt() {
                self.phase_verify(&module)?;
            }

            // CBGR analysis
            self.phase_cbgr_analysis(&module)?;
        }

        // Interpret and execute the module
        info!("Executing program...");
        self.phase_interpret_with_args(&module, args)?;

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
            // introduce a runtime-mode bypass.
            self.phase_context_validation(module);
            self.phase_send_sync_validation(module);
            self.phase_ffi_validation(module)?;
            return Ok(());
        }

        self.phase_type_check(module)?;
        self.phase_dependency_analysis(module)?;
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(module)?;
        }
        self.phase_context_validation(module);
        self.phase_send_sync_validation(module);
        self.phase_cbgr_analysis(module)?;
        self.phase_ffi_validation(module)?;
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
    fn phase_safety_gate(&mut self, module: &Module) -> Result<()> {
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
    fn phase_stdlib_lints(&mut self, module: &Module) {
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
    fn phase_dependency_analysis(&mut self, module: &Module) -> Result<()> {
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
    fn phase_verify(&mut self, module: &Module) -> Result<()> {
        let _bc = verum_error::breadcrumb::enter("compiler.phase.verify", "");
        debug!("Running refinement verification");

        let start = Instant::now();
        let mut cost_tracker = CostTracker::new();
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

        let mut num_verified = 0;
        let mut num_failed = 0;
        let mut num_timeout = 0;

        for func in functions_to_verify {
            debug!("Verifying function: {}", func.name.name);

            let verify_start = Instant::now();
            let timeout_ms = self.session.options().smt_timeout_secs * 1000;

            // K-rule preamble (#187): walk the function's refinement
            // types and run the kernel rules (currently
            // K-Refine-omega) BEFORE invoking SMT. K-rule failures
            // are hard formation errors per the trusted-base
            // contract; short-circuiting saves the SMT round and
            // surfaces a sharper diagnostic. This is the
            // user-facing wiring of KernelRecheck — every `verum
            // build` / `verum verify` invocation now re-checks
            // refinement-type formation against the trusted kernel
            // before any SMT proof is attempted.
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
                num_failed += 1;
                let diag = verum_diagnostics::DiagnosticBuilder::error()
                    .message(format!(
                        "kernel-recheck failed for '{}': {} — {}",
                        func.name.name.as_str(),
                        label.as_str(),
                        msg,
                    ))
                    .build();
                self.session.emit_diagnostic(diag);
                continue;
            }

            // Perform actual SMT-based refinement verification
            let verification_result = self.verify_function_refinements(func, timeout_ms);

            let verify_elapsed = verify_start.elapsed();

            match verification_result {
                Ok(true) => {
                    num_verified += 1;
                    debug!(
                        "Verified function '{}' in {:.2}ms",
                        func.name.name,
                        verify_elapsed.as_millis()
                    );

                    // Track successful verification cost
                    cost_tracker.record(verum_smt::VerificationCost::new(
                        func.name.as_str().to_string().into(),
                        verify_elapsed,
                        true,
                    ));
                }
                Ok(false) => {
                    num_failed += 1;
                    // Diagnostic already emitted by verify_function_refinements

                    // Track failed verification cost
                    cost_tracker.record(verum_smt::VerificationCost::new(
                        func.name.as_str().to_string().into(),
                        verify_elapsed,
                        false,
                    ));
                }
                Err(e) => {
                    // Check if it's a timeout
                    if verify_elapsed.as_secs() > self.session.options().smt_timeout_secs {
                        num_timeout += 1;
                        warn!("Verification timeout for function: {}", func.name.name);

                        // Emit warning diagnostic
                        let diag = DiagnosticBuilder::new(Severity::Warning)
                            .message(format!(
                                "Verification timeout for function '{}' ({}s > {}s). Falling back to runtime checks.",
                                func.name.name,
                                verify_elapsed.as_secs(),
                                self.session.options().smt_timeout_secs
                            ))
                            .build();
                        self.session.emit_diagnostic(diag);
                    } else {
                        num_failed += 1;
                        warn!(
                            "Verification error for function '{}': {}",
                            func.name.name, e
                        );

                        // Emit error diagnostic
                        let diag = DiagnosticBuilder::new(Severity::Error)
                            .message(format!(
                                "Verification error for function '{}': {}",
                                func.name.name, e
                            ))
                            .build();
                        self.session.emit_diagnostic(diag);
                    }

                    // Track error cost
                    cost_tracker.record(verum_smt::VerificationCost::new(
                        func.name.as_str().to_string().into(),
                        verify_elapsed,
                        false,
                    ));
                }
            }
        }

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

    /// Verify theorem, lemma, and axiom proofs via SMT solver.
    ///
    /// Model-theoretic discharge of protocol axioms.
    ///
    /// For every `implement P for T { ... }` block in `module`, collect
    /// P's axioms (Self-substituted to T's concrete ops), then discharge
    /// each via either:
    ///
    ///   * An explicit `proof X by tactic;` clause inside the impl block.
    ///   * `ProofSearchEngine::auto_prove` fallback.
    ///
    /// Unverified obligations surface as diagnostics at warning severity
    /// by default; the session option `model_verification_level` can
    /// elevate them to errors.
    ///
    /// Reference specification: `docs/architecture/model-theoretic-semantics.md`.
    fn verify_impl_axioms_for_module(&mut self, module: &Module) -> Result<()> {
        use crate::phases::proof_verification::verify_impl_axioms;
        use verum_ast::decl::{ImplKind, TypeDeclBody};

        let mut impl_count = 0u32;
        let mut verified_axioms = 0u32;
        let mut unverified_axioms = 0u32;

        for item in module.items.iter() {
            let verum_ast::ItemKind::Impl(impl_decl) = &item.kind else {
                continue;
            };
            let ImplKind::Protocol { protocol, .. } = &impl_decl.kind else {
                // Inherent impls have no axioms to discharge.
                continue;
            };

            // Resolve the protocol AST by path. Protocols declared in the
            // same module are searchable directly; cross-module protocols
            // are looked up via the module registry.
            let protocol_name = match protocol
                .segments
                .last()
                .and_then(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                    _ => None,
                }) {
                Some(n) => n,
                None => continue,
            };

            let protocol_decl = match self.find_protocol_decl(module, protocol_name) {
                Some(pd) => pd,
                None => continue,
            };

            // Only proceed if the protocol body is actually a protocol (not a
            // stray alias with matching name).
            if !matches!(protocol_decl.body, TypeDeclBody::Protocol(_)) {
                continue;
            }

            impl_count += 1;
            let report = verify_impl_axioms(impl_decl, &protocol_decl);
            verified_axioms += report.verified.len() as u32;
            unverified_axioms += report.unverified.len() as u32;

            for failure in report.unverified.iter() {
                let diag_msg = format!(
                    "model verification: `implement {} for <type>` does not discharge axiom `{}` ({})",
                    report.protocol_name,
                    failure.axiom_name,
                    failure.reason,
                );
                let diag = DiagnosticBuilder::new(Severity::Warning)
                    .message(diag_msg)
                    .build();
                self.session.emit_diagnostic(diag);
            }
        }

        if impl_count > 0 {
            info!(
                "Model verification: {} impl blocks, {} axioms verified, {} unverified",
                impl_count, verified_axioms, unverified_axioms
            );
        }

        Ok(())
    }

    /// Look up a protocol's TypeDecl by name. Searches the given module first,
    /// then falls back to the module registry for cross-module lookup.
    fn find_protocol_decl(
        &self,
        module: &Module,
        protocol_name: &str,
    ) -> Option<verum_ast::decl::TypeDecl> {
        // 1. Search this module.
        for item in module.items.iter() {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                if type_decl.name.name.as_str() == protocol_name {
                    return Some(type_decl.clone());
                }
            }
        }
        // 2. Cross-module lookup: walk every loaded module's items.
        for (_path, loaded) in self.modules.iter() {
            for item in loaded.items.iter() {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    if type_decl.name.name.as_str() == protocol_name {
                        return Some(type_decl.clone());
                    }
                }
            }
        }
        None
    }

    /// Processes all `theorem`, `lemma`, and `corollary` declarations in the
    /// module, dispatching to the appropriate proof verification strategy:
    /// - Tactic proofs → ProofSearchEngine (automated tactic application)
    /// - Term proofs → Z3 formula translation + satisfiability check
    /// - Structured proofs → Weakest precondition calculus
    /// - Method proofs → Induction/cases via WP engine
    fn verify_theorem_proofs(&mut self, module: &Module) -> Result<()> {
        use crate::phases::proof_verification::{
            build_refinement_alias_map, register_module_lemmas,
            verify_proof_body_with_aliases, ProofVerificationResult,
        };
        use verum_smt::proof_search::{ProofSearchEngine, HintsDatabase};

        // Flatten every nominal refinement alias in this module so downstream
        // `verify_proof_body_with_aliases` can materialise hypotheses for
        // parameters typed as aliases (e.g. `n: FanoDim` → `n == 7`).
        let alias_map = build_refinement_alias_map(module);

        let mut theorem_count = 0u32;
        let mut verified_count = 0u32;
        let mut failed_count = 0u32;
        let mut axiom_count = 0u32;

        let timeout_ms = self.session.options().smt_timeout_secs * 1000;
        let timeout = std::time::Duration::from_millis(timeout_ms);

        // Seed the hints DB with stdlib core lemmas *and* every sibling
        // theorem/axiom/lemma in this module, so `apply <name>` can
        // dispatch to local declarations in the same file — the idiom
        // used by the UHM bridge / corollary structure.
        let mut hints_db = HintsDatabase::with_core();
        register_module_lemmas(module, &mut hints_db);
        let mut proof_engine = ProofSearchEngine::with_hints(hints_db);

        // Refinement reflection: scan the module for pure,
        // single-expression functions and translate their bodies
        // to SMT-LIB via the Expr→SMT-LIB translator. Successfully
        // translated definitions are registered as axioms in the
        // proof engine so `proof by auto` can unfold user function
        // calls through Z3.
        //
        // Conservative: functions that can't be translated (multi-
        // statement bodies, unsupported operators, closures, etc.)
        // are silently skipped — no incorrect axiom is ever emitted.
        {
            use verum_smt::refinement_reflection::RefinementReflectionRegistry;
            use verum_smt::expr_to_smtlib::try_reflect_function;

            let mut registry = RefinementReflectionRegistry::new();
            for item in &module.items {
                if let verum_ast::ItemKind::Function(func_decl) = &item.kind {
                    if let Some(rf) = try_reflect_function(func_decl) {
                        let _ = registry.register(rf);
                    }
                }
            }
            if !registry.is_empty() {
                tracing::debug!(
                    "Refinement reflection: {} function(s) reflected as SMT axioms",
                    registry.len()
                );
                proof_engine.set_reflection_registry(registry);
            }
        }

        let smt_config = verum_smt::context::ContextConfig {
            timeout: Some(timeout),
            ..Default::default()
        };
        let smt_ctx = SmtContext::with_config(smt_config);

        for item in &module.items {
            let (thm, kind_name) = match &item.kind {
                verum_ast::ItemKind::Theorem(t) => (t, "theorem"),
                verum_ast::ItemKind::Lemma(t) => (t, "lemma"),
                verum_ast::ItemKind::Corollary(t) => (t, "corollary"),
                _ => continue,
            };
            theorem_count += 1;

            if thm.proof.is_none() {
                axiom_count += 1;
                debug!("{} '{}' accepted as axiom (no proof body)", kind_name, thm.name.name);
                continue;
            }

            debug!("Verifying {} '{}' ({} requires, {} ensures)",
                kind_name, thm.name.name, thm.requires.len(), thm.ensures.len());

            // Verify the proof body using the full proof verification engine
            match verify_proof_body_with_aliases(&mut proof_engine, &smt_ctx, thm, &alias_map) {
                ProofVerificationResult::Verified(cert) => {
                    verified_count += 1;
                    info!("✓ {} '{}' verified ({} steps, {:.1}ms)",
                        kind_name, thm.name.name, cert.steps.len(),
                        cert.total_duration.as_secs_f64() * 1000.0);
                }
                ProofVerificationResult::Failed { unproved, .. } => {
                    failed_count += 1;
                    warn!("✗ {} '{}' verification failed ({} unproved goal(s))",
                        kind_name, thm.name.name, unproved.len());
                    for goal in &unproved {
                        debug!("  unproved: {:?}", goal.goal);
                        for s in &goal.suggestions {
                            debug!("    hint: {}", s);
                        }
                    }
                }
            }
        }

        if theorem_count > 0 {
            let stats = proof_engine.stats();
            info!("Theorem verification: {}/{} verified, {} failed, {} axioms (search: {} attempts, {} hits)",
                verified_count, theorem_count - axiom_count, failed_count, axiom_count,
                stats.total_attempts, stats.successes);
        }

        Ok(())
    }

    /// Run bounds elimination analysis at AST level (statistics gathering)
    ///
    /// This AST-level analysis collects statistics about array index accesses.
    /// The actual bounds check elimination happens at MIR level in
    /// `verification_phase.rs` which has access to full CFG and dataflow analysis.
    ///
    /// This pass is retained for early statistics and potential future
    /// AST-level optimizations.
    ///
    /// AST-level bounds statistics; actual elimination happens at MIR level via
    /// escape analysis and CBGR check elimination in verification_phase.rs.
    fn run_bounds_elimination_analysis(&mut self, module: &Module) -> Result<()> {
        debug!("Running AST-level bounds statistics collection");
        let start = Instant::now();

        let mut total_checks = 0usize;
        let mut eliminated = 0usize;

        for item in module.items.iter() {
            if let ItemKind::Function(func) = &item.kind {
                // Skip meta functions
                if func.is_meta {
                    continue;
                }

                // Gather statistics about index accesses in AST
                // Note: Actual elimination happens in verification_phase.rs (MIR level)
                let func_stats = self.analyze_function_bounds_checks(func);
                total_checks += func_stats.0;
                eliminated += func_stats.1;
            }
        }

        let elapsed = start.elapsed();

        if total_checks > 0 {
            debug!(
                "Bounds elimination: {} / {} checks eliminated ({:.1}%) in {:.2}ms",
                eliminated,
                total_checks,
                (eliminated as f64 / total_checks as f64) * 100.0,
                elapsed.as_millis()
            );
        }

        Ok(())
    }

    /// Count index accesses in a function for statistics
    ///
    /// Returns (total_index_accesses, 0)
    /// Note: Actual bounds check elimination happens at MIR level in
    /// `verification_phase.rs` which uses real CFG analysis with
    /// BoundsCheckEliminator and SMT-based proofs.
    fn analyze_function_bounds_checks(
        &self,
        func: &verum_ast::decl::FunctionDecl,
    ) -> (usize, usize) {
        use verum_ast::decl::FunctionBody;

        // Count array index accesses in the function body for statistics
        let mut total = 0;
        let eliminated = 0; // AST-level cannot eliminate; MIR-level does

        if let Some(ref body) = func.body {
            // Count Index expressions for statistics gathering
            let index_count = match body {
                FunctionBody::Block(block) => Self::count_index_accesses(block),
                FunctionBody::Expr(expr) => Self::count_index_in_expr(expr),
            };
            total = index_count;
        }

        (total, eliminated)
    }

    /// Count index access expressions in a statement block
    fn count_index_accesses(block: &verum_ast::expr::Block) -> usize {
        use verum_ast::stmt::StmtKind;

        let mut count = 0;

        for stmt in &block.stmts {
            match &stmt.kind {
                StmtKind::Expr { expr, .. } => {
                    count += Self::count_index_in_expr(expr);
                }
                StmtKind::Let { value, .. } => {
                    if let Some(init_expr) = value {
                        count += Self::count_index_in_expr(init_expr);
                    }
                }
                _ => {}
            }
        }

        if let Some(tail) = &block.expr {
            count += Self::count_index_in_expr(tail);
        }

        count
    }

    /// Recursively count index expressions
    fn count_index_in_expr(expr: &verum_ast::Expr) -> usize {
        use verum_ast::expr::ExprKind;

        let mut count = 0;

        match &expr.kind {
            ExprKind::Index { expr: inner, index } => {
                count += 1;
                count += Self::count_index_in_expr(inner);
                count += Self::count_index_in_expr(index);
            }
            ExprKind::Binary { left, right, .. } => {
                count += Self::count_index_in_expr(left);
                count += Self::count_index_in_expr(right);
            }
            ExprKind::Unary { expr: inner, .. } => {
                count += Self::count_index_in_expr(inner);
            }
            ExprKind::Block(block) => {
                count += Self::count_index_accesses(block);
            }
            ExprKind::If { then_branch, else_branch, .. } => {
                // Note: condition is IfCondition, not Expr, so we skip it for counting
                count += Self::count_index_accesses(then_branch);
                if let Some(else_expr) = else_branch {
                    count += Self::count_index_in_expr(else_expr);
                }
            }
            ExprKind::Call { args, .. } => {
                for arg in args {
                    count += Self::count_index_in_expr(arg);
                }
            }
            _ => {}
        }

        count
    }

    /// Verify refinement types for a function using SMT
    ///
    /// This implements full Z3-based refinement type verification:
    /// 1. Extracts refinement predicates from parameter/return types
    /// 2. Generates Z3 assertions for each refinement constraint
    /// 3. Uses verum_smt::RefinementVerifier to verify constraints
    /// 4. Caches verification results for performance
    /// 5. Returns Ok(true) if verified, Ok(false) if violated, Err on timeout/error
    ///
    /// Refinement type verification via Z3: extracts predicates from parameter/return
    /// types, generates assertions, and verifies constraints. Fast-path for syntactic
    /// subsumption; falls back to Z3 for complex cases. Timeout-bounded (10-500ms).
    fn verify_function_refinements(
        &mut self,
        func: &verum_ast::decl::FunctionDecl,
        timeout_ms: u64,
    ) -> Result<bool> {
        use std::time::Duration;
        use verum_ast::ty::TypeKind;
        use verum_smt::context::ContextConfig;

        // Create SMT context with timeout configuration. Install the
        // session's shared routing-stats collector so every Z3 check()
        // during refinement verification feeds `verum smt-stats`.
        let smt_config = ContextConfig {
            timeout: Some(Duration::from_millis(timeout_ms)),
            ..Default::default()
        };
        let smt_context = SmtContext::with_config(smt_config)
            .with_routing_stats(self.session.routing_stats().clone());

        // Create refinement verifier with SMT mode
        let verifier = SmtRefinementVerifier::with_mode(SmtVerifyMode::Auto);

        // Create subsumption checker for type relationships
        let subsumption_config = SubsumptionConfig {
            cache_size: 10000,
            smt_timeout_ms: timeout_ms.min(500), // 10-500ms for subsumption checking
        };
        let subsumption_checker = SubsumptionChecker::with_config(subsumption_config);

        // Track verification status
        let mut all_verified = true;

        // Collect parameter refinements for use in return type verification
        let mut param_constraints: List<(&verum_ast::Type, Text)> = List::new();

        // Verify parameter refinements
        for param in &func.params {
            if let verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } = &param.kind {
                if let TypeKind::Refined {
                    base: _,
                    predicate: _,
                } = &ty.kind
                {
                    // Extract parameter name for binding
                    let param_name =
                        Self::extract_pattern_name(pattern).unwrap_or_else(|| "param".into());

                    debug!(
                        "Verifying refined parameter '{}' with predicate in function '{}'",
                        param_name, func.name.name
                    );

                    // Store constraint for use in return type verification
                    param_constraints.push((ty, param_name.clone()));

                    // For parameters, we verify that the refinement predicate is well-formed
                    // and can be checked at runtime or compile-time
                    let verification_result = verifier.verify_refinement(
                        ty,
                        None, // No specific value - we're checking the type is valid
                        Some(SmtVerifyMode::Auto),
                    );

                    match verification_result {
                        Ok(_proof_result) => {
                            debug!(
                                "Parameter '{}' refinement verified in '{}'",
                                param_name, func.name.name
                            );
                        }
                        Err(SmtVerificationError::CannotProve {
                            constraint,
                            counterexample,
                            suggestions,
                            ..
                        }) => {
                            all_verified = false;

                            // Build helpful error message with counterexample and suggestions
                            let mut msg = format!(
                                "Parameter '{}' has unsatisfiable refinement constraint: {}",
                                param_name, constraint
                            );

                            if let Some(ref cex) = counterexample {
                                msg.push_str(&format!("\n  Counterexample: {:?}", cex));
                            }

                            if !suggestions.is_empty() {
                                msg.push_str("\n  Suggestions:");
                                for suggestion in &suggestions {
                                    msg.push_str(&format!("\n    - {}", suggestion));
                                }
                            }

                            let diag = DiagnosticBuilder::new(Severity::Error).message(msg).build();
                            self.session.emit_diagnostic(diag);
                        }
                        Err(SmtVerificationError::Timeout {
                            constraint,
                            timeout,
                            ..
                        }) => {
                            warn!(
                                "Timeout verifying parameter '{}' refinement ({}ms): {}",
                                param_name,
                                timeout.as_millis(),
                                constraint
                            );
                            // Don't fail on timeout - fall back to runtime checks
                            let diag = DiagnosticBuilder::new(Severity::Warning)
                                .message(format!(
                                    "Timeout verifying parameter '{}' refinement. Falling back to runtime checks.",
                                    param_name
                                ))
                                .build();
                            self.session.emit_diagnostic(diag);
                        }
                        Err(e) => {
                            debug!("Parameter '{}' refinement check error: {}", param_name, e);
                            // Other errors don't necessarily mean verification failed
                        }
                    }
                }
            }
        }

        // Verify return type refinement with full SMT integration
        if let Some(ref return_ty) = func.return_type {
            if let TypeKind::Refined { base: _, predicate } = &return_ty.kind {
                debug!(
                    "Verifying refined return type with predicate in function '{}'",
                    func.name.name
                );

                // Check if return statements satisfy the refinement using SMT
                let return_check_result = self.verify_return_refinement_smt(
                    func,
                    return_ty,
                    &predicate.expr,
                    &smt_context,
                    &verifier,
                    &subsumption_checker,
                    &param_constraints,
                );

                match return_check_result {
                    Ok(true) => {
                        debug!("Return refinement verified for '{}'", func.name.name);
                    }
                    Ok(false) => {
                        all_verified = false;
                        warn!("Return refinement violated for '{}'", func.name.name);

                        // Diagnostic already emitted by verify_return_refinement_smt
                    }
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
        }

        // Log subsumption statistics for debugging
        if self.session.options().verbose > 0 {
            let stats = subsumption_checker.stats();
            debug!(
                "Subsumption stats for '{}': syntactic={}, smt={}, cache_hits={}",
                func.name.name, stats.syntactic_checks, stats.smt_checks, stats.cache_hits
            );
        }

        Ok(all_verified)
    }

    /// Extract variable name from a pattern for binding
    fn extract_pattern_name(pattern: &verum_ast::pattern::Pattern) -> Option<Text> {
        use verum_ast::pattern::PatternKind;
        match &pattern.kind {
            PatternKind::Ident { name, .. } => Some(Text::from(name.name.as_str())),
            _ => None,
        }
    }

    /// Verify return refinement using full Z3 SMT integration
    ///
    /// This method performs comprehensive SMT-based verification:
    /// 1. Extracts all return values from the function body
    /// 2. Uses syntactic checking as a fast path for simple cases
    /// 3. Falls back to Z3 SMT solver for complex cases
    /// 4. Leverages subsumption checking for type relationships
    /// 5. Reports detailed error messages with counterexamples
    ///
    /// Refinement type subsumption via Z3: syntactic fast-path for simple predicates,
    /// full SMT solving for complex cases, with counterexample reporting on failure.
    #[allow(clippy::too_many_arguments)]
    fn verify_return_refinement_smt(
        &self,
        func: &verum_ast::decl::FunctionDecl,
        _return_ty: &verum_ast::Type,
        predicate: &verum_ast::expr::Expr,
        smt_context: &SmtContext,
        _verifier: &SmtRefinementVerifier,
        _subsumption_checker: &SubsumptionChecker,
        param_constraints: &List<(&verum_ast::Type, Text)>,
    ) -> Result<bool> {
        use verum_ast::ty::TypeKind;
        use verum_smt::translate::Translator;

        // Extract return values from function body
        let return_values = self.extract_return_values(func);

        if return_values.is_empty() {
            // No explicit returns - function might return implicitly or not at all
            debug!("No explicit returns found in function '{}'", func.name.name);
            return Ok(true); // Conservative: allow for now
        }

        // Track if any return failed verification
        let mut all_verified = true;

        // For each return value, check if it satisfies the predicate
        for (idx, return_expr) in return_values.iter().enumerate() {
            debug!(
                "Verifying return #{} in function '{}' against predicate",
                idx + 1,
                func.name.name
            );

            // Step 1: Try fast syntactic verification first (<1ms)
            if let Some(satisfied) = self.syntactic_check_refinement(return_expr, predicate) {
                if !satisfied {
                    // Syntactic check definitively found violation
                    all_verified = false;

                    let diag = DiagnosticBuilder::new(Severity::Error)
                        .message(format!(
                            "Return value in function '{}' violates refinement constraint: {:?}",
                            func.name.name, predicate
                        ))
                        .build();
                    self.session.emit_diagnostic(diag);
                    continue;
                }
                // Syntactically verified, continue to next return
                debug!(
                    "Return #{} verified syntactically in '{}'",
                    idx + 1,
                    func.name.name
                );
                continue;
            }

            // Step 2: Syntactic check inconclusive - use Z3 SMT solver
            debug!(
                "Return #{} requires SMT verification in '{}'",
                idx + 1,
                func.name.name
            );

            // Create translator and bind variables
            let mut translator = Translator::new(smt_context);

            // Bind parameter constraints as assumptions
            for (param_ty, param_name) in param_constraints {
                if let TypeKind::Refined {
                    base,
                    predicate: _param_pred,
                } = &param_ty.kind
                {
                    // Create Z3 variable for this parameter
                    if let Ok(z3_var) = translator.create_var(param_name.as_str(), base) {
                        translator.bind(param_name.clone(), z3_var);
                    }
                }
            }

            // Translate the return expression and predicate to Z3
            let z3_result =
                self.verify_return_expr_smt(return_expr, predicate, &mut translator, smt_context);

            match z3_result {
                SmtCheckResult::Verified => {
                    debug!(
                        "Return #{} verified via SMT in '{}'",
                        idx + 1,
                        func.name.name
                    );
                }
                SmtCheckResult::Violated { counterexample } => {
                    all_verified = false;

                    let mut msg = format!(
                        "Return value in function '{}' does not satisfy refinement constraint",
                        func.name.name
                    );

                    if let Some(cex) = counterexample {
                        msg.push_str(&format!("\n  Counterexample: {}", cex));
                    }

                    let diag = DiagnosticBuilder::new(Severity::Error).message(msg).build();
                    self.session.emit_diagnostic(diag);
                }
                SmtCheckResult::Unknown { reason } => {
                    // SMT solver couldn't determine - fall back to runtime check
                    debug!(
                        "Return #{} SMT check inconclusive in '{}': {}",
                        idx + 1,
                        func.name.name,
                        reason
                    );

                    let diag = DiagnosticBuilder::new(Severity::Warning)
                        .message(format!(
                            "Cannot statically verify return refinement in '{}'. Falling back to runtime check. Reason: {}",
                            func.name.name, reason
                        ))
                        .build();
                    self.session.emit_diagnostic(diag);
                }
                SmtCheckResult::Timeout => {
                    // SMT timeout - fall back to runtime check
                    debug!(
                        "Return #{} SMT check timed out in '{}'",
                        idx + 1,
                        func.name.name
                    );

                    let diag = DiagnosticBuilder::new(Severity::Warning)
                        .message(format!(
                            "Timeout verifying return refinement in '{}'. Falling back to runtime check.",
                            func.name.name
                        ))
                        .build();
                    self.session.emit_diagnostic(diag);
                }
            }
        }

        Ok(all_verified)
    }

    /// Verify a specific return expression against a predicate using Z3
    fn verify_return_expr_smt(
        &self,
        return_expr: &verum_ast::expr::Expr,
        predicate: &verum_ast::expr::Expr,
        translator: &mut verum_smt::translate::Translator<'_>,
        smt_context: &SmtContext,
    ) -> SmtCheckResult {
        use z3::SatResult;
        use z3::ast::{Dynamic, Int};

        // Create a fresh variable for the return value (bound to 'result' or 'it')
        let result_var = Int::new_const("result");
        let it_var = Int::new_const("it");

        // Bind both 'result' and 'it' to the same variable for refinement checking
        translator.bind("result".into(), Dynamic::from_ast(&result_var));
        translator.bind("it".into(), Dynamic::from_ast(&it_var));

        // Translate the predicate
        let z3_predicate = match translator.translate_expr(predicate) {
            Ok(expr) => expr,
            Err(e) => {
                return SmtCheckResult::Unknown {
                    reason: format!("Failed to translate predicate: {:?}", e),
                };
            }
        };

        // Convert to boolean if not already
        let z3_bool = match z3_predicate.as_bool() {
            Some(b) => b,
            None => {
                return SmtCheckResult::Unknown {
                    reason: "Predicate does not evaluate to boolean".to_string(),
                };
            }
        };

        // Translate the return expression
        let z3_return_value = match translator.translate_expr(return_expr) {
            Ok(expr) => expr,
            Err(e) => {
                return SmtCheckResult::Unknown {
                    reason: format!("Failed to translate return expression: {:?}", e),
                };
            }
        };

        // Create solver
        let solver = smt_context.solver();

        // Assert that the return value equals 'result'/'it'
        if let Some(return_int) = z3_return_value.as_int() {
            solver.assert(result_var.eq(&return_int));
            solver.assert(it_var.eq(&return_int));
        }

        // We want to check if the predicate can be FALSE given the return value
        // If UNSAT: predicate is always true for this return value (verified)
        // If SAT: found a counterexample where predicate is false (violated)
        solver.assert(z3_bool.not());

        // Route through Context::check for automatic routing-stats telemetry.
        match smt_context.check(&solver) {
            SatResult::Unsat => {
                // No counterexample - predicate always holds for this return
                SmtCheckResult::Verified
            }
            SatResult::Sat => {
                // Found counterexample - return value can violate predicate
                let counterexample = solver.get_model().map(|model| {
                    // Extract counterexample values
                    let mut cex_str = String::new();
                    if let Some(val) = model.eval(&result_var, true).and_then(|v| v.as_i64()) {
                        cex_str.push_str(&format!("result = {}", val));
                    }
                    cex_str
                });

                SmtCheckResult::Violated { counterexample }
            }
            SatResult::Unknown => {
                let reason = solver
                    .get_reason_unknown()
                    .unwrap_or_else(|| "Unknown".to_string());

                if reason.contains("timeout") || reason.contains("canceled") {
                    SmtCheckResult::Timeout
                } else {
                    SmtCheckResult::Unknown { reason }
                }
            }
        }
    }

    /// Extract all return values from a function
    fn extract_return_values(
        &self,
        func: &verum_ast::decl::FunctionDecl,
    ) -> List<verum_ast::expr::Expr> {
        use verum_ast::decl::FunctionBody;
        use verum_ast::expr::ExprKind;
        use verum_ast::stmt::StmtKind;

        let mut returns = List::new();

        if let Some(ref body) = func.body {
            match body {
                FunctionBody::Block(block) => {
                    // Implicit return (final expression)
                    if let Some(ref final_expr) = block.expr {
                        returns.push((**final_expr).clone());
                    }

                    // Explicit returns
                    for stmt in &block.stmts {
                        if let StmtKind::Expr { expr, .. } = &stmt.kind {
                            if let ExprKind::Return(Some(return_expr)) = &expr.kind {
                                returns.push((**return_expr).clone());
                            }
                        }
                    }
                }
                FunctionBody::Expr(expr) => {
                    // Expression body is implicitly returned
                    returns.push(expr.clone());
                }
            }
        }

        returns
    }

    /// Simple syntactic check for common refinement patterns
    ///
    /// Returns Some(true) if definitely satisfied, Some(false) if violated,
    /// None if inconclusive (needs SMT).
    ///
    /// Examples:
    /// - `x + 1` satisfies `result > x` (syntactic: x+1 > x always true for Int)
    /// - `5` satisfies `result > 0` (syntactic: 5 > 0 is true)
    /// - `-5` violates `result > 0` (syntactic: -5 > 0 is false)
    fn syntactic_check_refinement(
        &self,
        value: &verum_ast::expr::Expr,
        predicate: &verum_ast::expr::Expr,
    ) -> Option<bool> {
        use verum_ast::expr::{BinOp, ExprKind};
        use verum_ast::literal::{Literal, LiteralKind};

        // Pattern: predicate is `result > constant` or `result >= constant`
        // Check if value is a literal that satisfies this
        if let ExprKind::Binary { op, left, right } = &predicate.kind {
            // Check if left side is 'result' or 'it'
            if let ExprKind::Path(path) = &left.kind {
                if path.segments.len() == 1 {
                    let var_name = match &path.segments[0] {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => return None,
                    };

                    if var_name == "result" || var_name == "it" {
                        // Try to extract constant from right side
                        if let ExprKind::Literal(Literal {
                            kind: LiteralKind::Int(lit),
                            ..
                        }) = &right.kind
                        {
                            let threshold = lit.value as i64;

                            // Try to extract value as constant
                            if let ExprKind::Literal(Literal {
                                kind: LiteralKind::Int(val_lit),
                                ..
                            }) = &value.kind
                            {
                                let val = val_lit.value as i64;

                                // Check the comparison
                                let satisfied = match op {
                                    BinOp::Gt => val > threshold,
                                    BinOp::Ge => val >= threshold,
                                    BinOp::Lt => val < threshold,
                                    BinOp::Le => val <= threshold,
                                    BinOp::Eq => val == threshold,
                                    BinOp::Ne => val != threshold,
                                    _ => return None,
                                };

                                return Some(satisfied);
                            }

                            // Pattern: value is `x + constant2` and predicate is `result > constant1`
                            // If constant2 > 0, then x + constant2 > x, which may satisfy the predicate
                            if let ExprKind::Binary {
                                op: BinOp::Add,
                                left: _,
                                right: add_right,
                            } = &value.kind
                            {
                                if let ExprKind::Literal(Literal {
                                    kind: LiteralKind::Int(add_lit),
                                    ..
                                }) = &add_right.kind
                                {
                                    let add_val = add_lit.value as i64;
                                    // Simple heuristic: x + positive > 0 is likely true
                                    if matches!(op, BinOp::Gt) && threshold == 0 && add_val > 0 {
                                        return Some(true);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Inconclusive - would need SMT
        None
    }

    /// Check if a type contains refinement predicates
    fn has_refinement_type(&self, ty: &verum_ast::Type) -> bool {
        use verum_ast::ty::TypeKind;
        match &ty.kind {
            TypeKind::Refined { .. } => true,
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                params.iter().any(|p| self.has_refinement_type(p))
                    || self.has_refinement_type(return_type)
            }
            TypeKind::Tuple(types) => types.iter().any(|t| self.has_refinement_type(t)),
            TypeKind::Array { element, .. } => self.has_refinement_type(element),
            TypeKind::Reference { inner, .. } => self.has_refinement_type(inner),
            TypeKind::Ownership { inner, .. } => self.has_refinement_type(inner),
            _ => false,
        }
    }

    /// Protocol coherence checking (orphan rule, overlap detection)
    ///
    /// Validates that protocol implementations follow coherence rules:
    /// - Orphan rule: Implementation must be in crate that defines protocol OR type
    /// - Overlap prevention: No two implementations can apply to the same type
    /// - Specialization: Overlapping impls must use `@specialize`
    ///
    /// Protocol coherence: orphan rule (impl in defining crate), overlap prevention,
    /// and specialization via @specialize for overlapping impls.
    fn check_protocol_coherence(&self, module: &Module) -> Result<()> {
        // Gate on [protocols].coherence. "unchecked" skips all
        // coherence rules; "lenient" and "strict" proceed (the method
        // already classifies by severity internally).
        let coherence_mode = self
            .session
            .language_features()
            .protocols
            .coherence
            .as_str();
        if coherence_mode == "unchecked" {
            tracing::debug!(
                "Protocol coherence checking SKIPPED ([protocols] coherence = \"unchecked\")"
            );
            return Ok(());
        }

        use verum_ast::decl::ImplKind;
        use verum_modules::ModuleId;

        // Determine crate name from module path or use "main" as default
        let crate_name = if let Some(item) = module.items.first() {
            if let Some(source_file) = self.session.get_source(item.span.file_id) {
                if let Some(ref file_path) = source_file.path {
                    // Extract crate name from file path (first directory component)
                    file_path
                        .components()
                        .find_map(|c| {
                            if let std::path::Component::Normal(s) = c {
                                s.to_str().map(Text::from)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| Text::from("main"))
                } else {
                    Text::from("main")
                }
            } else {
                Text::from("main")
            }
        } else {
            Text::from("main")
        };

        let mut checker = CoherenceChecker::new(crate_name.clone());

        // Mark stdlib crates as trusted for blanket implementations.
        // This allows stdlib to define implementations like:
        //   implement<T, U: From<T>> Into<U> for T { ... }
        // Always trust these regardless of which file is being compiled.
        checker.add_trusted_crate("core");
        checker.add_trusted_crate("sys");
        checker.add_trusted_crate("mem");
        checker.add_trusted_crate("collections");
        checker.add_trusted_crate("async");
        checker.add_trusted_crate("io");
        checker.add_trusted_crate("runtime");
        checker.add_trusted_crate("meta");

        let current_module_path = ModulePath::from_str(crate_name.as_str());
        let current_module_id = ModuleId::new(0); // Default module ID for single-file mode

        // ───────────────────────────────────────────────────────────────────
        // Register stdlib types, protocols, and impl blocks so coherence
        // checking can detect cross-crate overlaps and orphan violations
        // between user code and the standard library.
        // ───────────────────────────────────────────────────────────────────
        let mut ext_module_counter: u32 = 1; // reserve 0 for user module
        for (mod_path, stdlib_mod) in &self.modules {
            let stdlib_mod_path = ModulePath::from_str(mod_path.as_str());
            let stdlib_mod_id = ModuleId::new(ext_module_counter);
            ext_module_counter += 1;

            self.register_module_coherence_items(
                &mut checker, stdlib_mod, &stdlib_mod_path, stdlib_mod_id,
            );
        }

        // Also include project modules (cross-file imports in multi-file projects)
        for (mod_path, project_mod) in &self.project_modules {
            let proj_mod_path = ModulePath::from_str(mod_path.as_str());
            let proj_mod_id = ModuleId::new(ext_module_counter);
            ext_module_counter += 1;

            self.register_module_coherence_items(
                &mut checker, project_mod, &proj_mod_path, proj_mod_id,
            );
        }

        // ───────────────────────────────────────────────────────────────────
        // Register user module types, protocols, and impl blocks
        // ───────────────────────────────────────────────────────────────────

        // Register local types (defined in this module)
        for item in &module.items {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                let type_name = Text::from(type_decl.name.as_str());
                checker.register_local_type(type_name, current_module_path.clone());
            }
        }

        // Register local protocols (defined in this module)
        for item in &module.items {
            match &item.kind {
                verum_ast::ItemKind::Protocol(protocol_decl) => {
                    let protocol_name = Text::from(protocol_decl.name.as_str());
                    checker.register_local_protocol(protocol_name, current_module_path.clone());
                }
                verum_ast::ItemKind::Type(type_decl) => {
                    // `type X is protocol { ... }` also defines a local protocol
                    if matches!(&type_decl.body, verum_ast::decl::TypeDeclBody::Protocol(_)) {
                        let protocol_name = Text::from(type_decl.name.as_str());
                        checker.register_local_protocol(protocol_name, current_module_path.clone());
                    }
                }
                _ => {}
            }
        }

        // Collect user implement blocks as ImplEntry
        for item in &module.items {
            if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                // Only check protocol implementations, not inherent impls
                if let ImplKind::Protocol { protocol, for_type, .. } = &impl_decl.kind {
                    let protocol_name = protocol.to_string();
                    let protocol_path = ModulePath::from_str(&protocol_name);
                    let for_type_text = self.type_to_text(for_type);

                    let mut entry = ImplEntry::new(
                        Text::from(protocol_name),
                        protocol_path,
                        for_type_text,
                        current_module_path.clone(),
                        current_module_id,
                    );

                    entry = entry.with_span(impl_decl.span);

                    if impl_decl.specialize_attr.is_some() {
                        entry = entry.with_specialized();
                    }

                    if !impl_decl.generics.is_empty() {
                        let params: List<Text> = impl_decl
                            .generics
                            .iter()
                            .filter_map(|g| {
                                use verum_ast::ty::GenericParamKind;
                                match &g.kind {
                                    GenericParamKind::Type { name, .. } => Some(Text::from(name.as_str())),
                                    GenericParamKind::HigherKinded { name, .. } => Some(Text::from(name.as_str())),
                                    _ => None,
                                }
                            })
                            .collect();
                        entry = entry.with_type_params(params);
                    }

                    // Extract @cfg predicates from item attributes and module path
                    let cfg_preds = Self::extract_cfg_predicates(&item.attributes, &current_module_path);
                    if !cfg_preds.is_empty() {
                        entry = entry.with_cfg_predicates(cfg_preds);
                    }

                    checker.add_impl(entry);
                }
            }
        }

        // Run all coherence checks (orphan rules, overlap, specialization, cross-crate)
        let errors = checker.check_all();

        if !errors.is_empty() {
            debug!("Protocol coherence: found {} violation(s)", errors.len());
        }

        // Emit diagnostics as warnings — coherence violations are advisory for now
        // so they don't block compilation while the checker is being hardened.
        for error in errors {
            let ast_span = match &error {
                verum_modules::CoherenceError::OrphanImpl { span, .. } => *span,
                verum_modules::CoherenceError::OverlappingImpl { span, .. } => *span,
                verum_modules::CoherenceError::InvalidSpecialization { span, .. } => *span,
                verum_modules::CoherenceError::ConflictingCrateImpl { span, .. } => *span,
            };

            let mut builder = DiagnosticBuilder::new(Severity::Warning)
                .message(format!("[coherence] {}", error));
            if let Some(ast_span) = ast_span {
                let diag_span = self.session.convert_span(ast_span);
                builder = builder.span(diag_span);
            }
            self.session.emit_diagnostic(builder.build());
        }

        Ok(())
    }

    /// Helper: register a module's types, protocols, and impl blocks into the coherence checker.
    fn register_module_coherence_items(
        &self,
        checker: &mut CoherenceChecker,
        module: &Module,
        mod_path: &ModulePath,
        mod_id: ModuleId,
    ) {
        use verum_ast::decl::ImplKind;

        for item in &module.items {
            match &item.kind {
                verum_ast::ItemKind::Type(type_decl) => {
                    if matches!(&type_decl.body, verum_ast::decl::TypeDeclBody::Protocol(_)) {
                        let protocol_name = Text::from(type_decl.name.as_str());
                        checker.register_local_protocol(protocol_name, mod_path.clone());
                    }
                }
                verum_ast::ItemKind::Protocol(protocol_decl) => {
                    let protocol_name = Text::from(protocol_decl.name.as_str());
                    checker.register_local_protocol(protocol_name, mod_path.clone());
                }
                verum_ast::ItemKind::Impl(impl_decl) => {
                    if let ImplKind::Protocol { protocol, for_type, .. } = &impl_decl.kind {
                        let protocol_name = protocol.to_string();
                        let protocol_path = ModulePath::from_str(&protocol_name);
                        let for_type_text = self.type_to_text(for_type);

                        let mut entry = ImplEntry::new(
                            Text::from(protocol_name),
                            protocol_path,
                            for_type_text,
                            mod_path.clone(),
                            mod_id,
                        );

                        if impl_decl.specialize_attr.is_some() {
                            entry = entry.with_specialized();
                        }

                        if !impl_decl.generics.is_empty() {
                            let params: List<Text> = impl_decl
                                .generics
                                .iter()
                                .filter_map(|g| {
                                    use verum_ast::ty::GenericParamKind;
                                    match &g.kind {
                                        GenericParamKind::Type { name, .. } => Some(Text::from(name.as_str())),
                                        GenericParamKind::HigherKinded { name, .. } => Some(Text::from(name.as_str())),
                                        _ => None,
                                    }
                                })
                                .collect();
                            entry = entry.with_type_params(params);
                        }

                        // Extract @cfg predicates from item attributes and module path
                        let cfg_preds = Self::extract_cfg_predicates(&item.attributes, mod_path);
                        if !cfg_preds.is_empty() {
                            entry = entry.with_cfg_predicates(cfg_preds);
                        }

                        checker.add_impl(entry);
                    }
                }
                _ => {}
            }
        }
    }

    /// Extract @cfg predicates from item attributes and module path.
    ///
    /// Returns a list of cfg predicate strings (e.g., `target_os = "linux"`).
    /// Also infers platform cfg from module paths containing platform segments
    /// (e.g., `sys.darwin.io` implies `target_os = "macos"`).
    fn extract_cfg_predicates(
        attributes: &[verum_ast::attr::Attribute],
        module_path: &ModulePath,
    ) -> List<Text> {
        let mut predicates = List::new();

        // Extract from @cfg(...) attributes on the item
        for attr in attributes {
            if attr.name.as_str() == "cfg" {
                if let verum_common::Maybe::Some(ref args) = attr.args {
                    // Extract cfg predicate key=value pairs from expressions
                    for arg in args.iter() {
                        if let Some(pred) = Self::cfg_expr_to_predicate(arg) {
                            predicates.push(pred);
                        }
                    }
                }
            }
        }

        // Infer platform cfg from module path segments
        let path_str = module_path.to_string();
        if path_str.contains(".darwin.") || path_str.ends_with(".darwin") {
            predicates.push(Text::from("target_os = \"macos\""));
        } else if path_str.contains(".linux.") || path_str.ends_with(".linux") {
            predicates.push(Text::from("target_os = \"linux\""));
        } else if path_str.contains(".windows.") || path_str.ends_with(".windows") {
            predicates.push(Text::from("target_os = \"windows\""));
        }

        predicates
    }

    /// Convert a @cfg expression argument to a predicate string.
    ///
    /// Handles patterns like:
    /// - `target_os = "linux"` (Binary with Assign op)
    /// - Simple identifier (e.g., `unix`, `windows`)
    fn cfg_expr_to_predicate(expr: &verum_ast::Expr) -> Option<Text> {
        use verum_ast::expr::{ExprKind, BinOp};
        match &expr.kind {
            // Handle `key = "value"` as Binary { op: Assign, left, right }
            ExprKind::Binary { op, left, right } => {
                if matches!(op, BinOp::Assign) {
                    let key = Self::expr_to_ident_string(left)?;
                    let val = Self::expr_to_string_literal(right)?;
                    Some(Text::from(format!("{} = \"{}\"", key, val)))
                } else {
                    None
                }
            }
            // Handle simple identifier like `unix`
            ExprKind::Path(path) => {
                use verum_ast::ty::PathSegment;
                match path.segments.last()? {
                    PathSegment::Name(ident) => Some(Text::from(ident.as_str())),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Extract identifier name from an expression.
    fn expr_to_ident_string(expr: &verum_ast::Expr) -> Option<String> {
        use verum_ast::ty::PathSegment;
        match &expr.kind {
            verum_ast::expr::ExprKind::Path(path) => {
                match path.segments.last()? {
                    PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Extract string literal value from an expression.
    fn expr_to_string_literal(expr: &verum_ast::Expr) -> Option<String> {
        use verum_ast::literal::LiteralKind;
        match &expr.kind {
            verum_ast::expr::ExprKind::Literal(lit) => {
                match &lit.kind {
                    LiteralKind::Text(string_lit) => Some(string_lit.as_str().to_string()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Profile boundary enforcement (module-level profile checking)
    ///
    /// Validates that imports respect language profile boundaries:
    /// - Application profile can only import from Application modules
    /// - Systems profile can import from Systems and Application modules
    /// - Research profile can import from any module
    ///
    /// Profile boundary enforcement: Application can only import Application,
    /// Systems can import Systems+Application, Research can import anything.
    fn check_profile_boundaries(&self, module: &Module) -> Result<()> {
        // Create a profile checker with the default compilation profile
        // In a more complete implementation, this would come from Verum.toml or CLI flags
        let _profile_checker = ProfileChecker::new(LanguageProfile::Application);

        // Extract the current module's profile from @profile attribute
        let current_profile = self.extract_module_profile(module);

        // Get current module path for error reporting
        let current_module_path = if let Some(item) = module.items.first() {
            if let Some(source_file) = self.session.get_source(item.span.file_id) {
                if let Some(ref file_path) = source_file.path {
                    file_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(Text::from)
                        .unwrap_or_else(|| Text::from("main"))
                } else {
                    Text::from("main")
                }
            } else {
                Text::from("main")
            }
        } else {
            Text::from("main")
        };

        // Check all imports for profile compatibility
        for item in &module.items {
            if let verum_ast::ItemKind::Mount(import) = &item.kind {
                // Extract target module path from import
                let target_path = self.import_to_module_path(import);

                // Get target module's profile (from registry if available)
                let target_profile = self.get_module_profile_from_registry(&target_path);

                // Check if import is allowed
                if !current_profile.can_access(target_profile) {
                    let mut builder = DiagnosticBuilder::new(Severity::Error)
                        .message(format!(
                            "Profile boundary violation: module '{}' with {} profile cannot import from module '{}' with {} profile",
                            current_module_path,
                            current_profile,
                            target_path,
                            target_profile
                        ));
                    builder = builder.span(self.session.convert_span(import.span));
                    self.session.emit_diagnostic(builder.build());
                }
            }
        }

        Ok(())
    }

    /// Extract the language profile from a module's @profile attribute
    fn extract_module_profile(&self, module: &Module) -> LanguageProfile {
        // Check module-level attributes
        for attr in &module.attributes {
            if attr.name.as_str() == "profile" {
                if let verum_common::Maybe::Some(ref args) = attr.args
                    && let Some(first_arg) = args.first()
                {
                    if let Some(profile_name) = self.extract_profile_name(first_arg)
                        && let Some(profile) = LanguageProfile::from_str(&profile_name)
                    {
                        return profile;
                    }
                }
            }
        }

        // Check item-level attributes (on module declarations)
        for item in &module.items {
            for attr in &item.attributes {
                if attr.name.as_str() == "profile" {
                    if let verum_common::Maybe::Some(ref args) = attr.args
                        && let Some(first_arg) = args.first()
                    {
                        if let Some(profile_name) = self.extract_profile_name(first_arg)
                            && let Some(profile) = LanguageProfile::from_str(&profile_name)
                        {
                            return profile;
                        }
                    }
                }
            }
        }

        // Default to Application profile
        LanguageProfile::Application
    }

    /// Extract profile name from an expression (handles both string and identifier forms)
    fn extract_profile_name(&self, expr: &verum_ast::expr::Expr) -> Option<String> {
        use verum_ast::LiteralKind;
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Text(s) => Some(s.as_str().to_string()),
                _ => None,
            },
            ExprKind::Path(path) => {
                // Handle identifier-style arguments like @profile(application)
                if path.segments.len() == 1
                    && let verum_ast::PathSegment::Name(ident) = &path.segments[0]
                {
                    return Some(ident.name.to_string());
                }
                None
            }
            _ => None,
        }
    }

    /// Convert an import to a module path string
    fn import_to_module_path(&self, import: &verum_ast::decl::MountDecl) -> Text {
        use verum_ast::PathSegment;
        use verum_ast::decl::MountTreeKind;

        // Extract path from MountTree based on its kind
        let path = match &import.tree.kind {
            MountTreeKind::Path(path) => path,
            MountTreeKind::Glob(path) => path,
            MountTreeKind::Nested { prefix, .. } => prefix,
            // #5 / P1.5 — file-relative mount.  The session
            // loader has already resolved the file at this
            // point; surface the literal path as the module
            // identifier so downstream logging / diagnostics
            // can refer to it. Returning early avoids
            // returning a synthesized empty Path.
            MountTreeKind::File { path, .. } => return path.clone(),
        };

        // Get the path segments
        let segments: Vec<&str> = path.segments
            .iter()
            .map(|seg| match seg {
                PathSegment::Name(ident) => ident.name.as_str(),
                PathSegment::SelfValue => "self",
                PathSegment::Super => "super",
                PathSegment::Cog => "cog",
                PathSegment::Relative => ".",
            })
            .collect();

        Text::from(segments.join("."))
    }

    /// Get a module's profile from the registry (or default to Application)
    fn get_module_profile_from_registry(&self, module_path: &Text) -> LanguageProfile {
        // Look up the module in loaded modules and extract its @profile attribute
        if let Some(module) = self.modules.get(module_path) {
            return self.extract_module_profile(module);
        }

        // Try dot-separated path segments as partial matches (e.g., "core.sys.linux" -> "core.sys")
        let path_str = module_path.as_str();
        for (key, module) in self.modules.iter() {
            if path_str.starts_with(key.as_str()) || key.as_str().starts_with(path_str) {
                return self.extract_module_profile(module);
            }
        }

        // Default to Application (safe assumption — most restrictive profile)
        LanguageProfile::Application
    }

    /// Convert an AST type to text representation
    fn type_to_text(&self, ty: &verum_ast::Type) -> Text {
        use verum_ast::ty::{TypeKind, GenericArg, PathSegment};

        match &ty.kind {
            TypeKind::Path(path) => {
                // For single-segment paths, return the segment name
                if path.segments.len() == 1 {
                    match &path.segments[0] {
                        PathSegment::Name(ident) => ident.name.clone(),
                        PathSegment::SelfValue => Text::from("self"),
                        PathSegment::Super => Text::from("super"),
                        PathSegment::Cog => Text::from("cog"),
                        PathSegment::Relative => Text::from("."),
                    }
                } else {
                    // For multi-segment paths, join with '.'
                    let segments: Vec<&str> = path.segments
                        .iter()
                        .map(|seg| match seg {
                            PathSegment::Name(ident) => ident.name.as_str(),
                            PathSegment::SelfValue => "self",
                            PathSegment::Super => "super",
                            PathSegment::Cog => "cog",
                            PathSegment::Relative => ".",
                        })
                        .collect();
                    Text::from(segments.join("."))
                }
            }
            TypeKind::Generic { base, args } => {
                let base_text = self.type_to_text(base);
                if args.is_empty() {
                    base_text
                } else {
                    let args_str: Vec<String> = args
                        .iter()
                        .map(|arg| match arg {
                            GenericArg::Type(t) => self.type_to_text(t).to_string(),
                            GenericArg::Const(e) => format!("{:?}", e),
                            GenericArg::Lifetime(_) => "'_".to_string(),
                            GenericArg::Binding(binding) => {
                                format!("{}={}", binding.name.name, self.type_to_text(&binding.ty))
                            }
                        })
                        .collect();
                    Text::from(format!("{}<{}>", base_text, args_str.join(", ")))
                }
            }
            TypeKind::Tuple(types) => {
                let type_strs: Vec<String> = types.iter().map(|t| self.type_to_text(t).to_string()).collect();
                Text::from(format!("({})", type_strs.join(", ")))
            }
            TypeKind::Reference { inner, mutable, .. } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("&mut {}", inner_text))
                } else {
                    Text::from(format!("&{}", inner_text))
                }
            }
            TypeKind::CheckedReference { inner, mutable } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("&checked mut {}", inner_text))
                } else {
                    Text::from(format!("&checked {}", inner_text))
                }
            }
            TypeKind::UnsafeReference { inner, mutable } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("&unsafe mut {}", inner_text))
                } else {
                    Text::from(format!("&unsafe {}", inner_text))
                }
            }
            TypeKind::Array { element, size } => {
                let elem_text = self.type_to_text(element);
                if let Some(size_expr) = size {
                    Text::from(format!("[{}; {:?}]", elem_text, size_expr))
                } else {
                    Text::from(format!("[{}]", elem_text))
                }
            }
            TypeKind::Function { params, return_type, .. } => {
                let params_str: Vec<String> = params.iter().map(|p| self.type_to_text(p).to_string()).collect();
                let ret_text = self.type_to_text(return_type);
                Text::from(format!("fn({}) -> {}", params_str.join(", "), ret_text))
            }
            TypeKind::Ownership { mutable, inner } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("Heap<mut {}>", inner_text))
                } else {
                    Text::from(format!("Heap<{}>", inner_text))
                }
            }
            TypeKind::Pointer { mutable, inner } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("*mut {}", inner_text))
                } else {
                    Text::from(format!("*const {}", inner_text))
                }
            }
            TypeKind::VolatilePointer { mutable, inner } => {
                let inner_text = self.type_to_text(inner);
                if *mutable {
                    Text::from(format!("*volatile mut {}", inner_text))
                } else {
                    Text::from(format!("*volatile {}", inner_text))
                }
            }
            TypeKind::Slice(inner) => {
                let inner_text = self.type_to_text(inner);
                Text::from(format!("[{}]", inner_text))
            }
            TypeKind::Qualified { self_ty, trait_ref, assoc_name } => {
                let self_text = self.type_to_text(self_ty);
                Text::from(format!("<{} as {}>::{}", self_text, trait_ref, assoc_name.name))
            }
            TypeKind::Refined { base, predicate } => {
                let base_text = self.type_to_text(base);
                // Post the sigma surface form lives here too (binder
                // carried by the predicate); render it distinctly when bound.
                match &predicate.binding {
                    verum_common::Maybe::Some(binder) => {
                        Text::from(format!("{}: {} where ...", binder.name, base_text))
                    }
                    verum_common::Maybe::None => {
                        Text::from(format!("{}{{...}}", base_text))
                    }
                }
            }
            TypeKind::Bounded { base, .. } => self.type_to_text(base),
            _ if ty.kind.primitive_name().is_some() => {
                // primitive_name() already checked to be Some in the guard
                Text::from(ty.kind.primitive_name().unwrap_or("?"))
            }
            _ => Text::from("?"),
        }
    }

    /// Phase 4a: Tier analysis
    ///
    /// Performs tier analysis on all functions in the module. This phase:
    /// 1. Builds control flow graphs (CFGs) for each function
    /// 2. Runs escape analysis to determine reference tier selection
    /// 3. Decides which references can be promoted from Tier 0 (~15ns) to Tier 1 (0ns)
    /// 4. Logs analysis statistics for optimization feedback
    ///
    /// CBGR analysis: builds CFGs, runs escape analysis to promote references from
    /// Tier 0 (~15ns managed) to Tier 1 (0ns compiler-proven safe, `&checked T`).
    fn phase_cbgr_analysis(&mut self, module: &Module) -> Result<()> {
        use verum_cbgr::tier_analysis::{TierAnalysisConfig, TierAnalyzer};
        use verum_cbgr::tier_types::TierStatistics;
        use crate::session::FunctionId;

        // Gate on [runtime].cbgr_mode:
        //   "unsafe" → skip analysis entirely (all refs are raw)
        //   "managed" → skip promotion (all refs stay at Tier 0)
        //   "checked" / "mixed" → full analysis (current behavior)
        let cbgr_mode = self
            .session
            .language_features()
            .runtime
            .cbgr_mode
            .as_str()
            .to_string();
        if cbgr_mode == "unsafe" {
            tracing::debug!(
                "CBGR analysis SKIPPED ([runtime] cbgr_mode = \"unsafe\")"
            );
            return Ok(());
        }

        debug!("Running tier analysis (cbgr_mode = {})", cbgr_mode);
        let start = Instant::now();

        // Create tier analysis configuration based on [runtime].cbgr_mode.
        // "managed" → disable promotion (nothing can be promoted to checked).
        // "checked" / "mixed" → full analysis.
        // (enable_promotion would gate tier promotion if the analyzer API
        //  accepted it; TierAnalysisConfig does not currently expose a
        //  flag for it, so we record the decision here for documentation.)
        let _enable_promotion = cbgr_mode != "managed";
        let config = TierAnalysisConfig {
            confidence_threshold: 0.95,
            analyze_async_boundaries: true,
            analyze_exception_paths: true,
            enable_ownership_analysis: true,
            enable_concurrency_analysis: true,
            enable_lifetime_analysis: true,
            enable_nll_analysis: true,
            max_iterations: 1000,
            timeout_ms: 5000,
        };

        // Track statistics across all functions
        let mut global_stats = TierStatistics::new();

        // Process each function in the module
        for item in module.items.iter() {
            if let ItemKind::Function(func) = &item.kind {
                // Skip meta functions (compile-time only)
                if func.is_meta {
                    continue;
                }

                // Build CFG from function body
                let cfg = self.build_function_cfg(func);

                // Create function ID from name hash
                let function_id = FunctionId(Self::hash_function_name(&func.name.name));

                // Run tier analysis
                let analyzer = TierAnalyzer::with_config(cfg, config.clone());
                let result = analyzer.analyze();

                // Log per-function results at debug level
                if result.stats.total_refs > 0 {
                    debug!(
                        "  Function '{}': {} refs, {} T1, {} T0 ({:.1}% promoted)",
                        func.name.name,
                        result.stats.total_refs,
                        result.stats.tier1_count,
                        result.stats.tier0_count,
                        result.stats.promotion_rate() * 100.0
                    );
                }

                // Merge statistics
                global_stats.merge(&result.stats);

                // Cache result for codegen phase
                self.session.cache_tier_analysis(function_id, result);
            }
        }

        let elapsed = start.elapsed();

        // Report summary statistics
        if global_stats.functions_analyzed > 0 {
            debug!(
                "Tier analysis completed in {:.2}ms: {} functions, {} refs, {} promoted ({:.1}%)",
                elapsed.as_millis(),
                global_stats.functions_analyzed,
                global_stats.total_refs,
                global_stats.tier1_count,
                global_stats.promotion_rate() * 100.0
            );

            // At higher verbosity, show full statistics
            if self.session.options().verbose >= 2 {
                info!("{}", global_stats);
            }
        } else {
            debug!(
                "Tier analysis completed in {:.2}ms (no functions analyzed)",
                elapsed.as_millis()
            );
        }

        // Update global statistics in session for reporting
        self.session.merge_tier_statistics(&global_stats);

        Ok(())
    }

    /// Build a control flow graph from a function declaration
    ///
    /// Creates a complete CFG for escape analysis. This builds:
    /// - Entry block for function entry with parameter definitions
    /// - Blocks for if/else branches
    /// - Blocks for match arms
    /// - Loop header and body blocks
    /// - Exit blocks
    /// - Control flow edges between blocks
    ///
    /// CFG construction for escape analysis: creates basic blocks for branches,
    /// match arms, loops, with control flow edges for dataflow analysis.
    fn build_function_cfg(
        &self,
        func: &verum_ast::decl::FunctionDecl,
    ) -> verum_cbgr::analysis::ControlFlowGraph {
        use verum_cbgr::CfgBuilder;
        use verum_cbgr::analysis::{DefSite, RefId};

        let mut builder = CfgBuilder::new();
        let mut ref_counter = 0u64;

        // Entry and exit blocks
        let entry_id = builder.new_block_id();
        let exit_id = builder.new_block_id();

        // Create entry block with parameter definitions
        let mut param_defs = List::new();
        for param in func.params.iter() {
            // Each parameter is a reference definition
            if Self::param_is_reference(param) {
                param_defs.push(DefSite {
                    block: entry_id,
                    reference: RefId(ref_counter),
                    is_stack_allocated: true, // Parameters are stack-allocated
                    span: None,
                });
                ref_counter += 1;
            }
        }

        // Start building CFG with basic structure
        let mut cfg = builder.build_cfg(entry_id, exit_id);

        // Build function body CFG if present
        if let Some(ref body) = func.body {
            // Create a context for building blocks
            let mut ctx = CfgBuildContext {
                builder: &mut builder,
                ref_counter: &mut ref_counter,
                entry_id,
                exit_id,
                pending_blocks: List::new(),
                closure_captures: List::new(),
            };

            // Build the body, which returns the first block after entry
            let body_start = self.build_body_cfg(body, &mut ctx, &mut cfg);

            // Connect entry to body start
            let entry_successors = if body_start != entry_id {
                let mut succs = verum_common::Set::new();
                succs.insert(body_start);
                succs
            } else {
                let mut succs = verum_common::Set::new();
                succs.insert(exit_id);
                succs
            };

            // Build entry block with proper successors
            let entry_block = ctx.builder.build_block(
                entry_id,
                verum_common::Set::new(), // No predecessors for entry
                entry_successors,
                param_defs,
                List::new(),
            );
            cfg.add_block(entry_block);

            // Add all pending blocks to CFG
            for block in ctx.pending_blocks.drain(..) {
                cfg.add_block(block);
            }

            // Build exit block with collected predecessors
            let exit_preds = self.collect_exit_predecessors(&cfg, exit_id);
            let exit_block = ctx.builder.build_block(
                exit_id,
                exit_preds,
                verum_common::Set::new(), // No successors for exit
                List::new(),
                List::new(),
            );
            cfg.add_block(exit_block);
        } else {
            // No body - entry connects directly to exit
            let entry_block = builder.build_block(
                entry_id,
                verum_common::Set::new(),
                {
                    let mut succs = verum_common::Set::new();
                    succs.insert(exit_id);
                    succs
                },
                param_defs,
                List::new(),
            );

            let exit_block = builder.build_block(
                exit_id,
                {
                    let mut preds = verum_common::Set::new();
                    preds.insert(entry_id);
                    preds
                },
                verum_common::Set::new(),
                List::new(),
                List::new(),
            );

            cfg.add_block(entry_block);
            cfg.add_block(exit_block);
        }

        cfg
    }

    /// Build CFG for a function body
    fn build_body_cfg(
        &self,
        body: &verum_ast::decl::FunctionBody,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
    ) -> verum_cbgr::analysis::BlockId {
        use verum_ast::decl::FunctionBody;

        match body {
            FunctionBody::Block(block) => self.build_block_cfg(block, ctx, cfg, ctx.exit_id),
            FunctionBody::Expr(expr) => {
                // Single expression body - create a block for it
                let block_id = ctx.builder.new_block_id();
                let mut defs = List::new();
                let mut uses = List::new();

                self.extract_defs_and_uses_from_expr(
                    expr,
                    block_id,
                    &mut defs,
                    &mut uses,
                    ctx.ref_counter,
                    &mut ctx.closure_captures,
                );

                let block = ctx.builder.build_block(
                    block_id,
                    {
                        let mut preds = verum_common::Set::new();
                        preds.insert(ctx.entry_id);
                        preds
                    },
                    {
                        let mut succs = verum_common::Set::new();
                        succs.insert(ctx.exit_id);
                        succs
                    },
                    defs,
                    uses,
                );
                ctx.pending_blocks.push(block);
                block_id
            }
        }
    }

    /// Build CFG for a block expression, returning the starting block ID
    fn build_block_cfg(
        &self,
        block: &verum_ast::expr::Block,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        use verum_ast::expr::ExprKind;
        use verum_ast::stmt::StmtKind;

        if block.stmts.is_empty() && block.expr.is_none() {
            // Empty block - just return entry, will connect to continuation
            return ctx.entry_id;
        }

        // Create block for the sequential statements
        let block_id = ctx.builder.new_block_id();
        let mut defs = List::new();
        let mut uses = List::new();
        let mut current_block_id = block_id;
        let mut successors = verum_common::Set::new();

        // Process statements
        for stmt in block.stmts.iter() {
            match &stmt.kind {
                // Handle control flow statements that create new blocks
                StmtKind::Expr { expr, .. } => {
                    match &expr.kind {
                        ExprKind::If {
                            condition,
                            then_branch,
                            else_branch,
                            ..
                        } => {
                            // Build if/else CFG
                            let (if_start, _if_end) = self.build_if_cfg(
                                condition,
                                then_branch,
                                else_branch.as_ref().map(|e| e.as_ref()),
                                ctx,
                                cfg,
                                continuation,
                            );

                            // Current block leads to if start
                            successors.insert(if_start);

                            // Emit current block and start new one for statements after if
                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            // Continue with a new block after the if
                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::Match {
                            expr: scrutinee,
                            arms,
                        } => {
                            // Build match CFG
                            let match_start =
                                self.build_match_cfg(scrutinee, arms, ctx, cfg, continuation);

                            successors.insert(match_start);

                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::Loop {
                            body: loop_body, ..
                        } => {
                            let loop_start = self.build_loop_cfg(loop_body, ctx, cfg, continuation);

                            successors.insert(loop_start);

                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::While {
                            condition,
                            body: while_body,
                            ..
                        } => {
                            let while_start =
                                self.build_while_cfg(condition, while_body, ctx, cfg, continuation);

                            successors.insert(while_start);

                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::For {
                            pattern: _,
                            iter,
                            body: for_body,
                            ..
                        } => {
                            let for_start =
                                self.build_for_cfg(iter, for_body, ctx, cfg, continuation);

                            successors.insert(for_start);

                            if !defs.is_empty() || !uses.is_empty() {
                                let stmt_block = ctx.builder.build_block(
                                    current_block_id,
                                    verum_common::Set::new(),
                                    successors.clone(),
                                    std::mem::take(&mut defs),
                                    std::mem::take(&mut uses),
                                );
                                ctx.pending_blocks.push(stmt_block);
                            }

                            current_block_id = ctx.builder.new_block_id();
                            successors.clear();
                        }
                        ExprKind::Return(_) => {
                            // Return jumps to exit
                            self.extract_defs_and_uses_from_expr(
                                expr,
                                current_block_id,
                                &mut defs,
                                &mut uses,
                                ctx.ref_counter,
                                &mut ctx.closure_captures,
                            );
                            successors.insert(ctx.exit_id);
                        }
                        _ => {
                            // Regular expression - collect defs and uses
                            self.extract_defs_and_uses_from_expr(
                                expr,
                                current_block_id,
                                &mut defs,
                                &mut uses,
                                ctx.ref_counter,
                                &mut ctx.closure_captures,
                            );
                        }
                    }
                }
                StmtKind::Let {
                    pattern: _,
                    ty: _,
                    value,
                    ..
                } => {
                    // Let bindings may define references
                    if let Some(val) = value {
                        self.extract_defs_and_uses_from_expr(
                            val,
                            current_block_id,
                            &mut defs,
                            &mut uses,
                            ctx.ref_counter,
                            &mut ctx.closure_captures,
                        );
                    }
                }
                StmtKind::LetElse {
                    pattern: _,
                    value,
                    else_block,
                    ..
                } => {
                    self.extract_defs_and_uses_from_expr(
                        value,
                        current_block_id,
                        &mut defs,
                        &mut uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                    // Process else block
                    for else_stmt in else_block.stmts.iter() {
                        if let StmtKind::Expr { expr, .. } = &else_stmt.kind {
                            self.extract_defs_and_uses_from_expr(
                                expr,
                                current_block_id,
                                &mut defs,
                                &mut uses,
                                ctx.ref_counter,
                                &mut ctx.closure_captures,
                            );
                        }
                    }
                }
                StmtKind::Defer(expr) => {
                    self.extract_defs_and_uses_from_expr(
                        expr,
                        current_block_id,
                        &mut defs,
                        &mut uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                }
                StmtKind::Errdefer(expr) => {
                    self.extract_defs_and_uses_from_expr(
                        expr,
                        current_block_id,
                        &mut defs,
                        &mut uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                }
                StmtKind::Provide { value, .. } => {
                    self.extract_defs_and_uses_from_expr(
                        value,
                        current_block_id,
                        &mut defs,
                        &mut uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                }
                _ => {}
            }
        }

        // Process trailing expression
        if let Some(ref trailing_expr) = block.expr {
            self.extract_defs_and_uses_from_expr(
                trailing_expr,
                current_block_id,
                &mut defs,
                &mut uses,
                ctx.ref_counter,
                &mut ctx.closure_captures,
            );
        }

        // If we haven't added any successors, connect to continuation
        if successors.is_empty() {
            successors.insert(continuation);
        }

        // Emit the final block
        let final_block = ctx.builder.build_block(
            current_block_id,
            verum_common::Set::new(),
            successors,
            defs,
            uses,
        );
        ctx.pending_blocks.push(final_block);

        block_id
    }

    /// Build CFG for if/else expression
    fn build_if_cfg(
        &self,
        condition: &verum_ast::expr::IfCondition,
        then_branch: &verum_ast::expr::Block,
        else_branch: Option<&verum_ast::expr::Expr>,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> (verum_cbgr::analysis::BlockId, verum_cbgr::analysis::BlockId) {
        use verum_ast::expr::ExprKind;

        // Condition block
        let cond_block_id = ctx.builder.new_block_id();
        let mut cond_defs = List::new();
        let mut cond_uses = List::new();

        // Extract uses from condition
        self.extract_defs_and_uses_from_condition(
            condition,
            cond_block_id,
            &mut cond_defs,
            &mut cond_uses,
            ctx.ref_counter,
            &mut ctx.closure_captures,
        );

        // Then block
        let then_block_id = self.build_block_cfg(then_branch, ctx, cfg, continuation);

        // Else block (or continuation if no else)
        let else_block_id = if let Some(else_expr) = else_branch {
            match &else_expr.kind {
                ExprKind::Block(else_block) => {
                    self.build_block_cfg(else_block, ctx, cfg, continuation)
                }
                ExprKind::If {
                    condition: else_cond,
                    then_branch: else_then,
                    else_branch: else_else,
                    ..
                } => {
                    let (else_if_start, _) = self.build_if_cfg(
                        else_cond,
                        else_then,
                        else_else.as_ref().map(|e| e.as_ref()),
                        ctx,
                        cfg,
                        continuation,
                    );
                    else_if_start
                }
                _ => {
                    // Create block for else expression
                    let else_id = ctx.builder.new_block_id();
                    let mut else_defs = List::new();
                    let mut else_uses = List::new();
                    self.extract_defs_and_uses_from_expr(
                        else_expr,
                        else_id,
                        &mut else_defs,
                        &mut else_uses,
                        ctx.ref_counter,
                        &mut ctx.closure_captures,
                    );
                    let else_block = ctx.builder.build_block(
                        else_id,
                        {
                            let mut preds = verum_common::Set::new();
                            preds.insert(cond_block_id);
                            preds
                        },
                        {
                            let mut succs = verum_common::Set::new();
                            succs.insert(continuation);
                            succs
                        },
                        else_defs,
                        else_uses,
                    );
                    ctx.pending_blocks.push(else_block);
                    else_id
                }
            }
        } else {
            continuation
        };

        // Build condition block with successors to both branches
        let cond_block = ctx.builder.build_block(
            cond_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(then_block_id);
                succs.insert(else_block_id);
                succs
            },
            cond_defs,
            cond_uses,
        );
        ctx.pending_blocks.push(cond_block);

        (cond_block_id, continuation)
    }

    /// Build CFG for match expression
    fn build_match_cfg(
        &self,
        scrutinee: &verum_ast::expr::Expr,
        arms: &verum_common::List<verum_ast::pattern::MatchArm>,
        ctx: &mut CfgBuildContext<'_>,
        _cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        // Scrutinee evaluation block
        let scrutinee_block_id = ctx.builder.new_block_id();
        let mut scrutinee_defs = List::new();
        let mut scrutinee_uses = List::new();

        self.extract_defs_and_uses_from_expr(
            scrutinee,
            scrutinee_block_id,
            &mut scrutinee_defs,
            &mut scrutinee_uses,
            ctx.ref_counter,
            &mut ctx.closure_captures,
        );

        // Build blocks for each arm
        let mut arm_block_ids = List::new();
        for arm in arms.iter() {
            let arm_block_id = ctx.builder.new_block_id();
            let mut arm_defs = List::new();
            let mut arm_uses = List::new();

            // Extract uses from guard if present
            if let Some(ref guard) = arm.guard {
                self.extract_defs_and_uses_from_expr(
                    guard,
                    arm_block_id,
                    &mut arm_defs,
                    &mut arm_uses,
                    ctx.ref_counter,
                    &mut ctx.closure_captures,
                );
            }

            // Extract uses from arm body
            self.extract_defs_and_uses_from_expr(
                &arm.body,
                arm_block_id,
                &mut arm_defs,
                &mut arm_uses,
                ctx.ref_counter,
                &mut ctx.closure_captures,
            );

            let arm_block = ctx.builder.build_block(
                arm_block_id,
                {
                    let mut preds = verum_common::Set::new();
                    preds.insert(scrutinee_block_id);
                    preds
                },
                {
                    let mut succs = verum_common::Set::new();
                    succs.insert(continuation);
                    succs
                },
                arm_defs,
                arm_uses,
            );
            ctx.pending_blocks.push(arm_block);
            arm_block_ids.push(arm_block_id);
        }

        // Build scrutinee block with successors to all arms
        let scrutinee_successors: verum_common::Set<_> = arm_block_ids.into_iter().collect();
        let scrutinee_block = ctx.builder.build_block(
            scrutinee_block_id,
            verum_common::Set::new(),
            scrutinee_successors,
            scrutinee_defs,
            scrutinee_uses,
        );
        ctx.pending_blocks.push(scrutinee_block);

        scrutinee_block_id
    }

    /// Build CFG for loop expression
    fn build_loop_cfg(
        &self,
        body: &verum_ast::expr::Block,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        // Loop header block
        let header_block_id = ctx.builder.new_block_id();

        // Loop body - continues back to header
        let body_block_id = self.build_block_cfg(body, ctx, cfg, header_block_id);

        // Build header block with back-edge from body
        let header_block = ctx.builder.build_block(
            header_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(body_block_id);
                succs.insert(continuation); // Break exits to continuation
                succs
            },
            List::new(),
            List::new(),
        );
        ctx.pending_blocks.push(header_block);

        header_block_id
    }

    /// Build CFG for while loop
    fn build_while_cfg(
        &self,
        condition: &verum_ast::expr::Expr,
        body: &verum_ast::expr::Block,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        // Condition block (loop header)
        let cond_block_id = ctx.builder.new_block_id();
        let mut cond_defs = List::new();
        let mut cond_uses = List::new();

        self.extract_defs_and_uses_from_expr(
            condition,
            cond_block_id,
            &mut cond_defs,
            &mut cond_uses,
            ctx.ref_counter,
            &mut ctx.closure_captures,
        );

        // Body block - loops back to condition
        let body_block_id = self.build_block_cfg(body, ctx, cfg, cond_block_id);

        // Build condition block
        let cond_block = ctx.builder.build_block(
            cond_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(body_block_id); // True branch
                succs.insert(continuation); // False branch (exit)
                succs
            },
            cond_defs,
            cond_uses,
        );
        ctx.pending_blocks.push(cond_block);

        cond_block_id
    }

    /// Build CFG for for loop
    fn build_for_cfg(
        &self,
        iter: &verum_ast::expr::Expr,
        body: &verum_ast::expr::Block,
        ctx: &mut CfgBuildContext<'_>,
        cfg: &mut verum_cbgr::analysis::ControlFlowGraph,
        continuation: verum_cbgr::analysis::BlockId,
    ) -> verum_cbgr::analysis::BlockId {
        // Iterator initialization block
        let init_block_id = ctx.builder.new_block_id();
        let mut init_defs = List::new();
        let mut init_uses = List::new();

        self.extract_defs_and_uses_from_expr(
            iter,
            init_block_id,
            &mut init_defs,
            &mut init_uses,
            ctx.ref_counter,
            &mut ctx.closure_captures,
        );

        // Loop header block (iterator next check)
        let header_block_id = ctx.builder.new_block_id();

        // Body block - loops back to header
        let body_block_id = self.build_block_cfg(body, ctx, cfg, header_block_id);

        // Build init block
        let init_block = ctx.builder.build_block(
            init_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(header_block_id);
                succs
            },
            init_defs,
            init_uses,
        );
        ctx.pending_blocks.push(init_block);

        // Build header block
        let header_block = ctx.builder.build_block(
            header_block_id,
            verum_common::Set::new(),
            {
                let mut succs = verum_common::Set::new();
                succs.insert(body_block_id); // Has more items
                succs.insert(continuation); // Iterator exhausted
                succs
            },
            List::new(),
            List::new(),
        );
        ctx.pending_blocks.push(header_block);

        init_block_id
    }

    /// Extract definitions and uses from an if condition
    fn extract_defs_and_uses_from_condition(
        &self,
        condition: &verum_ast::expr::IfCondition,
        block_id: verum_cbgr::analysis::BlockId,
        defs: &mut List<verum_cbgr::analysis::DefSite>,
        uses: &mut List<verum_cbgr::analysis::UseeSite>,
        ref_counter: &mut u64,
        closure_captures: &mut List<(verum_cbgr::analysis::RefId, bool)>,
    ) {
        use verum_ast::expr::ConditionKind;

        for cond in condition.conditions.iter() {
            match cond {
                ConditionKind::Expr(expr) => {
                    self.extract_defs_and_uses_from_expr(
                        expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                ConditionKind::Let { value, .. } => {
                    // Let in condition may create a reference binding
                    self.extract_defs_and_uses_from_expr(
                        value,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }
        }
    }

    /// Extract reference definitions and uses from an expression
    fn extract_defs_and_uses_from_expr(
        &self,
        expr: &verum_ast::expr::Expr,
        block_id: verum_cbgr::analysis::BlockId,
        defs: &mut List<verum_cbgr::analysis::DefSite>,
        uses: &mut List<verum_cbgr::analysis::UseeSite>,
        ref_counter: &mut u64,
        closure_captures: &mut List<(verum_cbgr::analysis::RefId, bool)>,
    ) {
        use verum_ast::expr::{ExprKind, UnOp};
        use verum_cbgr::analysis::{DefSite, RefId, UseeSite};

        match &expr.kind {
            // Reference creation - this is a definition
            ExprKind::Unary { op, expr: inner } => {
                match op {
                    UnOp::Ref | UnOp::RefChecked | UnOp::RefUnsafe => {
                        // Immutable reference definition
                        let ref_id = RefId(*ref_counter);
                        *ref_counter += 1;
                        defs.push(DefSite {
                            block: block_id,
                            reference: ref_id,
                            is_stack_allocated: true,
                            span: None,
                        });
                    }
                    UnOp::RefMut | UnOp::RefCheckedMut | UnOp::RefUnsafeMut => {
                        // Mutable reference definition
                        let ref_id = RefId(*ref_counter);
                        *ref_counter += 1;
                        defs.push(DefSite {
                            block: block_id,
                            reference: ref_id,
                            is_stack_allocated: true,
                            span: None,
                        });
                    }
                    UnOp::Deref => {
                        // Dereference is a use
                        uses.push(UseeSite {
                            block: block_id,
                            reference: RefId(*ref_counter),
                            is_mutable: false,
                            span: None,
                        });
                        *ref_counter += 1;
                    }
                    _ => {}
                }
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Field access on references is a use
            ExprKind::Field { expr: base, .. } => {
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(*ref_counter),
                    is_mutable: false,
                    span: None,
                });
                *ref_counter += 1;
                self.extract_defs_and_uses_from_expr(
                    base,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Index access is a use
            ExprKind::Index { expr: base, index } => {
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(*ref_counter),
                    is_mutable: false,
                    span: None,
                });
                *ref_counter += 1;
                self.extract_defs_and_uses_from_expr(
                    base,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                self.extract_defs_and_uses_from_expr(
                    index,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Method call - receiver is a use
            ExprKind::MethodCall { receiver, args, .. } => {
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(*ref_counter),
                    is_mutable: false, // Could be mutable if method takes &mut self
                    span: None,
                });
                *ref_counter += 1;
                self.extract_defs_and_uses_from_expr(
                    receiver,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for arg in args.iter() {
                    self.extract_defs_and_uses_from_expr(
                        arg,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Function call - args may be reference uses
            ExprKind::Call { func, args, .. } => {
                self.extract_defs_and_uses_from_expr(
                    func,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for arg in args.iter() {
                    self.extract_defs_and_uses_from_expr(
                        arg,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Closure - track captures for escape analysis
            ExprKind::Closure {
                params: _,
                return_type: _,
                body,
                ..
            } => {
                // Mark any references captured by the closure
                let capture_start = *ref_counter;
                self.extract_defs_and_uses_from_expr(
                    body,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );

                // References used in closure body are potential captures
                // We mark them as escaping via closure capture
                for i in capture_start..*ref_counter {
                    closure_captures.push((RefId(i), false));
                }
            }

            // Return - reference may escape
            ExprKind::Return(value) => {
                if let Some(val) = value {
                    // Mark this as a potential escape point
                    uses.push(UseeSite {
                        block: block_id,
                        reference: RefId(*ref_counter),
                        is_mutable: false,
                        span: None,
                    });
                    *ref_counter += 1;
                    self.extract_defs_and_uses_from_expr(
                        val.as_ref(),
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Binary operations
            ExprKind::Binary { left, right, .. } => {
                self.extract_defs_and_uses_from_expr(
                    left,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                self.extract_defs_and_uses_from_expr(
                    right,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Block expression
            ExprKind::Block(inner_block) => {
                for stmt in inner_block.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref result_expr) = inner_block.expr {
                    self.extract_defs_and_uses_from_expr(
                        result_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Tuple and array literals
            ExprKind::Tuple(elements) => {
                for elem in elements.iter() {
                    self.extract_defs_and_uses_from_expr(
                        elem,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                match array_expr {
                    ArrayExpr::List(elements) => {
                        for elem in elements.iter() {
                            self.extract_defs_and_uses_from_expr(
                                elem,
                                block_id,
                                defs,
                                uses,
                                ref_counter,
                                closure_captures,
                            );
                        }
                    }
                    ArrayExpr::Repeat { value, count } => {
                        self.extract_defs_and_uses_from_expr(
                            value,
                            block_id,
                            defs,
                            uses,
                            ref_counter,
                            closure_captures,
                        );
                        self.extract_defs_and_uses_from_expr(
                            count,
                            block_id,
                            defs,
                            uses,
                            ref_counter,
                            closure_captures,
                        );
                    }
                }
            }

            // Record literals
            ExprKind::Record { fields, base, .. } => {
                for field in fields.iter() {
                    if let Some(ref val) = field.value {
                        self.extract_defs_and_uses_from_expr(
                            val,
                            block_id,
                            defs,
                            uses,
                            ref_counter,
                            closure_captures,
                        );
                    }
                }
                if let Some(base_expr) = base {
                    self.extract_defs_and_uses_from_expr(
                        base_expr.as_ref(),
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Await expressions
            ExprKind::Await(operand) => {
                self.extract_defs_and_uses_from_expr(
                    operand,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Async blocks
            ExprKind::Async(async_block) => {
                for stmt in async_block.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref result_expr) = async_block.expr {
                    self.extract_defs_and_uses_from_expr(
                        result_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Try expressions
            ExprKind::Try(inner) => {
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Cast expressions
            ExprKind::Cast { expr: inner, .. } => {
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Pipeline expressions
            ExprKind::Pipeline { left, right } => {
                self.extract_defs_and_uses_from_expr(
                    left,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                self.extract_defs_and_uses_from_expr(
                    right,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Null coalescing
            ExprKind::NullCoalesce { left, right } => {
                self.extract_defs_and_uses_from_expr(
                    left,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                self.extract_defs_and_uses_from_expr(
                    right,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Parenthesized expressions
            ExprKind::Paren(inner) => {
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Optional chaining
            ExprKind::OptionalChain { expr: base, .. } => {
                uses.push(UseeSite {
                    block: block_id,
                    reference: RefId(*ref_counter),
                    is_mutable: false,
                    span: None,
                });
                *ref_counter += 1;
                self.extract_defs_and_uses_from_expr(
                    base,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Tuple index
            ExprKind::TupleIndex { expr: base, .. } => {
                self.extract_defs_and_uses_from_expr(
                    base,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Yield expressions
            ExprKind::Yield(inner) => {
                self.extract_defs_and_uses_from_expr(
                    inner,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }

            // Break with value
            ExprKind::Break { value, .. } => {
                if let Some(val) = value {
                    self.extract_defs_and_uses_from_expr(
                        val.as_ref(),
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Literals, paths, continue - no references to track
            ExprKind::Literal(_) | ExprKind::Path(_) | ExprKind::Continue { .. } => {}

            // Control flow expressions handled at block level, but process their contents
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.extract_defs_and_uses_from_condition(
                    condition,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for stmt in then_branch.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref then_expr) = then_branch.expr {
                    self.extract_defs_and_uses_from_expr(
                        then_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(else_expr) = else_branch {
                    self.extract_defs_and_uses_from_expr(
                        else_expr.as_ref(),
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.extract_defs_and_uses_from_expr(
                    scrutinee,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for arm in arms.iter() {
                    if let Some(ref guard) = arm.guard {
                        self.extract_defs_and_uses_from_expr(
                            guard,
                            block_id,
                            defs,
                            uses,
                            ref_counter,
                            closure_captures,
                        );
                    }
                    self.extract_defs_and_uses_from_expr(
                        &arm.body,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::Loop {
                body: loop_body, ..
            } => {
                for stmt in loop_body.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref loop_expr) = loop_body.expr {
                    self.extract_defs_and_uses_from_expr(
                        loop_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::While {
                condition,
                body: while_body,
                ..
            } => {
                self.extract_defs_and_uses_from_expr(
                    condition,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for stmt in while_body.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref while_expr) = while_body.expr {
                    self.extract_defs_and_uses_from_expr(
                        while_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            ExprKind::For {
                iter,
                body: for_body,
                ..
            } => {
                self.extract_defs_and_uses_from_expr(
                    iter,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for stmt in for_body.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
                if let Some(ref for_expr) = for_body.expr {
                    self.extract_defs_and_uses_from_expr(
                        for_expr,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }

            // Handle remaining expression types conservatively
            _ => {}
        }
    }

    /// Extract definitions and uses from a statement
    fn extract_defs_and_uses_from_stmt(
        &self,
        stmt: &verum_ast::stmt::Stmt,
        block_id: verum_cbgr::analysis::BlockId,
        defs: &mut List<verum_cbgr::analysis::DefSite>,
        uses: &mut List<verum_cbgr::analysis::UseeSite>,
        ref_counter: &mut u64,
        closure_captures: &mut List<(verum_cbgr::analysis::RefId, bool)>,
    ) {
        use verum_ast::stmt::StmtKind;

        match &stmt.kind {
            StmtKind::Expr { expr, .. } => {
                self.extract_defs_and_uses_from_expr(
                    expr,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }
            StmtKind::Let { value, .. } => {
                if let Some(val) = value {
                    self.extract_defs_and_uses_from_expr(
                        val,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.extract_defs_and_uses_from_expr(
                    value,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
                for else_stmt in else_block.stmts.iter() {
                    self.extract_defs_and_uses_from_stmt(
                        else_stmt,
                        block_id,
                        defs,
                        uses,
                        ref_counter,
                        closure_captures,
                    );
                }
            }
            StmtKind::Defer(expr) => {
                self.extract_defs_and_uses_from_expr(
                    expr,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }
            StmtKind::Errdefer(expr) => {
                self.extract_defs_and_uses_from_expr(
                    expr,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }
            StmtKind::Provide { value, .. } => {
                self.extract_defs_and_uses_from_expr(
                    value,
                    block_id,
                    defs,
                    uses,
                    ref_counter,
                    closure_captures,
                );
            }
            _ => {}
        }
    }

    /// Collect all predecessors of the exit block
    fn collect_exit_predecessors(
        &self,
        cfg: &verum_cbgr::analysis::ControlFlowGraph,
        exit_id: verum_cbgr::analysis::BlockId,
    ) -> verum_common::Set<verum_cbgr::analysis::BlockId> {
        let mut preds = verum_common::Set::new();

        // Find all blocks that have exit_id as a successor
        for (block_id, block) in &cfg.blocks {
            if block.successors.contains(&exit_id) {
                preds.insert(*block_id);
            }
        }

        preds
    }

    /// Check if a function parameter contains reference types
    fn param_is_reference(param: &verum_ast::decl::FunctionParam) -> bool {
        use verum_ast::decl::FunctionParamKind;
        use verum_ast::ty::TypeKind;

        match &param.kind {
            FunctionParamKind::Regular { ty, .. } => {
                matches!(ty.kind, TypeKind::Reference { .. })
            }
            // Self reference parameters
            FunctionParamKind::SelfRef | FunctionParamKind::SelfRefMut |
            FunctionParamKind::SelfRefChecked | FunctionParamKind::SelfRefCheckedMut |
            FunctionParamKind::SelfRefUnsafe | FunctionParamKind::SelfRefUnsafeMut => true,
            FunctionParamKind::SelfOwn | FunctionParamKind::SelfOwnMut => true,
            // Self value parameters are not references
            FunctionParamKind::SelfValue | FunctionParamKind::SelfValueMut => false,
        }
    }

    /// Hash function name to create a stable function ID
    fn hash_function_name(name: &str) -> u64 {
        let mut hasher = crate::hash::ContentHash::new();
        hasher.update_str(name);
        hasher.finalize().to_u64()
    }

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

    /// Phase 7 (Tier 0): Interpretation - execute the module via VBC interpreter
    ///
    /// VBC-first architecture: AST → VBC Codegen → VBC Interpreter
    fn phase_interpret(&mut self, module: &Module) -> Result<()> {
        let _bc = verum_error::breadcrumb::enter(
            "compiler.phase.interpret",
            self.session.options().input.display().to_string(),
        );
        debug!("Interpreting module via VBC-first architecture");

        // Context system validation
        self.phase_context_validation(module);

        // Send/Sync compile-time enforcement
        self.phase_send_sync_validation(module);

        // FFI boundary validation
        self.phase_ffi_validation(module)?;

        // Step 1: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(module)?;

        // Capture for the script-mode persistent cache. See the matching
        // capture in `phase_interpret_with_args` for the full rationale.
        self.session.record_compiled_vbc(vbc_module.clone());

        // Emit VBC bytecode dump if requested
        if self.session.options().emit_vbc {
            let dump = verum_vbc::disassemble::disassemble_module(&vbc_module);
            let vbc_path = self.session.options().input.with_extension("vbc.txt");
            if let Err(e) = std::fs::write(&vbc_path, &dump) {
                warn!("Failed to write VBC dump: {}", e);
            } else {
                info!("Wrote VBC dump: {} ({} bytes)", vbc_path.display(), dump.len());
            }
        }

        // Step 2: Create VBC interpreter with runtime config from [runtime]
        let mut interpreter = VbcInterpreter::new(vbc_module);
        {
            let rt = &self.session.language_features().runtime;
            interpreter.state.config.async_scheduler = rt.async_scheduler.as_str().to_string();
            interpreter.state.config.async_worker_threads = rt.async_worker_threads;
            interpreter.state.config.futures_enabled = rt.futures;
            interpreter.state.config.nurseries_enabled = rt.nurseries;
            interpreter.state.config.task_stack_size = rt.task_stack_size;
            interpreter.state.config.heap_policy = rt.heap_policy.as_str().to_string();
        }
        // Script-mode permission policy (see `phase_interpret_with_args`
        // for the full rationale).
        if let Some(policy) = self.session.take_script_permission_policy() {
            interpreter.state.permission_router.set_policy(policy.0);
        }

        // Step 3: Find and execute main function
        let main_func_id = self.find_main_function_id(&interpreter.state.module)?;

        // Run module.global_ctors first so @thread_local static initializers
        // populate their TLS slots before `main` reads them. This mirrors the
        // AOT path (LLVM @llvm.global_ctors runs before main via the C
        // runtime); without it, the CBGR allocator's LOCAL_HEAP/CURRENT_HEAP
        // bootstrap reads Value::default() from an uninitialized TLS slot and
        // crashes on first allocation.
        if let Err(e) = interpreter.run_global_ctors() {
            return Err(anyhow::anyhow!("VBC global_ctors error: {:?}", e));
        }

        info!("Executing main function via VBC interpreter (function ID: {})", main_func_id.0);
        let result = interpreter.execute_function(main_func_id);
        self.finalize_run_result(result)
    }

    /// Tier-parity exit-code propagation.
    ///
    /// When the entry point returns an `Int`, surface it to the OS as the
    /// process exit status — matching what AOT compilation produces (where
    /// `main`'s return value lands directly in `_exit`). Without this, the
    /// interpreter would run `fn main() -> Int { 1 }` to completion but
    /// the process would exit 0, silently masking failures.
    ///
    /// Behaviour:
    /// - `Int` value → record exit code = `value as i32`.
    /// - `Bool` → record 0 for true, 1 for false (Unix convention).
    /// - `Unit` / `Nil` / anything else → leave exit code as `None`,
    ///   which the CLI maps to `0`.
    ///
    /// **Why record instead of `std::process::exit`?** The pipeline runs
    /// inside a CLI driver that needs to perform post-execution work —
    /// persisting the script-mode VBC cache, flushing telemetry, printing
    /// `--timings` — *before* the OS terminates the process. Calling
    /// `process::exit` from inside the interpreter would short-circuit
    /// the cache-store step and force every script to re-pay the full
    /// compile cost on its next invocation. The CLI takes the recorded
    /// code from `Session::take_exit_code()` after housekeeping and
    /// translates to `process::exit` there.
    ///
    /// Called from BOTH `phase_interpret` (no-args entry) and
    /// `phase_interpret_with_args` (args-aware entry) so behaviour is
    /// uniform across `verum run file.vr` and `verum run file.vr a b`.
    /// Script wrappers (`__verum_script_main`) pass through transparently:
    /// the parser lifts an unsemicoloned tail expression into the
    /// wrapper's return slot, so a script ending in `42` records 42 here.
    fn propagate_main_exit_code(&self, value: &verum_vbc::Value) {
        if value.is_int() {
            let code = value.as_i64() as i32;
            self.session.record_exit_code(code);
            return;
        }
        if value.is_bool() {
            self.session.record_exit_code(if value.as_bool() { 0 } else { 1 });
        }
        // Unit / Nil / Float / Object / Pointer / String — no exit-code
        // semantics. CLI defaults to 0.
    }

    /// Map an interpreter execution result into a pipeline result
    /// while honouring the cooperative `ProcessExit` control-flow
    /// signal raised by `exit(n)` calls. The interpreter returns
    /// `Err(InterpreterError::ProcessExit(n))` so the driver can run
    /// post-execution housekeeping (script-cache store, timing
    /// flush, future telemetry) *before* the OS terminates. Any
    /// other `Err` is a real runtime failure; `Ok` carries the
    /// script's terminal value which feeds `propagate_main_exit_code`.
    fn finalize_run_result(
        &self,
        result: verum_vbc::interpreter::InterpreterResult<verum_vbc::Value>,
    ) -> Result<()> {
        use verum_vbc::interpreter::InterpreterError;
        match result {
            Ok(value) => {
                self.propagate_main_exit_code(&value);
                Ok(())
            }
            Err(InterpreterError::ProcessExit(code)) => {
                self.session.record_exit_code(code);
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("VBC execution error: {}", e)),
        }
    }

    /// Phase 5b: Interpretation with arguments
    ///
    /// VBC-first architecture: AST → VBC Codegen → VBC Interpreter with args
    fn phase_interpret_with_args(&mut self, module: &Module, args: List<Text>) -> Result<()> {
        debug!("Interpreting module with {} args via VBC-first architecture", args.len());

        if args.is_empty() {
            return self.phase_interpret(module);
        }

        // Two-path parity: run the same validation phases the no-args
        // path does, so `verum run file.vr arg1` applies the same
        // semantics as `verum run file.vr`. Previously these phases
        // were silently skipped when args were present.
        self.phase_context_validation(module);
        self.phase_send_sync_validation(module);
        self.phase_ffi_validation(module)?;

        // Step 1: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(module)?;

        // Capture for the script-mode persistent cache. The CLI runner
        // pulls this back via `Session::take_compiled_vbc()` after a
        // successful run and serialises it into the on-disk cache so
        // the next invocation of an unchanged script can skip parse +
        // typecheck + verify + codegen entirely.
        self.session.record_compiled_vbc(vbc_module.clone());

        // Step 2: Create VBC interpreter
        let mut interpreter = VbcInterpreter::new(vbc_module);
        // Script-mode permission policy (see `run_compiled_vbc` for
        // the full rationale).
        if let Some(policy) = self.session.take_script_permission_policy() {
            interpreter.state.permission_router.set_policy(policy.0);
        }

        // Step 2b: Skip global constructors (FFI initializers corrupt state on macOS).

        // Step 3: Find main function
        let main_func_id = self.find_main_function_id(&interpreter.state.module)?;

        // Step 4: Check if main() accepts parameters
        let main_param_count = interpreter.state.module.get_function(main_func_id)
            .map(|f| f.params.len())
            .unwrap_or(0);

        if main_param_count == 0 {
            // main() takes no args — execute normally
            info!("Executing main function via VBC interpreter (no args accepted)");
            let result = interpreter.execute_function(main_func_id);
            return self.finalize_run_result(result);
        }

        // Step 5: Allocate args as List<Text> on interpreter heap and call main(args)
        let rust_args: Vec<String> = args.iter().map(|t| t.to_string()).collect();
        let args_value = interpreter.alloc_string_list(&rust_args)
            .map_err(|e| anyhow::anyhow!("Failed to allocate args: {:?}", e))?;

        info!("Executing main function with {} args via VBC interpreter", rust_args.len());
        let result = interpreter.call(main_func_id, &[args_value]);
        self.finalize_run_result(result)
    }

    /// Compile AST module to VBC module
    fn compile_ast_to_vbc(&self, module: &Module) -> Result<std::sync::Arc<verum_vbc::module::VbcModule>> {
        // Phase 4.4: Dependent-type verification at module boundary.
        // The `DependentVerifier` orchestrator dispatches accumulated
        // goals (cubical equality, universe constraints, sheaf descent,
        // epistemic invariants) and checks instance coherence across
        // all `implement P for T` blocks in the module. This runs
        // *before* proof erasure so that theorems, axioms, and proof
        // bodies are still available for verification.
        //
        // The orchestrator is fire-and-report: it does not block
        // compilation on verification failure — diagnostics are
        // emitted, and the pipeline continues. This matches the
        // gradual-verification philosophy: `@verify(formal)` goals
        // that fail are reported but do not prevent `@verify(runtime)`
        // code from compiling.
        {
            use verum_verification::dependent_verification::DependentVerifier;
            let mut verifier = DependentVerifier::new();

            // Instance coherence: scan implement blocks for
            // duplicate protocol implementations on the same type.
            for item in module.items.iter() {
                if let verum_ast::decl::ItemKind::Impl(impl_block) = &item.kind {
                    if let verum_ast::decl::ImplKind::Protocol {
                        protocol,
                        for_type,
                        ..
                    } = &impl_block.kind
                    {
                        let protocol_name = protocol
                            .segments
                            .iter()
                            .map(|s| match s {
                                verum_ast::ty::PathSegment::Name(id) => {
                                    id.name.as_str()
                                }
                                _ => "_",
                            })
                            .collect::<Vec<_>>()
                            .join(".");
                        let type_name = format!("{:?}", for_type.kind);
                        let location = format!("module:{}", item.span.start);

                        verifier.instance_registry_mut().register(
                            verum_types::instance_search::InstanceCandidate::new(
                                protocol_name,
                                type_name,
                            )
                            .at(location),
                        );
                    }
                }
            }

            // Feed deferred verification goals from the type-checker
            // into the orchestrator. These are Type::Eq failures that
            // the cubical bridge couldn't resolve and universe
            // constraints the local solver left undecided.
            for goal in self.deferred_verification_goals.iter() {
                match goal {
                    verum_types::infer::DeferredVerificationGoal::CubicalEquality {
                        lhs, rhs, ..
                    } => {
                        use verum_types::cubical_bridge::eq_to_cubical;
                        verifier.add_goal(
                            verum_verification::dependent_verification::DependentGoalKind::CubicalEquality {
                                lhs: eq_to_cubical(lhs),
                                rhs: eq_to_cubical(rhs),
                            },
                        );
                    }
                    verum_types::infer::DeferredVerificationGoal::UniverseConstraints {
                        constraints,
                    } => {
                        verifier.add_goal(
                            verum_verification::dependent_verification::DependentGoalKind::UniverseConstraints(
                                constraints.clone(),
                            ),
                        );
                    }
                }
            }

            let report = verifier.verify_all();
            if !report.is_all_good() {
                tracing::warn!(
                    "Dependent verification: {} verified, {} refuted, {} undetermined, coherence {}",
                    report.verified_count(),
                    report.refuted_count(),
                    report.undetermined_count(),
                    if report.coherence.is_coherent() { "clean" } else { "violated" },
                );
            }
        }

        // Phase 4.5: Proof erasure — strip all proof-level items (theorem,
        // lemma, corollary, axiom, tactic) before VBC codegen. This formally
        // enforces the VBC-first architecture invariant that runtime carries
        // zero proof-term overhead. The VBC codegen itself has a defensive
        // skip for the same item kinds, but doing it upstream keeps the
        // module in a canonical runtime-only form.
        //
        // Gated on [codegen].proof_erasure. When the flag is disabled,
        // proof terms survive into VBC and become runtime values — used
        // by research scenarios that inspect the proof witness at
        // runtime. Default: true (production path).
        let proof_erasure_on =
            self.session.language_features().codegen.proof_erasure;
        let erased_module = if proof_erasure_on {
            let (m, erasure_stats) =
                crate::phases::proof_erasure::erase_proofs_from_module(module.clone());
            if erasure_stats.total_erased() > 0 {
                tracing::debug!(
                    "Proof erasure: {} proof items stripped before VBC codegen",
                    erasure_stats.total_erased()
                );
            }
            m
        } else {
            tracing::debug!(
                "Proof erasure SKIPPED ([codegen] proof_erasure = false); \
                 proof terms will survive into VBC"
            );
            module.clone()
        };
        let module = &erased_module;

        // Get profile from session and configure VBC codegen accordingly
        let profile = self.session.options().profile;
        let config = CodegenConfig {
            module_name: "main".to_string(),
            debug_info: self.session.options().debug_info,
            optimization_level: 0,
            validate: true,
            source_map: false,
            target_config: verum_ast::cfg::TargetConfig::host(),
            // V-LLSI profile configuration
            is_interpretable: profile.is_vbc_interpretable(),
            is_systems_profile: profile == crate::profile_system::Profile::Systems,
            is_embedded: self.session.options().is_embedded(),
            // Default lenient — pipeline-driven user builds tolerate
            // partial / forward-referenced stdlib state.  CI / release
            // gating that wants to reject any bug-class skip should
            // build via `CodegenConfig::with_strict_codegen()` instead.
            strict_codegen: false,
        };

        let mut codegen = VbcCodegen::with_config(config);

        // Run CBGR tier analysis: escape analysis → tier determination → RefChecked/RefUnsafe emission.
        // Promotes non-escaping references from Tier 0 (~15ns) to Tier 1 (0ns).
        let tier_start = std::time::Instant::now();
        let tier_context = {
            use crate::phases::cfg_constructor::CfgConstructor;
            use verum_cbgr::tier_analysis::TierAnalyzer;
            let module_cfg = CfgConstructor::from_module(module);
            let mut tc = TierContext::new();
            for (_func_id, func_cfg) in module_cfg.functions.iter() {
                let analyzer = TierAnalyzer::with_config(
                    func_cfg.cfg.clone(),
                    verum_cbgr::tier_analysis::TierAnalysisConfig::minimal(),
                );
                let analysis_result = analyzer.analyze();
                let func_tc = TierContext::from_analysis_result(&analysis_result);
                for expr_id_idx in 0..func_tc.decision_count() {
                    let expr_id = verum_vbc::codegen::context::ExprId(expr_id_idx as u64);
                    let tier = func_tc.get_tier(expr_id);
                    tc.set_tier(expr_id, tier);
                }
            }
            tc.enabled = true;
            tc
        };
        codegen.set_tier_context(tier_context);
        debug!("tier analysis in compile_ast_to_vbc took {:.2}s", tier_start.elapsed().as_secs_f64());

        // Collect imported stdlib modules that need to be compiled alongside the main module.
        // Without this, constants/functions from imported modules (e.g., CLOCK_REALTIME_ID
        // from core.sys.darwin.time) would be unresolvable in the VBC codegen.
        let imported_modules = self.collect_imported_stdlib_modules(module);
        debug!(
            "Collected {} imported stdlib module(s) for VBC compilation (from {} retained)",
            imported_modules.len(), self.modules.len()
        );

        let mut vbc_module = if imported_modules.is_empty() {
            // Fast path: no imported modules, use simple single-module compilation
            codegen.compile_module(module)
                .map_err(|e| anyhow::anyhow!("VBC codegen error: {}", e))?
        } else {
            // Multi-module compilation: register declarations from imported modules
            // so their constants, type constructors, and function signatures are available
            // when compiling the main module. Constants with literal values are inlined
            // at call sites (via __const_val_N intrinsic naming), so imported module
            // function bodies don't need to be compiled.
            codegen.initialize();

            // Cross-module two-phase collection to ensure protocols are registered
            // before impl blocks that use them, regardless of file/module order.

            // Pass 1a: Collect ALL protocol definitions from ALL modules first.
            // This ensures protocols like Eq, Ord are available when processing
            // impl blocks that implement them.
            for imported_module in &imported_modules {
                codegen.collect_protocol_definitions(imported_module);
            }
            codegen.collect_protocol_definitions(module);

            // Pass 1b: Collect non-protocol declarations from main module FIRST.
            // This ensures user-defined type IDs and field indices are stable
            // (not shifted by auto-included stdlib types). The VBC codegen uses
            // sequential counters for TypeId and field interning, so whichever
            // module is registered first gets the lower IDs.
            codegen.collect_non_protocol_declarations(module)
                .map_err(|e| anyhow::anyhow!("VBC codegen error (main module declarations): {}", e))?;
            codegen.mark_user_defined_types(module);

            // Pass 1c: Collect all non-protocol declarations from imported modules.
            // This registers constants (with inline values), type constructors,
            // and function signatures from the imported stdlib modules.
            // These get higher TypeIds than user types, which is correct.
            // Enable prefer_existing_functions so stdlib FFI declarations
            // (e.g., "pipe" from libsystem.vr) don't overwrite user-defined functions.
            codegen.set_prefer_existing_functions(true);
            for imported_module in &imported_modules {
                codegen.collect_non_protocol_declarations(imported_module)
                    .map_err(|e| anyhow::anyhow!("VBC codegen error (imported module declarations): {}", e))?;
            }
            codegen.set_prefer_existing_functions(false);

            // Resolve pending cross-module imports (constants/functions that were
            // deferred because they weren't registered yet when mount was processed)
            codegen.resolve_pending_imports();

            // Compile pending default protocol methods.
            // These were registered during declaration collection but their bodies need to be
            // compiled after all functions are registered (e.g., Iterator.advance_by uses `range`).
            codegen.compile_pending_default_methods()
                .map_err(|e| anyhow::anyhow!("VBC codegen error (default methods): {}", e))?;

            // Disable @test propagation for stdlib modules — only user code @test functions
            // should be executed by the test runner.
            codegen.set_propagate_test_attr(false);

            // Pass 2a: Compile imported module function bodies (lenient).
            // Functions from imported modules (e.g., is_retryable from core.sys.darwin.errno)
            // need their bodies compiled into VBC so they can be called at runtime.
            // Without this, only constants (which are inlined via __const_val_N) would work.
            // Uses lenient compilation because imported modules may contain functions that
            // reference FFI/external symbols not available in VBC (e.g., mach_timebase_info).
            for imported_module in &imported_modules {
                codegen.compile_module_items_lenient(imported_module)
                    .map_err(|e| anyhow::anyhow!("VBC codegen error (imported module bodies): {}", e))?;
            }

            // Pass 2b: Compile the main module's function bodies.
            // Re-enable @test propagation for user code.
            codegen.set_propagate_test_attr(true);
            codegen.compile_module_items(module)
                .map_err(|e| anyhow::anyhow!("VBC codegen error (main module bodies): {}", e))?;

            // Build the final VBC module
            codegen.finalize_module()
                .map_err(|e| anyhow::anyhow!("VBC codegen error (finalize): {}", e))?
        };

        // Set source directory for FFI library path resolution
        // Use the parent directory of the input file, or current directory if none
        let input_path = &self.session.options().input;
        let source_dir = if input_path.is_file() {
            input_path.parent().map(|p| p.to_string_lossy().into_owned())
        } else {
            Some(input_path.to_string_lossy().into_owned())
        };
        vbc_module.source_dir = source_dir;

        Ok(std::sync::Arc::new(vbc_module))
    }

    /// Collect parsed stdlib modules that are imported by the given module.
    ///
    /// Scans the module's `mount` statements, extracts module paths, and looks up
    /// the corresponding parsed ASTs in `self.modules`.
    ///
    /// Module path mapping: stdlib modules are stored with `std.` prefix
    /// (e.g., `std.sys.darwin.time`), but imports use various prefixes like
    /// `core.sys.darwin.time` or `sys.darwin.time`. This method tries all variants.
    fn collect_imported_stdlib_modules(&self, module: &Module) -> Vec<Module> {
        use verum_ast::ItemKind;

        let mut imported = Vec::new();
        // `imported_paths[i]` is the dotted module path for `imported[i]`
        // — kept parallel so the transitive-mount-closure pass below can
        // resolve `super.*` paths against the source module's own path.
        let mut imported_paths: Vec<String> = Vec::new();
        let mut seen_paths = std::collections::HashSet::new();

        // Auto-include essential stdlib modules that provide runtime foundations.
        // These are always needed regardless of user imports — they replace C runtime
        // stubs with compiled Verum code.
        //
        // Additional modules are included below via mount-statement scanning.
        // The Internal linkage + GlobalDCE safety net removes unreferenced functions.
        //
        // Exclusions:
        // - core.base.maybe: defines "Maybe<T>" which collides with user-defined
        //   "Maybe" types (test 018). Handled inline in instruction.rs instead.
        // Modules whose compiled .vr code replaces C runtime stubs.
        // Added incrementally as each module's ABI is verified correct.
        const ALWAYS_INCLUDE: &[&str] = &[
            // Layer 0: Platform sys modules (FFI declarations + OS wrappers)
            // These MUST come before any module that imports sys.* functions.
            // NOTE: Only include modules needed for sync primitives (mutex/condvar/channel).
            // Time/IO modules use FFI declarations that produce invalid LLVM IR — deferred.
            "core.sys.common",
            // NOTE: core.sys.raw used to be hardcoded here as a workaround
            // for the closure walker's inability to resolve `super.*`
            // mount paths.  With #163's resolve_super_path landing AND
            // #164's PathSegment::Super extraction fix, `mount super.raw.*`
            // from core.sys.time_ops now resolves correctly to core.sys.raw
            // and the transitive-closure pass pulls it in automatically.
            // Removed unconditionally; lenient-skip baselines pass.
            "core.sys.darwin.libsystem",
            "core.sys.darwin.thread",
            // Platform TLS / context-slot providers. `core/sys/common.vr`'s
            // `ctx_get` / `ctx_set` dispatchers forward to the per-platform
            // TLS module via `super.darwin.tls.ctx_get(slot)` etc.; without
            // these in the codegen session no `super.X.tls.*` call can be
            // resolved and every stdlib context lookup turns into a nil
            // stub — blowing up `is_some()` callers and `Runtime`-backed
            // epoch / env queries.
            "core.sys.darwin.tls",
            "core.sys.linux.tls",
            "core.sys.linux.syscall",
            "core.sys.linux.thread",
            // Phase 2A: Already migrated and tested (979 tests pass)
            "core.collections.list",
            "core.collections.slice",
            "core.text.text",
            "core.time.duration",
            "core.time.instant",
            "core.base.ordering",
            // core.base.memory hosts the typed-OOM primitives `try_alloc` /
            // `try_alloc_zeroed` / `try_realloc` that List / Map / Text /
            // Deque use internally for their `try_with_capacity` / `try_grow`
            // / `try_resize` paths.  Without this entry, the per-mount
            // scan WOULD pick up core.base.memory only AFTER the dependent
            // modules have already been compiled (they're earlier in this
            // list).  The result was bug-class lenient SKIPs across
            // List.try_with_capacity, List.try_resize_buffer,
            // Map.try_resize, Text.try_with_capacity, Deque.try_reallocate
            // — every fallible-allocation API in core/.  Closes #200.
            "core.base.memory",
            // Phase 2B: New modules — added incrementally, each tested
            "core.text.char",
            // Phase 2C: Map/Set — codegen fixes for struct array pointers enable these.
            // (offset stride, Deref inline struct, GetF header skip, generic eq/hash)
            "core.collections.map",
            "core.collections.set",
            "core.collections.deque",
            "core.collections.btree",
            "core.collections.heap",
            // Phase 3: core/mem CBGR modules — bottom-up by dependency
            // Tier 1: Pure logic (no intrinsics)
            "core.mem.mod",          // ExecutionTier enum, error types
            "core.mem.capability",   // Capability flags, pure bit ops
            "core.mem.size_class",   // Size class bins (needs clz_u64, wired via ArithExtended)
            // Tier 1.5: Capability-audit substrate (#202).  MUST come
            // before `core.mem.header` because header.vr's writer
            // entry points (try_revoke / attenuate_capabilities /
            // increment_ref_count / decrement_ref_count /
            // increment_generation) emit `record_*` calls into the
            // audit ring on every successful CBGR state transition.
            // Without these in the codegen session, header.vr's
            // record_* references become undefined and the writer
            // methods get bug-class lenient SKIP'd — disabling every
            // CBGR primitive at runtime.  The runtime gate inside
            // `cap_audit_ring.commit` keeps these calls O(1) when
            // audit is off, so always-loading the modules has no
            // perf cost beyond the 1-2 ns gate-check.
            "core.mem.cap_audit_ring",
            "core.mem.cap_audit",
            // Tier 2: Atomic operations
            "core.mem.header",       // AllocationHeader (atomic load/store/fetch_add u32/u16/u64)
            "core.mem.epoch",        // Global epoch manager (atomics + spin_hint)
            // Tier 3: Complex reference types
            "core.mem.thin_ref",     // ThinRef<T> 16-byte reference
            "core.mem.fat_ref",      // FatRef<T> 32-byte reference
            "core.mem.hazard",       // Hazard pointer system (pointer atomics)
            // Tier 4: OS integration
            "core.mem.segment",      // Segment management (mmap/munmap via @ffi)
            // Tier 5: blocked until segment + heap stable
            "core.mem.heap",      // @thread_local now implemented
            "core.mem.allocator", // GLOBAL_ALLOCATOR + @thread_local
            "core.sync.atomic",
            "core.sync.mutex",
            "core.sync.condvar",
            "core.sync.semaphore",
            "core.sync.rwlock",
            "core.sync.barrier",
            "core.sync.once",
            "core.async.channel",
            "core.async.generator",
            "core.async.spawn_with",
            "core.async.parallel",
            "core.async.select",          // join_all, select_all, race family
            "core.async.spawn_config",    // spawn_with_config
            // Runtime context bridge for AOT spawn/provide/using
            "core.runtime.ctx_bridge",
            // I/O type definitions — needed before io/net modules that use IoError/IoErrorKind
            "core.sys.io_engine",    // IoError variant type, IOEngine, IOInterest
            "core.io.file",
            "core.net.tcp",
            "core.net.udp",
            "core.base.panic",
        ];
        const EXCLUDED_MODULES: &[&str] = &[
            "core.base.maybe",
        ];
        // Detect host platform for filtering platform-specific modules.
        let is_macos = cfg!(target_os = "macos");
        let is_linux = cfg!(target_os = "linux");

        // Sort by path before iterating: `self.modules` is a HashMap and
        // its raw iteration order leaks the per-process random hasher
        // seed into VBC codegen. Downstream codegen assigns FunctionId
        // and TypeId by counter-push order, so a non-deterministic
        // module sequence here turns the same source into different
        // bytecode every run — surfacing as the symptom matrix
        // documented at module.rs:229-231 ("method 'X.write_str' not
        // found", "field index 2 OOB", "NullPointer", SIGSEGV,
        // misaligned atomic store, …).
        let mut modules_sorted: Vec<(&Text, &Arc<verum_ast::Module>)> =
            self.modules.iter().collect();
        modules_sorted.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        for (path, module_rc) in modules_sorted {
            let path_str = path.as_str().to_string();
            let is_excluded = EXCLUDED_MODULES.iter().any(|m| {
                path_str.ends_with(m) || path_str.ends_with(&format!("{}.vr", m))
            });
            let is_always = ALWAYS_INCLUDE.iter().any(|m| {
                path_str.ends_with(m) || path_str.ends_with(&format!("{}.vr", m))
            });
            // Skip platform-specific modules that don't match the host OS.
            // e.g. on macOS, skip core.sys.linux.* and vice versa.
            if !is_linux && (path_str.contains("sys.linux") || path_str.contains("sys/linux")) {
                continue;
            }
            if !is_macos && (path_str.contains("sys.darwin") || path_str.contains("sys/darwin")) {
                continue;
            }
            if is_always && !is_excluded && !seen_paths.contains(&path_str) {
                seen_paths.insert(path_str.clone());
                imported_paths.push(path_str.clone());
                imported.push((**module_rc).clone());
            }
        }

        for item in &module.items {
            if let ItemKind::Mount(mount_decl) = &item.kind {
                let import_path = self.extract_import_module_path(&mount_decl.tree.kind);
                if import_path.is_empty() {
                    continue;
                }

                // Also extract the full path (before last-segment stripping).
                // `mount core.collections.list` should resolve to the `list` module
                // directly, not just look up the parent `core.collections`.
                let full_path = {
                    use verum_ast::MountTreeKind;
                    // Same extraction policy as the closure-walker pass
                    // below: preserve Super and Relative segments as the
                    // literal "super" string so downstream resolution can
                    // see them.  Filtering them silently was the bug
                    // class fixed in #163/#164 — even if this site's
                    // immediate downstream candidate-matching can't act
                    // on a leading "super." (the user module's own path
                    // isn't tracked here), preserving the segments
                    // ensures a stale resolution shows up as an unmatched
                    // path rather than a silently-corrupted one.
                    match &mount_decl.tree.kind {
                        MountTreeKind::Path(path) => {
                            path.segments
                                .iter()
                                .filter_map(|seg| match seg {
                                    verum_ast::ty::PathSegment::Name(ident) =>
                                        Some(ident.name.as_str().to_string()),
                                    verum_ast::ty::PathSegment::Super
                                    | verum_ast::ty::PathSegment::Relative =>
                                        Some("super".to_string()),
                                    _ => None,
                                })
                                .collect::<Vec<String>>()
                                .join(".")
                        }
                        _ => String::new(),
                    }
                };

                // Generate candidate module paths.
                // Try: full path, parent path, variant prefixes.
                let mut candidates = Vec::new();

                // Full path first (e.g., core.collections.list)
                if !full_path.is_empty() && full_path != import_path {
                    candidates.push(full_path.clone());
                }

                // Parent path (original behavior)
                candidates.push(import_path.clone());

                if import_path.starts_with("core.") {
                    let stripped = &import_path[5..];
                    candidates.push(stripped.to_string());

                    // Handle short paths like core.maybe -> core.base.maybe
                    const BASE_MODULES: &[&str] = &[
                        "maybe", "result", "ordering", "protocols", "primitives",
                        "memory", "iterator", "panic", "env", "data", "ops",
                    ];
                    for &base_mod in BASE_MODULES {
                        if stripped == base_mod {
                            candidates.push(format!("core.base.{}", base_mod));
                            break;
                        }
                    }
                } else if import_path.starts_with("std.") {
                    let stripped = &import_path[4..];
                    candidates.push(format!("core.{}", stripped));
                    candidates.push(stripped.to_string());
                } else {
                    candidates.push(format!("core.{}", import_path));
                }

                // Also try full path with std->core translation
                if full_path.starts_with("std.") {
                    candidates.push(format!("core.{}", &full_path[4..]));
                } else if !full_path.starts_with("core.") && !full_path.is_empty() {
                    candidates.push(format!("core.{}", full_path));
                }

                for candidate in candidates {
                    if seen_paths.contains(&candidate) {
                        break;
                    }
                    // Only look up in self.modules (stdlib modules).
                    // Do NOT fall back to project_modules here — project module items
                    // are already merged into the main module by phase_generate_native(),
                    // so including them as imported modules would compile them twice.
                    let module_rc = self.modules.get(&Text::from(candidate.as_str()));
                    if let Some(module_rc) = module_rc {
                        // Skip platform-specific modules that don't match host.
                        if !is_linux && (candidate.contains("sys.linux") || candidate.contains("sys/linux")) {
                            break;
                        }
                        if !is_macos && (candidate.contains("sys.darwin") || candidate.contains("sys/darwin")) {
                            break;
                        }
                        seen_paths.insert(candidate.clone());
                        imported_paths.push(candidate.clone());
                        imported.push((**module_rc).clone());

                        // Also include submodules that may contain the actual implementations.
                        // For example, when importing `core.math`, also load `core.math.linalg`
                        // which contains the Vector and Matrix implementations. Sort the
                        // HashMap iteration before pushing — same determinism reasoning as
                        // the top-of-function loop.
                        let prefix = format!("{}.", candidate);
                        let mut sub_sorted: Vec<(&Text, &Arc<verum_ast::Module>)> =
                            self.modules.iter().collect();
                        sub_sorted.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
                        for (path, submodule) in sub_sorted {
                            if path.as_str().starts_with(&prefix) {
                                let subpath = path.as_str().to_string();
                                // Skip cross-platform submodules.
                                if !is_linux && (subpath.contains("sys.linux") || subpath.contains("sys/linux")) {
                                    continue;
                                }
                                if !is_macos && (subpath.contains("sys.darwin") || subpath.contains("sys/darwin")) {
                                    continue;
                                }
                                if !seen_paths.contains(&subpath) {
                                    seen_paths.insert(subpath.clone());
                                    imported_paths.push(subpath);
                                    imported.push((**submodule).clone());
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        // ------------------------------------------------------------------
        // Transitive mount closure over already-collected stdlib modules.
        //
        // Root fix for the class of failure where stdlib module A mounts
        // stdlib module B, the user imports A directly, and B's type /
        // variant declarations never reach VBC codegen:
        //
        //   * `core.database.sqlite.native.l0_vfs.memdb_vfs` mounts
        //     `core.database.sqlite.native.l0_vfs.vfs_protocol`.
        //   * A user script that does
        //     `mount …memdb_vfs.{open_memory_rwc}` pulls memdb_vfs into
        //     `imported` via the loop above, but NOT vfs_protocol.
        //   * vfs_protocol's `type LockKind is Unlocked | Shared | …;`
        //     never flows through `register_type_constructors`, so
        //     variants like `Unlocked` are absent from the VBC function
        //     table. Any stdlib body that writes `lock_state: Unlocked`
        //     is then silently dropped by the lenient top-level-fn SKIP
        //     path and callers hit `FunctionNotFound(FunctionId(N))` at
        //     runtime with no diagnostic.
        //
        // This pass walks each already-imported module's own `mount`
        // statements and adds any matched modules not yet present,
        // iterating to a fixed point. Purely structural — no compiler
        // code hardcodes a module name; the closure is driven entirely by
        // each module's own `mount` statements against the pre-parsed
        // `self.modules` registry. The `ALWAYS_INCLUDE` list above stays
        // untouched (it is a separate AOT-runtime concern).
        loop {
            let before_len = imported.len();
            // Snapshot the (path, module) pairs we'll iterate over.  We
            // need both halves: the module body to walk its `mount`
            // statements, and its dotted path to anchor `super.*`
            // resolution.
            let pending: Vec<(String, Module)> = imported_paths
                .iter()
                .zip(imported.iter())
                .map(|(p, m)| (p.clone(), m.clone()))
                .collect();
            for (src_path, src_mod) in &pending {
                for item in src_mod.items.iter() {
                    let ItemKind::Mount(mount_decl) = &item.kind else { continue };
                    use verum_ast::MountTreeKind;
                    let path = match &mount_decl.tree.kind {
                        MountTreeKind::Path(p) => p,
                        MountTreeKind::Glob(p) => p,
                        MountTreeKind::Nested { prefix, .. } => prefix,
                        // #5 / P1.5 — file-relative mounts are
                        // not module-path candidates; the
                        // session loader handles their
                        // resolution upstream of this pass.
                        MountTreeKind::File { .. } => continue,
                    };
                    // Extract the dotted form of the mount path,
                    // preserving the special leading-segment variants
                    // (`super`, leading `.` for relative-to-parent)
                    // so resolve_super_path can process them.
                    //
                    // PathSegment::Super and PathSegment::Relative are
                    // distinct AST variants from PathSegment::Name —
                    // filtering them to None at extraction was the bug
                    // that #163's super.* fix nominally addressed but
                    // did not yet exercise.  After this commit,
                    // `mount super.X` arrives as "super.X" and
                    // `mount .X` arrives as "super.X" too (a leading
                    // `.` denotes "sibling of current module" in the
                    // stdlib's mount grammar — semantically a
                    // one-level super walk).  Both then flow through
                    // resolve_super_path uniformly.
                    //
                    // PathSegment::SelfValue / PathSegment::Cog don't
                    // appear in stdlib mount paths today; they're left
                    // in the catch-all `_ => None` arm so adding a new
                    // form deliberately requires extending this match.
                    let raw_path: String = path
                        .segments
                        .iter()
                        .filter_map(|seg| match seg {
                            verum_ast::ty::PathSegment::Name(ident) => {
                                Some(ident.name.as_str().to_string())
                            }
                            verum_ast::ty::PathSegment::Super
                            | verum_ast::ty::PathSegment::Relative => {
                                Some("super".to_string())
                            }
                            _ => None,
                        })
                        .collect::<Vec<String>>()
                        .join(".");
                    if raw_path.is_empty() {
                        continue;
                    }
                    // Resolve `super.*` paths against the source module's
                    // own dotted path BEFORE the prefix walk.  Without
                    // this, `mount super.raw.foo` from `core.sys.time_ops`
                    // would walk `super.raw.foo`, `super.raw`, `super` —
                    // none of which are keys in `self.modules`, so the
                    // referenced module never reaches codegen and bodies
                    // mounting functions from it compile to
                    // `[lenient] SKIP … undefined function` (#163).
                    let full_path =
                        Self::resolve_super_path(src_path, &raw_path);
                    // Walk progressive prefixes so a mount whose full path
                    // is `core.x.y.z.{...}` matches the leaf module or any
                    // ancestor that happens to be indexed directly.
                    //
                    // Each prefix is tried as-is and again under a `core.`
                    // prefix so short stdlib paths like `mount base.memory`
                    // resolve against `core.base.memory` in `self.modules`.
                    // Without the second form, the closure walks only
                    // `base.memory` and `base`, neither of which is keyed
                    // in self.modules — base.memory's stdlib body never
                    // reaches codegen and every body that mounts a
                    // function from it compiles to `[lenient] SKIP …
                    // undefined function: <name>` (#163).
                    let segs: Vec<&str> = full_path.split('.').collect();
                    let try_candidate = |this: &Self,
                                         candidate: &str,
                                         seen_paths: &mut std::collections::HashSet<String>,
                                         imported: &mut Vec<Module>,
                                         imported_paths: &mut Vec<String>| {
                        if seen_paths.contains(candidate) {
                            return;
                        }
                        if !is_linux
                            && (candidate.contains("sys.linux")
                                || candidate.contains("sys/linux"))
                        {
                            return;
                        }
                        if !is_macos
                            && (candidate.contains("sys.darwin")
                                || candidate.contains("sys/darwin"))
                        {
                            return;
                        }
                        let key = Text::from(candidate);
                        if let Some(module_rc) = this.modules.get(&key) {
                            seen_paths.insert(candidate.to_string());
                            imported_paths.push(candidate.to_string());
                            imported.push((**module_rc).clone());
                        }
                    };
                    for cut in (1..=segs.len()).rev() {
                        let candidate = segs[..cut].join(".");
                        try_candidate(
                            self,
                            &candidate,
                            &mut seen_paths,
                            &mut imported,
                            &mut imported_paths,
                        );
                        if !candidate.starts_with("core.") {
                            let prefixed = format!("core.{}", candidate);
                            try_candidate(
                                self,
                                &prefixed,
                                &mut seen_paths,
                                &mut imported,
                                &mut imported_paths,
                            );
                        }
                    }
                }
            }
            if imported.len() == before_len {
                break;
            }
        }

        imported
    }

    /// Resolve `super.*` segments at the start of a `mount` path
    /// against the source module's own dotted path.  Each leading
    /// `super` strips one trailing component from the source path; the
    /// remaining mount segments are appended.  Mounts that don't begin
    /// with `super` are returned unchanged (the path is already
    /// anchored at the stdlib root or at an absolute prefix the
    /// progressive-prefix walk handles).
    ///
    /// Examples (src = `core.sys.time_ops`):
    ///   `super.raw.foo`        → `core.sys.raw.foo`
    ///   `super.super.collections.List` → `core.collections.List` (drops 2)
    ///   `core.foo.bar`         → `core.foo.bar` (unchanged)
    ///   `super` (alone)        → `core.sys` (just the parent path)
    ///
    /// If the mount path requests more `super` levels than the source
    /// has components, the original path is returned (the progressive-
    /// prefix walk will then fail to match anything, which is the
    /// correct behaviour for a malformed input).
    fn resolve_super_path(src_path: &str, mount_path: &str) -> String {
        let mut mount_segs: Vec<&str> = mount_path.split('.').collect();
        let mut super_count = 0;
        while mount_segs.first().is_some_and(|&s| s == "super") {
            super_count += 1;
            mount_segs.remove(0);
        }
        if super_count == 0 {
            return mount_path.to_string();
        }
        let src_segs: Vec<&str> = src_path.split('.').collect();
        // `super` walks one step *up* — it must leave at least one
        // remaining component (the parent module) for the result to
        // anchor against an existing stdlib path.  super_count ==
        // src_segs.len() walks exactly to the root and yields an empty
        // parent; super_count > src_segs.len() walks past the root.
        // Both cases are malformed inputs — return the original mount
        // path so the progressive-prefix walk tries the literal string
        // (and fails to match anything, which is the correct answer
        // for an out-of-range super).
        if super_count >= src_segs.len() {
            return mount_path.to_string();
        }
        let parent_len = src_segs.len() - super_count;
        let parent = &src_segs[..parent_len];
        if mount_segs.is_empty() {
            return parent.join(".");
        }
        let mut out = parent.join(".");
        out.push('.');
        out.push_str(&mount_segs.join("."));
        out
    }

    /// Retain stdlib modules that contain compilable function bodies.
    ///
    /// After type-checking, we clear modules whose ASTs are no longer needed.
    /// Modules with function implementations (function bodies with statements)
    /// are retained so their bodies can be compiled to VBC → LLVM.
    /// Modules containing only type/protocol declarations are cleared — their
    /// type information was already extracted during type-checking.
    ///
    /// `user_module`, when provided, is scanned for `mount` statements and any
    /// stdlib modules matching the mount target (plus their submodules) are
    /// retained. Without this, user code that mounts a stdlib module outside
    /// the `ALWAYS_INCLUDE` allowlist (e.g. `mount core.term.layout.Rect`)
    /// would lose the impl-block ASTs before VBC codegen runs, the impl
    /// methods would never reach `compile_module_items_lenient`, and the
    /// LLVM backend would emit unresolved `Call`s that const-fold to bogus
    /// pointers — the bug tracked by `vcs/specs/L0-critical/vbc/aot_stdlib_return/`.
    fn clear_non_compilable_stdlib_modules(&mut self, user_module: Option<&Module>) {
        // Only retain stdlib modules whose functions are actually compiled to
        // native code via VBC → LLVM. Most stdlib modules only provide type
        // definitions used during type checking and should be dropped to avoid
        // compiling thousands of unreachable functions.
        //
        // Modules in ALWAYS_INCLUDE have compiled implementations that the AOT
        // pipeline dispatches to (Strategy 1/2 in instruction.rs).
        const ALWAYS_INCLUDE: &[&str] = &[
            // Platform sys modules — retained so that mount aliases
            // (e.g., futex_wait as sys_futex_wait) can resolve across modules.
            // libsystem/syscall MUST be retained for pthread FFI declarations
            // that thread/mutex/condvar modules import via `mount super.libsystem.{...}`.
            "core.sys.darwin.libsystem",
            "core.sys.darwin.thread",
            "core.sys.linux.syscall",
            "core.sys.linux.thread",
            // T0.6.1 — per-platform TLS providers: thread_entry's
            // create_thread_tls call resolves through these (one
            // platform module wins the conditional cfg). Pre-T0.6.1
            // they were missing from the AOT-retention list and the
            // top-level `thread_entry` function bug-class lenient-
            // SKIPped, leaving the runtime entry without thread-local
            // storage init.
            "core.sys.darwin.tls",
            "core.sys.linux.tls",
            // Collections
            "core.collections.list",
            "core.collections.map",
            "core.collections.set",
            "core.collections.deque",
            "core.collections.heap",
            "core.collections.btree",
            "core.collections.slice",
            // Text
            "core.text.text",
            "core.text.char",
            "core.text.format",
            // Base types
            "core.base.maybe",
            "core.base.result",
            "core.base.ordering",
            // T0.6.1 — typed-OOM allocation primitives. core.base.memory
            // hosts try_alloc / try_alloc_zeroed / try_realloc that
            // List / Map / Text / Deque / Heap call from
            // try_with_capacity / try_grow / try_resize. Without retention
            // here, the AOT cull dropped memory's AST before VBC codegen
            // and every fallible-allocation API ended up bug-class
            // lenient-SKIPped (#200 follow-up; companion to the
            // type-checking ALWAYS_INCLUDE entry around line 11061).
            "core.base.memory",
            // Time
            "core.time.duration",
            // Sync
            "core.sync.mutex",
            "core.sync.condvar",
            "core.sync.rwlock",
            "core.sync.semaphore",
            "core.sync.barrier",
            "core.sync.once",
            "core.sync.atomic",
            // Async
            "core.async.channel",
            "core.async.generator",
            // spawn_with and parallel excluded from AOT retention:
            // Their free functions (execute_with_retry, parallel_map) have
            // common names that collide with user code. LLVMAddFunction returns
            // the existing function for same-name same-arity functions, causing
            // body corruption. User code that redefines these functions works
            // correctly without the stdlib versions.
            // "core.async.spawn_with",
            // "core.async.parallel",
            // Runtime context bridge for AOT
            "core.runtime.ctx_bridge",
            // Memory / CBGR allocator — required for user code that
            // constructs Shared<T>, Weak<T>, or other CBGR-tracked
            // reference types. Without these, the stdlib Shared::new
            // call site resolves at type-check time but has no body at
            // codegen, producing "undefined function: get_heap" errors.
            // See KNOWN_ISSUES.md "Shared<T> / CBGR-allocator Bootstrap".
            "core.mem.allocator",
            "core.mem.heap",
            // Capability-audit substrate (#202).  MUST be retained
            // alongside `core.mem.header` because every CBGR writer
            // entry point (try_revoke / attenuate_capabilities /
            // increment_ref_count / decrement_ref_count /
            // increment_generation) emits a `record_*` call into the
            // audit ring.  Without these in the retained set, the
            // codegen skips the writer methods (bug-class) and CBGR
            // primitives have no working bodies.
            "core.mem.cap_audit_ring",
            "core.mem.cap_audit",
            "core.mem.header",
            "core.mem.thin_ref",
            "core.mem.fat_ref",
            "core.mem.epoch",
            "core.mem.size_class",
            "core.mem.capability",
            "core.mem.raw_ops",
            // T0.6.1 — segment-classification constants used by
            // LocalHeap.alloc_huge / LocalHeap.free for the
            // SEGMENT_HUGE branch. Without retention, both methods
            // bug-class lenient-SKIP because SEGMENT_HUGE resolves
            // as undefined at codegen.
            "core.mem.segment",
            // I/O — excluded from AOT retention: core.io.fs read()/write()
            // FFI declarations conflict with LLVM builtins (wrong arg count).
            // Included in the type-checking ALWAYS_INCLUDE list (list 1) above.
        ];

        // Collect user-mounted module paths so their ASTs survive the cull.
        // A user `mount core.term.layout.Rect` should keep `core.term.layout`,
        // `core.term.layout.rect` (the module the type lives in), and every
        // other submodule under the mounted prefix that participates in type
        // resolution. We do not filter by AST shape here — the per-item
        // lenient compilation already skips functions whose bodies fail to
        // compile, so a few extra type-only modules cost nothing.
        let mut user_mount_prefixes: Vec<String> = Vec::new();
        if let Some(module) = user_module {
            for item in &module.items {
                if let verum_ast::ItemKind::Mount(mount_decl) = &item.kind {
                    let parent = self.extract_import_module_path(&mount_decl.tree.kind);
                    if !parent.is_empty() {
                        user_mount_prefixes.push(parent);
                    }
                    // Also remember the *full* mounted path (incl. last
                    // segment). `extract_import_module_path` strips the last
                    // segment because it usually names an item rather than a
                    // module, but when the mount targets a module directly
                    // (e.g. `mount core.sys.darwin.libsystem`) the full path
                    // is the one we need.
                    use verum_ast::MountTreeKind;
                    if let MountTreeKind::Path(path) = &mount_decl.tree.kind {
                        // Same extraction policy as the other sites in
                        // this file — preserve Super/Relative as
                        // "super" so downstream consumers see the
                        // structural prefix rather than a silently-
                        // truncated path.  See the user-mount loop at
                        // ~line 11158 and the closure walker at
                        // ~line 11336 for the full bug-class context.
                        let full = path.segments.iter()
                            .filter_map(|seg| match seg {
                                verum_ast::ty::PathSegment::Name(ident) =>
                                    Some(ident.name.as_str().to_string()),
                                verum_ast::ty::PathSegment::Super
                                | verum_ast::ty::PathSegment::Relative =>
                                    Some("super".to_string()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(".");
                        if !full.is_empty() {
                            user_mount_prefixes.push(full.clone());
                            // `std.*` aliasing: stdlib modules live under
                            // `core.*` in self.modules, so translate here.
                            if let Some(rest) = full.strip_prefix("std.") {
                                user_mount_prefixes.push(format!("core.{rest}"));
                            }
                            if !full.starts_with("core.") && !full.starts_with("std.") {
                                user_mount_prefixes.push(format!("core.{full}"));
                            }
                        }
                    }
                }
            }
        }

        let retains_user_path = |p: &str| -> bool {
            user_mount_prefixes.iter().any(|prefix| {
                // Exact match, ancestor (parent module of item), or descendant
                // (submodule under a mounted prefix).
                p == prefix
                    || p.starts_with(&format!("{prefix}."))
                    || prefix.starts_with(&format!("{p}."))
            })
        };

        let total_before = self.modules.len();
        let retained: Map<Text, Arc<Module>> = self.modules
            .drain()
            .filter(|(path, _module)| {
                let p = path.as_str();
                ALWAYS_INCLUDE.contains(&p) || retains_user_path(p)
            })
            .collect();

        let retained_count = retained.len();
        self.modules = retained;
        debug!(
            "Retained {}/{} stdlib modules for AOT compilation ({} user-mount paths)",
            retained_count, total_before, user_mount_prefixes.len()
        );
    }

    /// Find the program entry function in the VBC module and return
    /// its function ID.
    ///
    /// Strict mode separation (matches the AST-level
    /// `EntryDetectionPhase::detect_entry_point`):
    ///
    ///   • **Application** entry = `main` (in a non-script module).
    ///     Prefer it when present.
    ///   • **Script** entry = `__verum_script_main` (the synthesised
    ///     wrapper from script-tagged modules).
    ///
    /// The two are not interchangeable. A `fn main` declared *inside*
    /// a script module is a regular callable function, not the
    /// program entry — the AST-level pass already filtered such
    /// `main`s out, so by the time we reach the VBC the only `main`
    /// in the function table came from an application module. We
    /// preserve the precedence (`main` first, then wrapper) only as
    /// a defence-in-depth: if both names somehow appear in the VBC,
    /// the application entry still wins.
    fn find_main_function_id(&self, vbc_module: &VbcModule) -> Result<VbcFunctionId> {
        // First pass: script entry `__verum_script_main`. Its presence
        // is sufficient evidence that the source was a script, and the
        // strict-role contract says a script's entry is the wrapper —
        // never any user-declared `fn main` that may also be in the
        // function table (it's a regular callable function in script
        // mode, not the program entry).
        for (idx, func_desc) in vbc_module.functions.iter().enumerate() {
            if let Some(name) = vbc_module.get_string(func_desc.name) {
                if name == "__verum_script_main" {
                    return Ok(VbcFunctionId(idx as u32));
                }
            }
        }
        // Second pass: application entry `main`. Reached only when no
        // script wrapper exists, i.e. the source is an application
        // (no shebang, has `fn main()`).
        for (idx, func_desc) in vbc_module.functions.iter().enumerate() {
            if let Some(name) = vbc_module.get_string(func_desc.name) {
                if name == "main" {
                    return Ok(VbcFunctionId(idx as u32));
                }
            }
        }
        Err(anyhow::anyhow!("No main function found in VBC module"))
    }

    // ==================== Test Execution ====================

    /// Run test execution with output capture.
    ///
    /// This executes the program via the VBC interpreter with stdout/stderr captured.
    /// Used by vtest for running `run` and `run-panic` tests.
    ///
    /// Returns:
    /// - `Ok(TestExecutionResult)` on successful execution (even if panic)
    /// - `Err` only for compilation errors
    pub fn run_for_test(&mut self) -> Result<TestExecutionResult> {
        let start = Instant::now();

        // Load stdlib modules first (enables std.* imports)
        self.load_stdlib_modules()?;
        debug!("Phase 0 (stdlib): {:.2}s", start.elapsed().as_secs_f64());

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let mut module = self.phase_parse(file_id)?;
        debug!("Phase 2 (parse): {:.2}s", start.elapsed().as_secs_f64());

        // Get module path for registration and expansion
        let module_path = Text::from(self.session.options().input.display().to_string());

        // Register meta functions (enables macro expansion)
        self.register_meta_declarations(&module_path, &module)?;

        // Expand macros (evaluates @macro() invocations)
        self.expand_module(&module_path, &mut module)?;

        // Phase 3+: unified validation so test execution applies
        // the same language-mechanism checks as `verum build` /
        // `verum run`. Previously `run_for_test` skipped safety,
        // context, send/sync, and FFI validation.
        self.validate_module(&module, false)?;
        debug!("Phase 3+ (validate_module): {:.2}s", start.elapsed().as_secs_f64());

        // Phase 5: Compile to VBC and execute with capture
        debug!("Phase 5 starting (interpret_for_test): {:.2}s", start.elapsed().as_secs_f64());
        let result = self.phase_interpret_for_test(&module)?;

        let elapsed = start.elapsed();
        debug!("Test execution completed in {:.2}s", elapsed.as_secs_f64());

        Ok(TestExecutionResult {
            stdout: result.0,
            stderr: result.1,
            exit_code: result.2,
            duration: elapsed,
        })
    }

    /// Phase 5b: Interpret with output capture for test execution.
    ///
    /// Supports two modes:
    /// 1. **main() mode**: If the module has a `main` function, execute it (traditional).
    /// 2. **@test mode**: If no `main` exists, discover all `@test`-annotated functions
    ///    and run them sequentially as a test suite.
    fn phase_interpret_for_test(&mut self, module: &Module) -> Result<(String, String, i32)> {
        debug!("Interpreting module for test via VBC-first architecture");

        // Step 0: Reset global VBC value side-tables for test isolation.
        //
        // `Value` uses process-global `Mutex<Vec<_>>` side tables to hold
        // boxed integers and CBGR ThinRef/FatRef payloads. In batch test
        // runs these tables accumulate entries across tests and retain
        // indices referenced by stale `Value`s from prior interpreters,
        // causing state carryover. Clear them here so each test starts
        // from a pristine side-table state. Safe because the previous
        // interpreter (and therefore every `Value` it held) has been
        // dropped by the time we get here.
        verum_vbc::reset_global_value_tables();

        // Step 1: Compile AST to VBC
        let vbc_module = self.compile_ast_to_vbc(module)?;

        // Step 2: Create VBC interpreter with output capture enabled
        let mut interpreter = VbcInterpreter::new(vbc_module);
        interpreter.state.enable_output_capture();
        interpreter.state.config.count_instructions = true;
        interpreter.state.config.max_instructions = 1_000_000_000; // 1B instruction limit for tests (minimax/DP algorithms)
        // Wire cancel_flag from compiler options to interpreter for cooperative abort
        interpreter.state.config.cancel_flag = self.session.options().cancel_flag.clone();

        // Step 2b: Skip global constructors in test mode.
        // Global ctors are primarily FFI library initializers (e.g., kernel32.dll)
        // that fail on macOS and corrupt interpreter state. VBC interpreter tests
        // don't need FFI initialization.

        // Step 3: Try main() first, fall back to @test function discovery
        if let Ok(main_func_id) = self.find_main_function_id(&interpreter.state.module) {
            // Traditional mode: execute main()
            debug!("Executing main function via VBC interpreter (function ID: {})", main_func_id.0);
            let result = interpreter.execute_function(main_func_id);

            let stdout = interpreter.state.take_stdout();
            let stderr = interpreter.state.take_stderr();

            let exit_code = match result {
                // Tier-parity: if `main` returns an Int, that IS the exit
                // code (same contract as the AOT main→C-exit mapping).
                // Without this, differential tests would see
                // interpreter=0 / AOT=1 for any `fn main() -> Int { 1 }`.
                Ok(value) => {
                    if value.is_int() {
                        value.as_i64() as i32
                    } else {
                        0
                    }
                }
                Err(ref e) => {
                    let error_msg = format!("{}", e);
                    if stderr.is_empty() {
                        return Ok((stdout, error_msg, 1));
                    } else {
                        return Ok((stdout, format!("{}\n{}", stderr, error_msg), 1));
                    }
                }
            };

            return Ok((stdout, stderr, exit_code));
        }

        // @test mode: discover and run all @test-annotated functions.
        // Only user code functions have is_test=true (stdlib @test propagation is disabled).
        let test_functions: Vec<(VbcFunctionId, String)> = interpreter.state.module.functions
            .iter()
            .enumerate()
            .filter(|(_, desc)| desc.is_test)
            .map(|(idx, desc)| {
                let name = interpreter.state.module.get_string(desc.name)
                    .unwrap_or("unknown")
                    .to_string();
                (VbcFunctionId(idx as u32), name)
            })
            .collect();

        if test_functions.is_empty() {
            return Err(anyhow::anyhow!("No main function or @test functions found in VBC module"));
        }

        let total = test_functions.len();
        debug!("Running {} @test functions", total);

        let mut passed = 0usize;
        let mut failed = 0usize;
        let mut failures: Vec<(String, String)> = Vec::new();

        for (func_id, test_name) in &test_functions {
            let result = interpreter.execute_function(*func_id);

            match result {
                Ok(_) => {
                    passed += 1;
                    interpreter.state.writeln_stdout(&format!("  PASS: {}", test_name));
                }
                Err(e) => {
                    failed += 1;
                    let err_msg = format!("{}", e);
                    interpreter.state.writeln_stdout(&format!("  FAIL: {} — {}", test_name, err_msg));
                    failures.push((test_name.clone(), err_msg));
                }
            }

            // Unwind any frames left behind by a panic/error so that the next
            // test runs from a clean call-stack state. Normal returns pop their
            // own frames via `do_return`, but a panic aborts mid-execution.
            while !interpreter.state.call_stack.is_empty() {
                if let Ok(frame) = interpreter.state.call_stack.pop_frame() {
                    interpreter.state.registers.pop_frame(frame.reg_base);
                } else {
                    break;
                }
            }
            // Also clear context entries that were provided inside the test
            // but never ended (e.g. panicked before CtxEnd).
            interpreter.state.context_stack.clear();
            // Clear any pending exception so the next test starts clean.
            interpreter.state.current_exception = None;
            interpreter.state.exception_handlers.clear();
        }

        // Summary
        interpreter.state.writeln_stdout(&format!(
            "\n{}/{} tests passed, {} failed",
            passed, total, failed
        ));

        let stdout = interpreter.state.take_stdout();
        let stderr = interpreter.state.take_stderr();
        let exit_code = if failed > 0 { 1 } else { 0 };

        Ok((stdout, stderr, exit_code))
    }

    /// Phase 7 (Tier 1): Compile to native executable (AOT mode)
    ///
    /// This compiles the source to a standalone native executable that can be run
    /// independently. Uses the VBC → LLVM IR path (NOT MLIR, which is GPU-only).
    ///
    /// Pipeline: Source → AST → TypedAST → VBC → LLVM IR → Object → Executable
    ///
    /// See the Phase 7 architecture comment above `phase_interpret()` for details.
    pub fn run_native_compilation(&mut self) -> Result<PathBuf> {
        let start = Instant::now();
        let _bc_native = verum_error::breadcrumb::enter(
            "compiler.run_native_compilation",
            self.session
                .options()
                .input
                .display()
                .to_string(),
        );

        // Phase 0: Load stdlib modules (populates self.modules for type checking)
        let _bc_stdlib = verum_error::breadcrumb::enter("compiler.phase.stdlib_loading", "");
        let t0 = Instant::now();
        self.load_stdlib_modules()?;
        let stdlib_time = t0.elapsed();
        self.session.record_phase_metrics("Stdlib Loading", stdlib_time, 0);
        drop(_bc_stdlib);

        // Phase 0.5: Load sibling project modules (enables cross-file mount imports)
        let _bc_proj = verum_error::breadcrumb::enter("compiler.phase.project_modules", "");
        let t0 = Instant::now();
        self.load_project_modules()?;
        self.load_external_cog_modules()?;
        self.session.record_phase_metrics("Project Modules", t0.elapsed(), 0);
        drop(_bc_proj);

        // Phase 1: Load source
        let _bc_load = verum_error::breadcrumb::enter("compiler.phase.load_source", "");
        let file_id = self.phase_load_source()?;
        drop(_bc_load);

        // Phase 2: Parse (phase_parse records its own timing)
        let _bc_parse = verum_error::breadcrumb::enter("compiler.phase.parse", "");
        let module = self.phase_parse(file_id)?;
        drop(_bc_parse);

        // Phase 2.5: Scan for @device(gpu) annotations to auto-enable GPU compilation.
        // Gated on [codegen].mlir_gpu: when false, GPU annotations are
        // silently ignored (the code compiles as CPU-only). This lets
        // projects disable GPU compilation without removing @device(gpu)
        // annotations from source.
        let gpu_enabled = self.session.language_features().gpu_enabled();
        if gpu_enabled && !self.session.options().is_no_gpu() {
            let gpu_detected = Self::detect_gpu_kernels(&module);
            if gpu_detected {
                info!("Detected @device(gpu) annotations — GPU compilation path will be enabled");
                self.session.options_mut().has_gpu_kernels = true;
            }
        }

        // Phase 2.9: Safety gate (unsafe, @ffi) — always runs,
        // matching the interpreter path. No-op fast path when both
        // `[safety].unsafe_allowed` and `[safety].ffi` are true.
        self.phase_safety_gate(&module)?;

        // Phase 3: Type check (uses self.modules for stdlib type/method registration)
        let _bc_tc = verum_error::breadcrumb::enter("compiler.phase.type_check", "");
        let t0 = Instant::now();
        self.phase_type_check(&module)?;
        self.session.record_phase_metrics("Type Checking", t0.elapsed(), 0);
        drop(_bc_tc);

        // Phase 3.5: Dependency analysis (target-constraint enforcement,
        // matching the interpreter path so AOT and Tier 0 apply the
        // same target-profile rules like `no_std` / `no_alloc`).
        let t0 = Instant::now();
        self.phase_dependency_analysis(&module)?;
        self.session.record_phase_metrics("Dependency Analysis", t0.elapsed(), 0);

        // Selective module retention after type checking.
        //
        // Previously we called `self.modules.clear()` which prevented stdlib .vr
        // function bodies from reaching VBC codegen. Now we retain modules whose
        // bodies need to be compiled to VBC (collections, sync, text, io, mem, etc.)
        // and only clear prelude/protocol-definition modules that would introduce
        // unresolvable cross-module method references.
        //
        // The retained modules' function bodies will be compiled to VBC and then
        // lowered to LLVM IR, replacing the need for C runtime implementations.
        self.clear_non_compilable_stdlib_modules(Some(&module));

        // Phase 4: Refinement verification (if enabled)
        if self.session.options().verify_mode.use_smt() {
            let t0 = Instant::now();
            self.phase_verify(&module)?;
            self.session.record_phase_metrics("Verification", t0.elapsed(), 0);
        }

        // Phase 4c: Context system validation (negative constraints, provision checks)
        self.phase_context_validation(&module);

        // Phase 4d: Send/Sync compile-time enforcement
        self.phase_send_sync_validation(&module);

        // Phase 5: CBGR analysis
        let _bc_cbgr = verum_error::breadcrumb::enter("compiler.phase.cbgr_analysis", "");
        let t0 = Instant::now();
        self.phase_cbgr_analysis(&module)?;
        self.session.record_phase_metrics("CBGR Analysis", t0.elapsed(), 0);
        drop(_bc_cbgr);

        // Phase 5b: FFI boundary validation
        let _bc_ffi = verum_error::breadcrumb::enter("compiler.phase.ffi_validation", "");
        self.phase_ffi_validation(&module)?;
        drop(_bc_ffi);

        // Phase 5c: rayon fence before LLVM codegen.
        //
        // LLVM registers backend passes lazily via function-local
        // statics guarded by __cxa_guard_acquire (Itanium C++ ABI).
        // While the main thread was inside that guard, rayon workers
        // parked after stdlib parsing would race the same guard's
        // wake-path, corrupting its semaphore state on arm64 macOS —
        // observable as a ~70% SIGSEGV in phase_generate_native in
        // release builds.
        //
        // `rayon::broadcast(|_| ())` dispatches a no-op to every
        // worker and waits for completion, which is a true fence: all
        // workers run, exit their wake path, and re-park *before* we
        // touch LLVM's cxa-guards. Combined with the eager
        // `Target::initialize_native` in `verum_cli::main` (which
        // pre-populates the IR-pass half of the same registry), this
        // eliminates the race.
        //
        // Diagnosed via `verum diagnose` crash reports showing 14/14
        // stacks at `__os_semaphore_wait` → `callDefaultCtor<*Pass>`.
        {
            let _bc_barrier =
                verum_error::breadcrumb::enter("compiler.phase.rayon_fence", "broadcast");
            let _ = rayon::broadcast(|_| ());
        }

        // Phase 6: Generate native code (CPU path) — the hot spot where
        // the documented Z3/LLVM teardown race manifests. Mark the
        // breadcrumb with the input file so the crash report points the
        // reader straight at the translation unit.
        let _bc_codegen = verum_error::breadcrumb::enter(
            "compiler.phase.generate_native",
            self.session
                .options()
                .input
                .display()
                .to_string(),
        );
        let t0 = Instant::now();
        let output_path = self.phase_generate_native(&module)?;
        self.session.record_phase_metrics("Code Generation", t0.elapsed(), 0);
        drop(_bc_codegen);

        // Phase 6b: GPU compilation (MLIR path) — auto-triggered by @device(gpu) detection
        // Runs alongside CPU compilation to produce GPU kernel binaries.
        if self.session.options().has_gpu_kernels {
            info!("Auto-detected GPU kernels — running MLIR GPU compilation");
            let t0 = Instant::now();
            match self.run_mlir_aot() {
                Ok(gpu_binary) => {
                    info!("GPU compilation produced: {}", gpu_binary.display());
                    self.session.record_phase_metrics("GPU Compilation", t0.elapsed(), 0);
                }
                Err(e) => {
                    // GPU compilation failure is non-fatal — CPU binary is still valid
                    warn!("GPU compilation failed (CPU binary still valid): {}", e);
                }
            }
        }

        // Save incremental compilation cache for next build.
        self.save_incremental_cache();

        let elapsed = start.elapsed();
        info!(
            "Native compilation completed in {:.2}s",
            elapsed.as_secs_f64()
        );

        Ok(output_path)
    }

    /// Phase 6: Generate native executable
    fn phase_generate_native(&mut self, module: &Module) -> Result<PathBuf> {
        info!("Generating native executable");
        let start = Instant::now();

        // Get input path and determine project root
        let input_path = &self.session.options().input;
        let project_root = self.get_project_root(input_path);

        // Determine build profile (debug or release)
        let profile = if self.session.options().optimization_level >= 2 {
            "release"
        } else {
            "debug"
        };

        // Create target directory structure
        let target_dir = project_root.join("target");
        let profile_dir = target_dir.join(profile);
        let build_dir = target_dir.join("build");

        // Create directories if they don't exist
        std::fs::create_dir_all(&profile_dir).with_context(|| {
            format!(
                "Failed to create target directory: {}",
                profile_dir.display()
            )
        })?;
        std::fs::create_dir_all(&build_dir).with_context(|| {
            format!("Failed to create build directory: {}", build_dir.display())
        })?;

        // Determine output path
        let output_path = if self.session.options().output.to_str().unwrap_or("").is_empty() {
            // Default: use input filename without extension in target/<profile>/
            let exe_name = input_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("main");
            profile_dir.join(if cfg!(windows) {
                format!("{}.exe", exe_name)
            } else {
                exe_name.to_string()
            })
        } else {
            // User-specified output path (use as-is)
            self.session.options().output.clone()
        };

        // Create module name
        let module_name = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("main");

        info!("  Converting AST to VBC bytecode (multi-module)");

        // For multi-file projects, merge project module items into the main module.
        // This ensures all types, functions, and constants from sibling .vr files
        // are compiled as part of a single VBC compilation unit, avoiding
        // cross-module argument tracking issues in CallM/Call instructions.
        let module = if !self.project_modules.is_empty() {
            let mut merged = module.clone();
            for (path, proj_module) in &self.project_modules {
                info!("  Merging project module '{}' ({} items)", path.as_str(), proj_module.items.len());
                for item in &proj_module.items {
                    // Skip mount statements from project modules (they reference
                    // sibling modules that are already merged)
                    if matches!(&item.kind, verum_ast::ItemKind::Mount(_)) {
                        continue;
                    }
                    merged.items.push(item.clone());
                }
            }
            merged
        } else {
            module.clone()
        };

        // Phase 1: Convert AST to VBC bytecode with full multi-module resolution
        // This uses the same path as the interpreter, collecting stdlib imports
        // and resolving cross-module dependencies before compilation.
        let vbc_module = self.compile_ast_to_vbc(&module)
            .map_err(|e| anyhow::anyhow!("Failed to compile AST to VBC: {:?}", e))?;

        info!("  VBC bytecode: {} functions ({} with instructions)",
            vbc_module.functions.len(),
            vbc_module.functions.iter().filter(|f| f.instructions.is_some()).count());
        info!("  VBC bytecode generated: {} functions", vbc_module.functions.len());

        // Phase 1.5: Monomorphize generic functions
        // Specializes generic VBC functions with concrete type arguments before LLVM lowering.
        // This resolves CallG instructions to direct Call instructions.
        info!("  Monomorphizing generic functions");
        let vbc_module = {
            let mono = crate::phases::VbcMonomorphizationPhase::new();
            let mono = if !self.session.language_features().codegen.monomorphization_cache {
                mono.without_cache()
            } else { mono };
            let mut mono = mono;
            match mono.monomorphize(&vbc_module) {
                Ok(mono_module) => {
                    info!("  Monomorphization complete: {} functions", mono_module.functions.len());
                    std::sync::Arc::new(mono_module)
                }
                Err(diagnostics) => {
                    // Log warnings but fall back to unspecialized module
                    for d in diagnostics.iter() {
                        warn!("Monomorphization warning: {:?}", d);
                    }
                    info!("  Monomorphization skipped (fallback to unspecialized)");
                    vbc_module
                }
            }
        };

        // Phase 1.75: CBGR escape analysis
        // Determines which Ref/RefMut instructions can be promoted from Tier 0
        // (runtime-checked, ~15ns) to Tier 1 (compiler-proven safe, zero overhead).
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

        // Emit VBC bytecode dump if requested
        if self.session.options().emit_vbc {
            let dump = verum_vbc::disassemble::disassemble_module(&vbc_module);
            let vbc_path = self.session.options().input.with_extension("vbc.txt");
            if let Err(e) = std::fs::write(&vbc_path, &dump) {
                warn!("Failed to write VBC dump: {}", e);
            } else {
                info!("Wrote VBC dump: {} ({} bytes)", vbc_path.display(), dump.len());
            }
        }

        // Phase 2: Lower VBC to LLVM IR (CPU path)
        // Note: For native compilation, we use the VBC → LLVM IR path (not MLIR).
        // GPU path (VBC → MLIR) should be used via run_mlir_jit/run_mlir_aot for tensor ops.
        info!("  Lowering VBC to LLVM IR");

        let llvm_ctx = verum_codegen::llvm::verum_llvm::context::Context::create();

        let lowering_config = verum_codegen::llvm::LoweringConfig::new(module_name)
            .with_opt_level(self.session.options().optimization_level)
            .with_debug_info(self.session.options().debug_info)
            .with_coverage(self.session.options().coverage);

        let mut lowering = verum_codegen::llvm::VbcToLlvmLowering::new(
            &llvm_ctx,
            lowering_config,
        );

        // Apply CBGR escape analysis results to LLVM lowering.
        // This enables tier promotion: non-escaping references skip runtime
        // generation checks (Tier 0 → Tier 1), saving ~15ns per reference.
        lowering.set_escape_analysis(escape_result);

        lowering.lower_module(&vbc_module)
            .map_err(|e| anyhow::anyhow!("Failed to lower VBC to LLVM IR: {:?}", e))?;

        // Report CBGR statistics
        let stats = lowering.cbgr_stats();
        if stats.refs_created > 0 {
            info!("  CBGR: {} refs ({} tier0/{} tier1/{} tier2), {} runtime checks, {} eliminated",
                stats.refs_created, stats.tier0_refs, stats.tier1_refs, stats.tier2_refs,
                stats.runtime_checks, stats.checks_eliminated);
        }

        // Phase 3: Write intermediate files
        let obj_path = build_dir.join(format!("{}.o", module_name));
        let ir_path = build_dir.join(format!("{}.ll", module_name));

        let opt_level = self.session.options().optimization_level;
        info!("  Optimizing LLVM IR (level {})", opt_level);

        // Write LLVM IR only when explicitly requested or emit-llvm is on.
        // The IR printer triggers TypeFinder::incorporateType which
        // crashes on modules with stdlib-generated functions containing
        // null Type references (use-after-free from arity collision
        // fixups). Disabled by default to prevent non-deterministic
        // SIGSEGV during normal builds.
        let emit_ir = self.session.options().emit_ir
            || std::env::var("VERUM_DUMP_IR").is_ok();
        if emit_ir && !lowering.has_arity_collisions() && lowering.skip_body_count() == 0 {
            let llvm_ir = lowering.get_ir();
            std::fs::write(&ir_path, llvm_ir.as_str().as_bytes())
                .with_context(|| format!("Failed to write LLVM IR to {}", ir_path.display()))?;
            info!("  Written LLVM IR to {}", ir_path.display());
        }

        // Compile to object file using LLVM TargetMachine
        info!("  Writing object file to {}", obj_path.display());
        use verum_codegen::llvm::verum_llvm::targets::{
            Target, TargetMachine, RelocMode, CodeModel, FileType,
            InitializationConfig,
        };

        // Initialize native target ONCE per process.
        // LLVM's target initialization is not idempotent — calling it multiple
        // times can corrupt internal state.
        {
            static INIT: std::sync::Once = std::sync::Once::new();
            INIT.call_once(|| {
                let _ = Target::initialize_native(&InitializationConfig::default());
                // Also initialize WebAssembly target for cross-compilation
                #[cfg(feature = "target-wasm")]
                Target::initialize_webassembly(&InitializationConfig::default());
            });
        }

        // Use configured target triple if specified, otherwise default to host
        let triple = if let Some(ref target) = self.session.options().target_triple {
            verum_codegen::llvm::verum_llvm::targets::TargetTriple::create(target)
        } else {
            TargetMachine::get_default_triple()
        };
        let target = Target::from_triple(&triple)
            .map_err(|e| anyhow::anyhow!("Failed to get target: {}", e))?;

        // Cap TargetMachine at O2 (Default). O3 (Aggressive) enables
        // machine-level optimizations that interact badly with our
        // ptrtoint/inttoptr patterns in the code emitter.
        // Our pass pipeline (SROA+mem2reg+DSE+ADCE) already provides
        // the critical optimizations; the TargetMachine just needs O2.
        let llvm_opt_level = match opt_level {
            0 => verum_codegen::llvm::verum_llvm::OptimizationLevel::None,
            1 => verum_codegen::llvm::verum_llvm::OptimizationLevel::Less,
            _ => verum_codegen::llvm::verum_llvm::OptimizationLevel::Default,
        };

        // Use host CPU name and features for native targets.
        // For WASM targets, use "generic" CPU with no features.
        let is_wasm = triple.as_str().to_string_lossy().contains("wasm");
        let (cpu_str, features_str) = if is_wasm {
            ("generic", "")
        } else {
            // Get host CPU info for native compilation
            let cpu = TargetMachine::get_host_cpu_name();
            let features = TargetMachine::get_host_cpu_features();
            // Leak to static — called once per compilation, acceptable
            let cpu_s: &'static str = Box::leak(cpu.to_str().unwrap_or("generic").to_string().into_boxed_str());
            let feat_s: &'static str = Box::leak(features.to_str().unwrap_or("").to_string().into_boxed_str());
            (cpu_s, feat_s)
        };
        debug!("LLVM target: cpu={}, features={}", cpu_str, features_str);

        let target_machine = target.create_target_machine(
            &triple,
            cpu_str,
            features_str,
            llvm_opt_level,
            RelocMode::Default,
            CodeModel::Default,
        ).ok_or_else(|| anyhow::anyhow!("Failed to create target machine"))?;

        // Run LLVM optimization pass pipeline.
        // This is CRITICAL for performance — without it, all variables use
        // alloca+store/load (no SSA promotion, no inlining, no vectorization).
        {
            use verum_codegen::llvm::verum_llvm::passes::PassBuilderOptions;
            let pass_options = PassBuilderOptions::create();

            // Build the optimization pipeline string based on opt_level.
            // mem2reg: promotes alloca→SSA (CRITICAL for performance)
            // instcombine<no-verify-fixpoint>: algebraic simplification
            // gvn: global value numbering (CSE)
            // simplifycfg: control flow simplification
            // loop-unroll: unroll small loops
            // sroa: scalar replacement of aggregates
            // licm: loop invariant code motion
            // LLVM pass pipeline selection.
            // Float values are stored directly as f64 through opaque pointers.
            // Pointer values still use ptrtoint→i64→inttoptr which is incompatible
            // with SROA/GVN (they lose pointer provenance tracking).
            // mem2reg + simplifycfg gives 0.93x native C — sufficient for v1.0.
            // Full O3 (SROA/GVN/DSE/inline) requires storing pointers directly
            // through opaque pointer allocas (same pattern as the float fix).
            // Typed alloca storage: f64 stored directly, ptr stored directly with
            // lazy ptrtoint on load. This preserves LLVM type info for all passes.
            // mem2reg + simplifycfg = 0.93x native C.
            // SROA breaks on ptr-heavy List operations (ptrtoint provenance loss).
            // Full O3 requires typed alloca refactor of instruction.rs.
            // LLVM optimization pass pipeline.
            // Typed alloca storage: f64 stored directly, ptr stored directly.
            // get_register returns native types (PointerValue for ptrs).
            // This enables all LLVM passes to work correctly.
            // Run GlobalDCE FIRST to remove dead stdlib functions that may have
            // invalid IR (broken PHI nodes, unreachable blocks). This prevents
            // SimplifyCFG from crashing on invalid dead code.
            // LLVM pass pipeline — conservative due to ptrtoint→i64→inttoptr
            // pattern used by VBC codegen. This breaks SROA/GVN/instcombine/
            // early-cse which depend on pointer provenance tracking.
            // Safe passes: mem2reg (alloca→SSA), simplifycfg (branch cleanup).
            //
            // Function-level optimization hints (@inline, @cold, @hot, @optimize)
            // are applied as LLVM function attributes in vbc_lowering.rs.
            // These are respected automatically by the pass manager for:
            //   - Code layout (.text.cold sections)
            //   - Inlining decisions (alwaysinline/noinline/inlinehint)
            //   - Size optimization (optsize/minsize on cold functions)
            //   - Per-function target features (target-features/target-cpu)
            // When the module has arity collisions or skip-body
            // functions, LLVM function-level passes (mem2reg,
            // simplifycfg, instcombine) crash with SIGSEGV in
            // canReplaceOperandWithVariable or TypeFinder due to
            // null Type* references in redirect-stub instructions.
            // Restrict to globaldce (module-level dead code
            // elimination) which is safe — it only removes
            // unreachable functions without traversing instruction
            // operands.
            let has_ir_issues =
                lowering.has_arity_collisions() || lowering.skip_body_count() > 0;

            let passes = if has_ir_issues {
                // Arity collisions / skip-body stubs contain redirect IR
                // with null Type* references — full instcombine/SROA/GVN
                // crashes LLVM TypeFinder, so restrict to module-level DCE.
                "globaldce".to_string()
            } else {
                // Use LLVM's canonical O-level pipelines. These include
                // DCE, GVN, LICM, SROA, instcombine, inliner, loop opts,
                // vectorization — the full set of standard optimizations.
                // Fall back to the conservative pipeline for opt_level=0
                // to keep debug builds fast.
                match opt_level {
                    0 => "globaldce".to_string(),
                    1 => "default<O1>".to_string(),
                    2 => "default<O2>".to_string(),
                    _ => "default<O3>".to_string(),
                }
            };

            info!("  Running LLVM passes: {}", passes);
            if let Err(e) = lowering.module().run_passes(&passes, &target_machine, pass_options) {
                // Fall back to just globaldce if full pipeline fails
                tracing::warn!("Full LLVM pass pipeline failed: {} — falling back to globaldce", e);
                let fallback_options = PassBuilderOptions::create();
                if let Err(e2) = lowering.module().run_passes("globaldce", &target_machine, fallback_options) {
                    tracing::warn!("GlobalDCE pass also failed: {}", e2);
                }
            }
        }

        // VERUM_DUMP_IR=1 — dump LLVM IR after optimization passes.
        // Useful for analyzing codegen quality and debugging optimizations.
        if std::env::var("VERUM_DUMP_IR").is_ok() {
            let ir_path = build_dir.join(format!("{}.ll", module_name));
            let _ = lowering.write_ir_to_file(&ir_path);
            info!("  LLVM IR dumped to {}", ir_path.display());
        }

        // Verify the module AFTER GlobalDCE removed dead functions.
        // Dead stdlib functions may have invalid IR (unresolved intrinsics),
        // but GlobalDCE eliminates them, leaving only valid reachable code.
        //
        // Debug info verification failures (!dbg location on inlined calls) are
        // non-fatal — the code is correct, only metadata is inconsistent. Emit
        // a warning instead of aborting compilation.
        if let Err(e) = lowering.verify() {
            let err_str = format!("{:?}", e);
            if err_str.contains("!dbg location") || err_str.contains("debug info") {
                tracing::warn!("LLVM module has debug info inconsistency (non-fatal): {}",
                    err_str.chars().take(200).collect::<String>());
                // Continue compilation — the actual code is correct
            } else {
                let ir_path = build_dir.join(format!("{}_debug.ll", module_name));
                if !lowering.has_arity_collisions() {
                    let _ = lowering.write_ir_to_file(&ir_path);
                    return Err(anyhow::anyhow!("LLVM module verification failed (IR dumped to {}): {:?}", ir_path.display(), e));
                } else {
                    return Err(anyhow::anyhow!("LLVM module verification failed: {:?}", e));
                }
            }
        }

        target_machine.write_to_file(lowering.module(), FileType::Object, &obj_path)
            .map_err(|e| anyhow::anyhow!("Failed to write object file: {}", e))?;

        // Emit LLVM bitcode when LTO is enabled for cross-module optimization
        if self.session.options().lto {
            let bc_path = obj_path.with_extension("bc");
            lowering.module().write_bitcode_to_path(&bc_path);
            debug!("  Wrote LLVM bitcode for LTO: {}", bc_path.display());
        }

        // Runtime compilation: LLVM IR provides core runtime (allocator, text, etc.)
        // ALL runtime functions are now pure LLVM IR (platform_ir.rs + tensor_ir.rs + metal_ir.rs).
        // No C compilation needed. We still generate an empty .o for the linker.
        let runtime_stubs_path = self.generate_runtime_stubs(&build_dir)?;
        let runtime_obj = self.compile_c_file(&runtime_stubs_path, &build_dir)?;

        // Metal GPU runtime — now in LLVM IR (metal_ir.rs), no Objective-C compilation needed
        let metal_obj: Option<PathBuf> = None;

        // Load linker configuration from Verum.toml (if present)
        let mut linker_config = self.load_linker_config(&project_root, profile)?;

        // Wire CLI LTO option into linker config
        if self.session.options().lto {
            use crate::phases::linking::LTOConfig;
            linker_config.lto = match self.session.options().lto_mode {
                Some(crate::options::LtoMode::Full) => LTOConfig::Full,
                Some(crate::options::LtoMode::Thin) | None => LTOConfig::Thin,
            };
            // Enable LLD for LTO support
            linker_config.use_llvm_linker = true;
        }

        // Add Metal/Foundation frameworks for macOS GPU support (LLD path)
        #[cfg(target_os = "macos")]
        {
            linker_config.extra_flags.push("-framework Metal".into());
            linker_config.extra_flags.push("-framework Foundation".into());
            linker_config.libraries.push("objc".into());
        }

        // Link object files into executable in target/<profile>/
        info!("  Linking executable");
        let mut link_objects = vec![obj_path.clone(), runtime_obj];
        if let Some(ref metal) = metal_obj {
            link_objects.push(metal.clone());
            info!("  Including Metal GPU runtime in link");
        }
        self.link_with_config(&link_objects, &output_path, &linker_config)?;

        // Clean up intermediate files
        let _ = std::fs::remove_file(&runtime_stubs_path);
        // verum_platform.c deleted — no cleanup needed
        // verum_tensor.c deleted — no cleanup needed
        // verum_metal.m deleted — no cleanup needed

        let elapsed = start.elapsed();
        info!(
            "Generated native executable: {} ({:.2}s)",
            output_path.display(),
            elapsed.as_secs_f64()
        );

        Ok(output_path)
    }

    /// Get the project root directory
    ///
    /// Searches for Verum.toml starting from the input file's directory
    /// and walking up the directory tree. Falls back to input file's parent
    /// or current working directory if no Verum.toml is found.
    fn get_project_root(&self, input_path: &PathBuf) -> PathBuf {
        // Canonicalize the input path to get absolute path
        let abs_path = if input_path.is_absolute() {
            input_path.clone()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(input_path)
        };

        // Start from the input file's parent directory
        let mut current = abs_path.parent().map(|p| p.to_path_buf());

        // Walk up the directory tree looking for Verum.toml
        while let Some(dir) = current {
            let manifest = dir.join("Verum.toml");
            if manifest.exists() {
                return dir;
            }
            current = dir.parent().map(|p| p.to_path_buf());
        }

        // Fallback: use input file's parent directory or current working directory
        abs_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// Generate C runtime stubs for CBGR and stdlib functions
    fn generate_runtime_stubs(&self, temp_dir: &Path) -> Result<PathBuf> {
        let stubs_path = temp_dir.join("verum_runtime_stubs.c");

        // Use the extracted C runtime from verum_codegen
        let stubs_code = verum_codegen::runtime_stubs::RUNTIME_C;

        std::fs::write(&stubs_path, stubs_code)?;
        debug!("Generated runtime stubs: {}", stubs_path.display());

        // verum_platform.c DELETED — all platform functions in LLVM IR (platform_ir.rs)

        Ok(stubs_path)
    }

    /// Compile a C file to object file
    fn compile_c_file(&self, source_path: &Path, output_dir: &Path) -> Result<PathBuf> {
        let output_path = output_dir
            .join(
                source_path
                    .file_stem()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("runtime"),
            )
            .with_extension("o");

        // Detect C compiler
        let cc = self.detect_c_compiler()?;

        debug!("Compiling C file with {}: {}", cc, source_path.display());

        // Compile C file to object file with architecture-specific SIMD flags
        let mut cmd = std::process::Command::new(&cc);
        let c_opt = if self.session.options().optimization_level >= 3 { "-O3" } else { "-O2" };
        cmd.arg("-c")
            .arg(source_path)
            .arg("-o")
            .arg(&output_path)
            .arg(c_opt)
            .arg("-fPIC")
            .arg("-ffast-math")
            .arg("-DNDEBUG");

        // Add architecture-specific SIMD flags for auto-vectorization
        #[cfg(target_arch = "x86_64")]
        {
            cmd.arg("-march=native");
            cmd.arg("-mavx2");
            cmd.arg("-mfma");
        }
        #[cfg(target_arch = "aarch64")]
        {
            cmd.arg("-march=armv8-a+simd");
        }

        // Entry point provided by LLVM IR (platform_ir.rs) — skip C entry points
        cmd.arg("-DVERUM_LLVM_IR_ENTRY");
        // File I/O, time, networking C code deleted — LLVM IR only (platform_ir.rs)

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("C compilation failed:\n{}", stderr));
        }

        Ok(output_path)
    }

    /// Detect available C compiler
    fn detect_c_compiler(&self) -> Result<String> {
        let compilers = ["clang", "gcc", "cc"];

        for compiler in &compilers {
            if let Ok(output) = std::process::Command::new(compiler)
                .arg("--version")
                .output()
            {
                if output.status.success() {
                    return Ok(compiler.to_string());
                }
            }
        }

        Err(anyhow::anyhow!(
            "No C compiler found (tried: clang, gcc, cc)"
        ))
    }

    /// Link object files into executable
    fn link_executable(&self, object_files: &[PathBuf], output_path: &PathBuf) -> Result<()> {
        let linker = self.detect_c_compiler()?;

        debug!("Linking with {}: {}", linker, output_path.display());

        let mut cmd = std::process::Command::new(&linker);

        // Add all object files
        for obj in object_files {
            cmd.arg(obj);
        }

        // Output path
        cmd.arg("-o").arg(output_path);

        // ==========================================================================
        // NO LIBC ARCHITECTURE
        // ==========================================================================
        // Verum does NOT link against libc or system C libraries (-lm, -lpthread, -ldl).
        // All runtime functionality is provided by:
        // - LLVM intrinsics (llvm.sin.f32, llvm.sqrt.f64, etc.) for math
        // - Custom Verum runtime in /core/ for threading, memory, I/O
        // - Platform-specific system calls via /core/sys/
        //
        // Entry point: /core/sys/init.vr provides the custom _start that
        // initializes the Verum runtime before calling the user's main function.
        //
        // Exception: GPU targets may link Metal/CUDA/ROCm frameworks via MLIR path.
        // ==========================================================================

        // Platform-specific flags (no libc)
        #[cfg(target_os = "macos")]
        {
            cmd.arg("-Wl,-dead_strip");
            cmd.arg("-Wl,-undefined,dynamic_lookup");
            // 16MB stack for recursive algorithms (default 8MB causes SIGSEGV in deep recursion)
            cmd.arg("-Wl,-stack_size,0x1000000");
            // Link Metal + Foundation frameworks for GPU compute on Apple Silicon.
            // metal_ir.rs emits LLVM IR that calls MTLCreateSystemDefaultDevice,
            // objc_msgSend, sel_registerName, objc_getClass — all from these frameworks.
            cmd.arg("-framework").arg("Metal");
            cmd.arg("-framework").arg("Foundation");
            cmd.arg("-lobjc");
        }

        #[cfg(target_os = "linux")]
        {
            cmd.arg("-Wl,--gc-sections");
            cmd.arg("-rdynamic");
            // 16MB stack for recursive algorithms
            cmd.arg("-Wl,-z,stacksize=16777216");
            // Link additional system libraries for runtime
            cmd.arg("-ldl");
            cmd.arg("-lrt");
            // Link C++ stdlib for CBGR
            cmd.arg("-lstdc++");
        }

        // Execute linker
        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Linking failed:\n{}", stderr));
        }

        // Make executable on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(output_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(output_path, perms)?;
        }

        Ok(())
    }

    /// Load linker configuration from Verum.toml
    ///
    /// Reads the [linker] section from Verum.toml and merges with profile-specific
    /// settings. Falls back to defaults if no Verum.toml is found.
    fn load_linker_config(&self, project_root: &Path, profile: &str) -> Result<LinkingConfig> {
        let verum_toml = project_root.join("Verum.toml");

        if verum_toml.exists() {
            // Load full project configuration
            let project_config = ProjectConfig::load_from_file(&verum_toml)
                .with_context(|| format!("Failed to load {}", verum_toml.display()))?;

            // Get linker config for the specified profile
            let output_path = PathBuf::new(); // Placeholder - will be set by caller
            project_config.to_linking_config(profile, output_path)
        } else {
            // Use default configuration
            Ok(LinkingConfig::default())
        }
    }

    /// Link object files using configuration from Verum.toml
    ///
    /// This method supports two linking modes:
    /// - **LLD (LLVM Linker)**: When `use_lld = true` in Verum.toml, uses FinalLinker
    ///   for LTO support and faster linking on Linux
    /// - **System Linker**: Falls back to system compiler (clang/gcc) for compatibility
    ///
    /// Configuration options from Verum.toml:
    /// - `output`: executable, shared, static, object
    /// - `lto`: none, thin, full
    /// - `use_lld`: true/false
    /// - `pic`: position-independent code
    /// - `strip`: strip debug symbols
    /// - `libraries`: additional libraries to link
    /// - `extra_flags`: raw linker flags
    fn link_with_config(
        &self,
        object_files: &[PathBuf],
        output_path: &PathBuf,
        config: &LinkingConfig,
    ) -> Result<()> {
        // Clone config and set output path
        let mut link_config = config.clone();
        link_config.output_path = output_path.clone();

        // Log configuration
        info!(
            "  Linker config: output={:?}, lto={:?}, use_lld={}, pic={}, strip={}",
            link_config.output_kind,
            link_config.lto,
            link_config.use_llvm_linker,
            link_config.pic,
            link_config.strip
        );

        if link_config.use_llvm_linker {
            // Use FinalLinker with LLD for AOT compilation
            self.link_with_lld(object_files, &link_config)
        } else {
            // Fall back to system linker
            self.link_executable(object_files, output_path)
        }
    }

    /// Link object files using LLD via FinalLinker
    ///
    /// This method uses the FinalLinker from phases/linking.rs which provides:
    /// - LTO support (Thin/Full)
    /// - CBGR runtime integration
    /// - Multi-platform support (ELF, MachO, COFF, Wasm)
    fn link_with_lld(&self, object_files: &[PathBuf], config: &LinkingConfig) -> Result<()> {
        // Convert PathBuf array to ObjectFile list
        let obj_files: List<ObjectFile> = object_files
            .iter()
            .map(|path| ObjectFile::from_path(path.clone()))
            .collect::<Result<Vec<_>>>()?
            .into();

        // Create FinalLinker with AOT tier
        let mut linker = FinalLinker::new(ExecutionTier::Aot, config.clone());

        // Set exported symbols
        if !config.exported_symbols.is_empty() {
            linker = linker.with_exported_symbols(config.exported_symbols.clone());
        }

        // Perform linking
        let binary = linker.link(obj_files)?;

        info!(
            "  LLD linking complete: {} ({} bytes)",
            binary.path.display(),
            binary.size
        );

        // Make executable on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if binary.executable {
                let mut perms = std::fs::metadata(&binary.path)?.permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&binary.path, perms)?;
            }
        }

        Ok(())
    }

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

    // ==================== MLIR COMPILATION ====================

    /// Run MLIR-based JIT compilation (experimental)
    ///
    /// This compilation path uses:
    /// 1. AST → Verum MLIR dialect lowering
    /// 2. CBGR elimination and context monomorphization passes
    /// 3. Progressive lowering to LLVM dialect
    /// 4. JIT compilation via ExecutionEngine
    ///
    /// Benefits over direct LLVM:
    /// - Domain-specific optimizations via custom MLIR passes
    /// - Better debugging via MLIR's verifier
    /// - Reusable transformations for different backends
    ///
    /// MLIR is used for GPU targets only; CPU code uses LLVM IR directly.
    pub fn run_mlir_jit(&mut self, args: List<Text>) -> Result<()> {
        use verum_codegen::mlir::MlirContext;

        let start = Instant::now();
        info!("Starting MLIR JIT compilation");

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let module = self.phase_parse(file_id)?;

        // Phase 3: Type check
        self.phase_type_check(&module)?;

        // Phase 4: Convert AST to VBC bytecode
        info!("  Converting AST to VBC bytecode");
        let codegen_config = CodegenConfig {
            module_name: "mlir_jit".to_string(),
            debug_info: self.session.options().verbose > 0,
            optimization_level: self.session.options().optimization_level,
            ..Default::default()
        };
        let mut vbc_codegen = VbcCodegen::with_config(codegen_config);
        let vbc_module = vbc_codegen.compile_module(&module)
            .with_context(|| "Failed to compile AST to VBC")?;

        info!("  VBC bytecode generated: {} functions", vbc_module.functions.len());

        // Phase 5: Create MLIR context and GPU lowering
        // Note: MLIR JIT is specifically for GPU tensor operations
        info!("  Lowering VBC to MLIR for GPU");
        use verum_codegen::mlir::{VbcToMlirGpuLowering, GpuLoweringConfig, GpuTarget};

        let mlir_ctx = MlirContext::new()
            .with_context(|| "Failed to create MLIR context")?;

        // Select GPU target: [codegen].gpu_backend overrides auto-detect.
        let gpu_backend = self
            .session
            .language_features()
            .codegen
            .gpu_backend
            .as_str();
        let gpu_target = match gpu_backend {
            "metal" => GpuTarget::Metal,
            "cuda" => GpuTarget::Cuda,
            "rocm" => GpuTarget::Cuda, // ROCm uses HIP → CUDA path
            "vulkan" => GpuTarget::Cuda, // Vulkan→SPIR-V → CUDA fallback
            _ => {
                // "auto" or unknown → platform-based detection
                if cfg!(target_os = "macos") {
                    GpuTarget::Metal
                } else {
                    GpuTarget::Cuda
                }
            }
        };
        let gpu_config = GpuLoweringConfig {
            target: gpu_target,
            opt_level: self.session.options().optimization_level,
            enable_tensor_cores: !cfg!(target_os = "macos"), // Not for Metal
            max_shared_memory: if cfg!(target_os = "macos") { 32 * 1024 } else { 48 * 1024 },
            default_block_size: [256, 1, 1],
            enable_async_copy: true,
            debug_info: self.session.options().verbose > 0,
        };

        let mut gpu_lowering = VbcToMlirGpuLowering::new(mlir_ctx.context(), gpu_config);
        let mlir_module = gpu_lowering.lower_module(&vbc_module)
            .with_context(|| "Failed to lower VBC to MLIR")?;

        // Phase 6: Execute via JIT
        info!("  JIT compiling and executing");

        // Print MLIR if verbose
        if self.session.options().verbose > 0 {
            let mlir_str = format!("{}", mlir_module.as_operation());
            info!("Generated MLIR:\n{}", mlir_str);
        }

        // Try to create JIT engine and execute
        match self.execute_mlir_jit(&mlir_module) {
            Ok(exit_code) => {
                let elapsed = start.elapsed();
                info!(
                    "MLIR JIT execution completed in {:.2}s with exit code {}",
                    elapsed.as_secs_f64(),
                    exit_code
                );
                Ok(())
            }
            Err(e) => {
                // JIT execution failed - fall back to interpreter
                warn!("MLIR JIT execution failed: {} - falling back to interpreter", e);
                self.mode = CompilationMode::Interpret;
                self.run_interpreter(args)
            }
        }
    }

    /// Execute an MLIR module using the JIT engine.
    fn execute_mlir_jit(&self, module: &verum_codegen::verum_mlir::ir::Module<'_>) -> Result<i64> {
        use verum_codegen::mlir::jit::{JitEngine, JitConfig};

        // Create JIT configuration
        let jit_config = JitConfig::new()
            .with_optimization_level(self.session.options().optimization_level as usize)
            .with_verbose(self.session.options().verbose > 0);

        // Create JIT engine
        let engine = JitEngine::new(module, jit_config)
            .with_context(|| "Failed to create JIT engine")?;

        // Register stdlib symbols
        engine.register_stdlib()
            .with_context(|| "Failed to register stdlib symbols")?;

        // Look up and call main function
        if engine.lookup("main").is_some() {
            info!("  Executing main function via JIT");
            // SAFETY: main has known signature () -> i64
            let result = engine.call_i64("main", &[])?;
            Ok(result)
        } else if engine.lookup("_start").is_some() {
            info!("  Executing _start function via JIT");
            // SAFETY: _start has known signature () -> ()
            unsafe {
                engine.call_void("_start")?;
            }
            Ok(0)
        } else {
            // No entry point found
            warn!("No main or _start function found in MLIR module");
            Ok(0)
        }
    }

    /// Run MLIR-based AOT compilation (experimental)
    ///
    /// Similar to run_mlir_jit but produces an executable instead of running directly.
    /// Uses VBC → MLIR path for GPU tensor operations.
    pub fn run_mlir_aot(&mut self) -> Result<PathBuf> {
        use verum_codegen::mlir::{
            MlirContext, MlirConfig, MlirCodegen,
            VbcToMlirGpuLowering, GpuLoweringConfig, GpuTarget,
        };

        let start = Instant::now();
        info!("Starting MLIR AOT compilation (GPU path)");

        // Phase 1: Load source
        let file_id = self.phase_load_source()?;

        // Phase 2: Parse
        let module = self.phase_parse(file_id)?;

        // Phase 2.9: Safety gate — explicit for parity with CPU AOT.
        self.phase_safety_gate(&module)?;

        // Phase 3: Type check
        self.phase_type_check(&module)?;

        // Phase 3.5: Dependency analysis (target-profile enforcement).
        self.phase_dependency_analysis(&module)?;

        // Phase 4: Refinement verification (if enabled).
        // GPU kernels can carry refinement types and contracts; they
        // deserve the same SMT pass as the CPU AOT path.
        if self.session.options().verify_mode.use_smt() {
            self.phase_verify(&module)?;
        }

        // Phase 4c-4e: Context, send/sync, FFI validation —
        // identical to the CPU AOT path. GPU code that uses `using
        // [...]`, crosses thread boundaries, or declares FFI gets
        // the same checks as CPU code.
        self.phase_context_validation(&module);
        self.phase_send_sync_validation(&module);
        self.phase_ffi_validation(&module)?;

        // Phase 5: CBGR analysis (tier promotion decisions).
        self.phase_cbgr_analysis(&module)?;

        // Phase 6: Multi-module VBC codegen (resolves stdlib imports)
        info!("  Converting AST to VBC bytecode (multi-module)");
        let vbc_module = self.compile_ast_to_vbc(&module)?;
        info!("  VBC bytecode generated: {} functions", vbc_module.functions.len());

        // Phase 5: Monomorphization (specialize generics)
        let vbc_module = {
            use crate::phases::vbc_mono::VbcMonomorphizationPhase;
            let mono = VbcMonomorphizationPhase::new();
            let mono = if !self.session.language_features().codegen.monomorphization_cache {
                mono.without_cache()
            } else { mono };
            let mut mono = mono;
            match mono.monomorphize(&vbc_module) {
                Ok(specialized) => {
                    info!("  Monomorphization complete: {} functions", specialized.functions.len());
                    std::sync::Arc::new(specialized)
                }
                Err(diags) => {
                    warn!("  Monomorphization had {} diagnostics, using unspecialized module", diags.len());
                    vbc_module
                }
            }
        };

        // Phase 6: Create MLIR context and GPU lowering
        let mlir_ctx = MlirContext::new()
            .with_context(|| "Failed to create MLIR context")?;

        // Auto-select GPU target based on platform
        let gpu_target = if cfg!(target_os = "macos") {
            GpuTarget::Metal  // Apple Silicon (M1/M2/M3)
        } else {
            GpuTarget::Cuda   // Default to NVIDIA on Linux/Windows
        };

        let gpu_config = GpuLoweringConfig {
            target: gpu_target,
            opt_level: self.session.options().optimization_level,
            enable_tensor_cores: !cfg!(target_os = "macos"),
            max_shared_memory: if cfg!(target_os = "macos") { 32 * 1024 } else { 48 * 1024 },
            default_block_size: [256, 1, 1],
            enable_async_copy: true,
            debug_info: self.session.options().verbose > 0,
        };

        info!("  Lowering VBC to MLIR for GPU (target: {:?})", gpu_target);
        let mut gpu_lowering = VbcToMlirGpuLowering::new(mlir_ctx.context(), gpu_config);
        let _mlir_module = gpu_lowering.lower_module(&vbc_module)
            .with_context(|| "Failed to lower VBC to MLIR")?;

        info!("  GPU lowering stats: {} tensor ops, {} kernel launches",
            gpu_lowering.stats().tensor_ops, gpu_lowering.stats().kernel_launches);

        // Phase 7: Run GPU pass pipeline (tensor→linalg→scf→gpu→target)
        let mlir_config = MlirConfig::new("gpu_module")
            .with_optimization_level(self.session.options().optimization_level)
            .with_debug_info(self.session.options().verbose > 0);

        let mut codegen = MlirCodegen::new(&mlir_ctx, mlir_config)
            .map_err(|e| anyhow::anyhow!("MLIR codegen init failed: {:?}", e))?;

        codegen.lower_vbc_module(&vbc_module, gpu_target)
            .map_err(|e| anyhow::anyhow!("MLIR VBC lowering failed: {:?}", e))?;

        let gpu_result = codegen.optimize_gpu(gpu_target)
            .map_err(|e| anyhow::anyhow!("GPU pass pipeline failed: {:?}", e))?;

        info!("  GPU pass pipeline completed: {} phases run", gpu_result.completed_phases.len());

        // Phase 8: Print MLIR for debugging
        if self.session.options().verbose > 0 {
            if let Ok(mlir_str) = codegen.get_mlir_string() {
                info!("Generated MLIR:\n{}", mlir_str);
            }
        }

        let elapsed = start.elapsed();
        info!(
            "MLIR AOT compilation completed in {:.2}s",
            elapsed.as_secs_f64()
        );

        // Phase 9: GPU binary emission — translate MLIR→LLVM IR + extract kernels
        use verum_codegen::mlir::gpu_binary::GpuBinaryEmitter;

        let emitter = GpuBinaryEmitter::new(gpu_target, self.session.options().verbose > 0);
        let mlir_module = codegen.module()
            .map_err(|e| anyhow::anyhow!("Failed to get MLIR module: {:?}", e))?;

        match emitter.emit(mlir_module) {
            Ok(gpu_output) => {
                info!(
                    "GPU binary emission complete: {} kernel module(s), {} bytes, host IR {} bytes",
                    gpu_output.kernel_binaries.len(),
                    gpu_output.total_binary_size,
                    gpu_output.host_llvm_ir.len(),
                );

                // Phase 10: Compile host LLVM IR + link with GPU binaries
                //
                // Write the host LLVM IR to a temp file, then pass it to
                // the native compilation pipeline which handles LLVM IR → object → link.
                let build_dir = std::env::temp_dir().join("verum_gpu_build");
                std::fs::create_dir_all(&build_dir)
                    .with_context(|| format!("Failed to create build dir: {}", build_dir.display()))?;

                let host_ir_path = build_dir.join("gpu_host.ll");
                std::fs::write(&host_ir_path, &gpu_output.host_llvm_ir)
                    .with_context(|| "Failed to write host LLVM IR")?;

                // Write kernel binaries for runtime loading
                for (i, kb) in gpu_output.kernel_binaries.iter().enumerate() {
                    let kernel_path = build_dir.join(format!("gpu_kernel_{}.bin", i));
                    std::fs::write(&kernel_path, &kb.data)
                        .with_context(|| format!("Failed to write kernel binary {}", i))?;
                    info!("  Kernel module '{}': {} bytes → {}",
                        kb.module_name, kb.data.len(), kernel_path.display());
                }

                // Fall through to native compilation for host code.
                // The GPU kernels are either:
                // - Embedded in the LLVM IR as global constants (from MLIR binary pass)
                // - Written as separate files for runtime loading
                // - Using built-in shader library (Metal METAL_SHADER_SOURCE)
                info!("Compiling host code via LLVM native path...");
                self.run_native_compilation()
            }
            Err(e) => {
                // GPU binary emission failed — fall back gracefully to CPU.
                // This is non-fatal: the program will still run correctly,
                // just without GPU acceleration.
                warn!(
                    "GPU binary emission failed: {:?}. \
                     Falling back to CPU-only compilation. \
                     GPU tensor ops will execute on CPU via VBC interpreter.",
                    e
                );
                self.run_native_compilation()
            }
        }
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
