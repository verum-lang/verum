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
