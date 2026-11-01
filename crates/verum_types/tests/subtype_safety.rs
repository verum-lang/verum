//! Subtyping must preserve evidence, callable contracts and mutable invariants.

use verum_ast::{
    Expr,
    expr::{BinOp, ExprKind},
    literal::Literal,
    span::Span,
    ty::{Ident, Path, PathSegment},
};
use verum_common::List;
use verum_types::{
    computational_properties::{ComputationalProperty, PropertySet},
    context::TypeParam,
    di::requirement::{ContextExpr, ContextRef, ContextRequirement},
    protocol::ProtocolBound,
    refinement::RefinementPredicate,
    subtype::Subtyping,
    ty::{Type, TypeVar},
};

fn path(name: &str) -> Path {
    Path::new(
        name.split('.')
            .map(|segment| PathSegment::Name(Ident::new(segment, Span::dummy())))
            .collect(),
        Span::dummy(),
    )
}

fn refined(predicate: Expr) -> Type {
    Type::refined(
        Type::int(),
        RefinementPredicate::new(predicate, "x".into(), Span::dummy()),
    )
}

fn truth(value: bool) -> Type {
    refined(Expr::literal(Literal::bool(value, Span::dummy())))
}

fn greater_than(bound: i128) -> Type {
    let span = Span::dummy();
    refined(Expr::new(
        ExprKind::Binary {
            op: BinOp::Gt,
            left: Box::new(Expr::ident(Ident::new("x", span))),
            right: Box::new(Expr::literal(Literal::int(bound, span))),
        },
        span,
    ))
}

fn constructor(name: &str, args: List<Type>, named: bool) -> Type {
    if named {
        Type::Named {
            path: path(name),
            args,
        }
    } else {
        Type::Generic {
            name: name.into(),
            args,
        }
    }
}

fn callable(properties: Option<PropertySet>, type_params: List<TypeParam>) -> Type {
    Type::Function {
        params: vec![Type::int()].into(),
        return_type: Box::new(Type::int()),
        contexts: None,
        properties,
        type_params,
    }
}

#[test]
fn a_base_type_does_not_prove_a_refinement() {
    let subtyping = Subtyping::new();
    assert!(!subtyping.is_subtype(&Type::int(), &greater_than(0)));
    assert!(!subtyping.is_subtype(&Type::int(), &truth(false)));
    assert!(subtyping.is_subtype(&Type::int(), &truth(true)));
    assert!(subtyping.is_subtype(&greater_than(0), &Type::int()));
    assert!(subtyping.is_subtype(&Type::Never, &greater_than(0)));
}

#[test]
fn a_true_refinement_does_not_prove_an_arbitrary_predicate() {
    let subtyping = Subtyping::new();
    let unresolved = refined(Expr::ident(Ident::new("unproved", Span::dummy())));
    for target in [greater_than(0), truth(false), unresolved] {
        assert!(!subtyping.is_subtype(&truth(true), &target));
    }
    assert!(subtyping.is_subtype(&greater_than(0), &truth(true)));
    assert!(subtyping.is_subtype(&truth(false), &greater_than(0)));
    assert!(subtyping.is_subtype(&greater_than(10), &greater_than(0)));
    assert!(!subtyping.is_subtype(&greater_than(0), &greater_than(10)));
}

#[test]
fn callable_subtyping_cannot_erase_computational_properties() {
    let subtyping = Subtyping::new();
    let pure = callable(Some(PropertySet::pure()), List::new());
    for property in [
        ComputationalProperty::IO,
        ComputationalProperty::Mutates,
        ComputationalProperty::Async,
        ComputationalProperty::Divergent,
    ] {
        let impure = callable(Some(PropertySet::single(property)), List::new());
        assert!(!subtyping.is_subtype(&impure, &pure));
    }
    let io = callable(
        Some(PropertySet::single(ComputationalProperty::IO)),
        List::new(),
    );
    let io_and_mutation = callable(
        Some(PropertySet::from_properties([
            ComputationalProperty::IO,
            ComputationalProperty::Mutates,
        ])),
        List::new(),
    );
    assert!(subtyping.is_subtype(&pure, &io));
    assert!(subtyping.is_subtype(&io, &io_and_mutation));
    assert!(!subtyping.is_subtype(&io_and_mutation, &io));
}

