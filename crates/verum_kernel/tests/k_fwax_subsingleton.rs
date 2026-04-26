//! K-FwAx subsingleton-check integration tests (V8, #217).
//!
//! Per `verification-architecture.md` §4.4 and §4.5, a framework
//! axiom's body must be a *subsingleton* (proof-irrelevant: at
//! most one inhabitant up to definitional equality) for subject
//! reduction to hold. Two acceptance routes:
//!
//!   1. **Closed-proposition route** — body has no free vars.
//!   2. **UIP route** — body may have free vars iff module
//!      imports `core.math.frameworks.uip` (caller signals via
//!      [`SubsingletonRegime::UipPermitted`]).
//!
//! These tests exercise both routes plus the legacy-unchecked
//! shim used for backwards-compat with pre-V8 callers.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, CoreTerm, FrameworkId, KernelError, SubsingletonRegime, UniverseLevel,
    free_vars,
};

fn fw(name: &str) -> FrameworkId {
    FrameworkId {
        framework: Text::from(name),
        citation: Text::from("test"),
    }
}

fn unit_ty() -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from("Unit"),
        args: List::new(),
    }
}

fn closed_prop_ty() -> CoreTerm {
    // `Π (_: Unit). Unit` — closed; no free vars.
    CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(unit_ty()),
        codomain: Heap::new(unit_ty()),
    }
}

fn open_prop_ty() -> CoreTerm {
    // `Π (_: A). A` — A is a free variable; not closed.
    CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        codomain: Heap::new(CoreTerm::Var(Text::from("A"))),
    }
}

// =============================================================================
// free_vars — pure walker contract
// =============================================================================

#[test]
fn free_vars_atomic_var_is_free() {
    let t = CoreTerm::Var(Text::from("A"));
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 1);
    assert!(fv.contains(&Text::from("A")));
}

#[test]
fn free_vars_universe_has_none() {
    let t = CoreTerm::Universe(UniverseLevel::Concrete(0));
    assert!(free_vars(&t).is_empty());
}

#[test]
fn free_vars_pi_binds_its_binder() {
    // Pi(x: A). x — `x` is bound by the Pi; `A` is free.
    let t = CoreTerm::Pi {
        binder: Text::from("x"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        codomain: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 1);
    assert!(fv.contains(&Text::from("A")));
    assert!(!fv.contains(&Text::from("x")));
}

#[test]
fn free_vars_nested_pi_correctly_scopes_each_binder() {
    // Pi(x: A). Pi(y: x). y — both `x` and `y` are bound; `A` is free.
    let inner = CoreTerm::Pi {
        binder: Text::from("y"),
        domain: Heap::new(CoreTerm::Var(Text::from("x"))),
        codomain: Heap::new(CoreTerm::Var(Text::from("y"))),
    };
    let outer = CoreTerm::Pi {
        binder: Text::from("x"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        codomain: Heap::new(inner),
    };
    let fv = free_vars(&outer);
    assert_eq!(fv.len(), 1);
    assert!(fv.contains(&Text::from("A")));
}

#[test]
fn free_vars_lambda_binds_correctly() {
    // λ(z: B). z — `z` bound; `B` free.
    let t = CoreTerm::Lam {
        binder: Text::from("z"),
        domain: Heap::new(CoreTerm::Var(Text::from("B"))),
        body: Heap::new(CoreTerm::Var(Text::from("z"))),
    };
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 1);
    assert!(fv.contains(&Text::from("B")));
}

#[test]
fn free_vars_sigma_binds_correctly() {
    // Σ(x: A). B — `x` bound in B; A and B both free.
    let t = CoreTerm::Sigma {
        binder: Text::from("x"),
        fst_ty: Heap::new(CoreTerm::Var(Text::from("A"))),
        snd_ty: Heap::new(CoreTerm::Var(Text::from("B"))),
    };
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 2);
    assert!(fv.contains(&Text::from("A")));
    assert!(fv.contains(&Text::from("B")));
}

#[test]
fn free_vars_refine_binds_predicate_var() {
    // {x: A | x} — `x` bound in predicate; `A` free.
    let t = CoreTerm::Refine {
        base: Heap::new(CoreTerm::Var(Text::from("A"))),
        binder: Text::from("x"),
        predicate: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 1);
    assert!(fv.contains(&Text::from("A")));
}

#[test]
fn free_vars_app_collects_both_sides() {
    let t = CoreTerm::App(
        Heap::new(CoreTerm::Var(Text::from("F"))),
        Heap::new(CoreTerm::Var(Text::from("X"))),
    );
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 2);
}

#[test]
fn free_vars_path_ty_collects_carrier_endpoints() {
    let t = CoreTerm::PathTy {
        carrier: Heap::new(CoreTerm::Var(Text::from("A"))),
        lhs: Heap::new(CoreTerm::Var(Text::from("a"))),
        rhs: Heap::new(CoreTerm::Var(Text::from("b"))),
    };
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 3);
}

