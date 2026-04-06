//! Instantiation graph for tracking generic function instantiations.
//!
//! The instantiation graph is built during type checking and represents
//! all generic function instantiations required in a compilation unit.
//!
//! Built during type checking: records each (function_id, type_args) pair and
//! dependency edges between instantiations for topological specialization ordering.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::module::FunctionId;
use crate::types::TypeRef;

// ============================================================================
// Source Location
// ============================================================================

/// Source location for error messages and debugging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[derive(Default)]
pub struct SourceLocation {
    /// File ID in the source map.
    pub file_id: u32,
    /// Byte offset start.
    pub start: u32,
    /// Byte offset end.
    pub end: u32,
}


// ============================================================================
// Instantiation Request
// ============================================================================

/// Single instantiation request representing a generic function call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstantiationRequest {
    /// Generic function to specialize.
    pub function_id: FunctionId,
    /// Concrete type arguments.
    pub type_args: Vec<TypeRef>,
    /// Source location (for error messages).
    pub source: SourceLocation,
    /// Precomputed hash for deduplication and caching.
    pub hash: u64,
}

impl Hash for InstantiationRequest {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}

impl InstantiationRequest {
    /// Creates a new instantiation request.
    pub fn new(
        function_id: FunctionId,
        type_args: Vec<TypeRef>,
        source: SourceLocation,
    ) -> Self {
        let hash = Self::compute_hash(function_id, &type_args);
        Self {
            function_id,
            type_args,
            source,
            hash,
        }
    }

    /// Computes a stable hash for caching and deduplication.
    ///
    /// The hash includes:
    /// - Function ID
    /// - All type arguments (recursively hashed)
    pub fn compute_hash(function_id: FunctionId, type_args: &[TypeRef]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();

        // Hash function ID
        function_id.0.hash(&mut hasher);

        // Hash type arguments count
        type_args.len().hash(&mut hasher);

        // Hash each type argument recursively
        for type_arg in type_args {
            hash_type_ref(type_arg, &mut hasher);
        }

        hasher.finish()
    }

    /// Returns a key suitable for HashMap lookups.
    pub fn key(&self) -> InstantiationKey {
        InstantiationKey {
            function_id: self.function_id,
            type_args: self.type_args.clone(),
        }
    }
}

// ============================================================================
// Instantiation Key
// ============================================================================

/// Lightweight key for instantiation lookups (without source location).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstantiationKey {
    /// Generic function to specialize.
    pub function_id: FunctionId,
    /// Concrete type arguments.
    pub type_args: Vec<TypeRef>,
}

impl InstantiationKey {
    /// Creates a new key.
    pub fn new(function_id: FunctionId, type_args: Vec<TypeRef>) -> Self {
        Self { function_id, type_args }
    }

    /// Computes a stable hash for caching.
    pub fn compute_hash(&self) -> u64 {
        InstantiationRequest::compute_hash(self.function_id, &self.type_args)
    }
}

// ============================================================================
// Type Reference Hashing
// ============================================================================

/// Recursively hashes a TypeRef for stable cache keys.
fn hash_type_ref<H: Hasher>(type_ref: &TypeRef, hasher: &mut H) {
    // Tag byte to distinguish variants
    match type_ref {
        TypeRef::Concrete(id) => {
            0u8.hash(hasher);
            id.0.hash(hasher);
        }
        TypeRef::Generic(id) => {
            1u8.hash(hasher);
            id.0.hash(hasher);
        }
        TypeRef::Instantiated { base, args } => {
            2u8.hash(hasher);
            base.0.hash(hasher);
            args.len().hash(hasher);
            for arg in args {
                hash_type_ref(arg, hasher);
            }
        }
        TypeRef::Function { params, return_type, contexts } => {
            3u8.hash(hasher);
            params.len().hash(hasher);
            for param in params {
                hash_type_ref(param, hasher);
            }
            hash_type_ref(return_type, hasher);
            contexts.len().hash(hasher);
            for ctx in contexts {
                ctx.0.hash(hasher);
            }
        }
        TypeRef::Reference { inner, mutability, tier } => {
            4u8.hash(hasher);
            hash_type_ref(inner, hasher);
            (*mutability as u8).hash(hasher);
            (*tier as u8).hash(hasher);
        }
        TypeRef::Tuple(elements) => {
            5u8.hash(hasher);
            elements.len().hash(hasher);
            for elem in elements {
                hash_type_ref(elem, hasher);
            }
        }
        TypeRef::Array { element, length } => {
            6u8.hash(hasher);
            hash_type_ref(element, hasher);
            length.hash(hasher);
        }
        TypeRef::Slice(element) => {
            7u8.hash(hasher);
            hash_type_ref(element, hasher);
        }
        TypeRef::Rank2Function { type_param_count, params, return_type, contexts } => {
            8u8.hash(hasher);
            type_param_count.hash(hasher);
            params.len().hash(hasher);
            for param in params {
                hash_type_ref(param, hasher);
            }
            hash_type_ref(return_type, hasher);
            contexts.len().hash(hasher);
            for ctx in contexts {
                ctx.0.hash(hasher);
            }
        }
    }
}

