// Configuration management for Verum projects
// Handles verum.toml parsing and project manifests
// Parses verum.toml project manifests with sections: [package], [language],
// [dependencies], [profiles], [build], [workspace], [lsp], [registry]

use crate::error::{CliError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use verum_common::{List, Map, Text};

// ========================================================================
// MLIR Backend Configuration Types (formerly LLVM config)
// These types are kept for manifest compatibility but now configure MLIR
// ========================================================================

/// MLIR/LLVM backend configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlvmConfig {
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    #[serde(default)]
    pub target_triple: Option<Text>,
    /// Target CPU (e.g., "native", "generic")
    #[serde(default)]
    pub target_cpu: Option<Text>,
    /// Target features (e.g., "+avx2,+fma")
    #[serde(default)]
    pub target_features: Option<Text>,
}

impl LlvmConfig {
    /// Validate the LLVM/MLIR configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        // Target triple validation is now handled by MLIR backend
        Ok(())
    }
}

/// Optimization pass configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptimizationConfig {
    /// Optimization level (0-3)
    #[serde(default)]
    pub level: u8,
    /// Enable size optimization
    #[serde(default)]
    pub size_opt: bool,
    /// Enable inline optimization
    #[serde(default)]
    pub inline: bool,
}

impl OptimizationConfig {
    /// Validate the optimization configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.level > 3 {
            return Err(format!("Optimization level must be 0-3, got {}", self.level));
        }
        Ok(())
    }

    /// Create a debug configuration (no optimizations)
    pub fn debug() -> Self {
        Self {
            level: 0,
            size_opt: false,
            inline: false,
        }
    }
}

/// Link-time optimization configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LtoConfig {
    /// Enable LTO
    #[serde(default)]
    pub enabled: bool,
    /// LTO mode: "thin" or "full"
    #[serde(default)]
    pub mode: Option<Text>,
}

impl LtoConfig {
    /// Validate the LTO configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        if let Some(mode) = &self.mode {
            match mode.as_str() {
                "thin" | "full" => {}
                _ => return Err(format!("Invalid LTO mode: {}, must be 'thin' or 'full'", mode)),
            }
        }
        Ok(())
    }
}

/// Profile-guided optimization configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PgoConfig {
    /// Enable PGO
    #[serde(default)]
    pub enabled: bool,
    /// Profile data path
    #[serde(default)]
    pub profile_path: Option<Text>,
}

impl PgoConfig {
    /// Validate the PGO configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        // Profile path validation is deferred to runtime
        Ok(())
    }
}

/// Cross-compilation configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CrossCompileConfig {
    /// Target platform
    #[serde(default)]
    pub target: Option<Text>,
    /// Sysroot path
    #[serde(default)]
    pub sysroot: Option<Text>,
    /// Linker to use
    #[serde(default)]
    pub linker: Option<Text>,
}

