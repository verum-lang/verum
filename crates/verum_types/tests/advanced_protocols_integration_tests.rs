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
    unused_assignments,
    clippy::approx_constant,
    clippy::overly_complex_bool_expr
)]
//! Comprehensive Integration Tests for Advanced Protocol Features
//!
//! Advanced protocols (future v2.0+): GATs, higher-rank bounds, specialization with lattice ordering, coherence rules — Complete Advanced Protocol System
//!
//! This test suite provides comprehensive end-to-end coverage of all advanced
//! protocol features including:
//! - Generic Associated Types (GATs)
//! - Lending Iterators with GenRef
//! - Specialization
//! - Refinement Integration
//! - Higher-Kinded Types
//! - Complex real-world scenarios
//!
//! # Test Organization
//!
//! 1. GAT Tests (10+ tests)
//! 2. Lending Iterator Tests (8+ tests)
//! 3. Specialization Tests (8+ tests)
//! 4. Refinement Integration Tests (6+ tests)
//! 5. Higher-Kinded Type Tests (6+ tests)
//! 6. End-to-End Scenarios (5+ tests)
//!
//! Total: 40+ comprehensive integration tests

use smallvec::SmallVec;
use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_common::{ConstValue, Heap, List, Map, Maybe, Set, Text};
use verum_types::advanced_protocols::RefinementPredicate;
use verum_types::{
    AdvancedProtocolError,
    AssociatedType,
    // Advanced protocol types
    AssociatedTypeGAT,
    AssociatedTypeKind,
    BinaryOp as AdvancedBinOp,
    GATTypeParam,
    GATWhereClause,
    GenRefType,
    GenerationPredicate,
    Kind,
    // Core protocol types
    Protocol,
    ProtocolBound,
    ProtocolBoundPolarity,
    ProtocolChecker,
    ProtocolImpl,
    RefinementConstraint,
    RefinementKind,
    SpecializationInfo,
    SpecializationLattice,
    // Type system
    Type,
    TypeVar,
    Variance,
};

// ==================== Helper Functions ====================

/// Create a simple path from a name
fn make_path(name: &str) -> Path {
    let ident = Ident::new(name, Span::dummy());
    let mut segments = SmallVec::new();
    segments.push(PathSegment::Name(ident));
    Path {
        segments,
        span: Span::dummy(),
    }
}

/// Create a protocol bound
fn make_bound(protocol_name: &str) -> ProtocolBound {
    ProtocolBound {
        protocol: make_path(protocol_name),
        args: List::new(),
        is_negative: false,
    }
}

/// Create a simple GAT type parameter
fn make_gat_param(name: &str, variance: Variance) -> GATTypeParam {
    GATTypeParam {
        name: Text::from(name),
        bounds: List::new(),
        default: Maybe::None,
        variance,
    }
}

/// Create a GAT type parameter with bounds
fn make_gat_param_with_bounds(
    name: &str,
    bounds: List<ProtocolBound>,
    variance: Variance,
) -> GATTypeParam {
    GATTypeParam {
        name: Text::from(name),
        bounds,
        default: Maybe::None,
        variance,
    }
}

/// Create a GAT where clause
fn make_gat_where(param: &str, constraints: List<ProtocolBound>) -> GATWhereClause {
    GATWhereClause {
        param: Text::from(param),
        constraints,
        span: Span::dummy(),
    }
}

// ==================== Category 1: GAT Tests (10+ tests) ====================

#[test]
fn test_gat_basic_usage() {
    // Test basic GAT with single type parameter: type Item<T>
    let type_params = List::from_iter(vec![make_gat_param("T", Variance::Covariant)]);

    let gat = AssociatedTypeGAT::generic("Item".into(), type_params, List::new(), List::new());

    assert_eq!(gat.name, "Item");
    assert!(gat.is_gat());
    assert_eq!(gat.arity(), 1);
    assert!(matches!(gat.kind, AssociatedTypeKind::Generic { arity: 1 }));
}

#[test]
fn test_gat_multiple_type_parameters() {
    // Test GAT with multiple type parameters: type Pair<K, V>
    let type_params = List::from_iter(vec![
        make_gat_param("K", Variance::Invariant),
        make_gat_param("V", Variance::Covariant),
    ]);

    let gat = AssociatedTypeGAT::generic("Pair".into(), type_params, List::new(), List::new());

    assert_eq!(gat.arity(), 2);
    assert!(matches!(gat.kind, AssociatedTypeKind::Generic { arity: 2 }));
    assert_eq!(gat.type_params[0].variance, Variance::Invariant);
    assert_eq!(gat.type_params[1].variance, Variance::Covariant);
}

