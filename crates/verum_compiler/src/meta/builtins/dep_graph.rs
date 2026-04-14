//! Dependency Graph Analysis Intrinsics (Tier 1 - Requires MetaTypes)
//!
//! Provides compile-time dependency graph analysis for module relationships.
//! Uses the module registry from MetaContext to build and query the dependency graph.
//!
//! ## Dependency Query Functions
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `dep_dependencies_of(module)` | `(Text) -> List<Text>` | Direct dependencies |
//! | `dep_transitive_dependencies(module)` | `(Text) -> List<Text>` | All transitive deps |
//! | `dep_dependents_of(module)` | `(Text) -> List<Text>` | Who depends on this |
//! | `dep_depth(module)` | `(Text) -> Int` | Max depth in dep tree |
//!
//! ## Graph Analysis Functions
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `dep_find_cycles()` | `() -> List<List<Text>>` | Find all cycles |
//! | `dep_topological_order()` | `() -> List<Text>` | Topological sort |
//! | `dep_compilation_order()` | `() -> List<Text>` | Optimized compilation order |
//! | `dep_leaf_modules()` | `() -> List<Text>` | Modules with no deps |
//! | `dep_root_modules()` | `() -> List<Text>` | Modules nothing depends on |
//! | `dep_strongly_connected_components()` | `() -> List<List<Text>>` | SCCs (Tarjan's) |
//!
//! ## Context Requirements
//!
//! **Tier 1**: All functions require `using [MetaTypes]` context.

use std::collections::{HashMap, HashSet};

use verum_common::{List, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register dependency graph builtins with context requirements
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // ========================================================================
    // Dependency Query Functions (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("dep_dependencies_of"),
        BuiltinInfo::meta_types(
            meta_dep_dependencies_of,
            "Get direct dependencies of a module",
            "(Text) -> List<Text>",
        ),
    );
    map.insert(
        Text::from("dep_transitive_dependencies"),
        BuiltinInfo::meta_types(
            meta_dep_transitive_dependencies,
            "Get all transitive dependencies of a module",
            "(Text) -> List<Text>",
        ),
    );
    map.insert(
        Text::from("dep_dependents_of"),
        BuiltinInfo::meta_types(
            meta_dep_dependents_of,
            "Get modules that depend on this module",
            "(Text) -> List<Text>",
        ),
    );
    map.insert(
        Text::from("dep_depth"),
        BuiltinInfo::meta_types(
            meta_dep_depth,
            "Get maximum depth of module in dependency tree",
            "(Text) -> Int",
        ),
    );

    // ========================================================================
    // Graph Analysis Functions (Tier 1 - MetaTypes)
    // ========================================================================

    map.insert(
        Text::from("dep_find_cycles"),
        BuiltinInfo::meta_types(
            meta_dep_find_cycles,
            "Find all dependency cycles in the module graph",
            "() -> List<List<Text>>",
        ),
    );
    map.insert(
        Text::from("dep_topological_order"),
        BuiltinInfo::meta_types(
            meta_dep_topological_order,
            "Get modules in topological order",
            "() -> List<Text>",
        ),
    );
    map.insert(
        Text::from("dep_compilation_order"),
        BuiltinInfo::meta_types(
            meta_dep_compilation_order,
            "Get optimized compilation order for modules",
            "() -> List<Text>",
        ),
    );
    map.insert(
        Text::from("dep_leaf_modules"),
        BuiltinInfo::meta_types(
            meta_dep_leaf_modules,
            "Get modules with no dependencies (leaf nodes)",
            "() -> List<Text>",
        ),
    );
    map.insert(
        Text::from("dep_root_modules"),
        BuiltinInfo::meta_types(
            meta_dep_root_modules,
            "Get modules that nothing depends on (root nodes)",
            "() -> List<Text>",
        ),
    );
    map.insert(
        Text::from("dep_strongly_connected_components"),
        BuiltinInfo::meta_types(
            meta_dep_strongly_connected_components,
            "Find strongly connected components using Tarjan's algorithm",
            "() -> List<List<Text>>",
        ),
    );
}

