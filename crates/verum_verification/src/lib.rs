#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(unexpected_cfgs)]
//! # Verum Gradual Verification System
//!
//! This crate implements Verum's gradual verification system, providing a smooth
//! transition from runtime checking to compile-time formal verification.
//!
//! ## Three-Level Verification System
//!
//! 1. **Runtime (dynamic)**: Quick runtime checks with ~5-15ns overhead
//!    - Default mode for development and prototyping
//!    - All safety checks executed at runtime
//!    - Immediate feedback during testing
//!
//! 2. **Static (compile-time)**: SMT verification at compile time, 0ns runtime
//!    - Conservative static analysis proves safety
//!    - Checks eliminated in AOT-compiled code when proven safe
//!    - Fallback to runtime checks if proof incomplete
//!
//! 3. **Proof (formal)**: Full formal proofs with proof objects
//!    - SMT solver generates complete correctness proofs
//!    - Mathematical guarantees of safety properties
//!    - Optional proof certificate generation
//!
//! ## Gradual Transition Mechanism
//!
//! The system supports seamless migration between verification levels:
//! - Start with `@verify(runtime)` for rapid prototyping
//! - Gradually add `@verify(static)` for performance-critical code
//! - Use `@verify(proof)` for critical safety requirements
//!
//! ## Architecture
//!
//! - [`level`]: Verification level types and traits
//! - [`context`]: Verification context and boundary tracking
//! - [`transition`]: Gradual transition between verification levels
//! - [`cost`]: Cost reporting and verification decision making
//! - [`boundary`]: Trusted/untrusted code boundaries
//! - [`integration`]: Integration with type system and SMT
//!
//! ## Example
//!
//! ```no_run
//! use verum_verification::{VerificationLevel, VerificationContext};
//!
//! // Create verification context
//! let mut ctx = VerificationContext::new();
//!
//! // Verify a function with gradual verification
//! // (Full example code omitted - see tests for complete examples)
//! ```
//!
//! # Design Principles
//!
//! This implementation follows the Verum verification system design:
//! - Three-level gradual verification: runtime -> static -> proof
//! - Conservative static analysis: safety checks are either proven unnecessary
//!   at compile time or executed at runtime; Verum never speculates on safety
//! - SMT-backed contract verification via weakest precondition calculus
//! - Refinement types integration: types carry predicates (e.g., Int{> 0})
//!   that compose with contract specifications for SMT solving

#![deny(missing_debug_implementations)]
#![deny(rust_2018_idioms)]
#![allow(missing_docs)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]

pub mod boundary;
pub mod bounds_elimination;
pub mod cbgr_elimination;
pub mod context;
pub mod contract;
pub mod cost;
pub mod dependent_verification;
pub mod hoare_logic;
pub mod integration;
pub mod kernel_recheck;
pub mod level;
pub mod lock_ordering;
pub mod math_structures;
pub mod metrics;
pub mod passes;
pub mod proof_validator;
pub mod separation_logic;
pub mod ssa;
pub mod subsumption;
pub mod tactic_evaluation;
pub mod tactic_heuristics;
pub mod tensor_shapes;
pub mod transition;
pub mod vcgen;

