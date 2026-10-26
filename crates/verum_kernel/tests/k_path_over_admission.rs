use verum_common::{Heap, Text};
use verum_kernel::{AxiomRegistry, Context, CoreTerm, UniverseLevel, infer, normalize};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}
fn universe() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}
fn apply(f: CoreTerm, x: CoreTerm) -> CoreTerm {
    CoreTerm::App(Heap::new(f), Heap::new(x))
}
fn path(left: CoreTerm, right: CoreTerm) -> CoreTerm {
    CoreTerm::PathTy {
        carrier: Heap::new(var("B")),
        lhs: Heap::new(left),
        rhs: Heap::new(right),
    }
}
fn context() -> Context {
    Context::new()
        .extend(Text::from("B"), universe())
        .extend(Text::from("C"), universe())
        .extend(Text::from("a"), var("B"))
        .extend(Text::from("b"), var("B"))
        .extend(
            Text::from("M"),
            CoreTerm::Pi {
                binder: Text::from("x"),
                domain: Heap::new(var("B")),
                codomain: Heap::new(universe()),
            },
        )
        .extend(
            Text::from("N"),
            CoreTerm::Pi {
                binder: Text::from("x"),
                domain: Heap::new(var("C")),
                codomain: Heap::new(universe()),
            },
        )
        .extend(Text::from("p"), path(var("a"), var("b")))
        .extend(Text::from("loop"), path(var("a"), var("a")))
        .extend(Text::from("left"), apply(var("M"), var("a")))
        .extend(Text::from("right"), apply(var("M"), var("b")))
}
fn over(motive: CoreTerm, evidence: CoreTerm, left: CoreTerm, right: CoreTerm) -> CoreTerm {
    CoreTerm::PathOver {
        motive: Heap::new(motive),
        path: Heap::new(evidence),
        lhs: Heap::new(left),
        rhs: Heap::new(right),
    }
}

#[test]
fn dependent_endpoints_check_in_their_actual_fibers() {
    let result = infer(
        &context(),
        &over(var("M"), var("p"), var("left"), var("right")),
        &AxiomRegistry::new(),
    )
    .unwrap();
    assert_eq!(result, universe());
}

#[test]
fn annotations_and_nonpath_values_are_not_path_evidence() {
    for evidence in [path(var("a"), var("b")), var("a")] {
        assert!(
            infer(
                &context(),
                &over(var("M"), evidence, var("left"), var("right")),
                &AxiomRegistry::new()
            )
            .is_err()
        );
    }
}

#[test]
fn motive_domain_must_match_the_path_carrier() {
    assert!(
        infer(
            &context(),
            &over(var("N"), var("p"), var("left"), var("right")),
            &AxiomRegistry::new()
        )
        .is_err()
    );
}

#[test]
fn each_endpoint_must_inhabit_its_fiber() {
    for (left, right) in [
        (var("a"), var("right")),
        (var("left"), var("b")),
        (var("right"), var("left")),
    ] {
        assert!(
            infer(
                &context(),
                &over(var("M"), var("p"), left, right),
                &AxiomRegistry::new()
            )
            .is_err()
        );
    }
}

#[test]
fn reflexive_reduction_preserves_the_inferred_universe() {
    let term = over(
        var("M"),
        CoreTerm::Refl(Heap::new(var("a"))),
        var("left"),
        var("left"),
    );
    let expected = CoreTerm::PathTy {
        carrier: Heap::new(apply(var("M"), var("a"))),
        lhs: Heap::new(var("left")),
        rhs: Heap::new(var("left")),
    };
    let normal = normalize(&term);
    assert_eq!(normal, expected);
    assert_eq!(
        infer(&context(), &normal, &AxiomRegistry::new()).unwrap(),
        infer(&context(), &term, &AxiomRegistry::new()).unwrap()
    );
}

#[test]
fn a_loop_annotation_cannot_authorize_homogeneous_conversion() {
    let term = over(var("M"), path(var("a"), var("a")), var("left"), var("left"));
    assert_eq!(normalize(&term), term);
}

#[test]
fn a_neutral_loop_is_not_definitionally_reflexivity() {
    let term = over(var("M"), var("loop"), var("left"), var("left"));
    assert_eq!(
        infer(&context(), &term, &AxiomRegistry::new()).unwrap(),
        universe()
    );
    assert_eq!(normalize(&term), term);
}

#[test]
fn beta_reduced_motives_retain_valid_dependent_endpoints() {
    let family_type = CoreTerm::Pi {
        binder: Text::from("x"),
        domain: Heap::new(var("B")),
        codomain: Heap::new(universe()),
    };
    let identity = CoreTerm::Lam {
        binder: Text::from("family"),
        domain: Heap::new(family_type),
        body: Heap::new(var("family")),
    };
    let term = over(
        apply(identity, var("M")),
        var("p"),
        var("left"),
        var("right"),
    );
    assert_eq!(
        infer(&context(), &term, &AxiomRegistry::new()).unwrap(),
        universe()
    );
}
