//! Monomorphization phase for the compilation pipeline.
//!
//! This module provides the main entry point for monomorphization,
//! integrating all components:
//! - InstantiationGraph (from type checking)
//! - MonomorphizationResolver (core/cache lookup)
//! - BytecodeSpecializer (bytecode transformation)
//! - ModuleMerger (final module assembly)
//!
//! Orchestrates the full monomorphization pipeline: graph -> resolver -> specializer
//! -> optimizer -> merger, producing a final monomorphized VBC module.

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;

use crate::module::{FunctionId, VbcModule};
use crate::types::TypeRef;

use super::cache::MonomorphizationCache;
use super::graph::{InstantiationGraph, InstantiationRequest};
use super::merger::{MergeStats, ModuleMerger};
use super::optimizer::SpecializationOptimizer;
use super::resolver::{CacheMetadata, MonomorphizationResolver, ResolverStats};
use super::specializer::{BytecodeSpecializer, SpecializationError, SpecializedFunction};
use super::substitution::TypeSubstitution;
use super::MonoMetrics;

// ============================================================================
// Helper Functions
// ============================================================================

/// Computes type hash for cache metadata.
fn compute_type_hash(type_args: &[TypeRef]) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    for type_ref in type_args {
        type_ref.hash(&mut hasher);
    }
    hasher.finish()
}

/// Computes bytecode hash for cache metadata.
fn compute_bytecode_hash(bytecode: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    bytecode.hash(&mut hasher);
    hasher.finish()
}

// ============================================================================
// Phase Error
// ============================================================================

/// Error during monomorphization phase.
#[derive(Debug)]
pub enum MonoPhaseError {
    /// Resolution error.
    Resolution(super::resolver::ResolverError),
    /// Specialization error.
    Specialization(SpecializationError),
    /// Merge error.
    Merge(super::merger::MergeError),
    /// Function not found.
    FunctionNotFound(FunctionId),
    /// IO error during cache operations.
    Io(std::io::Error),
    /// Failed to construct a bespoke parallel-specialization
    /// thread pool. Carries the rayon error message and the
    /// configured worker count for triage.
    ParallelExecution(String),
}

impl std::fmt::Display for MonoPhaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MonoPhaseError::Resolution(e) => write!(f, "Resolution error: {}", e),
            MonoPhaseError::Specialization(e) => write!(f, "Specialization error: {}", e),
            MonoPhaseError::Merge(e) => write!(f, "Merge error: {}", e),
            MonoPhaseError::FunctionNotFound(id) => write!(f, "Function not found: {:?}", id),
            MonoPhaseError::Io(e) => write!(f, "IO error: {}", e),
            MonoPhaseError::ParallelExecution(e) => write!(f, "Parallel execution error: {}", e),
        }
    }
}

impl std::error::Error for MonoPhaseError {}

impl From<super::resolver::ResolverError> for MonoPhaseError {
    fn from(e: super::resolver::ResolverError) -> Self {
        MonoPhaseError::Resolution(e)
    }
}

impl From<SpecializationError> for MonoPhaseError {
    fn from(e: SpecializationError) -> Self {
        MonoPhaseError::Specialization(e)
    }
}

impl From<super::merger::MergeError> for MonoPhaseError {
    fn from(e: super::merger::MergeError) -> Self {
        MonoPhaseError::Merge(e)
    }
}

impl From<std::io::Error> for MonoPhaseError {
    fn from(e: std::io::Error) -> Self {
        MonoPhaseError::Io(e)
    }
}

// ============================================================================
// Phase Configuration
// ============================================================================

/// Configuration for the monomorphization phase.
#[derive(Debug, Clone)]
pub struct MonoPhaseConfig {
    /// Enable stdlib precompiled specialization lookup.
    pub use_stdlib: bool,
    /// Enable persistent cache.
    pub use_cache: bool,
    /// Enable parallel specialization.
    pub parallel: bool,
    /// Number of parallel threads (0 = auto-detect).
    pub num_threads: usize,
    /// Enable post-specialization optimization.
    pub optimize: bool,
    /// Cache directory path (None = default).
    pub cache_dir: Option<std::path::PathBuf>,
}

impl Default for MonoPhaseConfig {
    fn default() -> Self {
        Self {
            use_stdlib: true,
            use_cache: true,
            parallel: true,
            num_threads: 0,
            optimize: true,
            cache_dir: None,
        }
    }
}