// Re-export main types
pub use boundary::{
    // Diagnostics
    BoundaryDiagnostic,
    BoundaryDirection,
    BoundaryKind,
    CallEdge,
    // Call graph types
    CallGraph,
    CallGraphBuilder,
    CallGraphNode,
    CallGraphStats,
    DetectedBoundary,
    DiagnosticSeverity,
    FunctionId,
    FunctionsByLevel,
    ObligationGenerator,
    ObligationKind,
    ProofObligation,
    RequiredObligation,
    SourceLocation,
    TrustedBoundary,
    UntrustedBoundary,
    generate_boundary_diagnostics,
};
pub use bounds_elimination::{
    ArrayAccess,
    ArrayBounds,
    BinaryOp,
    BoundsCheckEliminator,
    // Errors
    BoundsError,
    // Core types
    CheckDecision,
    // Dataflow
    DataflowAnalyzer,
    Definition,
    // Statistics
    EliminationStats,
    Expression,
    IndexConstraint,
    // Loop support
    LoopId,
    LoopInvariant,
    // Meta parameters
    MetaConstraint,
    // Refinement integration
    Refinement,
    ValueRange,
    // Public API
    analyze_bounds_check,
    analyze_function_bounds,
    compute_elimination_stats,
};
pub use cbgr_elimination::{
    BasicBlock as EscapeBasicBlock, CBGROptimizer, ControlFlowGraph as EscapeCFG,
    EscapeAnalysisResult, EscapeStatus, Function as EscapeFunction, OptimizationConfig,
    RefVariable, Scope as EscapeScope, analyze_escape, can_eliminate_check, optimize_function,
    prove_scope_validity,
};
pub use context::{ScopeId, VerificationBoundary, VerificationContext, VerificationScope};
pub use contract::{
    ContractBinOp,
    ContractClause,
    // Error types
    ContractError,
    ContractExpr,
    // Parser
    ContractParser,
    // Translation and instrumentation
    ContractSmtTranslator,
    // Contract AST types
    ContractSpec,
    ContractUnOp,
    InstrumentedContract,
    OldExpr,
    Predicate as ContractPredicate,
    QuantifierBinding,
    QuantifierRange,
    RuntimeInstrumenter,
    contract_to_smtlib,
    generate_contract_vcs,
    instrument_contract,
    // Public API functions
    parse_contract,
    parse_contract_no_validate,
    validate_contract,
};
pub use cost::{
    CostModel, CostReport, CostThreshold, DecisionCriteria, VerificationCost, VerificationDecision,
};
pub use hoare_logic::{
    Command,
    // Frame rule
    FrameRule,
    FunctionBody,
    FunctionContract,
    HoareLogic,
    HoareStats,
    // Core types
    HoareTriple,
    VCKind as HoareVCKind,
    // Supporting types
    VerificationCondition as HoareVC,
    WPCalculator,
    // Errors
    WPError,
    apply_frame,
    generate_vc as hoare_generate_vc,
    // Public API functions
    wp as hoare_wp,
};
pub use integration::{
    CodegenIntegration, HeapCounterexample, HoareVerificationResult, HoareZ3Verifier,
    SepLogicVerificationResult, SeparationLogicZ3Verifier, SmtIntegration, TypeSystemIntegration,
    VarSort,
};
pub use kernel_recheck::{KernelRecheck, KernelRecheckError};
pub use level::{
    ProofLevel, RuntimeLevel, StaticLevel, VerificationConfig, VerificationLevel, VerificationMode,
};
pub use lock_ordering::{
    HeldLocks, LockAcquisition, LockAcquisitionGraph, LockInfo, LockLevel, LockOrderingConfig,
    LockOrderingError, LockOrderingResult, LockOrderingStats, LockOrderingVerifier, LockRegistry,
    LockTypeId, SourceLocation as LockSourceLocation, verify_lock_acquisition,
};
pub use math_structures::{
    Axiom,
    // Category theory
    Category,
    CompactnessDefinition,
    // Analysis
    CompleteOrderedField,
    ContinuityDefinition,
    ContinuousFunction,
    Field,
    Functor,
    // Algebra
    GroupBuilder,
    Homomorphism,
    // Lemmas and verification
    Lemma,
    LemmaDatabase,
    LimitDefinition,
    MathOperation,
    // Core structures
    MathStructure,
    MathStructureVerifier,
    NaturalTransformation,
    // Number theory
    NumberTheory,
    ProofMethod,
    Ring,
    StructureCategory,
    Subgroup,
    Theorem,
    // Topology
    TopologicalSpace,
    VectorSpace,
    category_grp,
    category_set,
    // Standard structures
    integer_addition_group,
    real_field,
    vector_space_r2,
};
pub use metrics::{
    CodeMetricsCollector,
    CoverageData,
    EnhancedCodeMetrics,
    GitHistory,
    MetricsError,
    MetricsResult,
    ProfilingData,
    // Convenience functions
    analyze_function as analyze_function_metrics,
    analyze_loop_nesting,
    analyze_module as analyze_module_metrics,
    // CFG analysis functions
    calculate_cyclomatic_complexity,
    complexity_from_cfg,
    nesting_from_cfg,
};
pub use passes::{
    KernelRecheckPass, SmtVerificationPass, SmtVerificationResult, SmtVerificationStats,
    TransitionRecommendation, TransitionRecommendationPass, VCStatus, VCVerificationResult,
    VerificationError, VerificationPass, VerificationPipeline, VerificationResult,
};
pub use proof_validator::{
    CertificateFormat, HypothesisContext, ProofCertificateGenerator, ProofValidator,
    ValidationConfig, ValidationError, ValidationResult,
};
pub use separation_logic::{
    // Core types
    Address,
    CbgrSepLogic,
    FrameRule as SepFrameRule,
    Heap as SepHeap,
    HeapCommand,
    SepLogicEncoder,
    SepProp,
    StandardPredicates,
    SymbolicState,
    Value as SepValue,
    generate_heap_vcs,
    verify_triple as verify_sep_triple,
    // Public API functions
    wp_heap,
};
pub use subsumption::{
    CompareOp, Counterexample, Predicate, SubsumptionChecker, SubsumptionConfig, SubsumptionResult,
    SubsumptionStats, Value, check_subsumption, extract_counterexample, smt_check,
    try_syntactic_check,
};
pub use tactic_evaluation::{
    EvaluationStats, Goal, GoalMetadata, Hypothesis, HypothesisSource, ProofState, TacticConfig,
    TacticError, TacticEvaluator, TacticResult,
};
pub use tensor_shapes::{
    // Constraint system types
    ConstraintCheckResult,
    // Core types
    Dimension,
    DimensionConstraint,
    DimensionConstraintSystem,
    DimensionEqualityResult,
    MetaParam,
    // Error types
    ShapeError,
    ShapeResult,
    // Verifier
    ShapeVerifier,
    TensorShape,
    VerificationConfig as TensorVerificationConfig,
};
pub use transition::{
    CodeMetrics, MigrationPath, MigrationStep, TransitionAnalyzer, TransitionDecision,
    TransitionStrategy,
};
pub use vcgen::{
    // Contract attribute parsing
    ContractContext,
    CounterExample as VCCounterExample,
    // Core types
    Formula,
    FunctionSignature,
    SmtBinOp,
    SmtExpr,
    SmtUnOp,
    // Helper types
    SourceLocation as VCSourceLocation,
    SymbolTable,
    // Generator
    VCGenerator,
    VCKind,
    VCResult,
    VarType,
    Variable as VCVariable,
    VerificationCondition,
    // Public API functions
    generate_vcs,
    substitute,
    vc_to_smtlib,
    wp,
};
// Re-export SourceLocation from vcgen (contract module uses it)
// Note: boundary::SourceLocation and vcgen::SourceLocation are the same conceptually

