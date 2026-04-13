#![allow(unexpected_cfgs)]
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
// Intentional patterns:
// - `only_used_in_recursion`: recursive helper functions need consistent parameter signatures
// - `should_implement_trait`: custom from_str methods may have different return types than std::str::FromStr
// - `single_match`: sometimes match is clearer than if-let for future extensibility
// - `match_single_binding`: sometimes match is clearer for documentation purposes
// - `doc_lazy_continuation`: complex documentation formatting
// - `doc_overindented_list_items`: intentional alignment in proofs/equations
// - `manual_unwrap_or_default`: explicit error handling is clearer for SMT verification
// - `redundant_guards`: sometimes guards make the code clearer
// - `double_must_use`: explicit #[must_use] on guard-returning functions documents intent
// - `needless_range_loop`: sometimes indexing is clearer for matrix operations
// - `new_without_default`: not all types want Default semantics
// - `manual_strip`: explicit slicing is sometimes clearer
// - `manual_ok_err`: explicit match is clearer for complex error handling
// - `while_let_loop`: complex SCC algorithm benefits from explicit loop structure
#![allow(clippy::only_used_in_recursion)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::single_match)]
#![allow(clippy::match_single_binding)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::doc_overindented_list_items)]
#![allow(clippy::manual_unwrap_or_default)]
#![allow(clippy::manual_unwrap_or)]
#![allow(clippy::redundant_guards)]
#![allow(clippy::double_must_use)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::new_without_default)]
#![allow(clippy::manual_strip)]
#![allow(clippy::manual_ok_err)]
#![allow(clippy::while_let_loop)]
//! Verum Type System with Bidirectional Type Checking
//!
//! This crate implements Verum's complete type system including:
//! - **Bidirectional type checking** (3x faster than Algorithm W)
//! - **Refinement types** with SMT integration and five binding rules
//! - **Three-tier reference model** (&T, &checked T, &unsafe T)
//! - **Hindley-Milner inference** with let-polymorphism
//! - **Protocol system** (traits/type classes)
//! - **Context tracking** through types
//! - **Meta parameters** for compile-time computation
//!
//! # Refinement Types (Refinement types with gradual verification: types can carry predicates (Int{> 0}) verified at compile-time or runtime depending on verification level — )
//!
//! Verum supports five binding rules for refinement types:
//!
//! 1. **Inline refinement** - `Int{> 0}` (implicit `it` binding) - **Preferred**
//! 2. **Lambda-style** - `Int where |x| x > 0` (explicit binding) - **Recommended**
//! 3. **Sigma-type** - `x: Int where x > 0` (canonical form) - **Dependent types**
//! 4. **Named predicate** - `Int where is_positive` (reusable logic) - **Reusable**
//! 5. **Bare where** - `Int where it > 0` (deprecated) - **Backward compatibility**
//!
//! # Three-Tier Reference Model (Three-tier reference model: &T (managed, CBGR ~15ns), &checked T (statically verified, 0ns), &unsafe T (unchecked, 0ns). Memory layouts: ThinRef 16 bytes (ptr+generation+epoch), FatRef 24 bytes (+len) — )
//!
//! - **&T** - CBGR-managed reference (~15ns overhead, runtime safety)
//! - **&checked T** - Statically verified reference (0ns, compile-time proof)
//! - **&unsafe T** - Unsafe reference (0ns, no checks, manual safety)
//!
//! Coercion hierarchy: `&unsafe T <: &checked T <: &T` (implicit upcasts only)
//!
//! # Architecture
//!
//! The type checker operates in two modes:
//! - **Synthesis mode (⇒)**: Infer type from expression
//! - **Checking mode (⇐)**: Check expression against expected type
//!
//! This bidirectional approach provides:
//! - 3-5x faster type checking than traditional unification
//! - Better error messages with context
//! - Fewer type annotations required
//! - Foundation for IDE integration
//!
//! # Key Components
//!
//! - [`ty`]: Type representation (primitives, compounds, refinements)
//! - [`infer`]: **PRIMARY** bidirectional type inference engine
//! - [`unify`]: Type unification algorithm
//! - [`subtype`]: Subtyping with refinement types
//! - [`refinement`]: Refinement type checking (P0 for v1.0!)
//! - [`protocol`]: Protocol (trait) system
//! - [`context`]: Type environment and context management
//!
//! # Performance Targets
//!
//! - **Type checking**: < 100ms for 10K LOC
//! - **Inference speed**: 3-5x faster than Algorithm W
//! - **Memory usage**: < 10% overhead vs naive approach
//! - **Compilation**: > 50K LOC/sec in release mode
//!
//! # Example
//!
//! ```ignore
//! use verum_types::TypeChecker;
//! use verum_ast::expr::Expr;
//!
//! // Create type checker
//! let mut checker = TypeChecker::new();
//!
//! // Infer type from expression (synthesis mode)
//! let expr = /* ... */;
//! let result = checker.infer(&expr)?;
//!
//! // Check expression against expected type (checking mode)
//! let expected = /* ... */;
//! checker.check(&expr, &expected)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Integration Points
//!
//! - **verum_ast**: AST definition and traversal
//! - **verum_parser**: Parsing produces AST
//! - **verum_codegen**: Code generation from typed AST
//! - **verum_error**: Error reporting and diagnostics
//! - **verum_smt**: SMT-backed refinement checking

#![allow(missing_docs)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(dead_code)]

pub mod attr; // Attribute registry and validation (Attribute registry: validation rules for @derive, @verify, @cfg, @repr and other compile-time attributes — )
pub mod core_metadata; // Stdlib type metadata definitions (Stdlib type metadata: definitions extracted from core .vr files during compilation pipeline — )
pub mod core_pipeline; // Dependency-ordered stdlib compilation (Stdlib bootstrap: dependency-ordered compilation of core .vr modules, type metadata extracted from parsed stdlib files — )
pub mod capability; // Capability attenuation for contexts (Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0)
pub mod const_eval;
pub mod context;
pub mod context_check;
pub mod context_resolution; // Context group resolution for type checking
pub mod contract_integration; // Phase 3a → Phase 4 contract integration
pub mod cubical; // Phase B.2: Cubical type theory normalizer
pub mod cubical_bridge; // Phase B.2: EqTerm ↔ CubicalTerm translator
pub mod expr_to_eqterm; // Phase B.6: structured Expr → EqTerm lowering
pub mod qtt_usage; // QTT: Linear/Affine/Omega usage tracker
pub mod instance_search; // Phase D.4: Protocol instance search + coherence
pub mod universe_solver; // Phase A.2: Universe constraint solving
pub mod contracts; // Precondition and postcondition validation
pub mod control_flow; // Flow-sensitive analysis for @must_handle
pub mod dependent_helpers; // Helper methods for dependent type checking
pub mod dependent_integration; // Integration with verum_smt dependent types (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
pub mod dependent_match; // Dependent pattern matching (Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing — )
pub mod error_conversions; // Conversions to verum_error::VerumError
pub mod exhaustiveness; // Pattern exhaustiveness checking (Pattern exhaustiveness checking: ensure match expressions cover all possible values, witness generation for missing patterns — )
pub mod existential; // Existential type inference (Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — )
pub mod implicit; // Implicit argument resolution (Implicit arguments: compiler-inferred function arguments resolved by unification or type class search — )
pub mod infer;
pub(crate) mod infer_path_resolution; // Path resolution methods extracted from infer.rs
pub(crate) mod infer_patterns; // Pattern binding methods extracted from infer.rs
pub mod integer_hierarchy;
pub mod module_context; // Module-level type inference context
pub mod proof_checker; // Proof type checker (Formal proof system (future v2.0+): machine-checkable proofs with tactics (simp, ring, omega, blast, induction), theorem/lemma/corollary statements — )
pub mod projection; // Associated type projection resolution (Associated type bounds: constraining associated types in where clauses (where T.Item: Display)
pub mod protocol;
pub mod references; // Deref protocol implementations for reference types
pub mod refinement;
pub mod refinement_diagnostics;
pub mod refinement_evidence; // Refinement types enhancement: flow-sensitive refinement propagation, evidence tracking for verified predicates — Flow-sensitive refinement propagation
pub mod send_sync;
pub mod smt_backend;
pub mod source_files; // Source file registry for span→line:col conversion
pub mod subtype;
pub mod termination; // Termination checking for recursive functions (Termination checking: ensuring recursive functions terminate via structural recursion on well-founded orderings — )
pub mod ty;
pub mod type_level_computation; // Type-level computation for dependent types (Type-level computation: compile-time evaluation of type expressions, reduction rules, normalization — )
pub mod type_registry; // AST → Type mapping for codegen
pub mod unify;
pub mod variance;

// New features (Spec compliance)
pub mod affine; // Spec 1.12: Affine type system
pub mod aliasing; // Spec 4.2.3: Reference aliasing detection
pub mod annotations; // Spec 1.13-1.14: @must_handle, @cold
pub mod computational_properties; // Computational properties: tracking Pure, IO, Async, Fallible, Mutates as compile-time properties (Pure/IO/Async/etc)
pub mod dependency_injection; // Spec 1.11.7: DI type checking
pub mod di;
pub mod meta_context; // Meta context validation for meta fn (Meta contexts: meta functions have restricted context access (only compile-time-safe contexts) — )
pub mod stage_checker; // Staged metaprogramming validation (N-level meta: meta(N) fn)
pub mod literal_conversion; // Spec 1.4: Protocol-based literals
pub mod where_clause; // Where clause disambiguation: separating value refinements from type constraints // Context system (dependency injection): provide/using for runtime DI, ~5-30ns overhead

// Advanced Protocol Features (Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — )
pub mod advanced_protocol_errors; // Enhanced error messages for advanced protocols
pub mod advanced_protocols; // GATs, Specialization, GenRef, Higher-Kinded Types
pub mod kind_inference; // Higher-Kinded Type Inference (Higher-kinded type (HKT) inference and specialization selection: kind inference for type constructors (Type -> Type), automatic selection of most specific specialization — )
pub mod specialization; // Specialization validation and enforcement (Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering — )
pub mod specialization_selection; // Automatic specialization selection during type inference (Higher-kinded type (HKT) inference and specialization selection: kind inference for type constructors (Type -> Type), automatic selection of most specific specialization — )

// SIMD and Tensor Protocol (Tensor protocol: operations on Tensor<T, Shape> including element-wise ops, reductions, reshaping with compile-time shape validation — )
pub mod simd; // SIMD type validation (Vec<T, N>, Mask<N>, intrinsics)
pub mod tensor_protocol; // FromTensorLiteral protocol for compile-time tensor construction
pub mod tensor_shape_checker; // Tensor shape validation and broadcasting

// Stdlib-agnostic type system architecture
pub mod operator_protocols; // Operator -> Protocol mapping for stdlib-agnostic operators
pub mod method_resolution; // Protocol-based method resolution
pub mod core_integration; // Bridge between stdlib-agnostic architecture and ProtocolChecker
pub mod type_exporter; // Type metadata serialization for separate compilation
pub mod unified_type_error; // Unified type error wrapper for Phase 2 error consolidation

// Tests moved to tests/ directory

// Re-export main types for convenience
// PRIMARY API: Use these for all type checking operations
pub use capability::{
    CapabilityChecker, CapabilityError, CapabilityRequirement, ContextCapabilities, TypeCapability,
    TypeCapabilitySet,
};
pub use context::{
    ModuleId,
    TypeContext,
    TypeEnv,
    TypeScheme,
    // Universe hierarchy tracking (Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — )
    UniverseConstraint,
    UniverseContext,
    UniverseSubstitution,
    UniverseVar,
    // Definite assignment analysis (L0-critical/memory-safety/uninitialized)
    InitState,
    InitTracker,
    PartialInit,
};
pub use contracts::{
    ContractStats, PostconditionError, PostconditionValidator, PreconditionError,
    PreconditionValidator,
};
pub use implicit::{
    ConstraintSource, ImplicitArg, ImplicitConstraint, ImplicitContext, ImplicitElaborator,
    ImplicitResolver,
};
pub use infer::{InferMode, InferResult, TypeChecker};
pub use module_context::{
    DependencyGraph, FunctionTypeInfo, InferenceState, ModuleContext, ModuleInferenceMetrics,
    ModuleTypeInference, TypeSource,
};
pub use proof_checker::ProofChecker;
pub use subtype::Subtyping;
pub use ty::{
    // Dependent Types (Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — )
    CoinductiveDestructor,
    EqConst,
    EqTerm,
    InductiveConstructor,
    PathConstructor,
    PathEndpoints,
    ProjComponent,
    Quantity,
    // Core types
    Substitution,
    Type,
    TypeVar,
    UniverseLevel,
};
pub use type_registry::TypeRegistry;
pub use unify::Unifier;

