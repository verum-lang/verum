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
//!  │
//!  ▼
//! ┌─────────────────────────────────────────┐
//! │ VBC MONOMORPHIZATION │
//! │ │
//! │ 1. Build InstantiationGraph │
//! │ 2. Apply TypeSubstitution │
//! │ 3. Specialize bytecode │
//! │ 4. Run optimization passes │
//! │ 5. Cache results │
//! └─────────────────────────────────────────┘
//!  │
//!  ▼
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

mod cache;
mod graph;
mod merger;
mod optimizer;
mod phase;
mod resolver;
mod specializer;
mod substitution;

/// **MONO-DEFAULT-ON-1 (task #44)** — the ONE authority for the
/// monomorphization + coherent-reference-model gate.
///
/// Default is ON: the structural specializer (canonical codec,
/// jump-safe rewrites), the instantiation fixpoint, spec-time Display
/// expansion and the uniform-i64/pointer-tagging reference model ship
/// enabled.  `VERUM_DISABLE_MONO_AOT=1` is the kill-switch (triage
/// sweeps / A/B); the legacy opt-in spelling `VERUM_ENABLE_MONO_AOT`
/// remains accepted as a redundant force-on so existing scripts keep
/// working.  Every consumer (codegen seeding, AOT ref-tagging, the
/// pointer-store branch) reads THIS function — per-site env reads were
/// exactly the drift channel the one-authority rule exists to kill.
pub fn mono_aot_enabled() -> bool {
    static GATE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GATE.get_or_init(|| {
        if std::env::var_os("VERUM_DISABLE_MONO_AOT").is_some_and(|v| v != "0") {
            return false;
        }
        true
    })
}

pub use cache::MonomorphizationCache;
pub use graph::{
    CallSite, InstantiationGraph, InstantiationKey, InstantiationRequest, SourceLocation,
};
pub use merger::{FunctionMapping, IncrementalMerger, MergeError, MergeStats, ModuleMerger};
pub use optimizer::{OptimizationStats, SpecializationOptimizer};
pub use phase::{
    MonoPhaseConfig, MonoPhaseError, MonoPhaseResult, MonomorphizationPhase, monomorphize,
    monomorphize_minimal, monomorphize_with_core,
};
pub use resolver::{
    CacheMetadata, MonomorphizationResolver, ResolvedSpecialization, ResolverError, ResolverStats,
    Version,
};
pub use specializer::{
    BytecodeSpecializer, SpecializationError, SpecializedFunction, SpecializerStats, TypeLayout,
};
pub use substitution::TypeSubstitution;

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
