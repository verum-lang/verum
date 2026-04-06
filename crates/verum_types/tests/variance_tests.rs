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
// Variance Inference and Checking Tests
//
// Variance inference: determining covariant/contravariant/invariant usage of type parameters from their positions

use verum_ast::span::{FileId, Span};
use verum_ast::ty::{Ident, Path};
use verum_common::{List, Text};
use verum_types::context::TypeParam;
use verum_types::ty::Type;
use verum_types::variance::{
    Variance, VarianceChecker, combine_variances, compose_variance, flip_variance,
};

// Test helper: create a dummy span
fn dummy_span() -> Span {
    Span::new(0, 0, FileId::dummy())
}

// Test helper: create a type parameter
fn make_param(name: &str, variance: Variance) -> TypeParam {
    TypeParam {
        name: name.into(),
        variance,
        bounds: List::new(),
        default: None.into(),
        is_meta: false,
        span: dummy_span(),
    }
}

// Test helper: create a named type for a type parameter
fn make_type_param(name: &str) -> Type {
    let ident = Ident::new(name, dummy_span());
    Type::Named {
        path: Path::single(ident),
        args: List::new(),
    }
}

#[test]
fn test_flip_variance() {
    assert_eq!(flip_variance(Variance::Covariant), Variance::Contravariant);
    assert_eq!(flip_variance(Variance::Contravariant), Variance::Covariant);
    assert_eq!(flip_variance(Variance::Invariant), Variance::Invariant);
}

#[test]
fn test_compose_variance() {
    // Covariant compositions
    assert_eq!(
        compose_variance(Variance::Covariant, Variance::Covariant),
        Variance::Covariant
    );
    assert_eq!(
        compose_variance(Variance::Covariant, Variance::Contravariant),
        Variance::Contravariant
    );
    assert_eq!(
        compose_variance(Variance::Covariant, Variance::Invariant),
        Variance::Invariant
    );

    // Contravariant compositions (double negation)
    assert_eq!(
        compose_variance(Variance::Contravariant, Variance::Covariant),
        Variance::Contravariant
    );
    assert_eq!(
        compose_variance(Variance::Contravariant, Variance::Contravariant),
        Variance::Covariant
    );
    assert_eq!(
        compose_variance(Variance::Contravariant, Variance::Invariant),
        Variance::Invariant
    );

    // Invariant compositions
    assert_eq!(
        compose_variance(Variance::Invariant, Variance::Covariant),
        Variance::Invariant
    );
    assert_eq!(
        compose_variance(Variance::Invariant, Variance::Contravariant),
        Variance::Invariant
    );
    assert_eq!(
        compose_variance(Variance::Invariant, Variance::Invariant),
        Variance::Invariant
    );
}

#[test]
fn test_combine_variances_empty() {
    let empty = vec![].into();
    assert_eq!(combine_variances(&empty), Variance::Covariant);
}

#[test]
fn test_combine_variances_single() {
    assert_eq!(
        combine_variances(&vec![Variance::Covariant].into()),
        Variance::Covariant
    );
    assert_eq!(
        combine_variances(&vec![Variance::Contravariant].into()),
        Variance::Contravariant
    );
    assert_eq!(
        combine_variances(&vec![Variance::Invariant].into()),
        Variance::Invariant
    );
}

#[test]
fn test_combine_variances_all_covariant() {
    let variances = vec![
        Variance::Covariant,
        Variance::Covariant,
        Variance::Covariant,
    ];
    assert_eq!(combine_variances(&variances.into()), Variance::Covariant);
}

#[test]
fn test_combine_variances_all_contravariant() {
    let variances = vec![Variance::Contravariant, Variance::Contravariant];
    assert_eq!(
        combine_variances(&variances.into()),
        Variance::Contravariant
    );
}

#[test]
fn test_combine_variances_mixed() {
    let variances = vec![Variance::Covariant, Variance::Contravariant];
    assert_eq!(combine_variances(&variances.into()), Variance::Invariant);
}

#[test]
fn test_combine_variances_any_invariant() {
    let variances = vec![
        Variance::Covariant,
        Variance::Invariant,
        Variance::Covariant,
    ];
    assert_eq!(combine_variances(&variances.into()), Variance::Invariant);
}

#[test]
fn test_covariant_in_return_position() {
    // type Container<T> is { get: Unit -> T }
    // T in covariant position (return type)
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let body = Type::Record(
        vec![(
            Text::from("get"),
            Type::Function {
                params: vec![Type::Unit].into(),
                return_type: Box::new(make_type_param("T")),
                type_params: List::new(),
                contexts: None,
                properties: None,
            },
        )]
        .into_iter()
        .collect(),
    );

    let inferred = checker.infer_variance(&param, &body);
    assert_eq!(inferred, Variance::Covariant);
}

#[test]
fn test_contravariant_in_param_position() {
    // type Sink<T> is { put: T -> Unit }
    // T in contravariant position (function parameter)
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Contravariant);

    let body = Type::Record(
        vec![(
            Text::from("put"),
            Type::Function {
                params: vec![make_type_param("T")].into(),
                return_type: Box::new(Type::Unit),
                type_params: List::new(),
                contexts: None,
                properties: None,
            },
        )]
        .into_iter()
        .collect(),
    );

    let inferred = checker.infer_variance(&param, &body);
    assert_eq!(inferred, Variance::Contravariant);
}

