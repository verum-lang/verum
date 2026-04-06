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
use indexmap::IndexMap;
use verum_ast::{
    expr::Expr,
    literal::Literal,
    span::Span,
    ty::{Ident, Path},
};
use verum_common::{List, Map, Maybe, Text};
use verum_common::ConstValue;
use verum_types::refinement::RefinementPredicate;
use verum_types::ty::*;

// ==================== Primitive Types ====================

#[test]
fn test_primitive_types_display() {
    assert_eq!(Type::unit().to_string(), "Unit");
    assert_eq!(Type::bool().to_string(), "Bool");
    assert_eq!(Type::int().to_string(), "Int");
    assert_eq!(Type::float().to_string(), "Float");
    assert_eq!(Type::text().to_string(), "Text");
}

#[test]
fn test_primitive_types_equality() {
    assert_eq!(Type::unit(), Type::Unit);
    assert_eq!(Type::bool(), Type::Bool);
    assert_eq!(Type::int(), Type::Int);
    assert_eq!(Type::float(), Type::Float);
    assert_ne!(Type::int(), Type::float());
    assert_ne!(Type::bool(), Type::unit());
}

#[test]
fn test_primitive_types_are_monotypes() {
    assert!(Type::unit().is_monotype());
    assert!(Type::bool().is_monotype());
    assert!(Type::int().is_monotype());
    assert!(Type::float().is_monotype());
    assert!(Type::text().is_monotype());
}

// ==================== Named Types ====================

#[test]
fn test_named_type_without_args() {
    let ident = Ident::new("List", Span::dummy());
    let path = Path::single(ident);
    let named = Type::Named {
        path: path.clone(),
        args: vec![].into(),
    };

    assert_eq!(named.to_string(), "List");
}

#[test]
fn test_named_type_with_args() {
    let ident = Ident::new("List", Span::dummy());
    let path = Path::single(ident);
    let named = Type::Named {
        path,
        args: vec![Type::int()].into(),
    };

    assert_eq!(named.to_string(), "List<Int>");
}

#[test]
fn test_named_type_with_multiple_args() {
    let ident = Ident::new("Map", Span::dummy());
    let path = Path::single(ident);
    let named = Type::Named {
        path,
        args: vec![Type::text(), Type::int()].into(),
    };

    assert_eq!(named.to_string(), "Map<Text, Int>");
}

// ==================== Function Types ====================

#[test]
fn test_function_type_display() {
    let func = Type::function(vec![Type::int(), Type::int()].into(), Type::int());
    assert_eq!(func.to_string(), "fn(Int, Int) -> Int");
}

#[test]
fn test_function_with_contexts() {
    use std::any::TypeId;
    use verum_types::di::requirement::{ContextRef, ContextRequirement};

    let db_ref = ContextRef::new("Database".into(), TypeId::of::<()>());
    let logger_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
    let contexts = ContextRequirement::from_contexts(vec![db_ref, logger_ref]);

    let func = Type::function_with_contexts(vec![Type::int()].into(), Type::bool(), contexts);
    let func_str = func.to_string();
    // Context order may vary due to set-based storage
    assert!(
        func_str == "fn(Int) -> Bool using [Database, Logger]"
            || func_str == "fn(Int) -> Bool using [Logger, Database]",
        "Unexpected function string: {}",
        func_str
    );
}

#[test]
fn test_function_is_function() {
    let func = Type::function(vec![Type::int()].into(), Type::bool());
    assert!(func.is_function());
    assert!(!Type::int().is_function());
}

// ==================== Tuple Types ====================

#[test]
fn test_tuple_type_display() {
    let tuple = Type::tuple(vec![Type::int(), Type::bool()].into());
    assert_eq!(tuple.to_string(), "(Int, Bool)");
}

#[test]
fn test_tuple_empty() {
    // Empty tuple is canonicalized to Unit type
    let tuple = Type::tuple(vec![].into());
    assert_eq!(tuple.to_string(), "Unit");
}

#[test]
fn test_tuple_single() {
    let tuple = Type::tuple(vec![Type::int()].into());
    assert_eq!(tuple.to_string(), "(Int)");
}

#[test]
fn test_tuple_nested() {
    let inner = Type::tuple(vec![Type::int(), Type::bool()].into());
    let outer = Type::tuple(vec![inner, Type::text()].into());
    assert_eq!(outer.to_string(), "((Int, Bool), Text)");
}

// ==================== Array Types ====================

#[test]
fn test_array_with_size() {
    let arr = Type::array(Type::int(), Some(5));
    assert_eq!(arr.to_string(), "[Int; 5]");
}

#[test]
fn test_array_without_size() {
    let arr = Type::array(Type::int(), None);
    assert_eq!(arr.to_string(), "[Int]");
}

#[test]
fn test_array_nested() {
    let inner = Type::array(Type::int(), Some(3));
    let outer = Type::array(inner, Some(2));
    assert_eq!(outer.to_string(), "[[Int; 3]; 2]");
}

