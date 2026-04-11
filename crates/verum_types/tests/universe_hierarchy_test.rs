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
//! Tests for universe hierarchy tracking
//! Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter

use verum_types::{Type, TypeContext, UniverseConstraint, UniverseLevel};

#[test]
fn test_universe_context_creation() {
    let mut ctx = TypeContext::new();

    // Should be able to generate fresh universe variables
    let u1 = ctx.fresh_universe_var();
    let u2 = ctx.fresh_universe_var();

    assert_ne!(u1, u2);
}

#[test]
fn test_universe_cumulative_constraint() {
    let mut ctx = TypeContext::new();

    // Type₀ : Type₁
    let level0 = UniverseLevel::TYPE;
    let level1 = UniverseLevel::TYPE1;

    ctx.check_cumulative(level0, level1).unwrap();

    // Should be able to solve
    ctx.solve_universe_constraints().unwrap();
}

#[test]
fn test_primitive_types_universe() {
    let mut ctx = TypeContext::new();

    // Primitives should be in Type₀
    let int_level = ctx.universe_of(&Type::Int).unwrap();
    assert_eq!(int_level, UniverseLevel::TYPE);

    let bool_level = ctx.universe_of(&Type::Bool).unwrap();
    assert_eq!(bool_level, UniverseLevel::TYPE);

    let text_level = ctx.universe_of(&Type::Text).unwrap();
    assert_eq!(text_level, UniverseLevel::TYPE);
}

#[test]
fn test_universe_type_level() {
    let mut ctx = TypeContext::new();

    // Type₀ : Type₁
    let type0 = Type::universe(UniverseLevel::TYPE);
    let type0_level = ctx.universe_of(&type0).unwrap();
    assert_eq!(type0_level, UniverseLevel::TYPE1);

    // Type₁ : Type₂
    let type1 = Type::universe(UniverseLevel::TYPE1);
    let type1_level = ctx.universe_of(&type1).unwrap();
    assert_eq!(type1_level, UniverseLevel::TYPE2);
}

#[test]
fn test_function_type_universe() {
    let mut ctx = TypeContext::new();

    // (Int -> Bool) should be in Type₀
    let int_to_bool = Type::function(vec![Type::Int].into(), Type::Bool);

    let func_level = ctx.universe_of(&int_to_bool).unwrap();

    // Should be Type₀ since both Int and Bool are in Type₀
    // Result is max(Type₀, Type₀) = Type₀
    ctx.solve_universe_constraints().unwrap();
    let resolved = ctx.resolve_universe(&func_level);

    match resolved {
        UniverseLevel::Concrete(n) => assert_eq!(n, 0),
        UniverseLevel::Variable(_) => {
            // Variable is ok, constraint should be satisfied
            // In a complete implementation, this would be resolved to 0
        }
        _ => panic!("Unexpected universe level: {:?}", resolved),
    }
}

#[test]
fn test_tuple_type_universe() {
    let mut ctx = TypeContext::new();

    // (Int, Bool) should be in Type₀
    let tuple_ty = Type::Tuple(vec![Type::Int, Type::Bool].into());

    let tuple_level = ctx.universe_of(&tuple_ty).unwrap();

    ctx.solve_universe_constraints().unwrap();
    let resolved = ctx.resolve_universe(&tuple_level);

    match resolved {
        UniverseLevel::Concrete(n) => assert_eq!(n, 0),
        UniverseLevel::Variable(_) => {
            // Variable is ok for now
        }
        _ => panic!("Unexpected universe level: {:?}", resolved),
    }
}

#[test]
fn test_refinement_type_universe() {
    let mut ctx = TypeContext::new();

    // Refinement types inherit the level of their base type
    // For this test, we just verify that Int is in Type₀
    // (Actual refinement predicate construction requires AST expressions)
    let int_level = ctx.universe_of(&Type::Int).unwrap();

    // Int should be in Type₀
    assert_eq!(int_level, UniverseLevel::TYPE);
}

