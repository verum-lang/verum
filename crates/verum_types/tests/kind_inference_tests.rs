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
//! Comprehensive Kind Inference Tests
//!
//! Higher-kinded type (HKT) kind inference: infers kinds for type constructors
//! (e.g., List has kind Type -> Type, Map has kind Type -> Type -> Type).
//! Uses constraint-based kind inference with unification.
//!
//! Tests cover:
//! 1. Basic kind inference (primitives, named types)
//! 2. Type constructor kinds (List, Map, etc.)
//! 3. Kind unification
//! 4. Kind checking
//! 5. Protocol GAT kind inference
//! 6. Higher-kinded type parameters
//! 7. Kind errors and diagnostics
//! 8. Edge cases and complex scenarios

use verum_ast::{
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{List, Map, Maybe, Text};
use verum_types::{
    TypeChecker,
    advanced_protocols::{GATTypeParam, Variance},
    kind_inference::KindInference,
    kind_inference::{Kind, KindConstraint, KindError, KindInferer, KindSubstitution},
    protocol::{AssociatedType, Protocol, ProtocolBound},
    ty::Type,
};

// Helper to create a simple path
fn simple_path(name: &str) -> Path {
    Path::single(Ident::new(name, Span::default()))
}

// Helper to create a named type
fn named_type(name: &str, args: impl Into<List<Type>>) -> Type {
    Type::Named {
        path: simple_path(name),
        args: args.into(),
    }
}

// ==================== Basic Kind Inference Tests ====================

#[test]
fn test_kind_display() {
    assert_eq!(Kind::Type.to_string(), "*");
    assert_eq!(Kind::unary_constructor().to_string(), "* -> *");
    assert_eq!(Kind::binary_constructor().to_string(), "* -> * -> *");
    assert_eq!(Kind::KindVar(0).to_string(), "?k0");
}

#[test]
fn test_kind_arity() {
    assert_eq!(Kind::Type.arity(), 0);
    assert_eq!(Kind::unary_constructor().arity(), 1);
    assert_eq!(Kind::binary_constructor().arity(), 2);

    // Nested arrow: (* -> *) -> *
    let higher_order = Kind::arrow(Kind::unary_constructor(), Kind::Type);
    assert_eq!(higher_order.arity(), 1);
}

#[test]
fn test_kind_apply_n() {
    let kind = Kind::binary_constructor(); // * -> * -> *

    assert_eq!(kind.apply_n(0), Maybe::Some(kind.clone()));
    assert_eq!(kind.apply_n(1), Maybe::Some(Kind::unary_constructor()));
    assert_eq!(kind.apply_n(2), Maybe::Some(Kind::Type));
    assert_eq!(kind.apply_n(3), Maybe::None);
}

#[test]
fn test_kind_free_vars() {
    let k1 = Kind::KindVar(0);
    let k2 = Kind::arrow(Kind::KindVar(1), Kind::Type);
    let k3 = Kind::arrow(Kind::KindVar(0), Kind::KindVar(1));

    assert_eq!(k1.free_vars().len(), 1);
    assert!(k1.free_vars().contains(&0));

    assert_eq!(k2.free_vars().len(), 1);
    assert!(k2.free_vars().contains(&1));

    assert_eq!(k3.free_vars().len(), 2);
    assert!(k3.free_vars().contains(&0));
    assert!(k3.free_vars().contains(&1));
}

#[test]
fn test_kind_is_concrete() {
    assert!(Kind::Type.is_concrete());
    assert!(Kind::unary_constructor().is_concrete());
    assert!(!Kind::KindVar(0).is_concrete());

    let mixed = Kind::arrow(Kind::KindVar(0), Kind::Type);
    assert!(!mixed.is_concrete());
}

// ==================== Kind Substitution Tests ====================

#[test]
fn test_kind_substitution_basic() {
    let mut subst = KindSubstitution::new();
    subst.insert(0, Kind::Type);
    subst.insert(1, Kind::unary_constructor());

    let kind = Kind::KindVar(0);
    assert_eq!(kind.apply(&subst), Kind::Type);

    let kind = Kind::KindVar(1);
    assert_eq!(kind.apply(&subst), Kind::unary_constructor());

    let kind = Kind::KindVar(2);
    assert_eq!(kind.apply(&subst), Kind::KindVar(2));
}

#[test]
fn test_kind_substitution_arrow() {
    let mut subst = KindSubstitution::new();
    subst.insert(0, Kind::Type);

    let kind = Kind::arrow(Kind::KindVar(0), Kind::KindVar(0));
    let result = kind.apply(&subst);

    assert_eq!(result, Kind::arrow(Kind::Type, Kind::Type));
}

#[test]
fn test_kind_substitution_compose() {
    let mut s1 = KindSubstitution::new();
    s1.insert(0, Kind::KindVar(1));

    let mut s2 = KindSubstitution::new();
    s2.insert(1, Kind::Type);

    let composed = s1.compose(&s2);

    // s1[0] = ?k1, s2[?k1] = *, so composed[0] = *
    assert_eq!(composed.get(&0), Maybe::Some(&Kind::Type));
}

// ==================== Kind Inference Tests ====================

#[test]
fn test_infer_primitive_kinds() {
    let mut inferer = KindInferer::new();

    assert_eq!(inferer.infer_kind(&Type::Int).unwrap(), Kind::Type);
    assert_eq!(inferer.infer_kind(&Type::Bool).unwrap(), Kind::Type);
    assert_eq!(inferer.infer_kind(&Type::Text).unwrap(), Kind::Type);
    assert_eq!(inferer.infer_kind(&Type::Float).unwrap(), Kind::Type);
    assert_eq!(inferer.infer_kind(&Type::Char).unwrap(), Kind::Type);
    assert_eq!(inferer.infer_kind(&Type::Unit).unwrap(), Kind::Type);
}

#[test]
fn test_infer_type_variable_kind() {
    let mut inferer = KindInferer::new();

    let ty = Type::Var(verum_types::ty::TypeVar::with_id(0));
    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);
}