// ============================================================================
// Internal: Dependency Graph Construction
// ============================================================================

/// A dependency graph built from the MetaContext module registry
struct DepGraph {
    /// module -> direct dependencies
    edges: HashMap<String, Vec<String>>,
    /// All module names
    modules: Vec<String>,
}

impl DepGraph {
    /// Build a dependency graph from the MetaContext module registry
    fn from_context(ctx: &MetaContext) -> Self {
        let mut edges: HashMap<String, Vec<String>> = HashMap::new();
        let mut module_set: HashSet<String> = HashSet::new();

        for (module_path, module_info) in ctx.module_registry.iter() {
            let module_name = module_path.to_string();
            module_set.insert(module_name.clone());

            let deps: Vec<String> = module_info
                .dependencies
                .iter()
                .map(|d| d.to_string())
                .collect();

            for dep in &deps {
                module_set.insert(dep.clone());
            }

            edges.insert(module_name, deps);
        }

        // Ensure all modules have an entry (even those only referenced as deps)
        for module in &module_set {
            edges.entry(module.clone()).or_default();
        }

        let mut modules: Vec<String> = module_set.into_iter().collect();
        modules.sort();

        Self { edges, modules }
    }

    /// Get direct dependencies of a module
    fn dependencies_of(&self, module: &str) -> Vec<String> {
        self.edges
            .get(module)
            .cloned()
            .unwrap_or_default()
    }