#[test]
fn free_vars_universe_inductive_path_is_global_not_free() {
    let t = CoreTerm::Inductive {
        path: Text::from("core.collections.list.List"),
        args: List::new(),
    };
    assert!(free_vars(&t).is_empty());
}

#[test]
fn free_vars_inductive_args_descended() {
    let mut args: List<CoreTerm> = List::new();
    args.push(CoreTerm::Var(Text::from("Tparam")));
    let t = CoreTerm::Inductive {
        path: Text::from("List"),
        args,
    };
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 1);
    assert!(fv.contains(&Text::from("Tparam")));
}

#[test]
fn free_vars_modal_box_descends() {
    let t = CoreTerm::ModalBox(Heap::new(CoreTerm::Var(Text::from("p"))));
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 1);
    assert!(fv.contains(&Text::from("p")));
}

#[test]
fn free_vars_modal_big_and_supremum_over_args() {
    let mut args: List<Heap<CoreTerm>> = List::new();
    args.push(Heap::new(CoreTerm::Var(Text::from("p"))));
    args.push(Heap::new(CoreTerm::Var(Text::from("q"))));
    let t = CoreTerm::ModalBigAnd(args);
    let fv = free_vars(&t);
    assert_eq!(fv.len(), 2);
}

#[test]
fn free_vars_smt_proof_has_none() {
    // Certificates carry only opaque trace bytes; no syntactic vars.
    let cert = verum_kernel::SmtCertificate::new(
        Text::from("z3"),
        Text::from("4.13.0"),
        List::new(),
        Text::from("sha:abc"),
    );
    let t = CoreTerm::SmtProof(cert);
    assert!(free_vars(&t).is_empty());
}

// =============================================================================
// register_subsingleton — closed-proposition route
// =============================================================================

#[test]
fn closed_axiom_admitted_under_subsingleton_regime() {
    let mut reg = AxiomRegistry::new();
    let res = reg.register_subsingleton(
        Text::from("closed_witness"),
        closed_prop_ty(),
        fw("test_corpus"),
    );
    assert!(res.is_ok(), "closed axiom must be admitted: {:?}", res);
}

#[test]
fn open_axiom_rejected_under_subsingleton_regime() {
    let mut reg = AxiomRegistry::new();
    let res = reg.register_subsingleton(
        Text::from("open_witness"),
        open_prop_ty(),
        fw("test_corpus"),
    );
    match res {
        Err(KernelError::AxiomNotSubsingleton {
            name,
            free_vars_count,
            free_vars_rendered,
        }) => {
            assert_eq!(name.as_str(), "open_witness");
            assert_eq!(free_vars_count, 1);
            assert!(
                free_vars_rendered.as_str().contains("A"),
                "rendered must include A: {}",
                free_vars_rendered.as_str(),
            );
        }
        other => panic!(
            "expected AxiomNotSubsingleton, got {:?}",
            other,
        ),
    }
}

#[test]
fn open_axiom_admitted_under_uip_permitted_regime() {
    use verum_kernel::AxiomRegistry;
    let mut reg = AxiomRegistry::new();
    let res = reg.register_with_regime(
        Text::from("uip_axiom_user"),
        open_prop_ty(),
        fw("test_corpus"),
        SubsingletonRegime::UipPermitted,
    );
    assert!(
        res.is_ok(),
        "UIP regime admits open body: {:?}",
        res,
    );
}

#[test]
fn open_axiom_admitted_under_legacy_regime() {
    let mut reg = AxiomRegistry::new();
    let res = reg.register_with_regime(
        Text::from("legacy_axiom"),
        open_prop_ty(),
        fw("test_corpus"),
        SubsingletonRegime::LegacyUnchecked,
    );
    assert!(res.is_ok(), "legacy shim must admit unchecked");
}

#[test]
fn legacy_register_preserves_pre_v8_behaviour() {
    // The original `register()` entry point must continue to
    // accept open bodies — backwards compat for the existing
    // test corpus + stdlib bring-up registrations.
    let mut reg = AxiomRegistry::new();
    let res = reg.register(
        Text::from("legacy_via_register"),
        open_prop_ty(),
        fw("test_corpus"),
    );
    assert!(res.is_ok(), "register() must preserve pre-V8 semantics");
}

#[test]
fn duplicate_name_rejected_before_subsingleton_check() {
    let mut reg = AxiomRegistry::new();
    reg.register_subsingleton(
        Text::from("dup"),
        closed_prop_ty(),
        fw("test_corpus"),
    )
    .expect("first registration succeeds");
    let res = reg.register_subsingleton(
        Text::from("dup"),
        closed_prop_ty(),
        fw("test_corpus"),
    );
    assert!(matches!(res, Err(KernelError::DuplicateAxiom(_))));
}

