//! Compilation Pipeline Phases
//!
//! This module implements all phases of the Verum compilation pipeline
//! Multi-pass compilation phases: parsing, meta registry, macro expansion,
//!
//! ## VBC-First Pipeline (Target Architecture)
//!
//! ### Common Pipeline (Source → TypedAST)
//!
//! - **Phase 1**: Lexical Analysis & Parsing
//! - **Phase 2**: Meta Registry & AST Registration
//! - **Phase 3**: Macro Expansion & Literal Processing
//! - **Phase 3a**: Contract Verification (SMT-based)
//! - **Phase 4**: Semantic Analysis (Type Inference, CBGR Tier Analysis)
//! - **Phase 4b**: Context System Validation
//!
//! ### Backend Pipeline (TypedAST → Execution)
//!
//! - **Phase 5**: VBC Code Generation (TypedAST → VBC bytecode)
//! - **Phase 6**: VBC Monomorphization (Generic specialization)
//! - **Phase 7**: VBC Execution (Interpreter Tier 0 | AOT Tier 1)
//! - **Phase 7.5**: Final Linking (for AOT)
//!
//! ## Legacy MIR Pipeline (Deprecated - for verification only)
//!
//! MIR infrastructure exists only for:
//! - SMT-based verification
//! - CBGR analysis
//! - Advanced optimization passes
//!
//! **NOT** used in main compilation path.
//!
//! ## Key Features
//!
//! - VBC-first: All tiers share the same VBC intermediate representation
//! - CBGR tier analysis determines reference safety levels
//! - Context system requires explicit 'using' declarations
//! - Persistent monomorphization cache for fast recompilation
//!
//! Multi-pass compilation pipeline: Parse → Meta Registry → Macro Expansion →
//! Contract Verification → Semantic Analysis → HIR → MIR → Optimization → Codegen.
//! VBC-first execution: Source → VBC → Interpreter (Tier 0) or LLVM AOT (Tier 1).

pub mod autodiff_compilation;
pub mod cfg_constructor;
pub mod codegen_tiers;
pub mod context_validation;
pub mod contract_verification;
pub mod dependency_analysis;
pub mod contract_verification_diagnostics;
pub mod entry_detection;
pub mod ffi_boundary;
pub mod lexical_parsing;
pub mod linking;
pub mod macro_expansion;
pub mod meta_registry_phase;
pub mod mir_lowering;
pub mod optimization;
pub mod phase0_stdlib;
pub mod proof_erasure;
pub mod proof_verification;
pub mod semantic_analysis;
pub mod send_sync_validation;
pub mod vbc_codegen;
// Note: vbc_execution.rs was removed - Phase 7 execution is handled directly in pipeline.rs
// via phase_interpret() (Tier 0) and run_native_compilation() (Tier 1).
// See pipeline.rs "Phase 7: VBC Execution" comment block for architecture details.
pub mod vbc_mono;
pub mod verification_phase;
pub mod verified_contract;

use anyhow::Result;
use std::time::Duration;
use verum_diagnostics::Diagnostic;
use verum_common::{List, Text};

pub use verified_contract::{
    ContractTarget, RegistryStats, VerifiedContract, VerifiedContractRegistry,
};

// Re-export the full internal MIR module structure for use by other phases
// This enables proper optimization capabilities through the pipeline
pub use mir_lowering::MirModule;

// Re-export linking types
pub use linking::{FinalLinker, LinkingConfig, LTOConfig, OutputKind};

// Re-export stdlib types
pub use phase0_stdlib::{Phase0CoreCompiler, StdlibArtifacts};

// Re-export VBC phases for VBC-first pipeline
pub use vbc_codegen::VbcCodegenPhase;
pub use vbc_mono::VbcMonomorphizationPhase;
// Note: VbcExecutionPhase was removed - execution is in pipeline.rs directly

// Re-export verification phase for full verification pipeline integration
pub use verification_phase::{
    BoundsEliminationResults, CBGROptimizationResults, SmtVerificationResults,
    VerificationPhase, VerificationPhaseConfig, VerificationPhaseResults,
};