#[test]
fn test_infer_list_kind() {
    let mut inferer = KindInferer::new();

    // List<Int> has kind *
    let ty = named_type("List", List::from(vec![Type::Int]));
    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);
}

#[test]
fn test_infer_map_kind() {
    let mut inferer = KindInferer::new();

    // Map<Text, Int> has kind *
    let ty = named_type("Map", List::from(vec![Type::Text, Type::Int]));
    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);
}

#[test]
fn test_infer_nested_type_constructor() {
    let mut inferer = KindInferer::new();

    // List<List<Int>> has kind *
    let inner = named_type("List", List::from(vec![Type::Int]));
    let ty = named_type("List", vec![inner]);

    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);
}

#[test]
fn test_infer_function_kind() {
    let mut inferer = KindInferer::new();

    let ty = Type::Function {
        params: vec![Type::Int, Type::Text].into(),
        return_type: Box::new(Type::Bool),
        contexts: None,
        type_params: vec![].into(),
        properties: None,
    };

    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);
}

#[test]
fn test_infer_tuple_kind() {
    let mut inferer = KindInferer::new();

    let ty = Type::Tuple(vec![Type::Int, Type::Text, Type::Bool].into());
    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);
}

#[test]
fn test_infer_reference_kind() {
    let mut inferer = KindInferer::new();

    let ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Int),
    };
    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);

    let ty = Type::CheckedReference {
        mutable: true,
        inner: Box::new(Type::Text),
    };
    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);

    let ty = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(Type::Bool),
    };
    assert_eq!(inferer.infer_kind(&ty).unwrap(), Kind::Type);
}

// ==================== Kind Unification Tests ====================

#[test]
fn test_unify_concrete_kinds() {
    let mut inferer = KindInferer::new();

    // Unify * with *
    let result = inferer.unify(&Kind::Type, &Kind::Type, Span::default(), "test".into());
    assert!(result.is_ok());

    // Unify * -> * with * -> *
    let result = inferer.unify(
        &Kind::unary_constructor(),
        &Kind::unary_constructor(),
        Span::default(),
        "test".into(),
    );
    assert!(result.is_ok());
}

#[test]
fn test_unify_kind_variables() {
    let mut inferer = KindInferer::new();

    // Unify ?k0 with *
    let result = inferer.unify(
        &Kind::KindVar(0),
        &Kind::Type,
        Span::default(),
        "test".into(),
    );
    assert!(result.is_ok());
    let subst = result.unwrap();
    assert_eq!(subst.get(&0), Maybe::Some(&Kind::Type));

    // Unify ?k1 with ?k2
    let result = inferer.unify(
        &Kind::KindVar(1),
        &Kind::KindVar(2),
        Span::default(),
        "test".into(),
    );
    assert!(result.is_ok());
}

