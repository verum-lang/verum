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
//! Comprehensive tests for points-to analysis
//!
//! This test suite validates the Andersen-style points-to analysis implementation
//! with over 20 comprehensive test cases covering all functionality.

use verum_cbgr::analysis::{BasicBlock, BlockId, ControlFlowGraph, DefSite, RefId, UseeSite};
use verum_cbgr::points_to_analysis::*;
use verum_common::{List, Map, Maybe, Set};

// ==================================================================================
// Test Utilities
// ==================================================================================

/// Create a simple CFG for testing
fn create_test_cfg() -> ControlFlowGraph {
    let entry = BlockId(0);
    let exit = BlockId(1);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    // Entry block
    let entry_block = BasicBlock {
        id: entry,
        predecessors: Set::new(),
        successors: {
            let mut s = Set::new();
            s.insert(exit);
            s
        },
        definitions: vec![DefSite {
            block: entry,
            reference: RefId(1),
            is_stack_allocated: true, span: None,
        }].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    // Exit block
    let exit_block = BasicBlock {
        id: exit,
        predecessors: {
            let mut p = Set::new();
            p.insert(entry);
            p
        },
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![UseeSite {
            block: exit,
            reference: RefId(1),
            is_mutable: false, span: None,
        }].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    cfg.add_block(entry_block);
    cfg.add_block(exit_block);

    cfg
}

// ==================================================================================
// Section 1: PointsToSet Tests (5 tests)
// ==================================================================================

#[test]
fn test_points_to_set_empty() {
    let pts = PointsToSet::new(VarId(1));
    assert!(pts.is_empty());
    assert_eq!(pts.variable, VarId(1));
    assert_eq!(pts.size(), Maybe::Some(0));
}

#[test]
fn test_points_to_set_add_location() {
    let mut pts = PointsToSet::new(VarId(1));

    // Add first location
    assert!(pts.add_location(LocationId(42)));
    assert!(pts.may_point_to(LocationId(42)));
    assert_eq!(pts.size(), Maybe::Some(1));

    // Add same location (no change)
    assert!(!pts.add_location(LocationId(42)));
    assert_eq!(pts.size(), Maybe::Some(1));

    // Add different location
    assert!(pts.add_location(LocationId(99)));
    assert_eq!(pts.size(), Maybe::Some(2));
}

#[test]
fn test_points_to_set_conservative() {
    let mut pts = PointsToSet::conservative(VarId(1));

    assert!(pts.conservative);
    assert_eq!(pts.size(), Maybe::None);

    // Adding locations has no effect
    assert!(!pts.add_location(LocationId(42)));

    // May point to anything
    assert!(pts.may_point_to(LocationId(42)));
    assert!(pts.may_point_to(LocationId(999)));
    assert!(pts.may_point_to(LocationId(0)));
}

#[test]
fn test_points_to_set_union() {
    let mut pts1 = PointsToSet::new(VarId(1));
    pts1.add_location(LocationId(1));
    pts1.add_location(LocationId(2));

    let mut pts2 = PointsToSet::new(VarId(2));
    pts2.add_location(LocationId(2));
    pts2.add_location(LocationId(3));

    // Union pts2 into pts1
    assert!(pts1.add_all(&pts2));
    assert_eq!(pts1.size(), Maybe::Some(3));
    assert!(pts1.may_point_to(LocationId(1)));
    assert!(pts1.may_point_to(LocationId(2)));
    assert!(pts1.may_point_to(LocationId(3)));

    // Union again (no change)
    assert!(!pts1.add_all(&pts2));
}

#[test]
fn test_points_to_set_conservative_union() {
    let mut pts1 = PointsToSet::new(VarId(1));
    pts1.add_location(LocationId(1));

    let pts2 = PointsToSet::conservative(VarId(2));

    // Union conservative set makes pts1 conservative
    assert!(pts1.add_all(&pts2));
    assert!(pts1.conservative);
    assert_eq!(pts1.size(), Maybe::None);
}

// ==================================================================================
// Section 2: PointsToGraph Tests (5 tests)
// ==================================================================================

#[test]
fn test_points_to_graph_basic() {
    let mut graph = PointsToGraph::new();

    let var1 = VarId(1);
    let loc1 = LocationId(10);

    // Add points-to relationship
    assert!(graph.add_points_to(var1, loc1));

    // Verify
    let pts = graph.get_points_to_set(var1).unwrap();
    assert!(pts.may_point_to(loc1));
}

#[test]
fn test_points_to_graph_location_types() {
    let mut graph = PointsToGraph::new();

    let loc1 = LocationId(1);
    let loc2 = LocationId(2);
    let loc3 = LocationId(3);

    graph.set_location_type(loc1, LocationType::Stack);
    graph.set_location_type(loc2, LocationType::Heap);
    graph.set_location_type(loc3, LocationType::Global);

    assert_eq!(graph.get_location_type(loc1), LocationType::Stack);
    assert_eq!(graph.get_location_type(loc2), LocationType::Heap);
    assert_eq!(graph.get_location_type(loc3), LocationType::Global);

    // Unknown location
    assert_eq!(
        graph.get_location_type(LocationId(999)),
        LocationType::Unknown
    );
}

#[test]
fn test_points_to_graph_may_alias() {
    let mut graph = PointsToGraph::new();

    let var1 = VarId(1);
    let var2 = VarId(2);
    let var3 = VarId(3);
    let loc1 = LocationId(10);
    let loc2 = LocationId(20);

    // var1 and var2 point to same location
    graph.add_points_to(var1, loc1);
    graph.add_points_to(var2, loc1);

    // var3 points to different location
    graph.add_points_to(var3, loc2);

    // var1 and var2 may alias (share loc1)
    assert!(graph.may_alias(var1, var2));

    // var1 and var3 don't alias
    assert!(!graph.may_alias(var1, var3));
}

#[test]
fn test_points_to_graph_must_alias() {
    let mut graph = PointsToGraph::new();

    let var1 = VarId(1);
    let var2 = VarId(2);
    let var3 = VarId(3);
    let loc1 = LocationId(10);

    // var1 and var2 point to exactly one location each (same)
    graph.add_points_to(var1, loc1);
    graph.add_points_to(var2, loc1);

    // Must alias (both point to single location)
    assert!(graph.must_alias(var1, var2));

    // Add another location to var2
    graph.add_points_to(var2, LocationId(20));

    // No longer must-alias (var2 points to multiple locations)
    assert!(!graph.must_alias(var1, var2));
}

#[test]
fn test_points_to_graph_points_to_heap() {
    let mut graph = PointsToGraph::new();

    let var1 = VarId(1);
    let var2 = VarId(2);
    let stack_loc = LocationId(10);
    let heap_loc = LocationId(20);

    graph.set_location_type(stack_loc, LocationType::Stack);
    graph.set_location_type(heap_loc, LocationType::Heap);

    // var1 points to stack
    graph.add_points_to(var1, stack_loc);
    assert!(!graph.points_to_heap(var1));

    // var2 points to heap
    graph.add_points_to(var2, heap_loc);
    assert!(graph.points_to_heap(var2));

    // var1 also points to heap
    graph.add_points_to(var1, heap_loc);
    assert!(graph.points_to_heap(var1));
}

// ==================================================================================
// Section 3: Constraint Tests (4 tests)
// ==================================================================================

#[test]
fn test_constraint_address_of() {
    let constraint = PointsToConstraint::AddressOf {
        variable: VarId(1),
        location: LocationId(42),
    };

    let vars = constraint.referenced_variables();
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0], VarId(1));
}

