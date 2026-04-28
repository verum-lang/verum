//! Compiler Options and Configuration
//!
//! This module defines all compiler options including verification modes,
//! profiling settings, and output formats.

use std::path::PathBuf;
use verum_common::{List, Text};

use crate::language_features::LanguageFeatures;
use crate::lint::LintConfig;
use crate::profile_system::Profile;

/// Output format for diagnostics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Human-readable colored output
    #[default]
    Human,
    /// JSON output for IDE integration
    Json,
}

/// Verification mode for refinement types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum VerifyMode {
    /// Always use runtime checks (skip SMT)
    Runtime,

    /// Always use SMT verification
    Proof,

    /// Automatically decide based on complexity heuristics
    #[default]
    Auto,
}

/// Link-Time Optimization mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LtoMode {
    /// ThinLTO - faster, good for incremental builds
    Thin,
    /// Full LTO - slower, maximum optimization
    Full,
}

impl LtoMode {
    /// Parse LTO mode from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "thin" => Some(LtoMode::Thin),
            "full" => Some(LtoMode::Full),
            _ => None,
        }
    }
}

/// Output emission mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmitMode {
    /// Default: produce executable/library
    #[default]
    Binary,
    /// Emit assembly (.s)
    Assembly,
    /// Emit LLVM IR (.ll)
    LlvmIr,
    /// Emit LLVM bitcode (.bc)
    Bitcode,
    /// Emit object file only (.o)
    Object,
}

impl VerifyMode {
    /// Should use SMT solver for this mode?
    pub fn use_smt(&self) -> bool {
        matches!(self, VerifyMode::Proof | VerifyMode::Auto)
    }

    /// Should use runtime checks?
    pub fn use_runtime(&self) -> bool {
        matches!(self, VerifyMode::Runtime | VerifyMode::Auto)
    }
}

/// Comprehensive compiler options
#[derive(Debug, Clone)]
pub struct CompilerOptions {
    // I/O Options
    /// Input source file
    pub input: PathBuf,

    /// Output file (binary/executable)
    pub output: PathBuf,

    // Verification Options
    /// Verification mode for refinement types
    pub verify_mode: VerifyMode,

    /// SMT solver timeout in seconds
    pub smt_timeout_secs: u64,

    /// SMT backend selection strategy. Drives which solver the refinement /
    /// contract verification path invokes: Z3 exclusively, CVC5 exclusively,
    /// automatic heuristic selection, portfolio (parallel), or capability
    /// routing. Defaults to `Z3` to preserve historical behaviour.
    pub smt_solver: verum_smt::backend_switcher::BackendChoice,

    /// Total verification budget in seconds (None = unlimited)
    pub verification_budget_secs: Option<u64>,

    /// Per-function slow threshold in seconds (default: 5s)
    pub slow_verification_threshold_secs: u64,

    /// Show verification costs (P0 feature!)
    pub show_verification_costs: bool,

    /// Export verification results to JSON
    pub export_verification_json: bool,

    /// Path for JSON export
    pub verification_json_path: Option<PathBuf>,

    /// Enable verification profiling (detailed bottleneck analysis)
    pub profile_verification: bool,

    /// Enable per-obligation profiling granularity.
    ///
    /// When `true`, the verification profiler surfaces a per-
    /// obligation breakdown within each verified function instead
    /// of only the function-level aggregate. Obligations include
    /// preconditions, postconditions, refinement checks, loop
    /// invariants, termination measures, and structural-recursion
    /// conditions. Implies `profile_verification = true`.
    ///
    /// The human-readable report adds a "Slowest obligations"
    /// section sorted by wall-clock time; the JSON export carries
    /// the full per-obligation list under
    /// `per_obligation_timings`.
    ///
    /// Plumbed from the CLI flag `--profile-obligation`. See
    /// `docs/verification/performance.md §5`.
    pub profile_obligation: bool,

    /// URL of a distributed verification cache (e.g. `s3://bucket/path`,
    /// `redis://host:6379`, `file:///nfs/verify-cache`).
    ///
    /// When set, cache lookups and stores are routed through the configured
    /// backend in addition to the in-memory cache. Intended for CI/CD where
    /// multiple agents share proofs. The actual transport is owned by the
    /// verification cache layer; the option here is the CLI handle.
    pub distributed_cache_url: Option<String>,