#[test]
fn test_unify_arrow_kinds() {
    let mut inferer = KindInferer::new();

    let k1 = Kind::arrow(Kind::Type, Kind::KindVar(0));
    let k2 = Kind::arrow(Kind::Type, Kind::Type);

    let result = inferer.unify(&k1, &k2, Span::default(), "test".into());
    assert!(result.is_ok());

    let subst = result.unwrap();
    assert_eq!(subst.get(&0), Maybe::Some(&Kind::Type));
}

#[test]
fn test_unify_mismatch() {
    let mut inferer = KindInferer::new();

    // Try to unify * with * -> *
    let result = inferer.unify(
        &Kind::Type,
        &Kind::unary_constructor(),
        Span::default(),
        "test".into(),
    );
    assert!(result.is_err());
}

#[test]
fn test_occurs_check() {
    let mut inferer = KindInferer::new();

    // Create infinite kind: ?k0 = ?k0 -> *
    let infinite_kind = Kind::Arrow(Box::new(Kind::KindVar(0)), Box::new(Kind::Type));

    let result = inferer.bind_kind_var(0, infinite_kind, Span::default());
    assert!(result.is_err());
}

// ==================== Kind Checking Tests ====================

#[test]
fn test_check_kind_success() {
    let mut inferer = KindInferer::new();

    // Check that Int has kind *
    let result = inferer.check_kind(&Type::Int, &Kind::Type);
    assert!(result.is_ok());

    // Check that List<Int> has kind *
    let ty = named_type("List", List::from(vec![Type::Int]));
    let result = inferer.check_kind(&ty, &Kind::Type);
    assert!(result.is_ok());
}

#[test]
fn test_check_kind_failure() {
    let mut inferer = KindInferer::new();

    // Try to check that Int has kind * -> * (should fail)
    let result = inferer.check_kind(&Type::Int, &Kind::unary_constructor());
    assert!(result.is_err());
}

// ==================== Arity Mismatch Tests ====================

#[test]
fn test_arity_mismatch_too_few_args() {
    let mut inferer = KindInferer::new();

    // Map expects 2 args, but we provide 1
    let ty = named_type("Map", List::from(vec![Type::Int]));
    let result = inferer.infer_kind(&ty);

    assert!(result.is_err());
}

#[test]
fn test_arity_mismatch_too_many_args() {
    let mut inferer = KindInferer::new();

    // List expects 1 arg, but we provide 2
    let ty = named_type("List", List::from(vec![Type::Int, Type::Text]));
    let result = inferer.infer_kind(&ty);

    assert!(result.is_err());
}

#[test]
fn test_arity_correct() {
    let mut inferer = KindInferer::new();

    // List with 1 arg (correct)
    let ty = named_type("List", List::from(vec![Type::Int]));
    let result = inferer.infer_kind(&ty);
    assert!(result.is_ok());

    // Map with 2 args (correct)
    let ty = named_type("Map", List::from(vec![Type::Text, Type::Int]));
    let result = inferer.infer_kind(&ty);
    assert!(result.is_ok());
}

// ==================== Protocol Kind Tests ====================

#[test]
fn test_protocol_regular_associated_type() {
    let mut inferer = KindInferer::new();

    // Create protocol with regular associated type
    let mut associated_types: Map<Text, _> = Map::new();
    associated_types.insert(
        "Item".into(),
        AssociatedType::simple("Item".into(), vec![].into()),
    );

    let protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Iterator".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types,
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("test".into()),
        span: Span::default(),
    };

    let result = inferer.check_protocol_kinds(&protocol);
    assert!(result.is_ok());
}

#[test]
fn test_protocol_gat_associated_type() {
    let mut inferer = KindInferer::new();

    // Create protocol with GAT
    let type_params: List<GATTypeParam> = vec![GATTypeParam {
        name: "T".into(),
        bounds: vec![].into(),
        default: Maybe::None,
        variance: Variance::Covariant,
    }]
    .into();

    let mut associated_types: Map<Text, _> = Map::new();
    associated_types.insert(
        "Item".into(),
        AssociatedType::generic("Item".into(), type_params, vec![].into(), vec![].into()),
    );

    let protocol = Protocol {
        kind: verum_types::protocol::ProtocolKind::Constraint,
        name: "Collection".into(),
        type_params: vec![].into(),
        methods: Map::new(),
        associated_types,
        associated_consts: Map::new(),
        super_protocols: vec![].into(),
        specialization_info: Maybe::None,
        defining_crate: Maybe::Some("test".into()),
        span: Span::default(),
    };

    let result = inferer.check_protocol_kinds(&protocol);
    assert!(result.is_ok());
}

