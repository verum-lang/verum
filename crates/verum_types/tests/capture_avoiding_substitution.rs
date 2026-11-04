//! Free-variable preservation under the shared checked type substitution.
use verum_common::{Heap, List, Set};
use verum_types::ty::{Substitution, Type, TypeVar};

fn tuple(types: impl IntoIterator<Item = Type>) -> Type {
    Type::Tuple(types.into_iter().collect())
}
fn forall(vars: impl IntoIterator<Item = TypeVar>, body: Type) -> Type {
    Type::Forall {
        vars: vars.into_iter().collect(),
        body: Heap::new(body),
    }
}
fn substitution(variable: TypeVar, replacement: Type) -> Substitution {
    [(variable, replacement)].into_iter().collect()
}
fn free(ty: &Type, expected: impl IntoIterator<Item = TypeVar>) {
    assert_eq!(ty.free_vars(), expected.into_iter().collect(), "{ty:?}");
}

#[test]
fn forall_substitution_does_not_capture_the_replacements_free_variable() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let ty = forall([beta], tuple([Type::Var(alpha), Type::Var(beta)]));
    let actual = ty
        .try_apply_subst(&substitution(alpha, Type::Var(beta)))
        .unwrap();
    free(&actual, [beta]);
    let Type::Forall { vars, body } = actual else {
        panic!()
    };
    assert_ne!(vars[0], beta);
    assert_eq!(*body, tuple([Type::Var(beta), Type::Var(vars[0])]));
}

#[test]
fn existential_substitution_uses_the_same_capture_avoiding_rule() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let ty = Type::Exists {
        var: beta,
        body: Heap::new(tuple([Type::Var(alpha), Type::Var(beta)])),
    };
    let actual = ty
        .try_apply_subst(&substitution(alpha, Type::Var(beta)))
        .unwrap();
    free(&actual, [beta]);
    let Type::Exists { var, body } = actual else {
        panic!()
    };
    assert_ne!(var, beta);
    assert_eq!(*body, tuple([Type::Var(beta), Type::Var(var)]));
}

#[test]
fn nested_shadowing_keeps_each_local_binding_distinct_from_inserted_free_values() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let ty = forall(
        [beta],
        tuple([
            Type::Var(beta),
            forall([beta], tuple([Type::Var(alpha), Type::Var(beta)])),
        ]),
    );
    let actual = ty
        .try_apply_subst(&substitution(alpha, Type::Var(beta)))
        .unwrap();
    free(&actual, [beta]);
    let Type::Forall { vars: outer, body } = actual else {
        panic!()
    };
    let Type::Tuple(parts) = *body else { panic!() };
    let Type::Forall { vars: inner, body } = &parts[1] else {
        panic!()
    };
    assert_ne!(outer[0], inner[0]);
    assert_ne!(outer[0], beta);
    assert_ne!(inner[0], beta);
    assert_eq!(parts[0], Type::Var(outer[0]));
    assert_eq!(**body, tuple([Type::Var(beta), Type::Var(inner[0])]));
}

#[test]
fn an_irrelevant_range_does_not_rename_a_binder() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let ty = forall([beta], Type::Var(beta));
    assert_eq!(
        ty.try_apply_subst(&substitution(alpha, Type::Var(beta)))
            .unwrap(),
        ty
    );
    assert_eq!(ty.try_apply_subst(&Substitution::new()).unwrap(), ty);
}

#[test]
fn a_closed_replacement_does_not_rename_a_same_spelled_outer_binder() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let replacement = forall([beta], Type::Var(beta));
    let ty = forall([beta], tuple([Type::Var(alpha), Type::Var(beta)]));
    let actual = ty
        .try_apply_subst(&substitution(alpha, replacement.clone()))
        .unwrap();
    assert_eq!(
        actual,
        forall([beta], tuple([replacement, Type::Var(beta)]))
    );
    free(&actual, []);
}

#[test]
fn simultaneous_substitution_protects_bound_keys_without_rewriting_replacement_ranges() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let ty = forall([beta], tuple([Type::Var(alpha), Type::Var(beta)]));
    let mapping = [(alpha, Type::Var(beta)), (beta, Type::Int)]
        .into_iter()
        .collect();
    let actual = ty.try_apply_subst(&mapping).unwrap();
    free(&actual, [beta]);
    let Type::Forall { vars, body } = actual else {
        panic!()
    };
    assert_eq!(*body, tuple([Type::Var(beta), Type::Var(vars[0])]));
}