    /// Get transitive dependencies via BFS
    fn transitive_dependencies(&self, module: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut queue = std::collections::VecDeque::new();

        // Start with direct deps
        if let Some(direct) = self.edges.get(module) {
            for dep in direct {
                if visited.insert(dep.clone()) {
                    queue.push_back(dep.clone());
                }
            }
        }

        while let Some(current) = queue.pop_front() {
            if let Some(deps) = self.edges.get(&current) {
                for dep in deps {
                    if visited.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        // Don't include the module itself
        visited.remove(module);
        let mut result: Vec<String> = visited.into_iter().collect();
        result.sort();
        result
    }

    /// Get reverse dependencies (who depends on this module)
    fn dependents_of(&self, module: &str) -> Vec<String> {
        let mut dependents = Vec::new();
        for (mod_name, deps) in &self.edges {
            if deps.iter().any(|d| d == module) {
                dependents.push(mod_name.clone());
            }
        }
        dependents.sort();
        dependents
    }

    /// Compute maximum depth of a module in the dependency tree
    fn depth(&self, module: &str) -> i64 {
        let mut memo: HashMap<String, i64> = HashMap::new();
        self.depth_recursive(module, &mut memo, &mut HashSet::new())
    }

    fn depth_recursive(
        &self,
        module: &str,
        memo: &mut HashMap<String, i64>,
        visiting: &mut HashSet<String>,
    ) -> i64 {
        if let Some(&cached) = memo.get(module) {
            return cached;
        }

        // Cycle detection
        if !visiting.insert(module.to_string()) {
            return 0; // Break cycle with depth 0
        }

        let deps = self.dependencies_of(module);
        let max_child_depth = deps
            .iter()
            .map(|dep| self.depth_recursive(dep, memo, visiting))
            .max()
            .unwrap_or(-1);

        visiting.remove(module);
        let result = max_child_depth + 1;
        memo.insert(module.to_string(), result);
        result
    }

    /// Find all cycles using DFS
    fn find_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = Vec::new();
        let mut on_stack = HashSet::new();

        for module in &self.modules {
            if !visited.contains(module.as_str()) {
                self.find_cycles_dfs(
                    module,
                    &mut visited,
                    &mut rec_stack,
                    &mut on_stack,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    fn find_cycles_dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut Vec<String>,
        on_stack: &mut HashSet<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(node.to_string());
        rec_stack.push(node.to_string());
        on_stack.insert(node.to_string());

        if let Some(deps) = self.edges.get(node) {
            for dep in deps {
                if !visited.contains(dep.as_str()) {
                    self.find_cycles_dfs(dep, visited, rec_stack, on_stack, cycles);
                } else if on_stack.contains(dep.as_str()) {
                    // Found a cycle: extract from rec_stack
                    let cycle_start = rec_stack
                        .iter()
                        .position(|n| n == dep)
                        .unwrap_or(0);
                    let mut cycle: Vec<String> =
                        rec_stack[cycle_start..].to_vec();
                    cycle.push(dep.clone());
                    cycles.push(cycle);
                }
            }
        }

        rec_stack.pop();
        on_stack.remove(node);
    }

    /// Topological sort using Kahn's algorithm
    fn topological_order(&self) -> Vec<String> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();

        // Initialize in-degree for all modules
        for module in &self.modules {
            in_degree.insert(module.as_str(), 0);
        }

        // Count in-degrees
        for deps in self.edges.values() {
            for dep in deps {
                if let Some(count) = in_degree.get_mut(dep.as_str()) {
                    *count += 1;
                }
            }
        }

        // Start with modules that have no incoming edges (nothing depends on them)
        // Wait, in-degree here counts how many modules depend on each module.
        // For topological sort, we want to start with leaves (no dependencies).
        // Recompute: in-degree = number of deps each module HAS (outgoing in reverse)

        // Actually, topological sort for compilation: a module must be compiled
        // AFTER all its dependencies. So edges go from dependency -> dependent.
        // In our graph, edges[A] = [B, C] means A depends on B and C.
        // For topo sort, B and C must come before A.

        let mut dep_count: HashMap<&str, usize> = HashMap::new();
        for module in &self.modules {
            let count = self.edges.get(module.as_str()).map_or(0, |d| d.len());
            dep_count.insert(module.as_str(), count);
        }

        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        for module in &self.modules {
            if dep_count.get(module.as_str()) == Some(&0) {
                queue.push_back(module.clone());
            }
        }

        let mut result = Vec::new();

        while let Some(module) = queue.pop_front() {
            result.push(module.clone());

            // For each module that depends on `module`, reduce its dep count
            for (mod_name, deps) in &self.edges {
                if deps.iter().any(|d| d == &module) {
                    if let Some(count) = dep_count.get_mut(mod_name.as_str()) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            queue.push_back(mod_name.clone());
                        }
                    }
                }
            }
        }

        result
    }

    /// Get leaf modules (modules with no dependencies)
    fn leaf_modules(&self) -> Vec<String> {
        let mut leaves = Vec::new();
        for module in &self.modules {
            let deps = self.dependencies_of(module);
            if deps.is_empty() {
                leaves.push(module.clone());
            }
        }
        leaves.sort();
        leaves
    }

    /// Get root modules (modules that nothing depends on)
    fn root_modules(&self) -> Vec<String> {
        let mut depended_on: HashSet<String> = HashSet::new();
        for deps in self.edges.values() {
            for dep in deps {
                depended_on.insert(dep.clone());
            }
        }

        let mut roots = Vec::new();
        for module in &self.modules {
            if !depended_on.contains(module.as_str()) {
                roots.push(module.clone());
            }
        }
        roots.sort();
        roots
    }

    /// Tarjan's algorithm for strongly connected components
    fn strongly_connected_components(&self) -> Vec<Vec<String>> {
        let mut index_counter: usize = 0;
        let mut stack: Vec<String> = Vec::new();
        let mut on_stack: HashSet<String> = HashSet::new();
        let mut indices: HashMap<String, usize> = HashMap::new();
        let mut lowlinks: HashMap<String, usize> = HashMap::new();
        let mut result: Vec<Vec<String>> = Vec::new();

        for module in &self.modules {
            if !indices.contains_key(module) {
                self.tarjan_strongconnect(
                    module,
                    &mut index_counter,
                    &mut stack,
                    &mut on_stack,
                    &mut indices,
                    &mut lowlinks,
                    &mut result,
                );
            }
        }

        // Sort each SCC and sort the list of SCCs for deterministic output
        for scc in &mut result {
            scc.sort();
        }
        result.sort();
        result
    }

    fn tarjan_strongconnect(
        &self,
        v: &str,
        index_counter: &mut usize,
        stack: &mut Vec<String>,
        on_stack: &mut HashSet<String>,
        indices: &mut HashMap<String, usize>,
        lowlinks: &mut HashMap<String, usize>,
        result: &mut Vec<Vec<String>>,
    ) {
        let v_index = *index_counter;
        *index_counter += 1;
        indices.insert(v.to_string(), v_index);
        lowlinks.insert(v.to_string(), v_index);
        stack.push(v.to_string());
        on_stack.insert(v.to_string());

        // Consider successors
        if let Some(deps) = self.edges.get(v) {
            for w in deps {
                if !indices.contains_key(w.as_str()) {
                    // w has not been visited; recurse
                    self.tarjan_strongconnect(
                        w,
                        index_counter,
                        stack,
                        on_stack,
                        indices,
                        lowlinks,
                        result,
                    );
                    let w_low = lowlinks.get(w.as_str()).copied().unwrap_or(0);
                    if let Some(v_low) = lowlinks.get_mut(v) {
                        if w_low < *v_low {
                            *v_low = w_low;
                        }
                    }
                } else if on_stack.contains(w.as_str()) {
                    // w is on the stack -> in current SCC
                    let w_idx = indices.get(w.as_str()).copied().unwrap_or(0);
                    if let Some(v_low) = lowlinks.get_mut(v) {
                        if w_idx < *v_low {
                            *v_low = w_idx;
                        }
                    }
                }
            }
        }

        // If v is a root node, pop the SCC
        let v_low = lowlinks.get(v).copied().unwrap_or(0);
        let v_idx = indices.get(v).copied().unwrap_or(0);
        if v_low == v_idx {
            let mut scc = Vec::new();
            while let Some(w) = stack.pop() {
                on_stack.remove(&w);
                scc.push(w.clone());
                if w == v {
                    break;
                }
            }
            result.push(scc);
        }
    }
}

// ============================================================================
// Helper: Extract text arg
// ============================================================================

fn extract_text_arg(args: &List<ConstValue>, index: usize) -> Result<Text, MetaError> {
    match args.get(index) {
        Some(ConstValue::Text(t)) => Ok(t.clone()),
        Some(other) => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: other.type_name(),
        }),
        None => Err(MetaError::ArityMismatch {
            expected: index + 1,
            got: index,
        }),
    }
}