// Advanced features
pub use const_eval::{ConstEvalError, ConstEvaluator};
pub use verum_common::ConstValue;
pub use dependent_match::{ConstructorRefinement, DependentPatternChecker, Motive};
// Note: ContextChecker types from context_check with unique names to avoid conflict with di module
pub use advanced_protocol_errors::{
    CandidateInfo, GATArityError, GATWhereClauseError, GenerationMismatchError, ImplementId,
    NegativeSpecializationError, SpecializationAmbiguityError,
};
pub use advanced_protocols::{
    // Errors
    AdvancedProtocolError,
    AssociatedTypeGAT,
    AssociatedTypeKind,
    BinaryOp,
    // ConstValue is exported from const_eval module
    // GAT Support
    GATTypeParam,
    GATWhereClause,
    // GenRef
    GenRefType,
    GenerationPredicate,
    // Kind is exported from kind_inference module
    ProtocolBoundPolarity,
    // Refinement Integration
    RefinementConstraint,
    RefinementKind,
    RefinementPredicate as AdvancedRefinementPredicate,
    // Specialization
    SpecializationInfo,
    SpecializationLattice,
    // Variance is exported from variance module
};
pub use context_check::{
    ContextChecker,
    ContextEnv as TwoLevelContextEnv, // Two-Level context environment (check-time)
    ContextRequirement as TwoLevelContextRequirement, // Two-Level context requirements (check-time)
    ContextSet,
    // Advanced context patterns - negative context verification: checking !Ctx constraints across transitive call chains
    NegativeContextViolation,
    ContextAccess,
    collect_context_accesses,
    verify_direct_negative_contexts,
    build_negative_context_map,
    check_function_contexts,
    // Advanced context patterns - call graph analysis: ensuring context requirements are satisfied transitively through all callees
    CallGraph,
    CallGraphNode,
    CallSiteInfo,
    CallChainStep,
    TransitiveViolationInfo,
    ContextPath,
    FunctionContextInfo,
    // Advanced context patterns - module-level alias validation: verifying @using module annotations don't conflict with function-level contexts
    AliasUsage,
    AliasConflict,
    ModuleAliasRegistry,
    validate_module_aliases,
};
pub use integer_hierarchy::{
    CheckedOps, IntegerHierarchy, IntegerKind, OverflowMode, SaturatingOps, WrappingOps,
};
pub use kind_inference::{
    // Kind System
    Kind,
    Kind as KindType, // Alias for backward compatibility
    KindConstraint,
    KindError,
    KindInference,
    // Kind Inference
    KindInferer,
    KindSubstitution,
    // HKT instantiation: applying type constructors to type arguments (e.g., List applied to Int gives List<Int>)
    HKTInstantiationResult,
};
pub use variance::Variance;
pub use projection::{
    check_associated_type_bound, parse_projection, DeferredProjection, Projection,
    ProjectionError, ProjectionResolver, ProjectionResult,
};
pub use protocol::{
    AssociatedConst, AssociatedType, MethodResolution, MethodSource, Protocol, ProtocolBound,
    ProtocolChecker, ProtocolError, ProtocolImpl, ProtocolMethod, UnifiedProtocolError, VTable,
    VTableLayout, WhereClause,
};
pub use refinement::{
    CounterExample, NamedPredicate, RefinementChecker, RefinementConfig, RefinementError,
    RefinementErrorGenerator, RefinementPredicate, RefinementType, SmtBackend, SmtResult,
    VerificationCondition, VerificationResult, VerificationStats,
};
pub use refinement_diagnostics::{
    ConstraintEvaluation, ConstraintResult, ErrorContext, PredicateEvaluator, RefinementDiagnostic,
    RefinementDiagnosticBuilder, RefinementSource, Suggestion, SuggestionGenerator,
};
pub use send_sync::{
    SendSyncDerivation, register_send_sync_protocols, register_standard_send_sync_impls,
};
pub use smt_backend::{BackendStats, Z3Backend, check_subsumption_smt};

// New features (where clause disambiguation, type aliases, HKTs, existential types, type-level strings)
pub use affine::{AffineTracker, check_linear_modifier, check_resource_modifier};
pub use annotations::{
    AnnotationRegistry, ColdFunctionRegistry, MustHandleRegistry, OptimizationHint, ResultUsage,
};
pub use control_flow::{
    BasicBlock, BlockId, ControlFlowGraph, FlowSensitiveChecker, ResultInfo, ResultState,
    Statement, Terminator, VarId,
};
pub use where_clause::{DisambiguatedWhereClause, WhereClauseContext, WhereClauseKind};
// Note: DI module DependencyGraph renamed to avoid conflict with module_context::DependencyGraph
pub use computational_properties::{ComputationalProperty, PropertyInferenceContext, PropertySet};
pub use dependency_injection::{
    DITypeChecker,
    DependencyGraph as DIGraph, // Renamed to avoid conflict
    DependencyRef,
    InjectableMetadata,
    InjectableRegistry,
    Scope,
};
pub use literal_conversion::{LiteralConverter, LiteralProtocol};
pub use references::{CheckedRef, CheckedRefMut, UnsafeRef, UnsafeRefMut};

// Specialization - Specialization: more specific protocol implementations override general ones, with lattice-based specificity ordering
pub use specialization::{OverlapDetector, SpecializationValidationError, SpecializationValidator};

// Specialization Selection - Higher-kinded type (HKT) inference and specialization selection: kind inference for type constructors (Type -> Type), automatic selection of most specific specialization
// Note: ImplementId is exported from advanced_protocol_errors module
pub use specialization_selection::{
    CoherenceChecker, CoherenceViolation, NegativeBoundResult, SelectionStats,
    SpecializationError, SpecializationSelector,
};

// Context System (Dependency Injection) - Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage
// Note: TypeParam renamed to avoid conflict with module_context::TypeParam
pub use di::{
    ContextDecl,
    ContextEnv,
    ContextError,
    ContextGroup,
    ContextGroupRegistry,
    ContextOperation,
    ContextProvider,
    ContextRef, // DI context requirements (runtime)
    ContextRequirement,
    GroupError,
    ProviderError,
    ProviderScope,
    SharedContextEnv,         // DI context environment (runtime)
    TypeParam as DITypeParam, // Renamed to avoid conflict
};

// Tensor Protocol - Tensor protocol: operations on Tensor<T, Shape> including element-wise ops, reductions, reshaping with compile-time shape validation
pub use tensor_protocol::{
    NestedArray, TensorLiteralValidator, create_from_tensor_literal_protocol,
    register_tensor_literal_protocol,
};

// Tensor Shape Checker - SIMD and tensor system: unified Tensor<T, Shape> type with compile-time shape validation, SIMD acceleration (SSE/AVX/NEON), auto-differentiation
pub use tensor_shape_checker::{TensorShapeChecker, TensorShapeError};

// SIMD Type Validation - SIMD type validation: verifying SIMD vector types match hardware capabilities and element type constraints
pub use simd::{
    MultiversionInfo, SimdCheckStats, SimdElementType, SimdErrorKind, SimdMaskInfo,
    SimdTargetFeature, SimdTypeChecker, SimdTypeError, SimdVecInfo, SimdWidth, TargetArch as SimdTargetArch,
};

// Staged Metaprogramming - N-level meta (meta(N) fn)
pub use stage_checker::{
    FunctionStageInfo, StageChecker, StageConfig, StageError, StageWarning,
};

// Dependent Type Integration - Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking
pub use dependent_helpers::{DependentTypeCheckerExt, enable_dependent_types, has_dependent_types};
pub use dependent_integration::{
    DependentTypeChecker, DependentTypeConstraint, DependentVerificationStats,
    SmtDependentTypeChecker,
};

// Termination Checking - Termination checking: ensuring recursive functions terminate via structural recursion on well-founded orderings
pub use termination::{TerminationChecker, TerminationError};

// Unified type error wrapper
pub use unified_type_error::UnifiedTypeError;

use thiserror::Error;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
use verum_common::{List, Text};

/// Type checking errors.
/// Error codes follow Verum specification:
/// - E310-E319: Type system errors (mismatch, inference, etc.)
/// - E320-E329: Memory safety errors (stack overflow, etc.)
/// - E400-E409: Semantic errors (type confusion, invalid cast)
/// - E500-E509: Concurrency errors (data race, Send/Sync)
#[derive(Debug, Error)]
pub enum TypeError {
    #[error("Type mismatch: expected '{expected}', found '{actual}'")]
    Mismatch {
        expected: Text,
        actual: Text,
        span: verum_ast::span::Span,
    },

    #[error("cannot infer type for lambda without annotation")]
    CannotInferLambda { span: verum_ast::span::Span },

    #[error("infinite type: {var} = {ty}")]
    InfiniteType {
        var: Text,
        ty: Text,
        span: verum_ast::span::Span,
    },

    #[error("unbound variable: {name}")]
    UnboundVariable {
        name: Text,
        span: verum_ast::span::Span,
    },

    #[error("not a function type: {ty}")]
    NotAFunction {
        ty: Text,
        span: verum_ast::span::Span,
    },

    #[error("Type mismatch: if-expression branches have incompatible types: then has '{then_ty}', else has '{else_ty}'")]
    BranchMismatch {
        then_ty: Text,
        else_ty: Text,
        span: verum_ast::span::Span,
    },

    #[error("{message}")]
    InvalidIndex {
        message: Text,
        span: verum_ast::span::Span,
    },

    #[error("Protocol constraint not satisfied: '{ty}' does not implement '{protocol}'")]
    ProtocolNotSatisfied {
        ty: Text,
        protocol: Text,
        span: verum_ast::span::Span,
    },

    #[error("refinement constraint not satisfied: {predicate}")]
    RefinementFailed {
        predicate: Text,
        span: verum_ast::span::Span,
    },

    #[error("invalid refinement predicate: {message}")]
    RefinementPredicateInvalid {
        message: Text,
        span: verum_ast::span::Span,
    },

    #[error("context not allowed: {context}")]
    ContextNotAllowed {
        context: Text,
        span: verum_ast::span::Span,
    },

    #[error("const generic mismatch: expected {expected}, found {actual}")]
    ConstMismatch {
        expected: Text,
        actual: Text,
        span: verum_ast::span::Span,
    },

    #[error("ambiguous type: cannot infer without more context")]
    AmbiguousType { span: verum_ast::span::Span },

    #[error("affine type violation: {ty} used more than once")]
    AffineViolation {
        ty: Text,
        first_use: verum_ast::span::Span,
        second_use: verum_ast::span::Span,
    },

    /// Visibility error - attempting to access a private item from outside its module
    /// Visibility control: private (default), public (pub), cog-public (internal), module-scoped pub(module) — Visibility
    #[error("visibility error: '{name}' is {visibility} in module '{module_path}'")]
    VisibilityError {
        name: Text,
        visibility: Text,
        module_path: Text,
        span: verum_ast::span::Span,
    },

    /// Ambiguous name error - the same name is imported from multiple modules
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Import Ambiguity
    #[error("ambiguous name: '{name}' is imported from multiple modules: {sources}")]
    AmbiguousName {
        name: Text,
        sources: Text,
        span: verum_ast::span::Span,
    },

    /// Circular constant dependency error - constants cannot depend on each other cyclically
    /// Constant initialization ordering: topological sort of dependencies, cycle detection for const declarations — Constant Initialization Order
    #[error("circular constant dependency: {cycle_path}")]
    CircularConstantDependency {
        /// The cycle path, e.g., "VALUE_A -> VALUE_B -> VALUE_A"
        cycle_path: Text,
        /// The constants involved in the cycle
        constants_in_cycle: List<Text>,
        span: verum_ast::span::Span,
    },

    #[error("linear type violation: `{ty}` not used exactly once (used {usage_count} times)")]
    LinearViolation {
        ty: Text,
        usage_count: usize,
        span: verum_ast::span::Span,
    },