// Re-export dependency analysis for embedded constraints
pub use dependency_analysis::{
    DependencyAnalyzer, ItemRequirements, TargetError, TargetProfile,
};

/// Common trait for all compilation phases
pub trait CompilationPhase: Send + Sync {
    /// Name of the phase (for logging and diagnostics)
    fn name(&self) -> &str;

    /// Description of what this phase does
    fn description(&self) -> &str;

    /// Execute the phase
    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>>;

    /// Can this phase be parallelized with others?
    fn can_parallelize(&self) -> bool {
        false
    }

    /// Get performance metrics for this phase
    fn metrics(&self) -> PhaseMetrics;
}

/// Input for a compilation phase
///
/// Note: Session reference is managed separately by the pipeline
/// to avoid lifetime issues with Clone. Access to Session for span conversion
/// should be provided via the compilation phase implementation.
#[derive(Debug, Clone)]
pub struct PhaseInput {
    /// Phase-specific data
    pub data: PhaseData,

    /// Shared context across phases
    pub context: PhaseContext,
}

/// Phase-specific data
#[derive(Debug, Clone)]
pub enum PhaseData {
    /// Source files (Phase 0-1)
    SourceFiles(List<Text>),

    /// Parsed AST modules (Phase 2-3)
    AstModules(List<verum_ast::Module>),

    /// AST modules with verified contracts (Phase 3a output → Phase 4 input)
    AstModulesWithContracts {
        modules: List<verum_ast::Module>,
        verification_results: VerificationResults,
    },

    /// High-level IR (Phase 4-5)
    Hir(List<HirModule>),

    /// Mid-level IR (Phase 6)
    /// Uses the full internal MIR structure from mir_lowering for proper optimization
    Mir(List<mir_lowering::MirModule>),

    /// Optimized IR (Phase 7)
    /// Uses the full internal MIR structure from mir_lowering
    OptimizedMir(List<mir_lowering::MirModule>),

    /// VBC bytecode modules (VBC-first pipeline)
    ///
    /// This variant is used when compiling directly to VBC bytecode
    /// instead of going through MIR. All tier analysis happens before
    /// VBC generation, and the bytecode includes tier-aware instructions.
    Vbc(List<VbcModuleData>),
}

/// VBC module data with tier analysis results.
///
/// Produced by the VBC codegen phase after tier analysis.
#[derive(Debug, Clone)]
pub struct VbcModuleData {
    /// The compiled VBC module.
    pub module: verum_vbc::module::VbcModule,

    /// Statistics from tier analysis.
    pub tier_stats: VbcTierStats,
}

/// Statistics from VBC tier analysis and codegen.
#[derive(Debug, Clone, Default)]
pub struct VbcTierStats {
    /// Number of Tier 0 references (runtime checked, ~15ns).
    pub tier0_refs: usize,
    /// Number of Tier 1 references (compiler proven safe, 0ns).
    pub tier1_refs: usize,
    /// Number of Tier 2 references (unsafe, 0ns).
    pub tier2_refs: usize,
    /// Tier 1 promotion rate (tier1 / total).
    pub promotion_rate: f64,
}

/// Results from contract verification phase
#[derive(Debug, Clone, Default)]
pub struct VerificationResults {
    /// Verified contracts by function name
    pub verified_contracts: List<VerifiedContract>,

    /// Verification statistics
    pub stats: contract_verification::VerificationStats,

    /// Was verification successful overall?
    pub success: bool,
}

/// High-level intermediate representation (HIR) module.
///
/// HIR is the typed AST representation produced by Phase 4 (Semantic Analysis).
/// It contains all type information, resolved names, and validated contracts.
///
/// Phase 4: Semantic analysis with bidirectional type checking, refinement
/// subsumption (syntactic + SMT), reference validation, context resolution.
#[derive(Debug, Clone)]
pub struct HirModule {
    pub name: Text,
    pub items: List<HirItem>,
    /// FFI boundaries declared in this module
    pub ffi_boundaries: List<verum_ast::ffi::FFIBoundary>,
    /// Verified contracts from Phase 3a
    pub verified_contracts: List<VerifiedContract>,
    /// Context requirements for this module
    pub context_requirements: List<Text>,
}