// ==================== Record Types ====================

#[test]
fn test_record_type() {
    let mut fields = IndexMap::new();
    fields.insert("x".into(), Type::int());
    fields.insert("y".into(), Type::int());
    let record = Type::Record(fields);

    assert_eq!(record.to_string(), "{ x: Int, y: Int }");
}

#[test]
fn test_record_empty() {
    let record = Type::Record(IndexMap::new());
    assert_eq!(record.to_string(), "{  }");
}

#[test]
fn test_record_nested() {
    let mut inner_fields = IndexMap::new();
    inner_fields.insert("a".into(), Type::int());
    let inner = Type::Record(inner_fields);

    let mut outer_fields = IndexMap::new();
    outer_fields.insert("inner".into(), inner);
    let outer = Type::Record(outer_fields);

    assert_eq!(outer.to_string(), "{ inner: { a: Int } }");
}

// ==================== Variant Types ====================

#[test]
fn test_variant_type() {
    let mut variants = IndexMap::new();
    variants.insert("Some".into(), Type::int());
    variants.insert("None".into(), Type::unit());
    let variant = Type::Variant(variants);

    assert_eq!(variant.to_string(), "Some(Int) | None(Unit)");
}

#[test]
fn test_variant_single() {
    let mut variants = IndexMap::new();
    variants.insert("Value".into(), Type::int());
    let variant = Type::Variant(variants);

    assert_eq!(variant.to_string(), "Value(Int)");
}

// ==================== Reference Types ====================

#[test]
fn test_reference_immutable() {
    let ref_ty = Type::reference(false, Type::int());
    assert_eq!(ref_ty.to_string(), "&Int");
}

#[test]
fn test_reference_mutable() {
    let ref_ty = Type::reference(true, Type::int());
    assert_eq!(ref_ty.to_string(), "&mut Int");
}

#[test]
fn test_checked_reference_immutable() {
    let ref_ty = Type::checked_reference(false, Type::int());
    assert_eq!(ref_ty.to_string(), "&checked Int");
}

#[test]
fn test_checked_reference_mutable() {
    let ref_ty = Type::checked_reference(true, Type::int());
    assert_eq!(ref_ty.to_string(), "&checked mut Int");
}

#[test]
fn test_unsafe_reference_immutable() {
    let ref_ty = Type::unsafe_reference(false, Type::int());
    assert_eq!(ref_ty.to_string(), "&unsafe Int");
}

#[test]
fn test_unsafe_reference_mutable() {
    let ref_ty = Type::unsafe_reference(true, Type::int());
    assert_eq!(ref_ty.to_string(), "&unsafe mut Int");
}

#[test]
fn test_ownership_reference() {
    let own = Type::Ownership {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    // Display uses "owned" keyword for clarity
    assert_eq!(own.to_string(), "owned Int");
}

#[test]
fn test_pointer_type() {
    let ptr = Type::Pointer {
        mutable: false,
        inner: Box::new(Type::int()),
    };
    assert_eq!(ptr.to_string(), "*const Int");
}

// ==================== Refinement Types ====================

#[test]
fn test_refinement_type() {
    let base = Type::int();
    let pred = RefinementPredicate::inline(
        Expr::literal(Literal::bool(true, Span::dummy())),
        Span::dummy(),
    );
    let refined = Type::refined(base, pred);

    assert!(matches!(refined, Type::Refined { .. }));
    assert_eq!(refined.base(), &Type::int());
}

#[test]
fn test_refinement_base_unwrapping() {
    let base = Type::int();
    let pred = RefinementPredicate::inline(
        Expr::literal(Literal::bool(true, Span::dummy())),
        Span::dummy(),
    );
    let refined = Type::refined(base.clone(), pred.clone());
    let double_refined = Type::refined(refined, pred.clone());

    assert_eq!(double_refined.base(), &base);
}

// ==================== Meta Types ====================

#[test]
fn test_meta_type() {
    let meta = Type::meta("N".into(), Type::Int, None);
    assert_eq!(meta.to_string(), "N: meta Int");
}

#[test]
fn test_meta_type_with_refinement() {
    let pred = RefinementPredicate::inline(
        Expr::literal(Literal::bool(true, Span::dummy())),
        Span::dummy(),
    );
    let meta = Type::meta("N".into(), Type::Int, Some(pred));
    assert!(meta.to_string().starts_with("N: meta Int{"));
}

// ==================== Future and Generator Types ====================

#[test]
fn test_future_type() {
    let future = Type::future(Type::int());
    assert_eq!(future.to_string(), "Future<Int>");
}

#[test]
fn test_generator_type() {
    let generator = Type::generator(Type::int(), Type::unit());
    assert_eq!(generator.to_string(), "Generator<Int, Unit>");
}

// ==================== Tensor Types ====================

#[test]
fn test_tensor_1d() {
    let shape = vec![ConstValue::UInt(4)].into();
    let tensor = Type::tensor(Type::float(), shape, Span::dummy());
    assert_eq!(tensor.to_string(), "Tensor<Float, [4u]>");
}

#[test]
fn test_tensor_2d() {
    let shape = vec![ConstValue::UInt(2), ConstValue::UInt(3)].into();
    let tensor = Type::tensor(Type::float(), shape, Span::dummy());
    assert_eq!(tensor.to_string(), "Tensor<Float, [2u, 3u]>");
}

#[test]
fn test_tensor_strides() {
    let shape = vec![
        ConstValue::UInt(2),
        ConstValue::UInt(3),
        ConstValue::UInt(4),
    ]
    .into();
    let tensor = Type::tensor(Type::float(), shape, Span::dummy());

    if let Type::Tensor { strides, .. } = tensor {
        assert_eq!(strides, vec![12, 4, 1].into());
    } else {
        panic!("Expected Tensor type");
    }
}

// ==================== Type Variables ====================

#[test]
fn test_type_var_fresh() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();
    assert_ne!(v1, v2);
}

