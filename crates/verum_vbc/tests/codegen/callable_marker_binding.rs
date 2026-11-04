//! Generic witness identity is invariant even where source call coercion is valid.
use super::bind_generic_free;
use crate::types::{CbgrTier, Mutability, TypeId, TypeParamId, TypeRef};
use std::collections::BTreeMap;

fn callable(is_async: bool, output: TypeRef) -> TypeRef {
    TypeRef::Function {
        is_async,
        params: Vec::new(),
        return_type: Box::new(output),
        contexts: Default::default(),
    }
}

fn exact_in(shape: impl Fn(TypeRef) -> TypeRef) {
    for expected_async in [false, true] {
        for actual_async in [false, true] {
            let formal = shape(callable(expected_async, TypeRef::Generic(TypeParamId(0))));
            let actual = shape(callable(actual_async, TypeRef::Concrete(TypeId::INT)));
            let mut bindings = BTreeMap::new();
            bind_generic_free(&formal, &actual, &mut bindings);
            if expected_async == actual_async {
                assert_eq!(bindings.get(&0), Some(&TypeRef::Concrete(TypeId::INT)));
                assert_eq!(bindings.len(), 1);
            } else {
                assert!(
                    bindings.is_empty(),
                    "structural binding cannot invent callable coercion: {formal:?} vs {actual:?}"
                );
            }
        }
    }
}

#[test]
fn top_level_binding_requires_the_actual_callable_marker() {
    exact_in(|ty| ty);
}

#[test]
fn nominal_arguments_do_not_inherit_callable_assignability() {
    exact_in(|ty| TypeRef::Instantiated {
        base: TypeId(3000),
        args: vec![ty],
    });
}

#[test]
fn reference_targets_preserve_exact_callable_identity() {
    for mutability in [Mutability::Immutable, Mutability::Mutable] {
        exact_in(|ty| TypeRef::Reference {
            inner: Box::new(ty),
            mutability,
            tier: CbgrTier::Tier0,
        });
    }
}

#[test]
fn nested_parameter_binding_cannot_assume_covariant_marker_coercion() {
    exact_in(|ty| TypeRef::Function {
        is_async: false,
        params: vec![ty],
        return_type: Box::new(TypeRef::Concrete(TypeId::UNIT)),
        contexts: Default::default(),
    });
}