// ==================== TypeChecker Integration Tests ====================

#[test]
fn test_typechecker_kind_inference() {
    let mut checker = TypeChecker::new();

    // Infer kind of Int
    let kind = checker.infer_kind(&Type::Int).unwrap();
    assert_eq!(kind, Kind::Type);

    // Infer kind of List<Int>
    let ty = named_type("List", List::from(vec![Type::Int]));
    let kind = checker.infer_kind(&ty).unwrap();
    assert_eq!(kind, Kind::Type);
}

#[test]
fn test_typechecker_check_kind() {
    let mut checker = TypeChecker::new();

    // Check that Int has kind *
    let result = checker.check_kind(&Type::Int, &Kind::Type);
    assert!(result.is_ok());

    // Check that List<Int> has kind *
    let ty = named_type("List", List::from(vec![Type::Int]));
    let result = checker.check_kind(&ty, &Kind::Type);
    assert!(result.is_ok());
}

// ==================== Higher-Kinded Type Tests ====================

#[test]
fn test_higher_kinded_functor() {
    let mut inferer = KindInferer::new();

    // Register Functor.F as a higher-kinded type (* -> *)
    inferer.register_type_constructor(Text::from("F"), Kind::unary_constructor());

    // F<Int> should have kind *
    let ty = named_type("F", List::from(vec![Type::Int]));
    let kind = inferer.infer_kind(&ty).unwrap();
    assert_eq!(kind, Kind::Type);
}

#[test]
fn test_higher_kinded_nested() {
    let mut inferer = KindInferer::new();

    // F has kind * -> *
    inferer.register_type_constructor(Text::from("F"), Kind::unary_constructor());

    // F<List<Int>> should have kind *
    let inner = named_type("List", List::from(vec![Type::Int]));
    let ty = named_type("F", vec![inner]);
    let kind = inferer.infer_kind(&ty).unwrap();
    assert_eq!(kind, Kind::Type);
}

// ==================== Error Message Tests ====================

#[test]
fn test_kind_error_suggestions() {
    use verum_types::kind_inference::KindError;

    let err = KindError::ArityMismatch {
        constructor: "List".into(),
        expected: 1,
        found: 2,
        span: Span::default(),
    };

    let suggestion = err.with_suggestion();
    assert!(suggestion.contains("List"));
    assert!(suggestion.contains("expects 1"));
}

// ==================== Complex Scenarios ====================

#[test]
fn test_deeply_nested_types() {
    let mut inferer = KindInferer::new();

    // List<Map<Text, List<Int>>>
    let inner_list = named_type("List", List::from(vec![Type::Int]));
    let map = named_type("Map", List::from(vec![Type::Text, inner_list]));
    let outer_list = named_type("List", vec![map]);

    let kind = inferer.infer_kind(&outer_list).unwrap();
    assert_eq!(kind, Kind::Type);
}

#[test]
fn test_multiple_type_constructors() {
    let mut inferer = KindInferer::new();

    // Test all standard library constructors
    let constructors = vec![
        ("List", 1),
        ("Maybe", 1),
        ("Set", 1),
        ("Map", 2),
        ("Result", 2),
        ("GenRef", 1),
        ("Shared", 1),
        ("Heap", 1),
    ];

    for (name, arity) in constructors {
        let args: List<Type> = (0..arity).map(|_| Type::Int).collect();
        let ty = named_type(name, args);

        let kind = inferer.infer_kind(&ty);
        assert!(kind.is_ok(), "Failed to infer kind for {}", name);
        assert_eq!(kind.unwrap(), Kind::Type);
    }
}