#[test]
fn test_type_var_display() {
    let v = TypeVar::with_id(0);
    assert_eq!(v.to_string(), "α");
    let v2 = TypeVar::with_id(1);
    assert_eq!(v2.to_string(), "β");
}

#[test]
fn test_type_var_in_type() {
    let v = TypeVar::fresh();
    let ty = Type::Var(v);
    assert!(!ty.is_monotype());
}

// ==================== Free Variables ====================

#[test]
fn test_free_vars_basic() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let ty = Type::function(vec![Type::Var(v1)].into(), Type::Var(v2));
    let free = ty.free_vars();
    assert_eq!(free.len(), 2);
    assert!(free.contains(&v1));
    assert!(free.contains(&v2));
}

#[test]
fn test_free_vars_tuple() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let tuple = Type::tuple(vec![Type::Var(v1), Type::int(), Type::Var(v2)].into());
    let free = tuple.free_vars();
    assert_eq!(free.len(), 2);
    assert!(free.contains(&v1));
    assert!(free.contains(&v2));
}

#[test]
fn test_free_vars_array() {
    let v = TypeVar::fresh();
    let arr = Type::array(Type::Var(v), Some(5));
    let free = arr.free_vars();
    assert_eq!(free.len(), 1);
    assert!(free.contains(&v));
}

#[test]
fn test_free_vars_record() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let mut fields = IndexMap::new();
    fields.insert("x".into(), Type::Var(v1));
    fields.insert("y".into(), Type::Var(v2));
    let record = Type::Record(fields);

    let free = record.free_vars();
    assert_eq!(free.len(), 2);
}

#[test]
fn test_free_vars_variant() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let mut variants = IndexMap::new();
    variants.insert("A".into(), Type::Var(v1));
    variants.insert("B".into(), Type::Var(v2));
    let variant = Type::Variant(variants);

    let free = variant.free_vars();
    assert_eq!(free.len(), 2);
}

#[test]
fn test_free_vars_reference() {
    let v = TypeVar::fresh();
    let ref_ty = Type::reference(false, Type::Var(v));
    let free = ref_ty.free_vars();
    assert_eq!(free.len(), 1);
    assert!(free.contains(&v));
}

#[test]
fn test_free_vars_exists() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let exists = Type::Exists {
        var: v1,
        body: Box::new(Type::function(vec![Type::Var(v1)].into(), Type::Var(v2))),
    };

    let free = exists.free_vars();
    // v1 is bound, only v2 is free
    assert_eq!(free.len(), 1);
    assert!(free.contains(&v2));
    assert!(!free.contains(&v1));
}

#[test]
fn test_free_vars_forall() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();
    let v3 = TypeVar::fresh();

    let forall = Type::Forall {
        vars: vec![v1, v2].into(),
        body: Box::new(Type::function(
            vec![Type::Var(v1), Type::Var(v2)].into(),
            Type::Var(v3),
        )),
    };

    let free = forall.free_vars();
    // v1 and v2 are bound, only v3 is free
    assert_eq!(free.len(), 1);
    assert!(free.contains(&v3));
}

#[test]
fn test_free_vars_named_with_args() {
    let v = TypeVar::fresh();
    let ident = Ident::new("List", Span::dummy());
    let path = Path::single(ident);
    let named = Type::Named {
        path,
        args: vec![Type::Var(v)].into(),
    };

    let free = named.free_vars();
    assert_eq!(free.len(), 1);
    assert!(free.contains(&v));
}

#[test]
fn test_free_vars_future() {
    let v = TypeVar::fresh();
    let future = Type::future(Type::Var(v));
    let free = future.free_vars();
    assert_eq!(free.len(), 1);
}