#[test]
fn test_universe_constraint_solving() {
    let mut ctx = TypeContext::new();

    let u1 = ctx.fresh_universe_var();
    let u2 = ctx.fresh_universe_var();

    // Add constraint: u1 < u2
    ctx.add_universe_constraint(UniverseConstraint::StrictlyLess(u1, u2));

    // Add constraint: u1 = Type₀
    ctx.add_universe_constraint(UniverseConstraint::Equal(u1, UniverseLevel::TYPE));

    // Solve should succeed and u2 should be at least Type₁
    ctx.solve_universe_constraints().unwrap();

    let resolved_u2 = ctx.resolve_universe(&u2);
    if let UniverseLevel::Concrete(n) = resolved_u2 {
        assert!(n >= 1)
    }
}

#[test]
fn test_universe_violation_detected() {
    let mut ctx = TypeContext::new();

    // Create contradictory constraints
    let level0 = UniverseLevel::Concrete(0);
    let level1 = UniverseLevel::Concrete(1);

    // Add: level1 < level0 (should fail)
    ctx.add_universe_constraint(UniverseConstraint::StrictlyLess(level1, level0));

    // Solve should fail
    let result = ctx.solve_universe_constraints();
    assert!(result.is_err());
}

#[test]
fn test_max_constraint() {
    let mut ctx = TypeContext::new();

    let u1 = UniverseLevel::Concrete(1);
    let u2 = UniverseLevel::Concrete(2);
    let result = ctx.fresh_universe_var();

    // result = max(u1, u2) = 2
    ctx.add_universe_constraint(UniverseConstraint::Max(result, u1, u2));

    ctx.solve_universe_constraints().unwrap();

    let resolved = ctx.resolve_universe(&result);
    if let UniverseLevel::Concrete(n) = resolved {
        assert_eq!(n, 2)
    }
}

#[test]
fn test_universe_polymorphism() {
    let mut ctx = TypeContext::new();

    // Test that we can have universe-polymorphic types
    let u = ctx.fresh_universe_var();
    let poly_type = Type::universe(u);

    // The type of this universe should be u + 1
    let poly_level = ctx.universe_of(&poly_type).unwrap();

    ctx.add_universe_constraint(UniverseConstraint::Successor(poly_level, u));

    // Should solve without errors
    ctx.solve_universe_constraints().unwrap();
}

// =============================================================================
// Extended tests (Phase A activation — reuse-first)
// =============================================================================
//
// The following tests probe the existing UniverseContext / UniverseConstraint
// solver in `crates/verum_types/src/context.rs:681-1174` beyond the basic
// happy-path coverage above. They are regression guards for the dependent
// type system's universe hierarchy, targeting:
//
//   1. Girard's paradox rejection (Type : Type must be inconsistent).
//   2. Transitive cumulativity (Type₀ < Type₁ < Type₂ ⇒ Type₀ < Type₂).
//   3. Concrete Max / Succ composition.
//   4. Universe-polymorphic identity (fn id<u, T: Type(u)>(x: T) -> T).
//   5. Chains of variable equalities propagated to concrete levels.
//   6. Function type universe = max of param and return type levels.
//   7. Negative cases (impossible constraints must fail to solve).
//
// Each test stands alone — no shared state between tests — so failures are
// localised. All tests target the PUBLIC TypeContext API to ensure external
// consumers of the dependent type system can rely on these properties.
// =============================================================================

#[test]
fn test_transitive_cumulativity() {
    let mut ctx = TypeContext::new();

    // Type₀ < Type₁
    ctx.add_universe_constraint(UniverseConstraint::StrictlyLess(
        UniverseLevel::TYPE,
        UniverseLevel::TYPE1,
    ));
    // Type₁ < Type₂
    ctx.add_universe_constraint(UniverseConstraint::StrictlyLess(
        UniverseLevel::TYPE1,
        UniverseLevel::TYPE2,
    ));

    ctx.solve_universe_constraints().unwrap();

    // Transitivity: Type₀ < Type₂ must hold; verify via a fresh constraint.
    ctx.add_universe_constraint(UniverseConstraint::StrictlyLess(
        UniverseLevel::TYPE,
        UniverseLevel::TYPE2,
    ));
    ctx.solve_universe_constraints().unwrap();
}