#[test]
fn test_kind_inference_with_refinements() {
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::literal::{Literal, LiteralKind};
    use verum_ast::span::Span;
    use verum_types::refinement::{RefinementBinding, RefinementPredicate};

    let mut inferer = KindInferer::new();

    // Int{> 0} should have kind *
    let predicate = RefinementPredicate {
        predicate: Expr {
            kind: ExprKind::Literal(Literal::new(LiteralKind::Bool(true), Span::dummy())),
            span: Span::dummy(),
            ref_kind: None,
            check_eliminated: false,
        },
        binding: RefinementBinding::Inline,
        span: Span::dummy(),
    };

    let ty = Type::Refined {
        base: Box::new(Type::Int),
        predicate,
    };

    let kind = inferer.infer_kind(&ty).unwrap();
    assert_eq!(kind, Kind::Type);
}

#[test]
fn test_constraint_solving() {
    let mut inferer = KindInferer::new();

    // Add some constraints
    inferer.add_constraint(KindConstraint::equal(
        Kind::KindVar(0),
        Kind::Type,
        Span::default(),
        "test constraint 1",
    ));

    inferer.add_constraint(KindConstraint::equal(
        Kind::KindVar(1),
        Kind::unary_constructor(),
        Span::default(),
        "test constraint 2",
    ));

    // Solve constraints
    let result = inferer.solve();
    assert!(result.is_ok());

    let subst = result.unwrap();
    assert_eq!(subst.get(&0), Maybe::Some(&Kind::Type));
    assert_eq!(subst.get(&1), Maybe::Some(&Kind::unary_constructor()));
}

#[test]
fn test_fresh_kind_vars() {
    let mut inferer = KindInferer::new();

    let k1 = inferer.fresh_kind_var();
    let k2 = inferer.fresh_kind_var();
    let k3 = inferer.fresh_kind_var();

    // Each should be distinct
    assert_ne!(k1, k2);
    assert_ne!(k2, k3);
    assert_ne!(k1, k3);

    // All should be kind variables
    assert!(matches!(k1, Kind::KindVar(_)));
    assert!(matches!(k2, Kind::KindVar(_)));
    assert!(matches!(k3, Kind::KindVar(_)));
}

#[test]
fn test_well_kindedness() {
    let mut inferer = KindInferer::new();

    // Well-kinded types
    assert!(inferer.is_well_kinded(&Type::Int));
    assert!(inferer.is_well_kinded(&named_type("List", List::from(vec![Type::Int]))));
    assert!(inferer.is_well_kinded(&named_type("Map", List::from(vec![Type::Text, Type::Int]))));

    // Ill-kinded types (wrong arity)
    assert!(!inferer.is_well_kinded(&named_type("List", vec![])));
    assert!(!inferer.is_well_kinded(&named_type("Map", List::from(vec![Type::Int]))));
}

// ==================== Performance Tests ====================

#[test]
fn test_kind_inference_performance() {
    use std::time::Instant;

    let mut inferer = KindInferer::new();

    // Create a complex nested type
    let mut ty = Type::Int;
    for _ in 0..10 {
        ty = named_type("List", vec![ty]);
    }

    let start = Instant::now();
    let result = inferer.infer_kind(&ty);
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    // Allow more time for debug builds - 500µs is reasonable for 10-level nested type
    assert!(
        elapsed.as_micros() < 500,
        "Kind inference too slow: {:?}",
        elapsed
    );
}

#[test]
fn test_constraint_solving_performance() {
    use std::time::Instant;

    let mut inferer = KindInferer::new();

    // Add many constraints
    for i in 0..100 {
        inferer.add_constraint(KindConstraint::equal(
            Kind::KindVar(i),
            if i % 2 == 0 {
                Kind::Type
            } else {
                Kind::unary_constructor()
            },
            Span::default(),
            format!("constraint {}", i),
        ));
    }

    let start = Instant::now();
    let result = inferer.solve();
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    assert!(
        elapsed.as_millis() < 10,
        "Constraint solving too slow: {:?}",
        elapsed
    );
}

// ==================== HKT Instantiation Tests (HKT instantiation) ====================

#[test]
fn test_check_type_application_kind_unary() {
    let mut inferer = KindInferer::new();

    // Register List as having kind * -> *
    inferer.register_type_constructor("List", Kind::unary_constructor());

    // Create a type constructor for List
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    // Check List<Int> - should produce kind *
    let result = inferer.check_type_application_kind(&list_ctor, &[Type::Int], Span::default());
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), Kind::Type);
}