    // Profiling Options (P0!)
    /// Enable CBGR memory profiling
    pub profile_memory: bool,

    /// Hot path threshold percentage (default 5%)
    pub hot_path_threshold: f64,

    // Optimization Options
    /// Optimization level (0-3)
    pub optimization_level: u8,

    /// Enable link-time optimization
    pub lto: bool,

    /// LTO mode (thin/full) - more specific than lto bool
    pub lto_mode: Option<LtoMode>,

    // Linking Options
    /// Enable static linking (no runtime dependencies)
    pub static_link: bool,

    /// Strip all symbols from output binary
    pub strip_symbols: bool,

    /// Strip debug info only (keep function names)
    pub strip_debug: bool,

    // Compilation Options
    /// Continue compilation after errors
    pub continue_on_error: bool,

    /// Check only (no code generation)
    pub check_only: bool,

    // Output Options
    /// Output format for diagnostics
    pub output_format: OutputFormat,

    /// Output emission mode (binary, asm, llvm-ir, bitcode)
    pub emit_mode: EmitMode,

    /// Emit intermediate representations
    pub emit_ir: bool,

    /// Emit AST in JSON format
    pub emit_ast: bool,

    /// Emit type information
    pub emit_types: bool,

    /// Emit VBC bytecode dump (human-readable disassembly)
    pub emit_vbc: bool,

    // Performance Options
    /// Number of threads for parallel compilation
    pub num_threads: usize,

    /// Enable incremental compilation
    pub incremental: bool,

    // Coverage Options
    /// Enable code coverage instrumentation.
    /// When true, the compiler inserts function-level counters and
    /// emits a coverage report after test execution.
    pub coverage: bool,

    // Debug Options
    /// Include debug information
    pub debug_info: bool,

    /// Verbose output level
    pub verbose: u8,

    // Target Platform Options (for @cfg evaluation)
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    /// If None, uses the host target
    pub target_triple: Option<Text>,

    /// Enabled cfg features (e.g., ["test", "feature_x"])
    pub cfg_features: List<Text>,

    /// Custom cfg key-value pairs (e.g., {"profile": "release"})
    pub cfg_custom: List<(Text, Text)>,

    /// Enable test mode (sets `test` cfg flag)
    pub test_mode: bool,

    // Lint Options
    /// Lint configuration for intrinsic diagnostics and warnings
    pub lint_config: LintConfig,

    // Gradual Verification Options (verum_verification integration)
    /// Enable bounds check elimination (removes proven-safe array bounds checks)
    pub enable_bounds_elimination: bool,

    /// Enable CBGR elimination (promotes &T to &checked T when safe)
    pub enable_cbgr_elimination: bool,

    /// Emit proof certificates (Coq, Lean, Dedukti formats)
    pub emit_proof_certificate: bool,

    /// Proof certificate format (coq, lean, dedukti)
    pub proof_certificate_format: Option<Text>,

    /// Path for proof certificate output
    pub proof_certificate_path: Option<PathBuf>,

    // GPU Kernel Detection
    /// Set to true when the AST scanner detects `@device(gpu)` or `@device(GPU)`
    /// attributes on any function. When true, the GPU compilation path (VBC -> MLIR)
    /// is automatically invoked alongside CPU compilation, without requiring
    /// an explicit `--gpu` flag.
    pub has_gpu_kernels: bool,

    // V-LLSI Profile Configuration
    /// Language profile for compilation.
    ///
    /// - **Application**: Safe, productive development (default). VBC-interpretable.
    /// - **Systems**: Performance-critical, optional unsafe. NOT VBC-interpretable.
    /// - **Research**: Experimental features. VBC-interpretable.
    ///
    /// V-LLSI (Verum Language Structured Interpretation) architecture defines
    /// three progressive profiles: Application (safe, VBC-interpretable),
    /// Systems (performance-critical, AOT-only, enables raw pointers/inline asm),
    /// Research (experimental features like dependent types, VBC-interpretable).
    pub profile: Profile,