fn texts_to_const_array(texts: Vec<String>) -> ConstValue {
    let values: List<ConstValue> = texts
        .into_iter()
        .map(|s| ConstValue::Text(Text::from(s)))
        .collect();
    ConstValue::Array(values)
}

// ============================================================================
// Builtin Implementations
// ============================================================================

/// Get direct dependencies of a module
fn meta_dep_dependencies_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let module = extract_text_arg(&args, 0)?;
    let graph = DepGraph::from_context(ctx);
    let deps = graph.dependencies_of(module.as_str());
    Ok(texts_to_const_array(deps))
}

/// Get all transitive dependencies of a module
fn meta_dep_transitive_dependencies(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let module = extract_text_arg(&args, 0)?;
    let graph = DepGraph::from_context(ctx);
    let deps = graph.transitive_dependencies(module.as_str());
    Ok(texts_to_const_array(deps))
}

/// Get modules that depend on this module
fn meta_dep_dependents_of(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let module = extract_text_arg(&args, 0)?;
    let graph = DepGraph::from_context(ctx);
    let dependents = graph.dependents_of(module.as_str());
    Ok(texts_to_const_array(dependents))
}

/// Get maximum depth of module in dependency tree
fn meta_dep_depth(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch {
            expected: 1,
            got: args.len(),
        });
    }

    let module = extract_text_arg(&args, 0)?;
    let graph = DepGraph::from_context(ctx);
    let depth = graph.depth(module.as_str());
    Ok(ConstValue::Int(depth as i128))
}

/// Find all dependency cycles
fn meta_dep_find_cycles(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let graph = DepGraph::from_context(ctx);
    let cycles = graph.find_cycles();

    let cycle_values: List<ConstValue> = cycles
        .into_iter()
        .map(|cycle| texts_to_const_array(cycle))
        .collect();

    Ok(ConstValue::Array(cycle_values))
}

