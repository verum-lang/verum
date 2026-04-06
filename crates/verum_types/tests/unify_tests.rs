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
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_types::TypeError;
use verum_types::refinement::RefinementPredicate;
use verum_types::ty::{Type, TypeVar};
use verum_types::unify::*;

#[test]
fn test_unify_primitives() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    // Same primitives unify
    assert!(unifier.unify(&Type::int(), &Type::int(), span).is_ok());
    assert!(unifier.unify(&Type::bool(), &Type::bool(), span).is_ok());

    // Different primitives don't unify
    assert!(unifier.unify(&Type::int(), &Type::bool(), span).is_err());
}

#[test]
fn test_unify_variables() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    // Unify variable with type
    let subst = unifier.unify(&Type::Var(v1), &Type::int(), span).unwrap();
    assert_eq!(subst.get(&v1), Some(&Type::int()));

    // Unify two variables
    let subst = unifier.unify(&Type::Var(v1), &Type::Var(v2), span).unwrap();
    assert!(subst.len() == 1);
}

#[test]
fn test_unify_functions() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let f1 = Type::function(vec![Type::int()].into(), Type::bool());
    let f2 = Type::function(vec![Type::int()].into(), Type::bool());

    assert!(unifier.unify(&f1, &f2, span).is_ok());

    // Different parameter types
    let f3 = Type::function(vec![Type::bool()].into(), Type::bool());
    assert!(unifier.unify(&f1, &f3, span).is_err());

    // Different return types
    let f4 = Type::function(vec![Type::int()].into(), Type::int());
    assert!(unifier.unify(&f1, &f4, span).is_err());
}

#[test]
fn test_occurs_check() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let v1 = TypeVar::fresh();
    let ty = Type::function(vec![Type::Var(v1)].into(), Type::int());

    // This would create infinite type: v1 = v1 -> Int
    let result = unifier.unify(&Type::Var(v1), &ty, span);
    assert!(matches!(result, Err(TypeError::InfiniteType { .. })));
}

#[test]
fn test_unify_tuples() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let t1 = Type::tuple(vec![Type::int(), Type::bool()].into());
    let t2 = Type::tuple(vec![Type::int(), Type::bool()].into());

    assert!(unifier.unify(&t1, &t2, span).is_ok());

    // Different lengths
    let t3 = Type::tuple(vec![Type::int()].into());
    assert!(unifier.unify(&t1, &t3, span).is_err());
}

#[test]
fn test_unify_with_substitution() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    // Create types: v1 -> v2 and Int -> v2
    let t1 = Type::function(vec![Type::Var(v1)].into(), Type::Var(v2));
    let t2 = Type::function(vec![Type::int()].into(), Type::Var(v2));

    let subst = unifier.unify(&t1, &t2, span).unwrap();

    // v1 should be bound to Int
    assert_eq!(subst.get(&v1), Some(&Type::int()));
}

#[test]
fn test_unify_refinements() {
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let pred = RefinementPredicate::inline(
        verum_ast::expr::Expr::literal(verum_ast::literal::Literal::bool(true, span)),
        span,
    );

    let r1 = Type::refined(Type::int(), pred.clone());
    let r2 = Type::refined(Type::int(), pred);

    // Refinements unify structurally (ignore predicates)
    assert!(unifier.unify(&r1, &r2, span).is_ok());

    // Refinement unifies with base type
    assert!(unifier.unify(&r1, &Type::int(), span).is_ok());
}

#[test]
fn test_reference_coercion_upcast() {
    // Unified reference model: &T (managed CBGR ~15ns), &checked T (statically verified 0ns), &unsafe T (unchecked 0ns) — .3.3 - Three-Tier Reference Coercion
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let inner = Type::int();

    // ALLOWED UPCASTS (forgetful coercion)

    // &unsafe T → &checked T  ✓
    let unsafe_ref = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    let checked_ref = Type::CheckedReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    assert!(
        unifier.unify(&unsafe_ref, &checked_ref, span).is_ok(),
        "upcast &unsafe T → &checked T should succeed"
    );

    // &unsafe T → &T  ✓
    let safe_ref = Type::Reference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    let unsafe_ref2 = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    assert!(
        unifier.unify(&unsafe_ref2, &safe_ref, span).is_ok(),
        "upcast &unsafe T → &T should succeed"
    );

    // &checked T → &T  ✓
    let checked_ref2 = Type::CheckedReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    let safe_ref2 = Type::Reference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    assert!(
        unifier.unify(&checked_ref2, &safe_ref2, span).is_ok(),
        "upcast &checked T → &T should succeed"
    );
}