#[test]
fn test_free_vars_generator() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();
    let generator = Type::generator(Type::Var(v1), Type::Var(v2));
    let free = generator.free_vars();
    assert_eq!(free.len(), 2);
}

#[test]
fn test_free_vars_tensor() {
    let v = TypeVar::fresh();
    let shape = vec![ConstValue::UInt(4)].into();
    let tensor = Type::tensor(Type::Var(v), shape, Span::dummy());
    let free = tensor.free_vars();
    assert_eq!(free.len(), 1);
}

// ==================== Substitution ====================

#[test]
fn test_substitution_basic() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let ty = Type::function(vec![Type::Var(v1)].into(), Type::Var(v2));

    let mut subst = Substitution::new();
    subst.insert(v1, Type::int());
    subst.insert(v2, Type::bool());

    let result = ty.apply_subst(&subst);
    assert_eq!(
        result,
        Type::function(vec![Type::int()].into(), Type::bool())
    );
}

#[test]
fn test_substitution_compose() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let mut s1 = Substitution::new();
    s1.insert(v1, Type::Var(v2));

    let mut s2 = Substitution::new();
    s2.insert(v2, Type::int());

    let composed = s1.compose(&s2);
    assert_eq!(composed.get(&v1), Some(&Type::int()));
}

#[test]
fn test_substitution_tuple() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let tuple = Type::tuple(vec![Type::Var(v1), Type::Var(v2)].into());

    let mut subst = Substitution::new();
    subst.insert(v1, Type::int());
    subst.insert(v2, Type::bool());

    let result = tuple.apply_subst(&subst);
    assert_eq!(result, Type::tuple(vec![Type::int(), Type::bool()].into()));
}

#[test]
fn test_substitution_array() {
    let v = TypeVar::fresh();
    let arr = Type::array(Type::Var(v), Some(5));

    let mut subst = Substitution::new();
    subst.insert(v, Type::int());

    let result = arr.apply_subst(&subst);
    assert_eq!(result, Type::array(Type::int(), Some(5)));
}

#[test]
fn test_substitution_record() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let mut fields = IndexMap::new();
    fields.insert("x".into(), Type::Var(v1));
    fields.insert("y".into(), Type::Var(v2));
    let record = Type::Record(fields);

    let mut subst = Substitution::new();
    subst.insert(v1, Type::int());
    subst.insert(v2, Type::bool());

    let result = record.apply_subst(&subst);

    if let Type::Record(result_fields) = result {
        assert_eq!(result_fields.get("x"), Some(&Type::int()));
        assert_eq!(result_fields.get("y"), Some(&Type::bool()));
    } else {
        panic!("Expected record type");
    }
}

#[test]
fn test_substitution_variant() {
    let v = TypeVar::fresh();

    let mut variants = IndexMap::new();
    variants.insert("Some".into(), Type::Var(v));
    variants.insert("None".into(), Type::unit());
    let variant = Type::Variant(variants);

    let mut subst = Substitution::new();
    subst.insert(v, Type::int());

    let result = variant.apply_subst(&subst);

    if let Type::Variant(result_variants) = result {
        assert_eq!(result_variants.get("Some"), Some(&Type::int()));
    } else {
        panic!("Expected variant type");
    }
}

#[test]
fn test_substitution_reference() {
    let v = TypeVar::fresh();
    let ref_ty = Type::reference(false, Type::Var(v));

    let mut subst = Substitution::new();
    subst.insert(v, Type::int());

    let result = ref_ty.apply_subst(&subst);
    assert_eq!(result, Type::reference(false, Type::int()));
}

#[test]
fn test_substitution_named() {
    let v = TypeVar::fresh();
    let ident = Ident::new("List", Span::dummy());
    let path = Path::single(ident);
    let named = Type::Named {
        path: path.clone(),
        args: vec![Type::Var(v)].into(),
    };

    let mut subst = Substitution::new();
    subst.insert(v, Type::int());

    let result = named.apply_subst(&subst);

    if let Type::Named { args, .. } = result {
        assert_eq!(args, vec![Type::int()].into());
    } else {
        panic!("Expected named type");
    }
}

#[test]
fn test_substitution_exists() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let exists = Type::Exists {
        var: v1,
        body: Box::new(Type::Var(v2)),
    };

    let mut subst = Substitution::new();
    subst.insert(v1, Type::int()); // Should not substitute bound variable
    subst.insert(v2, Type::bool());

    let result = exists.apply_subst(&subst);

    if let Type::Exists { body, .. } = result {
        assert_eq!(*body, Type::bool());
    } else {
        panic!("Expected exists type");
    }
}

#[test]
fn test_substitution_forall() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let forall = Type::Forall {
        vars: vec![v1].into(),
        body: Box::new(Type::Var(v2)),
    };

    let mut subst = Substitution::new();
    subst.insert(v1, Type::int()); // Should not substitute bound variable
    subst.insert(v2, Type::bool());

    let result = forall.apply_subst(&subst);

    if let Type::Forall { body, .. } = result {
        assert_eq!(*body, Type::bool());
    } else {
        panic!("Expected forall type");
    }
}

