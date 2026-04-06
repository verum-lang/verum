// CBGR Memory Safety System - Compile-Time Analysis
// Suppress clippy lints - this is a low-level analysis system where many
// clippy warnings are intentional design decisions.
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]
#![allow(clippy::cargo)]
#![allow(unexpected_cfgs)]
#![allow(dead_code)]
//! CBGR (Counter-Based Garbage Rejection) Compile-Time Analysis
//!
//! Verum uses a two-tier reference system with ThinRef (16 bytes: ptr + generation + epoch_caps)
//! and FatRef (32 bytes: adds metadata + offset for slices/traits). Three reference types exist:
//!   - `&T` (managed): Runtime CBGR validation (~15ns), default for application code
//!   - `&checked T`: Zero-cost (0ns), requires compile-time proof via escape analysis
//!   - `&unsafe T`: Zero-cost (0ns), manual safety proof required, must be in @unsafe function
//! CBGR uses epoch-based generation tracking with atomic acquire-release semantics for
//! thread-safe validation. Escape analysis automatically promotes &T to &checked T when
//! all four criteria are met: no escape, no concurrent access, allocation dominates uses,
//! and lifetime is stack-bounded.
//!
//! This crate provides **compile-time analysis** for the CBGR memory safety system.
//! Runtime CBGR checks are emitted as inline LLVM IR by `verum_codegen` (platform_ir.rs).
//!
//! # Three-Tier Safety Model
//!
//! - **Tier 0 (&T)**: CBGR-managed references with runtime checks (~15ns overhead)
//! - **Tier 1 (&checked T)**: Statically-verified references (0ns overhead)
//! - **Tier 2 (&unsafe T)**: Unsafe references (0ns overhead, manual safety)
//!
//! # Compile-Time Analysis Components
//!
//! - [`tier_types`]: Unified tier types (ReferenceTier, Tier0Reason, TierStatistics)
//! - [`tier_analysis`]: Main tier analyzer for VBC integration
//! - [`escape_analysis`]: Forward dataflow escape analysis
//! - [`points_to_analysis`]: Andersen-style points-to analysis
//! - [`dominance_analysis`]: Dominance-based promotion decisions
//!
//! # Codegen (Moved to verum_vbc)
//!
//! CBGR codegen abstractions are now in `verum_vbc::cbgr`:
//! - `DereferenceCodegen`, `CapabilityCheckCodegen`, `CbgrDereferenceStrategy`
//!
//! # Usage
//!
//! For compile-time analysis:
//!
//! ```ignore
//! use verum_cbgr::{EscapeAnalysisConfig, EnhancedEscapeAnalyzer};
//! ```

// ============================================================================
// Compile-time analysis modules
// ============================================================================

pub mod analysis;
pub mod array_analysis;
pub mod call_graph;
pub mod concurrency_analysis;
pub mod context_enhancements;
pub mod diagnostics;
pub mod dominance_analysis;
pub mod escape_analysis;
pub mod escape_categories;
pub mod escape_codegen_integration;
pub mod field_heap_tracking;
pub mod flow_functions;
pub mod ir_call_extraction;
pub mod lifetime_analysis;
pub mod loop_unrolling;
pub mod nll_analysis;
pub mod ownership_analysis;
pub mod points_to_analysis;
pub mod polonius_analysis;
pub mod predicate_abstraction;
pub mod promotion_decision;
pub mod smt_alias_verification;
pub mod ssa;
pub mod tier_analysis;
pub mod tier_types;
pub mod type_analysis;
pub mod value_tracking;
pub mod z3_feasibility;

// ============================================================================
// Re-exports: Compile-time analysis types
// ============================================================================

// Unified tier types (primary API)
pub use tier_types::{
    CbgrTier, ReferenceId, ReferenceTier, Tier0Reason, TierStatistics,
};

// CFG builder for tier analysis
pub use analysis::{CfgBuilder, DefSite, Span, UseeSite};