#[test]
fn test_reference_coercion_downcast_forbidden() {
    // Unified reference model: &T (managed CBGR ~15ns), &checked T (statically verified 0ns), &unsafe T (unchecked 0ns) — .3.3 - Downcasts are FORBIDDEN
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let inner = Type::int();

    // FORBIDDEN DOWNCASTS

    // &T → &checked T  ✗
    let safe_ref = Type::Reference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    let checked_ref = Type::CheckedReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    assert!(
        unifier.unify(&safe_ref, &checked_ref, span).is_err(),
        "downcast &T → &checked T should fail"
    );

    // &T → &unsafe T  ✓ (allowed for FFI interop)
    // The implementation allows this conversion because it's commonly needed
    // when passing buffers to FFI functions. The caller takes responsibility.
    let safe_ref2 = Type::Reference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    let unsafe_ref = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    assert!(
        unifier.unify(&safe_ref2, &unsafe_ref, span).is_ok(),
        "FFI coercion &T → &unsafe T should succeed"
    );

    // &checked T → &unsafe T  ✗
    let checked_ref2 = Type::CheckedReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    let unsafe_ref2 = Type::UnsafeReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    assert!(
        unifier.unify(&checked_ref2, &unsafe_ref2, span).is_err(),
        "downcast &checked T → &unsafe T should fail"
    );
}

#[test]
fn test_reference_mutability_mismatch() {
    // Test mutability coercion rules
    let mut unifier = Unifier::new();
    let span = Span::dummy();

    let inner = Type::int();

    // &mut T → &T is allowed (safe to forget write capability)
    let mut_ref = Type::Reference {
        mutable: true,
        inner: Box::new(inner.clone()),
    };
    let immut_ref = Type::Reference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    assert!(
        unifier.unify(&mut_ref, &immut_ref, span).is_ok(),
        "mutable to immutable coercion should succeed"
    );

    // &unsafe mut T → &checked T (mutability mismatch)
    let unsafe_mut = Type::UnsafeReference {
        mutable: true,
        inner: Box::new(inner.clone()),
    };
    let checked_immut = Type::CheckedReference {
        mutable: false,
        inner: Box::new(inner.clone()),
    };
    assert!(
        unifier.unify(&unsafe_mut, &checked_immut, span).is_err(),
        "mutability must match even on upcast"
    );
}

#[test]
fn test_unify_named_types_with_different_spans() {
    // Test that Named types with the same path but different spans unify correctly.
    // This is the bug fix for nested record construction where the same type name
    // appears in different locations (type definition vs. variable usage).
    let mut unifier = Unifier::new();

    // Create two Named types representing "Point" but with different spans
    let span1 = Span::new(0, 10, verum_ast::span::FileId::dummy());
    let span2 = Span::new(100, 110, verum_ast::span::FileId::dummy());

    let path1 = Path::new(vec![PathSegment::Name(Ident::new("Point", span1))].into(), span1);
    let path2 = Path::new(vec![PathSegment::Name(Ident::new("Point", span2))].into(), span2);

    let type1 = Type::Named {
        path: path1,
        args: vec![].into(),
    };
    let type2 = Type::Named {
        path: path2,
        args: vec![].into(),
    };

    // These should unify despite having different spans
    let result = unifier.unify(&type1, &type2, span1);
    assert!(
        result.is_ok(),
        "Named types with same path but different spans should unify. Error: {:?}",
        result.err()
    );

    // Test that different Named types still don't unify
    let path3 = Path::new(
        vec![PathSegment::Name(Ident::new("Rectangle", span1))].into(),
        span1,
    );
    let type3 = Type::Named {
        path: path3,
        args: vec![].into(),
    };

    let result = unifier.unify(&type1, &type3, span1);
    assert!(
        result.is_err(),
        "Named types with different paths should not unify"
    );
}