#[test]
fn test_substitution_domain() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let mut subst = Substitution::new();
    subst.insert(v1, Type::int());
    subst.insert(v2, Type::bool());

    let domain = subst.domain();
    assert_eq!(domain.len(), 2);
    assert!(domain.contains(&v1));
    assert!(domain.contains(&v2));
}

// ==================== Complex Type Equality ====================

#[test]
fn test_complex_type_equality() {
    let func1 = Type::function(vec![Type::int(), Type::bool()].into(), Type::unit());
    let func2 = Type::function(vec![Type::int(), Type::bool()].into(), Type::unit());
    assert_eq!(func1, func2);

    let func3 = Type::function(vec![Type::int()].into(), Type::unit());
    assert_ne!(func1, func3);
}

#[test]
fn test_nested_type_equality() {
    let inner1 = Type::tuple(vec![Type::int(), Type::bool()].into());
    let outer1 = Type::array(inner1.clone(), Some(5));

    let inner2 = Type::tuple(vec![Type::int(), Type::bool()].into());
    let outer2 = Type::array(inner2, Some(5));

    assert_eq!(outer1, outer2);
}

// ==================== Edge Cases ====================

#[test]
fn test_monotype_check() {
    assert!(Type::int().is_monotype());
    assert!(Type::function(vec![Type::int()].into(), Type::bool()).is_monotype());

    let v = TypeVar::fresh();
    assert!(!Type::Var(v).is_monotype());
    assert!(!Type::function(vec![Type::Var(v)].into(), Type::bool()).is_monotype());
}

#[test]
fn test_nested_substitution() {
    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();
    let v3 = TypeVar::fresh();

    let ty = Type::tuple(
        vec![
            Type::Var(v1),
            Type::function(vec![Type::Var(v2)].into(), Type::Var(v3)),
        ]
        .into(),
    );

    let mut subst = Substitution::new();
    subst.insert(v1, Type::int());
    subst.insert(v2, Type::bool());
    subst.insert(v3, Type::text());

    let result = ty.apply_subst(&subst);
    let expected = Type::tuple(
        vec![
            Type::int(),
            Type::function(vec![Type::bool()].into(), Type::text()),
        ]
        .into(),
    );

    assert_eq!(result, expected);
}

// ==================== GenRef Types ====================

#[test]
fn test_genref_type_creation() {
    let genref = Type::genref(Type::int());
    assert!(matches!(genref, Type::GenRef { .. }));
}

#[test]
fn test_genref_type_display() {
    let genref = Type::genref(Type::int());
    assert_eq!(genref.to_string(), "GenRef<Int>");
}

#[test]
fn test_genref_nested() {
    let inner = Type::reference(false, Type::text());
    let genref = Type::genref(inner);
    assert_eq!(genref.to_string(), "GenRef<&Text>");
}

#[test]
fn test_genref_free_vars() {
    let v = TypeVar::fresh();
    let genref = Type::genref(Type::Var(v));
    let free = genref.free_vars();
    assert_eq!(free.len(), 1);
    assert!(free.contains(&v));
}

#[test]
fn test_genref_apply_subst() {
    let v = TypeVar::fresh();
    let genref = Type::genref(Type::Var(v));

    let mut subst = Substitution::new();
    subst.insert(v, Type::int());

    let result = genref.apply_subst(&subst);
    assert_eq!(result, Type::genref(Type::int()));
}

#[test]
fn test_genref_complex() {
    // GenRef<List<&mut T>>
    let ident = Ident::new("List", Span::dummy());
    let path = Path::single(ident);
    let v = TypeVar::fresh();
    let inner = Type::Named {
        path,
        args: vec![Type::reference(true, Type::Var(v))].into(),
    };
    let genref = Type::genref(inner);

    let free = genref.free_vars();
    assert_eq!(free.len(), 1);
    assert!(free.contains(&v));
}

// ==================== Type Constructor ====================

#[test]
fn test_type_constructor_unary() {
    use verum_types::advanced_protocols::Kind;

    let list_ctor = Type::type_constructor("List".into(), 1, Kind::unary_constructor());

    assert!(matches!(list_ctor, Type::TypeConstructor { .. }));
}

#[test]
fn test_type_constructor_binary() {
    use verum_types::advanced_protocols::Kind;

    let map_ctor = Type::type_constructor("Map".into(), 2, Kind::binary_constructor());

    if let Type::TypeConstructor { arity, .. } = map_ctor {
        assert_eq!(arity, 2);
    } else {
        panic!("Expected TypeConstructor");
    }
}