#[test]
fn test_check_type_application_kind_binary() {
    let mut inferer = KindInferer::new();

    // Register Map as having kind * -> * -> *
    inferer.register_type_constructor("Map", Kind::binary_constructor());

    // Create a type constructor for Map
    let map_ctor = Type::TypeConstructor {
        name: "Map".into(),
        arity: 2,
        kind: Kind::binary_constructor(),
    };

    // Check Map<Text, Int> - should produce kind *
    let result = inferer.check_type_application_kind(&map_ctor, &[Type::Text, Type::Int], Span::default());
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), Kind::Type);
}

#[test]
fn test_check_type_application_kind_arity_mismatch() {
    let mut inferer = KindInferer::new();

    // Create a type constructor for List (* -> *)
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    // Check List<Int, Bool> - arity mismatch, should fail
    let result = inferer.check_type_application_kind(&list_ctor, &[Type::Int, Type::Bool], Span::default());
    assert!(result.is_err());
}

#[test]
fn test_check_type_application_kind_with_nested_constructor() {
    let mut inferer = KindInferer::new();

    // Register List and Maybe as type constructors
    inferer.register_type_constructor("List", Kind::unary_constructor());
    inferer.register_type_constructor("Maybe", Kind::unary_constructor());

    // Create a type constructor for List (* -> *)
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    // Create a fully applied Maybe<Int> - this has kind *
    let maybe_int = Type::make_type_app(
        Type::TypeConstructor {
            name: "Maybe".into(),
            arity: 1,
            kind: Kind::unary_constructor(),
        },
        List::from(vec![Type::Int]),
    );

    // Check List<Maybe<Int>> - should succeed because Maybe<Int> has kind *
    let result = inferer.check_type_application_kind(&list_ctor, &[maybe_int], Span::default());
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), Kind::Type);
}

#[test]
fn test_instantiate_hkt_param_success() {
    let mut inferer = KindInferer::new();

    // Create a type constructor for List with kind * -> *
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    // Expected kind for the HKT parameter F
    let expected_kind = Kind::unary_constructor();

    // No protocol bounds for this test
    let bounds: Vec<ProtocolBound> = vec![];

    // A protocol checker that always returns true (for testing)
    let check_protocol = |_: &Type, _: &ProtocolBound| true;

    // Instantiate F<_> with List
    let result = inferer.instantiate_hkt_param(
        "F",
        &expected_kind,
        &list_ctor,
        &bounds,
        Span::default(),
        check_protocol,
    );

    assert!(result.is_ok());
    let result = result.unwrap();
    assert!(result.protocol_bounds_satisfied);
    assert_eq!(result.resulting_kind, Kind::unary_constructor());
}

#[test]
fn test_instantiate_hkt_param_kind_mismatch() {
    let mut inferer = KindInferer::new();

    // Create a type constructor for List with kind * -> *
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    // Expected kind for the HKT parameter is * -> * -> * (binary)
    let expected_kind = Kind::binary_constructor();

    let bounds: Vec<ProtocolBound> = vec![];
    let check_protocol = |_: &Type, _: &ProtocolBound| true;

    // Try to instantiate - should fail due to kind mismatch
    let result = inferer.instantiate_hkt_param(
        "F",
        &expected_kind,
        &list_ctor,
        &bounds,
        Span::default(),
        check_protocol,
    );

    assert!(result.is_err());
}

#[test]
fn test_instantiate_hkt_param_protocol_not_satisfied() {
    let mut inferer = KindInferer::new();

    // Create a type constructor for List with kind * -> *
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    let expected_kind = Kind::unary_constructor();

    // Protocol bound that List must implement Functor
    let functor_bound = ProtocolBound::simple(simple_path("Functor"));

    // A protocol checker that returns false (for testing protocol violation)
    let check_protocol = |_: &Type, _: &ProtocolBound| false;

    let result = inferer.instantiate_hkt_param(
        "F",
        &expected_kind,
        &list_ctor,
        &[functor_bound],
        Span::default(),
        check_protocol,
    );

    // Kind check passes, but protocol_bounds_satisfied should be false
    assert!(result.is_ok());
    let result = result.unwrap();
    assert!(!result.protocol_bounds_satisfied);
}

#[test]
fn test_check_constructor_protocol_compatibility() {
    let mut inferer = KindInferer::new();

    // Create a type constructor for List with kind * -> *
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    // Functor requires * -> *
    let result = inferer.check_constructor_protocol_compatibility(
        &list_ctor,
        "Functor",
        &Kind::unary_constructor(),
        Span::default(),
    );

    assert!(result.is_ok());
}