#[test]
fn test_constraint_copy() {
    let constraint = PointsToConstraint::Copy {
        dest: VarId(1),
        src: VarId(2),
    };

    let vars = constraint.referenced_variables();
    assert_eq!(vars.len(), 2);
    assert!(vars.contains(&VarId(1)));
    assert!(vars.contains(&VarId(2)));
}

#[test]
fn test_constraint_load() {
    let constraint = PointsToConstraint::Load {
        dest: VarId(1),
        ptr: VarId(2),
    };

    let vars = constraint.referenced_variables();
    assert_eq!(vars.len(), 2);
    assert!(vars.contains(&VarId(1)));
    assert!(vars.contains(&VarId(2)));
}

#[test]
fn test_constraint_store() {
    let constraint = PointsToConstraint::Store {
        ptr: VarId(1),
        value: VarId(2),
    };

    let vars = constraint.referenced_variables();
    assert_eq!(vars.len(), 2);
    assert!(vars.contains(&VarId(1)));
    assert!(vars.contains(&VarId(2)));
}

// ==================================================================================
// Section 4: Analyzer Tests (7 tests)
// ==================================================================================

#[test]
fn test_analyzer_allocate_ids() {
    let mut analyzer = PointsToAnalyzer::new();

    let loc1 = analyzer.allocate_location();
    let loc2 = analyzer.allocate_location();
    assert_ne!(loc1, loc2);

    let var1 = analyzer.allocate_variable();
    let var2 = analyzer.allocate_variable();
    assert_ne!(var1, var2);
}