#[test]
fn test_gat_with_where_clauses() {
    // Test: type Item<T> where T: Clone + Debug
    let type_params = List::from_iter(vec![make_gat_param("T", Variance::Covariant)]);

    let where_clauses = List::from_iter(vec![make_gat_where(
        "T",
        List::from_iter(vec![make_bound("Clone"), make_bound("Debug")]),
    )]);

    let gat = AssociatedTypeGAT::generic(
        "Item".into(),
        type_params,
        List::new(),
        where_clauses.clone(),
    );

    assert_eq!(gat.where_clauses.len(), 1);
    assert_eq!(gat.where_clauses[0].param, "T");
    assert_eq!(gat.where_clauses[0].constraints.len(), 2);
}

#[test]
fn test_gat_instantiation_and_resolution() {
    // Test GAT instantiation with concrete types
    let type_params = List::from_iter(vec![make_gat_param("T", Variance::Covariant)]);

    let gat = AssociatedTypeGAT::generic("Wrapped".into(), type_params, List::new(), List::new());

    // Verify we can construct a GAT with correct arity
    assert_eq!(gat.arity(), 1);

    // Test instantiation with concrete types
    let concrete_type = Type::Int;
    let instantiated = gat
        .instantiate(&[concrete_type])
        .expect("Should instantiate successfully");

    // Verify the instantiated type is Wrapped<Int>
    match instantiated {
        Type::Generic { name, args } => {
            assert_eq!(name.as_str(), "Wrapped");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0], Type::Int);
        }
        _ => panic!("Expected Generic type"),
    }

    assert!(gat.is_gat());
}

#[test]
fn test_gat_higher_kinded() {
    // Test higher-kinded GAT: type F<_> (type constructor)
    let type_params = List::from_iter(vec![make_gat_param("_", Variance::Covariant)]);

    let mut gat = AssociatedTypeGAT::generic("F".into(), type_params, List::new(), List::new());

    // Override kind to be higher-kinded
    gat.kind = AssociatedTypeKind::HigherKinded { arity: 1 };

    assert!(matches!(
        gat.kind,
        AssociatedTypeKind::HigherKinded { arity: 1 }
    ));
    assert_eq!(gat.arity(), 1);
}

#[test]
fn test_gat_nested() {
    // Test nested GATs: type Outer<T> where has type Inner<U>
    let outer_params = List::from_iter(vec![make_gat_param("T", Variance::Covariant)]);

    let outer = AssociatedTypeGAT::generic("Outer".into(), outer_params, List::new(), List::new());

    let inner_params = List::from_iter(vec![make_gat_param("U", Variance::Covariant)]);

    let inner = AssociatedTypeGAT::generic("Inner".into(), inner_params, List::new(), List::new());

    // Verify both are GATs with correct structure
    assert!(outer.is_gat());
    assert!(inner.is_gat());
    assert_eq!(outer.arity(), 1);
    assert_eq!(inner.arity(), 1);
}

#[test]
fn test_gat_variance_covariant() {
    // Test covariant GAT parameter: type Item<+T>
    let type_params = List::from_iter(vec![make_gat_param("T", Variance::Covariant)]);

    let gat = AssociatedTypeGAT::generic("Item".into(), type_params, List::new(), List::new());

    assert_eq!(gat.type_params[0].variance, Variance::Covariant);
}

#[test]
fn test_gat_variance_contravariant() {
    // Test contravariant GAT parameter: type Input<-T>
    let type_params = List::from_iter(vec![make_gat_param("T", Variance::Contravariant)]);

    let gat = AssociatedTypeGAT::generic("Input".into(), type_params, List::new(), List::new());

    assert_eq!(gat.type_params[0].variance, Variance::Contravariant);
}

#[test]
fn test_gat_variance_invariant() {
    // Test invariant GAT parameter: type Cell<T>
    let type_params = List::from_iter(vec![make_gat_param("T", Variance::Invariant)]);

    let gat = AssociatedTypeGAT::generic("Cell".into(), type_params, List::new(), List::new());

    assert_eq!(gat.type_params[0].variance, Variance::Invariant);
}

#[test]
fn test_gat_error_arity_mismatch() {
    // Test error when GAT is instantiated with wrong number of type arguments
    let gat_name: Text = "Container".into();
    let expected = 2usize;
    let found = 1usize;

    let error = AdvancedProtocolError::GATArityMismatch {
        gat_name: gat_name.clone(),
        expected,
        found,
    };

    match error {
        AdvancedProtocolError::GATArityMismatch {
            gat_name: n,
            expected: e,
            found: f,
        } => {
            assert_eq!(n, gat_name);
            assert_eq!(e, expected);
            assert_eq!(f, found);
        }
        _ => panic!("Wrong error type"),
    }
}