/// Get modules in topological order
fn meta_dep_topological_order(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let graph = DepGraph::from_context(ctx);
    let order = graph.topological_order();
    Ok(texts_to_const_array(order))
}

/// Get optimized compilation order
///
/// Same as topological order but with additional heuristics:
/// - Leaf modules first (they have no dependencies)
/// - Within the same depth level, sort alphabetically for determinism
fn meta_dep_compilation_order(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let graph = DepGraph::from_context(ctx);

    // Group modules by depth, then sort within each group
    let mut depth_groups: HashMap<i64, Vec<String>> = HashMap::new();
    for module in &graph.modules {
        let d = graph.depth(module);
        depth_groups.entry(d).or_default().push(module.clone());
    }

    let mut depths: Vec<i64> = depth_groups.keys().cloned().collect();
    depths.sort();

    let mut result = Vec::new();
    for d in depths {
        if let Some(mut group) = depth_groups.remove(&d) {
            group.sort();
            result.extend(group);
        }
    }

    Ok(texts_to_const_array(result))
}

/// Get leaf modules (no dependencies)
fn meta_dep_leaf_modules(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let graph = DepGraph::from_context(ctx);
    let leaves = graph.leaf_modules();
    Ok(texts_to_const_array(leaves))
}

/// Get root modules (nothing depends on them)
fn meta_dep_root_modules(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let graph = DepGraph::from_context(ctx);
    let roots = graph.root_modules();
    Ok(texts_to_const_array(roots))
}

