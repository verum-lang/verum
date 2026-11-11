use verum_ast::{Ident, Path, span::Span};
use verum_common::List;
use verum_types::{Substitution, Type, TypeVar, Unifier};

fn nested(mut inner: Type, depth: usize, wrapper: fn(Type) -> Type) -> Type {
    for _ in 0..depth {
        inner = wrapper(inner);
    }
    inner
}

fn nominal(inner: Type) -> Type {
    Type::Named {
        path: Path::from_ident(Ident::new("Envelope", Span::dummy())),
        args: List::from_iter([inner]),
    }
}

fn ordinary_stack(check: impl FnOnce() + Send + 'static) {
    // The repository test runner sets RUST_MIN_STACK. Exercise an ordinary
    // 2 MiB worker explicitly, so that setting cannot mask this regression.
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(check)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn type_nodes_do_not_embed_the_predicate_ast() {
    assert!(
        std::mem::size_of::<Type>() <= 320,
        "embedding syntax in Type amplifies every primitive, container and inference temporary"
    );
}

#[test]
fn dynamic_associated_types_keep_their_keys_during_reconstruction() {
    let mut substitution = Substitution::new();
    let mut bindings = verum_common::Map::new();
    let mut expected = verum_common::Map::new();
    for i in 0..32 {
        let variable = TypeVar::fresh();
        let name = verum_common::Text::from(format!("Member{i}"));
        let value = nested(Type::Bool, i % 20, nominal);
        substitution.insert(variable, value.clone());
        bindings.insert(name.clone(), Type::Var(variable));
        expected.insert(name, value);
    }
    let ty = Type::DynProtocol {
        bounds: List::new(),
        bindings,
    };
    let actual = ty.try_apply_subst(&substitution).unwrap();
    assert_eq!(
        actual,
        Type::DynProtocol {
            bounds: List::new(),
            bindings: expected
        }
    );
}

#[test]
fn application_reduction_reenters_the_checked_work_queue() {
    let variable = TypeVar::fresh();
    let constructor = nominal(Type::Var(variable));
    let argument = Type::Bool;
    let application = Type::TypeApp {
        constructor: Box::new(constructor),
        args: List::from_iter([argument]),
    };
    let actual = application.try_apply_subst(&Substitution::new()).unwrap();
    assert_eq!(actual, nominal(Type::Bool));

    // Unresolved associated projections are identities, not constructors that
    // can silently discard their receiver when applied with zero arguments.
    let projection = Type::TypeApp {
        constructor: Box::new(Type::Generic {
            name: "::Member".into(),
            args: List::from_iter([Type::Bool]),
        }),
        args: List::new(),
    };
    assert_eq!(
        projection.try_apply_subst(&Substitution::new()).unwrap(),
        projection
    );
}

#[test]
fn substitution_preserves_nested_carriers_on_an_ordinary_stack() {
    ordinary_stack(|| {
        let variable = TypeVar::fresh();
        let mut substitution = Substitution::new();
        substitution.insert(variable, Type::Bool);
        let wrappers: [fn(Type) -> Type; 4] = [
            nominal,
            |ty| Type::Reference {
                mutable: false,
                inner: Box::new(ty),
            },
            |ty| Type::Tuple(List::from_iter([ty, Type::Int])),
            |ty| Type::TypeApp {
                constructor: Box::new(ty),
                args: List::new(),
            },
        ];
        for wrapper in wrappers {
            let ty = nested(Type::Var(variable), 20, wrapper);
            let actual = ty.try_apply_subst(&substitution).unwrap();
            assert_eq!(actual, nested(Type::Bool, 20, wrapper));
            let excessive = nested(Type::Var(variable), 40, wrapper);
            assert!(
                excessive
                    .try_apply_subst(&substitution)
                    .unwrap_err()
                    .contains("structural depth budget")
            );
        }
    });
}

#[test]
fn unification_solves_and_rejects_nested_carriers_on_an_ordinary_stack() {
    ordinary_stack(|| {
        for wrapper in [
            nominal as fn(Type) -> Type,
            |ty| Type::Reference {
                mutable: false,
                inner: Box::new(ty),
            },
            |ty| Type::Tuple(List::from_iter([ty, Type::Int])),
        ] {
            let variable = TypeVar::fresh();
            let left = nested(Type::Var(variable), 20, wrapper);
            let right = nested(Type::Bool, 20, wrapper);
            let substitution = Unifier::new().unify(&left, &right, Span::dummy()).unwrap();
            assert_eq!(substitution.get(&variable), Some(&Type::Bool));
            let wrong = nested(Type::Int, 20, wrapper);
            assert!(Unifier::new().unify(&right, &wrong, Span::dummy()).is_err());
        }
    });
}