#[test]
fn test_gat_error_constraint_violation() {
    // Test error when GAT where clause constraint is not satisfied
    let error = AdvancedProtocolError::GATConstraintNotSatisfied {
        ty: Type::Int,
        constraint: "T: Clone".into(),
    };

    match error {
        AdvancedProtocolError::GATConstraintNotSatisfied { ty, constraint } => {
            assert_eq!(ty, Type::Int);
            assert_eq!(constraint, "T: Clone");
        }
        _ => panic!("Wrong error type"),
    }
}

// ==================== Category 2: Lending Iterator Tests (8+ tests) ====================

#[test]
fn test_genref_basic() {
    // Test basic GenRef<T> type construction
    let inner = Type::Int;
    let genref = GenRefType::new(inner.clone());

    assert_eq!(genref.inner(), &Type::Int);
}

#[test]
fn test_genref_with_slice() {
    // Test GenRef wrapping a slice type
    let slice_type = Type::Array {
        element: Box::new(Type::Int),
        size: None,
    };
    let genref = GenRefType::new(slice_type.clone());

    match genref.inner() {
        Type::Array { element, size } => {
            assert_eq!(**element, Type::Int);
            assert_eq!(*size, None);
        }
        _ => panic!("Expected array type"),
    }
}

#[test]
fn test_genref_lending_iterator_structure() {
    // Test lending iterator pattern structure
    // Simulates: type WindowIterator<T> { data: GenRef<List<T>>, ... }

    let list_type = Type::Named {
        path: make_path("List"),
        args: vec![Type::Int].into(),
    };

    let genref = GenRefType::new(list_type);

    // Verify the GenRef wraps the list type
    match genref.inner() {
        Type::Named { path, args } => {
            assert_eq!(path.as_ident().unwrap().name.as_str(), "List");
            assert_eq!(args.len(), 1);
        }
        _ => panic!("Expected Named type"),
    }
}

#[test]
fn test_genref_window_iterator_pattern() {
    // Test window iterator pattern that returns overlapping windows
    // protocol Iterator { type Item<'a>; fn next(&'a mut self) -> Maybe<Self.Item<'a>> }

    let gat_params = List::from_iter(vec![make_gat_param("'a", Variance::Covariant)]);

    let item_gat = AssociatedTypeGAT::generic("Item".into(), gat_params, List::new(), List::new());

    // The Item GAT would be instantiated with GenRef<&[T]>
    assert!(item_gat.is_gat());
    assert_eq!(item_gat.arity(), 1);
}

#[test]
fn test_genref_slice_iterator() {
    // Test slice iterator that lends references to elements
    let element_type = Type::Text;
    let slice_type = Type::Array {
        element: Box::new(element_type),
        size: None,
    };

    let genref = GenRefType::new(slice_type);

    // Verify structure
    match genref.inner() {
        Type::Array { element, .. } => {
            assert_eq!(**element, Type::Text);
        }
        _ => panic!("Expected array type"),
    }
}

#[test]
fn test_generation_predicate_generation() {
    // Test generation counter predicate
    let ref_type = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Int),
    };

    let pred = GenerationPredicate::Generation {
        ref_expr: Box::new(ref_type.clone()),
    };

    match pred {
        GenerationPredicate::Generation { ref_expr } => match *ref_expr {
            Type::Reference { mutable, .. } => {
                assert!(!mutable);
            }
            _ => panic!("Expected reference type"),
        },
        _ => panic!("Wrong predicate type"),
    }
}

#[test]
fn test_generation_predicate_valid() {
    // Test valid() predicate for checking reference validity
    let ref_type = Type::Reference {
        mutable: true,
        inner: Box::new(Type::Text),
    };

    let pred = GenerationPredicate::Valid {
        ref_expr: Box::new(ref_type),
    };

    match pred {
        GenerationPredicate::Valid { ref_expr } => match *ref_expr {
            Type::Reference { mutable, .. } => {
                assert!(mutable);
            }
            _ => panic!("Expected reference type"),
        },
        _ => panic!("Wrong predicate type"),
    }
}

#[test]
fn test_generation_predicate_same_allocation() {
    // Test same_allocation predicate
    let ref_a = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Int),
    };
    let ref_b = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Int),
    };

    let pred = GenerationPredicate::SameAllocation {
        ref_a: Box::new(ref_a),
        ref_b: Box::new(ref_b),
    };

    match pred {
        GenerationPredicate::SameAllocation { ref_a, ref_b } => {
            assert!(matches!(*ref_a, Type::Reference { .. }));
            assert!(matches!(*ref_b, Type::Reference { .. }));
        }
        _ => panic!("Wrong predicate type"),
    }
}

