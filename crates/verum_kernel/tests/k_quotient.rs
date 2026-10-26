//! Quotient formation and recursion check their actual typing and coherence
//! premises. Fixtures quantify over ordinary context-carried types: no opaque
//! unregistered type heads or postulated relation-respect theorem is needed.
use verum_common::{Heap, Text};
use verum_kernel::{AxiomRegistry, Context, CoreTerm, UniverseLevel, infer, normalize};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}
fn uni(level: u32) -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(level))
}
fn pi(name: &str, domain: CoreTerm, codomain: CoreTerm) -> CoreTerm {
    CoreTerm::Pi {
        binder: Text::from(name),
        domain: Heap::new(domain),
        codomain: Heap::new(codomain),
    }
}
fn lam(name: &str, domain: CoreTerm, body: CoreTerm) -> CoreTerm {
    CoreTerm::Lam {
        binder: Text::from(name),
        domain: Heap::new(domain),
        body: Heap::new(body),
    }
}
fn app(f: CoreTerm, x: CoreTerm) -> CoreTerm {
    CoreTerm::App(Heap::new(f), Heap::new(x))
}
fn path(carrier: CoreTerm, lhs: CoreTerm, rhs: CoreTerm) -> CoreTerm {
    CoreTerm::PathTy {
        carrier: Heap::new(carrier),
        lhs: Heap::new(lhs),
        rhs: Heap::new(rhs),
    }
}
fn quotient() -> CoreTerm {
    CoreTerm::Quotient {
        base: Heap::new(var("A")),
        equiv: Heap::new(var("R")),
    }
}
fn context() -> Context {
    Context::new()
        .extend(Text::from("A"), uni(0))
        .extend(Text::from("B"), uni(0))
        .extend(Text::from("a"), var("A"))
        .extend(Text::from("b"), var("B"))
        .extend(
            Text::from("R"),
            pi("x", var("A"), pi("y", var("A"), uni(0))),
        )
        .extend(Text::from("q"), quotient())
}
fn intro() -> CoreTerm {
    CoreTerm::QuotIntro {
        value: Heap::new(var("a")),
        base: Heap::new(var("A")),
        equiv: Heap::new(var("R")),
    }
}
fn constant_elim(scrutinee: CoreTerm) -> CoreTerm {
    CoreTerm::QuotElim {
        scrutinee: Heap::new(scrutinee),
        motive: Heap::new(lam("q", quotient(), var("B"))),
        case: Heap::new(lam("x", var("A"), var("b"))),
        respect: Heap::new(lam(
            "x",
            var("A"),
            lam(
                "y",
                var("A"),
                lam(
                    "r",
                    app(app(var("R"), var("x")), var("y")),
                    CoreTerm::Refl(Heap::new(var("b"))),
                ),
            ),
        )),
    }
}
fn check(ctx: &Context, term: &CoreTerm) -> Result<CoreTerm, verum_kernel::KernelError> {
    infer(ctx, term, &AxiomRegistry::new())
}

#[test]
fn quotient_formation_checks_both_relation_arguments_and_result() {
    assert_eq!(check(&context(), &quotient()).unwrap(), uni(0));
    for relation in [
        var("a"),
        lam("x", var("B"), lam("y", var("A"), var("B"))),
        lam("x", var("A"), lam("y", var("B"), var("B"))),
        lam("x", var("A"), lam("y", var("A"), var("b"))),
    ] {
        let term = CoreTerm::Quotient {
            base: Heap::new(var("A")),
            equiv: Heap::new(relation),
        };
        assert!(
            check(&context(), &term).is_err(),
            "malformed relation accepted: {term:?}"
        );
    }
}

#[test]
fn quotient_universe_includes_the_relation_universe() {
    let ctx = context().extend(
        Text::from("R"),
        pi("x", var("A"), pi("y", var("A"), uni(2))),
    );
    assert_eq!(check(&ctx, &quotient()).unwrap(), uni(2));
}

#[test]
fn quotient_rejects_a_value_as_carrier() {
    let term = CoreTerm::Quotient {
        base: Heap::new(var("a")),
        equiv: Heap::new(var("R")),
    };
    assert!(check(&context(), &term).is_err());
}

#[test]
fn quotient_introduction_checks_value_and_relation() {
    assert_eq!(check(&context(), &intro()).unwrap(), quotient());
    let mut wrong = intro();
    if let CoreTerm::QuotIntro { value, .. } = &mut wrong {
        *value = Heap::new(var("b"));
    }
    assert!(check(&context(), &wrong).is_err());
    let mut wrong = intro();
    if let CoreTerm::QuotIntro { equiv, .. } = &mut wrong {
        *equiv = Heap::new(var("a"));
    }
    assert!(check(&context(), &wrong).is_err());
}

#[test]
fn constant_map_has_a_checked_relation_respect_proof() {
    let term = constant_elim(var("q"));
    assert_eq!(normalize(&check(&context(), &term).unwrap()), var("B"));
}