impl CrossCompileConfig {
    /// Validate the cross-compilation configuration
    pub fn validate(&self) -> std::result::Result<(), String> {
        // Cross-compilation validation is now handled by MLIR backend
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub cog: Cog,
    #[serde(default)]
    pub language: LanguageConfig,
    #[serde(default)]
    pub dependencies: Map<Text, Dependency>,
    #[serde(default)]
    pub dev_dependencies: Map<Text, Dependency>,
    #[serde(default)]
    pub build_dependencies: Map<Text, Dependency>,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub features: Map<Text, List<Text>>,
    #[serde(default)]
    pub profile: ProfileConfig,
    #[serde(default)]
    pub workspace: Option<WorkspaceConfig>,
    #[serde(default)]
    pub lsp: LspConfig,
    #[serde(default)]
    pub registry: RegistryConfig,

    // ========================================================================
    // LLVM Backend Configuration Sections
    // ========================================================================
    /// LLVM backend configuration (target, CPU, features)
    #[serde(default)]
    pub llvm: LlvmConfig,

    /// Optimization pass configuration
    #[serde(default)]
    pub optimization: OptimizationConfig,

    /// Link-time optimization configuration
    #[serde(default)]
    pub lto: LtoConfig,

    /// Profile-guided optimization configuration
    #[serde(default)]
    pub pgo: PgoConfig,

    /// Cross-compilation configuration
    #[serde(default)]
    pub cross_compile: CrossCompileConfig,

    /// Formal verification configuration
    #[serde(default)]
    pub verify: VerifyConfig,

    // ========================================================================
    // Language Feature Configuration
    // Each section controls an orthogonal subsystem of the language.
    // ========================================================================

    /// Type system features (dependent, cubical, HKT, universe polymorphism, …)
    #[serde(default)]
    pub types: TypesConfig,

    /// Runtime behavior (CBGR mode, async scheduler, GC policy, …)
    #[serde(default)]
    pub runtime: RuntimeConfig,

    /// Code generation (execution tier, GPU, debug info, SIMD, …)
    #[serde(default)]
    pub codegen: CodegenConfig,

    /// Metaprogramming (compile-time fns, quote, reflection, derive, staging)
    #[serde(default)]
    pub meta: MetaConfig,

    /// Protocol / trait system (coherence, resolution, GATs, blanket impls)
    #[serde(default)]
    pub protocols: ProtocolsConfig,

    /// Context system / dependency injection (`using [...]`)
    #[serde(default)]
    pub context: ContextConfig,

    /// Safety constraints (unsafe, FFI, capabilities, MLS level)
    #[serde(default)]
    pub safety: SafetyConfig,

    /// Testing (differential, property-based, fuzzing, coverage)
    #[serde(default)]
    pub test: TestConfig,

    /// Debug adapter (DAP) configuration
    #[serde(default)]
    pub debug: DebugConfig,
}

/// Formal-verification configuration (the `[verify]` section).
///
/// Lets projects customize default verification behavior without needing
/// to annotate every function with `@verify(...)`. Strategy names here are
/// semantic (backend-agnostic) — see grammar/verum.ebnf for the full list.
///
/// ## Example `verum.toml`
///
/// ```toml
/// [verify]
/// default_strategy = "formal"
/// solver_timeout_ms = 10000
/// enable_telemetry = true
/// persist_stats = true
///
/// # Per-module overrides
/// [verify.modules."crypto.signing"]
/// strategy = "certified"
/// solver_timeout_ms = 60000
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyConfig {
    /// Default verification strategy when no `@verify(...)` attribute is present.
    ///
    /// Valid values: `runtime`, `static`, `formal`, `fast`, `thorough`,
    /// `certified`, `synthesize`. Defaults to `formal`.
    #[serde(default = "default_verify_strategy")]
    pub default_strategy: Text,

    /// Base solver timeout in milliseconds. Strategy-specific multipliers
    /// apply: `fast` uses 0.3×, `thorough` uses 2×, `certified` uses 3×,
    /// `synthesize` uses 5×. Default: 10000 (10 seconds).
    #[serde(default = "default_solver_timeout")]
    pub solver_timeout_ms: u64,

    /// Enable routing-statistics telemetry collection. When true, the
    /// compiler records which verification technique ran for each goal
    /// and writes results to `.verum/state/smt-stats.json`. Viewable via
    /// `verum smt-stats`. Default: true.
    #[serde(default = "default_true")]
    pub enable_telemetry: bool,

    /// Persist telemetry to disk at `.verum/state/smt-stats.json` so it
    /// survives across compilation sessions. When false, stats live only
    /// for the current run. Default: true.
    #[serde(default = "default_true")]
    pub persist_stats: bool,

    /// Treat any cross-validation divergence as a hard build error.
    ///
    /// When true (default), if a `@verify(certified)` goal produces
    /// divergent results between independent verifiers, the build fails.
    /// When false, divergences are logged but do not stop compilation
    /// (useful during verifier debugging).
    #[serde(default = "default_true")]
    pub fail_on_divergence: bool,

    /// Per-module verification overrides.
    ///
    /// Keys are module paths (e.g., `"crypto.signing"`); values are the
    /// same fields as the top-level `[verify]` section but narrowed to
    /// that module and its descendants.
    #[serde(default)]
    pub modules: Map<Text, VerifyModuleOverride>,

    // ─────────────────────────────────────────────────────────────
    // Profiler / budget / cache block — see
    // docs/detailed/25-developer-tooling.md §1.5
    // CLI flags (--profile / --budget / --export / --distributed-cache)
    // override these values when present.
    // ─────────────────────────────────────────────────────────────
    /// Human-readable project-wide verification budget (e.g. `"120s"`,
    /// `"5m"`). When set, builds fail if total verification time exceeds
    /// this limit. `None` means unbounded. Overridden by `--budget=…`.
    #[serde(default)]
    pub total_budget: Option<Text>,

    /// Per-function slow-verification threshold. Functions whose SMT run
    /// exceeds this duration are flagged in the `--profile` report.
    /// Default: `"5s"` (matches spec §1.5).
    #[serde(default)]
    pub slow_threshold: Option<Text>,

    /// Path (relative to workspace root) where the on-disk verification
    /// cache lives. Defaults to `.verum/verify-cache`.
    #[serde(default)]
    pub cache_dir: Option<Text>,

    /// Cache size cap, e.g. `"500MB"`. `None` means unbounded.
    #[serde(default)]
    pub cache_max_size: Option<Text>,

    /// Cache TTL, e.g. `"30d"`, `"24h"`. Entries older than this are
    /// evicted on the next run. `None` disables TTL-based eviction.
    #[serde(default)]
    pub cache_ttl: Option<Text>,

    /// URL of a distributed verification cache (e.g.
    /// `s3://bucket/verify-cache`, `redis://host/`). `None` disables.
    /// Overridden by `--distributed-cache=…`.
    #[serde(default)]
    pub distributed_cache: Option<Text>,

    /// Trust policy for distributed-cache reads. Accepted values:
    /// `"all"` (no verification — trust everything), `"signatures"`
    /// (require Ed25519 signature, default), `"signatures_and_expiry"`
    /// (signatures plus TTL freshness check). Determines what level
    /// of validation the local proof-cache layer applies to a result
    /// fetched from the configured `distributed_cache` backend before
    /// installing it into the in-memory cache. Unknown values fall
    /// back to `"signatures"` with a warning at construction time so
    /// a typo never silently downgrades to `All`.
    #[serde(default)]
    pub distributed_cache_trust: Option<Text>,

    /// Turn on per-function profiling of slow verifications. Default:
    /// `true` — the profiler is enabled iff `--profile` is passed on
    /// the CLI.
    #[serde(default = "default_true")]
    pub profile_slow_functions: bool,

    /// Threshold above which a function counts as "slow" for the
    /// profiler. Default: `"1s"`.
    #[serde(default)]
    pub profile_threshold: Option<Text>,

    /// Named verification profiles — e.g. `release`, `ci`, `dev`.
    ///
    /// Each profile inherits every field from the top-level `[verify]`
    /// section and overrides only what it names. Selected via
    /// `verum verify --profile <name>`. Matches the documented
    /// workflow in `docs/verification/cli-workflow.md §9`.
    ///
    /// Example:
    ///
    /// ```toml
    /// [verify]
    /// default_strategy = "formal"
    /// solver_timeout_ms = 10000
    ///
    /// [verify.profiles.release]
    /// default_strategy = "certified"
    /// solver_timeout_ms = 300000
    /// fail_on_divergence = true
    ///
    /// [verify.profiles.ci]
    /// default_strategy = "fast"
    /// solver_timeout_ms = 3000
    /// ```
    ///
    /// CLI flags STILL override profile values — selection order is:
    /// CLI flag > profile override > base `[verify]` > default.
    #[serde(default)]
    pub profiles: Map<Text, VerifyProfileOverride>,
}

/// Per-profile override block in the `[verify.profiles.<name>]` section.
///
/// Every field is optional — profile inheritance means unset fields
/// fall through to the base `[verify]` section. This preserves a
/// "specify only what differs" ergonomics without losing parent
/// settings silently.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyProfileOverride {
    /// Override for `default_strategy`.
    #[serde(default)]
    pub default_strategy: Option<Text>,

    /// Override for `solver_timeout_ms`.
    #[serde(default)]
    pub solver_timeout_ms: Option<u64>,

    /// Override for `enable_telemetry`.
    #[serde(default)]
    pub enable_telemetry: Option<bool>,

    /// Override for `persist_stats`.
    #[serde(default)]
    pub persist_stats: Option<bool>,

    /// Override for `fail_on_divergence`.
    #[serde(default)]
    pub fail_on_divergence: Option<bool>,

    /// Override for `total_budget`.
    #[serde(default)]
    pub total_budget: Option<Text>,

    /// Override for `slow_threshold`.
    #[serde(default)]
    pub slow_threshold: Option<Text>,

    /// Override for `cache_dir`.
    #[serde(default)]
    pub cache_dir: Option<Text>,

    /// Override for `cache_max_size`.
    #[serde(default)]
    pub cache_max_size: Option<Text>,

    /// Override for `cache_ttl`.
    #[serde(default)]
    pub cache_ttl: Option<Text>,

    /// Override for `distributed_cache`.
    #[serde(default)]
    pub distributed_cache: Option<Text>,

    /// Override for `distributed_cache_trust`.
    #[serde(default)]
    pub distributed_cache_trust: Option<Text>,
}

impl VerifyConfig {
    /// Apply a named profile's overrides on top of the base config,
    /// producing the effective `VerifyConfig` the CLI should use.
    ///
    /// Inheritance policy: profile values override base values; unset
    /// profile fields leave base values intact. Per-module overrides
    /// (`modules`) are NOT merged — profiles cannot change per-module
    /// settings in this release (see task #81 follow-up).
    ///
    /// Returns `Err` if the named profile does not exist.
    pub fn with_profile(self, name: &str) -> std::result::Result<Self, Text> {
        let profile = self
            .profiles
            .get(&Text::from(name))
            .ok_or_else(|| {
                Text::from(format!(
                    "verify profile '{}' is not declared in [verify.profiles.*]",
                    name
                ))
            })?
            .clone();

        let mut merged = self;
        if let Some(v) = profile.default_strategy {
            merged.default_strategy = v;
        }
        if let Some(v) = profile.solver_timeout_ms {
            merged.solver_timeout_ms = v;
        }
        if let Some(v) = profile.enable_telemetry {
            merged.enable_telemetry = v;
        }
        if let Some(v) = profile.persist_stats {
            merged.persist_stats = v;
        }
        if let Some(v) = profile.fail_on_divergence {
            merged.fail_on_divergence = v;
        }
        if let Some(v) = profile.total_budget {
            merged.total_budget = Some(v);
        }
        if let Some(v) = profile.slow_threshold {
            merged.slow_threshold = Some(v);
        }
        if let Some(v) = profile.cache_dir {
            merged.cache_dir = Some(v);
        }
        if let Some(v) = profile.cache_max_size {
            merged.cache_max_size = Some(v);
        }
        if let Some(v) = profile.cache_ttl {
            merged.cache_ttl = Some(v);
        }
        if let Some(v) = profile.distributed_cache {
            merged.distributed_cache = Some(v);
        }
        if let Some(v) = profile.distributed_cache_trust {
            merged.distributed_cache_trust = Some(v);
        }
        Ok(merged)
    }
}

