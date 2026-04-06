//! Module dependency graph and topological sorting.
//!
//! Manages module dependencies and determines compilation order.
//! Circular type dependencies (via references) are allowed, but circular
//! value dependencies (constants depending on each other) cause compile errors.
//! Function call cycles are allowed (resolved at runtime).
//!
//! The compiler uses topological sorting for compilation order:
//! 1. Build dependency graph from import statements
//! 2. Detect cycles in value dependencies
//! 3. Topologically sort modules
//! 4. Compile in sorted order (dependencies before dependents)

use crate::error::{ModuleError, ModuleResult};
use crate::path::{ModuleId, ModulePath};
use petgraph::algo::{is_cyclic_directed, toposort};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use verum_common::{List, Set, Text};

/// A node in the dependency graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DependencyNode {
    pub module_id: ModuleId,
    pub module_path: ModulePath,
    pub dependencies: List<ModuleId>,
}

impl DependencyNode {
    pub fn new(module_id: ModuleId, module_path: ModulePath) -> Self {
        Self {
            module_id,
            module_path,
            dependencies: List::new(),
        }
    }

    pub fn add_dependency(&mut self, dep: ModuleId) {
        if !self.dependencies.contains(&dep) {
            self.dependencies.push(dep);
        }
    }
}

/// Module dependency graph.
///
/// Tracks dependencies between modules and provides topological sorting
/// for compilation order.
#[derive(Debug)]
pub struct DependencyGraph {
    /// Directed graph of module dependencies
    graph: DiGraph<ModuleId, ()>,
    /// Mapping from ModuleId to graph NodeIndex
    id_to_node: HashMap<ModuleId, NodeIndex>,
    /// Mapping from NodeIndex to ModuleId
    node_to_id: HashMap<NodeIndex, ModuleId>,
    /// Module paths for error messages
    module_paths: HashMap<ModuleId, ModulePath>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            id_to_node: HashMap::new(),
            node_to_id: HashMap::new(),
            module_paths: HashMap::new(),
        }
    }

    /// Add a module to the graph.
    pub fn add_module(&mut self, id: ModuleId, path: ModulePath) {
        if !self.id_to_node.contains_key(&id) {
            let node = self.graph.add_node(id);
            self.id_to_node.insert(id, node);
            self.node_to_id.insert(node, id);
            self.module_paths.insert(id, path);
        }
    }

    /// Add a dependency edge: from depends on to.
    pub fn add_dependency(&mut self, from: ModuleId, to: ModuleId) -> ModuleResult<()> {
        let from_node = self
            .id_to_node
            .get(&from)
            .copied()
            .ok_or_else(|| ModuleError::Other {
                message: Text::from(format!("Module {:?} not found in graph", from)),
                span: None,
            })?;

        let to_node = self
            .id_to_node
            .get(&to)
            .copied()
            .ok_or_else(|| ModuleError::Other {
                message: Text::from(format!("Module {:?} not found in graph", to)),
                span: None,
            })?;

        self.graph.add_edge(from_node, to_node, ());
        Ok(())
    }

    /// Check if the graph has cycles.
    pub fn has_cycles(&self) -> bool {
        is_cyclic_directed(&self.graph)
    }

    /// Detect circular dependencies and return the cycle.
    pub fn detect_cycle(&self) -> Option<List<ModuleId>> {
        if !self.has_cycles() {
            return None;
        }

        // Use Tarjan's algorithm to find strongly connected components
        let sccs = petgraph::algo::tarjan_scc(&self.graph);

        // Find the first SCC with more than one node (a cycle)
        for scc in sccs {
            if scc.len() > 1 {
                let cycle: List<ModuleId> = scc
                    .iter()
                    .filter_map(|&node| self.node_to_id.get(&node).copied())
                    .collect();
                if !cycle.is_empty() {
                    return Some(cycle);
                }
            }
        }

        None
    }

    /// Get topological order for compilation.
    ///
    /// Returns modules in dependency order (dependencies before dependents).
    ///
    /// Returns modules sorted so dependencies come before dependents.
    /// Fails with a CircularDependency error if value-level cycles exist.
    pub fn topological_order(&self) -> ModuleResult<List<ModuleId>> {
        // Check for cycles first
        if let Some(cycle) = self.detect_cycle() {
            let cycle_paths = self.cycle_to_paths(&cycle);
            return Err(ModuleError::circular_dependency_with_paths(cycle, cycle_paths));
        }

        // Perform topological sort
        let sorted = toposort(&self.graph, None).map_err(|_| {
            let cycle = self.detect_cycle()
                .unwrap_or_else(|| List::from(vec![ModuleId::new(0)]));
            let cycle_paths = self.cycle_to_paths(&cycle);
            ModuleError::circular_dependency_with_paths(cycle, cycle_paths)
        })?;

        // Convert NodeIndex to ModuleId
        let mut result = List::new();
        for node in sorted {
            if let Some(&id) = self.node_to_id.get(&node) {
                result.push(id);
            }
        }

        Ok(result)
    }

    /// Get direct dependencies of a module.
    pub fn dependencies_of(&self, id: ModuleId) -> List<ModuleId> {
        let mut deps = List::new();

        if let Some(&node) = self.id_to_node.get(&id) {
            for edge in self.graph.edges(node) {
                // EdgeRef trait provides target() method
                let target_node = edge.target();
                if let Some(&dep_id) = self.node_to_id.get(&target_node) {
                    deps.push(dep_id);
                }
            }
        }

        deps
    }

    /// Get all transitive dependencies of a module.
    pub fn transitive_dependencies_of(&self, id: ModuleId) -> Set<ModuleId> {
        let mut visited = Set::new();
        let mut stack = vec![id];

        while let Some(current) = stack.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            for dep in self.dependencies_of(current) {
                if !visited.contains(&dep) {
                    stack.push(dep);
                }
            }
        }

        visited.remove(&id); // Don't include the module itself
        visited
    }

    /// Get modules that depend on this module.
    pub fn dependents_of(&self, id: ModuleId) -> List<ModuleId> {
        let mut dependents = List::new();

        if let Some(&node) = self.id_to_node.get(&id) {
            for edge in self
                .graph
                .edges_directed(node, petgraph::Direction::Incoming)
            {
                // For incoming edges, we want the source (the dependent module)
                // EdgeRef trait provides source() method
                let source_node = edge.source();
                if let Some(&dependent_id) = self.node_to_id.get(&source_node) {
                    dependents.push(dependent_id);
                }
            }
        }

        dependents
    }

    /// Get the path of a module.
    pub fn module_path(&self, id: ModuleId) -> Option<&ModulePath> {
        self.module_paths.get(&id)
    }

    /// Convert a cycle of module IDs to their paths.
    ///
    /// Used for generating informative error messages with cycle-breaking suggestions.
    pub fn cycle_to_paths(&self, cycle: &List<ModuleId>) -> List<ModulePath> {
        cycle
            .iter()
            .filter_map(|id| self.module_paths.get(id).cloned())
            .collect()
    }

    /// Get the number of modules in the graph.
    pub fn len(&self) -> usize {
        self.graph.node_count()
    }

    /// Check if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.graph.node_count() == 0
    }

    /// Clear the graph.
    pub fn clear(&mut self) {
        self.graph.clear();
        self.id_to_node.clear();
        self.node_to_id.clear();
        self.module_paths.clear();
    }

    /// Get all modules in the graph.
    pub fn all_modules(&self) -> impl Iterator<Item = ModuleId> + '_ {
        self.id_to_node.keys().copied()
    }

    /// Find independent groups of modules that can be loaded/compiled in parallel.
    ///
    /// Returns modules grouped by their depth in the dependency graph.
    /// Modules at the same depth have no dependencies on each other and can
    /// be processed in parallel.
    ///
    /// # Algorithm
    ///
    /// Uses a level-based approach where:
    /// - Level 0: Modules with no dependencies (leaves)
    /// - Level N: Modules whose dependencies are all at level < N
    ///
    /// # Example
    ///
    /// ```text
    /// A → B → C
    ///  ↘   ↗
    ///    D
    ///
    /// Level 0: [C]        (no dependencies)
    /// Level 1: [B, D]     (depend only on C)
    /// Level 2: [A]        (depends on B, D)
    /// ```
    ///
    /// Uses a level-based approach: Level 0 = modules with no dependencies (leaves),
    /// Level N = modules whose dependencies are all at level < N. Modules at
    /// the same level can be loaded/compiled in parallel.
    pub fn independent_groups(&self) -> List<List<ModuleId>> {
        let mut groups: List<List<ModuleId>> = List::new();
        let mut assigned: HashMap<ModuleId, usize> = HashMap::new();

        // Use topological order to assign levels.
        // Note: The graph has edges from dependent → dependency, so toposort
        // returns dependents before dependencies. We reverse to process
        // dependencies first, ensuring their levels are assigned before dependents.
        if let Ok(topo_order) = self.topological_order() {
            for module_id in topo_order.into_iter().rev() {
                // Find the maximum level of dependencies
                let deps = self.dependencies_of(module_id);
                let max_dep_level = deps
                    .iter()
                    .filter_map(|dep| assigned.get(dep))
                    .max()
                    .copied()
                    .unwrap_or(0);

                // This module's level is max_dep_level + 1 if it has deps, else 0
                let level = if deps.is_empty() {
                    0
                } else {
                    max_dep_level + 1
                };

                assigned.insert(module_id, level);

                // Ensure groups vector is large enough
                while groups.len() <= level {
                    groups.push(List::new());
                }

                // Add to appropriate group
                groups[level].push(module_id);
            }
        }

        groups
    }

    /// Get modules that have no dependencies (can be loaded first).
    ///
    /// These are the leaf modules in the dependency tree.
    pub fn root_modules(&self) -> List<ModuleId> {
        self.all_modules()
            .filter(|&id| self.dependencies_of(id).is_empty())
            .collect()
    }

    /// Get the in-degree (number of dependencies) for each module.
    ///
    /// Useful for parallel scheduling algorithms.
    pub fn in_degrees(&self) -> HashMap<ModuleId, usize> {
        let mut degrees = HashMap::new();
        for id in self.all_modules() {
            degrees.insert(id, self.dependencies_of(id).len());
        }
        degrees
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// INCREMENTAL COMPILATION SUPPORT
// =============================================================================

/// Compilation tier achieved for a module.
///
/// Verum uses a VBC-first architecture where all code passes through VBC
/// (Verum Bytecode) generation, then optionally progresses to higher tiers.
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CompilationTier {
    /// Source parsed but not yet compiled to VBC
    #[default]
    Parsed,
    /// VBC bytecode generated (Tier 0 - can be interpreted)
    Vbc,
    /// JIT Tier 1 - basic LLVM compilation without optimizations
    JitTier1,
    /// JIT Tier 2 - full LLVM optimizations
    JitTier2,
    /// AOT compiled - native code generated
    Aot,
}

impl CompilationTier {
    /// Check if VBC has been generated (tier >= Vbc)
    pub fn has_vbc(&self) -> bool {
        !matches!(self, CompilationTier::Parsed)
    }

    /// Check if native code has been generated (tier >= JitTier1)
    pub fn has_native(&self) -> bool {
        matches!(
            self,
            CompilationTier::JitTier1 | CompilationTier::JitTier2 | CompilationTier::Aot
        )
    }

    /// Check if fully optimized (tier >= JitTier2)
    pub fn is_optimized(&self) -> bool {
        matches!(self, CompilationTier::JitTier2 | CompilationTier::Aot)
    }
}

/// Backend-specific compilation state for dual-path architecture.
///
/// VBC can be compiled via two paths:
/// - CPU path: VBC → LLVM IR → native
/// - GPU path: VBC → MLIR → GPU kernels
///
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BackendState {
    /// CPU backend state (VBC → LLVM IR)
    pub cpu_compiled: bool,
    /// GPU backend state (VBC → MLIR)
    pub gpu_compiled: bool,
    /// Hash of generated LLVM IR (for CPU path)
    pub llvm_hash: Option<u64>,
    /// Hash of generated MLIR (for GPU path)
    pub mlir_hash: Option<u64>,
}

impl BackendState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_cpu_compiled(&mut self, llvm_hash: u64) {
        self.cpu_compiled = true;
        self.llvm_hash = Some(llvm_hash);
    }

    pub fn mark_gpu_compiled(&mut self, mlir_hash: u64) {
        self.gpu_compiled = true;
        self.mlir_hash = Some(mlir_hash);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Module state for incremental compilation.
///
/// Tracks the VBC-first compilation pipeline state for a module:
/// 1. Source content hash (for detecting source changes)
/// 2. VBC hash (for detecting VBC-level changes)
/// 3. Compilation tier achieved
/// 4. Backend-specific state (CPU/GPU)
///
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleState {
    /// Content hash of the source file
    pub content_hash: u64,
    /// Hash of generated VBC bytecode (if generated)
    pub vbc_hash: Option<u64>,
    /// Last modification time (if available)
    pub modified_time: Option<std::time::SystemTime>,
    /// Current compilation tier achieved
    pub tier: CompilationTier,
    /// Backend-specific compilation state
    pub backend: BackendState,
    /// Whether the module has been compiled in this session
    pub compiled: bool,
    /// Whether the module needs recompilation
    pub dirty: bool,
}

impl ModuleState {
    pub fn new(content_hash: u64) -> Self {
        Self {
            content_hash,
            vbc_hash: None,
            modified_time: None,
            tier: CompilationTier::Parsed,
            backend: BackendState::new(),
            compiled: false,
            dirty: true,
        }
    }

    pub fn with_modified_time(mut self, time: std::time::SystemTime) -> Self {
        self.modified_time = Some(time);
        self
    }

    /// Mark module as having VBC generated.
    pub fn mark_vbc_compiled(&mut self, vbc_hash: u64) {
        self.vbc_hash = Some(vbc_hash);
        self.tier = CompilationTier::Vbc;
    }

    /// Advance to a higher compilation tier.
    pub fn advance_tier(&mut self, tier: CompilationTier) {
        // Only advance, never regress
        if tier as u8 > self.tier as u8 {
            self.tier = tier;
        }
    }

    pub fn mark_compiled(&mut self) {
        self.compiled = true;
        self.dirty = false;
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.compiled = false;
        // When source changes, invalidate all downstream artifacts
        self.vbc_hash = None;
        self.tier = CompilationTier::Parsed;
        self.backend.reset();
    }

    /// Check if VBC needs generation.
    ///
    /// Returns true if VBC bytecode hasn't been generated yet.
    /// Note: When source changes, `mark_dirty()` resets tier to Parsed,
    /// so this will return true automatically.
    pub fn needs_vbc(&self) -> bool {
        !self.tier.has_vbc()
    }

    /// Check if native code needs generation to reach the target tier.
    ///
    /// Returns true if current tier is below the target tier.
    /// Note: When source changes, `mark_dirty()` resets tier to Parsed,
    /// so this will return true automatically for any target tier.
    pub fn needs_native(&self, target_tier: CompilationTier) -> bool {
        (self.tier as u8) < (target_tier as u8)
    }
}

/// Incremental compilation graph with change tracking.
///
/// Extends DependencyGraph with state tracking for incremental compilation.
/// When a module changes, all dependent modules are invalidated and need
/// recompilation.
///
/// # Invalidation Strategy
///
/// When module A changes:
/// 1. Mark A as dirty
/// 2. Find all transitive dependents of A (modules that import A, directly or transitively)
/// 3. Mark all dependents as dirty
/// 4. Return the list of modules that need recompilation in topological order
///
/// # Example
///
/// ```text
/// A -> B -> C
///  \       ^
///   -> D --+
///
/// If B changes: B, C, D need recompilation
/// If A changes: A, B, C, D need recompilation
/// ```
#[derive(Debug)]
pub struct IncrementalGraph {
    /// Base dependency graph
    pub graph: DependencyGraph,
    /// State tracking for each module
    states: HashMap<ModuleId, ModuleState>,
}

impl IncrementalGraph {
    pub fn new() -> Self {
        Self {
            graph: DependencyGraph::new(),
            states: HashMap::new(),
        }
    }

    /// Create from an existing dependency graph.
    pub fn from_graph(graph: DependencyGraph) -> Self {
        Self {
            graph,
            states: HashMap::new(),
        }
    }

    /// Add a module with its initial state.
    pub fn add_module(&mut self, id: ModuleId, path: ModulePath, content_hash: u64) {
        self.graph.add_module(id, path);
        self.states.insert(id, ModuleState::new(content_hash));
    }

    /// Add a module with state including modification time.
    pub fn add_module_with_time(
        &mut self,
        id: ModuleId,
        path: ModulePath,
        content_hash: u64,
        modified_time: std::time::SystemTime,
    ) {
        self.graph.add_module(id, path);
        self.states.insert(
            id,
            ModuleState::new(content_hash).with_modified_time(modified_time),
        );
    }

    /// Add a dependency edge.
    pub fn add_dependency(&mut self, from: ModuleId, to: ModuleId) -> ModuleResult<()> {
        self.graph.add_dependency(from, to)
    }

    /// Get the state of a module.
    pub fn get_state(&self, id: ModuleId) -> Option<&ModuleState> {
        self.states.get(&id)
    }

    /// Get mutable state of a module.
    pub fn get_state_mut(&mut self, id: ModuleId) -> Option<&mut ModuleState> {
        self.states.get_mut(&id)
    }

    /// Check if a module's content has changed.
    ///
    /// Compares the stored hash with the new hash.
    pub fn has_content_changed(&self, id: ModuleId, new_hash: u64) -> bool {
        match self.states.get(&id) {
            Some(state) => state.content_hash != new_hash,
            None => true, // New module is always "changed"
        }
    }

    /// Update a module's content hash and mark it dirty if changed.
    ///
    /// Returns true if the module was marked dirty (content changed).
    pub fn update_content(&mut self, id: ModuleId, new_hash: u64) -> bool {
        if let Some(state) = self.states.get_mut(&id) {
            if state.content_hash != new_hash {
                state.content_hash = new_hash;
                state.mark_dirty();
                return true;
            }
        }
        false
    }

    /// Invalidate a module and all its dependents.
    ///
    /// This marks the module and all modules that transitively depend on it
    /// as needing recompilation.
    ///
    /// Returns the list of invalidated module IDs.
    pub fn invalidate(&mut self, id: ModuleId) -> List<ModuleId> {
        let mut invalidated = List::new();

        // Mark the module itself dirty
        if let Some(state) = self.states.get_mut(&id) {
            state.mark_dirty();
            invalidated.push(id);
        }

        // Find and invalidate all transitive dependents
        let dependents = self.transitive_dependents_of(id);
        for dep_id in dependents {
            if let Some(state) = self.states.get_mut(&dep_id) {
                state.mark_dirty();
                invalidated.push(dep_id);
            }
        }

        invalidated
    }

    /// Get all modules that transitively depend on the given module.
    ///
    /// This returns modules that need to be recompiled when the given
    /// module changes.
    pub fn transitive_dependents_of(&self, id: ModuleId) -> Set<ModuleId> {
        let mut visited = Set::new();
        let mut stack = vec![id];

        while let Some(current) = stack.pop() {
            // Get direct dependents (modules that import current)
            for dependent in self.graph.dependents_of(current) {
                if !visited.contains(&dependent) {
                    visited.insert(dependent);
                    stack.push(dependent);
                }
            }
        }

        visited
    }

    /// Get modules that need recompilation in topological order.
    ///
    /// This returns dirty modules ordered so that dependencies are
    /// compiled before dependents.
    pub fn dirty_modules_in_order(&self) -> ModuleResult<List<ModuleId>> {
        // Get all dirty module IDs
        let dirty_ids: Set<ModuleId> = self
            .states
            .iter()
            .filter(|(_, state)| state.dirty)
            .map(|(id, _)| *id)
            .collect();

        // Get topological order of all modules
        // Note: The graph edges point from dependent to dependency,
        // so we need to reverse the order to get dependencies first.
        let topo_order = self.graph.topological_order()?;

        // Filter to only dirty modules and reverse to get dependencies first
        let result: List<ModuleId> = topo_order
            .into_iter()
            .rev()
            .filter(|id| dirty_ids.contains(id))
            .collect();

        Ok(result)
    }

    /// Mark a module as compiled.
    pub fn mark_compiled(&mut self, id: ModuleId) {
        if let Some(state) = self.states.get_mut(&id) {
            state.mark_compiled();
        }
    }

    /// Mark all modules as needing recompilation.
    pub fn invalidate_all(&mut self) {
        for state in self.states.values_mut() {
            state.mark_dirty();
        }
    }

    /// Check if any module needs recompilation.
    pub fn has_dirty_modules(&self) -> bool {
        self.states.values().any(|s| s.dirty)
    }

    /// Get count of dirty modules.
    pub fn dirty_count(&self) -> usize {
        self.states.values().filter(|s| s.dirty).count()
    }

    /// Clear all state, keeping the dependency graph.
    pub fn reset_state(&mut self) {
        for state in self.states.values_mut() {
            state.compiled = false;
            state.dirty = true;
        }
    }

    /// Check if a specific module needs recompilation.
    pub fn is_dirty(&self, id: ModuleId) -> bool {
        self.states.get(&id).is_none_or(|s| s.dirty)
    }

    /// Check if a specific module has been compiled this session.
    pub fn is_compiled(&self, id: ModuleId) -> bool {
        self.states.get(&id).is_some_and(|s| s.compiled)
    }
}

impl Default for IncrementalGraph {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// CONTENT HASH HELPER
// =============================================================================

/// Compute a fast hash of source content for change detection.
///
/// Uses Blake3 for consistent hashing across the compiler pipeline.
/// Blake3 provides:
/// - Cryptographic security guarantees
/// - 3-10x faster than SHA-256
/// - SIMD acceleration on modern CPUs
pub fn compute_content_hash(content: &str) -> u64 {
    let hash = blake3::hash(content.as_bytes());
    // Truncate to u64 for cache keys (sufficient for change detection)
    let bytes = hash.as_bytes();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

#[cfg(test)]
mod incremental_tests {
    use super::*;

    #[test]
    fn test_incremental_graph_basic() {
        let mut graph = IncrementalGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);

        graph.add_module(a, ModulePath::from_str("a"), 100);
        graph.add_module(b, ModulePath::from_str("b"), 200);
        graph.add_module(c, ModulePath::from_str("c"), 300);

        // b depends on a, c depends on b
        graph.add_dependency(b, a).unwrap();
        graph.add_dependency(c, b).unwrap();

        // All should be dirty initially
        assert!(graph.is_dirty(a));
        assert!(graph.is_dirty(b));
        assert!(graph.is_dirty(c));
    }

    #[test]
    fn test_invalidation_propagates() {
        let mut graph = IncrementalGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);

        graph.add_module(a, ModulePath::from_str("a"), 100);
        graph.add_module(b, ModulePath::from_str("b"), 200);
        graph.add_module(c, ModulePath::from_str("c"), 300);

        graph.add_dependency(b, a).unwrap();
        graph.add_dependency(c, b).unwrap();

        // Mark all as compiled
        graph.mark_compiled(a);
        graph.mark_compiled(b);
        graph.mark_compiled(c);

        assert!(!graph.is_dirty(a));
        assert!(!graph.is_dirty(b));
        assert!(!graph.is_dirty(c));

        // Invalidate a - should propagate to b and c
        let invalidated = graph.invalidate(a);

        assert!(invalidated.contains(&a));
        assert!(invalidated.contains(&b));
        assert!(invalidated.contains(&c));
        assert_eq!(invalidated.len(), 3);
    }

    #[test]
    fn test_dirty_modules_in_order() {
        let mut graph = IncrementalGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);

        graph.add_module(a, ModulePath::from_str("a"), 100);
        graph.add_module(b, ModulePath::from_str("b"), 200);
        graph.add_module(c, ModulePath::from_str("c"), 300);

        // b -> a, c -> b (b depends on a, c depends on b)
        graph.add_dependency(b, a).unwrap();
        graph.add_dependency(c, b).unwrap();

        // Get dirty modules in order
        let order = graph.dirty_modules_in_order().unwrap();

        // a should come before b, b should come before c
        let a_pos = order.iter().position(|&x| x == a).unwrap();
        let b_pos = order.iter().position(|&x| x == b).unwrap();
        let c_pos = order.iter().position(|&x| x == c).unwrap();

        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
    }

    #[test]
    fn test_content_change_detection() {
        let mut graph = IncrementalGraph::new();

        let a = ModuleId::new(1);
        graph.add_module(a, ModulePath::from_str("a"), 100);

        // Same hash - no change
        assert!(!graph.has_content_changed(a, 100));

        // Different hash - changed
        assert!(graph.has_content_changed(a, 200));
    }

    #[test]
    fn test_update_content_marks_dirty() {
        let mut graph = IncrementalGraph::new();

        let a = ModuleId::new(1);
        graph.add_module(a, ModulePath::from_str("a"), 100);
        graph.mark_compiled(a);

        assert!(!graph.is_dirty(a));

        // Update with same hash - no change
        let changed = graph.update_content(a, 100);
        assert!(!changed);
        assert!(!graph.is_dirty(a));

        // Update with different hash - marks dirty
        let changed = graph.update_content(a, 200);
        assert!(changed);
        assert!(graph.is_dirty(a));
    }

    #[test]
    fn test_compute_content_hash() {
        let hash1 = compute_content_hash("fn main() {}");
        let hash2 = compute_content_hash("fn main() { }");
        let hash3 = compute_content_hash("fn main() {}");

        assert_ne!(hash1, hash2); // Different content
        assert_eq!(hash1, hash3); // Same content
    }

    #[test]
    fn test_transitive_dependents() {
        let mut graph = IncrementalGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);
        let d = ModuleId::new(4);

        graph.add_module(a, ModulePath::from_str("a"), 100);
        graph.add_module(b, ModulePath::from_str("b"), 200);
        graph.add_module(c, ModulePath::from_str("c"), 300);
        graph.add_module(d, ModulePath::from_str("d"), 400);

        // a <- b <- c, a <- d <- c (diamond pattern)
        graph.add_dependency(b, a).unwrap();
        graph.add_dependency(c, b).unwrap();
        graph.add_dependency(d, a).unwrap();
        graph.add_dependency(c, d).unwrap();

        let dependents = graph.transitive_dependents_of(a);

        assert!(dependents.contains(&b));
        assert!(dependents.contains(&c));
        assert!(dependents.contains(&d));
        assert!(!dependents.contains(&a)); // Not itself
    }

    // ==========================================================================
    // VBC-FIRST ARCHITECTURE TESTS
    // ==========================================================================

    #[test]
    fn test_compilation_tier_has_vbc() {
        assert!(!CompilationTier::Parsed.has_vbc());
        assert!(CompilationTier::Vbc.has_vbc());
        assert!(CompilationTier::JitTier1.has_vbc());
        assert!(CompilationTier::JitTier2.has_vbc());
        assert!(CompilationTier::Aot.has_vbc());
    }

    #[test]
    fn test_compilation_tier_has_native() {
        assert!(!CompilationTier::Parsed.has_native());
        assert!(!CompilationTier::Vbc.has_native());
        assert!(CompilationTier::JitTier1.has_native());
        assert!(CompilationTier::JitTier2.has_native());
        assert!(CompilationTier::Aot.has_native());
    }

    #[test]
    fn test_compilation_tier_is_optimized() {
        assert!(!CompilationTier::Parsed.is_optimized());
        assert!(!CompilationTier::Vbc.is_optimized());
        assert!(!CompilationTier::JitTier1.is_optimized());
        assert!(CompilationTier::JitTier2.is_optimized());
        assert!(CompilationTier::Aot.is_optimized());
    }

    #[test]
    fn test_backend_state() {
        let mut backend = BackendState::new();

        assert!(!backend.cpu_compiled);
        assert!(!backend.gpu_compiled);
        assert!(backend.llvm_hash.is_none());
        assert!(backend.mlir_hash.is_none());

        backend.mark_cpu_compiled(12345);
        assert!(backend.cpu_compiled);
        assert_eq!(backend.llvm_hash, Some(12345));

        backend.mark_gpu_compiled(67890);
        assert!(backend.gpu_compiled);
        assert_eq!(backend.mlir_hash, Some(67890));

        backend.reset();
        assert!(!backend.cpu_compiled);
        assert!(!backend.gpu_compiled);
    }

    #[test]
    fn test_module_state_vbc_tracking() {
        let mut state = ModuleState::new(100);

        assert_eq!(state.tier, CompilationTier::Parsed);
        assert!(state.vbc_hash.is_none());
        assert!(state.needs_vbc());

        // Mark VBC compiled
        state.mark_vbc_compiled(200);
        assert_eq!(state.tier, CompilationTier::Vbc);
        assert_eq!(state.vbc_hash, Some(200));
        assert!(!state.needs_vbc());
    }

    #[test]
    fn test_module_state_tier_advancement() {
        let mut state = ModuleState::new(100);

        state.mark_vbc_compiled(200);
        assert_eq!(state.tier, CompilationTier::Vbc);

        // Advance to JIT Tier 1
        state.advance_tier(CompilationTier::JitTier1);
        assert_eq!(state.tier, CompilationTier::JitTier1);

        // Can't regress to lower tier
        state.advance_tier(CompilationTier::Vbc);
        assert_eq!(state.tier, CompilationTier::JitTier1);

        // Can advance to higher tier
        state.advance_tier(CompilationTier::Aot);
        assert_eq!(state.tier, CompilationTier::Aot);
    }

    #[test]
    fn test_module_state_dirty_resets_all() {
        let mut state = ModuleState::new(100);

        // Progress through compilation
        state.mark_vbc_compiled(200);
        state.advance_tier(CompilationTier::JitTier2);
        state.backend.mark_cpu_compiled(300);
        state.mark_compiled();

        assert!(!state.dirty);
        assert_eq!(state.tier, CompilationTier::JitTier2);

        // Mark dirty - should reset everything
        state.mark_dirty();

        assert!(state.dirty);
        assert_eq!(state.tier, CompilationTier::Parsed);
        assert!(state.vbc_hash.is_none());
        assert!(!state.backend.cpu_compiled);
    }

    #[test]
    fn test_module_state_needs_native() {
        let mut state = ModuleState::new(100);

        // Needs native at any tier when not compiled
        assert!(state.needs_native(CompilationTier::JitTier1));

        state.mark_vbc_compiled(200);
        state.advance_tier(CompilationTier::JitTier1);
        state.mark_compiled();

        // JitTier1 achieved, don't need JitTier1
        assert!(!state.needs_native(CompilationTier::JitTier1));
        // But still need JitTier2
        assert!(state.needs_native(CompilationTier::JitTier2));
        // And AOT
        assert!(state.needs_native(CompilationTier::Aot));
    }

    // ==========================================================================
    // PARALLEL LOADING TESTS
    // ==========================================================================

    #[test]
    fn test_independent_groups_linear() {
        // Linear chain: A → B → C (A depends on B, B depends on C)
        let mut graph = DependencyGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);

        graph.add_module(a, ModulePath::from_str("a"));
        graph.add_module(b, ModulePath::from_str("b"));
        graph.add_module(c, ModulePath::from_str("c"));

        graph.add_dependency(a, b).unwrap(); // a depends on b
        graph.add_dependency(b, c).unwrap(); // b depends on c

        let groups = graph.independent_groups();

        // Should have 3 levels: [c], [b], [a]
        assert_eq!(groups.len(), 3);
        assert!(groups[0].contains(&c)); // Level 0: c (no deps)
        assert!(groups[1].contains(&b)); // Level 1: b (deps on c)
        assert!(groups[2].contains(&a)); // Level 2: a (deps on b)
    }

    #[test]
    fn test_independent_groups_parallel() {
        // Parallel modules: A, B, C have no dependencies
        let mut graph = DependencyGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);

        graph.add_module(a, ModulePath::from_str("a"));
        graph.add_module(b, ModulePath::from_str("b"));
        graph.add_module(c, ModulePath::from_str("c"));

        let groups = graph.independent_groups();

        // Should have 1 level with all modules (can load in parallel)
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 3);
        assert!(groups[0].contains(&a));
        assert!(groups[0].contains(&b));
        assert!(groups[0].contains(&c));
    }

    #[test]
    fn test_independent_groups_diamond() {
        // Diamond pattern:
        //   A
        //  / \
        // B   C
        //  \ /
        //   D
        // A depends on B and C, both B and C depend on D
        let mut graph = DependencyGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);
        let d = ModuleId::new(4);

        graph.add_module(a, ModulePath::from_str("a"));
        graph.add_module(b, ModulePath::from_str("b"));
        graph.add_module(c, ModulePath::from_str("c"));
        graph.add_module(d, ModulePath::from_str("d"));

        graph.add_dependency(a, b).unwrap();
        graph.add_dependency(a, c).unwrap();
        graph.add_dependency(b, d).unwrap();
        graph.add_dependency(c, d).unwrap();

        let groups = graph.independent_groups();

        // Should have 3 levels: [d], [b, c], [a]
        assert_eq!(groups.len(), 3);
        assert!(groups[0].contains(&d)); // Level 0: d (no deps)
        assert_eq!(groups[1].len(), 2); // Level 1: b and c (can load in parallel)
        assert!(groups[1].contains(&b));
        assert!(groups[1].contains(&c));
        assert!(groups[2].contains(&a)); // Level 2: a
    }

    #[test]
    fn test_root_modules() {
        let mut graph = DependencyGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);

        graph.add_module(a, ModulePath::from_str("a"));
        graph.add_module(b, ModulePath::from_str("b"));
        graph.add_module(c, ModulePath::from_str("c"));

        graph.add_dependency(a, b).unwrap(); // a depends on b

        let roots = graph.root_modules();

        // b and c have no dependencies
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&b));
        assert!(roots.contains(&c));
        assert!(!roots.contains(&a)); // a has dependency
    }

    #[test]
    fn test_in_degrees() {
        let mut graph = DependencyGraph::new();

        let a = ModuleId::new(1);
        let b = ModuleId::new(2);
        let c = ModuleId::new(3);

        graph.add_module(a, ModulePath::from_str("a"));
        graph.add_module(b, ModulePath::from_str("b"));
        graph.add_module(c, ModulePath::from_str("c"));

        graph.add_dependency(a, b).unwrap(); // a depends on b
        graph.add_dependency(a, c).unwrap(); // a depends on c

        let degrees = graph.in_degrees();

        assert_eq!(degrees[&a], 2); // a has 2 dependencies
        assert_eq!(degrees[&b], 0); // b has 0 dependencies
        assert_eq!(degrees[&c], 0); // c has 0 dependencies
    }
}