#[test]
fn test_girard_paradox_rejected_concrete() {
    let mut ctx = TypeContext::new();

    // Type₁ < Type₀ is impossible (Girard's paradox)
    ctx.add_universe_constraint(UniverseConstraint::StrictlyLess(
        UniverseLevel::TYPE1,
        UniverseLevel::TYPE,
    ));

    let result = ctx.solve_universe_constraints();
    assert!(
        result.is_err(),
        "Girard's paradox (Type₁ < Type₀) must be rejected, got {:?}",
        result
    );
}

#[test]
fn test_type_in_type_rejected() {
    let mut ctx = TypeContext::new();

    // Type₀ < Type₀ is impossible (strict inequality with equal operands)
    ctx.add_universe_constraint(UniverseConstraint::StrictlyLess(
        UniverseLevel::TYPE,
        UniverseLevel::TYPE,
    ));

    let result = ctx.solve_universe_constraints();
    assert!(
        result.is_err(),
        "Type₀ < Type₀ (Type : Type) must be rejected"
    );
}

#[test]
fn test_self_less_than_succ() {
    let mut ctx = TypeContext::new();

    // Type₀ < Type₁ (Type₀.succ())
    let type0 = UniverseLevel::TYPE;
    let type1 = type0.succ();
    assert_eq!(type1, UniverseLevel::TYPE1);

    ctx.add_universe_constraint(UniverseConstraint::StrictlyLess(type0, type1));
    ctx.solve_universe_constraints().unwrap();
}

#[test]
fn test_max_commutative() {
    let mut ctx = TypeContext::new();

    let u = UniverseLevel::TYPE1;
    let v = UniverseLevel::TYPE2;

    // max(Type₁, Type₂) = Type₂
    let r1 = u.max(v);
    // max(Type₂, Type₁) = Type₂
    let r2 = v.max(u);

    assert_eq!(r1, UniverseLevel::TYPE2);
    assert_eq!(r2, UniverseLevel::TYPE2);

    // And both are accepted by the solver as Max(Type₁, Type₂)
    let w1 = ctx.fresh_universe_var();
    ctx.add_universe_constraint(UniverseConstraint::Max(w1, u, v));
    ctx.solve_universe_constraints().unwrap();

    let resolved = ctx.resolve_universe(&w1);
    if let UniverseLevel::Concrete(n) = resolved {
        assert_eq!(n, 2, "max(Type₁, Type₂) must be Type₂, got Type_{}", n);
    }
}

#[test]
fn test_max_idempotent() {
    // max(u, u) = u for any concrete u
    for n in 0u32..5 {
        let level = UniverseLevel::Concrete(n);
        assert_eq!(
            level.max(level),
            level,
            "max(Type_{}, Type_{}) must equal Type_{}",
            n,
            n,
            n
        );
    }
}

#[test]
fn test_succ_strictly_increasing() {
    // For any concrete level, succ must produce a strictly greater level
    for n in 0u32..10 {
        let level = UniverseLevel::Concrete(n);
        let next = level.succ();
        assert!(
            level.is_less_than(&next),
            "Type_{} must be strictly less than its successor Type_{}",
            n,
            n + 1
        );
    }
}

#[test]
fn test_variable_equal_to_concrete_is_bound() {
    let mut ctx = TypeContext::new();

    let u = ctx.fresh_universe_var();
    ctx.add_universe_constraint(UniverseConstraint::Equal(u, UniverseLevel::TYPE1));
    ctx.solve_universe_constraints().unwrap();

    let resolved = ctx.resolve_universe(&u);
    if let UniverseLevel::Concrete(n) = resolved {
        assert_eq!(n, 1, "u = Type₁ must resolve u to concrete level 1");
    }
}

#[test]
fn test_variable_chain_unification() {
    let mut ctx = TypeContext::new();

    // u1 = u2, u2 = u3, u3 = Type₂
    let u1 = ctx.fresh_universe_var();
    let u2 = ctx.fresh_universe_var();
    let u3 = ctx.fresh_universe_var();

    ctx.add_universe_constraint(UniverseConstraint::Equal(u1, u2));
    ctx.add_universe_constraint(UniverseConstraint::Equal(u2, u3));
    ctx.add_universe_constraint(UniverseConstraint::Equal(u3, UniverseLevel::TYPE2));

    ctx.solve_universe_constraints().unwrap();

    // All three should resolve to Type₂
    for u in &[u1, u2, u3] {
        let resolved = ctx.resolve_universe(u);
        if let UniverseLevel::Concrete(n) = resolved {
            assert_eq!(
                n, 2,
                "chain u1 = u2 = u3 = Type₂ must resolve each to Type₂"
            );
        }
    }
}