impl Default for VerifyConfig {
    fn default() -> Self {
        Self {
            default_strategy: default_verify_strategy(),
            solver_timeout_ms: default_solver_timeout(),
            enable_telemetry: true,
            persist_stats: true,
            fail_on_divergence: true,
            modules: Map::new(),
            total_budget: None,
            slow_threshold: None,
            cache_dir: None,
            cache_max_size: None,
            cache_ttl: None,
            distributed_cache: None,
            distributed_cache_trust: None,
            profile_slow_functions: true,
            profile_threshold: None,
            profiles: Map::new(),
        }
    }
}

/// Per-module verification settings (nested under `[verify.modules."path"]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyModuleOverride {
    /// Override the default strategy for this module.
    #[serde(default)]
    pub strategy: Option<Text>,
    /// Override the solver timeout for this module.
    #[serde(default)]
    pub solver_timeout_ms: Option<u64>,
}

fn default_verify_strategy() -> Text {
    Text::from("formal")
}

fn default_solver_timeout() -> u64 {
    10_000
}

fn default_true() -> bool {
    true
}

// ============================================================================
// Type System Configuration
// ============================================================================

/// Advanced type-system features (the `[types]` section).
///
/// Controls which advanced type-theoretic features the compiler enables.
/// Disabling features skips associated checks and may speed up compilation
/// at the cost of less expressive types.
///
/// ## Example
///
/// ```toml
/// [types]
/// dependent = true          # Pi/Sigma types, length-indexed vectors
/// refinement = true         # Int{> 0}, Text{len(it) > 0}
/// cubical = true            # Path types, transport, hcomp (HoTT)
/// higher_kinded = true      # F: Type -> Type in generic params
/// universe_polymorphism = false  # Type(u), @universe_poly
/// coinductive = true        # codata declarations + copatterns
/// quotient = true           # HIT-based quotient types
/// instance_search = true    # Automatic protocol-impl resolution
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypesConfig {
    /// Enable dependent types (Pi, Sigma, Eq) — length-indexed vectors, etc.
    #[serde(default = "default_true")]
    pub dependent: bool,

    /// Enable refinement types — `Int{predicate}` triggering SMT verification.
    #[serde(default = "default_true")]
    pub refinement: bool,

    /// Enable cubical type theory — Path types, transport, hcomp for HoTT.
    #[serde(default = "default_true")]
    pub cubical: bool,

    /// Enable higher-kinded types — `F: Type -> Type` in generic params.
    #[serde(default = "default_true")]
    pub higher_kinded: bool,

    /// Enable universe polymorphism — `Type(u)`, `@universe_poly`.
    /// Disabled by default (rare feature with performance cost).
    #[serde(default)]
    pub universe_polymorphism: bool,

    /// Enable coinductive types — `codata` declarations with copatterns.
    #[serde(default = "default_true")]
    pub coinductive: bool,

    /// Enable quotient types — HIT-based types for modular equivalence.
    #[serde(default = "default_true")]
    pub quotient: bool,

    /// Enable automatic protocol-implementation resolution.
    #[serde(default = "default_true")]
    pub instance_search: bool,

    /// Maximum coherence-check depth for instance resolution.
    #[serde(default = "default_coherence_depth")]
    pub coherence_check_depth: u32,
}

impl Default for TypesConfig {
    fn default() -> Self {
        Self {
            dependent: true,
            refinement: true,
            cubical: true,
            higher_kinded: true,
            universe_polymorphism: false,
            coinductive: true,
            quotient: true,
            instance_search: true,
            coherence_check_depth: default_coherence_depth(),
        }
    }
}

fn default_coherence_depth() -> u32 {
    16
}

// ============================================================================
// Runtime Configuration
// ============================================================================

/// Runtime system configuration (the `[runtime]` section).
///
/// Controls memory management, async execution, and low-level runtime behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// CBGR reference mode: `managed` (~15ns), `checked` (0ns, static proof),
    /// `unsafe` (0ns, no safety), `mixed` (auto-select per reference).
    ///
    /// Default: `mixed`.
    #[serde(default = "default_cbgr_mode")]
    pub cbgr_mode: Text,

    /// Async scheduler: `single_threaded`, `multi_threaded`, `work_stealing`.
    #[serde(default = "default_async_scheduler")]
    pub async_scheduler: Text,

    /// Number of worker threads for the async scheduler.
    /// 0 means auto-detect (= logical CPU count).
    #[serde(default)]
    pub async_worker_threads: u32,

    /// Enable future polling for cooperative concurrency.
    #[serde(default = "default_true")]
    pub futures: bool,

    /// Enable structured concurrency (nurseries).
    #[serde(default = "default_true")]
    pub nurseries: bool,

    /// Stack size for spawned tasks (bytes). 0 = default OS stack size.
    #[serde(default)]
    pub task_stack_size: u64,

    /// Heap growth policy: `aggressive`, `conservative`, `adaptive`.
    #[serde(default = "default_heap_policy")]
    pub heap_policy: Text,

    /// Panic strategy: `unwind`, `abort`.
    #[serde(default = "default_panic_strategy")]
    pub panic: Text,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            cbgr_mode: default_cbgr_mode(),
            async_scheduler: default_async_scheduler(),
            async_worker_threads: 0,
            futures: true,
            nurseries: true,
            task_stack_size: 0,
            heap_policy: default_heap_policy(),
            panic: default_panic_strategy(),
        }
    }
}

fn default_cbgr_mode() -> Text { Text::from("mixed") }
fn default_async_scheduler() -> Text { Text::from("work_stealing") }
fn default_heap_policy() -> Text { Text::from("adaptive") }
fn default_panic_strategy() -> Text { Text::from("unwind") }

// ============================================================================
// Codegen Configuration
// ============================================================================

/// Code-generation configuration (the `[codegen]` section).
///
/// Controls the compiler's execution tiers and target-specific code output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegenConfig {
    /// Execution tier: `interpret` (VBC), `aot` (LLVM native), `check` (type-check only).
    #[serde(default = "default_tier")]
    pub tier: Text,

    /// Enable MLIR GPU compilation path (for @device(GPU) annotated code).
    #[serde(default)]
    pub mlir_gpu: bool,

    /// GPU backend: `metal` (macOS), `cuda` (NVIDIA), `rocm` (AMD), `vulkan`.
    #[serde(default = "default_gpu_backend")]
    pub gpu_backend: Text,

    /// Enable monomorphization caching (speeds up rebuilds).
    #[serde(default = "default_true")]
    pub monomorphization_cache: bool,

    /// Proof erasure: strip proof terms before codegen (zero runtime cost).
    #[serde(default = "default_true")]
    pub proof_erasure: bool,

    /// Generate debug info: `none`, `line`, `full`.
    #[serde(default = "default_debug_info")]
    pub debug_info: Text,

    /// Enable tail-call optimization.
    #[serde(default = "default_true")]
    pub tail_call_optimization: bool,

    /// Enable automatic SIMD vectorization.
    #[serde(default = "default_true")]
    pub vectorize: bool,

    /// Maximum inline depth for generic specialization.
    #[serde(default = "default_inline_depth")]
    pub inline_depth: u32,
}