    /// Unified language-feature flags (types, runtime, codegen, meta,
    /// protocols, context, safety, test, debug). Populated by the CLI
    /// from the merged manifest (`verum.toml` + overrides) and validated
    /// once before the pipeline runs. Compiler phases read individual
    /// flags instead of re-loading configuration.
    pub language_features: LanguageFeatures,

    /// External cancellation flag for cooperative abort of VBC interpretation.
    ///
    /// When set to `true`, the VBC interpreter's dispatch loop will return
    /// `InstructionLimitExceeded` at the next check point (~every 1024 instructions).
    /// Used by the test runner to cancel timed-out tests.
    pub cancel_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,

    /// Script-mode entry-point flag.
    ///
    /// When `true`, the entry source identified by [`Self::input`] is parsed
    /// in **script mode** — top-level statements (let, expression-stmts,
    /// defer / errdefer / provide) are accepted alongside regular items and
    /// are folded into a synthesised `__verum_script_main` wrapper. The
    /// resulting module is tagged with `@![__verum_kind("script")]` so the
    /// entry-detection pass picks the wrapper as the program entry instead
    /// of requiring an explicit `fn main()`.
    ///
    /// Independently of this flag, **any** source whose first bytes are a
    /// `#!` shebang (BOM-tolerant) is auto-detected as a script and parsed
    /// in script mode regardless of CLI invocation form. The flag covers
    /// the case where the user invokes `verum hello.vr` (or `verum -e …`,
    /// stdin) on a source that has no shebang but should still be treated
    /// as a single-file script.
    ///
    /// Only the entry source — the file matching [`Self::input`] — is
    /// affected by this flag. Stdlib modules and imported user modules are
    /// always parsed in library mode (the shebang prefix check still applies
    /// per-file, but stdlib files never carry one).
    pub script_mode: bool,
}

impl Default for CompilerOptions {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            output: PathBuf::new(),
            verify_mode: VerifyMode::default(),
            smt_timeout_secs: 30,
            smt_solver: verum_smt::backend_switcher::BackendChoice::Z3,
            verification_budget_secs: None,
            slow_verification_threshold_secs: 5,
            show_verification_costs: false,
            export_verification_json: false,
            verification_json_path: None,
            profile_verification: false,
            profile_obligation: false,
            distributed_cache_url: None,
            profile_memory: false,
            hot_path_threshold: 5.0,
            optimization_level: 0,
            lto: false,
            lto_mode: None,
            static_link: false,
            strip_symbols: false,
            strip_debug: false,
            continue_on_error: false,
            check_only: false,
            output_format: OutputFormat::default(),
            emit_mode: EmitMode::default(),
            emit_ir: false,
            emit_ast: false,
            emit_types: false,
            emit_vbc: false,
            num_threads: num_cpus(),
            incremental: false,
            coverage: false,
            debug_info: true,
            verbose: 0,
            target_triple: None,
            cfg_features: List::new(),
            cfg_custom: List::new(),
            test_mode: false,
            lint_config: LintConfig::default(),
            // Gradual verification defaults
            enable_bounds_elimination: true,  // Enable by default for performance
            enable_cbgr_elimination: true,    // Enable by default for memory safety
            emit_proof_certificate: false,
            proof_certificate_format: None,
            proof_certificate_path: None,
            // GPU kernel detection (auto-detected from AST)
            has_gpu_kernels: false,
            // V-LLSI profile: Default is Application
            profile: Profile::Application,
            language_features: LanguageFeatures::default(),
            cancel_flag: None,
            script_mode: false,
        }
    }
}

impl CompilerOptions {
    /// Create new compiler options with required fields
    pub fn new(input: PathBuf, output: PathBuf) -> Self {
        Self {
            input,
            output,
            ..Default::default()
        }
    }

    /// Builder: Set verification mode
    pub fn with_verify_mode(mut self, mode: VerifyMode) -> Self {
        self.verify_mode = mode;
        self
    }

    /// Builder: Enable verification cost reporting
    pub fn with_verification_costs(mut self, show: bool) -> Self {
        self.show_verification_costs = show;
        self
    }