#[test]
fn uip_shape_rejected_under_all_regimes() {
    use verum_common::Heap;
    // Construct the precise UIP shape per inductive::is_uip_shape:
    //   Π A. Π a. Π b. Π p. Π q. PathTy(PathTy(A, a, b), p, q)
    let path_inner = CoreTerm::PathTy {
        carrier: Heap::new(CoreTerm::Var(Text::from("A"))),
        lhs: Heap::new(CoreTerm::Var(Text::from("a"))),
        rhs: Heap::new(CoreTerm::Var(Text::from("b"))),
    };
    let path_outer = CoreTerm::PathTy {
        carrier: Heap::new(path_inner.clone()),
        lhs: Heap::new(CoreTerm::Var(Text::from("p"))),
        rhs: Heap::new(CoreTerm::Var(Text::from("q"))),
    };
    let pi_q = CoreTerm::Pi {
        binder: Text::from("q"),
        domain: Heap::new(path_inner.clone()),
        codomain: Heap::new(path_outer),
    };
    let pi_p = CoreTerm::Pi {
        binder: Text::from("p"),
        domain: Heap::new(path_inner.clone()),
        codomain: Heap::new(pi_q),
    };
    let pi_b = CoreTerm::Pi {
        binder: Text::from("b"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        codomain: Heap::new(pi_p),
    };
    let pi_a = CoreTerm::Pi {
        binder: Text::from("a"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        codomain: Heap::new(pi_b),
    };
    let uip_shape = CoreTerm::Pi {
        binder: Text::from("A"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(pi_a),
    };

    // Under every regime, UIP-shape rejection fires before
    // subsingleton-check; the existing UipForbidden gate is
    // preserved.
    for regime in [
        SubsingletonRegime::ClosedPropositionOnly,
        SubsingletonRegime::UipPermitted,
        SubsingletonRegime::LegacyUnchecked,
    ] {
        let mut reg = AxiomRegistry::new();
        let res = reg.register_with_regime(
            Text::from("uip_attempt"),
            uip_shape.clone(),
            fw("test_corpus"),
            regime,
        );
        assert!(
            matches!(res, Err(KernelError::UipForbidden(_))),
            "UIP-shape must reject under regime {:?}, got {:?}",
            regime,
            res,
        );
    }
}

#[test]
fn rejected_subsingleton_check_does_not_commit_entry() {
    let mut reg = AxiomRegistry::new();
    let _ = reg.register_subsingleton(
        Text::from("rejected"),
        open_prop_ty(),
        fw("test_corpus"),
    );
    assert!(reg.all().is_empty(), "rejected registration must not leak");
    // Re-register with closed body — must succeed.
    let res = reg.register_subsingleton(
        Text::from("rejected"),
        closed_prop_ty(),
        fw("test_corpus"),
    );
    assert!(res.is_ok());
    assert_eq!(reg.all().len(), 1);
}

// =============================================================================
// V8 (#220) — body-is-Prop check at register time
// =============================================================================

#[test]
fn closed_inductive_body_admitted_as_prop() {
    // Unit is at Universe(Concrete(0)) — admitted under the
    // pragmatic Prop check (Concrete(0) ≈ Prop set-theoretically).
    let mut reg = AxiomRegistry::new();
    let res =
        reg.register_subsingleton(Text::from("unit_witness"), unit_ty(), fw("test"));
    assert!(res.is_ok(), "Unit body must be admitted: {:?}", res);
}

#[test]
fn closed_pi_body_at_type_zero_admitted_as_prop() {
    // Π(_: Unit). Unit lives at Universe(Max(0, 0)) — pragmatic
    // Prop accepts (no component >= 1).
    let mut reg = AxiomRegistry::new();
    let res = reg.register_subsingleton(
        Text::from("pi_witness"),
        closed_prop_ty(),
        fw("test"),
    );
    assert!(res.is_ok(), "Pi at Type_0 must be admitted: {:?}", res);
}

#[test]
fn b220_non_type_body_rejected_as_not_prop() {
    // A body that ISN'T a type (e.g., a Var that somehow
    // type-checks to a non-Universe) — unbound-Var case
    // surfaces as "infer-failed".
    //
    // We construct a body that is provably a non-Universe under
    // empty Γ + empty axioms: a `Refl(x)` term — its inferred
    // type is `PathTy<...>(x, x)` which is NOT a Universe head.
    // Under the closed-prop regime this should fail body-is-
    // Prop.
    use verum_kernel::CoreTerm;
    let mut reg = AxiomRegistry::new();
    let bogus_body = CoreTerm::Refl(verum_common::Heap::new(unit_ty()));
    let res = reg.register_subsingleton(
        Text::from("bogus_axiom"),
        bogus_body,
        fw("test"),
    );
    match res {
        Err(KernelError::AxiomBodyNotProp { name, inferred_universe_shape }) => {
            assert_eq!(name.as_str(), "bogus_axiom");
            // Refl's type infers to PathTy → shape head "Path".
            assert!(
                inferred_universe_shape.as_str().contains("Path")
                    || inferred_universe_shape.as_str() == "infer-failed",
                "shape should mention Path or be infer-failed: {}",
                inferred_universe_shape.as_str(),
            );
        }
        other => panic!("expected AxiomBodyNotProp, got {:?}", other),
    }
}

#[test]
fn b220_closed_body_with_unbound_inductive_admitted_via_pragmatic_fallback() {
    // The body references an unregistered inductive ("Bool" not
    // populated in InductiveRegistry). The legacy `infer` shim
    // (no registry) falls back to Universe(Concrete(0)) for
    // every Inductive arm — so this passes the body-is-Prop
    // check. Documents the fallback behaviour: production code
    // that wants stricter checking should populate an
    // InductiveRegistry alongside the axiom registry.
    use verum_kernel::CoreTerm;
    use verum_common::List;
    let mut reg = AxiomRegistry::new();
    let bool_ind = CoreTerm::Inductive {
        path: Text::from("Bool"),
        args: List::new(),
    };
    let res = reg.register_subsingleton(
        Text::from("bool_axiom"),
        bool_ind,
        fw("test"),
    );
    assert!(
        res.is_ok(),
        "unregistered inductive admitted under pragmatic fallback: {:?}",
        res,
    );
}

#[test]
fn b220_uip_permitted_skips_prop_check() {
    // UipPermitted regime delegates the inhabitation-uniqueness
    // obligation to the imported UIP framework — the Prop check
    // is also skipped (it's part of the same admission gate that
    // the UIP framework implicitly satisfies via its own rules).
    use verum_kernel::CoreTerm;
    let mut reg = AxiomRegistry::new();
    // Body that's NOT a Universe — non-Prop under strict
    // pragmatic check, but UipPermitted skips it.
    let non_prop_body = CoreTerm::Refl(verum_common::Heap::new(unit_ty()));
    let res = reg.register_with_regime(
        Text::from("uip_relaxed"),
        non_prop_body,
        fw("test"),
        SubsingletonRegime::UipPermitted,
    );
    assert!(
        res.is_ok(),
        "UipPermitted skips Prop check: {:?}",
        res,
    );
}

#[test]
fn b220_legacy_unchecked_skips_prop_check() {
    // Backwards-compat: LegacyUnchecked admits anything that
    // passes the duplicate + UIP-shape gates. Existing test
    // corpus continues to work.
    use verum_kernel::CoreTerm;
    let mut reg = AxiomRegistry::new();
    let non_prop_body = CoreTerm::Refl(verum_common::Heap::new(unit_ty()));
    let res = reg.register_with_regime(
        Text::from("legacy"),
        non_prop_body,
        fw("test"),
        SubsingletonRegime::LegacyUnchecked,
    );
    assert!(res.is_ok(), "Legacy regime preserves pre-V8: {:?}", res);
}

#[test]
fn b220_subsingleton_check_runs_before_prop_check() {
    // Ordering: subsingleton check (free-vars) precedes
    // body-is-Prop. An open body fails subsingleton FIRST and
    // never reaches the Prop check.
    let mut reg = AxiomRegistry::new();
    let res = reg.register_subsingleton(
        Text::from("open_body"),
        open_prop_ty(),
        fw("test"),
    );
    match res {
        Err(KernelError::AxiomNotSubsingleton { .. }) => {}
        other => panic!(
            "expected subsingleton failure first, got {:?}",
            other,
        ),
    }
}

#[test]
fn diagnostic_renders_free_vars_sorted_for_determinism() {
    // BTreeSet ordering means `A`, `B`, `C` must appear in that
    // order in the diagnostic regardless of how the caller built
    // the term.
    let t = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(CoreTerm::Var(Text::from("C"))),
        codomain: Heap::new(CoreTerm::Pi {
            binder: Text::from("_"),
            domain: Heap::new(CoreTerm::Var(Text::from("A"))),
            codomain: Heap::new(CoreTerm::Var(Text::from("B"))),
        }),
    };
    let mut reg = AxiomRegistry::new();
    let err = reg
        .register_subsingleton(Text::from("multi_open"), t, fw("test_corpus"))
        .expect_err("must reject");
    match err {
        KernelError::AxiomNotSubsingleton { free_vars_rendered, .. } => {
            // Sorted: "A, B, C"
            assert_eq!(free_vars_rendered.as_str(), "A, B, C");
        }
        other => panic!("expected AxiomNotSubsingleton, got {:?}", other),
    }
}