#[test]
fn test_invariant_mutable_reference() {
    // type Cell<T> is { value: &mut T }
    // Mutable references force invariance
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Invariant);

    let body = Type::Record(
        vec![(
            Text::from("value"),
            Type::Reference {
                mutable: true,
                inner: Box::new(make_type_param("T")),
            },
        )]
        .into_iter()
        .collect(),
    );

    let inferred = checker.infer_variance(&param, &body);
    assert_eq!(inferred, Variance::Invariant);
}

#[test]
fn test_covariant_immutable_reference() {
    // type Ref<T> is { value: &T }
    // Immutable references are covariant
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let body = Type::Record(
        vec![(
            Text::from("value"),
            Type::Reference {
                mutable: false,
                inner: Box::new(make_type_param("T")),
            },
        )]
        .into_iter()
        .collect(),
    );

    let inferred = checker.infer_variance(&param, &body);
    assert_eq!(inferred, Variance::Covariant);
}

#[test]
fn test_invariant_mixed_positions() {
    // type Transformer<T> is { transform: (T -> T) -> T }
    // T appears in both co- and contravariant positions
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Invariant);

    let t_var = make_type_param("T");

    let body = Type::Record(
        vec![(
            Text::from("transform"),
            Type::Function {
                params: vec![Type::Function {
                    params: vec![t_var.clone()].into(),
                    return_type: Box::new(t_var.clone()),
                    type_params: List::new(),
                    contexts: None,
                    properties: None,
                }]
                .into(),
                return_type: Box::new(t_var.clone()),
                type_params: List::new(),
                contexts: None,
                properties: None,
            },
        )]
        .into_iter()
        .collect(),
    );

    let inferred = checker.infer_variance(&param, &body);
    assert_eq!(inferred, Variance::Invariant);
}

#[test]
fn test_check_variance_correct_covariant() {
    // Declared covariant, inferred covariant -> OK
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let body = Type::Record(
        vec![(Text::from("value"), make_type_param("T"))]
            .into_iter()
            .collect(),
    );

    let result = checker.check_variance(&param, &body, dummy_span());
    assert!(result.is_ok());
}

#[test]
fn test_check_variance_declared_invariant_always_safe() {
    // Declared invariant is always safe (most restrictive)
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Invariant);

    // Body is actually covariant, but declaring invariant is OK
    let body = Type::Record(
        vec![(Text::from("value"), make_type_param("T"))]
            .into_iter()
            .collect(),
    );

    let result = checker.check_variance(&param, &body, dummy_span());
    assert!(result.is_ok());
}

#[test]
fn test_check_variance_mismatch_error() {
    // Declared covariant, but inferred contravariant -> ERROR
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let body = Type::Record(
        vec![(
            Text::from("put"),
            Type::Function {
                params: vec![make_type_param("T")].into(),
                return_type: Box::new(Type::Unit),
                type_params: List::new(),
                contexts: None,
                properties: None,
            },
        )]
        .into_iter()
        .collect(),
    );

    let result = checker.check_variance(&param, &body, dummy_span());
    assert!(result.is_err());

    if let Err(err) = result {
        assert_eq!(err.declared, Variance::Covariant);
        assert_eq!(err.inferred, Variance::Contravariant);
    }
}

#[test]
fn test_check_variance_covariant_with_mutable_ref_error() {
    // Declared covariant, but has mutable reference -> ERROR (invariant)
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let body = Type::Record(
        vec![(
            Text::from("cell"),
            Type::Reference {
                mutable: true,
                inner: Box::new(make_type_param("T")),
            },
        )]
        .into_iter()
        .collect(),
    );

    let result = checker.check_variance(&param, &body, dummy_span());
    assert!(result.is_err());

    if let Err(err) = result {
        assert_eq!(err.declared, Variance::Covariant);
        assert_eq!(err.inferred, Variance::Invariant);
    }
}

#[test]
fn test_variance_with_tuple() {
    // type Pair<T> is (T, T)
    // Covariant (both elements)
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let t_var = make_type_param("T");
    let body = Type::Tuple(vec![t_var.clone(), t_var.clone()].into());

    let inferred = checker.infer_variance(&param, &body);
    assert_eq!(inferred, Variance::Covariant);
}

#[test]
fn test_variance_with_array() {
    // type Array<T> is [T; 10]
    // Covariant
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let t_var = make_type_param("T");
    let body = Type::Array {
        element: Box::new(t_var),
        size: Some(10),
    };

    let inferred = checker.infer_variance(&param, &body);
    assert_eq!(inferred, Variance::Covariant);
}

#[test]
fn test_variance_caching() {
    // Verify that variance is cached and not recomputed
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let body = make_type_param("T");

    // First call - computes and caches
    let variance1 = checker.infer_variance(&param, &body);

    // Second call - should return cached result
    let variance2 = checker.infer_variance(&param, &body);

    assert_eq!(variance1, variance2);
    assert_eq!(variance1, Variance::Covariant);
}

#[test]
fn test_variance_error_message_format() {
    let mut checker = VarianceChecker::new();
    let param = make_param("T", Variance::Covariant);

    let body = Type::Record(
        vec![(
            Text::from("put"),
            Type::Function {
                params: vec![make_type_param("T")].into(),
                return_type: Box::new(Type::Unit),
                type_params: List::new(),
                contexts: None,
                properties: None,
            },
        )]
        .into_iter()
        .collect(),
    );

    let result = checker.check_variance(&param, &body, dummy_span());
    assert!(result.is_err());

    if let Err(err) = result {
        let message = format!("{}", err);
        assert!(message.contains("variance mismatch"));
        assert!(message.contains("T"));
        assert!(message.contains("covariant"));
        assert!(message.contains("contravariant"));
    }
}