#[test]
fn test_type_constructor_display_unary() {
    use verum_types::advanced_protocols::Kind;

    let list_ctor = Type::type_constructor("List".into(), 1, Kind::unary_constructor());

    assert_eq!(list_ctor.to_string(), "List<_>");
}

#[test]
fn test_type_constructor_display_binary() {
    use verum_types::advanced_protocols::Kind;

    let map_ctor = Type::type_constructor("Map".into(), 2, Kind::binary_constructor());

    assert_eq!(map_ctor.to_string(), "Map<_, _>");
}

#[test]
fn test_type_constructor_display_nullary() {
    use verum_types::advanced_protocols::Kind;

    let const_ctor = Type::type_constructor("Const".into(), 0, Kind::type_kind());

    assert_eq!(const_ctor.to_string(), "Const");
}

#[test]
fn test_type_constructor_kind_arity() {
    use verum_types::advanced_protocols::Kind;

    let unary = Kind::unary_constructor();
    assert_eq!(unary.arity(), 1);

    let binary = Kind::binary_constructor();
    assert_eq!(binary.arity(), 2);
}

// ==================== Type Application ====================

#[test]
fn test_type_app_simple() {
    use verum_types::advanced_protocols::Kind;

    let list_ctor = Type::type_constructor("List".into(), 1, Kind::unary_constructor());

    let list_int = Type::type_app(list_ctor, vec![Type::int()].into());
    assert!(matches!(list_int, Type::TypeApp { .. }));
}

#[test]
fn test_type_app_display() {
    use verum_types::advanced_protocols::Kind;

    let list_ctor = Type::type_constructor("List".into(), 1, Kind::unary_constructor());

    let list_int = Type::type_app(list_ctor, vec![Type::int()].into());
    // TypeApp displays as the resolved generic type
    assert_eq!(list_int.to_string(), "List<Int>");
}

#[test]
fn test_type_app_binary() {
    use verum_types::advanced_protocols::Kind;

    let map_ctor = Type::type_constructor("Map".into(), 2, Kind::binary_constructor());

    let map_str_int = Type::type_app(map_ctor, vec![Type::text(), Type::int()].into());

    if let Type::TypeApp { args, .. } = map_str_int {
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], Type::text());
        assert_eq!(args[1], Type::int());
    } else {
        panic!("Expected TypeApp");
    }
}

#[test]
fn test_type_app_free_vars() {
    use verum_types::advanced_protocols::Kind;

    let v1 = TypeVar::fresh();
    let v2 = TypeVar::fresh();

    let map_ctor = Type::type_constructor("Map".into(), 2, Kind::binary_constructor());

    let map_app = Type::type_app(map_ctor, vec![Type::Var(v1), Type::Var(v2)].into());

    let free = map_app.free_vars();
    assert_eq!(free.len(), 2);
    assert!(free.contains(&v1));
    assert!(free.contains(&v2));
}

#[test]
fn test_type_app_apply_subst() {
    use verum_types::advanced_protocols::Kind;

    let v = TypeVar::fresh();

    let list_ctor = Type::type_constructor("List".into(), 1, Kind::unary_constructor());

    let list_var = Type::type_app(list_ctor.clone(), vec![Type::Var(v)].into());

    let mut subst = Substitution::new();
    subst.insert(v, Type::int());

    let result = list_var.apply_subst(&subst);

    if let Type::TypeApp { args, .. } = result {
        assert_eq!(args[0], Type::int());
    } else {
        panic!("Expected TypeApp");
    }
}

#[test]
fn test_type_app_nested() {
    use verum_types::advanced_protocols::Kind;

    // List<Maybe<Int>>
    let list_ctor = Type::type_constructor("List".into(), 1, Kind::unary_constructor());

    let maybe_ctor = Type::type_constructor("Maybe".into(), 1, Kind::unary_constructor());

    let maybe_int = Type::type_app(maybe_ctor, vec![Type::int()].into());
    let list_maybe_int = Type::type_app(list_ctor, vec![maybe_int].into());

    // Should have nested structure
    if let Type::TypeApp { args, .. } = list_maybe_int {
        assert!(matches!(args[0], Type::TypeApp { .. }));
    } else {
        panic!("Expected TypeApp");
    }
}

// ==================== Advanced Protocol Integration ====================

#[test]
fn test_higher_kinded_functor() {
    use verum_types::advanced_protocols::Kind;

    // Simulating: protocol Functor { type F<_> }
    let f_ctor = Type::type_constructor("F".into(), 1, Kind::unary_constructor());

    // F<Int>
    let f_int = Type::type_app(f_ctor.clone(), vec![Type::int()].into());

    // F<Bool>
    let f_bool = Type::type_app(f_ctor, vec![Type::bool()].into());

    assert_ne!(f_int, f_bool);
}

