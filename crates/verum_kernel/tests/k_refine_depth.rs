//! K-Refine depth-check integration tests (VVA §2.4 / §4.4).
//!
//! Diakrisis axiom T-2f* ported to the Verum kernel: a refinement type
//! `{ x : base | P(x) }` is well-formed only when the predicate's
//! M-iteration depth is strictly less than the base's depth + 1.
//! Yanofsky 2003 establishes that this closes every self-referential
//! paradox schema in a cartesian-closed setting by blocking the exact
//! equality `dp(α) = dp(T^α)` that Russell / Curry / Gödel-type
//! diagonals require.
//!
//! This file exercises the kernel rule end-to-end — both the valid
//! refinement paths that must keep type-checking cleanly AND the
//! paradox-shape refinements that must be rejected with the precise
//! `KernelError::DepthViolation { binder, base_depth, pred_depth }`
//! diagnostic.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, FrameworkId, KernelError, UniverseLevel, infer, m_depth,
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
    // Per VVA §4.3 + kernel m_depth, dp(Axiom { ty }) = dp(ty) so
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
fn k_refine_accepts_trivial_predicate_over_inductive_base() {
    // {n : Int | True} — predicate is a variable reference, depth 0;
    // base Int has depth 1; 0 < 1 + 1 = 2. Valid.
    let t = refine(ind("Int", vec![]), "n", tvar("n"));
    let result = infer(&Context::new(), &t, &AxiomRegistry::new());
    assert!(
        result.is_ok(),
        "expected valid refinement to type-check, got {result:?}"
    );
}

#[test]
fn k_refine_accepts_predicate_equal_to_base_depth() {
    // {l : List<Int> | l} — predicate depth 0 (just the bound var);
    // base List<Int> depth 2; 0 < 2 + 1 = 3. Valid.
    let t = refine(ind("List", vec![ind("Int", vec![])]), "l", tvar("l"));
    let result = infer(&Context::new(), &t, &AxiomRegistry::new());
    assert!(
        result.is_ok(),
        "expected nested-Inductive refinement to type-check, got {result:?}"
    );
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
    let one_up = ind("List", vec![base.clone()]);        // depth 2
    let pred = ind("P", vec![one_up]);                   // depth 3
    let t = refine(base, "n", pred);

    let result = infer(&Context::new(), &t, &AxiomRegistry::new());
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
    let pred = ind("List", vec![ind("List", vec![ind("List", vec![base.clone()])])]);
    let t = refine(base, "n", pred);

    match infer(&Context::new(), &t, &AxiomRegistry::new()) {
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