    #[error("value `{name}` used after move (moved at {moved_at}, used at {used_at})")]
    MovedValueUsed {
        name: Text,
        moved_at: verum_ast::span::Span,
        used_at: verum_ast::span::Span,
    },

    #[error("affine value `{name}` cannot be used in loop (would be moved multiple times)")]
    AffineValueInLoop {
        name: Text,
        binding_span: verum_ast::span::Span,
        use_span: verum_ast::span::Span,
    },

    #[error("value `{name}` used after partial move (field `{moved_field}` was moved)")]
    PartiallyMovedValue {
        name: Text,
        moved_field: Text,
        moved_at: verum_ast::span::Span,
        used_at: verum_ast::span::Span,
    },

    /// Linear value must be consumed exactly once but was not used.
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 6 (Linear Types)
    #[error("linear value `{name}` must be consumed exactly once but was not used")]
    LinearNotConsumed {
        name: Text,
        binding_span: verum_ast::span::Span,
        scope_end: verum_ast::span::Span,
    },

    // Context System errors (Two-Level Model)
    #[error("missing context: {context}")]
    MissingContext {
        context: Text,
        span: verum_ast::span::Span,
    },

    #[error("undefined context: {name}")]
    UndefinedContext {
        name: Text,
        span: verum_ast::span::Span,
    },

    /// E808: Duplicate provide for same context in same scope
    /// Context requirements: functions declare needed contexts with "using [Ctx1, Ctx2]" after return type, callers must provide all — Provide Statements
    #[error("E808: duplicate provide for context '{context}' in same scope")]
    DuplicateProvide {
        context: Text,
        span: verum_ast::span::Span,
    },

    #[error("undefined context method: {context}.{method}")]
    UndefinedContextMethod {
        context: Text,
        method: Text,
        span: verum_ast::span::Span,
    },

    #[error("invalid sub-context: {context}.{sub_context}")]
    InvalidSubContext {
        context: Text,
        sub_context: Text,
        available: List<Text>,
        span: verum_ast::span::Span,
    },

    #[error("context mismatch: expected {expected}, found {actual}")]
    ContextMismatch {
        expected: Text,
        actual: Text,
        span: verum_ast::span::Span,
    },

    #[error("context propagation error: function requires {context} but caller doesn't provide it")]
    ContextPropagationError {
        context: Text,
        callee: Text,
        span: verum_ast::span::Span,
    },

    // Negative Context errors (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    #[error("excluded context violation: cannot use `{context}` which is explicitly excluded")]
    ExcludedContextViolation {
        context: Text,
        span: verum_ast::span::Span,
    },

    #[error("transitive negative context violation: calling `{callee}` uses excluded context `{excluded_context}`")]
    TransitiveNegativeContextViolation {
        excluded_context: Text,
        callee: Text,
        span: verum_ast::span::Span,
    },

    /// Direct negative context violation (E3050)
    /// Function body directly accesses a context that is excluded in the signature.
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    ///
    /// Example:
    /// ```verum
    /// fn pure_compute() using [!Database] {
    ///     Database.query(...);  // E3050: Direct violation
    /// }
    /// ```
    #[error("E3050: direct negative context violation: `{context}` is accessed but excluded via `!{context}`")]
    DirectNegativeContextViolation {
        /// The excluded context that was directly accessed
        context: Text,
        /// Function where the violation occurred
        function_name: Text,
        /// Location of the direct access
        usage_span: verum_ast::span::Span,
        /// Location of the negative constraint declaration
        declaration_span: verum_ast::span::Span,
    },

    /// Context alias conflict within a module (E3060)
    /// Two functions in the same module use the same alias for different contexts.
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
    ///
    /// Example:
    /// ```verum
    /// fn migrate() using [Database as primary] { ... }
    /// fn verify() using [Cache as primary] { ... }  // E3060: Alias 'primary' conflicts
    /// ```
    #[error("E3060: context alias conflict: alias `{alias}` used for different contexts")]
    ContextAliasConflict {
        /// The conflicting alias name
        alias: Text,
        /// First context using this alias
        first_context: Text,
        /// First function using this alias
        first_function: Text,
        /// Location of the first usage
        first_span: verum_ast::span::Span,
        /// Second context using this alias
        second_context: Text,
        /// Second function using this alias
        second_function: Text,
        /// Location of the second usage
        second_span: verum_ast::span::Span,
    },

    #[error("invalid cast: cannot cast {from} to {to}")]
    InvalidCast {
        from: Text,
        to: Text,
        reason: Text,
        span: verum_ast::span::Span,
    },

    #[error("method `{method}` not found for type `{ty}`")]
    MethodNotFound {
        ty: Text,
        method: Text,
        span: verum_ast::span::Span,
    },

    /// Capability violation - method requires a capability not available in the restricted type.
    ///
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 12 - Capability Attenuation as Types
    ///
    /// Example:
    /// ```verum
    /// fn analyze(db: Database with [Read]) -> Stats {
    ///     db.query("SELECT * FROM users");  // OK - query requires Read
    ///     db.delete("DELETE FROM users");   // ERROR - delete requires Write
    /// }
    /// ```
    #[error("method `{method}` on `{type_name}` requires `{required_capability}` capability but only [{available_capabilities:?}] are available")]
    CapabilityViolation {
        /// Method name that was called
        method: Text,
        /// Type name with capability restrictions
        type_name: Text,
        /// The capability required by this method
        required_capability: Text,
        /// Capabilities available on this value
        available_capabilities: List<Text>,
        span: verum_ast::span::Span,
    },