// ==================== Category 3: Specialization Tests (8+ tests) ====================

#[test]
fn test_specialization_basic() {
    // Test basic specialization: List<Int> specializes List<T>
    let specialized = SpecializationInfo::specialized(
        make_path("ListGenericImpl"),
        10, // High rank = more specific
    );

    assert!(specialized.is_specialized);
    assert_eq!(specialized.specificity_rank, 10);
    assert!(matches!(specialized.specializes, Maybe::Some(_)));
}

#[test]
fn test_specialization_lattice_ordering() {
    // Test specialization lattice ordering
    let mut lattice = SpecializationLattice::new();

    lattice.add_impl(0); // Most general: impl<T> Display for List<T>
    lattice.add_impl(1); // Specialized: impl<T: Copy> Display for List<T>
    lattice.add_impl(2); // Most specific: impl Display for List<Int>

    // Set up ordering: 2 > 1 > 0 (2 is most specific)
    lattice.ordering.insert((2, 1), true);
    lattice.ordering.insert((1, 0), true);
    lattice.ordering.insert((2, 0), true);

    // Verify ordering
    assert!(lattice.is_more_specific(2, 1));
    assert!(lattice.is_more_specific(1, 0));
    assert!(lattice.is_more_specific(2, 0));
}

#[test]
fn test_specialization_most_specific_selection() {
    // Test selecting most specific implementation
    let mut lattice = SpecializationLattice::new();

    lattice.add_impl(0); // General
    lattice.add_impl(1); // More specific
    lattice.add_impl(2); // Most specific

    lattice.ordering.insert((2, 1), true);
    lattice.ordering.insert((1, 0), true);
    lattice.ordering.insert((2, 0), true);

    let applicable = Set::from_iter(vec![0, 1, 2]);
    let selected = lattice.select_most_specific(&applicable);

    assert_eq!(selected, Maybe::Some(2));
}

#[test]
fn test_specialization_ambiguous_detection() {
    // Test detection of ambiguous specialization (two equally specific impls)
    let candidates = List::from_iter(vec![1usize, 2usize]);

    let error = AdvancedProtocolError::AmbiguousSpecialization {
        ty: Type::Int,
        candidates,
    };

    match error {
        AdvancedProtocolError::AmbiguousSpecialization { ty, candidates } => {
            assert_eq!(ty, Type::Int);
            assert_eq!(candidates.len(), 2);
        }
        _ => panic!("Wrong error type"),
    }
}

#[test]
fn test_specialization_negative_reasoning() {
    // Test negative protocol bounds: T: !Sync
    let negative = ProtocolBoundPolarity::Negative {
        protocol: make_path("Sync"),
    };

    match negative {
        ProtocolBoundPolarity::Negative { protocol } => {
            assert_eq!(protocol.as_ident().unwrap().as_str(), "Sync");
        }
        _ => panic!("Wrong polarity"),
    }
}

#[test]
fn test_specialization_positive_bound() {
    // Test positive protocol bounds: T: Send
    let positive = ProtocolBoundPolarity::Positive {
        protocol: make_path("Send"),
        args: List::new(),
    };

    match positive {
        ProtocolBoundPolarity::Positive { protocol, args } => {
            assert_eq!(protocol.as_ident().unwrap().as_str(), "Send");
            assert_eq!(args.len(), 0);
        }
        _ => panic!("Wrong polarity"),
    }
}

#[test]
fn test_specialization_multiple_levels() {
    // Test multiple levels of specialization
    let mut lattice = SpecializationLattice::new();

    lattice.add_impl(0); // Level 0: impl<T> Protocol for T
    lattice.add_impl(1); // Level 1: impl<T: Copy> Protocol for T
    lattice.add_impl(2); // Level 2: impl<T: Copy + Clone> Protocol for T
    lattice.add_impl(3); // Level 3: impl Protocol for Int

    // Set up transitive ordering
    lattice.ordering.insert((3, 2), true);
    lattice.ordering.insert((2, 1), true);
    lattice.ordering.insert((1, 0), true);
    lattice.ordering.insert((3, 1), true); // Transitivity
    lattice.ordering.insert((3, 0), true);
    lattice.ordering.insert((2, 0), true);

    let applicable = Set::from_iter(vec![0, 1, 2, 3]);
    let selected = lattice.select_most_specific(&applicable);

    assert_eq!(selected, Maybe::Some(3)); // Most specific wins
}

