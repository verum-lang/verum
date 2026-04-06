#![allow(unexpected_cfgs)]
// #![allow(dead_code)]
// Suppress informational clippy lints
#![allow(clippy::result_large_err)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::type_complexity)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::collapsible_match)]
// Suppress pedantic lints that don't affect correctness
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::vec_init_then_push)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::redundant_closure)]
#![allow(clippy::unnecessary_map_or)]
#![allow(clippy::single_match)]
#![allow(clippy::needless_return)]
#![allow(clippy::derivable_impls)]
#![allow(clippy::iter_kv_map)]
#![allow(clippy::while_let_on_iterator)]
#![allow(clippy::useless_format)]
#![allow(clippy::useless_conversion)]
#![allow(clippy::manual_map)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::manual_strip)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::new_without_default)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::match_like_matches_macro)]
#![allow(clippy::option_map_or_none)]
#![allow(clippy::unnecessary_filter_map)]
#![allow(clippy::match_single_binding)]
#![allow(clippy::match_ref_pats)]
#![allow(clippy::map_entry)]
#![allow(clippy::redundant_pattern_matching)]
#![allow(clippy::len_zero)]
#![allow(clippy::iter_cloned_collect)]
#![allow(clippy::iter_next_slice)]
#![allow(clippy::unnecessary_lazy_evaluations)]
#![allow(clippy::only_used_in_recursion)]
#![allow(clippy::single_match_else)]
#![allow(clippy::question_mark)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::redundant_guards)]
#![allow(clippy::let_and_return)]
#![allow(clippy::unwrap_or_default)]
#![allow(clippy::for_kv_map)]
#![allow(clippy::explicit_into_iter_loop)]
#![allow(clippy::wrong_self_convention)]
#![allow(clippy::option_map_unit_fn)]
#![allow(clippy::unnecessary_fold)]
#![allow(clippy::manual_flatten)]
#![allow(clippy::bind_instead_of_map)]
#![allow(clippy::unused_enumerate_index)]
//! Verum Compiler Driver Library
//!
//! Complete implementation of the VBC-first compilation pipeline as specified in
//! Multi-pass compilation pipeline with VBC-first architecture (Source → VBC → Execution).
//!
//! ## VBC-First Architecture
//!
//! The Verum compiler uses a **VBC-first** (Verum Bytecode first) architecture where
//! all execution tiers share the same intermediate representation:
//!
//! ```text
//! Source → TypedAST → VBC Bytecode → { Interpreter (Tier 0) | AOT (Tier 1) }
//! ```
//!
//! This enables:
//! - Unified IR for all execution modes
//! - Fast development iteration via interpreter
//! - Seamless tier promotion at runtime
//! - Consistent semantics across tiers
//!
//! ## Pipeline Phases
//!
//! ### Common Frontend (Source → TypedAST)
//!
//! - **Phase 0**: stdlib Preparation & Entry Point Detection
//! - **Phase 1**: Lexical Analysis & Parsing (verum_fast_parser)
//! - **Phase 2**: Meta Registry & AST Registration
//! - **Phase 3**: Macro Expansion & Literal Processing
//! - **Phase 3a**: Contract Verification (SMT-based)
//! - **Phase 4**: Semantic Analysis (Type Inference, CBGR Tier Analysis)
//! - **Phase 4a**: Autodiff Compilation
//! - **Phase 4b**: Context System Validation
//!
//! ### Backend (TypedAST → Execution)
//!
//! - **Phase 5**: VBC Code Generation (TypedAST → VBC bytecode)
//! - **Phase 6**: VBC Monomorphization (Generic specialization)
//! - **Phase 7**: Execution (Interpreter Tier 0 | AOT Tier 1 via LLVM)
//! - **Phase 7.5**: Final Linking (for AOT)
//!
//! ## Key Features
//!
//! - **CBGR Profiling**: Track and report CBGR overhead (<15ns per check)
//! - **SMT Verification**: Contract verification with Z3/CVC5
//! - **Two-Tier Execution (v2.1)**: Interpreter (Tier 0), AOT (Tier 1) with graceful fallback
//! - **Incremental Compilation**: Fast recompilation with smart caching
//! - **Rich Diagnostics**: Colorized errors with source snippets
//! - **Profile System**: Application/Systems/Research progressive complexity
//!
//! ## Performance Targets
//!
//! - Compilation: > 50K LOC/sec
//! - Type checking: < 100ms/10K LOC
//! - CBGR overhead: < 15ns per check
//! - Check elimination: 50-90% (typical)
//!
//! ## Note on MIR Infrastructure
//!
//! The `phases::mir_lowering` module contains an experimental MIR (Mid-level IR)
//! implementation used for:
//! - SMT-based verification (`phases::verification_phase`)
//! - Advanced optimization passes (`phases::optimization`)
//! - CBGR analysis integration (`passes::cbgr_integration`)
//!
//! MIR is NOT part of the main compilation pipeline; it exists for verification
//! and analysis purposes only. The actual compilation path goes directly from
//! TypedAST to VBC bytecode.
//!
//! # Example Usage
//!
//! ## Compiling User Code
//!
//! ```ignore
//! use verum_compiler::{Session, CompilerOptions, CompilationPipeline};
//! use std::path::PathBuf;
//!
//! let options = CompilerOptions {
//!     input: PathBuf::from("main.vr"),
//!     output: PathBuf::from("main"),
//!     ..Default::default()
//! };
//!
//! let mut session = Session::new(options);
//! let mut pipeline = CompilationPipeline::new(&mut session);
//!
//! // Compile source string
//! pipeline.compile_string("fn main() { print(\"Hello!\"); }")?;
//! ```
//!
//! ## Compiling stdlib (Bootstrap Mode)
//!
//! ```ignore
//! use verum_compiler::{Session, CompilationPipeline, CoreConfig};
//!
//! let config = CoreConfig::new("stdlib")
//!     .with_output("target/stdlib.vbca");
//!
//! let mut session = Session::default();
//! let mut pipeline = CompilationPipeline::new_core(&mut session, config);
//!
//! let result = pipeline.compile_core()?;
//! println!("Compiled {} modules", result.modules_compiled);
//! ```