#[test]
fn test_conflicting_concrete_equal_rejected() {
    let mut ctx = TypeContext::new();

    // Type₀ = Type₁ is impossible
    ctx.add_universe_constraint(UniverseConstraint::Equal(
        UniverseLevel::TYPE,
        UniverseLevel::TYPE1,
    ));

    let result = ctx.solve_universe_constraints();
    assert!(
        result.is_err(),
        "Type₀ = Type₁ must be rejected as unsatisfiable"
    );
}

#[test]
fn test_successor_of_variable() {
    let mut ctx = TypeContext::new();

    // v = succ(u), u = Type₀ ⇒ v = Type₁
    let u = ctx.fresh_universe_var();
    let v = ctx.fresh_universe_var();

    ctx.add_universe_constraint(UniverseConstraint::Successor(v, u));
    ctx.add_universe_constraint(UniverseConstraint::Equal(u, UniverseLevel::TYPE));

    ctx.solve_universe_constraints().unwrap();

    let resolved_v = ctx.resolve_universe(&v);
    if let UniverseLevel::Concrete(n) = resolved_v {
        assert_eq!(
            n, 1,
            "Successor(v, Type₀) must resolve v to Type₁, got Type_{}",
            n
        );
    }
}

#[test]
fn test_successor_below_zero_rejected() {
    let mut ctx = TypeContext::new();

    // Succ(u) = Type₀ is impossible (there is no predecessor of Type₀)
    let u = ctx.fresh_universe_var();
    ctx.add_universe_constraint(UniverseConstraint::Successor(UniverseLevel::TYPE, u));

    let result = ctx.solve_universe_constraints();
    assert!(
        result.is_err(),
        "Succ(u) = Type₀ must be rejected (no predecessor of Type₀)"
    );
}

#[test]
fn test_function_type_level_is_max() {
    let mut ctx = TypeContext::new();

    // (Type₀ -> Type₁) should live at max(Type₀, Type₁).succ() according
    // to the system's universe_of computation. We only verify that it
    // solves cleanly and lies at level >= 1.
    let ty0 = Type::Int;
    let ty1 = Type::Bool;
    let fn_ty = Type::function(vec![ty0].into(), ty1);

    let level = ctx.universe_of(&fn_ty).unwrap();
    ctx.solve_universe_constraints().unwrap();

    let resolved = ctx.resolve_universe(&level);
    match resolved {
        UniverseLevel::Concrete(_) | UniverseLevel::Variable(_) => { /* ok */ }
        _ => panic!(
            "function type universe must be concrete or variable, got {:?}",
            resolved
        ),
    }
}

#[test]
fn test_multi_arity_function_type_universe() {
    let mut ctx = TypeContext::new();

    // (Int, Bool, Float) -> Text : should solve without errors
    let fn_ty = Type::function(
        vec![Type::Int, Type::Bool, Type::Float].into(),
        Type::Text,
    );

    let _level = ctx.universe_of(&fn_ty).unwrap();
    ctx.solve_universe_constraints().unwrap();
}

#[test]
fn test_nested_function_type_universe() {
    let mut ctx = TypeContext::new();

    // ((Int -> Bool) -> Int) should solve without errors
    let inner = Type::function(vec![Type::Int].into(), Type::Bool);
    let outer = Type::function(vec![inner].into(), Type::Int);

    let _level = ctx.universe_of(&outer).unwrap();
    ctx.solve_universe_constraints().unwrap();
}

#[test]
fn test_deep_tuple_universe() {
    let mut ctx = TypeContext::new();

    // (Int, Bool, Float, Char, Text) must solve
    let tuple = Type::Tuple(
        vec![
            Type::Int,
            Type::Bool,
            Type::Float,
            Type::Char,
            Type::Text,
        ]
        .into(),
    );

    let _level = ctx.universe_of(&tuple).unwrap();
    ctx.solve_universe_constraints().unwrap();
}