    #[error(
        "method `{method}` requires protocol `{protocol}` but type `{ty}` does not implement it"
    )]
    ProtocolNotImplemented {
        ty: Text,
        protocol: Text,
        method: Text,
        span: verum_ast::span::Span,
    },

    #[error("wrong number of arguments for method `{method}`: expected {expected}, found {actual}")]
    WrongArgCount {
        method: Text,
        expected: usize,
        actual: usize,
        span: verum_ast::span::Span,
    },

    #[error("ambiguous method call: `{method}` could refer to multiple protocols")]
    AmbiguousMethod {
        method: Text,
        candidates: List<Text>,
        span: verum_ast::span::Span,
    },

    // Module system errors
    // Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Name resolution errors
    #[error("type not found: {name}")]
    TypeNotFound {
        name: Text,
        span: verum_ast::span::Span,
    },

    #[error("`{name}` is not a type (it is a {actual_kind})")]
    NotAType {
        name: Text,
        actual_kind: Text,
        span: verum_ast::span::Span,
    },

    // Try operator (?) errors
    // E0204 Multiple conversion paths: when try (?) operator finds multiple From implementations for error conversion, requiring explicit disambiguation — Specialized Diagnostics
    #[error("cannot use '?' operator outside of function context")]
    TryOperatorOutsideFunction { span: verum_ast::span::Span },

    #[error("cannot use '?' on non-Result/Maybe type")]
    TryOnNonResult { ty: Type, span: verum_ast::span::Span },

    #[error("'?' operator error type mismatch")]
    TryOperatorMismatch {
        expr_type: Type,
        return_type: Type,
        span: verum_ast::span::Span,
    },

    #[error("type mismatch in '?' operator: cannot convert {inner_error} to {outer_error}")]
    ResultTypeMismatch {
        inner_error: Text,
        outer_error: Text,
        span: verum_ast::span::Span,
        diagnostic: verum_diagnostics::Diagnostic,
    },

    #[error("multiple conversion paths from {from_type} to {to_type}")]
    MultipleConversionPaths {
        from_type: Text,
        to_type: Text,
        paths: List<Text>,
        span: verum_ast::span::Span,
        diagnostic: verum_diagnostics::Diagnostic,
    },

    #[error("cannot use '?' operator in function returning {function_return_type}")]
    TryInNonResultContext {
        expr_type: Text,
        function_return_type: Text,
        span: verum_ast::span::Span,
        diagnostic: verum_diagnostics::Diagnostic,
    },

    #[error("type {ty} is not Result or Maybe")]
    NotResultOrMaybe {
        ty: Text,
        span: verum_ast::span::Span,
    },

    // Two-pass type resolution errors
    #[error("cyclic type definition detected: {cycle_path}")]
    TypeCycle {
        /// The cycle path, e.g., "A -> B -> C -> A"
        cycle_path: Text,
        /// The types involved in the cycle
        types_in_cycle: List<Text>,
        span: verum_ast::span::Span,
    },

    #[error("unresolved type placeholder: {name}")]
    UnresolvedPlaceholder {
        name: Text,
        span: verum_ast::span::Span,
    },

    #[error("type `{name}` is not yet fully defined (forward reference in same declaration)")]
    IncompleteTypeReference {
        name: Text,
        span: verum_ast::span::Span,
    },

    /// Error: Protocol used in `using [...]` clause is not a context protocol.
    ///
    /// Verum distinguishes between:
    /// - **Constraint protocols**: Used in `where T: Protocol` bounds
    /// - **Context protocols**: Used in `using [Protocol]` dependency injection clauses
    ///
    /// A context protocol must be declared with the `context` modifier:
    /// ```verum
    /// context protocol Database { ... }  // Can be used in `using [Database]`
    /// protocol Comparable { ... }        // Can only be used in `where T: Comparable`
    /// ```
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Context Protocol Validation
    #[error("protocol '{name}' cannot be used as a context")]
    NonContextProtocolInUsing {
        /// Name of the protocol that was incorrectly used in a using clause
        name: Text,
        /// Source location of the using clause
        span: verum_ast::span::Span,
    },

    // ========== Advanced type syntax errors: malformed HKTs, invalid associated type projections, existential type misuse ==========

    /// Existential type escapes its scope
    ///
    /// Existential types: hiding concrete types behind protocol bounds (impl Protocol return types) — Existential Types
    ///
    /// Existential types are opaque - their concrete type cannot escape
    /// the scope where they are unpacked.
    #[error("existential type escapes its scope: {skolem_name} cannot be used outside")]
    ExistentialEscape {
        /// The skolem constant that escaped
        skolem_name: Text,
        /// Where the existential was unpacked
        unpacking_span: verum_ast::span::Span,
        /// Where it escaped
        escape_span: verum_ast::span::Span,
    },

    /// Existential bound not satisfied
    ///
    /// When packing an existential type, the witness must satisfy all bounds.
    #[error("existential bound not satisfied: {witness_type} does not implement {protocol}")]
    ExistentialBoundNotSatisfied {
        witness_type: Text,
        protocol: Text,
        span: verum_ast::span::Span,
    },

    /// Kind mismatch in higher-kinded type application
    ///
    /// Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-Kinded Types
    #[error("kind mismatch: expected {expected_kind}, found {actual_kind}")]
    KindMismatch {
        expected_kind: Text,
        actual_kind: Text,
        type_name: Text,
        span: verum_ast::span::Span,
    },

    /// Wrong arity for type constructor
    ///
    /// E.g., using `List` where `Map` (2 args) is expected
    #[error("type constructor '{name}' has arity {actual_arity}, expected {expected_arity}")]
    TypeConstructorArityMismatch {
        name: Text,
        expected_arity: usize,
        actual_arity: usize,
        span: verum_ast::span::Span,
    },

    /// Cannot resolve associated type projection
    ///
    /// Associated type bounds: constraining associated types in where clauses (where T.Item: Display) — Associated Type Bounds
    #[error("cannot resolve associated type '{assoc_name}' for type '{base_type}'")]
    CannotResolveAssociatedType {
        base_type: Text,
        assoc_name: Text,
        reason: Text,
        span: verum_ast::span::Span,
    },

    /// Ambiguous associated type - multiple implementations provide it
    #[error("ambiguous associated type '{assoc_name}': multiple implementations for '{base_type}'")]
    AmbiguousAssociatedType {
        base_type: Text,
        assoc_name: Text,
        candidates: List<Text>,
        span: verum_ast::span::Span,
    },

    /// Negative bound violated - type implements a forbidden protocol
    ///
    /// Multi-protocol bounds: combining multiple protocol constraints (T: Display + Debug) — Negative Bounds
    #[error("negative bound violated: {ty} implements {protocol}, but {protocol} is forbidden")]
    NegativeBoundViolated {
        ty: Text,
        protocol: Text,
        span: verum_ast::span::Span,
    },

    /// Specialization overlap due to negative bounds conflict
    #[error("specialization overlap: implementations conflict for type '{ty}'")]
    SpecializationOverlap {
        ty: Text,
        impl1: Text,
        impl2: Text,
        span: verum_ast::span::Span,
    },

    /// HKT protocol bound not satisfied
    #[error("higher-kinded bound not satisfied: {type_constructor} does not implement {protocol}")]
    HKTBoundNotSatisfied {
        type_constructor: Text,
        protocol: Text,
        span: verum_ast::span::Span,
    },

    // ========== Definite Assignment Analysis Errors (E201) ==========
    // Spec: L0-critical/memory-safety/uninitialized - Compile-time detection of partial initialization

    /// Use of completely uninitialized variable
    ///
    /// Detected when a variable declared without initializer is used before any assignment.
    ///
    /// Example:
    /// ```verum
    /// let x: Int;
    /// print(x);  // E201: Use of uninitialized variable 'x'
    /// ```
    #[error("E201: use of uninitialized variable '{name}'")]
    UseOfUninitializedVariable {
        /// Name of the uninitialized variable
        name: Text,
        /// Location where the variable is used
        span: verum_ast::span::Span,
    },

    /// Use of partially initialized compound variable
    ///
    /// Detected when a tuple, array, or struct has some but not all elements/fields initialized.
    ///
    /// Example:
    /// ```verum
    /// let tuple: (Int, Int, Int);
    /// tuple.0 = 1;
    /// tuple.1 = 2;
    /// // tuple.2 not initialized
    /// let sum = tuple.0 + tuple.1 + tuple.2;  // E201: Partially initialized variable
    /// ```
    #[error("E201: use of partially initialized variable '{name}' (missing: {missing})")]
    PartiallyInitializedVariable {
        /// Name of the partially initialized variable
        name: Text,
        /// Description of missing initialization (e.g., "field 'age'" or "element 2")
        missing: Text,
        /// Location where the variable is used
        span: verum_ast::span::Span,
    },

    /// Access to uninitialized struct field
    ///
    /// Example:
    /// ```verum
    /// let mut person: Person;
    /// person.name = "Alice";
    /// // person.age not initialized
    /// print(person.age);  // E201: Field 'age' is not initialized
    /// ```
    #[error("E201: field '{field}' of variable '{var}' is not initialized")]
    UninitializedField {
        /// Variable name
        var: Text,
        /// Uninitialized field name
        field: Text,
        /// Location of the access
        span: verum_ast::span::Span,
    },

    /// Access to uninitialized array element
    ///
    /// Example:
    /// ```verum
    /// let mut arr: [Int; 5];
    /// arr[0] = 10;
    /// arr[1] = 20;
    /// let val = arr[2];  // E201: Array element at index 2 is not initialized
    /// ```
    #[error("E201: array element at index {index} of '{var}' is not initialized")]
    UninitializedArrayElement {
        /// Variable name
        var: Text,
        /// Uninitialized index
        index: usize,
        /// Location of the access
        span: verum_ast::span::Span,
    },

    /// Access to uninitialized tuple element
    ///
    /// Example:
    /// ```verum
    /// let mut tuple: (Int, Int, Int);
    /// tuple.0 = 1;
    /// tuple.1 = 2;
    /// let val = tuple.2;  // E201: Tuple element 2 is not initialized
    /// ```
    #[error("E201: tuple element {index} of '{var}' is not initialized")]
    UninitializedTupleElement {
        /// Variable name
        var: Text,
        /// Uninitialized index
        index: usize,
        /// Location of the access
        span: verum_ast::span::Span,
    },

    /// Iteration over partially initialized array
    ///
    /// Example:
    /// ```verum
    /// let mut arr: [Int; 5];
    /// arr[0] = 10;
    /// for elem in arr { ... }  // E201: Cannot iterate over partially initialized array
    /// ```
    #[error("E201: cannot iterate over partially initialized array '{var}'")]
    IterationOverPartialArray {
        /// Variable name
        var: Text,
        /// Missing indices
        missing_indices: List<usize>,
        /// Location of the for loop
        span: verum_ast::span::Span,
    },

    // Reference Aliasing Errors
    // Reference safety invariants: managed refs validated at dereference, checked refs proven safe at compile time, unsafe refs unchecked — Borrow Rules

    /// Borrow conflict: attempting to create conflicting borrows
    ///
    /// Example:
    /// ```verum
    /// let mut x = 42;
    /// let r1 = &mut x;  // Mutable borrow
    /// let r2 = &x;      // E310: Cannot borrow `x` as immutable because it's already mutably borrowed
    /// ```
    #[error("E310: cannot borrow `{var}` - conflicting borrows")]
    BorrowConflict {
        /// Variable being borrowed
        var: Text,
        /// Span of existing borrow
        existing_borrow_span: verum_ast::span::Span,
        /// Whether existing borrow is mutable
        existing_is_mut: bool,
        /// Span of new borrow attempt
        new_borrow_span: verum_ast::span::Span,
        /// Whether new borrow is mutable
        new_is_mut: bool,
    },

    /// Field-level borrow conflict: attempting to borrow a field while another borrow conflicts
    ///
    /// Example:
    /// ```verum
    /// let mut point = Point { x: 1, y: 2 };
    /// let rx = &mut point.x;
    /// let rp = &mut point;  // E311: Cannot borrow `point` while field `x` is borrowed
    /// ```
    #[error("E311: cannot borrow `{var}` because field `{field}` is already borrowed")]
    FieldBorrowConflict {
        /// Variable being borrowed
        var: Text,
        /// Field that's already borrowed
        field: Text,
        /// Span of existing field borrow
        existing_span: verum_ast::span::Span,
        /// Span of new borrow attempt
        new_span: verum_ast::span::Span,
    },

    /// Dangling reference: reference outlives its referent
    ///
    /// Example:
    /// ```verum
    /// fn get_ref() -> &Int {
    ///     let x = 42;
    ///     &x  // E312: `x` does not live long enough
    /// }
    /// ```
    #[error("E312: `{var}` does not live long enough - reference outlives referent")]
    DanglingReference {
        /// Variable that doesn't live long enough
        var: Text,
        /// Span of the reference creation
        ref_span: verum_ast::span::Span,
        /// Span where the variable goes out of scope
        drop_span: verum_ast::span::Span,
    },

    /// &checked reference may escape through function call
    ///
    /// Example:
    /// ```verum
    /// fn capture(r: &Int) -> &Int { r }
    /// fn main() {
    ///     let local = 42;
    ///     let checked_ref: &checked Int = &checked local;
    ///     capture(checked_ref);  // E310: checked ref may escape
    /// }
    /// ```
    #[error("E310: `&checked` reference to `{var}` may escape through function call")]
    CheckedRefEscape {
        /// Variable being referenced
        var: Text,
        /// Span of the escaping reference argument
        span: verum_ast::span::Span,
    },

    /// Cannot move while borrowed
    ///
    /// Example:
    /// ```verum
    /// let x = vec![1, 2, 3];
    /// let r = &x;
    /// let y = x;  // E313: Cannot move `x` while it is borrowed
    /// println("{:?}", r);
    /// ```
    #[error("E313: cannot move `{var}` while it is borrowed")]
    MoveWhileBorrowed {
        /// Variable being moved
        var: Text,
        /// Span of the move
        move_span: verum_ast::span::Span,
        /// Span of the active borrow
        borrow_span: verum_ast::span::Span,
    },

    /// Cannot assign while borrowed
    ///
    /// Example:
    /// ```verum
    /// let mut x = 42;
    /// let r = &x;
    /// x = 100;  // E314: Cannot assign to `x` while it is borrowed
    /// println("{}", r);
    /// ```
    #[error("E314: cannot assign to `{var}` while it is borrowed")]
    AssignWhileBorrowed {
        /// Variable being assigned
        var: Text,
        /// Span of the assignment
        assign_span: verum_ast::span::Span,
        /// Span of the active borrow
        borrow_span: verum_ast::span::Span,
    },

    // =====================================================================
    // Stack Safety Errors (E320-E329)
    // Spec: L0-critical/memory-safety/buffer_overflow/no_stack_overflow
    // =====================================================================

    /// Stack allocation exceeds safe limit (E320)
    ///
    /// Verum prevents stack overflow by enforcing limits on stack-allocated data.
    /// Arrays and large structs should use heap allocation (Heap<T> or List<T>).
    ///
    /// Example:
    /// ```verum
    /// fn bad() {
    ///     let huge: [Int; 134_217_728] = [0; 134_217_728];  // E320
    /// }
    /// ```
    #[error("E320: stack allocation exceeds safe limit ({size} bytes exceeds {limit} byte limit)")]
    StackAllocationExceedsLimit {
        /// Size of the attempted allocation in bytes
        size: u64,
        /// Maximum allowed stack allocation
        limit: u64,
        /// Span of the allocation
        span: verum_ast::span::Span,
    },

    /// Potential stack overflow from unbounded recursion (E321)
    ///
    /// Verum detects functions that may recurse infinitely without a base case.
    /// Use @allow(unbounded_recursion) for intentionally unbounded recursion.
    ///
    /// Example:
    /// ```verum
    /// fn infinite(n: Int) -> Int {
    ///     infinite(n + 1)  // E321: no base case, unbounded recursion
    /// }
    /// ```
    #[error("E321: potential stack overflow from unbounded recursion in function `{func_name}`")]
    UnboundedRecursionDetected {
        /// Name of the recursive function
        func_name: Text,
        /// Span of the recursive call
        span: verum_ast::span::Span,
        /// Optional: functions involved in mutual recursion cycle
        cycle: List<Text>,
    },

    // =====================================================================
    // Import Resolution Errors (E401-E409)
    // Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Import Resolution
    // =====================================================================

    /// Imported item not found in module (E401)
    ///
    /// The specified item does not exist in the target module's exports.
    /// This is a compile-time error that prevents building incorrect code.
    ///
    /// Example:
    /// ```verum
    /// import core.{size_of};  // E401: `size_of` not found in module `core`
    /// ```
    #[error("E401: cannot find `{item_name}` in module `{module_path}`")]
    ImportItemNotFound {
        /// Name of the item that was not found
        item_name: Text,
        /// Path of the module being imported from
        module_path: Text,
        /// Available exports in the module (for suggestions)
        available_items: List<Text>,
        /// Span of the import statement
        span: verum_ast::span::Span,
    },

    /// Module not found during import resolution (E402)
    ///
    /// The specified module path does not correspond to any known module.
    ///
    /// Example:
    /// ```verum
    /// import nonexistent.module.{Item};  // E402: module `nonexistent.module` not found
    /// ```
    #[error("E402: module `{module_path}` not found")]
    ImportModuleNotFound {
        /// Path of the module that was not found
        module_path: Text,
        /// Similar module paths (for suggestions)
        similar_modules: List<Text>,
        /// Span of the import statement
        span: verum_ast::span::Span,
    },

    /// Undefined function call (E403)
    ///
    /// A function is called but not defined or imported in the current scope.
    /// This ensures all function calls can be resolved at compile time.
    ///
    /// Example:
    /// ```verum
    /// fn main() {
    ///     unknown_function();  // E403: undefined function `unknown_function`
    /// }
    /// ```
    #[error("E403: undefined function `{func_name}`")]
    UndefinedFunction {
        /// Name of the undefined function
        func_name: Text,
        /// Similar function names (for suggestions)
        similar_functions: List<Text>,
        /// Span of the function call
        span: verum_ast::span::Span,
    },

    /// Impure meta function (E501)
    ///
    /// Meta functions (`meta fn`) must be pure - they run at compile-time and
    /// cannot have side effects. This error is raised when a meta function's
    /// body contains impure operations.
    ///
    /// Meta function purity: meta functions are implicitly pure (no IO, no mutation of non-meta state) — Meta functions are implicitly pure
    ///
    /// Example:
    /// ```verum
    /// meta fn bad_meta() {
    ///     print("hello");  // E501: meta function cannot have IO
    /// }
    /// ```
    #[error("E501: meta function `{func_name}` must be pure but has side effects: {properties}")]
    ImpureMetaFunction {
        /// Name of the meta function
        func_name: Text,
        /// The impure properties detected (e.g., "IO, Mutates")
        properties: Text,
        /// Span of the function declaration
        span: verum_ast::span::Span,
    },

    /// Invalid meta context usage error.
    ///
    /// Meta functions (`meta fn`) can only use compiler-provided meta contexts,
    /// not runtime contexts like Database, Logger, etc.
    ///
    /// Meta contexts: meta functions have restricted context access (only compile-time-safe contexts) — Meta contexts
    ///
    /// Example:
    /// ```verum
    /// // Invalid: Database is a runtime context
    /// meta fn bad() using Database {  // E502
    ///     ...
    /// }
    ///
    /// // Valid: TypeInfo is a meta context
    /// meta fn good<T>() using TypeInfo {
    ///     TypeInfo.fields_of::<T>()
    /// }
    /// ```
    #[error("E502: meta function `{func_name}` uses runtime context(s) {invalid_contexts} which is/are not available at compile-time")]
    InvalidMetaContext {
        /// Name of the meta function
        func_name: Text,
        /// Invalid context names (comma-separated)
        invalid_contexts: Text,
        /// Span of the using clause
        span: verum_ast::span::Span,
    },

    /// Pure function purity violation (E503).
    ///
    /// A function declared with the `pure` modifier has side effects
    /// in its body that violate purity guarantees.
    ///
    /// Pure functions must not:
    /// - Perform IO operations
    /// - Mutate external state
    /// - Use async operations
    /// - Access external state (reads/writes)
    /// - Call FFI functions
    /// - Spawn concurrent tasks
    ///
    /// Example:
    /// ```verum
    /// pure fn bad(x: Int) -> Int {
    ///     print(f"Value: {x}");  // E503: IO side effect in pure function
    ///     x
    /// }
    /// ```
    #[error("E503: pure function `{func_name}` has side effects: {properties}")]
    ImpurePureFunction {
        /// Name of the pure function
        func_name: Text,
        /// The impure properties detected (e.g., "IO, Mutates")
        properties: Text,
        /// Span of the function declaration
        span: verum_ast::span::Span,
    },

    /// Async property enforcement (E504).
    ///
    /// An async function's body uses `.await` or other async constructs
    /// but the function is not declared `async`, or vice versa:
    /// a function declared `async` has no async operations in its body (warning).
    ///
    /// Example:
    /// ```verum
    /// fn bad() -> Int {
    ///     some_async_call().await  // E504: await in non-async function
    /// }
    /// ```
    #[error("E504: {message}")]
    AsyncPropertyViolation {
        /// Description of the violation
        message: Text,
        /// Span of the function or expression
        span: verum_ast::span::Span,
    },

    // =========================================================================
    // Quote Hygiene Errors (M400-M409)
    // Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — Quote Hygiene
    // =========================================================================

    /// Unbound splice variable (M400)
    ///
    /// A variable used in a splice (`$var` or `${expr}`) is not in scope
    /// at the point of quote evaluation.
    ///
    /// Example:
    /// ```verum
    /// meta fn bad() -> TokenStream {
    ///     quote {
    ///         let x = ${undefined_var};  // M400: 'undefined_var' not in scope
    ///     }
    /// }
    /// ```
    #[error("M400: unbound splice variable `{var_name}` - not in scope at quote evaluation")]
    UnboundSpliceVariable {
        /// Name of the unbound variable
        var_name: Text,
        /// Span of the splice expression
        span: verum_ast::span::Span,
    },

    /// Unquote outside quote (M401)
    ///
    /// A splice/unquote expression (`$` or `${...}`) appears outside of a quote block.
    ///
    /// Example:
    /// ```verum
    /// fn bad() {
    ///     let x = ${some_expr};  // M401: splice outside quote
    /// }
    /// ```
    #[error("M401: splice/unquote `${{{expr}}}` used outside of quote block")]
    UnquoteOutsideQuote {
        /// The expression that was spliced
        expr: Text,
        /// Span of the splice
        span: verum_ast::span::Span,
    },

    /// Accidental variable capture (M402)
    ///
    /// A quote block introduces a binding that shadows a variable from the
    /// surrounding scope, which could lead to unexpected behavior.
    ///
    /// Example:
    /// ```verum
    /// meta fn bad(x: Int) -> TokenStream {
    ///     quote {
    ///         let x = 10;  // M402: shadows outer 'x'
    ///         $x           // Which 'x' is used?
    ///     }
    /// }
    /// ```
    #[error("M402: accidental variable capture - `{var_name}` in quote shadows outer binding")]
    AccidentalCapture {
        /// Name of the captured variable
        var_name: Text,
        /// Span of the inner binding
        inner_span: verum_ast::span::Span,
        /// Span of the outer binding being shadowed
        outer_span: verum_ast::span::Span,
    },

    /// Gensym collision (M403)
    ///
    /// A generated symbol collides with a user-defined name.
    ///
    /// Example:
    /// ```verum
    /// meta fn bad() -> TokenStream {
    ///     let fresh = gensym("temp");  // generates __temp_42
    ///     quote {
    ///         let __temp_42 = 1;       // M403: collides with gensym
    ///     }
    /// }
    /// ```
    #[error("M403: gensym collision - generated symbol `{symbol}` collides with user-defined name")]
    GensymCollision {
        /// The colliding symbol
        symbol: Text,
        /// Span of the collision
        span: verum_ast::span::Span,
    },

    /// Scope resolution failure (M404)
    ///
    /// A name in a quote block cannot be resolved to a unique binding.
    ///
    /// Example:
    /// ```verum
    /// meta fn bad() -> TokenStream {
    ///     quote {
    ///         let x = ambiguous_name;  // M404: cannot resolve
    ///     }
    /// }
    /// ```
    #[error("M404: scope resolution failure - cannot resolve `{name}` in quote")]
    ScopeResolutionFailure {
        /// The unresolved name
        name: Text,
        /// Span of the reference
        span: verum_ast::span::Span,
    },

    /// Stage mismatch (M405)
    ///
    /// An expression is evaluated at the wrong compilation stage.
    ///
    /// Example:
    /// ```verum
    /// meta fn bad() -> TokenStream {
    ///     quote(1) {
    ///         $(stage 2) { runtime_value }  // M405: stage 2 in stage 1 quote
    ///     }
    /// }
    /// ```
    #[error("M405: stage mismatch - expected stage {expected}, found stage {actual}")]
    StageMismatch {
        /// Expected stage level
        expected: u32,
        /// Actual stage level
        actual: u32,
        /// Span of the mismatched expression
        span: verum_ast::span::Span,
    },

    /// Lift type mismatch (M406)
    ///
    /// A value cannot be lifted because its type is not representable as code.
    /// Closures, mutable references, and opaque types cannot be lifted.
    ///
    /// Example:
    /// ```verum
    /// meta fn bad() -> TokenStream {
    ///     let f = |x: Int| x * 2;  // Closure type
    ///     quote {
    ///         let func = lift(f);  // M406: cannot lift closure
    ///     }
    /// }
    /// ```
    #[error("M406: cannot lift type `{ty}` - {reason}")]
    LiftTypeMismatch {
        /// The type that cannot be lifted
        ty: Text,
        /// Reason why it cannot be lifted
        reason: Text,
        /// Span of the lift expression
        span: verum_ast::span::Span,
    },

    /// Invalid stage escape (M407)
    ///
    /// An invalid stage escape attempt was detected, such as escaping
    /// to a stage that doesn't exist or escaping in an invalid context.
    ///
    /// # Example
    /// ```verum
    /// meta fn bad_escape() -> TokenStream {
    ///     quote {
    ///         $(stage 99) { x }  // ERROR: Stage 99 doesn't exist
    ///     }
    /// }
    /// ```
    #[error("M407: invalid stage escape - {reason}")]
    InvalidStageEscape {
        /// Reason for the invalid escape
        reason: Text,
        /// Span of the stage escape
        span: verum_ast::span::Span,
    },

    /// Undeclared capture (M408)
    ///
    /// A bare identifier in a quote refers to a meta-level binding without
    /// using proper capture syntax (`$var` or `lift(var)`).
    ///
    /// # Example
    /// ```verum
    /// meta fn bad_capture() -> TokenStream {
    ///     let x = 42;  // Meta-level binding
    ///     quote {
    ///         let y = x;  // ERROR: Should use $x or lift(x)
    ///     }
    /// }
    /// ```
    #[error("M408: undeclared capture of meta-level binding `{var_name}` - use $var or lift(var)")]
    UndeclaredCapture {
        /// Name of the variable being captured
        var_name: Text,
        /// Span of the reference
        span: verum_ast::span::Span,
    },

    /// Repetition mismatch (M409)
    ///
    /// Repetition variables in a quote have mismatched lengths.
    ///
    /// # Example
    /// ```verum
    /// meta fn bad_repeat(xs: List<Int>, ys: List<Int>) -> TokenStream {
    ///     // If xs has 3 elements and ys has 5, this is a mismatch
    ///     quote {
    ///         $[for x in xs, y in ys { f($x, $y) }]  // ERROR: Length mismatch
    ///     }
    /// }
    /// ```
    #[error("M409: repetition mismatch - {reason}")]
    RepetitionMismatch {
        /// Description of the mismatch
        reason: Text,
        /// Span of the repetition
        span: verum_ast::span::Span,
    },

    // Inline assembly errors
    // Low-level type operations: raw pointer casting, transmute, memory layout control

    /// Invalid type for const asm operand
    #[error("invalid type for inline assembly const operand: {found}")]
    InvalidAsmConstType {
        found: Type,
        span: verum_ast::span::Span,
    },

    /// Asm output is not an lvalue
    #[error("inline assembly output operand must be an lvalue (assignable location)")]
    AsmOutputNotLvalue { span: verum_ast::span::Span },

    #[error("recursion limit exceeded: {0}")]
    RecursionLimit(Text),

    #[error("{0}")]
    Other(Text),

    #[error("{msg}")]
    OtherWithCode { code: Text, msg: Text },
}

