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
//! Comprehensive test suite for verum_protocol_types
//!
//! Tests cover:
//! - Protocol type construction
//! - Type equality and comparison
//! - Associated types and GATs
//! - CBGR predicates
//! - Specialization lattice
//! - Serialization (serde)
//! - Display/Debug implementations
//! - Helper methods

use std::time::Duration;
use verum_ast::{
    span::Span,
    ty::{Ident, Path, PathSegment, Type},
};
use verum_common::{List, Map, Maybe, Set, Text};
use verum_protocol_types::cbgr_predicates::ComparisonOp;
use verum_protocol_types::specialization::{SpecializationCondition, SpecializationNode};
use verum_protocol_types::*;

// Helper functions for creating test types
fn make_int_type() -> Type {
    Type::int(Span::default())
}

fn make_bool_type() -> Type {
    Type::bool(Span::default())
}

fn make_text_type() -> Type {
    Type::text(Span::default())
}

fn make_path(names: &[&str]) -> Path {
    let segments: Vec<PathSegment> = names
        .iter()
        .map(|name| PathSegment::Name(Ident::new(*name, Span::default())))
        .collect();
    Path {
        segments: segments.into_iter().collect(),
        span: Span::default(),
    }
}

// ==================== Protocol Base Tests ====================

#[test]
fn test_associated_type_construction() {
    let bounds = List::new();
    let assoc_type = AssociatedType::new(Text::from("Item"), bounds.clone());

    assert_eq!(assoc_type.name, "Item");
    assert_eq!(assoc_type.bounds.len(), 0);
    assert_eq!(assoc_type.default, None);
}

#[test]
fn test_associated_type_with_default() {
    let bounds = List::new();
    let default_type = make_int_type();
    let assoc_type = AssociatedType::with_default(Text::from("Item"), bounds, default_type.clone());

    assert_eq!(assoc_type.name, "Item");
    assert_eq!(assoc_type.default, Some(default_type));
}

#[test]
fn test_object_safety_error_display() {
    let error = ObjectSafetyError::ReturnsSelf {
        method_name: Text::from("clone"),
    };
    let display = format!("{}", error);
    assert!(display.contains("clone"));
    assert!(display.contains("returns Self"));

    let error2 = ObjectSafetyError::GenericMethod {
        method_name: Text::from("map"),
    };
    let display2 = format!("{}", error2);
    assert!(display2.contains("map"));
    assert!(display2.contains("generic"));

    let error3 = ObjectSafetyError::RequiresSized;
    let display3 = format!("{}", error3);
    assert!(display3.contains("Sized"));
}

#[test]
fn test_protocol_bound_equality() {
    let path1 = make_path(&["std", "Eq"]);
    let path2 = make_path(&["std", "Eq"]);

    let bound1 = ProtocolBound {
        protocol: path1,
        args: List::new(),
        is_negative: false,
    };
    let bound2 = ProtocolBound {
        protocol: path2,
        args: List::new(),
        is_negative: false,
    };

    assert_eq!(bound1, bound2);
}

#[test]
fn test_const_value_variants() {
    let int_val = ConstValue::Int(42);
    let bool_val = ConstValue::Bool(true);
    let text_val = ConstValue::Text(Text::from("hello"));

    match int_val {
        ConstValue::Int(v) => assert_eq!(v, 42),
        _ => panic!("Expected Int variant"),
    }

    match bool_val {
        ConstValue::Bool(v) => assert!(v),
        _ => panic!("Expected Bool variant"),
    }

    match text_val {
        ConstValue::Text(ref v) => assert_eq!(v.as_str(), "hello"),
        _ => panic!("Expected Text variant"),
    }
}

#[test]
fn test_method_source_variants() {
    let explicit = MethodSource::Explicit;
    let default = MethodSource::Default(Text::from("Display"));
    let inherited = MethodSource::Inherited(Text::from("Debug"));

    match explicit {
        MethodSource::Explicit => {}
        _ => panic!("Expected Explicit variant"),
    }

    match default {
        MethodSource::Default(ref name) => assert_eq!(name.as_str(), "Display"),
        _ => panic!("Expected Default variant"),
    }

    match inherited {
        MethodSource::Inherited(ref name) => assert_eq!(name.as_str(), "Debug"),
        _ => panic!("Expected Inherited variant"),
    }
}

// ==================== GAT Types Tests ====================

