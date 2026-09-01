//! An open ARGUMENT does not stop an impl from being selected; an open
//! HEAD does.
//!
//! `try_resolve_associated_type_projection_inner` guarded on
//! `has_unresolved_vars(base)` — does the base contain an open variable
//! ANYWHERE — and then returned `None` for everything that was not a
//! bare `Type::Var`.  `ListIter<τ>` satisfies that guard and names its
//! constructor outright, so `implement<T> Iterator for ListIter<T> {
//! type Item = T; }` selects against it and reduces `Item` to τ whatever
//! τ becomes.  Under the old guard it never got asked.
//!
//! What followed is the E404 class T0741 owns: the projection stayed
//! unreduced as `Item<ListIter<τ>>`, unification bound τ to it, and the
//! type grew on every use —
//! `SetIter<Item<SetIter<Item<SetIter<_>>>>>` — before being reported
//! as "not fully determined by this function", which blames the author
//! for a missing annotation.
//!
//! WHAT THIS FILE PINS, AND WHAT IT DOES NOT.  The end-to-end symptom
//! needs an impl loaded from the BAKED ARCHIVE — the same shape written
//! locally was always clean, because the projection was never queried —
//! so no unit test reaches it.  What is pinned here is the PREMISE the
//! repair rests on: the authority it now consults can in fact answer for
//! a base whose argument is still open.  If that stops being true the
//! repair goes inert, and inert is indistinguishable from absent by any
//! error count.
//!
//! The end-to-end leg is measured against the `core/` corpus instead:
//! 96 files carried the message before the repair, 891 errors; after it,
//! 868, with two files improving and none regressing (`set.vr` 29 → 7).
//! That 3% is the honest size of THIS route — the rest of the class
//! arrives by others, and they are named in T0741 rather than implied.

use verum_common::{List, Map, Maybe, Text};
use verum_types::protocol::{ProtocolChecker, ProtocolImpl};
use verum_types::ty::{Type, TypeVar};

/// `implement<T> Iterator for ListIter<T> { type Item = T; }`, with `T`
/// the same variable in both positions — which is what makes the impl
/// answer for any instantiation.
fn checker_with_listiter_impl(param: TypeVar) -> (ProtocolChecker, Type) {
    let for_type = Type::Named {
        path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            "ListIter",
            verum_ast::Span::dummy(),
        )),
        args: [Type::Var(param)].into_iter().collect(),
    };
    let mut assoc: Map<Text, Type> = Map::new();
    assoc.insert(Text::from("Item"), Type::Var(param));
    let impl_ = ProtocolImpl {
        protocol: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            "Iterator",
            verum_ast::Span::dummy(),
        )),
        protocol_args: List::new(),
        for_type: for_type.clone(),
        where_clauses: List::new(),
        methods: Map::new(),
        associated_types: assoc,
        associated_consts: Map::new(),
        specialization: Maybe::None,
        impl_crate: Maybe::None,
        span: verum_ast::Span::dummy(),
        type_param_fn_bounds: Map::new(),
    };
    let mut checker = ProtocolChecker::new();
    checker
        .register_impl(impl_)
        .expect("a single impl cannot conflict with itself");
    (checker, for_type)
}

#[test]
fn a_base_whose_argument_is_open_still_selects_its_impl() {
    let param = TypeVar::fresh();
    let (checker, for_type) = checker_with_listiter_impl(param);

    let answer = checker.try_find_associated_type(&for_type, &Text::from("Item"));
    assert!(
        answer.is_some(),
        "`ListIter<τ>` names its constructor, so the impl selects against \
         it — the repair in try_resolve_associated_type_projection_inner \
         relies on this answer existing"
    );
}

/// The control that keeps the test above from passing vacuously: a
/// checker with NO impl must not answer.
#[test]
fn with_no_impl_registered_there_is_no_answer() {
    let param = TypeVar::fresh();
    let for_type = Type::Named {
        path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            "ListIter",
            verum_ast::Span::dummy(),
        )),
        args: [Type::Var(param)].into_iter().collect(),
    };
    let checker = ProtocolChecker::new();
    assert!(
        checker
            .try_find_associated_type(&for_type, &Text::from("Item"))
            .is_none(),
        "an answer with nothing registered would mean the first test \
         measures the fixture, not the resolver"
    );
}

/// And the distinction the guard now makes, stated as data: the base
/// CONTAINS an open variable in both cases, and only one of them has an
/// open HEAD.
#[test]
fn containing_an_open_variable_is_not_the_same_as_having_an_open_head() {
    let param = TypeVar::fresh();
    let (checker, applied) = checker_with_listiter_impl(param);

    // Open head: a bare variable names nothing, so no impl can be
    // selected and the projection must stay deferred.
    let bare = Type::Var(TypeVar::fresh());
    assert!(
        checker
            .try_find_associated_type(&bare, &Text::from("Item"))
            .is_none(),
        "a bare variable has no constructor to match an impl against"
    );

    // Open argument, known head: answerable, and that is the whole point.
    assert!(
        checker
            .try_find_associated_type(&applied, &Text::from("Item"))
            .is_some(),
        "a known constructor with an open argument is answerable"
    );
}
