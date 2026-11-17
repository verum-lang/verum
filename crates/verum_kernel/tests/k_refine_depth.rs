//! K-Refine depth-check integration tests (/ §4.4).
//!
//! Refinement formation checks both predicate sort and the independent
//! M-iteration depth restriction. These tests establish those admission
//! boundaries, not a general consistency or paradox-immunity theorem.
//!
//! This file exercises the kernel rule end-to-end — both the valid
//! refinement paths that must keep type-checking cleanly AND the
//! paradox-shape refinements that must be rejected with the precise
//! `KernelError::DepthViolation { binder, base_depth, pred_depth }`
//! diagnostic.

mod fixtures;
use fixtures::*;

use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, FrameworkId, KernelError, UniverseLevel, m_depth,
};

fn tvar(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

fn ind(path: &str, args: Vec<CoreTerm>) -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from(path),
        args: List::from_iter(args),
    }
}

fn refine(base: CoreTerm, binder: &str, predicate: CoreTerm) -> CoreTerm {
    CoreTerm::Refine {
        base: Heap::new(base),
        binder: Text::from(binder),
        predicate: Heap::new(predicate),
    }
}

fn axiom(name: &str, ty: CoreTerm) -> CoreTerm {
    CoreTerm::Axiom {
        name: Text::from(name),
        ty: Heap::new(ty),
        framework: FrameworkId {
            framework: Text::from("vva_test"),
            citation: Text::from("cite"),
        },
    }
}

// -----------------------------------------------------------------------------
// m_depth — component behaviour
// -----------------------------------------------------------------------------

#[test]
fn m_depth_of_variable_is_zero() {
    assert_eq!(m_depth(&tvar("x")), 0);
}

#[test]
fn m_depth_of_concrete_universe_is_its_level() {
    assert_eq!(m_depth(&CoreTerm::Universe(UniverseLevel::Concrete(0))), 0);
    assert_eq!(m_depth(&CoreTerm::Universe(UniverseLevel::Concrete(3))), 3);
}

#[test]
fn m_depth_of_inductive_is_one_plus_args() {
    // Int: depth 1 (a named schema speaks about its zero-depth args).
    let int_ty = ind("Int", vec![]);
    assert_eq!(m_depth(&int_ty), 1);

    // List<Int>: depth 2 = 1 + dp(Int) = 1 + 1.
    let list_int = ind("List", vec![int_ty.clone()]);
    assert_eq!(m_depth(&list_int), 2);

    // List<List<Int>>: depth 3.
    let list_list_int = ind("List", vec![list_int]);
    assert_eq!(m_depth(&list_list_int), 3);
}

#[test]
fn m_depth_of_axiom_is_same_as_its_type() {
    // Axiom nodes are *terms* (proof witnesses), not meta-statements.
    // Per + kernel m_depth, dp(Axiom { ty }) = dp(ty) so
    // a witness sits at the same M-stratum as its asserted type.
    // The "+1 for framework axioms" is reserved for the declaration-
    // time path (AxiomRegistry::register), not the invocation site.
    let stmt = ind("Bool", vec![]);
    let ax = axiom("yoneda", stmt);
    assert_eq!(m_depth(&ax), 1); // dp(Bool) = 1.
}

#[test]
fn m_depth_of_refine_is_max_of_components() {
    // {n : Int | n == n}: predicate depth 0 (variable comparison),
    // base depth 1 (Inductive("Int")) — max = 1.
    let t = refine(
        ind("Int", vec![]),
        "n",
        CoreTerm::App(Heap::new(tvar("n")), Heap::new(tvar("n"))),
    );
    assert_eq!(m_depth(&t), 1);
}

// -----------------------------------------------------------------------------
// K-Refine — valid refinements (MUST type-check)
// -----------------------------------------------------------------------------

#[test]
fn k_refine_accepts_a_proposition_without_claiming_it_is_inhabited() {
    let ctx = Context::new().extend(Text::from("P"), prop());
    let t = refine(ind("Int", vec![]), "n", tvar("P"));
    assert_eq!(
        infer_fixture(&ctx, &t, &AxiomRegistry::new()).unwrap(),
        CoreTerm::Universe(UniverseLevel::Concrete(0))
    );
}

fn prop() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Prop)
}

fn pi(binder: &str, domain: CoreTerm, codomain: CoreTerm) -> CoreTerm {
    CoreTerm::Pi {
        binder: Text::from(binder),
        domain: Heap::new(domain),
        codomain: Heap::new(codomain),
    }
}

#[test]
fn k_refine_checks_dependent_predicates_in_the_shadowing_binder_context() {
    let int = ind("Int", vec![]);
    let ctx = Context::new()
        .extend(Text::from("P"), pi("item", int.clone(), prop()))
        .extend(Text::from("n"), ind("Bool", vec![]));
    let predicate = CoreTerm::App(Heap::new(tvar("P")), Heap::new(tvar("n")));
    assert_eq!(
        infer_fixture(&ctx, &refine(int, "n", predicate), &AxiomRegistry::new()).unwrap(),
        CoreTerm::Universe(UniverseLevel::Concrete(0))
    );
}