impl TypeError {
    /// Get the span for this error
    pub fn span(&self) -> verum_ast::span::Span {
        use TypeError::*;
        match self {
            Mismatch { span, .. } => *span,
            CannotInferLambda { span } => *span,
            InfiniteType { span, .. } => *span,
            UnboundVariable { span, .. } => *span,
            NotAFunction { span, .. } => *span,
            BranchMismatch { span, .. } => *span,
            InvalidIndex { span, .. } => *span,
            ProtocolNotSatisfied { span, .. } => *span,
            RefinementFailed { span, .. } => *span,
            RefinementPredicateInvalid { span, .. } => *span,
            ContextNotAllowed { span, .. } => *span,
            ConstMismatch { span, .. } => *span,
            AmbiguousType { span } => *span,
            AffineViolation { first_use, .. } => *first_use,
            LinearViolation { span, .. } => *span,
            MovedValueUsed { moved_at, .. } => *moved_at,
            AffineValueInLoop { use_span, .. } => *use_span,
            PartiallyMovedValue { used_at, .. } => *used_at,
            LinearNotConsumed { binding_span, .. } => *binding_span,
            MissingContext { span, .. } => *span,
            UndefinedContext { span, .. } => *span,
            DuplicateProvide { span, .. } => *span,
            UndefinedContextMethod { span, .. } => *span,
            InvalidSubContext { span, .. } => *span,
            ContextMismatch { span, .. } => *span,
            ContextPropagationError { span, .. } => *span,
            ExcludedContextViolation { span, .. } => *span,
            TransitiveNegativeContextViolation { span, .. } => *span,
            DirectNegativeContextViolation { usage_span, .. } => *usage_span,
            ContextAliasConflict { second_span, .. } => *second_span,
            InvalidCast { span, .. } => *span,
            MethodNotFound { span, .. } => *span,
            CapabilityViolation { span, .. } => *span,
            ProtocolNotImplemented { span, .. } => *span,
            WrongArgCount { span, .. } => *span,
            AmbiguousMethod { span, .. } => *span,
            TypeNotFound { span, .. } => *span,
            NotAType { span, .. } => *span,
            TryOperatorOutsideFunction { span } => *span,
            TryOnNonResult { span, .. } => *span,
            TryOperatorMismatch { span, .. } => *span,
            ResultTypeMismatch { span, .. } => *span,
            MultipleConversionPaths { span, .. } => *span,
            TryInNonResultContext { span, .. } => *span,
            NotResultOrMaybe { span, .. } => *span,
            TypeCycle { span, .. } => *span,
            UnresolvedPlaceholder { span, .. } => *span,
            IncompleteTypeReference { span, .. } => *span,
            NonContextProtocolInUsing { span, .. } => *span,
            // Advanced type system error spans (HKTs, associated types, existential types)
            ExistentialEscape { escape_span, .. } => *escape_span,
            ExistentialBoundNotSatisfied { span, .. } => *span,
            KindMismatch { span, .. } => *span,
            TypeConstructorArityMismatch { span, .. } => *span,
            CannotResolveAssociatedType { span, .. } => *span,
            AmbiguousAssociatedType { span, .. } => *span,
            NegativeBoundViolated { span, .. } => *span,
            SpecializationOverlap { span, .. } => *span,
            HKTBoundNotSatisfied { span, .. } => *span,
            // Definite assignment analysis error spans
            UseOfUninitializedVariable { span, .. } => *span,
            PartiallyInitializedVariable { span, .. } => *span,
            UninitializedField { span, .. } => *span,
            UninitializedArrayElement { span, .. } => *span,
            UninitializedTupleElement { span, .. } => *span,
            IterationOverPartialArray { span, .. } => *span,
            VisibilityError { span, .. } => *span,
            AmbiguousName { span, .. } => *span,
            CircularConstantDependency { span, .. } => *span,
            // Reference aliasing error spans
            BorrowConflict { new_borrow_span, .. } => *new_borrow_span,
            FieldBorrowConflict { new_span, .. } => *new_span,
            DanglingReference { ref_span, .. } => *ref_span,
            CheckedRefEscape { span, .. } => *span,
            MoveWhileBorrowed { move_span, .. } => *move_span,
            AssignWhileBorrowed { assign_span, .. } => *assign_span,
            StackAllocationExceedsLimit { span, .. } => *span,
            UnboundedRecursionDetected { span, .. } => *span,
            // Import resolution error spans
            ImportItemNotFound { span, .. } => *span,
            ImportModuleNotFound { span, .. } => *span,
            UndefinedFunction { span, .. } => *span,
            // Meta function purity error
            ImpureMetaFunction { span, .. } => *span,
            // Meta function context error
            InvalidMetaContext { span, .. } => *span,
            // Pure function purity error
            ImpurePureFunction { span, .. } => *span,
            // Async property violation
            AsyncPropertyViolation { span, .. } => *span,
            // Quote hygiene errors (M400-M409)
            UnboundSpliceVariable { span, .. } => *span,
            UnquoteOutsideQuote { span, .. } => *span,
            AccidentalCapture { inner_span, .. } => *inner_span,
            GensymCollision { span, .. } => *span,
            ScopeResolutionFailure { span, .. } => *span,
            StageMismatch { span, .. } => *span,
            LiftTypeMismatch { span, .. } => *span,
            InvalidStageEscape { span, .. } => *span,
            UndeclaredCapture { span, .. } => *span,
            RepetitionMismatch { span, .. } => *span,
            // Inline assembly errors
            InvalidAsmConstType { span, .. } => *span,
            AsmOutputNotLvalue { span } => *span,
            RecursionLimit(_) | Other(_) | OtherWithCode { .. } => verum_ast::span::Span::dummy(),
        }
    }