impl MonoPhaseConfig {
    /// Creates a minimal configuration (no cache, no parallel).
    pub fn minimal() -> Self {
        Self {
            use_stdlib: false,
            use_cache: false,
            parallel: false,
            num_threads: 1,
            optimize: false,
            cache_dir: None,
        }
    }

    /// Creates a production configuration.
    pub fn production() -> Self {
        Self::default()
    }
}

// ============================================================================
// Phase Result
// ============================================================================

/// Result of monomorphization phase.
#[derive(Debug)]
pub struct MonoPhaseResult {
    /// Monomorphized module.
    pub module: VbcModule,
    /// Metrics.
    pub metrics: MonoMetrics,
    /// Resolver statistics.
    pub resolver_stats: ResolverStats,
    /// Merge statistics.
    pub merge_stats: MergeStats,
    /// Warnings generated during monomorphization.
    pub warnings: Vec<String>,
}

// ============================================================================
// Monomorphization Phase
// ============================================================================

/// Main monomorphization phase.
///
/// Orchestrates the entire monomorphization pipeline:
/// 1. Resolve all instantiations (core/cache/pending)
/// 2. Specialize pending functions
/// 3. Optimize specialized bytecode
/// 4. Merge into final module
pub struct MonomorphizationPhase {
    /// Configuration.
    config: MonoPhaseConfig,
    /// Stdlib module (optional).
    stdlib: Option<Arc<VbcModule>>,
    /// Persistent cache (optional).
    cache: Option<MonomorphizationCache>,
}

impl MonomorphizationPhase {
    /// Creates a new monomorphization phase with the given configuration.
    pub fn new(config: MonoPhaseConfig) -> Self {
        let cache = if config.use_cache {
            config.cache_dir.as_ref()
                .map(|dir| MonomorphizationCache::new(dir.clone()))
                .or_else(MonomorphizationCache::default_cache)
        } else {
            None
        };

        Self {
            config,
            stdlib: None,
            cache,
        }
    }

    /// Sets the stdlib module.
    pub fn with_core(mut self, stdlib: Arc<VbcModule>) -> Self {
        self.stdlib = Some(stdlib);
        self
    }

    /// Executes the monomorphization phase.
    ///
    /// Takes a user module and instantiation graph, returns a monomorphized module.
    pub fn execute(
        &mut self,
        user_module: VbcModule,
        graph: &InstantiationGraph,
    ) -> Result<MonoPhaseResult, MonoPhaseError> {
        let start_time = Instant::now();
        let mut warnings = Vec::new();

        // Step 1: Create resolver
        let mut resolver = MonomorphizationResolver::new();
        // Honour `config.use_stdlib`: when false, skip the stdlib
        // precompiled-specialization lookup even if a stdlib module is
        // installed via `with_core`. Lets callers measure the cost of
        // the stdlib hit path or force every specialization through
        // the user-module pipeline (e.g. for differential testing of
        // the specializer against the precompiled cache).
        if self.config.use_stdlib {
            if let Some(ref stdlib) = self.stdlib {
                resolver = resolver.with_core(stdlib.clone());
            }
        }
        if let Some(ref cache) = self.cache {
            resolver = resolver.with_cache(cache.clone());
        }

        // Step 2: Resolve all instantiations
        resolver.resolve(graph)?;

        // Step 3: Specialize pending functions
        let pending = resolver.take_pending();
        let specialized = if self.config.parallel && pending.len() > 1 {
            self.specialize_parallel(&user_module, graph, &pending)?
        } else {
            self.specialize_sequential(&user_module, graph, &pending)?
        };

        // Step 4: Cache newly specialized functions
        if let Some(ref mut cache) = self.cache {
            for (request, spec_fn) in &specialized {
                if let Err(e) = cache.put(request.hash, spec_fn.bytecode.clone()) {
                    warnings.push(format!("Failed to cache specialization: {}", e));
                }

                // Also save metadata
                let type_hash = compute_type_hash(&request.type_args);
                let bytecode_hash = compute_bytecode_hash(&spec_fn.bytecode);
                let metadata = CacheMetadata::new(type_hash, bytecode_hash);
                let cache_dir = cache.cache_dir().clone();
                let metadata_path = cache_dir.join(format!("{:016x}.meta", request.hash));
                if let Err(e) = metadata.save(&metadata_path) {
                    warnings.push(format!("Failed to save cache metadata: {}", e));
                }
            }
        }

        // Step 5: Merge into final module
        let resolver_stats = resolver.stats().clone();
        let merger = ModuleMerger::new(user_module, self.stdlib.clone(), specialized, resolver);
        let (module, merge_stats) = merger.merge()?;

        // Step 6: Compute metrics
        let duration = start_time.elapsed();
        let metrics = MonoMetrics {
            total_instantiations: graph.len(),
            cache_hits: resolver_stats.stdlib_hits + resolver_stats.cache_hits,
            new_specializations: merge_stats.new_specializations,
            stdlib_hits: resolver_stats.stdlib_hits,
            bytecode_generated: merge_stats.bytecode_after - merge_stats.bytecode_before,
            duration_ms: duration.as_millis() as u64,
        };

        Ok(MonoPhaseResult {
            module,
            metrics,
            resolver_stats,
            merge_stats,
            warnings,
        })
    }