#[test]
fn uninferred_callable_properties_are_not_a_contract_proof() {
    let subtyping = Subtyping::new();
    let unknown = callable(None, List::new());
    for properties in [
        PropertySet::pure(),
        PropertySet::single(ComputationalProperty::IO),
    ] {
        let known = callable(Some(properties), List::new());
        assert!(!subtyping.is_subtype(&unknown, &known));
        assert!(!subtyping.is_subtype(&known, &unknown));
    }
    assert!(subtyping.is_subtype(&unknown, &unknown));
}

#[test]
fn callable_subtyping_preserves_generic_binders_and_bounds() {
    let subtyping = Subtyping::new();
    let parameter = TypeParam::new("T", Span::dummy());
    let bounded = parameter
        .clone()
        .with_bound(ProtocolBound::simple(path("Ordered")));
    let mono = callable(Some(PropertySet::pure()), List::new());
    let generic = callable(Some(PropertySet::pure()), vec![parameter].into());
    let constrained = callable(Some(PropertySet::pure()), vec![bounded].into());
    assert!(!subtyping.is_subtype(&mono, &generic));
    assert!(!subtyping.is_subtype(&generic, &mono));
    assert!(!subtyping.is_subtype(&constrained, &generic));
    assert!(subtyping.is_subtype(&constrained, &constrained));
}

#[test]
fn callable_parameter_and_result_variance_remain_available() {
    let subtyping = Subtyping::new();
    let source = Type::function(vec![Type::int()].into(), greater_than(10));
    let target = Type::function(vec![greater_than(0)].into(), Type::int());
    assert!(subtyping.is_subtype(&source, &target));
    assert!(!subtyping.is_subtype(&target, &source));
}

#[test]
fn unresolved_callable_contexts_cannot_discharge_dependency_requirements() {
    let subtyping = Subtyping::new();
    let first = TypeVar::fresh();
    let second = TypeVar::fresh();
    let with_context = |context| {
        let mut function = callable(Some(PropertySet::pure()), List::new());
        if let Type::Function { contexts, .. } = &mut function {
            *contexts = context;
        }
        function
    };
    let unknown = with_context(Some(ContextExpr::Variable(first)));
    let other_unknown = with_context(Some(ContextExpr::Variable(second)));
    let no_context = with_context(None);
    let empty = with_context(Some(ContextExpr::empty()));
    let logger = with_context(Some(ContextExpr::Concrete(ContextRequirement::single(
        ContextRef::new("Logger".into(), std::any::TypeId::of::<()>()),
    ))));
    assert!(!subtyping.is_subtype(&unknown, &no_context));
    assert!(!subtyping.is_subtype(&unknown, &empty));
    assert!(!subtyping.is_subtype(&unknown, &other_unknown));
    assert!(!subtyping.is_subtype(&unknown, &logger));
    assert!(!subtyping.is_subtype(&logger, &unknown));
    assert!(subtyping.is_subtype(&no_context, &unknown));
    assert!(subtyping.is_subtype(&empty, &unknown));

    // Check equality of the opaque variable through a non-reflexive function
    // judgment, so whole-type equality cannot mask the context rule.
    let mut narrower_result = unknown.clone();
    if let Type::Function { return_type, .. } = &mut narrower_result {
        *return_type = Box::new(greater_than(0));
    }
    assert!(subtyping.is_subtype(&narrower_result, &unknown));
}

#[test]
fn checked_to_managed_mutable_references_preserve_the_referent_invariant() {
    let subtyping = Subtyping::new();
    for mutable in [false, true] {
        let checked = Type::CheckedReference {
            mutable,
            inner: Box::new(greater_than(0)),
        };
        let managed = Type::Reference {
            mutable,
            inner: Box::new(Type::int()),
        };
        assert_eq!(subtyping.is_subtype(&checked, &managed), !mutable);
        let same_referent = Type::Reference {
            mutable,
            inner: Box::new(greater_than(0)),
        };
        assert!(subtyping.is_subtype(&checked, &same_referent));
    }
}