    /// Convert to diagnostic for error reporting with span information
    ///
    /// Note: This method creates diagnostics without file path/line information.
    /// For proper diagnostics with source locations, use the compiler's helper
    /// that converts AST spans to diagnostic spans via the session.
    pub fn to_diagnostic(&self) -> Diagnostic {
        self.to_diagnostic_with_span::<fn(verum_ast::span::Span) -> verum_diagnostics::Span>(None)
    }

    /// Convert to diagnostic with optional span converter
    ///
    /// When span_converter is provided, it will be used to add proper source
    /// location information to the diagnostic.
    pub fn to_diagnostic_with_span<F>(&self, span_converter: Option<F>) -> Diagnostic
    where
        F: Fn(verum_ast::span::Span) -> verum_diagnostics::Span,
    {
        use TypeError::*;

        // Helper to optionally convert span
        let convert_span = |ast_span: verum_ast::span::Span| -> Option<verum_diagnostics::Span> {
            span_converter.as_ref().map(|f| f(ast_span))
        };

        match self {
            Mismatch {
                expected,
                actual,
                span,
            } => {
                // E400: Type mismatch - type system incompatibility error
                // Format matches spec expectation: "Type mismatch: expected 'X', found 'Y'"
                // Note: E310 is for borrow/aliasing errors, E400 is for type mismatches
                let mut builder = DiagnosticBuilder::error()
                    .code("E400")
                    .message(format!(
                        "Type mismatch: expected '{}', found '{}'",
                        expected, actual
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            CannotInferLambda { span } => {
                let mut builder = DiagnosticBuilder::error().message("cannot infer lambda type");
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            InfiniteType { var, ty, span } => {
                let mut builder =
                    DiagnosticBuilder::error().code("E403").message(format!("Infinite type: {} = {}", var, ty));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            UnboundVariable { name, span } => {
                let mut builder =
                    DiagnosticBuilder::error().code("E100").message(format!("unbound variable: {}", name));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            NotAFunction { ty, span } => {
                let mut builder =
                    DiagnosticBuilder::error().message(format!("not a function: {}", ty));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            BranchMismatch {
                then_ty,
                else_ty,
                span,
            } => {
                // E400: Type mismatch in branch expressions
                let mut builder = DiagnosticBuilder::error()
                    .code("E400")
                    .message(format!(
                        "Type mismatch: if-expression branches have incompatible types: then has '{}', else has '{}'",
                        then_ty, else_ty
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            InvalidIndex { message, span } => {
                // E310: Array index / bounds error (memory safety)
                let mut builder = DiagnosticBuilder::error()
                    .code("E310")
                    .message(message.to_string());
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ProtocolNotSatisfied { ty, protocol, span } => {
                // Determine error code based on protocol type
                // E402: Type does not implement Send
                // E403: Type does not implement Sync
                // E312: Generic protocol constraint not satisfied
                let (code, msg) = match protocol.as_str() {
                    "Send" => ("E402", format!("Type does not implement Send: '{}' cannot be sent between threads", ty)),
                    "Sync" => ("E403", format!("Type does not implement Sync: '{}' cannot be shared between threads", ty)),
                    _ => ("E312", format!("Protocol constraint not satisfied: '{}' does not implement '{}'", ty, protocol)),
                };
                let mut builder = DiagnosticBuilder::error()
                    .code(code)
                    .message(msg);
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            RefinementFailed { predicate, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E500")
                    .message(format!("refinement constraint failed: {}", predicate));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            RefinementPredicateInvalid { message, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E501")
                    .message(format!("invalid refinement predicate: {}", message));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ContextNotAllowed { context, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("context {} not allowed in this context", context));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ConstMismatch {
                expected,
                actual,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "const generic parameter mismatch: expected {}, found {}",
                    expected, actual
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            AmbiguousType { span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message("ambiguous type: cannot infer type without more context");
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            AffineViolation {
                ty,
                first_use,
                second_use,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "affine type `{}` used more than once\n  \
                         first use at: {}\n  \
                         second use at: {}\n  \
                         help: affine types can only be used once to prevent resource leaks",
                    ty, first_use.start, second_use.start
                ));
                if let Some(diag_span) = convert_span(*first_use) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            LinearViolation {
                ty,
                usage_count,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "linear type `{}` not used exactly once (used {} times)\n  \
                         help: linear types must be consumed exactly once",
                    ty, usage_count
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            MovedValueUsed {
                name,
                moved_at,
                used_at,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "value `{}` used after move\n  \
                         moved at: {}\n  \
                         used at: {}\n  \
                         help: affine values cannot be used after being moved",
                    name, moved_at.start, used_at.start
                ));
                if let Some(diag_span) = convert_span(*moved_at) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            AffineValueInLoop {
                name,
                binding_span,
                use_span,
            } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E302")
                    .message(format!(
                        "affine value `{}` cannot be used in loop\n  \
                         defined at: {}\n  \
                         used in loop at: {}\n  \
                         help: affine values from outer scope cannot be moved in a loop \
                         because the loop may execute multiple times\n  \
                         help: consider cloning the value or restructuring the code",
                        name, binding_span.start, use_span.start
                    ));
                if let Some(diag_span) = convert_span(*use_span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            PartiallyMovedValue {
                name,
                moved_field,
                moved_at,
                used_at,
            } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E302")
                    .message(format!(
                        "value `{}` used after partial move\n  \
                         field `{}` was moved at: {}\n  \
                         whole struct used at: {}\n  \
                         help: after moving a field out of a struct, the whole struct \
                         cannot be used as a value\n  \
                         help: individual non-moved fields can still be accessed",
                        name, moved_field, moved_at.start, used_at.start
                    ));
                if let Some(diag_span) = convert_span(*used_at) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            LinearNotConsumed {
                name,
                binding_span,
                scope_end,
            } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E303")
                    .message(format!(
                        "linear value `{}` must be consumed exactly once\n  \
                         defined at: {}\n  \
                         scope ends at: {}\n  \
                         help: linear values must be used before going out of scope\n  \
                         help: call a function that consumes this value or pass it to another function",
                        name, binding_span.start, scope_end.start
                    ));
                if let Some(diag_span) = convert_span(*binding_span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            MissingContext { context, span } => {
                // E801: Context used but not declared in function signature
                let mut builder = DiagnosticBuilder::error()
                    .code("E801")
                    .message(format!(
                        "context `{}` used but not declared in function signature\n  \
                         help: add `using [{}]` to function signature\n  \
                         help: or provide it with `provide {} = ...`",
                        context, context, context
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            UndefinedContext { name, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!(
                        "undefined context: {}\n  \
                         help: check if context or protocol is imported\n  \
                         help: context declarations use `context Name {{ ... }}`\n  \
                         help: protocols defined with `type Name is protocol {{ ... }}` can also serve as contexts",
                        name
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            DuplicateProvide { context, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E808")
                    .message(format!(
                        "Duplicate provide for same context\n  \
                         context `{}` already provided in this scope\n  \
                         help: use different scopes with `provide {} = ... in {{ ... }}`",
                        context, context
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            UndefinedContextMethod {
                context,
                method,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "context `{}` has no method `{}`\n  \
                         help: check the context declaration for available methods",
                    context, method
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            InvalidSubContext {
                context,
                sub_context,
                available,
                span,
            } => {
                let mut msg = format!(
                    "sub-context `{}` does not exist in context `{}`",
                    sub_context, context
                );

                if !available.is_empty() {
                    msg.push_str(&format!(
                        "\n  note: context `{}` defines these sub-contexts:\n",
                        context
                    ));
                    for sub in available {
                        msg.push_str(&format!("           - {}\n", sub));
                    }
                    msg.push_str("  help: check the context definition or use a valid sub-context");
                } else {
                    msg.push_str(&format!(
                        "\n  note: context `{}` has no sub-contexts defined\n  \
                         help: sub-contexts are defined with nested `context` blocks inside the parent context",
                        context
                    ));
                }

                let mut builder = DiagnosticBuilder::error().message(msg);
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ContextMismatch {
                expected,
                actual,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "context mismatch: expected `{}`, found `{}`",
                    expected, actual
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ContextPropagationError {
                context,
                callee,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "function `{}` requires context `{}` but caller doesn't provide it\n  \
                         help: add `using [{}]` to caller's signature\n  \
                         help: or provide it locally with `provide {} = ...`",
                    callee, context, context, context
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ExcludedContextViolation { context, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "cannot use context `{}` which is explicitly excluded (`!{}`)\n  \
                         help: remove the context usage\n  \
                         help: or remove `!{}` from the function's `using` clause if context is needed",
                    context, context, context
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            TransitiveNegativeContextViolation {
                excluded_context,
                callee,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "calling `{}` violates negative context constraint\n  \
                         note: context `{}` is excluded (`!{}`), but `{}` requires it\n  \
                         help: remove the call to `{}`\n  \
                         help: or remove `!{}` from the function's `using` clause",
                    callee, excluded_context, excluded_context, callee, callee, excluded_context
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            DirectNegativeContextViolation {
                context,
                function_name,
                usage_span,
                declaration_span,
            } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E3050")
                    .message(format!(
                        "negative context violation in function `{}`\n   |\n{} | Cannot use '{}' context - explicitly excluded via `using [!{}]`\n   |\n   = help: Remove the !{} from the using clause if you need {} access\n   = note: Function '{}' excludes {} for testing purity",
                        function_name,
                        usage_span.start,
                        context, context,
                        context, context,
                        function_name, context
                    ));
                if let Some(diag_span) = convert_span(*usage_span) {
                    builder = builder.span(diag_span);
                }
                // Add secondary label for declaration location
                let _ = declaration_span; // Used for secondary location in full diagnostic
                builder.build()
            }

            ContextAliasConflict {
                alias,
                first_context,
                first_function,
                first_span,
                second_context,
                second_function,
                second_span,
            } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E3060")
                    .message(format!(
                        "context alias conflict in module\n   |\n{} |     fn {}() using [{} as {}] {{ ... }}\n   |                         ^^^^^^^^^^^^^^^^^\n   = note: Alias '{}' conflicts with usage in same module\n   |\n{} |     fn {}() using [{} as {}] {{ ... }}\n   |                        ^^^^^^^^^^^^^^^\n   |                        Also uses alias '{}' for different context\n   |\n   = help: Use distinct aliases for different contexts",
                        first_span.start,
                        first_function, first_context, alias,
                        alias,
                        second_span.start,
                        second_function, second_context, alias,
                        alias
                    ));
                if let Some(diag_span) = convert_span(*second_span) {
                    builder = builder.span(diag_span);
                }
                // Add secondary label for first usage
                let _ = first_span; // Used for secondary location in full diagnostic
                builder.build()
            }

            InvalidCast {
                from,
                to,
                reason,
                span,
            } => {
                // E401: Invalid assignment/cast - type conversion error
                // E402 for specific protocol constraints (Send/Sync), E401 for general casts
                let mut builder = DiagnosticBuilder::error()
                    .code("E401")
                    .message(format!(
                        "Cannot assign value of type '{}' to variable of type '{}'\n  \
                             reason: {}\n  \
                             help: use explicit conversion method if available\n  \
                             help: or ensure types are compatible for casting",
                        from, to, reason
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            MethodNotFound { ty, method, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "no method named `{}` found for type `{}`\n  \
                         help: check method name spelling\n  \
                         help: ensure type implements a protocol with this method\n  \
                         help: check available methods in protocol documentation",
                    method, ty
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            CapabilityViolation {
                method,
                type_name,
                required_capability,
                available_capabilities,
                span,
            } => {
                let available_list: Vec<&str> = available_capabilities.iter().map(|c| c.as_str()).collect();
                let mut builder = DiagnosticBuilder::error()
                    .code("E306")
                    .message(format!(
                        "capability violation: method `{}` on `{}` requires `{}` capability\n  \
                         note: available capabilities: [{}]\n  \
                         help: to call this method, the value needs `{}` capability\n  \
                         help: consider using a value with more capabilities, or choose a different method",
                        method, type_name, required_capability,
                        available_list.join(", "),
                        required_capability
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ProtocolNotImplemented {
                ty,
                protocol,
                method,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "method `{}` requires protocol `{}` but type `{}` does not implement it\n  \
                         help: implement `{}` for type `{}`\n  \
                         help: or use a different method that doesn't require this protocol",
                    method, protocol, ty, protocol, ty
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            WrongArgCount {
                method,
                expected,
                actual,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "method `{}` expects {} argument(s), but {} were provided\n  \
                         help: check the method signature",
                    method, expected, actual
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            AmbiguousMethod {
                method,
                candidates,
                span,
            } => {
                let mut msg = format!(
                    "ambiguous method call: `{}` could refer to multiple protocols:\n",
                    method
                );
                for candidate in candidates {
                    msg.push_str(&format!("  - {}\n", candidate));
                }
                msg.push_str("  help: specify the protocol explicitly or add type annotations");

                let mut builder = DiagnosticBuilder::error().message(msg);
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            TypeNotFound { name, span } => {
                let mut builder =
                    DiagnosticBuilder::error().code("E101").message(format!("type not found: {}", name));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            NotAType {
                name,
                actual_kind,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "`{}` is not a type (it is a {})",
                    name, actual_kind
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            TryOperatorOutsideFunction { span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message("cannot use '?' operator outside of function context");
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            TryOnNonResult { ty, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E0203")
                    .message(format!(
                        "cannot use '?' on non-Result/Maybe type: {}",
                        ty
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            TryOperatorMismatch { expr_type, return_type, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E0203")
                    .message(format!(
                        "type mismatch in '?' operator: cannot convert {} to {}",
                        expr_type,
                        return_type
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ResultTypeMismatch {
                inner_error,
                outer_error,
                span: _,
                diagnostic,
            } => diagnostic.clone(),

            MultipleConversionPaths {
                from_type,
                to_type,
                paths: _,
                span: _,
                diagnostic,
            } => diagnostic.clone(),

            TryInNonResultContext {
                expr_type,
                function_return_type,
                span: _,
                diagnostic,
            } => diagnostic.clone(),

            NotResultOrMaybe { ty, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("type `{}` is not Result or Maybe", ty));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            TypeCycle {
                cycle_path,
                types_in_cycle,
                span,
            } => {
                let mut msg = format!(
                    "cyclic type definition detected: {}\n  \
                     The following types form a cycle:\n",
                    cycle_path
                );
                for ty in types_in_cycle {
                    msg.push_str(&format!("    - {}\n", ty));
                }
                msg.push_str(
                    "  help: break the cycle by using a reference type, Box<T>, or indirect reference\n  \
                     help: recursive types must use indirection (e.g., Box<Self>) to have finite size"
                );
                let mut builder = DiagnosticBuilder::error().message(msg);
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            UnresolvedPlaceholder { name, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "unresolved type `{}`\n  \
                         This type was referenced but never defined.\n  \
                         help: check the spelling of the type name\n  \
                         help: ensure the type is defined or imported",
                    name
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            IncompleteTypeReference { name, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!(
                        "type `{}` cannot reference itself in its own definition without indirection\n  \
                         help: use Box<{}> or a reference to break the direct self-reference\n  \
                         help: for recursive types, wrap self-references in Box, List, or similar container",
                        name, name
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            NonContextProtocolInUsing { name, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "protocol '{}' cannot be used as a context\n  \
                         note: '{}' is a constraint protocol, use `where T: {}` instead\n  \
                         help: to make it injectable, declare as `context protocol {} {{ ... }}`",
                    name, name, name, name
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            // ========== Advanced type syntax errors: malformed HKTs, invalid associated type projections, existential type misuse ==========

            ExistentialEscape { skolem_name, unpacking_span, escape_span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "existential type escapes its scope\n  \
                         the opaque type '{}' cannot be used outside its unpacking scope\n  \
                         note: existential types hide their implementation details",
                    skolem_name
                ));
                if let Some(diag_span) = convert_span(*escape_span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            ExistentialBoundNotSatisfied { witness_type, protocol, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "existential bound not satisfied\n  \
                         type '{}' does not implement '{}'\n  \
                         help: ensure the witness type implements all required protocols",
                    witness_type, protocol
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            KindMismatch { expected_kind, actual_kind, type_name, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "kind mismatch for '{}'\n  \
                         expected kind: {}\n  \
                         found kind: {}\n  \
                         note: higher-kinded types must have compatible kinds",
                    type_name, expected_kind, actual_kind
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            TypeConstructorArityMismatch { name, expected_arity, actual_arity, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "type constructor '{}' has wrong arity\n  \
                         expected {} type argument(s), found {}\n  \
                         note: F<_> has arity 1, F<_, _> has arity 2",
                    name, expected_arity, actual_arity
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            CannotResolveAssociatedType { base_type, assoc_name, reason, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "cannot resolve associated type '{}.{}'\n  \
                         {}\n  \
                         help: ensure '{}' implements a protocol that defines '{}'",
                    base_type, assoc_name, reason, base_type, assoc_name
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            AmbiguousAssociatedType { base_type, assoc_name, candidates, span } => {
                let candidates_str = candidates.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "ambiguous associated type '{}.{}'\n  \
                         multiple implementations define '{}': {}\n  \
                         help: qualify with the specific protocol: Protocol.{}",
                    base_type, assoc_name, assoc_name, candidates_str, assoc_name
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            NegativeBoundViolated { ty, protocol, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "negative bound violated\n  \
                         type '{}' implements '{}', but '!{}' was required\n  \
                         note: negative bounds exclude types that implement the protocol",
                    ty, protocol, protocol
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            SpecializationOverlap { ty, impl1, impl2, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "specialization overlap for type '{}'\n  \
                         conflicting implementations:\n  \
                         - {}\n  \
                         - {}\n  \
                         help: use negative bounds or @specialize to disambiguate",
                    ty, impl1, impl2
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            HKTBoundNotSatisfied { type_constructor, protocol, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "higher-kinded bound not satisfied\n  \
                         type constructor '{}' does not implement '{}'\n  \
                         help: implement '{}' for '{}' or use a different type constructor",
                    type_constructor, protocol, protocol, type_constructor
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            // Inline assembly errors
            InvalidAsmConstType { found, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "invalid type for inline assembly const operand: {}\n  \
                         help: const operands must be integer or pointer types",
                    found
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            AsmOutputNotLvalue { span } => {
                let mut builder = DiagnosticBuilder::error().message(
                    "inline assembly output operand must be an lvalue\n  \
                         help: use a variable or field access, not a literal or expression"
                );
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            RecursionLimit(msg) => DiagnosticBuilder::error().message(format!("recursion limit exceeded: {}", msg)).build(),

            Other(msg) => DiagnosticBuilder::error().message(msg.as_str()).build(),

            OtherWithCode { code, msg } => DiagnosticBuilder::error().code(code.as_str()).message(msg.as_str()).build(),

            // Definite Assignment Analysis Errors (E201)
            UseOfUninitializedVariable { name, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("E201: use of uninitialized variable '{}'", name));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            PartiallyInitializedVariable { name, missing, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "E201: use of partially initialized variable '{}' (missing: {})",
                    name, missing
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            UninitializedField { var, field, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("E201: field '{}' of variable '{}' is not initialized", field, var));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            UninitializedArrayElement { var, index, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "E201: array element at index {} of '{}' is not initialized",
                    index, var
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            UninitializedTupleElement { var, index, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "E201: tuple element {} of '{}' is not initialized",
                    index, var
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            IterationOverPartialArray { var, span, .. } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("E201: cannot iterate over partially initialized array '{}'", var));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            VisibilityError {
                name,
                visibility,
                module_path,
                span,
            } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "E601: visibility error: '{}' is {} in module '{}'",
                    name, visibility, module_path
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            AmbiguousName { name, sources, span } => {
                let mut builder = DiagnosticBuilder::error().message(format!(
                    "E602: ambiguous name: '{}' is imported from multiple modules: {}",
                    name, sources
                ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            CircularConstantDependency {
                cycle_path,
                constants_in_cycle,
                span,
            } => {
                let mut msg = format!(
                    "E600: circular constant dependency detected: {}\n  \
                     The following constants form a cycle:\n",
                    cycle_path
                );
                for constant in constants_in_cycle {
                    msg.push_str(&format!("    - {}\n", constant));
                }
                msg.push_str(
                    "  help: break the cycle by removing one of the dependencies\n  \
                     help: constants must have a well-defined evaluation order"
                );
                let mut builder = DiagnosticBuilder::error().message(msg);
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder.build()
            }

            // Reference Aliasing Errors
            BorrowConflict { var, existing_borrow_span, existing_is_mut, new_borrow_span, new_is_mut } => {
                let new_kind = if *new_is_mut { "mutable" } else { "immutable" };
                let existing_kind = if *existing_is_mut { "mutable" } else { "immutable" };
                let mut builder = DiagnosticBuilder::error()
                    .message(format!(
                        "E310: cannot borrow `{}` as {} because it is already borrowed as {}",
                        var, new_kind, existing_kind
                    ));
                if let Some(diag_span) = convert_span(*new_borrow_span) {
                    builder = builder.span(diag_span);
                }
                if let Some(_existing_span) = convert_span(*existing_borrow_span) {
                    builder = builder.add_note(format!("previous {} borrow occurs here", existing_kind));
                }
                builder.build()
            }

            FieldBorrowConflict { var, field, existing_span, new_span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!(
                        "E311: cannot borrow `{}` because field `{}` is already borrowed",
                        var, field
                    ));
                if let Some(diag_span) = convert_span(*new_span) {
                    builder = builder.span(diag_span);
                }
                if let Some(_prev_span) = convert_span(*existing_span) {
                    builder = builder.add_note("previous borrow of field occurs here");
                }
                builder.build()
            }

            DanglingReference { var, ref_span, drop_span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!(
                        "E312: `{}` does not live long enough - reference outlives referent",
                        var
                    ));
                if let Some(diag_span) = convert_span(*ref_span) {
                    builder = builder.span(diag_span);
                }
                if let Some(_d_span) = convert_span(*drop_span) {
                    builder = builder.add_note("value goes out of scope here");
                }
                builder.build()
            }

            CheckedRefEscape { var, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!(
                        "E310: `&checked` reference to `{}` may escape through function call",
                        var
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note(
                    "&checked references have zero runtime overhead and cannot escape their scope"
                );
                builder = builder.add_note(
                    "help: use a managed reference (&T) instead, which has CBGR runtime checks"
                );
                builder.build()
            }

            MoveWhileBorrowed { var, move_span, borrow_span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("E313: cannot move `{}` while it is borrowed", var));
                if let Some(diag_span) = convert_span(*move_span) {
                    builder = builder.span(diag_span);
                }
                if let Some(_b_span) = convert_span(*borrow_span) {
                    builder = builder.add_note("borrow occurs here");
                }
                builder.build()
            }

            AssignWhileBorrowed { var, assign_span, borrow_span } => {
                let mut builder = DiagnosticBuilder::error()
                    .message(format!("E314: cannot assign to `{}` while it is borrowed", var));
                if let Some(diag_span) = convert_span(*assign_span) {
                    builder = builder.span(diag_span);
                }
                if let Some(_b_span) = convert_span(*borrow_span) {
                    builder = builder.add_note("borrow occurs here");
                }
                builder.build()
            }

            StackAllocationExceedsLimit { size, limit, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E320")
                    .message(format!(
                        "stack allocation exceeds safe limit ({} bytes exceeds {} byte limit)",
                        size, limit
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("use Heap<T> or List<T> for large allocations");
                builder.build()
            }

            UnboundedRecursionDetected { func_name, span, cycle } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E321")
                    .message(format!(
                        "potential stack overflow: unbounded recursion detected in function `{}`",
                        func_name
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                if !cycle.is_empty() {
                    builder = builder.add_note(format!(
                        "recursion cycle: {}",
                        cycle.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" -> ")
                    ));
                }
                builder = builder.add_note("add @allow(unbounded_recursion) to suppress this warning if intentional");
                builder.build()
            }

            // ========== Import Resolution Errors ==========

            ImportItemNotFound { item_name, module_path, available_items, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E401")
                    .message(format!(
                        "cannot find `{}` in module `{}`",
                        item_name, module_path
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                // Add suggestions for similar items
                if !available_items.is_empty() {
                    let similar = find_similar_names(item_name.as_str(), available_items);
                    if !similar.is_empty() {
                        builder = builder.help(format!(
                            "did you mean one of these? {}",
                            similar.iter().map(|s| format!("`{}`", s)).collect::<Vec<_>>().join(", ")
                        ));
                    }
                    // Show available exports (limited to first 10)
                    let items_preview: Vec<&str> = available_items.iter().take(10).map(|s| s.as_str()).collect();
                    let suffix = if available_items.len() > 10 { format!(" and {} more", available_items.len() - 10) } else { String::new() };
                    builder = builder.add_note(format!(
                        "module `{}` exports: {}{}",
                        module_path,
                        items_preview.join(", "),
                        suffix
                    ));
                }
                builder.build()
            }

            ImportModuleNotFound { module_path, similar_modules, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E402")
                    .message(format!(
                        "module `{}` not found",
                        module_path
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                // Add suggestions for similar modules
                if !similar_modules.is_empty() {
                    builder = builder.help(format!(
                        "did you mean one of these modules? {}",
                        similar_modules.iter().map(|s| format!("`{}`", s)).collect::<Vec<_>>().join(", ")
                    ));
                }
                builder = builder.add_note("check that the module path is correct and the module is available");
                builder.build()
            }

            UndefinedFunction { func_name, similar_functions, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("E403")
                    .message(format!(
                        "undefined function `{}`",
                        func_name
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                // Add suggestions for similar functions
                if !similar_functions.is_empty() {
                    builder = builder.help(format!(
                        "did you mean one of these? {}",
                        similar_functions.iter().map(|s| format!("`{}`", s)).collect::<Vec<_>>().join(", ")
                    ));
                }
                builder = builder.add_note("ensure the function is defined or imported");
                builder.build()
            }

            ImpureMetaFunction { func_name, properties, span } => {
                // E501: Meta function purity violation
                // Meta function purity: meta functions are implicitly pure (no IO, no mutation of non-meta state) — Meta functions are implicitly pure
                let mut builder = DiagnosticBuilder::error()
                    .code("E501")
                    .message(format!(
                        "meta function `{}` must be pure but has side effects: {}",
                        func_name, properties
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("meta functions run at compile-time and cannot have side effects");
                builder = builder.help("remove IO, external state access, or mutation from the function body");
                builder.build()
            }

            InvalidMetaContext { func_name, invalid_contexts, span } => {
                // E502: Invalid meta context usage
                // Meta contexts: meta functions have restricted context access (only compile-time-safe contexts) — Meta contexts
                let mut builder = DiagnosticBuilder::error()
                    .code("E502")
                    .message(format!(
                        "meta function `{}` uses runtime context(s) `{}` which are not available at compile-time",
                        func_name, invalid_contexts
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("meta functions can only use compiler-provided contexts");
                builder = builder.help("valid meta contexts: BuildAssets, TypeInfo, AstAccess, CompileDiag, MetaRuntime, MacroState");
                builder.build()
            }

            ImpurePureFunction { func_name, properties, span } => {
                // E503: Pure function purity violation
                let mut builder = DiagnosticBuilder::error()
                    .code("E503")
                    .message(format!(
                        "pure function `{}` has side effects: {}",
                        func_name, properties
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("pure functions must not have IO, mutation, async, or external state access");
                builder = builder.help("remove the `pure` modifier or eliminate side effects from the function body");
                builder.build()
            }

            AsyncPropertyViolation { message, span } => {
                // E504: Async property enforcement
                let mut builder = DiagnosticBuilder::error()
                    .code("E504")
                    .message(message.to_string());
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("async operations require async function context");
                builder.build()
            }

            // Quote hygiene errors (M400-M409)
            // Quote hygiene: macro-generated code uses hygienic naming to prevent variable capture and scope pollution — Quote Hygiene

            UnboundSpliceVariable { var_name, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M400")
                    .message(format!(
                        "unbound splice variable `{}` - not in scope at quote evaluation",
                        var_name
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("splice variables must be bound in the enclosing scope");
                builder = builder.help("ensure the variable is defined before the quote block");
                builder.build()
            }

            UnquoteOutsideQuote { expr, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M401")
                    .message(format!(
                        "splice/unquote `${{{}}}` used outside of quote block",
                        expr
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("splice expressions are only valid inside quote blocks");
                builder = builder.help("wrap the code in a quote block or use a regular expression");
                builder.build()
            }

            AccidentalCapture { var_name, inner_span, outer_span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M402")
                    .message(format!(
                        "accidental variable capture - `{}` in quote shadows outer binding",
                        var_name
                    ));
                if let Some(diag_span) = convert_span(*inner_span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note(format!(
                    "outer binding at position {}",
                    outer_span.start
                ));
                builder = builder.help("use a different name or explicitly capture with gensym");
                builder.build()
            }

            GensymCollision { symbol, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M403")
                    .message(format!(
                        "gensym collision - generated symbol `{}` collides with user-defined name",
                        symbol
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("generated symbols should be unique");
                builder = builder.help("avoid using names that look like gensyms (e.g., __name_N)");
                builder.build()
            }

            ScopeResolutionFailure { name, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M404")
                    .message(format!(
                        "scope resolution failure - cannot resolve `{}` in quote",
                        name
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("name resolution in quotes follows hygiene rules");
                builder = builder.help("check that the name is properly bound and visible");
                builder.build()
            }

            StageMismatch { expected, actual, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M405")
                    .message(format!(
                        "stage mismatch - expected stage {}, found stage {}",
                        expected, actual
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("stage escapes must match the quote's target stage");
                builder = builder.help(format!(
                    "use $(stage {}){{...}} to escape to stage {}",
                    expected, expected
                ));
                builder.build()
            }

            LiftTypeMismatch { ty, reason, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M406")
                    .message(format!(
                        "cannot lift type `{}` - {}",
                        ty, reason
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("lift() can only convert values whose types can be represented as code");
                builder = builder.help("use splice ($) for values that are already code, or restructure to avoid lifting unliftable types");
                builder.build()
            }

            InvalidStageEscape { reason, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M407")
                    .message(format!(
                        "invalid stage escape - {}",
                        reason
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("stage escapes must reference valid compilation stages");
                builder = builder.help("check that the stage number is valid and the escape is in a valid context");
                builder.build()
            }

            UndeclaredCapture { var_name, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M408")
                    .message(format!(
                        "undeclared capture of meta-level binding `{}` - use $var or lift(var)",
                        var_name
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("bare identifiers in quotes cannot directly reference meta-level bindings");
                builder = builder.help(format!(
                    "use ${} to splice the value, or lift({}) to convert it to syntax",
                    var_name, var_name
                ));
                builder.build()
            }

            RepetitionMismatch { reason, span } => {
                let mut builder = DiagnosticBuilder::error()
                    .code("M409")
                    .message(format!(
                        "repetition mismatch - {}",
                        reason
                    ));
                if let Some(diag_span) = convert_span(*span) {
                    builder = builder.span(diag_span);
                }
                builder = builder.add_note("all repetition variables must have the same length");
                builder = builder.help("ensure all lists/iterables in the repetition have matching lengths");
                builder.build()
            }
        }
    }
}

/// Find similar names using edit distance (for suggestions)
fn find_similar_names<'a>(target: &str, candidates: &'a [Text]) -> Vec<&'a str> {
    let target_lower = target.to_lowercase();
    let mut similar: Vec<(&'a str, usize)> = candidates
        .iter()
        .filter_map(|c| {
            let c_lower = c.to_lowercase();
            let dist = edit_distance(&target_lower, &c_lower);
            // Only suggest if edit distance is reasonably small (< 3) or prefix matches
            if dist < 3 || c_lower.starts_with(target_lower.as_str()) || target_lower.starts_with(c_lower.as_str()) {
                Some((c.as_str(), dist))
            } else {
                None
            }
        })
        .collect();
    similar.sort_by_key(|(_, d)| *d);
    similar.into_iter().take(3).map(|(s, _)| s).collect()
}

/// Simple edit distance calculation for name suggestions
fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 { return n; }
    if n == 0 { return m; }

    let mut dp = vec![vec![0usize; n + 1]; m + 1];

    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }

    dp[m][n]
}

pub type Result<T> = std::result::Result<T, TypeError>;

/// Performance metrics for type checking
#[derive(Debug, Clone, Default)]
pub struct TypeCheckMetrics {
    /// Number of synthesis operations
    pub synth_count: usize,
    /// Number of checking operations
    pub check_count: usize,
    /// Number of unifications
    pub unify_count: usize,
    /// Time spent in type checking (microseconds)
    pub time_us: u64,
    /// Number of refinement checks
    pub refinement_checks: usize,
    /// Number of protocol checks
    pub protocol_checks: usize,
}

impl TypeCheckMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn report(&self) -> Text {
        format!(
            "Type checking metrics:\n\
             - Synthesis ops: {}\n\
             - Checking ops: {}\n\
             - Unifications: {}\n\
             - Refinement checks: {}\n\
             - Protocol checks: {}\n\
             - Time: {} μs ({:.2} ms)",
            self.synth_count,
            self.check_count,
            self.unify_count,
            self.refinement_checks,
            self.protocol_checks,
            self.time_us,
            self.time_us as f64 / 1000.0
        )
        .into()
    }
}
