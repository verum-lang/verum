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
// Tests for boundary module
// Migrated from src/boundary.rs per CLAUDE.md standards

use verum_common::{List, Text};
use verum_verification::VerificationLevel;
use verum_verification::boundary::*;

#[test]
fn test_function_id() {
    let id1 = FunctionId::new(0);
    let id2 = FunctionId::new(1);
    assert_ne!(id1, id2);
    assert_eq!(id1.as_u64(), 0);
    assert_eq!(format!("{}", id1), "fn_0");
}

#[test]
fn test_boundary_direction() {
    assert_eq!(
        BoundaryDirection::from_levels(VerificationLevel::Runtime, VerificationLevel::Static),
        BoundaryDirection::MoreRestrictive
    );
    assert_eq!(
        BoundaryDirection::from_levels(VerificationLevel::Proof, VerificationLevel::Runtime),
        BoundaryDirection::LessRestrictive
    );
    assert_eq!(
        BoundaryDirection::from_levels(VerificationLevel::Static, VerificationLevel::Static),
        BoundaryDirection::Same
    );
}

#[test]
fn test_call_graph_node() {
    let mut node = CallGraphNode::new(
        FunctionId::new(0),
        Text::from("test_func"),
        VerificationLevel::Runtime,
        SourceLocation::default(),
    );

    node.add_caller(FunctionId::new(1));
    node.add_callee(FunctionId::new(2));

    assert_eq!(node.callers.len(), 1);
    assert_eq!(node.callees.len(), 1);

    // Adding duplicate should not increase count
    node.add_caller(FunctionId::new(1));
    assert_eq!(node.callers.len(), 1);
}

#[test]
fn test_call_graph_empty() {
    let graph = CallGraph::new();
    assert_eq!(graph.stats().total_functions, 0);
    assert_eq!(graph.stats().total_calls, 0);
}

#[test]
fn test_call_edge() {
    let edge = CallEdge::new(
        FunctionId::new(0),
        FunctionId::new(1),
        SourceLocation::default(),
    );

    assert!(!edge.crosses_boundary());
    assert!(!edge.is_recursive);
}

#[test]
fn test_obligation_generator() {
    let mut generator = ObligationGenerator::new();
    let mut boundary = DetectedBoundary {
        caller_id: FunctionId::new(0),
        callee_id: FunctionId::new(1),
        caller_name: Text::from("caller"),
        callee_name: Text::from("callee"),
        caller_level: VerificationLevel::Runtime,
        callee_level: VerificationLevel::Proof,
        call_site: SourceLocation::default(),
        boundary_kind: BoundaryKind::FunctionCall,
        direction: BoundaryDirection::MoreRestrictive,
        required_obligations: List::new(),
    };

    generator.generate_obligations(&mut boundary);
    assert!(!boundary.required_obligations.is_empty());
}

#[test]
fn test_detected_boundary_checks() {
    let boundary_more = DetectedBoundary {
        caller_id: FunctionId::new(0),
        callee_id: FunctionId::new(1),
        caller_name: Text::from("caller"),
        callee_name: Text::from("callee"),
        caller_level: VerificationLevel::Runtime,
        callee_level: VerificationLevel::Static,
        call_site: SourceLocation::default(),
        boundary_kind: BoundaryKind::FunctionCall,
        direction: BoundaryDirection::MoreRestrictive,
        required_obligations: List::new(),
    };

    assert!(boundary_more.requires_proof_obligations());
    assert!(!boundary_more.requires_runtime_checks());

    let boundary_less = DetectedBoundary {
        caller_id: FunctionId::new(0),
        callee_id: FunctionId::new(1),
        caller_name: Text::from("caller"),
        callee_name: Text::from("callee"),
        caller_level: VerificationLevel::Static,
        callee_level: VerificationLevel::Runtime,
        call_site: SourceLocation::default(),
        boundary_kind: BoundaryKind::FunctionCall,
        direction: BoundaryDirection::LessRestrictive,
        required_obligations: List::new(),
    };

    assert!(!boundary_less.requires_proof_obligations());
    assert!(boundary_less.requires_runtime_checks());
}