impl Default for CodegenConfig {
    fn default() -> Self {
        Self {
            tier: default_tier(),
            mlir_gpu: false,
            gpu_backend: default_gpu_backend(),
            monomorphization_cache: true,
            proof_erasure: true,
            debug_info: default_debug_info(),
            tail_call_optimization: true,
            vectorize: true,
            inline_depth: default_inline_depth(),
        }
    }
}

fn default_tier() -> Text { Text::from("aot") }
fn default_gpu_backend() -> Text { Text::from("auto") }
fn default_debug_info() -> Text { Text::from("line") }
fn default_inline_depth() -> u32 { 3 }

// ============================================================================
// Metaprogramming Configuration
// ============================================================================

/// Metaprogramming configuration (the `[meta]` section).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaConfig {
    /// Allow compile-time functions (`meta fn`, `@const`).
    #[serde(default = "default_true")]
    pub compile_time_functions: bool,

    /// Allow code quoting via `quote { ... }`.
    #[serde(default = "default_true")]
    pub quote_syntax: bool,

    /// Maximum macro-expansion recursion depth.
    #[serde(default = "default_macro_depth")]
    pub macro_recursion_limit: u32,

    /// Enable reflection APIs (`TypeInfo`, `AstAccess`, `CompileDiag`).
    #[serde(default = "default_true")]
    pub reflection: bool,

    /// Allow `@derive(...)` via rule-based codegen.
    #[serde(default = "default_true")]
    pub derive: bool,

    /// Maximum staging level: 0 = runtime only, 1 = meta fn, 2+ = multi-stage.
    #[serde(default = "default_stage_limit")]
    pub max_stage_level: u32,
}

impl Default for MetaConfig {
    fn default() -> Self {
        Self {
            compile_time_functions: true,
            quote_syntax: true,
            macro_recursion_limit: default_macro_depth(),
            reflection: true,
            derive: true,
            max_stage_level: default_stage_limit(),
        }
    }
}

fn default_macro_depth() -> u32 { 128 }
fn default_stage_limit() -> u32 { 2 }

// ============================================================================
// Protocol / Trait Configuration
// ============================================================================

/// Protocol-system configuration (the `[protocols]` section).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolsConfig {
    /// Specialization coherence: `strict`, `lenient`, `unchecked`.
    ///
    /// - `strict` (default): orphan rules enforced, no overlapping impls
    /// - `lenient`: allow orphan impls in same crate (Rust-like)
    /// - `unchecked`: skip coherence checking (unsafe)
    #[serde(default = "default_coherence")]
    pub coherence: Text,

    /// Resolution strategy when multiple impls match: `most_specific`,
    /// `first_declared`, `error`.
    #[serde(default = "default_resolution")]
    pub resolution_strategy: Text,

    /// Allow blanket implementations (`impl<T> Foo for T`).
    #[serde(default = "default_true")]
    pub blanket_impls: bool,

    /// Allow higher-kinded protocols (`protocol Functor<F: Type -> Type>`).
    #[serde(default = "default_true")]
    pub higher_kinded_protocols: bool,

    /// Enable associated types (`type Output;`).
    #[serde(default = "default_true")]
    pub associated_types: bool,

    /// Enable generic associated types (GATs).
    #[serde(default = "default_true")]
    pub generic_associated_types: bool,
}

impl Default for ProtocolsConfig {
    fn default() -> Self {
        Self {
            coherence: default_coherence(),
            resolution_strategy: default_resolution(),
            blanket_impls: true,
            higher_kinded_protocols: true,
            associated_types: true,
            generic_associated_types: true,
        }
    }
}

fn default_coherence() -> Text { Text::from("strict") }
fn default_resolution() -> Text { Text::from("most_specific") }

// ============================================================================
// Context System Configuration
// ============================================================================

/// Context-system / dependency-injection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Enable the context system (dependency injection via `using [...]`).
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Strictness for unresolved contexts: `error`, `warn`, `allow`.
    #[serde(default = "default_context_strictness")]
    pub unresolved_policy: Text,

    /// Allow negative context constraints (`!using [Foo]`).
    #[serde(default = "default_true")]
    pub negative_constraints: bool,

    /// Maximum context-propagation depth (through call chains).
    #[serde(default = "default_ctx_depth")]
    pub propagation_depth: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            unresolved_policy: default_context_strictness(),
            negative_constraints: true,
            propagation_depth: default_ctx_depth(),
        }
    }
}

fn default_context_strictness() -> Text { Text::from("error") }
fn default_ctx_depth() -> u32 { 32 }

// ============================================================================
// Safety Configuration
// ============================================================================

/// Safety constraints and capabilities (the `[safety]` section).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyConfig {
    /// Allow `unsafe` blocks and `@extern` declarations.
    #[serde(default = "default_true")]
    pub unsafe_allowed: bool,

    /// Allow FFI (calling C/C++ via `@ffi`).
    #[serde(default = "default_true")]
    pub ffi: bool,

    /// FFI boundary strictness: `strict` (no auto-unsafe), `lenient`.
    #[serde(default = "default_ffi_strictness")]
    pub ffi_boundary: Text,

    /// Require explicit capabilities for sensitive operations
    /// (I/O, network, unsafe memory). Like Java SecurityManager.
    #[serde(default)]
    pub capability_required: bool,

    /// MLS security level: `public`, `secret`, `top_secret`.
    /// Affects which operations are permitted in this project.
    #[serde(default = "default_mls_level")]
    pub mls_level: Text,

    /// Forbid use of `@extern` functions from stdlib.
    #[serde(default)]
    pub forbid_stdlib_extern: bool,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            unsafe_allowed: true,
            ffi: true,
            ffi_boundary: default_ffi_strictness(),
            capability_required: false,
            mls_level: default_mls_level(),
            forbid_stdlib_extern: false,
        }
    }
}

fn default_ffi_strictness() -> Text { Text::from("strict") }
fn default_mls_level() -> Text { Text::from("public") }

// ============================================================================
// Testing Configuration
// ============================================================================

/// Test / conformance-suite configuration (the `[test]` section).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    /// Enable differential testing (VBC vs LLVM AOT results must agree).
    #[serde(default)]
    pub differential: bool,

    /// Enable property-based testing via `proptest!` macro.
    #[serde(default = "default_true")]
    pub property_testing: bool,

    /// Default number of cases for property tests.
    #[serde(default = "default_proptest_cases")]
    pub proptest_cases: u32,

    /// Enable fuzzing targets (`cargo fuzz`).
    #[serde(default)]
    pub fuzzing: bool,

    /// Maximum execution time per test (seconds). 0 = no limit.
    #[serde(default = "default_test_timeout")]
    pub timeout_secs: u64,

    /// Parallel test execution.
    #[serde(default = "default_true")]
    pub parallel: bool,

    /// Collect coverage data.
    #[serde(default)]
    pub coverage: bool,

    /// Fail tests on any emitted warning.
    #[serde(default)]
    pub deny_warnings: bool,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            differential: false,
            property_testing: true,
            proptest_cases: default_proptest_cases(),
            fuzzing: false,
            timeout_secs: default_test_timeout(),
            parallel: true,
            coverage: false,
            deny_warnings: false,
        }
    }
}