#[test]
fn test_variance_equality() {
    assert_eq!(Variance::Covariant, Variance::Covariant);
    assert_eq!(Variance::Contravariant, Variance::Contravariant);
    assert_eq!(Variance::Invariant, Variance::Invariant);
    assert_ne!(Variance::Covariant, Variance::Invariant);
}

#[test]
fn test_kind_constructor() {
    // * (regular type)
    let star = Kind::constructor(0);
    assert_eq!(star, Kind::Star);
    assert_eq!(star.arity(), 0);

    // * -> * (type constructor with 1 arg)
    let unary = Kind::constructor(1);
    assert_eq!(unary.arity(), 1);

    // * -> * -> * (type constructor with 2 args)
    let binary = Kind::constructor(2);
    assert_eq!(binary.arity(), 2);
}

#[test]
fn test_kind_arity() {
    let star = Kind::Star;
    assert_eq!(star.arity(), 0);

    let arrow = Kind::Arrow {
        from: Box::new(Kind::Star),
        to: Box::new(Kind::Star),
    };
    assert_eq!(arrow.arity(), 1);
}

#[test]
fn test_associated_type_gat_simple() {
    let bounds = List::new();
    let gat = AssociatedTypeGAT::simple(Text::from("Item"), bounds);

    assert_eq!(gat.name, "Item");
    assert!(!gat.is_gat());
    assert_eq!(gat.arity(), 0);
    assert_eq!(gat.kind, AssociatedTypeKind::Regular);
}

#[test]
fn test_associated_type_gat_generic() {
    let mut type_params = List::new();
    type_params.push(GATTypeParam {
        name: Text::from("T"),
        bounds: List::new(),
        default: None,
        variance: Variance::Covariant,
    });

    let bounds = List::new();
    let where_clauses = List::new();

    let gat =
        AssociatedTypeGAT::generic(Text::from("Container"), type_params, bounds, where_clauses);

    assert_eq!(gat.name, "Container");
    assert!(gat.is_gat());
    assert_eq!(gat.arity(), 1);
    match gat.kind {
        AssociatedTypeKind::Generic { arity } => assert_eq!(arity, 1),
        _ => panic!("Expected Generic kind"),
    }
}

#[test]
fn test_gat_type_param_equality() {
    let param1 = GATTypeParam {
        name: Text::from("T"),
        bounds: List::new(),
        default: None,
        variance: Variance::Covariant,
    };

    let param2 = GATTypeParam {
        name: Text::from("T"),
        bounds: List::new(),
        default: None,
        variance: Variance::Covariant,
    };

    assert_eq!(param1, param2);
}

#[test]
fn test_gat_where_clause_equality() {
    let path = make_path(&["Clone"]);

    let bound = ProtocolBound {
        protocol: path,
        args: List::new(),
        is_negative: false,
    };

    let mut constraints = List::new();
    constraints.push(bound);

    let clause1 = GATWhereClause {
        param: Text::from("T"),
        constraints: constraints.clone(),
        span: Span::default(),
    };

    let clause2 = GATWhereClause {
        param: Text::from("T"),
        constraints,
        span: Span::default(),
    };

    assert_eq!(clause1, clause2);
}

#[test]
fn test_gat_error_variants() {
    let error1 = GATError::ConstraintViolation {
        param: Text::from("T"),
        constraint: Text::from("Clone"),
        counterexample: None,
    };

    let mut cycle = List::new();
    cycle.push(Text::from("A"));
    cycle.push(Text::from("B"));
    cycle.push(Text::from("A"));

    let error2 = GATError::CircularDependency { cycle };

    let error3 = GATError::VarianceViolation {
        param: Text::from("T"),
        expected: Variance::Covariant,
        found: Variance::Invariant,
    };

    match error1 {
        GATError::ConstraintViolation { .. } => {}
        _ => panic!("Expected ConstraintViolation"),
    }

    match error2 {
        GATError::CircularDependency { .. } => {}
        _ => panic!("Expected CircularDependency"),
    }

    match error3 {
        GATError::VarianceViolation { .. } => {}
        _ => panic!("Expected VarianceViolation"),
    }
}

// ==================== CBGR Predicates Tests ====================

#[test]
fn test_cbgr_predicate_generation() {
    let pred = CBGRPredicate::Generation {
        reference: Text::from("x"),
    };

    assert_eq!(pred.referenced_variables().len(), 1);
    assert!(pred.is_pure());

    let display = format!("{}", pred);
    assert!(display.contains("generation"));
    assert!(display.contains("x"));
}