#[test]
fn test_analyzer_add_constraint() {
    let mut analyzer = PointsToAnalyzer::new();

    let var = analyzer.allocate_variable();
    let loc = analyzer.allocate_location();

    analyzer.add_constraint(PointsToConstraint::AddressOf {
        variable: var,
        location: loc,
    });

    // Constraint should be added
    assert_eq!(analyzer.constraint_count(), 1);
}

#[test]
fn test_analyzer_solve_address_of() {
    let mut analyzer = PointsToAnalyzer::new();

    let var = analyzer.allocate_variable();
    let loc = analyzer.allocate_location();

    analyzer.add_constraint(PointsToConstraint::AddressOf {
        variable: var,
        location: loc,
    });

    let result = analyzer.solve();

    assert!(result.converged);
    assert!(result.iterations > 0);

    let graph = analyzer.get_graph();
    let pts = graph.get_points_to_set(var).unwrap();
    assert!(pts.may_point_to(loc));
}

#[test]
fn test_analyzer_solve_copy() {
    let mut analyzer = PointsToAnalyzer::new();

    let var1 = analyzer.allocate_variable();
    let var2 = analyzer.allocate_variable();
    let loc = analyzer.allocate_location();

    // var1 = &loc
    analyzer.add_constraint(PointsToConstraint::AddressOf {
        variable: var1,
        location: loc,
    });

    // var2 = var1
    analyzer.add_constraint(PointsToConstraint::Copy {
        dest: var2,
        src: var1,
    });

    let result = analyzer.solve();
    assert!(result.converged);

    let graph = analyzer.get_graph();

    // Both vars point to loc
    assert!(graph.get_points_to_set(var1).unwrap().may_point_to(loc));
    assert!(graph.get_points_to_set(var2).unwrap().may_point_to(loc));
}

#[test]
fn test_analyzer_solve_chain() {
    let mut analyzer = PointsToAnalyzer::new();

    let var1 = analyzer.allocate_variable();
    let var2 = analyzer.allocate_variable();
    let var3 = analyzer.allocate_variable();
    let loc = analyzer.allocate_location();

    // var1 = &loc
    analyzer.add_constraint(PointsToConstraint::AddressOf {
        variable: var1,
        location: loc,
    });

    // var2 = var1
    analyzer.add_constraint(PointsToConstraint::Copy {
        dest: var2,
        src: var1,
    });

    // var3 = var2
    analyzer.add_constraint(PointsToConstraint::Copy {
        dest: var3,
        src: var2,
    });

    let result = analyzer.solve();
    assert!(result.converged);

    let graph = analyzer.get_graph();

    // All vars point to loc
    assert!(graph.get_points_to_set(var1).unwrap().may_point_to(loc));
    assert!(graph.get_points_to_set(var2).unwrap().may_point_to(loc));
    assert!(graph.get_points_to_set(var3).unwrap().may_point_to(loc));
}