fn default_proptest_cases() -> u32 { 256 }
fn default_test_timeout() -> u64 { 60 }

// ============================================================================
// Debug / DAP Configuration
// ============================================================================

/// Debug / DAP (Debug Adapter Protocol) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugConfig {
    /// Enable DAP server for IDE integration.
    #[serde(default = "default_true")]
    pub dap_enabled: bool,

    /// Stepping granularity: `statement`, `line`, `instruction`.
    #[serde(default = "default_step_granularity")]
    pub step_granularity: Text,

    /// Maximum depth for variable inspection.
    #[serde(default = "default_inspect_depth")]
    pub inspect_depth: u32,

    /// Default DAP port (0 = auto).
    #[serde(default = "default_dap_port")]
    pub port: u16,

    /// Show erased proof terms in debug views.
    #[serde(default)]
    pub show_erased_proofs: bool,
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            dap_enabled: true,
            step_granularity: default_step_granularity(),
            inspect_depth: default_inspect_depth(),
            port: default_dap_port(),
            show_erased_proofs: false,
        }
    }
}

fn default_step_granularity() -> Text { Text::from("statement") }
fn default_inspect_depth() -> u32 { 8 }
fn default_dap_port() -> u16 { 0 }

// (default_true helper is defined earlier in this file)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cog {
    pub name: Text,
    pub version: Text,
    #[serde(default)]
    pub authors: List<Text>,
    #[serde(default)]
    pub description: Option<Text>,
    #[serde(default)]
    pub license: Option<Text>,
    #[serde(default)]
    pub repository: Option<Text>,
    #[serde(default)]
    pub homepage: Option<Text>,
    #[serde(default)]
    pub keywords: List<Text>,
    #[serde(default)]
    pub categories: List<Text>,
}

// Language profile configuration
// Profiles (application, systems, research) determine available features,
// default verification level, and compilation tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageConfig {
    pub profile: LanguageProfile,
}

impl Default for LanguageConfig {
    fn default() -> Self {
        Self {
            profile: LanguageProfile::Application,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LanguageProfile {
    Application, // 80% users: No unsafe, refinements + runtime checks
    Systems,     // 15% users: Full language including unsafe
    Research,    // 5% users: Dependent types, formal proofs
}

impl LanguageProfile {
    pub fn allows_unsafe(&self) -> bool {
        matches!(self, LanguageProfile::Systems)
    }

    pub fn requires_verification(&self) -> bool {
        matches!(self, LanguageProfile::Research)
    }

    pub fn description(&self) -> &'static str {
        match self {
            LanguageProfile::Application => "No unsafe, refinements + runtime checks (80% users)",
            LanguageProfile::Systems => "Full language including unsafe (15% users)",
            LanguageProfile::Research => "Dependent types, formal proofs (5% users)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    Simple(Text),
    Detailed {
        version: Option<Text>,
        path: Option<PathBuf>,
        git: Option<Text>,
        branch: Option<Text>,
        tag: Option<Text>,
        rev: Option<Text>,
        features: Option<List<Text>>,
        optional: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuildConfig {
    #[serde(default = "default_target")]
    pub target: Text,
    #[serde(default = "default_opt_level")]
    pub opt_level: u8,
    #[serde(default)]
    pub incremental: bool,
    #[serde(default)]
    pub lto: bool,
    #[serde(default)]
    pub codegen_units: Option<usize>,
    #[serde(default)]
    pub panic: PanicStrategy,
}

fn default_target() -> Text {
    "native".into()
}

fn default_opt_level() -> u8 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PanicStrategy {
    #[default]
    Unwind,
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileConfig {
    #[serde(default)]
    pub dev: Profile,
    #[serde(default)]
    pub release: Profile,
    #[serde(default)]
    pub test: Profile,
    #[serde(default)]
    pub bench: Profile,
}

// Two-tier compilation modes
// Tier 0: VBC Interpreter (instant start, full diagnostics)
// Tier 1: AOT via LLVM (optimized native binary)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum CompilationTier {
    /// Tier 0: VBC Interpreter (instant start, full diagnostics)
    #[serde(rename = "0", alias = "interpreter", alias = "interp")]
    #[default]
    Interpreter,

    /// Tier 1: AOT compilation via LLVM (production, 85-95% native speed)
    #[serde(rename = "1", alias = "aot", alias = "release", alias = "native")]
    Aot,
}

impl CompilationTier {
    /// Parse tier from numeric value (0-1)
    pub fn from_u8(tier: u8) -> Option<Self> {
        match tier {
            0 => Some(CompilationTier::Interpreter),
            1 => Some(CompilationTier::Aot),
            _ => None,
        }
    }

    /// Parse tier from string (numeric or named)
    /// Accepts: "0", "1", "interpreter", "aot", "release", "native"
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "0" | "interpreter" | "interp" => Some(CompilationTier::Interpreter),
            "1" | "aot" | "release" | "native" => Some(CompilationTier::Aot),
            _ => None,
        }
    }

    pub fn as_u8(&self) -> u8 {
        match self {
            CompilationTier::Interpreter => 0,
            CompilationTier::Aot => 1,
        }
    }

    /// Human-readable name for the tier
    pub fn name(&self) -> &'static str {
        match self {
            CompilationTier::Interpreter => "interpreter",
            CompilationTier::Aot => "aot",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            CompilationTier::Interpreter => "VBC Interpreter (instant start, full diagnostics)",
            CompilationTier::Aot => "AOT compilation (production, 85-95% native speed)",
        }
    }

    /// List all valid tier names for help text
    pub fn valid_values() -> &'static str {
        "interpreter|aot (or 0-1)"
    }
}

// Reference system modes
// Three-tier CBGR reference model: Managed (&T, ~15ns checks), Checked
// (&checked T, 0ns compiler-proven), Mixed (auto-select per reference)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ReferenceMode {
    #[default]
    Managed, // CBGR checks (~15ns overhead)
    Checked, // Static verification (0ns)
    Mixed,   // Smart selection (recommended)
}

// Verification levels — full VVA §2.3 9-strategy ladder.
//
// The ladder is *strictly monotone* under the ν-coordinate
// (`MsfsCoord.nu`); each strategy is sound, but completeness and
// cost grow strictly. A strategy at ν_i discharges every obligation
// a strategy at ν_j ≤ ν_i can, plus more.
//
// | Strategy   | Meaning                                                        | ν       |
// |------------|----------------------------------------------------------------|---------|
// | `none`     | No verification (unsafe!).                                     | —       |
// | `runtime`  | Emit runtime assertions; no compile-time discharge.            | 0       |
// | `static`   | Conservative dataflow / constant folding / CBGR.               | 1       |
// | `fast`     | Bounded SMT (single solver, ≤ 100 ms / goal).                  | 2       |
// | `formal`   | Full SMT portfolio (Z3 + CVC5) with decision procedures.       | ω       |
// | `proof`    | User tactic proof; kernel re-checks.                           | ω+1     |
// | `thorough` | `formal` + mandatory invariant/frame/termination obligations.  | ω·2     |
// | `reliable` | `thorough` + cross-solver agreement (Z3 ∧ CVC5 must agree).    | ω·2+1   |
// | `certified`| `reliable` + certificate re-check + cross-format export.       | ω·2+2   |
// | `synthesize`| Inverse proof search + dispatch to strictest non-synth.       | ≤ ω·3+1 |
//
// Direction: `@verify(proof)` on a function compiling under
// `@verify(runtime)` is always accepted (lax → strict). The reverse
// requires re-proof and is rejected by the level-inference pass.
//
// Wire format: `[serde(rename_all = "lowercase")]` so verum.toml
// uses the lowercase identifiers (`verification = "certified"`,
// `default_strategy = "synthesize"`, etc.).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum VerificationLevel {
    /// No verification — completely unchecked. Never auto-selected.
    None,
    /// Emit runtime assertions; no compile-time discharge. Default
    /// for the `application` profile.
    #[default]
    Runtime,
    /// Conservative dataflow / constant folding / CBGR. Discharges
    /// trivially-decidable refinements without SMT.
    Static,
    /// Bounded SMT (single solver, short timeout). Returns UNKNOWN
    /// conservatively without escalating.
    Fast,
    /// Full SMT portfolio (Z3 + CVC5) with decision procedures.
    /// Default for the `research` profile (dev / test / bench).
    Formal,
    /// User-supplied tactic proof; kernel re-checks. Promotes from
    /// `formal` when SMT cannot discharge.
    Proof,
    /// `formal` + mandatory invariant / frame / termination
    /// obligations. Catches missing specs.
    Thorough,
    /// `thorough` + cross-solver agreement (Z3 AND CVC5 must agree).
    /// Fails on any disagreement.
    Reliable,
    /// `reliable` + certificate re-check + Lean / Coq / Agda /
    /// Dedukti / Metamath cross-format export. Default for the
    /// `research` profile (release).
    Certified,
    /// Inverse proof search across the moduli space; dispatches to
    /// the strictest non-synthesize strategy in scope on success.
    Synthesize,
    /// VFE-6 — bounded-arithmetic verification (V_0 / V_1 / S^1_2 /
    /// V_NP / V_PH / IΔ_0). Polynomial-time decidable fragments
    /// inside the weak stratum; CI budget ≤ 30 s. ν < ω (n).
    ComplexityTyped,
    /// VFE-8 — coherent verification static fragment. Adds operational-
    /// coherence (108.T round-trip) checks at compile time; static
    /// portion only.
    CoherentStatic,
    /// VFE-8 — coherent verification runtime fragment. Same as
    /// coherent_static + ε-monitor emission for the runtime layer.
    CoherentRuntime,
    /// VFE-8 — full coherent verification. Combines coherent_static
    /// + coherent_runtime + bidirectional α-cert ⟺ ε-cert validation.
    Coherent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub tier: CompilationTier,
    #[serde(default)]
    pub verification: VerificationLevel,
    #[serde(default = "default_dev_opt")]
    pub opt_level: u8,
    #[serde(default)]
    pub debug: bool,
    #[serde(default)]
    pub debug_assertions: bool,
    #[serde(default)]
    pub overflow_checks: bool,
    #[serde(default)]
    pub lto: bool,
    #[serde(default)]
    pub incremental: bool,
    #[serde(default)]
    pub codegen_units: Option<usize>,
    #[serde(default)]
    pub cbgr_checks: CbgrCheckMode,
}