#[test]
fn unsafe_references_cannot_acquire_static_or_runtime_guarantees() {
    let subtyping = Subtyping::new();
    for mutable in [false, true] {
        let unchecked = Type::UnsafeReference {
            mutable,
            inner: Box::new(Type::int()),
        };
        let checked = Type::CheckedReference {
            mutable,
            inner: Box::new(Type::int()),
        };
        let managed = Type::Reference {
            mutable,
            inner: Box::new(Type::int()),
        };
        assert!(!subtyping.is_subtype(&unchecked, &checked));
        assert!(!subtyping.is_subtype(&unchecked, &managed));
    }
}

#[test]
fn same_tier_mutable_references_are_invariant() {
    let subtyping = Subtyping::new();
    let reference_kinds: [fn(bool, Box<Type>) -> Type; 3] = [
        |mutable, inner| Type::Reference { mutable, inner },
        |mutable, inner| Type::CheckedReference { mutable, inner },
        |mutable, inner| Type::UnsafeReference { mutable, inner },
    ];
    for reference in reference_kinds {
        let narrow = reference(true, Box::new(greater_than(0)));
        let wide = reference(true, Box::new(Type::int()));
        assert!(!subtyping.is_subtype(&narrow, &wide));
        assert!(!subtyping.is_subtype(&wide, &narrow));
        assert!(subtyping.is_subtype(
            &reference(false, Box::new(greater_than(0))),
            &reference(false, Box::new(Type::int())),
        ));
    }
}

#[test]
fn unknown_and_mutable_constructors_are_invariant_in_both_representations() {
    let subtyping = Subtyping::new();
    for name in ["UserCell", "Cell", "Mutex"] {
        for source_named in [false, true] {
            for target_named in [false, true] {
                let narrow = constructor(name, vec![greater_than(0)].into(), source_named);
                let wide = constructor(name, vec![Type::int()].into(), target_named);
                assert!(!subtyping.is_subtype(&narrow, &wide), "{name}");
                assert!(!subtyping.is_subtype(&wide, &narrow), "{name}");
                let equivalent = constructor(name, vec![greater_than(0)].into(), target_named);
                assert!(subtyping.is_subtype(&narrow, &equivalent), "{name}");
            }
        }
    }
}

#[test]
fn known_covariance_keeps_its_direction_across_representations() {
    let subtyping = Subtyping::new();
    for source_named in [false, true] {
        for target_named in [false, true] {
            let narrow = constructor("List", vec![greater_than(0)].into(), source_named);
            let wide = constructor("List", vec![Type::int()].into(), target_named);
            assert!(subtyping.is_subtype(&narrow, &wide));
            assert!(!subtyping.is_subtype(&wide, &narrow));
        }
    }
}

#[test]
fn qualified_names_do_not_inherit_identity_or_variance_from_their_leaf() {
    let subtyping = Subtyping::new();
    let bare = constructor("List", vec![Type::int()].into(), false);
    let qualified = constructor("user.List", vec![Type::int()].into(), true);
    assert!(!subtyping.is_subtype(&bare, &qualified));
    assert!(!subtyping.is_subtype(&qualified, &bare));
    let narrow = constructor("user.List", vec![greater_than(0)].into(), true);
    assert!(!subtyping.is_subtype(&narrow, &qualified));
    assert!(subtyping.is_subtype(&qualified, &qualified));
}

#[test]
fn variance_metadata_does_not_cover_an_unrecognized_constructor_arity() {
    let subtyping = Subtyping::new();
    for named in [false, true] {
        let narrow = constructor("List", vec![greater_than(0), Type::int()].into(), named);
        let wide = constructor("List", vec![Type::int(), Type::int()].into(), named);
        assert!(!subtyping.is_subtype(&narrow, &wide));
    }
}