#[test]
fn test_check_constructor_protocol_incompatible_kind() {
    let mut inferer = KindInferer::new();

    // Create a type constructor for List with kind * -> *
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    // Bifunctor requires * -> * -> *, but List has * -> *
    let result = inferer.check_constructor_protocol_compatibility(
        &list_ctor,
        "Bifunctor",
        &Kind::binary_constructor(),
        Span::default(),
    );

    assert!(result.is_err());
}

#[test]
fn test_is_type_constructor() {
    let mut inferer = KindInferer::new();

    // List is a type constructor (* -> *)
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };
    assert!(inferer.is_type_constructor(&list_ctor));

    // Int is not a type constructor (kind *)
    assert!(!inferer.is_type_constructor(&Type::Int));
}

#[test]
fn test_get_constructor_arity() {
    let mut inferer = KindInferer::new();

    // List has arity 1
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };
    assert_eq!(inferer.get_constructor_arity(&list_ctor), Maybe::Some(1));

    // Map has arity 2
    let map_ctor = Type::TypeConstructor {
        name: "Map".into(),
        arity: 2,
        kind: Kind::binary_constructor(),
    };
    assert_eq!(inferer.get_constructor_arity(&map_ctor), Maybe::Some(2));

    // Int has no arity (not a constructor)
    assert_eq!(inferer.get_constructor_arity(&Type::Int), Maybe::None);
}

// ==================== Type Helper Tests ====================

#[test]
fn test_type_make_type_app() {
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    let list_int = Type::make_type_app(list_ctor, List::from(vec![Type::Int]));

    assert!(list_int.is_type_app());
}

#[test]
fn test_type_is_fully_applied() {
    // Fully applied: List<Int>
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };
    let list_int = Type::make_type_app(list_ctor.clone(), List::from(vec![Type::Int]));
    assert!(list_int.is_fully_applied());

    // Not fully applied: List (no args)
    assert!(!list_ctor.is_fully_applied());

    // Concrete type is always fully applied
    assert!(Type::Int.is_fully_applied());
}

#[test]
fn test_type_remaining_arity() {
    // List has remaining arity 1 (needs 1 more arg)
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };
    assert_eq!(list_ctor.remaining_arity(), 1);

    // List<Int> has remaining arity 0 (fully applied)
    let list_int = Type::make_type_app(list_ctor, List::from(vec![Type::Int]));
    assert_eq!(list_int.remaining_arity(), 0);

    // Int has remaining arity 0 (concrete type)
    assert_eq!(Type::Int.remaining_arity(), 0);
}

#[test]
fn test_type_decompose_type_app() {
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };
    let list_int = Type::make_type_app(list_ctor.clone(), List::from(vec![Type::Int]));

    match list_int.decompose_type_app() {
        Maybe::Some((ctor, args)) => {
            assert!(matches!(ctor, Type::TypeConstructor { name, .. } if name == "List"));
            assert_eq!(args.len(), 1);
        }
        Maybe::None => panic!("Expected TypeApp"),
    }

    // Non-TypeApp should return None
    assert!(matches!(Type::Int.decompose_type_app(), Maybe::None));
}

#[test]
fn test_type_apply_type_args() {
    // Start with List constructor
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    // Apply Int to get List<Int>
    let list_int = list_ctor.apply_type_args(List::from(vec![Type::Int]));

    assert!(list_int.is_type_app());
    if let Type::TypeApp { constructor, args } = list_int {
        assert!(matches!(constructor.as_ref(), Type::TypeConstructor { name, .. } if name == "List"));
        assert_eq!(args.len(), 1);
    } else {
        panic!("Expected TypeApp");
    }
}

#[test]
fn test_type_get_constructor_kind() {
    let list_ctor = Type::TypeConstructor {
        name: "List".into(),
        arity: 1,
        kind: Kind::unary_constructor(),
    };

    match list_ctor.get_constructor_kind() {
        Maybe::Some(kind) => {
            assert_eq!(*kind, Kind::unary_constructor());
        }
        Maybe::None => panic!("Expected kind"),
    }

    // Non-TypeConstructor should return None
    assert!(matches!(Type::Int.get_constructor_kind(), Maybe::None));
}