// CBGR check modes
// All: every reference checked at runtime (~15ns each)
// Optimized: escape analysis eliminates provably-safe checks
// Proven: only emit checks where safety cannot be statically proven
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum CbgrCheckMode {
    #[default]
    All, // All CBGR checks enabled
    Optimized, // Escape analysis optimization
    Proven,    // Only unproven checks
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            tier: CompilationTier::Interpreter,
            verification: VerificationLevel::Runtime,
            opt_level: 0,
            debug: true,
            debug_assertions: true,
            overflow_checks: true,
            lto: false,
            incremental: true,
            codegen_units: Some(256),
            cbgr_checks: CbgrCheckMode::All,
        }
    }
}

// Workspace configuration
// Multi-cog workspace with shared dependencies and unified build
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub members: List<Text>,
    #[serde(default)]
    pub exclude: List<Text>,
}

// LSP configuration
// IDE integration settings: CBGR cost hints, refinement validation mode,
// auto-import, format-on-save
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    #[serde(default = "default_true")]
    pub enable_cost_hints: bool,
    #[serde(default = "default_incremental")]
    pub validation_mode: Text,
    #[serde(default = "default_true")]
    pub auto_import: bool,
    #[serde(default)]
    pub format_on_save: bool,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enable_cost_hints: true,
            validation_mode: "incremental".into(),
            auto_import: true,
            format_on_save: false,
        }
    }
}

fn default_incremental() -> Text {
    "incremental".into()
}

// Registry configuration
// Cog registry URL and authentication for package distribution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    #[serde(default = "default_registry")]
    pub index: Text,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            index: default_registry(),
        }
    }
}

fn default_registry() -> Text {
    "https://packages.verum.lang".into()
}

fn default_dev_opt() -> u8 {
    0
}