// Escape analysis categories
pub use escape_categories::{
    EscapeCategory, EscapePatternDetector, OptimizationDecision, SbglDiagnostic, categorize_escape,
};

// SSA types for escape analysis
pub use ssa::{
    DefKind, PhiNode, SsaBuildable, SsaBuilder, SsaError, SsaEscapeInfo, SsaFunction, SsaValue,
    UseSiteKey,
};

// Call graph types for interprocedural escape analysis
pub use call_graph::{
    CallEdge, CallGraph, CallGraphBuilder, CallSite, FunctionSignature,
    InterproceduralEscapeResult, RefFlow, analyze_interprocedural_escapes,
};

// Z3 feasibility checker
pub use z3_feasibility::{
    CacheStats, FeasibilityResult, Z3FeasibilityChecker, Z3FeasibilityCheckerBuilder,
};

// SMT alias verification types
pub use smt_alias_verification::{
    ArrayIndex, PointerConstraint, SmtAliasCache, SmtAliasResult, SmtAliasVerifier,
    SmtAliasVerifierBuilder,
};

// Predicate abstraction types
pub use predicate_abstraction::{
    AbstractPredicate, AbstractionConfig, AbstractionStats, AbstractorBuilder, PathAbstractionExt,
    PredicateAbstractor,
};

// Array analysis types
pub use array_analysis::{
    ArrayAccess, ArrayAnalysisStats, ArrayIndexAnalyzer, BinOp, IndexRange, InductionVariable,
    SymbolicIndex, VarId,
};

// Value tracking types
pub use value_tracking::{
    BinaryOp as ValueBinaryOp, ConcreteValue, PathPredicate, PropagationStats, SymbolicValue,
    ValuePropagator, ValueRange, ValueState, ValueTrackingConfig, ValueTrackingResult,
};

// Loop unrolling types
pub use loop_unrolling::{
    InductionVar, IterationInfo, LoopInfo, LoopUnroller, UnrollConfig, UnrolledLoop, UnrollingStats,
};

// Type analysis types
pub use type_analysis::{
    FieldInfo, FieldLayout, TypeAliasAnalyzer, TypeAliasResult, TypeCache, TypeCacheStats, TypeInfo,
};

// Points-to analysis types
pub use points_to_analysis::{
    FieldId, FieldLocation, LocationId, LocationType, PointsToAnalysisResult,
    PointsToAnalysisStats, PointsToAnalyzer, PointsToAnalyzerBuilder, PointsToConstraint,
    PointsToGenerationResult, PointsToGenerationStats, PointsToGraph, PointsToSet,
    PointsToSolveResult, VarId as PtsVarId, points_to_graph_to_alias_sets,
    reference_points_to_heap,
};

// IR call extraction types
pub use ir_call_extraction::{
    CallArgMapping, ExtractionStats, IrCallExtractor, IrCallInfo, IrCallSite, IrFunction,
    IrInstruction, IrOperand,
};

// Flow function types
pub use flow_functions::{
    FieldEffect, FieldFlowInfo, FieldFlowSummary, FieldPath, FlowFunction, FlowFunctionCompiler,
    FlowFunctionStats, FlowState, FunctionFieldSummary, InterproceduralFieldFlow,
    InterproceduralFlowStats, IrOperation, SsaId, build_flow_function, field_flow_across_call,
    merge_flow_states,
};

// Context enhancement types
pub use context_enhancements::{
    AbstractContext, AdaptiveDepthPolicy, AliasState, CallPattern, CompressionStats,
    ContextCompressor, ContextEquivalenceClass, DataflowState, EnhancedContextConfig,
    EnhancedStats, FlowSensitiveContext, ImportanceMetrics, Predicate,
    build_flow_sensitive_contexts, compute_importance_metrics,
};

// Field heap tracking types
pub use field_heap_tracking::{
    FieldHeapInfo, FieldHeapResult, FieldHeapTracker, HeapSiteId, HeapStore, HeapTrackingStatistics,
};