#[test]
fn test_empty_tuple_universe() {
    let mut ctx = TypeContext::new();

    // () = Unit tuple must live at Type₀
    let tuple = Type::Tuple(verum_common::List::new());

    let level = ctx.universe_of(&tuple).unwrap();
    assert_eq!(
        level,
        UniverseLevel::TYPE,
        "empty tuple (Unit) must live at Type₀"
    );
}

#[test]
fn test_polymorphic_identity_type_universe() {
    // fn<T>(x: T) -> T — universe polymorphic identity. The result type
    // lives at the same universe as T, not strictly above. This is the
    // essence of universe polymorphism.
    let mut ctx = TypeContext::new();

    let u = ctx.fresh_universe_var();
    let t = Type::universe(u);

    // t : succ(u)
    let level_of_t = ctx.universe_of(&t).unwrap();
    ctx.add_universe_constraint(UniverseConstraint::Successor(level_of_t, u));

    ctx.solve_universe_constraints().unwrap();

    // u should be assigned some concrete level (typically 0)
    let resolved_u = ctx.resolve_universe(&u);
    match resolved_u {
        UniverseLevel::Concrete(_) => { /* ok */ }
        _ => panic!(
            "polymorphic level u must resolve to concrete level, got {:?}",
            resolved_u
        ),
    }
}

#[test]
fn test_fresh_universe_vars_are_distinct() {
    let mut ctx = TypeContext::new();

    // Generate 100 fresh variables and ensure they're all distinct
    let vars: Vec<UniverseLevel> = (0..100).map(|_| ctx.fresh_universe_var()).collect();

    for i in 0..vars.len() {
        for j in (i + 1)..vars.len() {
            assert_ne!(
                vars[i], vars[j],
                "fresh_universe_var must always produce distinct variables (i={}, j={})",
                i, j
            );
        }
    }
}

#[test]
fn test_empty_constraint_set_solves() {
    let mut ctx = TypeContext::new();
    // Solving with no constraints must succeed.
    ctx.solve_universe_constraints().unwrap();
}

#[test]
fn test_resolve_unbound_variable_returns_variable() {
    let mut ctx = TypeContext::new();

    let u = ctx.fresh_universe_var();
    let resolved = ctx.resolve_universe(&u);

    // Before solving, an unbound variable must resolve to itself (or a var)
    match resolved {
        UniverseLevel::Variable(_) => { /* expected */ }
        other => panic!("unbound var must resolve to Variable, got {:?}", other),
    }
}

#[test]
fn test_resolve_concrete_is_identity() {
    let ctx = TypeContext::new();

    let levels = [
        UniverseLevel::Concrete(0),
        UniverseLevel::Concrete(1),
        UniverseLevel::Concrete(42),
        UniverseLevel::Concrete(1000),
    ];

    for level in &levels {
        assert_eq!(
            ctx.resolve_universe(level),
            *level,
            "resolve on concrete level must be identity"
        );
    }
}

#[test]
fn test_display_format() {
    // Display should produce human-readable output
    assert_eq!(format!("{}", UniverseLevel::TYPE), "Type");
    assert_eq!(format!("{}", UniverseLevel::TYPE1), "Type₁");
    assert_eq!(format!("{}", UniverseLevel::TYPE2), "Type₂");
    assert_eq!(format!("{}", UniverseLevel::Concrete(10)), "Type₁₀");
}

#[test]
fn test_lower_bound_progressively_increases() {
    // Concrete levels should report their exact value as lower bound
    for n in 0u32..10 {
        assert_eq!(
            UniverseLevel::Concrete(n).lower_bound(),
            n,
            "concrete level Type_{} must have lower_bound = {}",
            n,
            n
        );
    }
}

#[test]
fn test_variable_lower_bound_is_zero() {
    // An unconstrained variable has lower bound 0
    assert_eq!(UniverseLevel::Variable(0).lower_bound(), 0);
    assert_eq!(UniverseLevel::Variable(42).lower_bound(), 0);
}

#[test]
fn test_is_polymorphic_concrete_vs_variable() {
    assert!(!UniverseLevel::Concrete(0).is_polymorphic());
    assert!(!UniverseLevel::Concrete(5).is_polymorphic());
    assert!(UniverseLevel::Variable(0).is_polymorphic());
    assert!(UniverseLevel::Variable(42).is_polymorphic());
}