impl Manifest {
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|_| CliError::ProjectNotFound(path.to_path_buf()))?;

        toml::from_str(&content).map_err(CliError::from)
    }

    pub fn to_file(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn find_manifest_dir() -> Result<PathBuf> {
        let mut current = std::env::current_dir()?;

        loop {
            // Try both verum.toml and Verum.toml for compatibility
            let manifest_path = current.join("verum.toml");
            let manifest_path_alt = current.join("Verum.toml");

            if manifest_path.exists() || manifest_path_alt.exists() {
                return Ok(current);
            }

            if !current.pop() {
                return Err(CliError::ProjectNotFound(PathBuf::from("verum.toml")));
            }
        }
    }

    /// Load manifest from a directory (alias for find + from_file)
    pub fn load<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let dir = dir.as_ref();
        let manifest_path = Self::manifest_path(dir);
        Self::from_file(&manifest_path)
    }

    /// Canonical manifest filename (lowercase, like `verum.lock`,
    /// `verum.work`). Use this when *writing* a new manifest. When
    /// *reading*, prefer [`Config::manifest_path`] which falls back
    /// to the legacy capitalised form for backward compatibility.
    pub const MANIFEST_FILENAME: &'static str = "verum.toml";

    /// Canonical lockfile filename. See [`Config::MANIFEST_FILENAME`]
    /// for the read/write convention.
    pub const LOCKFILE_FILENAME: &'static str = "verum.lock";

    /// Resolve the manifest path under `dir`. Prefers lowercase
    /// `verum.toml` (canonical); falls back to legacy `Verum.toml`
    /// when the lowercase form is missing — keeps existing user
    /// projects working through the casing migration.
    pub fn manifest_path(dir: &Path) -> PathBuf {
        let verum_path = dir.join(Self::MANIFEST_FILENAME);
        if verum_path.exists() {
            verum_path
        } else {
            dir.join("Verum.toml")
        }
    }

    /// Resolve the lockfile path under `dir`. Same convention as
    /// [`Config::manifest_path`]: lowercase canonical, capitalised
    /// fallback.
    pub fn lockfile_path(dir: &Path) -> PathBuf {
        let verum_path = dir.join(Self::LOCKFILE_FILENAME);
        if verum_path.exists() {
            verum_path
        } else {
            dir.join("Verum.lock")
        }
    }

    pub fn get_profile(&self, release: bool) -> &Profile {
        if release {
            &self.profile.release
        } else {
            &self.profile.dev
        }
    }

    pub fn all_dependencies(&self) -> List<(&Text, &Dependency)> {
        self.dependencies
            .iter()
            .chain(self.dev_dependencies.iter())
            .chain(self.build_dependencies.iter())
            .collect()
    }

    pub fn validate(&self) -> Result<()> {
        // Validate cog name
        if !is_valid_cog_name(self.cog.name.as_str()) {
            return Err(CliError::InvalidProjectName(self.cog.name.to_string()));
        }

        // Validate version
        if semver::Version::parse(self.cog.version.as_str()).is_err() {
            return Err(CliError::Custom(format!(
                "Invalid version: {}",
                self.cog.version
            )));
        }

        // Validate LLVM configuration sections
        self.llvm
            .validate()
            .map_err(|e| CliError::Custom(format!("LLVM config: {}", e)))?;
        self.optimization
            .validate()
            .map_err(|e| CliError::Custom(format!("Optimization config: {}", e)))?;
        self.lto
            .validate()
            .map_err(|e| CliError::Custom(format!("LTO config: {}", e)))?;
        self.pgo
            .validate()
            .map_err(|e| CliError::Custom(format!("PGO config: {}", e)))?;
        self.cross_compile
            .validate()
            .map_err(|e| CliError::Custom(format!("Cross-compile config: {}", e)))?;

        // Surface inert top-level manifest sections via tracing.
        // The CLI parses `[optimization]`, `[lto]`, `[pgo]` from
        // verum.toml and validates the values, but no production
        // code path threads them into the actual codegen
        // pipeline — `[optimization].level`/`size_opt`/`inline`,
        // `[lto].enabled`/`mode`, and `[pgo].enabled`/`profile_path`
        // all land on the manifest but are forward-looking
        // until the corresponding pipeline integration arrives.
        // The actual LTO control today comes from
        // `[linker].lto = "thin" | "full"` (handled by
        // verum_compiler::linker_config::LinkerSection).
        //
        // Closes the inert-defense pattern at the only loader
        // entry-point (validate() runs once per manifest read)
        // so embedders see when their `[optimization]` /
        // `[lto]` / `[pgo]` settings are observed-but-unused
        // rather than silently no-op'd.
        if self.optimization.level != 0
            || self.optimization.size_opt
            || self.optimization.inline
        {
            tracing::debug!(
                "manifest [optimization] section observed (level={}, size_opt={}, inline={}) \
                 — these fields are forward-looking; the active codegen optimization knob is \
                 [profile.<dev|release>].optimization (per-profile)",
                self.optimization.level,
                self.optimization.size_opt,
                self.optimization.inline,
            );
        }
        if self.lto.enabled || self.lto.mode.is_some() {
            tracing::debug!(
                "manifest [lto] section observed (enabled={}, mode={:?}) — these fields are \
                 forward-looking; the active LTO control is [linker].lto = \"thin\" | \"full\" \
                 (handled by linker_config::LinkerSection)",
                self.lto.enabled,
                self.lto.mode.as_ref().map(|t| t.as_str()),
            );
        }
        if self.pgo.enabled || self.pgo.profile_path.is_some() {
            tracing::debug!(
                "manifest [pgo] section observed (enabled={}, profile_path={:?}) — these \
                 fields are forward-looking; PGO integration is not yet wired into the \
                 LLVM/MLIR backend",
                self.pgo.enabled,
                self.pgo.profile_path.as_ref().map(|t| t.as_str()),
            );
        }
        // Same recipe as [optimization]/[lto]/[pgo] above: the
        // cross-compile section validates and lands on the
        // manifest, but the active cross-compile path today goes
        // through `[llvm].target_triple` (consumed by the LLVM
        // backend at codegen) and the `--target` CLI flag — the
        // dedicated `[cross_compile]` section has no production
        // reader threading `target` / `sysroot` / `linker` into
        // the toolchain invocation. Surface non-default settings
        // so an embedder shipping `[cross_compile] target = …`
        // sees that the value is observed-but-unused rather than
        // silently no-op'd.
        if self.cross_compile.target.is_some()
            || self.cross_compile.sysroot.is_some()
            || self.cross_compile.linker.is_some()
        {
            tracing::warn!(
                "manifest [cross_compile] section observed (target={:?}, sysroot={:?}, \
                 linker={:?}) — these fields are forward-looking; the active cross-compile \
                 path today goes through [llvm].target_triple (LLVM backend) and the \
                 --target CLI flag; sysroot / linker are not yet wired into the toolchain \
                 invocation",
                self.cross_compile.target.as_ref().map(|t| t.as_str()),
                self.cross_compile.sysroot.as_ref().map(|t| t.as_str()),
                self.cross_compile.linker.as_ref().map(|t| t.as_str()),
            );
        }

        // Surface inert `[verify.modules.<path>]` per-module
        // overrides. The `with_profile()` builder at line 452
        // applies named profile overrides on top of base values
        // (consumed downstream), but the per-module `modules`
        // map has NO consumer — `VerifyConfig.modules` is
        // populated from manifest parsing yet no per-function
        // verification dispatch site looks up the module path
        // to apply the override. The struct doc at line 528
        // promises module-path-keyed strategy / timeout overrides;
        // until task #81 follow-up wires that lookup into the
        // verification phase, the only honest signal is a
        // tracing surface so an embedder's `[verify.modules.
        // "crypto.signing"] strategy = "certified"` setting
        // doesn't silently fall back to the base default.
        if !self.verify.modules.is_empty() {
            tracing::debug!(
                "manifest [verify.modules.*] section observed ({} per-module overrides) — \
                 these fields are forward-looking; per-module verification dispatch does \
                 not yet honour them. Profile overrides via [verify.profiles.<name>] ARE \
                 wired and reach the verification path",
                self.verify.modules.len(),
            );
        }

        // Surface inert top-level `[build]` section. The CLI parses
        // `manifest.build: BuildConfig` (target / opt_level /
        // incremental / lto / codegen_units / panic) at line ~148,
        // but no consumer reads any of these fields. The active
        // controls today are:
        //   - target → `[llvm].target_triple` + CLI `--target`
        //   - opt_level → `[profile.<dev|release>].optimization`
        //   - lto → `[linker].lto = "thin" | "full"`
        //   (handled by linker_config::LinkerSection)
        // The remaining fields (incremental, codegen_units, panic)
        // are forward-looking — not yet threaded into the toolchain
        // invocation.
        //
        // Emit a debug-level trace when any field is set to a
        // non-default value so embedders writing `[build] lto = true`
        // or `[build] codegen_units = 16` see the section was
        // observed-but-unused rather than silently no-op'd.
        let build_has_overrides = self.build.target.as_str() != "native"
            || self.build.opt_level != 2
            || self.build.incremental
            || self.build.lto
            || self.build.codegen_units.is_some()
            || !matches!(self.build.panic, PanicStrategy::Unwind);
        if build_has_overrides {
            tracing::debug!(
                "manifest [build] section observed (target={:?}, opt_level={}, \
                 incremental={}, lto={}, codegen_units={:?}) — these fields are \
                 forward-looking; the active controls are [llvm].target_triple + \
                 --target, [profile.<dev|release>].optimization, [linker].lto. The \
                 remaining fields (incremental, codegen_units, panic) are not yet \
                 wired into the toolchain invocation",
                self.build.target.as_str(),
                self.build.opt_level,
                self.build.incremental,
                self.build.lto,
                self.build.codegen_units,
            );
        }

        // Surface inert `[verify]` section telemetry knobs.
        // `verify.enable_telemetry` (default true) and
        // `verify.persist_stats` (default true) are populated from
        // the manifest and merged via `with_profile` (config.rs:471
        // -478) but no production code path consults either flag.
        // The build.rs:411 `persist_stats(&json)` call is gated on
        // a CLI-only `--smt-stats` flag, NOT on the manifest's
        // `verify.persist_stats`. Similarly, `verify.fail_on_divergence`
        // (default true, doc'd to fail builds on cross-validation
        // mismatch) is parsed but no cross-validation path consults
        // it — the SmtBackendSwitcher's ValidationConfig.fail_on_mismatch
        // is wired separately and isn't tied to the manifest's
        // verify.fail_on_divergence.
        //
        // Surface a debug trace when any of these is set to a non-
        // default value so embedders writing
        // `[verify] enable_telemetry = false` see the gap rather
        // than silently believing telemetry was disabled.
        let verify_has_overrides = !self.verify.enable_telemetry
            || !self.verify.persist_stats
            || !self.verify.fail_on_divergence;
        if verify_has_overrides {
            tracing::debug!(
                "manifest [verify] surface: enable_telemetry={}, persist_stats={}, \
                 fail_on_divergence={} — these fields land on VerifyConfig and are \
                 mergeable via [verify.profiles.<name>] but no production code path \
                 consults them. The CLI `--smt-stats` flag controls telemetry \
                 persistence (build.rs:411), not the manifest setting; the SMT \
                 cross-validation gate is on SmtBackendSwitcher's \
                 ValidationConfig.fail_on_mismatch (separately wired). Forward-\
                 looking knobs.",
                self.verify.enable_telemetry,
                self.verify.persist_stats,
                self.verify.fail_on_divergence,
            );
        }

        Ok(())
    }
}