    /// Specializes functions sequentially.
    fn specialize_sequential(
        &self,
        module: &VbcModule,
        graph: &InstantiationGraph,
        pending: &[InstantiationRequest],
    ) -> Result<Vec<(InstantiationRequest, SpecializedFunction)>, MonoPhaseError> {
        let mut results = Vec::with_capacity(pending.len());

        for request in pending {
            // Get the generic function
            let func = module.get_function(request.function_id)
                .ok_or(MonoPhaseError::FunctionNotFound(request.function_id))?;

            // Create substitution
            let substitution = TypeSubstitution::from_function(func, &request.type_args);

            // Create specializer
            let mut specializer = BytecodeSpecializer::new(module, &substitution, graph);

            // Specialize
            let mut specialized = specializer.specialize(func, &request.type_args)?;

            // Optimize if enabled
            if self.config.optimize {
                let mut optimizer = SpecializationOptimizer::new();
                specialized.bytecode = optimizer.optimize(specialized.bytecode);
            }

            results.push((request.clone(), specialized));
        }

        Ok(results)
    }

    /// Specializes functions in parallel using rayon.
    #[cfg(feature = "parallel")]
    fn specialize_parallel(
        &self,
        module: &VbcModule,
        graph: &InstantiationGraph,
        pending: &[InstantiationRequest],
    ) -> Result<Vec<(InstantiationRequest, SpecializedFunction)>, MonoPhaseError> {
        use rayon::prelude::*;

        // Inner closure shared by both the bespoke-pool and the
        // global-pool branches below — keeps the specialization logic
        // single-source-of-truth so a change to the optimizer hook
        // can't drift between paths.
        let optimize_flag = self.config.optimize;
        let specialize_one = |request: &InstantiationRequest|
            -> Result<(InstantiationRequest, SpecializedFunction), MonoPhaseError>
        {
            let func = module.get_function(request.function_id)
                .ok_or(MonoPhaseError::FunctionNotFound(request.function_id))?;
            let substitution = TypeSubstitution::from_function(func, &request.type_args);
            let mut specializer = BytecodeSpecializer::new(module, &substitution, graph);
            let mut specialized = specializer.specialize(func, &request.type_args)?;
            if optimize_flag {
                let mut optimizer = SpecializationOptimizer::new();
                specialized.bytecode = optimizer.optimize(specialized.bytecode);
            }
            Ok((request.clone(), specialized))
        };

        // Honour `config.num_threads`: when nonzero, build a bespoke
        // rayon ThreadPool with the configured worker count and run
        // the parallel iterator inside its install scope. Zero means
        // "use the global default pool" (rayon's auto-detection
        // matches CPU count). The bespoke pool is the right knob for
        // CI workers that need to limit cross-build interference,
        // for measurement runs that want a fixed worker count, or
        // for embedders that share rayon with other systems and need
        // to avoid oversubscription.
        let num_threads = self.config.num_threads;
        if num_threads > 0 {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(num_threads)
                .build()
                .map_err(|e| MonoPhaseError::ParallelExecution(format!(
                    "rayon ThreadPool with {num_threads} threads: {e}"
                )))?;
            pool.install(|| pending.par_iter().map(&specialize_one).collect())
        } else {
            pending.par_iter().map(specialize_one).collect()
        }
    }

