use verum_common::Heap;
use verum_kernel::{AxiomRegistry, Context, CoreTerm, UniverseLevel, verify_full};
fn u() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}
fn v(name: &str) -> CoreTerm {
    CoreTerm::Var(name.into())
}
fn pair(a: CoreTerm, b: CoreTerm) -> CoreTerm {
    CoreTerm::Pair(Heap::new(a), Heap::new(b))
}
fn path(a: CoreTerm, x: CoreTerm, y: CoreTerm) -> CoreTerm {
    CoreTerm::PathTy {
        carrier: Heap::new(a),
        lhs: Heap::new(x),
        rhs: Heap::new(y),
    }
}
fn sigma(name: &str, a: CoreTerm, b: CoreTerm) -> CoreTerm {
    CoreTerm::Sigma {
        binder: name.into(),
        fst_ty: Heap::new(a),
        snd_ty: Heap::new(b),
    }
}
fn pi(name: &str, a: CoreTerm, b: CoreTerm) -> CoreTerm {
    CoreTerm::Pi {
        binder: name.into(),
        domain: Heap::new(a),
        codomain: Heap::new(b),
    }
}
fn lam(name: &str, a: CoreTerm, b: CoreTerm) -> CoreTerm {
    CoreTerm::Lam {
        binder: name.into(),
        domain: Heap::new(a),
        body: Heap::new(b),
    }
}
fn refl(x: CoreTerm) -> CoreTerm {
    CoreTerm::Refl(Heap::new(x))
}
fn app(f: CoreTerm, x: CoreTerm) -> CoreTerm {
    CoreTerm::App(Heap::new(f), Heap::new(x))
}
fn context() -> Context {
    Context::new()
        .extend("A".into(), u())
        .extend("x".into(), v("A"))
        .extend("y".into(), v("A"))
}
fn target() -> CoreTerm {
    sigma("witness", v("A"), path(v("A"), v("x"), v("witness")))
}

#[test]
fn named_pairs_use_the_expected_dependent_family() {
    let ax = AxiomRegistry::new();
    let good = pair(v("x"), refl(v("x")));
    assert!(verify_full(&context(), &good, &target(), &ax).is_ok());
    assert!(verify_full(&context(), &pair(v("y"), refl(v("x"))), &target(), &ax).is_err());
    assert!(verify_full(&context(), &pair(v("x"), refl(v("y"))), &target(), &ax).is_err());
}

#[test]
fn checking_alpha_renames_both_binders_without_capturing_context_names() {
    let ax = AxiomRegistry::new();
    let expected = pi(
        "argument",
        v("A"),
        sigma("w", v("A"), path(v("A"), v("argument"), v("w"))),
    );
    assert!(
        verify_full(
            &context(),
            &lam(
                "different",
                v("A"),
                pair(v("different"), refl(v("different")))
            ),
            &expected,
            &ax
        )
        .is_ok()
    );
    // x is free in the goal but shadowed in the implementation. Renaming only
    // the goal binder to x would incorrectly identify two arbitrary values.
    let bad = lam("x", v("A"), pair(v("x"), refl(v("x"))));
    assert!(verify_full(&context(), &bad, &pi("argument", v("A"), target()), &ax).is_err());
}

#[test]
fn typed_argument_and_identity_boundaries_accept_dependent_witnesses() {
    let ax = AxiomRegistry::new();
    let witness = pair(v("x"), refl(v("x")));
    let consume = lam("p", target(), CoreTerm::Fst(Heap::new(v("p"))));
    assert!(verify_full(&context(), &app(consume, witness.clone()), &v("A"), &ax).is_ok());
    assert!(
        verify_full(
            &context(),
            &refl(witness.clone()),
            &path(target(), witness.clone(), witness),
            &ax
        )
        .is_ok()
    );
}

#[test]
fn definitions_constructors_and_induction_preserve_expected_families() {
    use verum_common::List;
    use verum_kernel::{
        ConstructorSig, FrameworkId, InductiveRegistry, RegisteredInductive, verify_with_inductives,
    };
    let mut axioms = AxiomRegistry::new();
    let closed = lam("A", u(), lam("x", v("A"), pair(v("x"), refl(v("x")))));
    let closed_type = pi("A", u(), pi("x", v("A"), target()));
    axioms
        .register_definition(
            "witness".into(),
            closed,
            closed_type,
            FrameworkId {
                framework: "test".into(),
                citation: "dependent introduction".into(),
            },
        )
        .unwrap();
    let mut registry = InductiveRegistry::new();
    let field = sigma("w", v("T"), path(v("T"), v("w"), v("w")));
    registry
        .register(RegisteredInductive::new(
            "Box".into(),
            List::from_iter(["T".into()]),
            List::from_iter([ConstructorSig {
                name: "Make".into(),
                arg_types: List::from_iter([field]),
            }]),
        ))
        .unwrap();
    let boxed = CoreTerm::Constructor {
        inductive: "Box".into(),
        name: "Make".into(),
        params: List::from_iter([v("A")]),
        args: List::from_iter([pair(v("x"), refl(v("x")))]),
    };
    let box_type = CoreTerm::Inductive {
        path: "Box".into(),
        args: List::from_iter([v("A")]),
    };
    verify_with_inductives(&context(), &boxed, &box_type, &axioms, &registry).unwrap();
    let field_type = sigma("w", v("A"), path(v("A"), v("w"), v("w")));
    let elimination = CoreTerm::Elim {
        motive: Heap::new(lam("box", box_type, target())),
        scrutinee: Heap::new(boxed),
        cases: List::from_iter([lam("field", field_type, pair(v("x"), refl(v("x"))))]),
    };
    verify_with_inductives(&context(), &elimination, &target(), &axioms, &registry).unwrap();
}

#[test]
fn synthesized_product_binders_do_not_capture_free_component_types() {
    let ctx = context().extend("_".into(), v("A"));
    let value = pair(v("x"), refl(v("_")));
    let projected = CoreTerm::Snd(Heap::new(value));
    verify_full(
        &ctx,
        &projected,
        &path(v("A"), v("_"), v("_")),
        &AxiomRegistry::new(),
    )
    .unwrap();
    assert!(
        verify_full(
            &ctx,
            &projected,
            &path(v("A"), v("x"), v("x")),
            &AxiomRegistry::new()
        )
        .is_err()
    );
}

#[test]
fn projections_expose_reducible_dependent_pair_types() {
    let alias = app(lam("T", u(), v("T")), target());
    let ctx = context().extend("p".into(), alias);
    let first = CoreTerm::Fst(Heap::new(v("p")));
    let second = CoreTerm::Snd(Heap::new(v("p")));
    verify_full(&ctx, &first, &v("A"), &AxiomRegistry::new()).unwrap();
    verify_full(
        &ctx,
        &second,
        &path(v("A"), v("x"), first),
        &AxiomRegistry::new(),
    )
    .unwrap();
}