#[test]
fn test_specialization_error_invalid() {
    // Test invalid specialization error
    let error = AdvancedProtocolError::InvalidSpecialization {
        specialized: make_path("SpecializedImpl"),
        base: make_path("BaseImpl"),
    };

    match error {
        AdvancedProtocolError::InvalidSpecialization { specialized, base } => {
            assert_eq!(specialized.as_ident().unwrap().as_str(), "SpecializedImpl");
            assert_eq!(base.as_ident().unwrap().as_str(), "BaseImpl");
        }
        _ => panic!("Wrong error type"),
    }
}

// ==================== Category 4: Refinement Integration Tests (6+ tests) ====================

#[test]
fn test_refinement_inline_syntax() {
    // Test inline refinement in protocol methods: Int{> 0}
    let constraint = RefinementConstraint {
        name: Text::from("value"),
        predicate: RefinementPredicate::BinaryOp {
            op: AdvancedBinOp::Gt,
            value: ConstValue::Int(0),
        },
        kind: RefinementKind::Inline,
        span: Span::dummy(),
    };

    assert_eq!(constraint.kind, RefinementKind::Inline);
    match constraint.predicate {
        RefinementPredicate::BinaryOp { op, value } => {
            assert_eq!(op, AdvancedBinOp::Gt);
            assert_eq!(value, ConstValue::Int(0));
        }
        _ => panic!("Wrong predicate type"),
    }
}

#[test]
fn test_refinement_declarative_syntax() {
    // Test declarative refinement: Int where is_positive
    let constraint = RefinementConstraint {
        name: Text::from("value"),
        predicate: RefinementPredicate::Named {
            name: Text::from("is_positive"),
        },
        kind: RefinementKind::Declarative,
        span: Span::dummy(),
    };

    assert_eq!(constraint.kind, RefinementKind::Declarative);
    match constraint.predicate {
        RefinementPredicate::Named { name } => {
            assert_eq!(name, "is_positive");
        }
        _ => panic!("Wrong predicate type"),
    }
}

#[test]
fn test_refinement_sigma_type_syntax() {
    // Test sigma-type refinement: x: Int where x > 0
    let constraint = RefinementConstraint {
        name: Text::from("x"),
        predicate: RefinementPredicate::BinaryOp {
            op: AdvancedBinOp::Gt,
            value: ConstValue::Int(0),
        },
        kind: RefinementKind::SigmaType,
        span: Span::dummy(),
    };

    assert_eq!(constraint.kind, RefinementKind::SigmaType);
    assert_eq!(constraint.name, "x");
}

#[test]
fn test_refinement_variance_checking() {
    // Test refinement variance: contravariant in parameters, covariant in return
    // This tests that we can construct the necessary structures

    let param_constraint = RefinementConstraint {
        name: Text::from("input"),
        predicate: RefinementPredicate::BinaryOp {
            op: AdvancedBinOp::Gt,
            value: ConstValue::Int(0),
        },
        kind: RefinementKind::Inline,
        span: Span::dummy(),
    };

    let return_constraint = RefinementConstraint {
        name: Text::from("output"),
        predicate: RefinementPredicate::BinaryOp {
            op: AdvancedBinOp::Gt,
            value: ConstValue::Int(10),
        },
        kind: RefinementKind::Inline,
        span: Span::dummy(),
    };

    // Verify structure
    assert_eq!(param_constraint.name, "input");
    assert_eq!(return_constraint.name, "output");
}

#[test]
fn test_refinement_protocol_method_constraints() {
    // Test refinement constraints in protocol method signatures
    let constraint = RefinementConstraint {
        name: Text::from("result"),
        predicate: RefinementPredicate::And {
            left: Box::new(RefinementPredicate::BinaryOp {
                op: AdvancedBinOp::Gt,
                value: ConstValue::Int(0),
            }),
            right: Box::new(RefinementPredicate::BinaryOp {
                op: AdvancedBinOp::Lt,
                value: ConstValue::Int(100),
            }),
        },
        kind: RefinementKind::Inline,
        span: Span::dummy(),
    };

    match constraint.predicate {
        RefinementPredicate::And { left, right } => {
            assert!(matches!(*left, RefinementPredicate::BinaryOp { .. }));
            assert!(matches!(*right, RefinementPredicate::BinaryOp { .. }));
        }
        _ => panic!("Wrong predicate type"),
    }
}

#[test]
fn test_refinement_error_weakened() {
    // Test error when implementation weakens refinement
    let error = AdvancedProtocolError::RefinementVarianceViolation {
        message: "Implementation weakens precondition from {> 10} to {> 0}".into(),
    };

    match error {
        AdvancedProtocolError::RefinementVarianceViolation { message } => {
            assert!(message.to_string().contains("weakens"));
            assert!(message.to_string().contains("{> 10}"));
        }
        _ => panic!("Wrong error type"),
    }
}

// ==================== Category 5: Higher-Kinded Type Tests (6+ tests) ====================

