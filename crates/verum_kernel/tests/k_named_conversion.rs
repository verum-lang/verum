//! Conversion identifies binders by scope and preserves free-variable identity.
use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, CoreTerm, UniverseLevel, definitional_eq, definitional_eq_with_axioms,
    free_vars, substitute_all,
};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}
fn ty() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}
fn lam(name: &str, body: CoreTerm) -> CoreTerm {
    CoreTerm::Lam {
        binder: Text::from(name),
        domain: Heap::new(ty()),
        body: Heap::new(body),
    }
}
fn bind(kind: usize, name: &str, domain: CoreTerm, body: CoreTerm) -> CoreTerm {
    let binder = Text::from(name);
    match kind {
        0 => CoreTerm::Pi {
            binder,
            domain: Heap::new(domain),
            codomain: Heap::new(body),
        },
        1 => CoreTerm::Lam {
            binder,
            domain: Heap::new(domain),
            body: Heap::new(body),
        },
        2 => CoreTerm::Sigma {
            binder,
            fst_ty: Heap::new(domain),
            snd_ty: Heap::new(body),
        },
        3 => CoreTerm::Refine {
            binder,
            base: Heap::new(domain),
            predicate: Heap::new(body),
        },
        _ => unreachable!(),
    }
}

#[test]
fn every_named_binder_supports_alpha_conversion() {
    for kind in 0..4 {
        let a = bind(kind, "x", ty(), var("x"));
        let b = bind(kind, "renamed", ty(), var("renamed"));
        assert!(definitional_eq(&a, &b));
        assert!(definitional_eq_with_axioms(&a, &b, &AxiomRegistry::new()));
        assert!(!definitional_eq(&a, &bind(kind, "renamed", ty(), var("x"))));
    }
}

#[test]
fn lexical_shadowing_is_distinct_from_an_outer_reference() {
    let inner = lam("x", lam("x", var("x")));
    assert!(definitional_eq(
        &inner,
        &lam("outer", lam("inner", var("inner")))
    ));
    assert!(!definitional_eq(
        &inner,
        &lam("outer", lam("inner", var("outer")))
    ));
}

#[test]
fn equal_spellings_do_not_confuse_a_free_variable_with_a_bound_one() {
    assert!(!definitional_eq(&lam("x", var("y")), &lam("y", var("y"))));
    assert!(!definitional_eq(&lam("x", var("x")), &lam("y", var("x"))));
    assert!(definitional_eq(
        &lam("x", var("free")),
        &lam("y", var("free"))
    ));
}

#[test]
fn a_binder_does_not_scope_over_its_own_domain() {
    for kind in 0..4 {
        assert!(definitional_eq(
            &bind(kind, "x", var("free"), var("x")),
            &bind(kind, "y", var("free"), var("y"))
        ));
        assert!(!definitional_eq(
            &bind(kind, "x", var("x"), var("x")),
            &bind(kind, "y", var("y"), var("y"))
        ));
    }
}

#[test]
fn simultaneous_substitution_swaps_without_rewriting_inserted_arguments() {
    let term = CoreTerm::Inductive {
        path: Text::from("Pair"),
        args: List::from_iter([var("A"), var("B")]),
    };
    let replacements = [(Text::from("A"), var("B")), (Text::from("B"), var("A"))];
    let expected = CoreTerm::Inductive {
        path: Text::from("Pair"),
        args: List::from_iter([var("B"), var("A")]),
    };
    assert_eq!(substitute_all(&term, &replacements), expected);
    assert_eq!(
        substitute_all(&term, &[replacements[1].clone(), replacements[0].clone()]),
        expected
    );
}

#[test]
fn simultaneous_substitution_avoids_capture_and_generated_name_collisions() {
    let term = lam(
        "B",
        CoreTerm::Pair(Heap::new(var("A")), Heap::new(var("A_subst0"))),
    );
    let actual = substitute_all(
        &term,
        &[(Text::from("A"), var("B")), (Text::from("B"), var("A"))],
    );
    let expected = lam(
        "local",
        CoreTerm::Pair(Heap::new(var("B")), Heap::new(var("A_subst0"))),
    );
    assert!(definitional_eq(&actual, &expected));
    assert!(free_vars(&actual).contains(&Text::from("B")));
    assert!(!free_vars(&actual).contains(&Text::from("A")));
}