#[test]
fn test_lending_iterator_genref() {
    // Simulating: type Item<T> with GenRef for lending
    let v = TypeVar::fresh();

    // GenRef<&[T]>
    let slice_ref = Type::reference(false, Type::array(Type::Var(v), None));
    let genref_slice = Type::genref(slice_ref);

    let free = genref_slice.free_vars();
    assert_eq!(free.len(), 1);
    assert!(free.contains(&v));
}

#[test]
fn test_monad_pattern() {
    use verum_types::advanced_protocols::Kind;

    // protocol Monad { type M<_>; fn pure<T>(T) -> M<T> }
    let m_ctor = Type::type_constructor("M".into(), 1, Kind::unary_constructor());

    // pure :: T -> M<T>
    let t_var = TypeVar::fresh();
    let m_t = Type::type_app(m_ctor, vec![Type::Var(t_var)].into());

    let pure_type = Type::function(vec![Type::Var(t_var)].into(), m_t);

    let free = pure_type.free_vars();
    assert_eq!(free.len(), 1); // Only t_var should be free
}

// ==================== Rank-2 Polymorphism Tests ====================

#[test]
fn test_rank2_forall_construction() {
    let r_var = TypeVar::fresh();
    let reducer_int = Type::function(vec![Type::Var(r_var), Type::int()].into(), Type::Var(r_var));
    let reducer_text = Type::function(vec![Type::Var(r_var), Type::text()].into(), Type::Var(r_var));
    let rank2 = Type::Forall {
        vars: vec![r_var].into(),
        body: Box::new(Type::function(vec![reducer_int].into(), reducer_text)),
    };
    let free = rank2.free_vars();
    assert!(free.is_empty(), "Rank-2 quantified vars should not be free");
}

#[test]
fn test_rank2_substitution_respects_binding() {
    let r_var = TypeVar::fresh();
    let forall_ty = Type::Forall {
        vars: vec![r_var].into(),
        body: Box::new(Type::function(vec![Type::Var(r_var)].into(), Type::Var(r_var))),
    };
    let mut subst = Substitution::new();
    subst.insert(r_var, Type::int());
    let result = forall_ty.apply_subst(&subst);
    if let Type::Forall { body, .. } = result {
        if let Type::Function { params, return_type, .. } = body.as_ref() {
            assert_eq!(params[0], Type::Var(r_var));
            assert_eq!(*return_type.as_ref(), Type::Var(r_var));
        } else { panic!("Expected Function inside Forall"); }
    } else { panic!("Expected Forall"); }
}

#[test]
fn test_rank2_free_vars_mixed() {
    let r_var = TypeVar::fresh();
    let a_var = TypeVar::fresh();
    let rank2 = Type::Forall {
        vars: vec![r_var].into(),
        body: Box::new(Type::function(
            vec![Type::Var(r_var), Type::Var(a_var)].into(), Type::Var(r_var),
        )),
    };
    let free = rank2.free_vars();
    assert_eq!(free.len(), 1);
    assert!(free.contains(&a_var));
    assert!(!free.contains(&r_var));
}

// ==================== Existential Type Tests ====================

#[test]
fn test_existential_construction() {
    let t_var = TypeVar::fresh();
    let exists = Type::Exists { var: t_var, body: Box::new(Type::Var(t_var)) };
    assert!(exists.free_vars().is_empty());
}

#[test]
fn test_existential_with_constraint() {
    let t_var = TypeVar::fresh();
    let exists = Type::Exists {
        var: t_var,
        body: Box::new(Type::Generic { name: "List".into(), args: vec![Type::Var(t_var)].into() }),
    };
    assert!(exists.free_vars().is_empty());
}

#[test]
fn test_existential_substitution() {
    let t_var = TypeVar::fresh();
    let a_var = TypeVar::fresh();
    let exists = Type::Exists {
        var: t_var,
        body: Box::new(Type::tuple(vec![Type::Var(t_var), Type::Var(a_var)].into())),
    };
    let mut subst = Substitution::new();
    subst.insert(a_var, Type::int());
    let result = exists.apply_subst(&subst);
    if let Type::Exists { body, .. } = result {
        if let Type::Tuple(elems) = body.as_ref() {
            assert_eq!(elems[0], Type::Var(t_var));
            assert_eq!(elems[1], Type::int());
        } else { panic!("Expected Tuple"); }
    } else { panic!("Expected Exists"); }
}

// ==================== HKT Tests ====================

#[test]
fn test_hkt_type_app_with_var_constructor() {
    let f_var = TypeVar::fresh();
    let f_int = Type::TypeApp { constructor: Box::new(Type::Var(f_var)), args: vec![Type::int()].into() };
    assert!(f_int.free_vars().contains(&f_var));
}