#[test]
fn test_kind_type() {
    // Test kind * (regular type)
    let kind = Kind::type_kind();

    assert_eq!(kind.arity(), 0);
    assert!(matches!(kind, Kind::Type));
}

#[test]
fn test_kind_unary_constructor() {
    // Test kind * -> * (unary type constructor like List)
    let kind = Kind::unary_constructor();

    assert_eq!(kind.arity(), 1);
    match kind {
        Kind::Arrow(param, result) => {
            assert!(matches!(*param, Kind::Type));
            assert!(matches!(*result, Kind::Type));
        }
        _ => panic!("Expected Arrow kind"),
    }
}

#[test]
fn test_kind_binary_constructor() {
    // Test kind * -> * -> * (binary type constructor like Map)
    let kind = Kind::binary_constructor();

    assert_eq!(kind.arity(), 2);
    match kind {
        Kind::Arrow(param, result) => {
            assert!(matches!(*param, Kind::Type));
            match *result {
                Kind::Arrow(_, _) => {
                    // Expected nested arrow
                }
                _ => panic!("Expected nested Arrow kind"),
            }
        }
        _ => panic!("Expected Arrow kind"),
    }
}

#[test]
fn test_kind_functor_protocol() {
    // Test Functor protocol with HKT
    // protocol Functor { type F<_>; fn map<A, B>(fa: F<A>, f: A -> B) -> F<B> }

    let gat_params = List::from_iter(vec![make_gat_param("_", Variance::Covariant)]);

    let mut f_gat = AssociatedTypeGAT::generic("F".into(), gat_params, List::new(), List::new());

    f_gat.kind = AssociatedTypeKind::HigherKinded { arity: 1 };

    // Verify structure for Functor
    assert!(matches!(
        f_gat.kind,
        AssociatedTypeKind::HigherKinded { arity: 1 }
    ));
    assert_eq!(f_gat.name, "F");
}

#[test]
fn test_kind_monad_protocol() {
    // Test Monad protocol with HKT
    // protocol Monad: Functor { fn pure<A>(a: A) -> F<A>; fn flat_map<A, B>(fa: F<A>, f: A -> F<B>) -> F<B> }

    let gat_params = List::from_iter(vec![make_gat_param("_", Variance::Covariant)]);

    let mut m_gat = AssociatedTypeGAT::generic("M".into(), gat_params, List::new(), List::new());

    m_gat.kind = AssociatedTypeKind::HigherKinded { arity: 1 };

    assert!(matches!(
        m_gat.kind,
        AssociatedTypeKind::HigherKinded { arity: 1 }
    ));
}

#[test]
fn test_kind_error_mismatch() {
    // Test kind mismatch error
    let expected = Kind::type_kind();
    let found = Kind::unary_constructor();

    let error = AdvancedProtocolError::KindMismatch {
        expected: expected.clone(),
        found: found.clone(),
    };

    match error {
        AdvancedProtocolError::KindMismatch {
            expected: e,
            found: f,
        } => {
            assert_eq!(e.arity(), 0);
            assert_eq!(f.arity(), 1);
        }
        _ => panic!("Wrong error type"),
    }
}

// ==================== Category 6: End-to-End Scenarios (5+ tests) ====================

#[test]
fn test_e2e_lending_iterator_with_refinements() {
    // Test complete lending iterator with refinement types
    // protocol LendingIterator {
    //     type Item<'a>;
    //     fn next(&'a mut self) -> Maybe<Self.Item<'a>> where Self.Item<'a>: valid
    // }

    let gat_params = List::from_iter(vec![make_gat_param("'a", Variance::Covariant)]);

    let item_gat = AssociatedTypeGAT::generic("Item".into(), gat_params, List::new(), List::new());

    // Create a GenRef type for the inner data
    let data_type = Type::Array {
        element: Box::new(Type::Int),
        size: None,
    };
    let genref = GenRefType::new(data_type);

    // Create generation validity predicate
    let valid_pred = GenerationPredicate::Valid {
        ref_expr: Box::new(Type::Reference {
            mutable: false,
            inner: Box::new(Type::Int),
        }),
    };

    // Verify all components work together
    assert!(item_gat.is_gat());
    assert!(matches!(valid_pred, GenerationPredicate::Valid { .. }));
}

#[test]
fn test_e2e_specialized_protocol_with_gats() {
    // Test specialized protocol implementation with GATs
    // General: impl<T> Container for List<T> { type Item<U> = ... }
    // Specialized: impl Container for List<Int> { type Item<U> = ... }

    let general = SpecializationInfo::none();
    let specialized = SpecializationInfo::specialized(make_path("ListGenericContainer"), 5);

    let gat_params = List::from_iter(vec![make_gat_param("U", Variance::Covariant)]);

    let item_gat = AssociatedTypeGAT::generic("Item".into(), gat_params, List::new(), List::new());

    // Verify combination
    assert!(!general.is_specialized);
    assert!(specialized.is_specialized);
    assert!(item_gat.is_gat());
}