    /// Builder: Enable verification profiling
    pub fn with_verification_profiling(mut self, enable: bool) -> Self {
        self.profile_verification = enable;
        self
    }

    /// Builder: Enable CBGR profiling
    pub fn with_memory_profiling(mut self, enable: bool) -> Self {
        self.profile_memory = enable;
        self
    }

    /// Builder: Set optimization level
    pub fn with_optimization(mut self, level: u8) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    /// Builder: Set LTO mode
    pub fn with_lto(mut self, mode: LtoMode) -> Self {
        self.lto = true;
        self.lto_mode = Some(mode);
        self
    }

    /// Builder: Enable static linking
    pub fn with_static_link(mut self, enable: bool) -> Self {
        self.static_link = enable;
        self
    }

    /// Builder: Strip symbols from binary
    pub fn with_strip_symbols(mut self, enable: bool) -> Self {
        self.strip_symbols = enable;
        self
    }

    /// Builder: Strip debug info only
    pub fn with_strip_debug(mut self, enable: bool) -> Self {
        self.strip_debug = enable;
        self
    }

    /// Builder: Set emit mode
    pub fn with_emit_mode(mut self, mode: EmitMode) -> Self {
        self.emit_mode = mode;
        self
    }

    /// Builder: Set output format
    pub fn with_output_format(mut self, format: OutputFormat) -> Self {
        self.output_format = format;
        self
    }

    /// Is this a debug build?
    pub fn is_debug(&self) -> bool {
        self.optimization_level == 0
    }

    /// Is this a release build?
    pub fn is_release(&self) -> bool {
        self.optimization_level >= 2
    }

    /// Should emit colored output?
    pub fn use_color(&self) -> bool {
        use is_terminal::IsTerminal;
        matches!(self.output_format, OutputFormat::Human) && std::io::stdout().is_terminal()
    }

    /// Builder: Set target triple
    pub fn with_target(mut self, target: impl Into<Text>) -> Self {
        self.target_triple = Some(target.into());
        self
    }

    /// Builder: Add a cfg feature flag
    pub fn with_cfg_feature(mut self, feature: impl Into<Text>) -> Self {
        self.cfg_features.push(feature.into());
        self
    }

    /// Builder: Add multiple cfg features
    pub fn with_cfg_features(mut self, features: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        for f in features {
            self.cfg_features.push(f.into());
        }
        self
    }

    /// Builder: Add a custom cfg key-value pair
    pub fn with_cfg_custom(mut self, key: impl Into<Text>, value: impl Into<Text>) -> Self {
        self.cfg_custom.push((key.into(), value.into()));
        self
    }

    /// Builder: Enable test mode
    pub fn with_test_mode(mut self, enabled: bool) -> Self {
        self.test_mode = enabled;
        self
    }

    /// Builder: Enable script mode for the entry source.
    ///
    /// When enabled, the entry file referenced by [`Self::input`] is parsed
    /// with top-level statements allowed; the parser folds them into a
    /// synthesised `__verum_script_main` wrapper that the entry-detection
    /// phase uses as the program entry. Sources beginning with a `#!`
    /// shebang are auto-detected as scripts independently of this flag.
    /// See the field documentation on [`Self::script_mode`].
    pub fn with_script_mode(mut self, enabled: bool) -> Self {
        self.script_mode = enabled;
        self
    }

    /// Builder: Set lint configuration
    pub fn with_lint_config(mut self, config: LintConfig) -> Self {
        self.lint_config = config;
        self
    }

    /// Check if no_alloc constraint is enabled (from cfg flags)
    pub fn is_no_alloc(&self) -> bool {
        self.cfg_features.iter().any(|f| f.as_str() == "no_alloc")
    }

    /// Check if no_std constraint is enabled (from cfg flags)
    pub fn is_no_std(&self) -> bool {
        self.cfg_features.iter().any(|f| f.as_str() == "no_std")
    }

    /// Check if embedded mode is enabled (from cfg flags)
    pub fn is_embedded(&self) -> bool {
        self.cfg_features.iter().any(|f| f.as_str() == "embedded")
    }

    /// Check if CBGR static-only mode is enabled
    pub fn is_cbgr_static_only(&self) -> bool {
        self.cfg_features.iter().any(|f| f.as_str() == "cbgr_static_only")
    }