use thiserror::Error;
use verum_common::Text;

/// Result type for verification operations
pub type Result<T> = std::result::Result<T, VerificationError>;

/// Errors that can occur during verification
#[derive(Debug, Error)]
pub enum Error {
    /// Verification failed with counterexample
    #[error("verification failed: {reason}\n{counterexample}")]
    VerificationFailed {
        /// Reason for failure
        reason: String,
        /// Counterexample that violates the property
        counterexample: Text,
    },

    /// Verification timeout
    #[error("verification timeout after {timeout_ms}ms")]
    Timeout {
        /// Timeout duration in milliseconds
        timeout_ms: u64,
    },

    /// Verification level mismatch at boundary
    #[error("verification level mismatch: expected {expected:?}, found {actual:?}")]
    LevelMismatch {
        /// Expected verification level
        expected: VerificationLevel,
        /// Actual verification level
        actual: VerificationLevel,
    },

    /// Missing proof obligation at boundary
    #[error("missing proof obligation: {obligation}")]
    MissingObligation {
        /// Description of the missing obligation
        obligation: Text,
    },

    /// Invalid transition between verification levels
    #[error("invalid verification transition: {from:?} -> {to:?}: {reason}")]
    InvalidTransition {
        /// Source verification level
        from: VerificationLevel,
        /// Target verification level
        to: VerificationLevel,
        /// Reason the transition is invalid
        reason: String,
    },

    /// Cost budget exceeded
    #[error("verification cost budget exceeded: {actual_ms}ms > {budget_ms}ms")]
    BudgetExceeded {
        /// Actual cost in milliseconds
        actual_ms: u64,
        /// Budget limit in milliseconds
        budget_ms: u64,
    },

    /// Integration error with type system
    #[error("type system integration error: {0}")]
    TypeSystem(Text),

    /// Integration error with SMT solver
    #[error("SMT integration error: {0}")]
    Smt(Text),

    /// Integration error with codegen
    #[error("codegen integration error: {0}")]
    Codegen(Text),

    /// Internal error
    #[error("internal verification error: {0}")]
    Internal(Text),
}