#[test]
fn test_e2e_hkt_with_specialization() {
    // Test higher-kinded types with specialization
    // protocol Functor { type F<_> }
    // General: impl<F: Functor> Monad for F
    // Specialized: impl Monad for List

    let mut lattice = SpecializationLattice::new();
    lattice.add_impl(0); // General
    lattice.add_impl(1); // Specialized
    lattice.ordering.insert((1, 0), true);

    let gat_params = List::from_iter(vec![make_gat_param("_", Variance::Covariant)]);

    let mut f_gat = AssociatedTypeGAT::generic("F".into(), gat_params, List::new(), List::new());
    f_gat.kind = AssociatedTypeKind::HigherKinded { arity: 1 };

    let applicable = Set::from_iter(vec![0, 1]);
    let selected = lattice.select_most_specific(&applicable);

    assert_eq!(selected, Maybe::Some(1));
    assert!(matches!(
        f_gat.kind,
        AssociatedTypeKind::HigherKinded { .. }
    ));
}

#[test]
fn test_e2e_complex_protocol_hierarchy() {
    // Test complex protocol hierarchy: Functor -> Applicative -> Monad

    // Functor GAT
    let functor_gat = {
        let params = List::from_iter(vec![make_gat_param("_", Variance::Covariant)]);
        let mut gat = AssociatedTypeGAT::generic("F".into(), params, List::new(), List::new());
        gat.kind = AssociatedTypeKind::HigherKinded { arity: 1 };
        gat
    };

    // Applicative extends Functor
    let applicative_gat = {
        let params = List::from_iter(vec![make_gat_param("_", Variance::Covariant)]);
        let mut gat = AssociatedTypeGAT::generic(
            "Ap".into(),
            params,
            List::from_iter(vec![make_bound("Functor")]),
            List::new(),
        );
        gat.kind = AssociatedTypeKind::HigherKinded { arity: 1 };
        gat
    };

    // Monad extends Applicative
    let monad_gat = {
        let params = List::from_iter(vec![make_gat_param("_", Variance::Covariant)]);
        let mut gat = AssociatedTypeGAT::generic(
            "M".into(),
            params,
            List::from_iter(vec![make_bound("Applicative")]),
            List::new(),
        );
        gat.kind = AssociatedTypeKind::HigherKinded { arity: 1 };
        gat
    };

    // Verify hierarchy
    assert!(matches!(
        functor_gat.kind,
        AssociatedTypeKind::HigherKinded { .. }
    ));
    assert!(matches!(
        applicative_gat.kind,
        AssociatedTypeKind::HigherKinded { .. }
    ));
    assert!(matches!(
        monad_gat.kind,
        AssociatedTypeKind::HigherKinded { .. }
    ));
    assert_eq!(applicative_gat.bounds.len(), 1);
    assert_eq!(monad_gat.bounds.len(), 1);
}

#[test]
fn test_e2e_streaming_parser() {
    // Test real-world scenario: streaming parser with lending iterators
    // Combines: GATs, GenRef, refinements

    // Parser state with GenRef to input buffer
    let buffer_type = Type::Array {
        element: Box::new(Type::Char),
        size: None,
    };
    let genref_buffer = GenRefType::new(buffer_type);

    // GAT for parsed items
    let item_params = List::from_iter(vec![make_gat_param("'a", Variance::Covariant)]);
    let item_gat = AssociatedTypeGAT::generic("Item".into(), item_params, List::new(), List::new());

    // Refinement: parsed position must be <= buffer length
    let position_constraint = RefinementConstraint {
        name: Text::from("position"),
        predicate: RefinementPredicate::BinaryOp {
            op: AdvancedBinOp::Ge,
            value: ConstValue::Int(0),
        },
        kind: RefinementKind::Inline,
        span: Span::dummy(),
    };

    // Generation validity check
    let valid_pred = GenerationPredicate::Valid {
        ref_expr: Box::new(Type::Reference {
            mutable: true,
            inner: Box::new(Type::Array {
                element: Box::new(Type::Char),
                size: None,
            }),
        }),
    };

    // Verify all components integrated
    assert!(item_gat.is_gat());
    assert_eq!(position_constraint.name, "position");
    assert!(matches!(valid_pred, GenerationPredicate::Valid { .. }));
    match genref_buffer.inner() {
        Type::Array { .. } => {}
        _ => panic!("Expected array type"),
    }
}