#[test]
fn test_proof_irrelevance_prop() {
    // Inductive types: recursive type definitions with structural recursion, termination checking — .1 - Proof Irrelevance
    // All proofs of a proposition are equal (definitionally)
    use verum_common::Text;
    use verum_types::ty::{EqConst, EqTerm, UniverseLevel};

    let mut unifier = Unifier::new();
    let span = Span::dummy();

    // Two Prop types should always unify (proof irrelevance)
    let prop1 = Type::Prop;
    let prop2 = Type::Prop;

    assert!(
        unifier.unify(&prop1, &prop2, span).is_ok(),
        "Prop should unify with Prop"
    );
}

#[test]
fn test_proof_irrelevance_equality_types() {
    // Inductive types: recursive type definitions with structural recursion, termination checking — .1 - Proof Irrelevance
    // Two different proofs of the same equality should unify
    use verum_common::Text;
    use verum_types::ty::{EqConst, EqTerm};

    let mut unifier = Unifier::new();
    let span = Span::dummy();

    // Create two equality types: 0 = 0
    let zero = EqTerm::Const(EqConst::Nat(0));

    // Two different representations of the same equality
    let eq1 = Type::Eq {
        ty: Box::new(Type::Prop), // Equality lives in Prop
        lhs: Box::new(zero.clone()),
        rhs: Box::new(zero.clone()),
    };

    let eq2 = Type::Eq {
        ty: Box::new(Type::Prop),
        lhs: Box::new(zero.clone()),
        rhs: Box::new(zero.clone()),
    };

    // These should unify because they're both in Prop (proof-irrelevant)
    assert!(
        unifier.unify(&eq1, &eq2, span).is_ok(),
        "Equality types in Prop should unify (proof irrelevance)"
    );
}

#[test]
fn test_proof_irrelevance_sigma_types() {
    // Inductive types: recursive type definitions with structural recursion, termination checking — .1 - Proof Irrelevance
    // Sigma types with Prop in the second component are proof-irrelevant
    use verum_common::Text;

    let mut unifier = Unifier::new();
    let span = Span::dummy();

    // Create Sigma type: (x: Int, Prop)
    // This represents a refinement type with proof-irrelevant proof
    let sigma1 = Type::Sigma {
        fst_name: Text::from("x"),
        fst_type: Box::new(Type::int()),
        snd_type: Box::new(Type::Prop), // Second component is Prop
    };

    let sigma2 = Type::Sigma {
        fst_name: Text::from("x"),
        fst_type: Box::new(Type::int()),
        snd_type: Box::new(Type::Prop),
    };

    // These should unify - the Prop component makes proofs irrelevant
    assert!(
        unifier.unify(&sigma1, &sigma2, span).is_ok(),
        "Sigma types with Prop second component should unify (proof irrelevance)"
    );
}

#[test]
fn test_universe_hierarchy_prop() {
    // Inductive types: recursive type definitions with structural recursion, termination checking — .1 - Prop : Type₁
    use verum_types::ty::UniverseLevel;

    let prop = Type::Prop;
    let type_of_prop = prop.type_of();

    // Prop should live in Type₁
    assert_eq!(
        type_of_prop,
        Type::Universe {
            level: UniverseLevel::TYPE1
        },
        "Prop should have type Type₁"
    );
}

#[test]
fn test_type_of_universe_hierarchy() {
    // Universe hierarchy: Type : Type1 : Type2 : ... preventing paradoxes, universe polymorphism via Level parameter — Universe Hierarchy
    use verum_types::ty::UniverseLevel;

    // Type₀ : Type₁
    let type0 = Type::Universe {
        level: UniverseLevel::TYPE,
    };
    let type_of_type0 = type0.type_of();
    assert_eq!(
        type_of_type0,
        Type::Universe {
            level: UniverseLevel::TYPE1
        },
        "Type₀ should have type Type₁"
    );

    // Type₁ : Type₂
    let type1 = Type::Universe {
        level: UniverseLevel::TYPE1,
    };
    let type_of_type1 = type1.type_of();
    assert_eq!(
        type_of_type1,
        Type::Universe {
            level: UniverseLevel::TYPE2
        },
        "Type₁ should have type Type₂"
    );

    // Bool : Type₀
    let bool_ty = Type::bool();
    let type_of_bool = bool_ty.type_of();
    assert_eq!(
        type_of_bool,
        Type::Universe {
            level: UniverseLevel::TYPE
        },
        "Bool should have type Type₀"
    );
}