/// Find strongly connected components using Tarjan's algorithm
fn meta_dep_strongly_connected_components(
    ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    if !args.is_empty() {
        return Err(MetaError::ArityMismatch {
            expected: 0,
            got: args.len(),
        });
    }

    let graph = DepGraph::from_context(ctx);
    let sccs = graph.strongly_connected_components();

    let scc_values: List<ConstValue> = sccs
        .into_iter()
        .map(|scc| texts_to_const_array(scc))
        .collect();

    Ok(ConstValue::Array(scc_values))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::subsystems::code_search::ModuleInfo;

    fn create_test_context() -> MetaContext {
        let mut ctx = MetaContext::new();

        // Build a simple dependency graph:
        //   app -> lib_a -> core
        //   app -> lib_b -> core
        //   lib_b -> lib_a
        let core_info = ModuleInfo::new();
        // core has no dependencies

        let mut lib_a_info = ModuleInfo::new();
        lib_a_info.dependencies.push(Text::from("core"));

        let mut lib_b_info = ModuleInfo::new();
        lib_b_info.dependencies.push(Text::from("core"));
        lib_b_info.dependencies.push(Text::from("lib_a"));

        let mut app_info = ModuleInfo::new();
        app_info.dependencies.push(Text::from("lib_a"));
        app_info.dependencies.push(Text::from("lib_b"));

        ctx.module_registry.insert(Text::from("core"), core_info);
        ctx.module_registry.insert(Text::from("lib_a"), lib_a_info);
        ctx.module_registry.insert(Text::from("lib_b"), lib_b_info);
        ctx.module_registry.insert(Text::from("app"), app_info);

        ctx
    }

    #[test]
    fn test_dependencies_of() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("app"))]);
        let result = meta_dep_dependencies_of(&mut ctx, args).unwrap();
        if let ConstValue::Array(deps) = result {
            assert_eq!(deps.len(), 2);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_dependencies_of_leaf() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("core"))]);
        let result = meta_dep_dependencies_of(&mut ctx, args).unwrap();
        if let ConstValue::Array(deps) = result {
            assert!(deps.is_empty());
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_transitive_dependencies() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("app"))]);
        let result = meta_dep_transitive_dependencies(&mut ctx, args).unwrap();
        if let ConstValue::Array(deps) = result {
            // app -> lib_a, lib_b -> core, lib_a
            // Transitive: core, lib_a, lib_b
            assert_eq!(deps.len(), 3);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_dependents_of() {
        let mut ctx = create_test_context();
        let args = List::from(vec![ConstValue::Text(Text::from("core"))]);
        let result = meta_dep_dependents_of(&mut ctx, args).unwrap();
        if let ConstValue::Array(deps) = result {
            // core is depended on by lib_a and lib_b
            assert_eq!(deps.len(), 2);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_depth() {
        let mut ctx = create_test_context();

        // core has depth 0
        let args = List::from(vec![ConstValue::Text(Text::from("core"))]);
        let result = meta_dep_depth(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(0));

        // lib_a has depth 1 (depends on core)
        let args = List::from(vec![ConstValue::Text(Text::from("lib_a"))]);
        let result = meta_dep_depth(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(1));

        // app has depth 3 (app -> lib_b -> lib_a -> core)
        let args = List::from(vec![ConstValue::Text(Text::from("app"))]);
        let result = meta_dep_depth(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(3));
    }

    #[test]
    fn test_leaf_modules() {
        let mut ctx = create_test_context();
        let result = meta_dep_leaf_modules(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(leaves) = result {
            assert_eq!(leaves.len(), 1);
            assert_eq!(leaves[0], ConstValue::Text(Text::from("core")));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_root_modules() {
        let mut ctx = create_test_context();
        let result = meta_dep_root_modules(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(roots) = result {
            // "app" is the only module nothing depends on
            assert_eq!(roots.len(), 1);
            assert_eq!(roots[0], ConstValue::Text(Text::from("app")));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_topological_order() {
        let mut ctx = create_test_context();
        let result = meta_dep_topological_order(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(order) = result {
            assert_eq!(order.len(), 4);
            // core must come before lib_a and lib_b
            let names: Vec<&str> = order
                .iter()
                .map(|v| match v {
                    ConstValue::Text(t) => t.as_str(),
                    _ => "",
                })
                .collect();
            let core_pos = names.iter().position(|&n| n == "core").unwrap();
            let lib_a_pos = names.iter().position(|&n| n == "lib_a").unwrap();
            let lib_b_pos = names.iter().position(|&n| n == "lib_b").unwrap();
            assert!(core_pos < lib_a_pos);
            assert!(core_pos < lib_b_pos);
            assert!(lib_a_pos < lib_b_pos); // lib_b depends on lib_a
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_cycles_no_cycles() {
        let mut ctx = create_test_context();
        let result = meta_dep_find_cycles(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(cycles) = result {
            assert!(cycles.is_empty(), "Expected no cycles in DAG");
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_find_cycles_with_cycle() {
        let mut ctx = MetaContext::new();

        // Create a cycle: a -> b -> c -> a
        let mut a_info = ModuleInfo::new();
        a_info.dependencies.push(Text::from("b"));
        let mut b_info = ModuleInfo::new();
        b_info.dependencies.push(Text::from("c"));
        let mut c_info = ModuleInfo::new();
        c_info.dependencies.push(Text::from("a"));

        ctx.module_registry.insert(Text::from("a"), a_info);
        ctx.module_registry.insert(Text::from("b"), b_info);
        ctx.module_registry.insert(Text::from("c"), c_info);

        let result = meta_dep_find_cycles(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(cycles) = result {
            assert!(!cycles.is_empty(), "Expected at least one cycle");
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_strongly_connected_components() {
        let mut ctx = create_test_context();
        let result =
            meta_dep_strongly_connected_components(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(sccs) = result {
            // In a DAG, each SCC has exactly one node
            assert_eq!(sccs.len(), 4);
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_compilation_order() {
        let mut ctx = create_test_context();
        let result = meta_dep_compilation_order(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(order) = result {
            assert_eq!(order.len(), 4);
            // core (depth 0) should come first
            assert_eq!(order[0], ConstValue::Text(Text::from("core")));
        } else {
            panic!("Expected Array");
        }
    }

    #[test]
    fn test_empty_graph() {
        let mut ctx = MetaContext::new();

        let result = meta_dep_leaf_modules(&mut ctx, List::new()).unwrap();
        if let ConstValue::Array(leaves) = result {
            assert!(leaves.is_empty());
        } else {
            panic!("Expected Array");
        }
    }
}