// ============================================================================
// Instantiation Graph
// ============================================================================

/// Graph of all generic function instantiations in a compilation unit.
///
/// The graph tracks:
/// - All instantiation requests (deduplicated)
/// - Index by function ID for fast lookup
/// - Dependencies between instantiations (A calls B<T>)
/// - Mapping from instantiation to specialized function ID
pub struct InstantiationGraph {
    /// All instantiation requests (deduplicated).
    instantiations: Vec<InstantiationRequest>,

    /// Index: function_id -> indices in instantiations vec.
    by_function: HashMap<FunctionId, Vec<usize>>,

    /// Index: hash -> index in instantiations vec.
    by_hash: HashMap<u64, usize>,

    /// Dependency edges: instantiation index -> required instantiation indices.
    /// If A calls B<T>, then dependencies[A] contains B's index.
    dependencies: HashMap<usize, Vec<usize>>,

    /// Reverse dependencies: instantiation index -> dependent instantiation indices.
    /// If A calls B<T>, then reverse_deps[B] contains A's index.
    reverse_deps: HashMap<usize, Vec<usize>>,

    /// Mapping from instantiation index to specialized function ID.
    /// Filled during specialization phase.
    specialization_map: HashMap<usize, FunctionId>,
}

impl Default for InstantiationGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl InstantiationGraph {
    /// Creates a new empty graph.
    pub fn new() -> Self {
        Self {
            instantiations: Vec::new(),
            by_function: HashMap::new(),
            by_hash: HashMap::new(),
            dependencies: HashMap::new(),
            reverse_deps: HashMap::new(),
            specialization_map: HashMap::new(),
        }
    }