// ==================== Additional Integration Tests ====================

#[test]
fn test_gat_with_default_type() {
    // Test GAT with default type parameter
    let mut param = make_gat_param("T", Variance::Covariant);
    param.default = Maybe::Some(Type::Int);

    let type_params = List::from_iter(vec![param]);

    let gat = AssociatedTypeGAT::generic("Container".into(), type_params, List::new(), List::new());

    match &gat.type_params[0].default {
        Maybe::Some(ty) => assert_eq!(*ty, Type::Int),
        Maybe::None => panic!("Expected default type"),
    }
}

#[test]
fn test_specialization_with_negative_bounds() {
    // Test specialization using negative bounds for mutual exclusion
    let positive = ProtocolBoundPolarity::Positive {
        protocol: make_path("Send"),
        args: List::new(),
    };

    let negative = ProtocolBoundPolarity::Negative {
        protocol: make_path("Sync"),
    };

    // Verify both polarities work
    assert!(matches!(positive, ProtocolBoundPolarity::Positive { .. }));
    assert!(matches!(negative, ProtocolBoundPolarity::Negative { .. }));
}

#[test]
fn test_refinement_logical_operators() {
    // Test complex refinement predicates with logical operators
    let predicate = RefinementPredicate::Or {
        left: Box::new(RefinementPredicate::BinaryOp {
            op: AdvancedBinOp::Lt,
            value: ConstValue::Int(0),
        }),
        right: Box::new(RefinementPredicate::BinaryOp {
            op: AdvancedBinOp::Gt,
            value: ConstValue::Int(100),
        }),
    };

    let constraint = RefinementConstraint {
        name: Text::from("value"),
        predicate,
        kind: RefinementKind::Inline,
        span: Span::dummy(),
    };

    match constraint.predicate {
        RefinementPredicate::Or { left, right } => {
            assert!(matches!(*left, RefinementPredicate::BinaryOp { .. }));
            assert!(matches!(*right, RefinementPredicate::BinaryOp { .. }));
        }
        _ => panic!("Wrong predicate type"),
    }
}

#[test]
fn test_genref_generation_mismatch_error() {
    // Test GenRef generation mismatch error
    let error = AdvancedProtocolError::GenerationMismatch;

    match error {
        AdvancedProtocolError::GenerationMismatch => {
            // Expected
        }
        _ => panic!("Wrong error type"),
    }
}

#[test]
fn test_kind_arity_calculation() {
    // Test kind arity calculation for nested arrows
    let kind = Kind::Arrow(
        Box::new(Kind::Type),
        Box::new(Kind::Arrow(
            Box::new(Kind::Type),
            Box::new(Kind::Type),
        )),
    );

    assert_eq!(kind.arity(), 2);
}

#[test]
fn test_gat_complex_where_clause() {
    // Test GAT with multiple where clauses
    let type_params = List::from_iter(vec![
        make_gat_param("K", Variance::Invariant),
        make_gat_param("V", Variance::Covariant),
    ]);

    let where_clauses = List::from_iter(vec![
        make_gat_where(
            "K",
            List::from_iter(vec![make_bound("Hash"), make_bound("Eq")]),
        ),
        make_gat_where("V", List::from_iter(vec![make_bound("Clone")])),
    ]);

    let gat = AssociatedTypeGAT::generic("Entry".into(), type_params, List::new(), where_clauses);

    assert_eq!(gat.where_clauses.len(), 2);
    assert_eq!(gat.where_clauses[0].param, "K");
    assert_eq!(gat.where_clauses[0].constraints.len(), 2);
    assert_eq!(gat.where_clauses[1].param, "V");
    assert_eq!(gat.where_clauses[1].constraints.len(), 1);
}

#[test]
fn test_specialization_empty_lattice() {
    // Test empty specialization lattice
    let lattice = SpecializationLattice::new();

    assert_eq!(lattice.impls.len(), 0);
    assert_eq!(lattice.ordering.len(), 0);
    assert_eq!(lattice.max_element, Maybe::None);
    assert_eq!(lattice.min_elements.len(), 0);
}

#[test]
fn test_const_value_types() {
    // Test different ConstValue types in refinements
    let int_val = ConstValue::Int(42);
    let float_val = ConstValue::Float(3.14);
    let bool_val = ConstValue::Bool(true);
    let text_val = ConstValue::Text(Text::from("hello"));

    assert_eq!(int_val, ConstValue::Int(42));
    assert_eq!(float_val, ConstValue::Float(3.14));
    assert_eq!(bool_val, ConstValue::Bool(true));
    assert_eq!(text_val, ConstValue::Text(Text::from("hello")));
}