    /// Check if GPU is disabled
    pub fn is_no_gpu(&self) -> bool {
        self.cfg_features.iter().any(|f| f.as_str() == "no_gpu")
    }

    /// Convert compiler options to target profile for dependency analysis
    pub fn to_target_profile(&self) -> crate::phases::dependency_analysis::TargetProfile {
        use crate::phases::dependency_analysis::TargetProfile;

        let embedded = self.is_embedded();
        TargetProfile {
            name: self.target_triple.clone().unwrap_or_else(|| "default".into()),
            no_alloc: self.is_no_alloc() || embedded,
            no_std: self.is_no_std() || embedded,
            embedded,
            cbgr_static_only: self.is_cbgr_static_only() || embedded,
            no_gpu: self.is_no_gpu() || embedded,
        }
    }

    /// Builder: Enable strict intrinsics mode (missing intrinsics become errors)
    pub fn with_strict_intrinsics(mut self, enable: bool) -> Self {
        self.lint_config.strict_intrinsics = enable;
        self
    }

    /// Builder: Treat all warnings as errors
    pub fn with_deny_warnings(mut self, enable: bool) -> Self {
        self.lint_config.deny_warnings = enable;
        self
    }

    // Gradual Verification Builders (verum_verification integration)

    /// Builder: Enable bounds check elimination
    ///
    /// When enabled, the verifier uses SMT to prove array accesses are within bounds,
    /// eliminating unnecessary runtime bounds checks in generated code.
    pub fn with_bounds_elimination(mut self, enable: bool) -> Self {
        self.enable_bounds_elimination = enable;
        self
    }

    /// Builder: Enable CBGR elimination
    ///
    /// When enabled, escape analysis promotes `&T` references to `&checked T`
    /// when the verifier can prove the reference doesn't escape its scope.
    pub fn with_cbgr_elimination(mut self, enable: bool) -> Self {
        self.enable_cbgr_elimination = enable;
        self
    }

    /// Builder: Enable proof certificate generation
    ///
    /// When enabled, generates machine-verifiable proof certificates in the
    /// specified format (Coq, Lean, or Dedukti).
    pub fn with_proof_certificate(mut self, enable: bool) -> Self {
        self.emit_proof_certificate = enable;
        self
    }

    /// Builder: Set proof certificate format
    pub fn with_proof_certificate_format(mut self, format: impl Into<Text>) -> Self {
        self.proof_certificate_format = Some(format.into());
        self
    }

    /// Builder: Set proof certificate output path
    pub fn with_proof_certificate_path(mut self, path: PathBuf) -> Self {
        self.proof_certificate_path = Some(path);
        self
    }

    // V-LLSI Profile Builders

    /// Builder: Set language profile.
    ///
    /// - **Application**: Safe, productive development (default). VBC-interpretable.
    /// - **Systems**: Performance-critical, optional unsafe. NOT VBC-interpretable.
    /// - **Research**: Experimental features. VBC-interpretable.
    ///
    /// V-LLSI architecture: Application (safe, VBC-interpretable), Systems
    /// (performance-critical, AOT-only), Research (experimental, VBC-interpretable).
    pub fn with_profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    /// Builder: Set Systems profile for low-level programming.
    ///
    /// Systems profile enables raw pointers, inline assembly, and no-libc linking.
    /// Systems profile code is NOT VBC-interpretable - AOT compilation required.
    pub fn with_systems_profile(mut self) -> Self {
        self.profile = Profile::Systems;
        self
    }

    /// Builder: Set Research profile for experimental features.
    ///
    /// Research profile enables dependent types, formal proofs, and linear types.
    /// Research profile code IS VBC-interpretable.
    pub fn with_research_profile(mut self) -> Self {
        self.profile = Profile::Research;
        self
    }

    /// Check if the current profile is VBC-interpretable.
    ///
    /// Systems profile is NOT interpretable (AOT only).
    pub fn is_vbc_interpretable(&self) -> bool {
        self.profile.is_vbc_interpretable()
    }
}

/// Get number of CPUs for parallel compilation
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