    /// Creates a graph with preallocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            instantiations: Vec::with_capacity(capacity),
            by_function: HashMap::with_capacity(capacity / 4),
            by_hash: HashMap::with_capacity(capacity),
            dependencies: HashMap::new(),
            reverse_deps: HashMap::new(),
            specialization_map: HashMap::new(),
        }
    }

    /// Records an instantiation request.
    ///
    /// Returns the index of the instantiation (existing or new).
    pub fn record(&mut self, request: InstantiationRequest) -> usize {
        // Check for existing instantiation
        if let Some(&idx) = self.by_hash.get(&request.hash) {
            return idx;
        }

        // Add new instantiation
        let idx = self.instantiations.len();
        self.by_hash.insert(request.hash, idx);
        self.by_function
            .entry(request.function_id)
            .or_default()
            .push(idx);
        self.instantiations.push(request);

        idx
    }

    /// Records an instantiation from components.
    pub fn record_instantiation(
        &mut self,
        function_id: FunctionId,
        type_args: Vec<TypeRef>,
        source: SourceLocation,
    ) -> usize {
        let request = InstantiationRequest::new(function_id, type_args, source);
        self.record(request)
    }

    /// Records a dependency: caller_idx depends on callee_idx.
    pub fn record_dependency(&mut self, caller_idx: usize, callee_idx: usize) {
        self.dependencies
            .entry(caller_idx)
            .or_default()
            .push(callee_idx);
        self.reverse_deps
            .entry(callee_idx)
            .or_default()
            .push(caller_idx);
    }

    /// Checks if an instantiation exists.
    pub fn contains(&self, function_id: FunctionId, type_args: &[TypeRef]) -> bool {
        let hash = InstantiationRequest::compute_hash(function_id, type_args);
        self.by_hash.contains_key(&hash)
    }

    /// Gets an instantiation by hash.
    pub fn get_by_hash(&self, hash: u64) -> Option<&InstantiationRequest> {
        self.by_hash.get(&hash).map(|&idx| &self.instantiations[idx])
    }

    /// Gets all instantiations for a function.
    pub fn get_instantiations(&self, function_id: FunctionId) -> Option<Vec<&InstantiationRequest>> {
        self.by_function.get(&function_id).map(|indices| {
            indices.iter().map(|&idx| &self.instantiations[idx]).collect()
        })
    }

    /// Gets the dependencies of an instantiation.
    pub fn get_dependencies(&self, idx: usize) -> Option<&[usize]> {
        self.dependencies.get(&idx).map(|v| v.as_slice())
    }

    /// Gets the reverse dependencies (dependents) of an instantiation.
    pub fn get_dependents(&self, idx: usize) -> Option<&[usize]> {
        self.reverse_deps.get(&idx).map(|v| v.as_slice())
    }

    /// Records a mapping from instantiation to specialized function.
    pub fn record_specialization(&mut self, idx: usize, specialized_fn: FunctionId) {
        self.specialization_map.insert(idx, specialized_fn);
    }

    /// Records a specialization by key.
    pub fn record_specialization_by_key(&mut self, key: &InstantiationKey, specialized_fn: FunctionId) {
        let hash = key.compute_hash();
        if let Some(&idx) = self.by_hash.get(&hash) {
            self.specialization_map.insert(idx, specialized_fn);
        }
    }

    /// Gets the specialized function for an instantiation.
    pub fn get_specialization(&self, idx: usize) -> Option<FunctionId> {
        self.specialization_map.get(&idx).copied()
    }

    /// Gets specialization by key.
    pub fn get_specialization_by_key(&self, key: &InstantiationKey) -> Option<FunctionId> {
        let hash = key.compute_hash();
        self.by_hash.get(&hash)
            .and_then(|&idx| self.specialization_map.get(&idx).copied())
    }

    /// Returns all instantiation requests.
    pub fn all_instantiations(&self) -> &[InstantiationRequest] {
        &self.instantiations
    }

    /// Returns all unique generic functions that have instantiations.
    pub fn generic_functions(&self) -> impl Iterator<Item = FunctionId> + '_ {
        self.by_function.keys().copied()
    }

    /// Returns the number of instantiations.
    pub fn len(&self) -> usize {
        self.instantiations.len()
    }

    /// Returns true if there are no instantiations.
    pub fn is_empty(&self) -> bool {
        self.instantiations.is_empty()
    }

    /// Returns instantiations in topological order (dependencies first).
    ///
    /// Uses Kahn's algorithm for topological sorting.
    /// Dependencies are processed before the nodes that depend on them.
    pub fn topological_order(&self) -> Vec<usize> {
        let n = self.instantiations.len();
        if n == 0 {
            return Vec::new();
        }

        // in_degree[i] = number of dependencies of node i that haven't been processed yet
        // A node can be processed when all its dependencies are processed (in_degree = 0)
        let mut in_degree = vec![0usize; n];
        for (node, deps) in &self.dependencies {
            if *node < n {
                in_degree[*node] = deps.len();
            }
        }

        // Start with nodes that have no dependencies
        let mut queue: Vec<usize> = (0..n)
            .filter(|&i| in_degree[i] == 0)
            .collect();
        let mut result = Vec::with_capacity(n);

        while let Some(node) = queue.pop() {
            result.push(node);

            // For each node that depends on the processed node, decrement its in_degree
            if let Some(dependents) = self.reverse_deps.get(&node) {
                for &dependent in dependents {
                    if dependent < n && in_degree[dependent] > 0 {
                        in_degree[dependent] -= 1;
                        if in_degree[dependent] == 0 {
                            queue.push(dependent);
                        }
                    }
                }
            }
        }

        // If we couldn't process all nodes, there's a cycle
        // Add remaining nodes (for recursive types)
        if result.len() < n {
            for i in 0..n {
                if !result.contains(&i) {
                    result.push(i);
                }
            }
        }

        result
    }

    /// Clears all data.
    pub fn clear(&mut self) {
        self.instantiations.clear();
        self.by_function.clear();
        self.by_hash.clear();
        self.dependencies.clear();
        self.reverse_deps.clear();
        self.specialization_map.clear();
    }
}