#![allow(missing_docs)]
#![deny(rust_2018_idioms)]

// Public API (clean interface for external use)
pub mod api;

// Core modules
pub mod compilation_metrics;
pub mod interpolation;
pub mod lint;
pub mod literal_parsers;
pub mod literal_registry;
pub mod options;
pub mod pipeline;
pub mod profile_cmd;
pub mod repl;
pub mod session;
pub mod unified_dashboard;
pub mod linker_config;
pub mod verification_config;
pub mod verification_profiler;
pub mod verify_cmd;

// Context system is in verum_runtime::context
// Compile-time analysis uses types from there

// Meta-system modules: unified compile-time computation (meta fn, @derive, @tagged_literal)
pub mod asset_registry;
pub mod derives;
pub mod hygiene;  // Sets-of-scopes hygiene for macro expansion
pub mod meta;  // Consolidated meta-system (context, evaluator, builtins, sandbox, registry, etc.)
pub mod quote;
pub mod quote_macro;
pub mod token_stream;


// Compilation pipeline modules: phases 0-7.5 from parsing through code generation
pub mod compilation_path;  // Dual-path compilation (CPU/GPU) infrastructure
pub mod contract_integration;
pub mod diagnostics_engine;
pub mod graceful_fallback;
pub mod incremental_compiler;
pub mod module_utils;  // Shared module utilities (cfg handling, path conversion)
pub mod passes;
pub mod phases;
pub mod profile_system;
pub mod staged_pipeline;  // N-level staged metaprogramming pipeline
pub mod hash;         // Unified Blake3-based hashing infrastructure
pub mod semantic_query; // Semantic query layer for content-addressed caching
pub mod content_addressed_storage; // Persistent content-addressed storage (CAS) for semantic cache
pub mod core_cache;   // Industrial-grade stdlib compilation caching
pub mod core_compiler;
pub mod core_loader;
pub mod core_source;  // Unified stdlib source abstraction (embedded VFS / local FS)
pub mod embedded_stdlib; // Embedded stdlib archive (zstd-compressed core/*.vr in binary)

