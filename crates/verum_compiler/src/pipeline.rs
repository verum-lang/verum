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
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter as VbcInterpreter;

// VBC → LLVM IR lowering (CPU compilation path)
use verum_codegen::llvm::{
    LoweringConfig as LlvmLoweringConfig, LoweringStats as LlvmLoweringStats, VbcToLlvmLowering,
};

// Compilation path analysis
use crate::compilation_path::{
    CompilationPath, TargetConfig as PathTargetConfig, analyze_function, determine_compilation_path,
};
use verum_lexer::Lexer;
use verum_modules::{
    ModuleId, ModuleInfo, ModuleLoader, ModulePath, ModuleRegistry, SharedModuleResolver,
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
use crate::core_cache::global_cache_or_init;
use crate::core_compiler::{CoreConfig, StdlibModuleResolver};
use crate::core_source::CoreSource;
use crate::phases::ExecutionTier;
use crate::phases::linking::{FinalLinker, LinkingConfig, ObjectFile};
use crate::phases::phase0_stdlib::{Phase0CoreCompiler, StdlibArtifacts};
use crate::phases::type_error_to_diagnostic;
use crate::session::Session;
// StdlibCompilationResult / StdlibModule now used only inside
// crate::pipeline::stdlib_bootstrap (#106 Phase 8).
use crate::hash::compute_item_hashes_from_module;
use crate::incremental_compiler::IncrementalCompiler;
use crate::staged_pipeline::{StagedConfig, StagedPipeline};

// Phase-specific submodule extractions (#106 — pipeline.rs split).
// Each submodule is a sibling file under `pipeline/` declaring an
// additional `impl<'s> CompilationPipeline<'s>` block (or a set of
// pure free helpers). Sibling-file submodules can access this
// crate's `pub(crate)` surface via `super::*`, so private fields
// of `CompilationPipeline` remain genuinely private — only methods
// move out of this file, not access boundaries.
mod ats_v_phase;
mod audit;
mod bounds_stats;
mod cbgr;
mod coherence;
mod compile_orchestration;
mod cross_file;
mod dispatch;
mod gpu_detect;
mod impl_axioms;
mod interpreter;
mod macros;
use crate::pipeline::macros::MacroExpander;
pub use macros::reset_test_isolation;
mod llvm_lowering;
mod loading;
mod mlir;
mod native_codegen;
mod phase0;
mod phases_orchestration;
mod profile_boundaries;
mod refinement_verify;
mod stdlib_bootstrap;
mod theorem_proofs;
mod tier_constructors;
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
    //
    // `std.*` is the historical alias root used by the loader's
    // synthetic prelude import (`use std.prelude.*;`).  The Verum
    // stdlib lives at `core.*`, so we install the `std → core` family
    // here so the prelude injection in `verum_modules::loader::
    // inject_prelude` resolves.  Without this, every module would
    // silently lose access to Maybe/Result/Ok/Err/etc.
    registry.register_path_alias("std", "core");
    registry.register_path_alias("std.prelude", "core.prelude");
    registry.register_path_alias("std.base", "core.base");
    registry.register_path_alias("std.maybe", "core.base.maybe");
    registry.register_path_alias("std.result", "core.base.result");
    registry.register_path_alias("std.collections", "core.collections");
    registry.register_path_alias("std.io", "core.io");
    registry.register_path_alias("std.sync", "core.sync");
    registry.register_path_alias("std.time", "core.time");
    registry.register_path_alias("std.math", "core.math");
    registry.register_path_alias("core.memory", "core.base.memory");
    registry.register_path_alias("core.maybe", "core.base.maybe");
    registry.register_path_alias("core.result", "core.base.result");
    registry.register_path_alias("core.process", "core.io.process");
    registry.register_path_alias("core.string", "core.text.text");
    registry.register_path_alias("core.text", "core.text.text");
    registry.register_path_alias("core.list", "core.collections.list");
    registry.register_path_alias("core.map", "core.collections.map");
    registry.register_path_alias("core.set", "core.collections.set");
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
    workspace_root
        .join("target")
        .join(".verum-cache")
        .join("stdlib")
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
        debug!(
            "Stdlib disk cache: format version mismatch ({} vs {})",
            cached.format_version, REGISTRY_CACHE_FORMAT_VERSION
        );
        return None;
    }
    if cached.compiler_version != env!("CARGO_PKG_VERSION") {
        debug!(
            "Stdlib disk cache: compiler version mismatch ({} vs {})",
            cached.compiler_version,
            env!("CARGO_PKG_VERSION")
        );
        return None;
    }
    if cached.llvm_version != verum_codegen::llvm::LLVM_VERSION {
        debug!(
            "Stdlib disk cache: LLVM version mismatch ({} vs {})",
            cached.llvm_version,
            verum_codegen::llvm::LLVM_VERSION
        );
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
                info!(
                    "Saved stdlib registry cache ({:.1} MB) to {}",
                    data.len() as f64 / 1_048_576.0,
                    cache_file.display()
                );
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
/// carries the path to a freshly-produced native executable. Future
/// tiers (Tier-0 interpret, MLIR JIT, MLIR AOT) extend this enum —
/// by-value matching at the caller side ensures each new variant is
/// exhaustively handled at every call site.
#[derive(Debug, Clone)]
pub enum RunResult {
    /// `check_only = true` — type-checking succeeded, no output
    /// produced. Embedders displaying build-completion UI should
    /// emit a "Check OK" message instead of pointing at a binary.
    Checked,
    /// AOT compilation succeeded — the path is the produced native
    /// executable on disk.
    Built(PathBuf),
}

impl RunResult {
    /// Path to the produced binary, or `None` for the check-only
    /// variant. Convenience for callers that only want the
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

/// State of the pipeline's `stdlib_metadata` slot.
///
/// `LazyEmbedded` defers the 5-12ms bincode decode of the embedded
/// `runtime.core_metadata` blob until the first read.  Cached-VBC
/// runs that bypass the frontend never trigger the read — saving the
/// entire decode cost on warm-cache invocations.  `Eager` is the
/// fall-back used in stdlib-bootstrap mode (no embedded blob exists
/// yet) and after an explicit `set_stdlib_metadata` call.
pub(crate) enum StdlibMetadataState {
    /// Decode lazily on first access from the embedded
    /// `runtime.core_metadata` sidecar.  Default for `NormalBuild`.
    LazyEmbedded(
        std::sync::OnceLock<
            Option<std::sync::Arc<verum_types::core_metadata::CoreMetadata>>,
        >,
    ),
    /// Eagerly held value (or `None` for "no metadata available").
    /// Used in `StdlibBootstrap` mode and after `set_stdlib_metadata`.
    Eager(Option<std::sync::Arc<verum_types::core_metadata::CoreMetadata>>),
}

impl StdlibMetadataState {
    /// Resolve the stored metadata, lazily decoding the embedded
    /// blob on first call when in `LazyEmbedded` state.
    pub(crate) fn get(
        &self,
    ) -> Option<&std::sync::Arc<verum_types::core_metadata::CoreMetadata>> {
        match self {
            Self::LazyEmbedded(cell) => cell
                .get_or_init(|| {
                    let t = std::time::Instant::now();
                    let m = crate::embedded_stdlib_metadata::get_runtime_metadata();
                    if std::env::var("VERUM_TRACE_PHASES").is_ok() {
                        eprintln!(
                            "[stdlib_metadata] lazy embedded decode: {:.2}ms",
                            t.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                    m
                })
                .as_ref(),
            Self::Eager(opt) => opt.as_ref(),
        }
    }

    pub(crate) fn is_some(&self) -> bool {
        match self {
            Self::LazyEmbedded(_) => crate::embedded_stdlib_metadata::has_runtime_metadata(),
            Self::Eager(opt) => opt.is_some(),
        }
    }

    pub(crate) fn is_none(&self) -> bool {
        !self.is_some()
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

    /// #91/#95 — typechecker-resolved call targets, drained from
    /// `TypeChecker::resolved_call_targets` after inference.  Keyed
    /// by `MethodCall` expression `Span`.  Applied to the AST via
    /// `apply_resolved_call_targets(&mut module)` after type
    /// checking, before codegen.  The codegen's
    /// `compile_method_call` fast path then reads
    /// `Expr::resolved_call_target` and skips the legacy 7-step
    /// name-resolution cascade in `try_resolve_static_method`.
    pub(crate) resolved_call_targets: std::collections::HashMap<
        verum_ast::span::Span,
        verum_ast::expr::ResolvedCallTarget,
    >,

    /// Stdlib metadata for NormalBuild mode.
    ///

    /// When set, the type checker uses pre-compiled stdlib types from embedded
    /// stdlib.vbca instead of parsing stdlib source files. This is the preferred
    /// mode for user code compilation.
    ///

    /// Pre-compiled stdlib type metadata from embedded stdlib.vbca archive.
    /// In NormalBuild mode, these types are loaded directly rather than re-parsing
    /// stdlib source, enabling fast compilation of user code.
    ///
    /// `LazyEmbedded` is the default for NormalBuild and defers the
    /// 5-12ms bincode decode of the embedded `runtime.core_metadata`
    /// blob until the first read.  Cached-VBC runs that bypass the
    /// frontend (e.g. `execute_cached_vbc`) never trigger the read at
    /// all — saving the entire decode cost on warm-cache invocations.
    /// `Eager` is used in StdlibBootstrap (no embedded blob to decode)
    /// and after `set_stdlib_metadata` (caller-supplied metadata).
    stdlib_metadata: StdlibMetadataState,

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
///  line (BOM-tolerant: `EF BB BF #!` is accepted) is a script regardless
///  of CLI invocation form. This makes shebang exec (`./hello.vr`) work
///  without any compiler-options plumbing.
///

/// 2. **Explicit entry-source flag** — `opts.script_mode` enables script
///  mode for the entry source identified by `opts.input`. We compare via
///  canonicalised paths when both sides exist (handles `./hello.vr` vs
///  `/abs/hello.vr` vs `hello.vr`); when canonicalisation fails (file
///  deleted between load and parse), fall back to a literal match. The
///  flag only matches the entry; stdlib and imported modules ignore it,
///  keeping their library-mode parsing untouched.
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
        assert!(should_parse_as_script(
            "#!/usr/bin/env verum\nprint(1)",
            &o,
            None
        ));
        assert!(should_parse_as_script(
            "#!/usr/bin/env verum\nprint(1)",
            &o,
            Some(std::path::Path::new("/tmp/x.vr"))
        ));
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
        assert!(!should_parse_as_script(
            "fn main(){}",
            &o,
            Some(std::path::Path::new("/tmp/foo.vr"))
        ));
    }

    #[test]
    fn flag_alone_requires_path_match() {
        let o = opts("/tmp/entry.vr", true);
        assert!(!should_parse_as_script("fn main(){}", &o, None));
        assert!(!should_parse_as_script(
            "fn main(){}",
            &o,
            Some(std::path::Path::new("/tmp/other.vr"))
        ));
        assert!(should_parse_as_script(
            "fn main(){}",
            &o,
            Some(std::path::Path::new("/tmp/entry.vr"))
        ));
    }

    #[test]
    fn flag_with_empty_input_matches_nothing() {
        let o = opts("", true);
        assert!(!should_parse_as_script(
            "fn main(){}",
            &o,
            Some(std::path::Path::new("/tmp/x.vr"))
        ));
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
                let cache_dir = session
                    .options()
                    .output
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join("target")
                    .join("incremental");
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
        let lazy_resolver: SharedModuleResolver =
            std::sync::Arc::new(std::sync::Mutex::new(lazy_loader));
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
            crate::meta::subsystems::project_info::capture_git_revision(&project_root_for_capture),
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
            resolved_call_targets: std::collections::HashMap::new(),
            // T2-extended single-path: when the compiler binary
            // embeds a precompiled stdlib `runtime.core_metadata`
            // sidecar, the typecheck phase prefers the
            // `Some(metadata) => TypeChecker::new_with_core(...)`
            // branch and skips the AST-walking stdlib registration
            // block entirely.  Decoding is deferred to first read —
            // cached-VBC fast paths that never typecheck don't pay
            // the 5-12ms decode cost at all.
            stdlib_metadata: StdlibMetadataState::LazyEmbedded(
                std::sync::OnceLock::new(),
            ),
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
                max_stage: 2, // Support meta(2), meta(1), runtime
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
    ///  .with_output("target/stdlib.vbca")
    ///  .with_debug_info();
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
        let lazy_resolver: SharedModuleResolver =
            std::sync::Arc::new(std::sync::Mutex::new(lazy_loader));

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
            crate::meta::subsystems::project_info::capture_git_revision(&project_root_for_capture),
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
            resolved_call_targets: std::collections::HashMap::new(),
            stdlib_metadata: StdlibMetadataState::Eager(None),
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
                max_stage: 3, // Support meta(3), meta(2), meta(1), runtime for stdlib
                enable_caching: true,
                warn_unused_stages: false, // Stdlib may not use all stages
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
        self.stdlib_metadata = StdlibMetadataState::Eager(Some(metadata));
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
    ///  .incremental_compiler()
    ///  .compute_incremental_sets_fine_grained(&all_files, |path| {
    ///  // compute hashes
    ///  });
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
                self.stdlib_metadata = StdlibMetadataState::Eager(Some(
                    std::sync::Arc::new(stdlib_meta),
                ));

                let elapsed = start.elapsed();
                info!(
                    "Stdlib cache initialization completed in {:.2}ms",
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            Err(e) => {
                warn!(
                    "Failed to load stdlib from cache: {}. Falling back to source parsing.",
                    e
                );
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
                name: Text::from(
                    cached_type
                        .path
                        .split('.')
                        .next_back()
                        .unwrap_or(&cached_type.path),
                ),
                module_path: Text::from(
                    cached_type
                        .path
                        .rsplit_once('.')
                        .map(|(p, _)| p)
                        .unwrap_or(""),
                ),
                generic_params: Self::parse_generic_params_from_definition(&cached_type.definition),
                kind: match cached_type.kind.as_str() {
                    "struct" | "record" => TypeDescriptorKind::Record {
                        fields: List::new(),
                    },
                    "variant" | "enum" => TypeDescriptorKind::Variant { cases: List::new() },
                    "protocol" | "trait" => TypeDescriptorKind::Protocol {
                        super_protocols: List::new(),
                        associated_types: List::new(),
                        required_methods: List::new(),
                        default_methods: List::new(),
                    },
                    "alias" => TypeDescriptorKind::Alias {
                        target: Text::new(),
                    },
                    _ => TypeDescriptorKind::Record {
                        fields: List::new(),
                    },
                },
                size: Maybe::None,
                alignment: Maybe::None,
                methods: List::new(),
                implements: List::new(),
                // #101 — incremental cached path; spans aren't cached.
                decl_span: Maybe::None,
            };
            metadata
                .types
                .insert(Text::from(cached_type.path.as_str()), type_desc);
        }

        // Convert functions
        for cached_func in &cached.functions {
            let func_desc = FunctionDescriptor {
                name: Text::from(
                    cached_func
                        .path
                        .split('.')
                        .next_back()
                        .unwrap_or(&cached_func.path),
                ),
                module_path: Text::from(
                    cached_func
                        .path
                        .rsplit_once('.')
                        .map(|(p, _)| p)
                        .unwrap_or(""),
                ),
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
                parent_type: Maybe::None,
                // #97 — cached metadata path doesn't carry const
                // markers; default to false (cached path is for
                // user-side incremental compilation, not stdlib).
                is_const: false,
                // #101 — incremental cached path; spans aren't cached.
                decl_span: Maybe::None,
            };
            metadata
                .functions
                .insert(Text::from(cached_func.path.as_str()), func_desc);
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
    fn parse_generic_params_from_definition(
        definition: &str,
    ) -> List<verum_types::core_metadata::GenericParam> {
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
                    if let Err(e) = self.meta_registry.register_meta_function(path, func) {
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
                        verum_ast::expr::Expr::new(ExprKind::Block(block.clone()), block.span)
                    }
                    Maybe::Some(FunctionBody::Expr(expr)) => expr.clone(),
                    Maybe::None => continue, // No body (declaration only)
                };

                let mut ctx = self
                    .fresh_meta_ctx_with_version_stamp()
                    .with_registry(std::sync::Arc::new(self.meta_registry.clone()))
                    .with_current_module(module_path.clone());

                // Enable contexts from the function's using clause
                if !func.contexts.is_empty() {
                    let context_names: Vec<verum_common::Text> = func
                        .contexts
                        .iter()
                        .filter_map(|c| {
                            c.path
                                .as_ident()
                                .map(|i| verum_common::Text::from(i.as_str()))
                        })
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
        matches!(
            e,
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
                            verum_ast::expr::Expr::new(ExprKind::Block(block.clone()), block.span)
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
                    let mut ctx = self
                        .fresh_meta_ctx_with_version_stamp()
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
                        verum_ast::stmt::StmtKind::Let {
                            value: Maybe::Some(e),
                            ..
                        } => {
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

    /// Unified pre-codegen validation.
    ///

    /// Runs every language-mechanism validation that must agree
    /// between the interpreter, CPU AOT, and GPU paths:
    ///  1. `[safety]` gates (unconditional, regardless of verify_mode)
    ///  2. Type check
    ///  3. Target-profile / dependency analysis
    ///  4. SMT refinement verification (if `verify_mode.use_smt()`)
    ///  5. Context / DI validation
    ///  6. Send/Sync boundary enforcement
    ///  7. CBGR tier analysis
    ///  8. FFI boundary validation
    ///

    /// Every pipeline entry point (`run_interpreter`,
    /// `run_native_compilation`, `run_mlir_aot`, `run_for_test`, …)
    /// should call this method to guarantee identical semantics on
    /// every .vr file across every execution path. The `skip_type`
    /// flag is offered for pathological fast-paths (e.g.,
    /// verify_mode = Runtime + no user-requested gates), but even
    /// then the safety gate still fires.
    fn validate_module(&mut self, module: &Module, skip_type_check: bool) -> Result<()> {
        let trace = std::env::var("VERUM_TRACE_PHASES").is_ok();
        let t_start = std::time::Instant::now();
        if trace {
            eprintln!(
                "[phase] validate_module: enter (skip_type_check={})",
                skip_type_check
            );
        }
        // Safety gate ALWAYS runs. Independent of verify_mode AND of
        // continue_on_error — a safety violation is a HARD security
        // boundary; collecting more diagnostics past that point risks
        // running gate-bypassed analyses on unsafe code.
        self.phase_safety_gate(module)?;
        if trace {
            eprintln!(
                "[phase] safety_gate: {:.2}ms",
                t_start.elapsed().as_secs_f64() * 1000.0
            );
        }

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
                self.session
                    .collect_phase_error("ffi_validation", ffi_result.into_inner().unwrap())?;
            } else {
                self.phase_context_validation(module);
                self.phase_send_sync_validation(module);
                let r = self.phase_ffi_validation(module);
                self.session.collect_phase_error("ffi_validation", r)?;
            }
            // Final accumulation point: under continue_on_error the
            // per-phase errors were swallowed into the diagnostic
            // stream; abort here if any accumulated.
            return self.session.abort_if_errors();
        }

        let t_tc = std::time::Instant::now();
        let r = self.phase_type_check(module);
        self.session.collect_phase_error("type_check", r)?;
        if trace {
            eprintln!(
                "[phase] type_check: {:.2}ms",
                t_tc.elapsed().as_secs_f64() * 1000.0
            );
        }

        // ATS-V phase 6.5 — architectural type checking. Walks every
        // `@arch_module(...)` declaration, runs the canonical 32-pattern
        // anti-pattern checker (AP-001..032), and emits per-violation
        // diagnostics with stable RFC codes per spec §17.4 + §32.4.
        // Modules without `@arch_module(...)` are silently skipped per
        // spec §17.5 backward-compat.
        let r = self.phase_ats_v(module);
        self.session.collect_phase_error("ats_v", r)?;

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

            self.session
                .collect_phase_error("dependency_analysis", dep_result.into_inner().unwrap())?;
            if smt_enabled {
                self.session
                    .collect_phase_error("verify", verify_result.into_inner().unwrap())?;
            }
            self.session
                .collect_phase_error("cbgr_analysis", cbgr_result.into_inner().unwrap())?;
            self.session
                .collect_phase_error("ffi_validation", ffi_result.into_inner().unwrap())?;
        } else {
            let r = self.phase_dependency_analysis(module);
            self.session.collect_phase_error("dependency_analysis", r)?;
            if smt_enabled {
                let r = self.phase_verify(module);
                self.session.collect_phase_error("verify", r)?;
            }
            self.phase_context_validation(module);
            self.phase_send_sync_validation(module);
            let r = self.phase_cbgr_analysis(module);
            self.session.collect_phase_error("cbgr_analysis", r)?;
            let r = self.phase_ffi_validation(module);
            self.session.collect_phase_error("ffi_validation", r)?;
        }

        // Final accumulation point: under continue_on_error the
        // per-phase errors were swallowed into the diagnostic
        // stream; abort here if any accumulated. Under the default
        // (continue_on_error=false), this is a no-op since any phase
        // Err already short-circuited above.
        self.session.abort_if_errors()
    }
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
        // root. We don't try to invent a sentinel — return the path
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
        // malformed; treat as out-of-range. Returning "x" verbatim
        // would let it match unrelated top-level modules in the
        // progressive-prefix walk, which is wrong.
        assert_eq!(
            resolve("core.sys.time_ops", "super.super.super.x"),
            "super.super.super.x",
        );
    }
}