#[test]
fn test_hkt_type_app_substitution() {
    use verum_types::advanced_protocols::Kind;
    let f_var = TypeVar::fresh();
    let a_var = TypeVar::fresh();
    let f_a = Type::TypeApp { constructor: Box::new(Type::Var(f_var)), args: vec![Type::Var(a_var)].into() };
    let list_ctor = Type::type_constructor("List".into(), 1, Kind::unary_constructor());
    let mut subst = Substitution::new();
    subst.insert(f_var, list_ctor);
    subst.insert(a_var, Type::int());
    let result = f_a.apply_subst(&subst);
    if let Type::TypeApp { constructor, args } = result {
        assert!(matches!(*constructor, Type::TypeConstructor { .. }));
        assert_eq!(args[0], Type::int());
    } else { panic!("Expected TypeApp"); }
}

// ==================== Skolem Tracker Tests ====================

#[test]
fn test_skolem_tracker_scope() {
    use verum_types::existential::SkolemTracker;
    let mut tracker = SkolemTracker::new();
    tracker.enter_scope();
    let skolem = tracker.create_skolem("sk_test".into(), verum_common::List::new(), verum_ast::span::Span::default());
    assert!(tracker.is_in_scope(skolem.id));
    let exiting = tracker.exit_scope();
    assert_eq!(exiting.len(), 1);
}

#[test]
fn test_skolem_tracker_nested_scope() {
    use verum_types::existential::SkolemTracker;
    let mut tracker = SkolemTracker::new();
    tracker.enter_scope();
    let outer = tracker.create_skolem("outer".into(), verum_common::List::new(), verum_ast::span::Span::default());
    tracker.enter_scope();
    let inner = tracker.create_skolem("inner".into(), verum_common::List::new(), verum_ast::span::Span::default());
    assert!(tracker.is_in_scope(outer.id));
    assert!(tracker.is_in_scope(inner.id));
    tracker.exit_scope();
    assert!(tracker.is_in_scope(outer.id));
    tracker.exit_scope();
}

// ==================== Unification Tests ====================

#[test]
fn test_unify_forall_alpha_rename() {
    use verum_types::unify::Unifier;
    let mut unifier = Unifier::new();
    let a = TypeVar::fresh();
    let b = TypeVar::fresh();
    let fa = Type::Forall { vars: vec![a].into(), body: Box::new(Type::function(vec![Type::Var(a)].into(), Type::Var(a))) };
    let fb = Type::Forall { vars: vec![b].into(), body: Box::new(Type::function(vec![Type::Var(b)].into(), Type::Var(b))) };
    assert!(unifier.unify(&fa, &fb, verum_ast::span::Span::default()).is_ok());
}

#[test]
fn test_unify_exists_alpha_rename() {
    use verum_types::unify::Unifier;
    let mut unifier = Unifier::new();
    let a = TypeVar::fresh();
    let b = TypeVar::fresh();
    let ea = Type::Exists { var: a, body: Box::new(Type::Var(a)) };
    let eb = Type::Exists { var: b, body: Box::new(Type::Var(b)) };
    assert!(unifier.unify(&ea, &eb, verum_ast::span::Span::default()).is_ok());
}

#[test]
fn test_unify_exists_with_concrete() {
    use verum_types::unify::Unifier;
    let mut unifier = Unifier::new();
    let a = TypeVar::fresh();
    let exists = Type::Exists { var: a, body: Box::new(Type::Var(a)) };
    assert!(unifier.unify(&exists, &Type::int(), verum_ast::span::Span::default()).is_ok());
}

#[test]
fn test_unify_type_app_vs_generic() {
    use verum_types::unify::Unifier;
    let mut unifier = Unifier::new();
    let f = TypeVar::fresh();
    let ta = Type::TypeApp { constructor: Box::new(Type::Var(f)), args: vec![Type::int()].into() };
    let li = Type::Generic { name: "List".into(), args: vec![Type::int()].into() };
    assert!(unifier.unify(&ta, &li, verum_ast::span::Span::default()).is_ok());
    let resolved = unifier.apply(&Type::Var(f));
    assert!(matches!(resolved, Type::TypeConstructor { .. }));
}

// ==================== Subtyping Tests ====================

#[test]
fn test_subtype_forall_reflexive() {
    use verum_types::subtype::Subtyping;
    let s = Subtyping::new();
    let a = TypeVar::fresh();
    let ty = Type::Forall { vars: vec![a].into(), body: Box::new(Type::function(vec![].into(), Type::Var(a))) };
    assert!(s.is_subtype(&ty, &ty));
}

#[test]
fn test_subtype_exists_reflexive() {
    use verum_types::subtype::Subtyping;
    let s = Subtyping::new();
    let a = TypeVar::fresh();
    let ty = Type::Exists { var: a, body: Box::new(Type::Var(a)) };
    assert!(s.is_subtype(&ty, &ty));
}

#[test]
fn test_subtype_concrete_to_existential() {
    use verum_types::subtype::Subtyping;
    let s = Subtyping::new();
    let a = TypeVar::fresh();
    let exists = Type::Exists { var: a, body: Box::new(Type::Var(a)) };
    assert!(s.is_subtype(&Type::int(), &exists));
}