#[test]
fn test_cbgr_predicate_epoch() {
    let pred = CBGRPredicate::Epoch {
        reference: Text::from("y"),
    };

    assert_eq!(pred.referenced_variables().len(), 1);
    assert!(pred.is_pure());

    let display = format!("{}", pred);
    assert!(display.contains("epoch"));
    assert!(display.contains("y"));
}

#[test]
fn test_cbgr_predicate_valid() {
    let pred = CBGRPredicate::Valid {
        reference: Text::from("ptr"),
    };

    assert_eq!(pred.referenced_variables().len(), 1);
    assert!(pred.is_pure());

    let display = format!("{}", pred);
    assert!(display.contains("valid"));
    assert!(display.contains("ptr"));
}

#[test]
fn test_cbgr_predicate_same_allocation() {
    let pred = CBGRPredicate::SameAllocation {
        ref_a: Text::from("a"),
        ref_b: Text::from("b"),
    };

    assert_eq!(pred.referenced_variables().len(), 2);
    assert!(pred.is_pure());

    let display = format!("{}", pred);
    assert!(display.contains("same_allocation"));
    assert!(display.contains("a"));
    assert!(display.contains("b"));
}

#[test]
fn test_cbgr_predicate_generation_compare() {
    let pred = CBGRPredicate::GenerationCompare {
        ref_a: Text::from("x"),
        ref_b: Text::from("y"),
        op: ComparisonOp::Lt,
    };

    assert_eq!(pred.referenced_variables().len(), 2);
    assert!(pred.is_pure());

    let display = format!("{}", pred);
    assert!(display.contains("generation"));
    assert!(display.contains("<"));
}

#[test]
fn test_comparison_op_display() {
    assert_eq!(format!("{}", ComparisonOp::Eq), "==");
    assert_eq!(format!("{}", ComparisonOp::Ne), "!=");
    assert_eq!(format!("{}", ComparisonOp::Lt), "<");
    assert_eq!(format!("{}", ComparisonOp::Le), "<=");
    assert_eq!(format!("{}", ComparisonOp::Gt), ">");
    assert_eq!(format!("{}", ComparisonOp::Ge), ">=");
}

#[test]
fn test_comparison_op_equality() {
    assert_eq!(ComparisonOp::Eq, ComparisonOp::Eq);
    assert_eq!(ComparisonOp::Lt, ComparisonOp::Lt);
    assert_ne!(ComparisonOp::Eq, ComparisonOp::Ne);
}

#[test]
fn test_generation_predicate_constructors() {
    let gen_pred = GenerationPredicate::generation(Text::from("ref1"));
    match gen_pred.kind {
        CBGRPredicate::Generation { .. } => {}
        _ => panic!("Expected Generation predicate"),
    }

    let epoch_pred = GenerationPredicate::epoch(Text::from("ref2"));
    match epoch_pred.kind {
        CBGRPredicate::Epoch { .. } => {}
        _ => panic!("Expected Epoch predicate"),
    }

    let valid_pred = GenerationPredicate::valid(Text::from("ref3"));
    match valid_pred.kind {
        CBGRPredicate::Valid { .. } => {}
        _ => panic!("Expected Valid predicate"),
    }

    let same_alloc_pred = GenerationPredicate::same_allocation(Text::from("a"), Text::from("b"));
    match same_alloc_pred.kind {
        CBGRPredicate::SameAllocation { .. } => {}
        _ => panic!("Expected SameAllocation predicate"),
    }

    let compare_pred =
        GenerationPredicate::generation_compare(Text::from("x"), Text::from("y"), ComparisonOp::Gt);
    match compare_pred.kind {
        CBGRPredicate::GenerationCompare { .. } => {}
        _ => panic!("Expected GenerationCompare predicate"),
    }
}

#[test]
fn test_reference_value_construction() {
    let ref_val = ReferenceValue {
        ptr: 0x1000,
        generation: 42,
        epoch: 1,
        is_valid: true,
    };

    assert_eq!(ref_val.ptr, 0x1000);
    assert_eq!(ref_val.generation, 42);
    assert_eq!(ref_val.epoch, 1);
    assert!(ref_val.is_valid);
}

#[test]
fn test_cbgr_stats_default() {
    let stats = CBGRStats::default();

    assert_eq!(stats.generation_checks, 0);
    assert_eq!(stats.epoch_checks, 0);
    assert_eq!(stats.validity_checks, 0);
    assert_eq!(stats.allocation_checks, 0);
    assert_eq!(stats.smt_time, Duration::from_secs(0));
}