#[test]
fn k_refine_rejects_data_values_types_proofs_and_unapplied_predicates() {
    let int = ind("Int", vec![]);
    let ctx = Context::new()
        .extend(Text::from("P"), prop())
        .extend(Text::from("proof"), tvar("P"))
        .extend(Text::from("flag"), ind("Bool", vec![]))
        .extend(Text::from("family"), pi("n", int.clone(), prop()));
    for predicate in [
        tvar("n"),
        int.clone(),
        prop(),
        CoreTerm::Universe(UniverseLevel::Concrete(0)),
        tvar("proof"),
        tvar("flag"),
        tvar("family"),
    ] {
        let result = infer_fixture(
            &ctx,
            &refine(int.clone(), "n", predicate.clone()),
            &AxiomRegistry::new(),
        );
        assert!(
            matches!(result, Err(KernelError::InvalidRefinementPredicate { ref binder, .. }) if binder.as_str() == "n"),
            "non-proposition {predicate:?}: {result:?}"
        );
    }
    let list = ind("List", vec![int]);
    assert!(matches!(
        infer_fixture(&ctx, &refine(list, "l", tvar("l")), &AxiomRegistry::new()),
        Err(KernelError::InvalidRefinementPredicate { .. })
    ));
}

#[test]
fn k_refine_preserves_dependent_argument_and_unbound_variable_errors() {
    let ctx = Context::new().extend(Text::from("P"), pi("b", ind("Bool", vec![]), prop()));
    let invalid = CoreTerm::App(Heap::new(tvar("P")), Heap::new(tvar("n")));
    assert!(matches!(
        infer_fixture(
            &ctx,
            &refine(ind("Int", vec![]), "n", invalid),
            &AxiomRegistry::new()
        ),
        Err(KernelError::TypeMismatch { .. })
    ));
    assert!(matches!(
        infer_fixture(
            &ctx,
            &refine(ind("Int", vec![]), "n", tvar("missing")),
            &AxiomRegistry::new()
        ),
        Err(KernelError::UnboundVariable(_))
    ));
}

#[test]
fn k_refine_normalizes_predicate_sort_without_recognizing_type_names() {
    let kind = CoreTerm::App(
        Heap::new(CoreTerm::Lam {
            binder: Text::from("A"),
            domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            body: Heap::new(prop()),
        }),
        Heap::new(ind("Unit", vec![])),
    );
    let ctx = Context::new().extend(Text::from("P"), kind);
    assert!(
        infer_fixture(
            &ctx,
            &refine(ind("Int", vec![]), "n", tvar("P")),
            &AxiomRegistry::new()
        )
        .is_ok()
    );
}

#[test]
fn k_refine_preserves_the_carriers_universe() {
    let ctx = Context::new().extend(Text::from("P"), prop());
    for level in [0, 2, 17] {
        let base = CoreTerm::Universe(UniverseLevel::Concrete(level));
        assert_eq!(
            infer_fixture(&ctx, &refine(base, "T", tvar("P")), &AxiomRegistry::new()).unwrap(),
            CoreTerm::Universe(UniverseLevel::Concrete(level + 1))
        );
    }
}

// -----------------------------------------------------------------------------
// K-Refine — depth violations (MUST be rejected)
// -----------------------------------------------------------------------------

#[test]
fn k_refine_rejects_schema_referencing_its_own_instantiation() {
    // Yanofsky-shape: {n : Int | P<List<Int>>}. `Int` (base) has
    // depth 1; `List<Int>` has depth 2; `P<List<Int>>` is a named
    // schema applied to a depth-2 argument, so pred depth = 1 + 2 = 3.
    // Base depth + 1 = 2. 3 >= 2 — violation. This is the exact
    // cartesian-closed-context diagonal `α: Y → T^Y` that T-2f*
    // blocks at comprehension time.
    let base = ind("Int", vec![]);
    let one_up = ind("List", vec![base.clone()]); // depth 2
    let pred = ind("P", vec![one_up]); // depth 3
    let t = refine(base, "n", pred);

    let result = infer_fixture(&Context::new(), &t, &AxiomRegistry::new());
    match result {
        Err(KernelError::DepthViolation {
            binder,
            base_depth,
            pred_depth,
        }) => {
            assert_eq!(binder.as_str(), "n");
            assert_eq!(base_depth, 1);
            assert_eq!(pred_depth, 3);
        }
        other => panic!("expected K-Refine DepthViolation, got {other:?}"),
    }
}

#[test]
fn k_refine_rejects_predicate_two_strata_deeper_than_base() {
    // Base Int (depth 1), predicate = List<List<List<Int>>> (depth 4).
    // 4 >= 1+1 — violation.
    let base = ind("Int", vec![]);
    let pred = ind(
        "List",
        vec![ind("List", vec![ind("List", vec![base.clone()])])],
    );
    let t = refine(base, "n", pred);

    match infer_fixture(&Context::new(), &t, &AxiomRegistry::new()) {
        Err(KernelError::DepthViolation {
            base_depth,
            pred_depth,
            ..
        }) => {
            assert_eq!(base_depth, 1);
            assert_eq!(pred_depth, 4);
        }
        other => panic!("expected K-Refine DepthViolation, got {other:?}"),
    }
}

#[test]
fn depth_violation_displays_diakrisis_and_yanofsky_context() {
    let err = KernelError::DepthViolation {
        binder: Text::from("x"),
        base_depth: 1,
        pred_depth: 3,
    };
    let msg = format!("{err}");
    // The diagnostic MUST point the user at the Diakrisis / Yanofsky
    // provenance so the rule's foundational source is discoverable
    // from the error alone.
    assert!(msg.contains("K-Refine"), "message lacks rule name: {msg}");
    assert!(msg.contains("T-2f*"), "message lacks axiom name: {msg}");
    assert!(
        msg.contains("Yanofsky"),
        "message lacks paradox-lineage source: {msg}"
    );
    assert!(msg.contains("1"), "message lacks base depth: {msg}");
    assert!(msg.contains("3"), "message lacks predicate depth: {msg}");
}
