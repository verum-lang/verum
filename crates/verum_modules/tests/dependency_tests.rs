#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
// Tests for dependency module
// Migrated from src/dependency.rs per CLAUDE.md standards

use verum_modules::dependency::*;
use verum_modules::{ModuleId, ModulePath};

#[test]
fn test_dependency_graph_basic() {
    let mut graph = DependencyGraph::new();
    let mod1 = ModuleId::new(1);
    let mod2 = ModuleId::new(2);
    let mod3 = ModuleId::new(3);

    graph.add_module(mod1, ModulePath::from_str("cog.a"));
    graph.add_module(mod2, ModulePath::from_str("cog.b"));
    graph.add_module(mod3, ModulePath::from_str("cog.c"));

    // mod1 depends on mod2, mod2 depends on mod3
    graph.add_dependency(mod1, mod2).unwrap();
    graph.add_dependency(mod2, mod3).unwrap();

    assert!(!graph.has_cycles());
    assert_eq!(graph.len(), 3);
}

#[test]
fn test_topological_order() {
    let mut graph = DependencyGraph::new();
    let mod1 = ModuleId::new(1);
    let mod2 = ModuleId::new(2);
    let mod3 = ModuleId::new(3);

    graph.add_module(mod1, ModulePath::from_str("cog.a"));
    graph.add_module(mod2, ModulePath::from_str("cog.b"));
    graph.add_module(mod3, ModulePath::from_str("cog.c"));

    // mod1 → mod2 → mod3
    graph.add_dependency(mod1, mod2).unwrap();
    graph.add_dependency(mod2, mod3).unwrap();

    let order = graph.topological_order().unwrap();

    // mod1 → mod2 → mod3 means mod1 depends on mod2, mod2 depends on mod3
    // In topological order with current implementation, mod1 comes before mod2, mod2 before mod3
    let mod3_pos = order.iter().position(|&id| id == mod3).unwrap();
    let mod2_pos = order.iter().position(|&id| id == mod2).unwrap();
    let mod1_pos = order.iter().position(|&id| id == mod1).unwrap();

    assert!(mod1_pos < mod2_pos);
    assert!(mod2_pos < mod3_pos);
}

#[test]
fn test_circular_dependency_detection() {
    let mut graph = DependencyGraph::new();
    let mod1 = ModuleId::new(1);
    let mod2 = ModuleId::new(2);

    graph.add_module(mod1, ModulePath::from_str("cog.a"));
    graph.add_module(mod2, ModulePath::from_str("cog.b"));

    // Create cycle: mod1 → mod2 → mod1
    graph.add_dependency(mod1, mod2).unwrap();
    graph.add_dependency(mod2, mod1).unwrap();

    assert!(graph.has_cycles());
    let cycle = graph.detect_cycle();
    assert!(cycle.is_some());
}

#[test]
fn test_dependencies_of() {
    let mut graph = DependencyGraph::new();
    let mod1 = ModuleId::new(1);
    let mod2 = ModuleId::new(2);
    let mod3 = ModuleId::new(3);

    graph.add_module(mod1, ModulePath::from_str("cog.a"));
    graph.add_module(mod2, ModulePath::from_str("cog.b"));
    graph.add_module(mod3, ModulePath::from_str("cog.c"));

    graph.add_dependency(mod1, mod2).unwrap();
    graph.add_dependency(mod1, mod3).unwrap();

    let deps = graph.dependencies_of(mod1);
    assert_eq!(deps.len(), 2);
    assert!(deps.contains(&mod2));
    assert!(deps.contains(&mod3));
}