#[test]
fn identity_map_on_equality_quotient_uses_the_relation_witness() {
    let relation = lam(
        "x",
        var("A"),
        lam("y", var("A"), path(var("A"), var("x"), var("y"))),
    );
    let q = CoreTerm::Quotient {
        base: Heap::new(var("A")),
        equiv: Heap::new(relation.clone()),
    };
    let ctx = context().extend(Text::from("eq_q"), q.clone());
    let term = CoreTerm::QuotElim {
        scrutinee: Heap::new(var("eq_q")),
        motive: Heap::new(lam("q", q, var("A"))),
        case: Heap::new(lam("x", var("A"), var("x"))),
        respect: Heap::new(lam(
            "x",
            var("A"),
            lam(
                "y",
                var("A"),
                lam("r", app(app(relation, var("x")), var("y")), var("r")),
            ),
        )),
    };
    assert_eq!(normalize(&check(&ctx, &term).unwrap()), var("A"));
}

#[test]
fn unrelated_case_cannot_prove_an_arbitrary_goal() {
    let mut term = constant_elim(var("q"));
    if let CoreTerm::QuotElim { case, .. } = &mut term {
        *case = Heap::new(lam("x", var("A"), var("a")));
    }
    assert!(check(&context(), &term).is_err());
}

#[test]
fn representative_identity_does_not_respect_an_arbitrary_relation() {
    let mut term = constant_elim(var("q"));
    if let CoreTerm::QuotElim {
        motive,
        case,
        respect,
        ..
    } = &mut term
    {
        *motive = Heap::new(lam("q", quotient(), var("A")));
        *case = Heap::new(lam("x", var("A"), var("x")));
        *respect = Heap::new(lam(
            "x",
            var("A"),
            lam(
                "y",
                var("A"),
                lam(
                    "r",
                    app(app(var("R"), var("x")), var("y")),
                    CoreTerm::Refl(Heap::new(var("x"))),
                ),
            ),
        ));
    }
    assert!(check(&context(), &term).is_err());
}

#[test]
fn unrelated_well_typed_evidence_is_not_a_respect_proof() {
    for evidence in [var("a"), CoreTerm::Refl(Heap::new(var("b"))), uni(0)] {
        let mut term = constant_elim(var("q"));
        if let CoreTerm::QuotElim { respect, .. } = &mut term {
            *respect = Heap::new(evidence);
        }
        assert!(check(&context(), &term).is_err());
    }
}

#[test]
fn malformed_motive_and_nonquotient_scrutinee_are_rejected() {
    assert!(check(&context(), &constant_elim(var("a"))).is_err());
    for bad in [
        var("b"),
        lam("q", var("A"), var("B")),
        lam("q", quotient(), var("b")),
    ] {
        let mut term = constant_elim(var("q"));
        if let CoreTerm::QuotElim { motive, .. } = &mut term {
            *motive = Heap::new(bad);
        }
        assert!(check(&context(), &term).is_err());
    }
}

#[test]
fn dependent_motive_is_not_treated_as_a_constant_at_one_representative() {
    let ctx = context().extend(Text::from("P"), pi("q", quotient(), uni(0)));
    let mut term = constant_elim(intro());
    if let CoreTerm::QuotElim { motive, .. } = &mut term {
        *motive = Heap::new(var("P"));
    }
    assert!(check(&ctx, &term).is_err());
}

#[test]
fn checked_recursor_beta_reduces_and_preserves_type() {
    let term = constant_elim(intro());
    let original_type = normalize(&check(&context(), &term).unwrap());
    let reduced = normalize(&term);
    assert_eq!(reduced, var("b"));
    assert_eq!(check(&context(), &reduced).unwrap(), original_type);
}

#[test]
fn generated_binders_do_not_capture_ambient_names() {
    let ctx = context().extend(Text::from("_quot_x_0"), var("B"));
    let term = verum_kernel::substitute(&constant_elim(var("q")), "b", &var("_quot_x_0"));
    assert_eq!(normalize(&check(&ctx, &term).unwrap()), var("B"));
}

#[test]
fn respect_evidence_is_part_of_serialized_and_structural_term_identity() {
    let term = constant_elim(var("q"));
    let mut encoded = serde_json::to_value(&term).unwrap();
    encoded["QuotElim"]
        .as_object_mut()
        .unwrap()
        .remove("respect");
    assert!(serde_json::from_value::<CoreTerm>(encoded).is_err());
    let mut changed = term.clone();
    if let CoreTerm::QuotElim { respect, .. } = &mut changed {
        *respect = Heap::new(var("evidence"));
    }
    assert!(verum_kernel::free_vars(&changed).contains(&Text::from("evidence")));
    assert!(!verum_kernel::definitional_eq(&term, &changed));
    let substituted = verum_kernel::substitute(&changed, "evidence", &var("replacement"));
    assert!(verum_kernel::free_vars(&substituted).contains(&Text::from("replacement")));
    assert!(!verum_kernel::free_vars(&substituted).contains(&Text::from("evidence")));
}