#[test]
fn substitution_without_capture_keeps_existing_binder_ids() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let gamma = TypeVar::fresh();
    let ty = forall(
        [beta, gamma],
        tuple([Type::Var(alpha), Type::Var(beta), Type::Var(gamma)]),
    );
    let actual = ty
        .try_apply_subst(&substitution(alpha, Type::Bool))
        .unwrap();
    assert_eq!(
        actual,
        forall(
            [beta, gamma],
            tuple([Type::Bool, Type::Var(beta), Type::Var(gamma)])
        )
    );
}

#[test]
fn freshening_avoids_manually_constructed_free_and_nested_bound_ids() {
    let start = TypeVar::peek_counter();
    let occupied: List<_> = (start..start + 64).map(TypeVar::with_id).collect();
    let alpha = occupied[0];
    let beta = occupied[1];
    let nested: List<_> = occupied.iter().skip(2).copied().collect();
    let ty = forall(
        [beta],
        tuple([
            Type::Var(alpha),
            Type::Var(beta),
            forall(nested.iter().copied(), Type::Int),
        ]),
    );
    let actual = ty
        .try_apply_subst(&substitution(alpha, Type::Var(beta)))
        .unwrap();
    free(&actual, [beta]);
    let Type::Forall { vars, body } = actual else {
        panic!()
    };
    assert!(
        !occupied.contains(&vars[0]),
        "fresh binder collided with an occupied ID"
    );
    let Type::Tuple(parts) = *body else { panic!() };
    assert_eq!(
        parts[2],
        forall(nested, Type::Int),
        "unaffected nested binders must remain unchanged"
    );
}

#[test]
fn renaming_a_quantified_row_variable_updates_the_row_occurrence() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let ty = forall(
        [beta],
        tuple([
            Type::Var(alpha),
            Type::ExtensibleRecord {
                fields: Default::default(),
                row_var: Some(beta),
            },
        ]),
    );
    let actual = ty
        .try_apply_subst(&substitution(alpha, Type::Var(beta)))
        .unwrap();
    free(&actual, [beta]);
    let Type::Forall { vars, body } = actual else {
        panic!()
    };
    let Type::Tuple(parts) = *body else { panic!() };
    let Type::ExtensibleRecord { row_var, .. } = parts[1] else {
        panic!()
    };
    assert_eq!(row_var, Some(vars[0]));
}

#[test]
fn capture_analysis_respects_the_checked_substitution_depth_budget() {
    let alpha = TypeVar::fresh();
    let beta = TypeVar::fresh();
    let mut body = Type::Var(alpha);
    for _ in 0..40 {
        body = Type::Reference {
            mutable: false,
            inner: Heap::new(body),
        };
    }
    let error = forall([beta], body)
        .try_apply_subst(&substitution(alpha, Type::Var(beta)))
        .unwrap_err();
    assert!(error.contains("depth budget"), "{error}");
}

#[test]
fn free_variable_law_holds_for_a_bounded_family_of_nested_types() {
    let variables: List<_> = (0..4).map(|_| TypeVar::fresh()).collect();
    for outer in &variables {
        for inner in &variables {
            for domain in &variables {
                for range in &variables {
                    let ty = forall(
                        [*outer],
                        tuple([
                            Type::Var(*domain),
                            Type::Exists {
                                var: *inner,
                                body: Heap::new(tuple([Type::Var(*domain), Type::Var(*range)])),
                            },
                        ]),
                    );
                    let mapping = substitution(*domain, Type::Var(*range));
                    let expected: Set<_> = ty
                        .free_vars()
                        .into_iter()
                        .flat_map(|variable| {
                            mapping
                                .get(&variable)
                                .map_or_else(|| [variable].into_iter().collect(), Type::free_vars)
                        })
                        .collect();
                    let actual = ty.try_apply_subst(&mapping).unwrap();
                    assert_eq!(
                        actual.free_vars(),
                        expected,
                        "{ty:?} under {mapping:?}: {actual:?}"
                    );
                }
            }
        }
    }
}