pub fn is_valid_cog_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

pub fn create_default_manifest(
    name: &str,
    _is_library: bool,
    profile: LanguageProfile,
) -> Manifest {
    Manifest {
        cog: Cog {
            name: name.into(),
            version: "0.1.0".into(),
            authors: List::new(),
            description: None,
            license: Some("MIT OR Apache-2.0".into()),
            repository: None,
            homepage: None,
            keywords: List::new(),
            categories: List::new(),
        },
        language: LanguageConfig { profile },
        dependencies: {
            let mut deps = Map::new();
            deps.insert("stdlib".into(), Dependency::Simple("0.1".into()));
            deps
        },
        dev_dependencies: Map::new(),
        build_dependencies: Map::new(),
        build: BuildConfig::default(),
        features: {
            let mut features = Map::new();
            features.insert("default".into(), List::new());
            features
        },
        profile: {
            // Research profile gets the strictest verification ladder
            // by default: dev = formal (full SMT portfolio with
            // decidable-fragment completeness), release = certified
            // (cross-format certificate export). Other profiles stay
            // at runtime — research is opt-in via the [language]
            // profile selection.
            let (dev_verify, release_verify, test_verify) =
                if profile == LanguageProfile::Research {
                    (
                        VerificationLevel::Formal,
                        VerificationLevel::Certified,
                        VerificationLevel::Formal,
                    )
                } else {
                    (
                        VerificationLevel::Runtime,
                        VerificationLevel::Runtime,
                        VerificationLevel::Runtime,
                    )
                };
            ProfileConfig {
                dev: Profile {
                    tier: CompilationTier::Interpreter,
                    verification: dev_verify,
                    opt_level: 0,
                    debug: true,
                    debug_assertions: true,
                    overflow_checks: true,
                    lto: false,
                    incremental: true,
                    codegen_units: Some(256),
                    cbgr_checks: CbgrCheckMode::All,
                },
                release: Profile {
                    tier: CompilationTier::Aot,
                    verification: release_verify,
                    opt_level: 3,
                    debug: false,
                    debug_assertions: false,
                    overflow_checks: false,
                    lto: true,
                    incremental: false,
                    codegen_units: Some(16),
                    cbgr_checks: CbgrCheckMode::Optimized,
                },
                test: Profile {
                    verification: test_verify,
                    ..Profile::default()
                },
                bench: Profile {
                    verification: test_verify,
                    ..Profile::default()
                },
            }
        },
        workspace: None,
        lsp: LspConfig::default(),
        registry: RegistryConfig::default(),
        llvm: LlvmConfig::default(),
        optimization: if profile == LanguageProfile::Research {
            // Research profile: maximum verification, minimal optimization
            OptimizationConfig::debug()
        } else {
            OptimizationConfig::default()
        },
        lto: LtoConfig::default(),
        pgo: PgoConfig::default(),
        cross_compile: CrossCompileConfig::default(),
        verify: VerifyConfig::default(),
        types: TypesConfig::default(),
        runtime: RuntimeConfig::default(),
        codegen: CodegenConfig::default(),
        meta: MetaConfig::default(),
        protocols: ProtocolsConfig::default(),
        context: ContextConfig::default(),
        safety: SafetyConfig::default(),
        test: TestConfig::default(),
        debug: DebugConfig::default(),
    }
}

/// Type alias for backwards compatibility
/// Some modules use Config instead of Manifest
pub type Config = Manifest;

#[cfg(test)]
mod verify_profile_tests {
    use super::*;

    // The tests deserialise `VerifyConfig` directly, so the TOML
    // fragment represents the *contents* of the `[verify]` table
    // (no wrapping table header). In a real `verum.toml` the same
    // fields live under a `[verify]` header; see cli-workflow.md §9.

    #[test]
    fn profile_override_replaces_named_fields_only() {
        let toml = r#"
            default_strategy = "formal"
            solver_timeout_ms = 10000
            enable_telemetry = true

            [profiles.release]
            default_strategy = "certified"
            solver_timeout_ms = 300000
        "#;
        let cfg: VerifyConfig = toml::from_str(toml).unwrap();
        let merged = cfg.with_profile("release").unwrap();
        assert_eq!(merged.default_strategy.as_str(), "certified");
        assert_eq!(merged.solver_timeout_ms, 300000);
        // enable_telemetry was not overridden; base value survives.
        assert!(merged.enable_telemetry);
    }

    #[test]
    fn profile_override_preserves_unset_base_values() {
        let toml = r#"
            default_strategy = "formal"

            [profiles.ci]
            solver_timeout_ms = 3000
        "#;
        let cfg: VerifyConfig = toml::from_str(toml).unwrap();
        let merged = cfg.with_profile("ci").unwrap();
        // default_strategy is inherited untouched.
        assert_eq!(merged.default_strategy.as_str(), "formal");
        assert_eq!(merged.solver_timeout_ms, 3000);
    }

    #[test]
    fn unknown_profile_is_an_error_not_a_silent_fallthrough() {
        let toml = r#"
            default_strategy = "formal"
        "#;
        let cfg: VerifyConfig = toml::from_str(toml).unwrap();
        let result = cfg.with_profile("nonexistent");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.as_str().contains("nonexistent"));
        assert!(err.as_str().contains("not declared"));
    }

    #[test]
    fn profile_inherits_cache_and_budget_settings() {
        let toml = r#"
            default_strategy = "formal"
            total_budget = "5m"
            cache_dir = ".verum/cache"

            [profiles.release]
            total_budget = "30m"
        "#;
        let cfg: VerifyConfig = toml::from_str(toml).unwrap();
        let merged = cfg.with_profile("release").unwrap();
        assert_eq!(
            merged.total_budget.as_ref().unwrap().as_str(),
            "30m"
        );
        // cache_dir inherited unchanged.
        assert_eq!(
            merged.cache_dir.as_ref().unwrap().as_str(),
            ".verum/cache"
        );
    }
}