/// A typed item in the HIR.
#[derive(Debug, Clone)]
pub struct HirItem {
    pub name: Text,
    pub kind: HirItemKind,
    /// Span for error reporting
    pub span: verum_ast::Span,
    /// Visibility of this item
    pub visibility: HirVisibility,
}

/// Visibility in HIR
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HirVisibility {
    /// Public to all
    Public,
    /// Public within crate
    PublicCrate,
    /// Private (default)
    #[default]
    Private,
}

/// Kind of HIR item, matching the AST ItemKind but with type information resolved.
#[derive(Debug, Clone)]
pub enum HirItemKind {
    /// Function declaration with resolved types
    Function(HirFunction),
    /// Type declaration
    Type(HirTypeDecl),
    /// Const declaration with evaluated value
    Const(HirConst),
    /// Static variable declaration
    Static(HirStatic),
    /// Protocol (trait) declaration
    Protocol(HirProtocol),
    /// Implementation block
    Impl(HirImpl),
    /// Module declaration
    Module(HirModule),
    /// Import statement (resolved)
    Import(HirImport),
    /// Context declaration
    Context(HirContext),
    /// Predicate declaration
    Predicate(HirPredicate),
}

/// HIR function with resolved types
#[derive(Debug, Clone)]
pub struct HirFunction {
    /// Resolved return type
    pub return_type: HirType,
    /// Parameter types (resolved)
    pub param_types: List<HirType>,
    /// Whether this is async
    pub is_async: bool,
    /// Whether this is a meta function
    pub is_meta: bool,
    /// Context requirements
    pub contexts: List<Text>,
    /// Verified contracts (preconditions/postconditions)
    pub contracts: List<VerifiedContract>,
}

/// HIR type declaration
#[derive(Debug, Clone)]
pub struct HirTypeDecl {
    /// Generic parameters (resolved)
    pub generics: List<Text>,
    /// The underlying type
    pub underlying: HirType,
}

/// HIR const declaration
#[derive(Debug, Clone)]
pub struct HirConst {
    /// Resolved type
    pub ty: HirType,
    /// Evaluated constant value (if available)
    pub value: Option<HirConstValue>,
}

/// HIR static declaration
#[derive(Debug, Clone)]
pub struct HirStatic {
    /// Resolved type
    pub ty: HirType,
    /// Is mutable
    pub is_mut: bool,
}

/// HIR protocol (trait) declaration
#[derive(Debug, Clone)]
pub struct HirProtocol {
    /// Required methods
    pub methods: List<Text>,
    /// Associated types
    pub associated_types: List<Text>,
}

/// HIR impl block
#[derive(Debug, Clone)]
pub struct HirImpl {
    /// Target type
    pub target_type: HirType,
    /// Protocol being implemented (if any)
    pub protocol: Option<Text>,
    /// Implemented methods
    pub methods: List<Text>,
}

/// HIR import (resolved)
#[derive(Debug, Clone)]
pub struct HirImport {
    /// Fully resolved path
    pub resolved_path: Text,
    /// What is being imported
    pub imported_names: List<Text>,
}

/// HIR context declaration
#[derive(Debug, Clone)]
pub struct HirContext {
    /// Is async context
    pub is_async: bool,
    /// Methods
    pub methods: List<Text>,
}

/// HIR predicate declaration
#[derive(Debug, Clone)]
pub struct HirPredicate {
    /// Return type (should be Bool)
    pub return_type: HirType,
    /// Parameter types
    pub param_types: List<HirType>,
}