#[test]
fn test_cbgr_verification_result() {
    let stats = CBGRStats::default();
    let result = CBGRVerificationResult {
        is_valid: true,
        duration: Duration::from_millis(100),
        counterexample: None,
        stats,
    };

    assert!(result.is_valid);
    assert_eq!(result.duration, Duration::from_millis(100));
    // Check counterexample is None using matches! since CBGRCounterexample doesn't derive PartialEq
    assert!(result.counterexample.is_none());
}

#[test]
fn test_cbgr_counterexample() {
    let mut ref_values = Map::new();
    ref_values.insert(
        Text::from("x"),
        ReferenceValue {
            ptr: 0x1000,
            generation: 10,
            epoch: 1,
            is_valid: false,
        },
    );

    let counterexample = CBGRCounterexample {
        ref_values,
        violated_property: Text::from("valid(x)"),
        explanation: Text::from("Reference is stale"),
    };

    assert_eq!(counterexample.violated_property, "valid(x)");
    assert_eq!(counterexample.explanation, "Reference is stale");
}

// ==================== Specialization Tests ====================

#[test]
fn test_specialization_lattice_creation() {
    let path = make_path(&["Display"]);

    let lattice = SpecializationLattice::new(path.clone());

    assert_eq!(lattice.protocol, path);
    assert_eq!(lattice.implementations.len(), 0);
    assert_eq!(lattice.ordering.len(), 0);
    assert_eq!(lattice.roots.len(), 0);
    assert_eq!(lattice.leaves.len(), 0);
}

#[test]
fn test_specialization_lattice_add_impl() {
    let path = make_path(&["Display"]);

    let mut lattice = SpecializationLattice::new(path);
    let for_type = make_int_type();

    lattice.add_impl(1, for_type.clone(), None);

    assert_eq!(lattice.implementations.len(), 1);
    assert!(lattice.implementations.contains_key(&1));
}

#[test]
fn test_specialization_lattice_add_ordering() {
    let path = make_path(&["Display"]);

    let mut lattice = SpecializationLattice::new(path);
    let for_type1 = make_int_type();
    let for_type2 = make_bool_type();

    lattice.add_impl(1, for_type1, None);
    lattice.add_impl(2, for_type2, None);
    lattice.add_ordering(1, 2);

    assert_eq!(lattice.ordering.len(), 1);
    assert!(lattice.is_more_specific(1, 2));
    assert!(!lattice.is_more_specific(2, 1));
}

#[test]
fn test_specialization_node_root_leaf() {
    let node_root = SpecializationNode {
        id: 1,
        for_type: make_int_type(),
        info: None,
        specializations: Set::new(),
        generalizations: Set::new(),
    };

    assert!(node_root.is_root());
    assert!(node_root.is_leaf());

    let mut node_with_spec = node_root.clone();
    node_with_spec.specializations.insert(2);

    assert!(node_with_spec.is_root());
    assert!(!node_with_spec.is_leaf());

    let mut node_with_gen = node_root.clone();
    node_with_gen.generalizations.insert(3);

    assert!(!node_with_gen.is_root());
    assert!(node_with_gen.is_leaf());
}

#[test]
fn test_specificity_ordering_display() {
    assert_eq!(
        format!("{}", SpecificityOrdering::MoreSpecific),
        "more specific"
    );
    assert_eq!(
        format!("{}", SpecificityOrdering::LessSpecific),
        "less specific"
    );
    assert_eq!(
        format!("{}", SpecificityOrdering::Equal),
        "equally specific"
    );
    assert_eq!(
        format!("{}", SpecificityOrdering::Incomparable),
        "incomparable"
    );
}

#[test]
fn test_specificity_ordering_equality() {
    assert_eq!(
        SpecificityOrdering::MoreSpecific,
        SpecificityOrdering::MoreSpecific
    );
    assert_eq!(SpecificityOrdering::Equal, SpecificityOrdering::Equal);
    assert_ne!(
        SpecificityOrdering::MoreSpecific,
        SpecificityOrdering::LessSpecific
    );
}

#[test]
fn test_specialization_error_variants() {
    let error1 = SpecializationError::AmbiguousSpecialization {
        ty: make_int_type(),
        protocol: Text::from("Display"),
        candidates: List::new(),
    };

    let mut cycle = List::new();
    cycle.push(1);
    cycle.push(2);
    cycle.push(1);

    let error2 = SpecializationError::SpecializationCycle { cycle };

    let error3 = SpecializationError::AntisymmetryViolation { impl1: 1, impl2: 2 };

    match error1 {
        SpecializationError::AmbiguousSpecialization { .. } => {}
        _ => panic!("Expected AmbiguousSpecialization"),
    }

    match error2 {
        SpecializationError::SpecializationCycle { .. } => {}
        _ => panic!("Expected SpecializationCycle"),
    }

    match error3 {
        SpecializationError::AntisymmetryViolation { .. } => {}
        _ => panic!("Expected AntisymmetryViolation"),
    }
}