// ============================================================================
// Call Site
// ============================================================================

/// A call site where a generic function is instantiated.
#[derive(Debug, Clone)]
pub struct CallSite {
    /// Function containing the call.
    pub caller: FunctionId,
    /// Bytecode offset within caller.
    pub offset: u32,
    /// Called generic function.
    pub callee: FunctionId,
    /// Type arguments at this call site.
    pub type_args: Vec<TypeRef>,
    /// Source location.
    pub source: SourceLocation,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TypeId;

    #[test]
    fn test_instantiation_request_hash() {
        let req1 = InstantiationRequest::new(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        let req2 = InstantiationRequest::new(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation { file_id: 1, start: 10, end: 20 },
        );
        let req3 = InstantiationRequest::new(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );

        // Same function and type args should have same hash
        assert_eq!(req1.hash, req2.hash);
        // Different type args should have different hash
        assert_ne!(req1.hash, req3.hash);
    }

    #[test]
    fn test_instantiation_graph_deduplication() {
        let mut graph = InstantiationGraph::new();

        let idx1 = graph.record_instantiation(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        let idx2 = graph.record_instantiation(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation { file_id: 1, start: 10, end: 20 },
        );

        // Should be deduplicated
        assert_eq!(idx1, idx2);
        assert_eq!(graph.len(), 1);
    }

    #[test]
    fn test_instantiation_graph_by_function() {
        let mut graph = InstantiationGraph::new();

        graph.record_instantiation(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        graph.record_instantiation(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::FLOAT)],
            SourceLocation::default(),
        );
        graph.record_instantiation(
            FunctionId(2),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        let func1_insts = graph.get_instantiations(FunctionId(1)).unwrap();
        assert_eq!(func1_insts.len(), 2);

        let func2_insts = graph.get_instantiations(FunctionId(2)).unwrap();
        assert_eq!(func2_insts.len(), 1);
    }

    #[test]
    fn test_instantiation_graph_dependencies() {
        let mut graph = InstantiationGraph::new();

        let idx_a = graph.record_instantiation(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        let idx_b = graph.record_instantiation(
            FunctionId(2),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        graph.record_dependency(idx_a, idx_b);

        assert_eq!(graph.get_dependencies(idx_a), Some(&[idx_b][..]));
        assert_eq!(graph.get_dependents(idx_b), Some(&[idx_a][..]));
    }

    #[test]
    fn test_topological_order() {
        let mut graph = InstantiationGraph::new();

        // A -> B -> C
        let idx_c = graph.record_instantiation(
            FunctionId(3),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        let idx_b = graph.record_instantiation(
            FunctionId(2),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );
        let idx_a = graph.record_instantiation(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        graph.record_dependency(idx_a, idx_b);
        graph.record_dependency(idx_b, idx_c);

        let order = graph.topological_order();
        // C should come before B, B should come before A
        let pos_a = order.iter().position(|&x| x == idx_a).unwrap();
        let pos_b = order.iter().position(|&x| x == idx_b).unwrap();
        let pos_c = order.iter().position(|&x| x == idx_c).unwrap();

        assert!(pos_c < pos_b);
        assert!(pos_b < pos_a);
    }

    #[test]
    fn test_specialization_mapping() {
        let mut graph = InstantiationGraph::new();

        let idx = graph.record_instantiation(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
            SourceLocation::default(),
        );

        graph.record_specialization(idx, FunctionId(100));

        assert_eq!(graph.get_specialization(idx), Some(FunctionId(100)));

        let key = InstantiationKey::new(
            FunctionId(1),
            vec![TypeRef::Concrete(TypeId::INT)],
        );
        assert_eq!(graph.get_specialization_by_key(&key), Some(FunctionId(100)));
    }
}
