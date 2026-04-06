//! Circular dependency tests.
//!
//! Tests detection and handling of circular dependencies between modules.
//! Type dependencies are allowed, value dependencies cause errors.
//!
//! Circular type dependencies (via references) are allowed. Circular value
//! dependencies (constants) cause compile errors. Function call cycles are
//! allowed (resolved at runtime). Topological sorting determines compilation order.

use std::fs;
use tempfile::TempDir;
use verum_modules::*;

struct TestProject {
    temp_dir: TempDir,
}

impl TestProject {
    fn new() -> Self {
        Self {
            temp_dir: TempDir::new().unwrap(),
        }
    }

    fn create_file(&self, path: &str, content: &str) {
        let full_path = self.temp_dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full_path, content).unwrap();
    }

    fn root_path(&self) -> &std::path::Path {
        self.temp_dir.path()
    }
}

#[test]
fn test_type_dependency_cycle_allowed() {
    // Type dependencies across modules are allowed (e.g., mutual &TypeX references)
    let project = TestProject::new();

    // Circular type dependencies are allowed
    project.create_file(
        "module_a.vr",
        r#"
import crate.module_b.TypeB;

public type TypeA is {
    reference: &TypeB,
}
"#,
    );

    project.create_file(
        "module_b.vr",
        r#"
import crate.module_a.TypeA;

public type TypeB is {
    reference: &TypeA,
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Both modules should load successfully
    let module_a = loader.load_module(&ModulePath::from_str("module_a"), ModuleId::new(1));
    assert!(module_a.is_ok(), "Type dependency cycle should be allowed");

    let module_b = loader.load_module(&ModulePath::from_str("module_b"), ModuleId::new(2));
    assert!(module_b.is_ok(), "Type dependency cycle should be allowed");
}

#[test]
fn test_function_dependency_cycle_allowed() {
    // Function call cycles are allowed (resolved at runtime via mutual recursion)
    let project = TestProject::new();

    // Circular function calls are allowed (resolved at runtime)
    project.create_file(
        "module_a.vr",
        r#"
import crate.module_b.func_b;

public fn func_a(x: Int) -> Int {
    if x == 0 { 0 } else { func_b(x - 1) }
}
"#,
    );

    project.create_file(
        "module_b.vr",
        r#"
import crate.module_a.func_a;

public fn func_b(x: Int) -> Int {
    if x == 0 { 1 } else { func_a(x - 1) }
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    let module_a = loader.load_module(&ModulePath::from_str("module_a"), ModuleId::new(1));
    assert!(
        module_a.is_ok(),
        "Function dependency cycle should be allowed"
    );

    let module_b = loader.load_module(&ModulePath::from_str("module_b"), ModuleId::new(2));
    assert!(
        module_b.is_ok(),
        "Function dependency cycle should be allowed"
    );
}

#[test]
fn test_dependency_graph_cycle_detection() {
    // Test DependencyGraph cycle detection
    let mut graph = DependencyGraph::new();
    let mod1 = ModuleId::new(1);
    let mod2 = ModuleId::new(2);
    let mod3 = ModuleId::new(3);

    graph.add_module(mod1, ModulePath::from_str("cog.a"));
    graph.add_module(mod2, ModulePath::from_str("cog.b"));
    graph.add_module(mod3, ModulePath::from_str("cog.c"));

    // Create cycle: mod1 → mod2 → mod3 → mod1
    graph.add_dependency(mod1, mod2).unwrap();
    graph.add_dependency(mod2, mod3).unwrap();
    graph.add_dependency(mod3, mod1).unwrap();

    // Should detect cycle
    assert!(graph.has_cycles(), "Cycle should be detected");

    let cycle = graph.detect_cycle();
    assert!(cycle.is_some(), "Cycle detection should return cycle path");

    // Topological sort should fail
    let result = graph.topological_order();
    assert!(
        result.is_err(),
        "Topological sort should fail on cyclic graph"
    );
}

#[test]
fn test_dependency_graph_no_cycle() {
    // Test that acyclic graphs are handled correctly
    let mut graph = DependencyGraph::new();
    let mod1 = ModuleId::new(1);
    let mod2 = ModuleId::new(2);
    let mod3 = ModuleId::new(3);

    graph.add_module(mod1, ModulePath::from_str("cog.a"));
    graph.add_module(mod2, ModulePath::from_str("cog.b"));
    graph.add_module(mod3, ModulePath::from_str("cog.c"));

    // Linear dependency: mod1 → mod2 → mod3
    graph.add_dependency(mod1, mod2).unwrap();
    graph.add_dependency(mod2, mod3).unwrap();

    assert!(!graph.has_cycles(), "No cycle should be detected");

    let cycle = graph.detect_cycle();
    assert!(cycle.is_none(), "No cycle should be found");

    let order = graph.topological_order();
    assert!(order.is_ok(), "Topological sort should succeed");
}

#[test]
fn test_self_dependency_detection() {
    // Module depending on itself
    let mut graph = DependencyGraph::new();
    let mod1 = ModuleId::new(1);

    graph.add_module(mod1, ModulePath::from_str("cog.a"));

    // Self-dependency
    let result = graph.add_dependency(mod1, mod1);

    // Should either prevent self-dependency or detect it as a cycle
    if result.is_ok() {
        assert!(
            graph.has_cycles(),
            "Self-dependency should be detected as cycle"
        );
    }
}

#[test]
fn test_diamond_dependency() {
    // Diamond pattern: A depends on B and C, both B and C depend on D
    let mut graph = DependencyGraph::new();
    let mod_a = ModuleId::new(1);
    let mod_b = ModuleId::new(2);
    let mod_c = ModuleId::new(3);
    let mod_d = ModuleId::new(4);

    graph.add_module(mod_a, ModulePath::from_str("cog.a"));
    graph.add_module(mod_b, ModulePath::from_str("cog.b"));
    graph.add_module(mod_c, ModulePath::from_str("cog.c"));
    graph.add_module(mod_d, ModulePath::from_str("cog.d"));

    // Diamond pattern
    graph.add_dependency(mod_a, mod_b).unwrap();
    graph.add_dependency(mod_a, mod_c).unwrap();
    graph.add_dependency(mod_b, mod_d).unwrap();
    graph.add_dependency(mod_c, mod_d).unwrap();

    // Should not have cycles
    assert!(
        !graph.has_cycles(),
        "Diamond dependency should not create cycle"
    );

    let order = graph.topological_order().unwrap();

    // D should come before both B and C, which should come before A
    let d_pos = order.iter().position(|&id| id == mod_d).unwrap();
    let b_pos = order.iter().position(|&id| id == mod_b).unwrap();
    let c_pos = order.iter().position(|&id| id == mod_c).unwrap();
    let a_pos = order.iter().position(|&id| id == mod_a).unwrap();

    assert!(d_pos < b_pos, "D should come before B");
    assert!(d_pos < c_pos, "D should come before C");
    assert!(b_pos < a_pos, "B should come before A");
    assert!(c_pos < a_pos, "C should come before A");
}

#[test]
fn test_complex_cycle_detection() {
    // More complex cycle: A → B → C → D → B (cycle in middle)
    let mut graph = DependencyGraph::new();
    let mod_a = ModuleId::new(1);
    let mod_b = ModuleId::new(2);
    let mod_c = ModuleId::new(3);
    let mod_d = ModuleId::new(4);

    graph.add_module(mod_a, ModulePath::from_str("cog.a"));
    graph.add_module(mod_b, ModulePath::from_str("cog.b"));
    graph.add_module(mod_c, ModulePath::from_str("cog.c"));
    graph.add_module(mod_d, ModulePath::from_str("cog.d"));

    graph.add_dependency(mod_a, mod_b).unwrap();
    graph.add_dependency(mod_b, mod_c).unwrap();
    graph.add_dependency(mod_c, mod_d).unwrap();
    graph.add_dependency(mod_d, mod_b).unwrap(); // Creates cycle

    assert!(graph.has_cycles(), "Cycle in middle should be detected");
}

#[test]
fn test_topological_sort_order() {
    // Topological sorting: dependencies compiled before dependents, cycles in value deps cause error
    let mut graph = DependencyGraph::new();
    let main = ModuleId::new(1);
    let network = ModuleId::new(2);
    let parser = ModuleId::new(3);
    let protocol = ModuleId::new(4);

    graph.add_module(main, ModulePath::from_str("cog.main"));
    graph.add_module(network, ModulePath::from_str("cog.network"));
    graph.add_module(parser, ModulePath::from_str("cog.parser"));
    graph.add_module(protocol, ModulePath::from_str("cog.protocol"));

    // Dependencies:
    // main → network → protocol
    // main → parser → protocol
    // main → protocol
    graph.add_dependency(main, network).unwrap();
    graph.add_dependency(main, parser).unwrap();
    graph.add_dependency(main, protocol).unwrap();
    graph.add_dependency(network, protocol).unwrap();
    graph.add_dependency(parser, protocol).unwrap();

    let order = graph.topological_order().unwrap();

    // Protocol should come first, main should come last
    let protocol_pos = order.iter().position(|&id| id == protocol).unwrap();
    let network_pos = order.iter().position(|&id| id == network).unwrap();
    let parser_pos = order.iter().position(|&id| id == parser).unwrap();
    let main_pos = order.iter().position(|&id| id == main).unwrap();

    assert!(protocol_pos < network_pos, "protocol before network");
    assert!(protocol_pos < parser_pos, "protocol before parser");
    assert!(protocol_pos < main_pos, "protocol before main");
    assert!(network_pos < main_pos, "network before main");
    assert!(parser_pos < main_pos, "parser before main");
}

#[test]
fn test_multiple_independent_modules() {
    // Test graph with multiple independent modules (no dependencies)
    let mut graph = DependencyGraph::new();
    let mod1 = ModuleId::new(1);
    let mod2 = ModuleId::new(2);
    let mod3 = ModuleId::new(3);

    graph.add_module(mod1, ModulePath::from_str("cog.a"));
    graph.add_module(mod2, ModulePath::from_str("cog.b"));
    graph.add_module(mod3, ModulePath::from_str("cog.c"));

    // No dependencies

    assert!(!graph.has_cycles());
    let order = graph.topological_order();
    assert!(order.is_ok());
    assert_eq!(order.unwrap().len(), 3);
}

#[test]
fn test_long_dependency_chain() {
    // Test long linear dependency chain
    let mut graph = DependencyGraph::new();
    let count = 10;
    let modules: Vec<ModuleId> = (0..count).map(|i| ModuleId::new(i)).collect();

    for (i, &id) in modules.iter().enumerate() {
        graph.add_module(id, ModulePath::from_str(&format!("cog.mod{}", i)));
    }

    // Create linear chain: mod0 → mod1 → mod2 → ... → mod9
    for i in 0..count - 1 {
        graph
            .add_dependency(modules[i as usize], modules[(i + 1) as usize])
            .unwrap();
    }

    assert!(!graph.has_cycles());
    let order = graph.topological_order().unwrap();
    assert_eq!(order.len(), count as usize);

    // Verify order: mod9 should come first, mod0 should come last
    let first_pos = order.iter().position(|&id| id == modules[9]).unwrap();
    let last_pos = order.iter().position(|&id| id == modules[0]).unwrap();
    assert!(first_pos < last_pos);
}

#[test]
fn test_mutual_type_dependency() {
    // Two types that reference each other
    let project = TestProject::new();

    project.create_file(
        "node.vr",
        r#"
import crate.tree.Tree;

public type Node is {
    value: Int,
    tree: &Tree,
}
"#,
    );

    project.create_file(
        "tree.vr",
        r#"
import crate.node.Node;

public type Tree is {
    root: &Node,
}
"#,
    );

    let mut loader = ModuleLoader::new(project.root_path());

    // Both should load (type cycles are allowed)
    let node = loader.load_module(&ModulePath::from_str("node"), ModuleId::new(1));
    assert!(node.is_ok());

    let tree = loader.load_module(&ModulePath::from_str("tree"), ModuleId::new(2));
    assert!(tree.is_ok());
}

#[test]
fn test_three_way_cycle() {
    // A → B → C → A
    let mut graph = DependencyGraph::new();
    let mod_a = ModuleId::new(1);
    let mod_b = ModuleId::new(2);
    let mod_c = ModuleId::new(3);

    graph.add_module(mod_a, ModulePath::from_str("cog.a"));
    graph.add_module(mod_b, ModulePath::from_str("cog.b"));
    graph.add_module(mod_c, ModulePath::from_str("cog.c"));

    graph.add_dependency(mod_a, mod_b).unwrap();
    graph.add_dependency(mod_b, mod_c).unwrap();
    graph.add_dependency(mod_c, mod_a).unwrap();

    assert!(graph.has_cycles());

    let cycle = graph.detect_cycle();
    assert!(cycle.is_some());

    // The cycle should contain all three modules
    if let Some(cycle_path) = cycle {
        assert!(
            cycle_path.len() >= 3,
            "Cycle should contain at least 3 modules"
        );
    }
}