/// HIR type representation
#[derive(Debug, Clone)]
pub enum HirType {
    /// Primitive type
    Primitive(HirPrimitiveType),
    /// Named type (user-defined or stdlib)
    Named(Text),
    /// Generic type instantiation
    Generic { name: Text, args: List<HirType> },
    /// Reference type with CBGR tier
    Reference {
        tier: HirRefTier,
        inner: Box<HirType>,
        is_mut: bool,
    },
    /// Function type
    Function {
        params: List<HirType>,
        ret: Box<HirType>,
    },
    /// Tuple type
    Tuple(List<HirType>),
    /// Unit type
    Unit,
    /// Inferred (not yet resolved - should not appear in final HIR)
    Infer,
}

/// HIR primitive types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirPrimitiveType {
    Bool,
    Int,
    Float,
    Text,
    Char,
    Never,
}

/// HIR reference tier (CBGR)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirRefTier {
    /// Tier 0: Full CBGR checks (~15ns overhead)
    Tier0Managed,
    /// Tier 1: Compiler-proven safe (0ns overhead)
    Tier1Checked,
    /// Tier 2: Manual safety proof required (0ns overhead)
    Tier2Unsafe,
}

/// HIR constant value
#[derive(Debug, Clone)]
pub enum HirConstValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(Text),
    Unit,
}

// NOTE: MirModule, MirBlock, MirInstruction types have been removed.
// PhaseData::Mir and PhaseData::OptimizedMir now use the full internal
// mir_lowering::MirModule structure directly, which enables proper
// optimization capabilities including:
// - CBGR check elimination (requires function/local access)
// - Bounds check elimination (requires statement-level analysis)
// - Function inlining (requires cross-function access)
// - Loop vectorization hints (requires CFG analysis)
// - SBGL optimizations (requires escape analysis on full MIR)

/// Shared context across phases
#[derive(Debug, Clone)]
pub struct PhaseContext {
    /// Language profile (Application/Systems/Research)
    pub profile: LanguageProfile,

    /// Target execution tier
    pub target_tier: ExecutionTier,

    /// Verification mode
    pub verify_mode: VerifyMode,

    /// Optimization level
    pub opt_level: OptimizationLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageProfile {
    Application,
    Systems,
    Research,
}

/// Execution tier for Verum compilation.
///
/// Verum uses a two-tier model:
/// - Interpreter: VBC bytecode execution for development/debugging
/// - Aot: Native code via LLVM for production
///
/// Note: JIT infrastructure (in verum_codegen/src/mlir/jit/) is preserved
/// as an internal implementation detail of the AOT pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionTier {
    /// Tier 0: VBC interpreter (~100ns CBGR overhead)
    #[default]
    Interpreter,

    /// Tier 1: AOT compilation via LLVM
    Aot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    Runtime,
    Proof,
    Auto,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationLevel {
    O0, // No optimization
    O1, // Basic optimization
    O2, // Standard optimization
    O3, // Aggressive optimization
}

/// Output from a compilation phase
#[derive(Debug, Clone)]
pub struct PhaseOutput {
    /// Phase-specific data
    pub data: PhaseData,

    /// Any warnings generated
    pub warnings: List<Diagnostic>,

    /// Performance metrics
    pub metrics: PhaseMetrics,
}

/// Performance metrics for a phase
#[derive(Debug, Clone, Default)]
pub struct PhaseMetrics {
    /// Phase name
    pub phase_name: Text,

    /// Time taken to execute
    pub duration: Duration,

    /// Number of items processed
    pub items_processed: usize,

    /// Memory allocated (bytes)
    pub memory_allocated: usize,