// Re-export main types
pub use compilation_metrics::{
    Bottleneck, BottleneckKind, CompilationProfileReport, CompilationStats, ModuleMetrics,
    PhasePerformanceMetrics,
};
pub use literal_registry::{LiteralRegistry, ParsedLiteral, TaggedLiteralHandler};
pub use options::{CompilerOptions, OutputFormat, VerifyMode};
pub use pipeline::{
    BuildMode, CheckResult, CompilationMode, CompilationPipeline, CompilerPass,
    TestExecutionResult, get_cached_stdlib_registry, reset_test_isolation,
};
pub use profile_cmd::ProfileCommand;
pub use repl::Repl;
pub use session::{BuildMetrics, Session};
pub use unified_dashboard::{
    CacheStatistics, CompilationMetrics, DashboardPhaseMetrics, HotSpot, HotSpotKind,
    OutputFormat as DashboardOutputFormat, Recommendation, ReferenceBreakdown, RuntimeMetrics,
    UnifiedDashboard, VerificationCost,
};
pub use linker_config::{
    LinkerSection, LinkerTomlConfig, CogSection, PlatformLinkerSection, ProfileConfig,
    ProjectConfig,
};
pub use verification_config::{VerificationConfig, VerifySection};
pub use verification_profiler::{
    CacheStatistics as ProfilerCacheStats, FileLocation, ProfileEntry, SmtSolver,
    VerificationProfiler, VerificationReport as ProfilerReport,
};
pub use verify_cmd::{
    FunctionResultJson, VerificationReport, VerificationReportJson, VerificationResult,
    VerifyCommand,
};

// Re-export meta-system types (all from consolidated meta/ module)
pub use meta::{
    // Async executor
    MetaAsyncExecutor, ParallelTaskBuilder, TaskDependencyGraph,
    // Context and values
    ConstValue,
    MetaContext,
    MetaError,
    // Core types
    ProtocolImplementation,
    TypeDefinition,
    // Reflection API types (aligned with core/meta/reflection.vr)
    AssociatedTypeInfo,
    FieldInfo,
    FieldOffset,
    FunctionInfo,
    GenericParam,
    GenericParamKind,
    LifetimeParam,
    MethodResolution,
    MethodSource,
    OwnershipInfo,
    ParamInfo,
    PrimitiveType,
    ProtocolInfo,
    SelfKind,
    TraitBound,
    TypeInfo,
    TypeKind,
    VariantInfo,
    VariantKind,
    Visibility,
    // Builtin function type
    BuiltinMetaFn,
    // Metrics
    MetaEvalMetrics,
    // Registry
    MacroDefinition, MacroKind, MetaFunction, MetaRegistry,
    // Sandbox
    MetaSandbox, SandboxOperation, SandboxError,
    // Value operations
    MetaValueOps,
};
pub use quote::{ToTokens, TokenStream};
pub use quote_macro::{
    MacroError, MacroExpansionContext, MacroResult, create_quote_context,
    create_quote_context_with_repeats, meta_quote, meta_unquote, quote_expr, quote_with_context,
    tokenstream_from_str, unquote_stream,
};

// Re-export pipeline types
pub use diagnostics_engine::DiagnosticsEngine;
pub use graceful_fallback::GracefulFallback;
pub use incremental_compiler::{CacheStats, IncrementalCompiler, TypeCheckResult};
pub use phases::{
    CompilationPhase, ExecutionTier, LanguageProfile, OptimizationLevel, PhaseContext, PhaseInput, PhaseMetrics,
    PhaseOutput, VerifyMode as PhaseVerifyMode,
};
pub use profile_system::{Feature, Profile, ProfileManager};

// Re-export stdlib loader types
pub use core_loader::{
    convert_archive_to_metadata, load_archive, load_archive_from_bytes, load_core_metadata,
    load_core_metadata_from_bytes, CoreLoadError,
};

// Re-export stdlib compiler types (for unified pipeline)
pub use core_compiler::{StdlibCompilationResult, CoreConfig};

// Re-export stdlib cache types (industrial-grade caching)
pub use core_cache::{
    CoreCache, CoreCacheEntry, CoreCacheKey, CoreCacheStore,
    CachedCoreMetadata, CachedTypeEntry, CachedFunctionEntry, CachedModuleEntry,
    // Meta-system cache types
    CachedMetaFunctionEntry, CachedMetaParam, CachedMacroEntry, CachedDeriveEntry,
    init_global_cache, global_cache, global_cache_or_init,
};

// Re-export staged pipeline types (N-level metaprogramming)
pub use staged_pipeline::{
    GeneratedFragment, StageCache, StagedConfig, StagedFunction, StagedPipeline, StagedResult,
    StagedStats,
};

// Re-export public API types
pub use api::{
    CommonPipelineConfig, CommonPipelineResult, CompilationArtifacts, CompilationError,
    CompilationErrorKind, CompilationResult, CompilerConfig, OutputType, PipelineMetrics,
    SourceFile,
};


/// Compiler version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Compiler build information
pub const BUILD_INFO: &str = concat!(
    "verum_compiler v",
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("CARGO_PKG_REPOSITORY"),
    ")"
);