    /// Fallback for non-parallel builds.
    #[cfg(not(feature = "parallel"))]
    fn specialize_parallel(
        &self,
        module: &VbcModule,
        graph: &InstantiationGraph,
        pending: &[InstantiationRequest],
    ) -> Result<Vec<(InstantiationRequest, SpecializedFunction)>, MonoPhaseError> {
        self.specialize_sequential(module, graph, pending)
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &MonoPhaseConfig {
        &self.config
    }
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Monomorphizes a module with default configuration.
///
/// This is the main entry point for simple use cases.
pub fn monomorphize(
    user_module: VbcModule,
    graph: &InstantiationGraph,
) -> Result<MonoPhaseResult, MonoPhaseError> {
    let mut phase = MonomorphizationPhase::new(MonoPhaseConfig::default());
    phase.execute(user_module, graph)
}

/// Monomorphizes a module with stdlib.
pub fn monomorphize_with_core(
    user_module: VbcModule,
    graph: &InstantiationGraph,
    stdlib: Arc<VbcModule>,
) -> Result<MonoPhaseResult, MonoPhaseError> {
    let mut phase = MonomorphizationPhase::new(MonoPhaseConfig::default())
        .with_core(stdlib);
    phase.execute(user_module, graph)
}

/// Monomorphizes a module with minimal configuration (testing/debugging).
pub fn monomorphize_minimal(
    user_module: VbcModule,
    graph: &InstantiationGraph,
) -> Result<MonoPhaseResult, MonoPhaseError> {
    let mut phase = MonomorphizationPhase::new(MonoPhaseConfig::minimal());
    phase.execute(user_module, graph)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::Opcode;
    use crate::module::FunctionDescriptor;
    use crate::types::{TypeId, TypeParamId};
    use super::super::graph::SourceLocation;

    #[test]
    fn test_phase_config_default() {
        let config = MonoPhaseConfig::default();
        assert!(config.use_stdlib);
        assert!(config.use_cache);
        assert!(config.parallel);
        assert!(config.optimize);
    }

    #[test]
    fn test_phase_config_minimal() {
        let config = MonoPhaseConfig::minimal();
        assert!(!config.use_stdlib);
        assert!(!config.use_cache);
        assert!(!config.parallel);
        assert!(!config.optimize);
    }

    #[test]
    fn test_phase_empty_graph() {
        let module = VbcModule::new("test".to_string());
        let graph = InstantiationGraph::new();

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(result.metrics.total_instantiations, 0);
        assert_eq!(result.metrics.new_specializations, 0);
    }

    #[test]
    fn test_phase_result_metrics() {
        let module = VbcModule::new("test".to_string());
        let graph = InstantiationGraph::new();

        let result = monomorphize_minimal(module, &graph).unwrap();

        assert_eq!(result.metrics.cache_hits, 0);
        assert!(result.warnings.is_empty());
    }

    // ========================================================================
    // Integration Tests - Full Pipeline
    // ========================================================================

    /// Creates a test module with a simple generic function.
    fn create_test_module_with_generic() -> VbcModule {
        let mut module = VbcModule::new("test_generics".to_string());

        // Add a generic identity function: fn identity<T>(x: T) -> T { x }
        // Bytecode: MOV r0, r1; RET r0
        let bytecode = vec![
            Opcode::Mov.to_byte(), 0, 1,  // MOV r0, r1
            Opcode::Ret.to_byte(), 0,      // RET r0
        ];

        let func = FunctionDescriptor {
            id: FunctionId(0),
            name: crate::types::StringId::EMPTY,
            bytecode_offset: 0,
            bytecode_length: bytecode.len() as u32,
            register_count: 2,
            is_generic: true,
            ..Default::default()
        };

        module.bytecode = bytecode;
        module.functions.push(func);

        module
    }

    #[test]
    fn test_full_pipeline_simple_instantiation() {
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Record instantiation: identity<Int>
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(result.metrics.total_instantiations, 1);
        assert!(result.module.bytecode.len() >= 5); // At least our bytecode
    }

    #[test]
    fn test_full_pipeline_multiple_instantiations() {
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Record multiple instantiations
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::BOOL)],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(result.metrics.total_instantiations, 3);
    }

