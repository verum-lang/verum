//! VBC Monomorphization
//!
//! This module implements generic function specialization for VBC bytecode.
//! It is used by the compilation phase to specialize generic functions with
//! concrete type arguments.
//!
//! # Architecture
//!
//! ```text
//! VBC Module (with generics)
//!       │
//!       ▼
//! ┌─────────────────────────────────────────┐
//! │         VBC MONOMORPHIZATION            │
//! │                                          │
//! │  1. Build InstantiationGraph            │
//! │  2. Apply TypeSubstitution              │
//! │  3. Specialize bytecode                 │
//! │  4. Run optimization passes             │
//! │  5. Cache results                       │
//! └─────────────────────────────────────────┘
//!       │
//!       ▼
//! VBC Module (fully monomorphized)
//! ```
//!
//! # Modules
//!
//! - [`graph`]: Instantiation tracking and dependency graph
//! - [`substitution`]: Type parameter substitution
//! - [`specializer`]: Bytecode specialization
//! - [`optimizer`]: Post-specialization optimization
//! - [`cache`]: Persistent specialization cache
//!
//! Monomorphization pipeline: (1) Resolution phase checks stdlib precompiled cache, then
//! persistent cache, scheduling misses for specialization. (2) Specialization phase loads
//! generic VBC, applies type substitution (replacing TypeRef::Param with concrete types),
//! and optimizes. (3) Merge phase combines user module VBC with stdlib precompiled and
//! newly monomorphized functions into a final fully-specialized VBC module. This enables
//! zero-overhead generics in AOT and optimized interpreter execution.

mod graph;
mod substitution;
mod specializer;
mod optimizer;
mod cache;
mod resolver;
mod merger;
mod phase;

pub use graph::{
    InstantiationGraph, InstantiationKey, InstantiationRequest,
    CallSite, SourceLocation,
};
pub use substitution::TypeSubstitution;
pub use specializer::{
    BytecodeSpecializer, SpecializedFunction, SpecializationError,
    SpecializerStats, TypeLayout,
};
pub use optimizer::{SpecializationOptimizer, OptimizationStats};
pub use cache::MonomorphizationCache;
pub use resolver::{
    MonomorphizationResolver, ResolvedSpecialization, ResolverStats,
    CacheMetadata, Version, ResolverError,
};
pub use merger::{
    ModuleMerger, IncrementalMerger, FunctionMapping,
    MergeError, MergeStats,
};
pub use phase::{
    MonomorphizationPhase, MonoPhaseConfig, MonoPhaseResult, MonoPhaseError,
    monomorphize, monomorphize_with_core, monomorphize_minimal,
};

/// Metrics for monomorphization.
#[derive(Debug, Clone, Default)]
pub struct MonoMetrics {
    /// Total instantiations processed.
    pub total_instantiations: usize,
    /// Cache hits (stdlib precompiled or persistent cache).
    pub cache_hits: usize,
    /// New specializations generated.
    pub new_specializations: usize,
    /// Stdlib precompiled hits.
    pub stdlib_hits: usize,
    /// Total bytecode generated (bytes).
    pub bytecode_generated: usize,
    /// Time spent in monomorphization.
    pub duration_ms: u64,
}