#[test]
fn test_analyzer_multiple_locations() {
    let mut analyzer = PointsToAnalyzer::new();

    let var = analyzer.allocate_variable();
    let loc1 = analyzer.allocate_location();
    let loc2 = analyzer.allocate_location();

    // var points to both locations
    analyzer.add_constraint(PointsToConstraint::AddressOf {
        variable: var,
        location: loc1,
    });
    analyzer.add_constraint(PointsToConstraint::AddressOf {
        variable: var,
        location: loc2,
    });

    let result = analyzer.solve();
    assert!(result.converged);

    let graph = analyzer.get_graph();
    let pts = graph.get_points_to_set(var).unwrap();

    assert!(pts.may_point_to(loc1));
    assert!(pts.may_point_to(loc2));
    assert_eq!(pts.size(), Maybe::Some(2));
}

#[test]
fn test_analyzer_statistics() {
    let mut analyzer = PointsToAnalyzer::new();

    let var1 = analyzer.allocate_variable();
    let var2 = analyzer.allocate_variable();
    let loc = analyzer.allocate_location();

    analyzer.add_constraint(PointsToConstraint::AddressOf {
        variable: var1,
        location: loc,
    });
    analyzer.add_constraint(PointsToConstraint::Copy {
        dest: var2,
        src: var1,
    });

    analyzer.solve();

    let stats = analyzer.get_statistics();
    assert_eq!(stats.total_variables, 2);
    assert_eq!(stats.total_locations, 1);
    assert_eq!(stats.total_constraints, 2);
    assert!(stats.iterations > 0);
}

// ==================================================================================
// Section 5: CFG Integration Tests (4 tests)
// ==================================================================================

#[test]
fn test_generate_constraints_from_cfg() {
    let cfg = create_test_cfg();
    let mut analyzer = PointsToAnalyzer::new();

    let result = analyzer.generate_constraints_from_cfg(&cfg);

    assert!(result.stats.total_constraints > 0);
    assert!(result.variables > 0);
    assert!(result.locations > 0);
}

#[test]
fn test_builder_with_cfg() {
    let cfg = create_test_cfg();

    let result = PointsToAnalyzerBuilder::new().with_cfg(&cfg).build();

    assert!(result.solve_result.converged);
    assert!(result.stats.total_constraints > 0);
}

#[test]
fn test_cfg_stack_allocation() {
    let cfg = create_test_cfg();
    let mut analyzer = PointsToAnalyzer::new();

    analyzer.generate_constraints_from_cfg(&cfg);
    analyzer.solve();

    let graph = analyzer.get_graph();

    // RefId(1) is stack-allocated in the test CFG
    let var = graph.get_var_for_ref(RefId(1));
    assert!(var.is_some());

    // Check if variable has points-to set
    let pts = graph.get_points_to_set(var.unwrap());
    assert!(pts.is_some());
}