    /// Phase-specific metrics
    pub custom_metrics: List<(Text, Text)>,
}

impl PhaseMetrics {
    pub fn new(phase_name: impl Into<Text>) -> Self {
        Self {
            phase_name: phase_name.into(),
            duration: Duration::from_secs(0),
            items_processed: 0,
            memory_allocated: 0,
            custom_metrics: List::new(),
        }
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    pub fn with_items_processed(mut self, count: usize) -> Self {
        self.items_processed = count;
        self
    }

    pub fn with_memory_allocated(mut self, bytes: usize) -> Self {
        self.memory_allocated = bytes;
        self
    }

    pub fn add_custom_metric(&mut self, key: impl Into<Text>, value: impl Into<Text>) {
        self.custom_metrics.push((key.into(), value.into()));
    }

    /// Generate a report of the metrics
    pub fn report(&self) -> Text {
        let mut report = format!(
            "Phase: {}\n  Duration: {:.2}ms\n  Items: {}\n  Memory: {} KB\n",
            self.phase_name,
            self.duration.as_millis(),
            self.items_processed,
            self.memory_allocated / 1024
        );

        for (key, value) in &self.custom_metrics {
            report.push_str(&format!("  {}: {}\n", key, value));
        }

        report.into()
    }

    /// Convert to PhasePerformanceMetrics for compilation profiling
    ///
    /// This is used to integrate phase metrics into the overall compilation
    /// profiling report. Percentages will be calculated by the report.
    pub fn to_performance_metrics(&self) -> crate::compilation_metrics::PhasePerformanceMetrics {
        use verum_common::Map;

        let mut custom_map: Map<Text, Text> = Map::new();
        for (key, value) in &self.custom_metrics {
            custom_map.insert(key.clone(), value.clone());
        }

        crate::compilation_metrics::PhasePerformanceMetrics {
            phase_name: self.phase_name.clone(),
            duration: self.duration,
            memory_allocated: self.memory_allocated,
            items_processed: self.items_processed,
            time_percentage: 0.0, // Will be calculated by CompilationProfileReport
            memory_percentage: 0.0, // Will be calculated by CompilationProfileReport
            custom_metrics: custom_map,
        }
    }
}

/// Convert a verum_ast::Span to a verum_diagnostics::Span (LineColSpan).
///
/// This function provides proper span conversion using the Session's source file
/// cache. It performs efficient O(log n) lookup via binary search on line starts.
///
/// # Performance
///
/// - Conversion time: < 1ms (typically ~100ns)
/// - Binary search on cached line start positions
/// - Graceful fallback for missing source files or synthetic spans
///
/// # Arguments
///
/// * `ast_span` - The byte-offset span from AST
/// * `session_opt` - Optional reference to the compilation session for source lookup
///
/// # Returns
///
/// A LineColSpan with 1-indexed line/column numbers for diagnostic display.
/// Returns placeholder if session is None or source file not found.
///
/// # Examples
///
/// ```ignore
/// use crate::session::Session;
/// use verum_ast::Span;
///
/// let session = Session::new(options);
/// let file_id = session.load_file(Path::new("test.vr"))?;
/// let ast_span = Span::new(0, 10, file_id);
/// let diag_span = ast_span_to_diagnostic_span(ast_span, Some(&session));
/// assert_eq!(diag_span.file, "test.vr");
/// ```
pub fn ast_span_to_diagnostic_span(
    ast_span: verum_ast::Span,
    session_opt: Option<&crate::session::Session>,
) -> verum_diagnostics::Span {
    match session_opt {
        Some(session) => session.convert_span(ast_span),
        None => {
            // Fallback when session is not available (shouldn't happen in normal flow)
            // This preserves backward compatibility for tests without full session
            verum_diagnostics::Span::new("<unknown>", 1, 1, 1)
        }
    }
}

/// Convert a TypeError to a Diagnostic with proper source location information.
///
/// This function uses the session to convert AST byte-offset spans to
/// line/column diagnostic spans, enabling file:line:column error reporting.
///
/// # Examples
///
/// ```ignore
/// use crate::session::Session;
/// use verum_types::TypeError;
///
/// let session = Session::new(options);
/// let type_error: TypeError = /* ... */;
/// let diagnostic = type_error_to_diagnostic(&type_error, Some(&session));
/// // diagnostic now includes proper file:line:column information
/// ```
pub fn type_error_to_diagnostic(
    error: &verum_types::TypeError,
    session_opt: Option<&crate::session::Session>,
) -> verum_diagnostics::Diagnostic {
    // Create a span converter closure if we have a session
    if let Some(session) = session_opt {
        error.to_diagnostic_with_span(Some(|ast_span| session.convert_span(ast_span)))
    } else {
        // Fallback to diagnostic without span information
        error.to_diagnostic()
    }
}