// Enhanced escape analysis types (Section 2.3)
pub use escape_analysis::{
    EnhancedEscapeAnalyzer, EscapeAnalysisConfig, EscapeAnalysisResult, EscapeAnalysisStats,
    EscapeKind, EscapePoint, EscapeState, SourceLocation,
};

// Escape codegen integration types
pub use escape_codegen_integration::{CodegenOptimizationStats, EscapeAwareCodegen};

// Dominance analysis types (Phase 3)
pub use dominance_analysis::{
    DominanceInfo, DominanceStats, EscapeCategory as DominanceEscapeCategory,
    PromotionDecision as DominancePromotionDecision, ReferenceInfo, decide_promotion,
};

// Promotion decision engine types (Phase 4)
pub use promotion_decision::{
    CodegenDirective, CodegenTier, EngineBuilder as PromotionEngineBuilder, PromotionConfig,
    PromotionDecisionEngine, PromotionDecisionStats,
};

// Unified tier analysis API (VBC integration)
pub use tier_analysis::{
    TierAnalysisConfig, TierAnalysisResult, TierAnalyzer, analyze_tiers, analyze_tiers_with_config,
};

// Ownership analysis for compile-time memory safety (Phase 6)
pub use ownership_analysis::{
    AllocId, AllocationInfo, AllocationKind, DeallocationKind, DeallocationSite,
    DoubleFreeWarning, LeakReason, LeakWarning, OwnershipAnalysisConfig, OwnershipAnalysisResult,
    OwnershipAnalyzer, OwnershipState, OwnershipStats, OwnershipTransfer, TransferKind,
    UseAfterFreeWarning,
};

// Concurrency analysis for data race detection (Phase 6)
pub use concurrency_analysis::{
    AccessKind, ConcurrencyAnalysisConfig, ConcurrencyAnalysisResult, ConcurrencyAnalyzer,
    ConcurrencyStats, DataRaceReason, DataRaceWarning, DeadlockKind, DeadlockWarning, LockId,
    LocationId as ConcurrencyLocationId, MemoryAccess, MemoryOrdering, SyncKind, SyncOperation,
    ThreadId, ThreadSafetyKind, ThreadSafetyViolation, VectorClock,
};

// Lifetime analysis for borrow checking (Phase 7)
pub use lifetime_analysis::{
    BorrowChecker, BorrowError, BorrowKind, BorrowRecord, BorrowState, ConstraintKind,
    ConstraintOrigin, Lifetime, LifetimeAnalysisConfig, LifetimeAnalysisResult, LifetimeAnalyzer,
    LifetimeConstraint, LifetimeId, LifetimeKind, LifetimeStats, LifetimeViolation, ProgramPoint,
    Region, RegionId, ViolationKind,
};

// Non-Lexical Lifetimes (NLL) analysis (Phase 8)
pub use nll_analysis::{
    BorrowData, BorrowId, BorrowSet, LiveRange, LivenessInfo, NllAnalysisResult, NllAnalyzer,
    NllBorrowKind, NllConfig, NllConstraint, NllConstraintKind, NllPoint, NllRegion, NllRegionId,
    NllRegionKind, NllStats, NllViolation, NllViolationKind, PointKind, TwoPhaseBorrowManager,
    UniversalElement,
};

// Polonius-style origin analysis (Phase 9)
pub use polonius_analysis::{
    InputFacts, Loan, LoanId, LoanKind, MoveTracker, OriginId, OutputFacts, PoloniusAnalysisResult,
    PoloniusAnalyzer, PoloniusConfig, PoloniusError, PoloniusErrorKind, PoloniusPoint, PoloniusStats,
};

// Diagnostics integration (converts CBGR warnings to verum_diagnostics)
pub use diagnostics::{
    codes as diagnostic_codes, generate_diagnostics, generate_diagnostics_with_config, has_errors,
    diagnostic_count, CbgrDiagnostics, DiagnosticsConfig,
};

// Comprehensive security and integration tests
#[cfg(test)]
mod security_tests;