#[test]
fn test_cfg_heap_allocation() {
    let entry = BlockId(0);
    let exit = BlockId(1);

    let mut cfg = ControlFlowGraph::new(entry, exit);

    // Entry block with heap allocation
    let entry_block = BasicBlock {
        id: entry,
        predecessors: Set::new(),
        successors: {
            let mut s = Set::new();
            s.insert(exit);
            s
        },
        definitions: vec![DefSite {
            block: entry,
            reference: RefId(1),
            is_stack_allocated: false, // HEAP!
            span: None,
        }].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    let exit_block = BasicBlock {
        id: exit,
        predecessors: {
            let mut p = Set::new();
            p.insert(entry);
            p
        },
        successors: Set::new(),
        definitions: vec![].into(),
        uses: vec![].into(),
        call_sites: vec![].into(),
        has_await_point: false,
        is_exception_handler: false,
        is_cleanup_handler: false,
        may_throw: false,
    };

    cfg.add_block(entry_block);
    cfg.add_block(exit_block);

    let mut analyzer = PointsToAnalyzer::new();
    analyzer.generate_constraints_from_cfg(&cfg);
    analyzer.solve();

    let graph = analyzer.get_graph();

    // Find the location and check it's marked as heap
    // Since we have one allocation, we should have one location
    if let Maybe::Some(var) = graph.get_var_for_ref(RefId(1))
        && let Maybe::Some(pts) = graph.get_points_to_set(var) {
            // Check that at least one location is heap
            let has_heap = pts
                .locations
                .iter()
                .any(|&loc| graph.get_location_type(loc).is_heap());
            assert!(has_heap || pts.conservative);
        }
}

// ==================================================================================
// Section 6: Integration Helper Tests (2 tests)
// ==================================================================================

#[test]
fn test_points_to_graph_to_alias_sets() {
    let mut graph = PointsToGraph::new();

    let ref_id = RefId(42);
    let var = VarId(1);
    let loc1 = LocationId(10);
    let loc2 = LocationId(20);

    graph.map_var_to_ref(var, ref_id);
    graph.add_points_to(var, loc1);
    graph.add_points_to(var, loc2);

    let alias_sets = points_to_graph_to_alias_sets(&graph, ref_id);
    assert!(alias_sets.is_some());

    let sets = alias_sets.unwrap();
    assert_eq!(sets.reference, ref_id);
    assert_eq!(sets.ssa_versions.len(), 2);
}

#[test]
fn test_reference_points_to_heap_function() {
    let mut graph = PointsToGraph::new();

    let ref_id = RefId(42);
    let var = VarId(1);
    let heap_loc = LocationId(10);

    graph.map_var_to_ref(var, ref_id);
    graph.set_location_type(heap_loc, LocationType::Heap);
    graph.add_points_to(var, heap_loc);

    assert!(reference_points_to_heap(&graph, ref_id));
}

// ==================================================================================
// Section 7: Edge Cases and Stress Tests (3 tests)
// ==================================================================================

#[test]
fn test_empty_analyzer() {
    let analyzer = PointsToAnalyzer::new();
    let stats = analyzer.get_statistics();

    assert_eq!(stats.total_variables, 0);
    assert_eq!(stats.total_locations, 0);
    assert_eq!(stats.total_constraints, 0);
}

#[test]
fn test_large_points_to_set() {
    let mut pts = PointsToSet::new(VarId(1));

    // Add 1000 locations
    for i in 0..1000 {
        pts.add_location(LocationId(i));
    }

    assert_eq!(pts.size(), Maybe::Some(1000));

    for i in 0..1000 {
        assert!(pts.may_point_to(LocationId(i)));
    }
}

#[test]
fn test_complex_constraint_graph() {
    let mut analyzer = PointsToAnalyzer::new();

    // Create a complex graph with 10 variables and 5 locations
    let mut vars = vec![];
    let mut locs = vec![];

    for _ in 0..10 {
        vars.push(analyzer.allocate_variable());
    }

    for _ in 0..5 {
        locs.push(analyzer.allocate_location());
    }

    // Create address-of constraints
    for i in 0..5 {
        analyzer.add_constraint(PointsToConstraint::AddressOf {
            variable: vars[i],
            location: locs[i],
        });
    }

    // Create copy constraints (chain)
    for i in 0..9 {
        analyzer.add_constraint(PointsToConstraint::Copy {
            dest: vars[i + 1],
            src: vars[i],
        });
    }

    let result = analyzer.solve();
    assert!(result.converged);
    assert!(result.iterations < 100); // Should converge quickly

    let stats = analyzer.get_statistics();
    assert_eq!(stats.total_variables, 10);
    assert_eq!(stats.total_locations, 5);
}