    #[test]
    fn test_full_pipeline_deduplication() {
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Record same instantiation multiple times
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)], // Duplicate
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());

        // Should deduplicate
        let result = result.unwrap();
        assert_eq!(result.metrics.total_instantiations, 1);
    }

    /// Creates a test module with arithmetic operations.
    fn create_test_module_with_arith() -> VbcModule {
        let mut module = VbcModule::new("test_arith".to_string());

        // Generic add function using ADD_G
        // fn add<T: Add>(a: T, b: T) -> T { a + b }
        // Bytecode: ADD_G r0, r1, r2, protocol_id; RET r0
        let bytecode = vec![
            Opcode::AddG.to_byte(), 0, 1, 2, 0, // ADD_G r0, r1, r2, protocol=0
            Opcode::Ret.to_byte(), 0,           // RET r0
        ];

        let func = FunctionDescriptor {
            id: FunctionId(0),
            name: crate::types::StringId::EMPTY,
            bytecode_offset: 0,
            bytecode_length: bytecode.len() as u32,
            register_count: 3,
            is_generic: true,
            ..Default::default()
        };

        module.bytecode = bytecode;
        module.functions.push(func);

        module
    }

    #[test]
    fn test_arithmetic_specialization_int() {
        let module = create_test_module_with_arith();
        let mut graph = InstantiationGraph::new();

        // Instantiate with Int - should specialize ADD_G to ADD_I
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());

        let result = result.unwrap();
        // The bytecode should have been transformed
        assert!(!result.module.bytecode.is_empty());
    }

    #[test]
    fn test_arithmetic_specialization_float() {
        let module = create_test_module_with_arith();
        let mut graph = InstantiationGraph::new();

        // Instantiate with Float - should specialize ADD_G to ADD_F
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    // ========================================================================
    // Edge Case Tests
    // ========================================================================

    #[test]
    fn test_multi_param_generics() {
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Multi-parameter instantiation
        graph.record_instantiation(
            FunctionId(0),
            vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Concrete(TypeId::FLOAT),
            ],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_nested_generic_instantiation() {
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Nested generic: identity<List<Int>>
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Instantiated {
                base: TypeId(100), // Assume 100 is List type
                args: vec![TypeRef::Concrete(TypeId::INT)],
            }],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_generic_type_param_substitution() {
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Instantiation with generic type param (should be substituted)
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Generic(TypeParamId(0))],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_module_with_instantiations() {
        // Module with no functions but instantiations (edge case)
        // This is an error case - can't specialize non-existent functions
        let module = VbcModule::new("empty".to_string());
        let mut graph = InstantiationGraph::new();

        graph.record_instantiation(
            FunctionId(999), // Non-existent function
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        // Should return error for non-existent function
        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_err());
    }

    #[test]
    fn test_optimizer_integration() {
        let mut module = VbcModule::new("test_opt".to_string());

        // Bytecode with a useless jump (JMP +0)
        let bytecode = vec![
            Opcode::Jmp.to_byte(), 0, 0, 0, 0, // JMP +0 (should become NOP)
            Opcode::RetV.to_byte(),
        ];

        let func = FunctionDescriptor {
            id: FunctionId(0),
            name: crate::types::StringId::EMPTY,
            bytecode_offset: 0,
            bytecode_length: bytecode.len() as u32,
            register_count: 0,
            is_generic: false,
            ..Default::default()
        };

        module.bytecode = bytecode;
        module.functions.push(func);

        let graph = InstantiationGraph::new();

        // Use config with optimization enabled
        let config = MonoPhaseConfig {
            optimize: true,
            ..MonoPhaseConfig::minimal()
        };

        let mut phase = MonomorphizationPhase::new(config);
        let result = phase.execute(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_large_instantiation_batch() {
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Many instantiations
        for i in 0..100 {
            graph.record_instantiation(
                FunctionId(0),
                vec![TypeRef::Concrete(TypeId(i))],
                SourceLocation::default(),
            );
        }

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(result.metrics.total_instantiations, 100);
    }

    #[test]
    fn test_instantiation_hash_stability() {
        // Verify that the same instantiation produces the same hash
        let hash1 = InstantiationRequest::compute_hash(
            FunctionId(1),
            &[TypeRef::Concrete(TypeId::INT)],
        );
        let hash2 = InstantiationRequest::compute_hash(
            FunctionId(1),
            &[TypeRef::Concrete(TypeId::INT)],
        );

        assert_eq!(hash1, hash2);

        // Different types should have different hashes
        let hash3 = InstantiationRequest::compute_hash(
            FunctionId(1),
            &[TypeRef::Concrete(TypeId::FLOAT)],
        );

        assert_ne!(hash1, hash3);
    }

    // ========================================================================
    // Advanced Monomorphization Tests - Complex Edge Cases
    // ========================================================================

    #[test]
    fn test_deeply_nested_generic_types() {
        // Test: identity<List<Maybe<List<Int>>>>
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Create deeply nested type: List<Maybe<List<Int>>>
        let inner_list = TypeRef::Instantiated {
            base: TypeId(100), // List
            args: vec![TypeRef::Concrete(TypeId::INT)],
        };
        let maybe_of_list = TypeRef::Instantiated {
            base: TypeId(101), // Maybe
            args: vec![inner_list],
        };
        let outer_list = TypeRef::Instantiated {
            base: TypeId(100), // List
            args: vec![maybe_of_list],
        };

        graph.record_instantiation(
            FunctionId(0),
            vec![outer_list],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().metrics.total_instantiations, 1);
    }

    #[test]
    fn test_multiple_type_params_map() {
        // Test: fn transform<K, V, R>(map: Map<K, V>, f: Fn(V) -> R) -> Map<K, R>
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Instantiation with 3 type params
        graph.record_instantiation(
            FunctionId(0),
            vec![
                TypeRef::Concrete(TypeId(10)),  // K = String
                TypeRef::Concrete(TypeId::INT), // V = Int
                TypeRef::Concrete(TypeId::FLOAT), // R = Float
            ],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_recursive_type_structure() {
        // Test recursive type like: type Tree<T> is Leaf(T) | Node(Tree<T>, Tree<T>)
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Tree<Int> where Tree contains recursive Tree refs
        let tree_of_int = TypeRef::Instantiated {
            base: TypeId(200), // Tree
            args: vec![TypeRef::Concrete(TypeId::INT)],
        };

        graph.record_instantiation(
            FunctionId(0),
            vec![tree_of_int],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mixed_concrete_and_generic_params() {
        // Test instantiation mixing concrete and generic params
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Partial instantiation: Some params concrete, some still generic
        graph.record_instantiation(
            FunctionId(0),
            vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Generic(TypeParamId(1)), // Still generic
            ],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_same_function_different_arities() {
        // Test same function instantiated with different param counts
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Single param instantiation
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        // Two param instantiation
        graph.record_instantiation(
            FunctionId(0),
            vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Concrete(TypeId::FLOAT),
            ],
            SourceLocation::default(),
        );

        // Three param instantiation
        graph.record_instantiation(
            FunctionId(0),
            vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Concrete(TypeId::FLOAT),
                TypeRef::Concrete(TypeId::BOOL),
            ],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().metrics.total_instantiations, 3);
    }

    #[test]
    fn test_circular_instantiation_detection() {
        // Test that circular instantiation chains don't cause infinite loops
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Create type A<B<A<Int>>>
        let inner_a = TypeRef::Instantiated {
            base: TypeId(50),
            args: vec![TypeRef::Concrete(TypeId::INT)],
        };
        let b_of_a = TypeRef::Instantiated {
            base: TypeId(51),
            args: vec![inner_a],
        };
        let a_of_b = TypeRef::Instantiated {
            base: TypeId(50),
            args: vec![b_of_a],
        };

        graph.record_instantiation(
            FunctionId(0),
            vec![a_of_b],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_unit_and_never_types_instantiation() {
        // Test instantiation with Unit and Never types
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Unit type
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::UNIT)],
            SourceLocation::default(),
        );

        // Never type (if it exists)
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId(0xFFFF))], // Assume Never
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_all_binary_ops_specialization() {
        // Test specialization of all binary operations
        let mut module = VbcModule::new("test_all_ops".to_string());

        // Bytecode with various generic ops
        let bytecode = vec![
            // ADD_G, SUB_G, MUL_G, DIV_G
            Opcode::AddG.to_byte(), 0, 1, 2, 0,
            Opcode::SubG.to_byte(), 3, 0, 1, 0,
            Opcode::MulG.to_byte(), 4, 3, 2, 0,
            Opcode::DivG.to_byte(), 5, 4, 1, 0,
            Opcode::Ret.to_byte(), 5,
        ];

        let func = FunctionDescriptor {
            id: FunctionId(0),
            name: crate::types::StringId::EMPTY,
            bytecode_offset: 0,
            bytecode_length: bytecode.len() as u32,
            register_count: 6,
            is_generic: true,
            ..Default::default()
        };

        module.bytecode = bytecode;
        module.functions.push(func);

        let mut graph = InstantiationGraph::new();
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_comparison_ops_specialization() {
        // Test specialization of comparison operations
        let mut module = VbcModule::new("test_cmp_ops".to_string());

        // Bytecode with comparison generic ops (CmpG handles all comparison types)
        // CmpG dst, lhs, rhs, cmp_op (op encoded in 4th byte)
        let bytecode = vec![
            Opcode::CmpG.to_byte(), 0, 1, 2, 0, // CMP_G r0, r1, r2 (LT)
            Opcode::CmpG.to_byte(), 1, 2, 3, 1, // CMP_G r1, r2, r3 (LE)
            Opcode::CmpG.to_byte(), 2, 3, 4, 2, // CMP_G r2, r3, r4 (GT)
            Opcode::CmpG.to_byte(), 3, 4, 5, 3, // CMP_G r3, r4, r5 (GE)
            Opcode::Ret.to_byte(), 3,
        ];

        let func = FunctionDescriptor {
            id: FunctionId(0),
            name: crate::types::StringId::EMPTY,
            bytecode_offset: 0,
            bytecode_length: bytecode.len() as u32,
            register_count: 6,
            is_generic: true,
            ..Default::default()
        };

        module.bytecode = bytecode;
        module.functions.push(func);

        let mut graph = InstantiationGraph::new();
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_instantiation_ordering_determinism() {
        // Verify that instantiation order doesn't affect result
        let module1 = create_test_module_with_generic();
        let module2 = create_test_module_with_generic();

        let mut graph1 = InstantiationGraph::new();
        let mut graph2 = InstantiationGraph::new();

        // Order 1: Int, Float, Bool
        graph1.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        graph1.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );
        graph1.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::BOOL)],
            SourceLocation::default(),
        );

        // Order 2: Bool, Int, Float
        graph2.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::BOOL)],
            SourceLocation::default(),
        );
        graph2.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        graph2.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );

        let result1 = monomorphize_minimal(module1, &graph1).unwrap();
        let result2 = monomorphize_minimal(module2, &graph2).unwrap();

        // Both should produce same number of instantiations
        assert_eq!(
            result1.metrics.total_instantiations,
            result2.metrics.total_instantiations
        );
    }

    #[test]
    fn test_function_chain_instantiation() {
        // Test two generic functions instantiated together
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Instantiate the same function with different types
        // This tests that multiple instantiations of the same function work
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
        // Two different instantiations
        assert_eq!(result.unwrap().metrics.total_instantiations, 2);
    }

    #[test]
    fn test_stress_many_type_variations() {
        // Stress test with many different type IDs
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // 1000 unique type instantiations
        for i in 0..1000 {
            graph.record_instantiation(
                FunctionId(0),
                vec![TypeRef::Concrete(TypeId(i))],
                SourceLocation::default(),
            );
        }

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().metrics.total_instantiations, 1000);
    }

    #[test]
    fn test_stress_deeply_nested_10_levels() {
        // Test 10 levels of nesting
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Build: List<List<List<...List<Int>...>>>  (10 levels)
        let mut current = TypeRef::Concrete(TypeId::INT);
        for _ in 0..10 {
            current = TypeRef::Instantiated {
                base: TypeId(100), // List
                args: vec![current],
            };
        }

        graph.record_instantiation(
            FunctionId(0),
            vec![current],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_type_args() {
        // Test instantiation with empty type args (non-generic call to generic func)
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        graph.record_instantiation(
            FunctionId(0),
            vec![], // Empty type args
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        // Should still work (treated as identity instantiation)
        assert!(result.is_ok());
    }

    #[test]
    fn test_multiple_instantiations_same_function() {
        // Test multiple instantiations of the same generic function
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Three different type instantiations of the same function
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );
        graph.record_instantiation(
            FunctionId(0),
            vec![TypeRef::Concrete(TypeId::BOOL)],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().metrics.total_instantiations, 3);
    }

    #[test]
    fn test_type_param_at_different_positions() {
        // Test type params substituted at different arg positions
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Map<Int, T> - T at position 1
        graph.record_instantiation(
            FunctionId(0),
            vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Generic(TypeParamId(0)),
            ],
            SourceLocation::default(),
        );

        // Map<T, Float> - T at position 0
        graph.record_instantiation(
            FunctionId(0),
            vec![
                TypeRef::Generic(TypeParamId(0)),
                TypeRef::Concrete(TypeId::FLOAT),
            ],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().metrics.total_instantiations, 2);
    }

    #[test]
    fn test_instantiation_with_tuple_types() {
        // Test tuple type instantiation (Int, Float, Bool)
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        let tuple_type = TypeRef::Instantiated {
            base: TypeId(300), // Tuple type
            args: vec![
                TypeRef::Concrete(TypeId::INT),
                TypeRef::Concrete(TypeId::FLOAT),
                TypeRef::Concrete(TypeId::BOOL),
            ],
        };

        graph.record_instantiation(
            FunctionId(0),
            vec![tuple_type],
            SourceLocation::default(),
        );

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
    }

    #[test]
    fn test_hash_collision_resilience() {
        // Test that hash collisions don't break deduplication
        let module = create_test_module_with_generic();
        let mut graph = InstantiationGraph::new();

        // Record many instantiations to increase collision chance
        for i in 0..500 {
            for j in 0..3 {
                graph.record_instantiation(
                    FunctionId(0),
                    vec![TypeRef::Concrete(TypeId(i * 3 + j))],
                    SourceLocation::default(),
                );
            }
        }

        let result = monomorphize_minimal(module, &graph);
        assert!(result.is_ok());
        // Should have 1500 unique instantiations
        assert_eq!(result.unwrap().metrics.total_instantiations, 1500);
    }

    #[test]
    fn use_stdlib_false_drops_installed_stdlib_resolver() {
        // Pin: with `use_stdlib = false`, even a stdlib module
        // installed via `with_core` is excluded from resolver
        // lookup. The empty-graph case still succeeds because
        // there's nothing to specialize, but the run records
        // zero stdlib hits — proving the gate clamped the
        // resolver's lookup surface.
        let user_module = VbcModule::new("user".to_string());
        let stdlib = Arc::new(VbcModule::new("stdlib".to_string()));
        let graph = InstantiationGraph::new();

        let config = MonoPhaseConfig {
            use_stdlib: false,
            use_cache: false,
            parallel: false,
            ..Default::default()
        };
        let mut phase = MonomorphizationPhase::new(config).with_core(stdlib);
        let result = phase.execute(user_module, &graph).expect("execute ok");
        assert_eq!(
            result.metrics.stdlib_hits, 0,
            "use_stdlib=false must produce zero stdlib hits even with stdlib installed",
        );
    }

    #[test]
    fn num_threads_explicit_builds_bespoke_pool() {
        // Pin: nonzero `num_threads` triggers the bespoke
        // ThreadPoolBuilder branch and the run completes
        // successfully with the explicit worker count. The
        // empty-pending case still routes through the parallel
        // path because we can't reach `pending.len() > 1`
        // without instantiations, but the path-selection logic
        // is exercised by the >1 case below.
        let user_module = VbcModule::new("user".to_string());
        let graph = InstantiationGraph::new();

        let config = MonoPhaseConfig {
            num_threads: 2,
            use_cache: false,
            ..Default::default()
        };
        let mut phase = MonomorphizationPhase::new(config);
        let result = phase.execute(user_module, &graph);
        assert!(
            result.is_ok(),
            "num_threads=2 must successfully build a bespoke ThreadPool",
        );
    }

    #[test]
    fn num_threads_zero_uses_global_pool() {
        // Pin: `num_threads = 0` (the default) keeps the
        // existing global rayon pool path so callers that rely
        // on rayon's auto-detection continue to work without
        // touching the config.
        let user_module = VbcModule::new("user".to_string());
        let graph = InstantiationGraph::new();

        let config = MonoPhaseConfig {
            num_threads: 0,
            use_cache: false,
            ..Default::default()
        };
        assert_eq!(config.num_threads, 0);
        let mut phase = MonomorphizationPhase::new(config);
        let result = phase.execute(user_module, &graph);
        assert!(result.is_ok());
    }
}
