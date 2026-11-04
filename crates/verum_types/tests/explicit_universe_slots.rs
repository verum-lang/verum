//! Source-owned explicit level slots share the scheme's actual substitution.
use verum_ast::Span;
use verum_common::{Heap, List, Map};
use verum_types::{
    context::{GenericBinder, InstantiatedScheme, TypeScheme},
    ty::{SubstitutionExt, Type, TypeVar},
    unify::Unifier,
    universe_level::{UniverseLevel as Level, UniverseMeta},
};
fn scheme() -> TypeScheme {
    let mut scheme = TypeScheme::mono(Type::function(
        List::from_single(Type::universe(Level::Variable(7))),
        Type::universe(Level::Variable(7)),
    ))
    .with_universe_parameters(List::from_single(7))
    .unwrap();
    scheme.explicit_method_vars = Some(List::from_single(Some(GenericBinder::Universe(
        Level::Variable(7),
    ))));
    scheme
}
fn slot(instance: &InstantiatedScheme) -> Level {
    let Some(GenericBinder::Universe(level)) = &instance.explicit_method_vars.as_ref().unwrap()[0]
    else {
        panic!("owned level slot")
    };
    level.clone()
}
#[test]
fn explicit_slot_uses_the_signature_receiver_and_bound_substitution() {
    let owner = TypeVar::fresh();
    let source = Type::universe(Level::Variable(7));
    let mut bounds = Map::new();
    bounds.insert(
        owner,
        List::from_single(Type::function(List::new(), source.clone())),
    );
    let mut scheme = TypeScheme::poly(
        List::from_single(owner),
        Type::function(List::from_single(source.clone()), source.clone()),
    )
    .with_type_bounds(bounds)
    .with_universe_parameters(List::from_single(7))
    .unwrap();
    scheme.impl_self_type = Some(source);
    scheme.explicit_method_vars = Some(List::from_single(Some(GenericBinder::Universe(
        Level::Variable(7),
    ))));
    let instance = scheme.try_instantiate().unwrap();
    let level = slot(&instance);
    assert!(matches!(level, Level::Meta(_)));
    assert_eq!(
        level,
        instance
            .universe_instantiation()
            .apply(&Level::Variable(7))
            .unwrap()
    );
    let Type::Function {
        params,
        return_type,
        ..
    } = &instance.ty
    else {
        panic!("function")
    };
    assert_eq!(params[0], Type::universe(level.clone()));
    assert_eq!(**return_type, Type::universe(level.clone()));
    assert_eq!(instance.impl_self_type, Some(Type::universe(level.clone())));
    let Type::Function { return_type, .. } = &instance.type_bounds[&instance.fresh_vars[0]][0]
    else {
        panic!("bound")
    };
    assert_eq!(**return_type, Type::universe(level));
}
#[test]
fn independent_instances_do_not_share_explicit_level_inference() {
    let scheme = scheme();
    let first = scheme.try_instantiate().unwrap();
    let second = scheme.try_instantiate().unwrap();
    assert_ne!(slot(&first), slot(&second));
    assert_eq!(
        scheme.explicit_method_vars,
        Some(List::from_single(Some(GenericBinder::Universe(
            Level::Variable(7)
        ))))
    );
}
#[test]
fn explicit_level_slot_requires_its_quantified_owner() {
    let mut scheme = scheme();
    scheme.explicit_method_vars = Some(List::from_single(Some(GenericBinder::Universe(
        Level::Variable(8),
    ))));
    assert!(scheme.try_instantiate().is_err());
}
#[test]
fn explicit_level_slot_cannot_be_a_constant_metavariable_or_expression() {
    for level in [
        Level::Concrete(0),
        Level::Meta(UniverseMeta::fresh()),
        Level::Variable(7).succ(),
        Level::Max(Heap::new(Level::Variable(7)), Heap::new(Level::Concrete(0))),
    ] {
        let mut scheme = scheme();
        scheme.explicit_method_vars = Some(List::from_single(Some(GenericBinder::Universe(level))));
        assert!(scheme.try_instantiate().is_err());
    }
}
#[test]
fn explicit_level_constraint_updates_the_actual_body() {
    let instance = scheme().try_instantiate().unwrap();
    let constraint = Unifier::new()
        .unify(
            &Type::universe(slot(&instance)),
            &Type::universe(Level::Concrete(2)),
            Span::default(),
        )
        .unwrap();
    let expected = Type::function(
        List::from_single(Type::universe(Level::Concrete(2))),
        Type::universe(Level::Concrete(2)),
    );
    assert_eq!(instance.ty.try_apply_subst(&constraint).unwrap(), expected);
}
#[test]
fn conflicting_explicit_and_inferred_level_constraints_cannot_be_composed() {
    let instance = scheme().try_instantiate().unwrap();
    let variable = Type::universe(slot(&instance));
    let first = Unifier::new()
        .unify(
            &variable,
            &Type::universe(Level::Concrete(1)),
            Span::default(),
        )
        .unwrap();
    let second = Unifier::new()
        .unify(
            &variable,
            &Type::universe(Level::Concrete(2)),
            Span::default(),
        )
        .unwrap();
    assert!(first.compose(&second).is_err());
}
#[test]
fn forwarded_caller_level_stays_rigid() {
    let instance = scheme().try_instantiate().unwrap();
    let caller = Type::universe(Level::Variable(42));
    let constraint = Unifier::new()
        .unify(&Type::universe(slot(&instance)), &caller, Span::default())
        .unwrap();
    let Type::Function { return_type, .. } = instance.ty.try_apply_subst(&constraint).unwrap()
    else {
        panic!("function")
    };
    assert_eq!(*return_type, caller);
    assert!(
        Unifier::new()
            .unify(
                &caller,
                &Type::universe(Level::Concrete(0)),
                Span::default()
            )
            .is_err()
    );
}