#[test]
fn test_specialization_stats_default() {
    let stats = SpecializationStats::default();

    assert_eq!(stats.impl_count, 0);
    assert_eq!(stats.relationships_checked, 0);
    assert_eq!(stats.overlap_checks, 0);
    assert_eq!(stats.smt_time, Duration::from_secs(0));
}

#[test]
fn test_specialization_verification_result() {
    let stats = SpecializationStats::default();
    let result = SpecializationVerificationResult {
        is_coherent: true,
        duration: Duration::from_millis(50),
        errors: List::new(),
        ambiguities: List::new(),
        stats,
    };

    assert!(result.is_coherent);
    assert_eq!(result.duration, Duration::from_millis(50));
    assert_eq!(result.errors.len(), 0);
    assert_eq!(result.ambiguities.len(), 0);
}

#[test]
fn test_specialization_condition_variants() {
    let cond1 = SpecializationCondition::ExactType {
        ty: make_int_type(),
    };

    let path = make_path(&["Clone"]);

    let bound = ProtocolBound {
        protocol: path.clone(),
        args: List::new(),
        is_negative: false,
    };

    let cond2 = SpecializationCondition::Constraint {
        bound: bound.clone(),
    };

    let cond3 = SpecializationCondition::NegativeConstraint { bound };

    match cond1 {
        SpecializationCondition::ExactType { .. } => {}
        _ => panic!("Expected ExactType"),
    }

    match cond2 {
        SpecializationCondition::Constraint { .. } => {}
        _ => panic!("Expected Constraint"),
    }

    match cond3 {
        SpecializationCondition::NegativeConstraint { .. } => {}
        _ => panic!("Expected NegativeConstraint"),
    }
}

#[test]
fn test_ambiguity_construction() {
    let ambiguity = Ambiguity {
        ty: make_int_type(),
        protocol: Text::from("Display"),
        candidates: List::new(),
        explanation: Text::from("Multiple equally specific implementations found"),
    };

    assert_eq!(ambiguity.protocol, "Display");
    assert_eq!(
        ambiguity.explanation,
        "Multiple equally specific implementations found"
    );
}

// ==================== Integration Tests ====================

#[test]
fn test_protocol_with_gat_associated_types() {
    // Create a GAT
    let mut type_params = List::new();
    type_params.push(GATTypeParam {
        name: Text::from("T"),
        bounds: List::new(),
        default: None,
        variance: Variance::Covariant,
    });

    let gat = AssociatedTypeGAT::generic(Text::from("Item"), type_params, List::new(), List::new());

    assert!(gat.is_gat());
    assert_eq!(gat.arity(), 1);
}

#[test]
fn test_cbgr_predicate_chain() {
    // Test chaining predicates (e.g., valid(x) && generation(x) > generation(y))
    let pred1 = CBGRPredicate::Valid {
        reference: Text::from("x"),
    };

    let pred2 = CBGRPredicate::GenerationCompare {
        ref_a: Text::from("x"),
        ref_b: Text::from("y"),
        op: ComparisonOp::Gt,
    };

    let refs1 = pred1.referenced_variables();
    let refs2 = pred2.referenced_variables();

    assert!(refs1.contains(&Text::from("x")));
    assert!(refs2.contains(&Text::from("x")));
    assert!(refs2.contains(&Text::from("y")));
}

#[test]
fn test_specialization_lattice_complex() {
    let path = make_path(&["Display"]);

    let mut lattice = SpecializationLattice::new(path);

    // Add three implementations with ordering:
    // impl1 (general) <- impl2 (specific) <- impl3 (most specific)
    lattice.add_impl(1, make_int_type(), None);
    lattice.add_impl(2, make_bool_type(), None);
    lattice.add_impl(3, make_text_type(), None);

    lattice.add_ordering(2, 1); // impl2 is more specific than impl1
    lattice.add_ordering(3, 2); // impl3 is more specific than impl2

    assert!(lattice.is_more_specific(2, 1));
    assert!(lattice.is_more_specific(3, 2));
    assert!(!lattice.is_more_specific(1, 2));
}
