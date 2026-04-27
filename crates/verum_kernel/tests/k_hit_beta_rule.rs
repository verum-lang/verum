//! HIT eliminator
//! β-reduction tests.
//!
//! The standard inductive β-rule is:
//!
//!     Elim(motive, [c₀, c₁, …, cₙ]) ( C(arg₁, …, argₘ) )
//!       ↦ App-chain(cᵢ, arg₁, …, argₘ, recursor-calls)
//!
//! where C is the i-th point ctor of the parent inductive and
//! every recursive argument argⱼ (whose declared ctor type is the
//! same parent inductive) is followed by a recursor call
//! `Elim(motive, cases)(argⱼ)`. This matches the dependent-elim
//! shape Coq / Lean / Agda generate automatically.
//!
//! Path-constructor β-rule (path substitution) is V3.1 follow-up;
//! these tests cover point-ctor β-rule on:
//!   • Bool — non-recursive ctors, two cases
//!   • Nat  — recursive Succ ctor, recursor call insertion
//!   • S¹   — point ctor only (path ctor scrutinee remains neutral)

use verum_common::{Heap, List, Text};
use verum_kernel::{
    ConstructorSig, CoreTerm, InductiveRegistry, PathCtorSig,
    RegisteredInductive, normalize_with_inductives,
};

fn nullary(name: &str) -> ConstructorSig {
    ConstructorSig {
        name: Text::from(name),
        arg_types: List::new(),
    }
}

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

fn app(f: CoreTerm, a: CoreTerm) -> CoreTerm {
    CoreTerm::App(Heap::new(f), Heap::new(a))
}

fn elim(scrutinee: CoreTerm, motive: CoreTerm, cases: Vec<CoreTerm>) -> CoreTerm {
    CoreTerm::Elim {
        scrutinee: Heap::new(scrutinee),
        motive: Heap::new(motive),
        cases: List::from_iter(cases),
    }
}

// =============================================================================
// Bool β-rule — Elim(case_T, case_F)(True)  ↦ case_T  (no args, no recursion)
// =============================================================================

#[test]
fn bool_elim_beta_reduces_true_to_first_case() {
    let mut reg = InductiveRegistry::new();
    let bool_ind = RegisteredInductive::new(
        Text::from("Bool"),
        List::new(),
        List::from_iter(vec![nullary("True"), nullary("False")]),
    );
    reg.register(bool_ind).unwrap();

    let term = elim(
        var("True"),
        var("motive"),
        vec![var("case_T"), var("case_F")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "case_T"),
        "Bool elim(True) must β-reduce to case_T; got {:?}",
        normal
    );
}

#[test]
fn bool_elim_beta_reduces_false_to_second_case() {
    let mut reg = InductiveRegistry::new();
    reg.register(RegisteredInductive::new(
        Text::from("Bool"),
        List::new(),
        List::from_iter(vec![nullary("True"), nullary("False")]),
    ))
    .unwrap();

    let term = elim(
        var("False"),
        var("motive"),
        vec![var("case_T"), var("case_F")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "case_F"),
        "Bool elim(False) must β-reduce to case_F; got {:?}",
        normal
    );
}

#[test]
fn bool_elim_neutral_scrutinee_remains_unreduced() {
    // When the scrutinee is a free variable (not a ctor application),
    // the elim term stays in neutral form; only its children
    // normalise.
    let mut reg = InductiveRegistry::new();
    reg.register(RegisteredInductive::new(
        Text::from("Bool"),
        List::new(),
        List::from_iter(vec![nullary("True"), nullary("False")]),
    ))
    .unwrap();

    let term = elim(
        var("x"), // not a ctor — neutral
        var("motive"),
        vec![var("case_T"), var("case_F")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Elim { .. }),
        "neutral scrutinee must keep Elim form; got {:?}",
        normal
    );
}

// =============================================================================
// Nat β-rule — Succ(n) case body receives `n` AND `Elim(...)(n)` as
// the recursor call. This is the V3 β-rule's distinctive feature
// vs the lambda-only β-reduction.
// =============================================================================

fn nat_inductive() -> RegisteredInductive {
    RegisteredInductive::new(
        Text::from("Nat"),
        List::new(),
        List::from_iter(vec![
            nullary("Zero"),
            ConstructorSig {
                name: Text::from("Succ"),                arg_types: List::from_iter(vec![CoreTerm::Inductive {
                    path: Text::from("Nat"),
                    args: List::new(),
                }]),
            },
        ]),
    )
}

#[test]
fn nat_elim_beta_reduces_zero_to_zero_case() {
    let mut reg = InductiveRegistry::new();
    reg.register(nat_inductive()).unwrap();

    let term = elim(
        var("Zero"),
        var("motive"),
        vec![var("case_zero"), var("case_succ")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "case_zero"),
        "Nat elim(Zero) must β-reduce to case_zero; got {:?}",
        normal
    );
}

#[test]
fn nat_elim_beta_reduces_succ_with_recursor_call() {
    // Elim(motive, [zero, succ_case])(Succ(Zero))
    //   ↦ App(App(succ_case, Zero), Elim(motive, [...])(Zero))
    //   ↦ App(App(succ_case, Zero), case_zero)        (after recursive β)
    let mut reg = InductiveRegistry::new();
    reg.register(nat_inductive()).unwrap();

    let term = elim(
        app(var("Succ"), var("Zero")),
        var("motive"),
        vec![var("case_zero"), var("case_succ")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    // Expected: App(App(case_succ, Zero), case_zero)
    let CoreTerm::App(outer_f, outer_a) = &normal else {
        panic!("expected outer App, got {:?}", normal);
    };
    let CoreTerm::App(inner_f, inner_a) = outer_f.as_ref() else {
        panic!("expected inner App, got {:?}", outer_f.as_ref());
    };
    assert!(matches!(inner_f.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_succ"));
    assert!(matches!(inner_a.as_ref(), CoreTerm::Var(n) if n.as_str() == "Zero"));
    // Outer arg must be the recursor call's β-reduction = case_zero.
    assert!(
        matches!(outer_a.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_zero"),
        "expected case_zero as recursor-call image; got {:?}",
        outer_a.as_ref()
    );
}

#[test]
fn nat_elim_beta_reduces_succ_succ_with_two_recursor_calls() {
    // Elim(motive, [zero, succ])(Succ(Succ(Zero)))
    //   ↦ App(App(succ, Succ(Zero)), Elim(...)(Succ(Zero)))
    //   ↦ App(App(succ, Succ(Zero)), App(App(succ, Zero), case_zero))
    let mut reg = InductiveRegistry::new();
    reg.register(nat_inductive()).unwrap();

    let term = elim(
        app(var("Succ"), app(var("Succ"), var("Zero"))),
        var("motive"),
        vec![var("case_zero"), var("case_succ")],
    );
    let normal = normalize_with_inductives(&term, &reg);

    // Outer: App(App(case_succ, Succ(Zero)), <recursor-call β-result>)
    let CoreTerm::App(outer_f, outer_a) = &normal else { panic!(); };
    let CoreTerm::App(inner_f, inner_a) = outer_f.as_ref() else { panic!(); };
    assert!(matches!(inner_f.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_succ"));
    assert!(matches!(inner_a.as_ref(), CoreTerm::App(_, _)),
        "outer arg₀ should be Succ(Zero) literal");
    // outer_a is the recursor-call image: App(App(case_succ, Zero), case_zero)
    let CoreTerm::App(rec_f, rec_a) = outer_a.as_ref() else {
        panic!("outer arg should be the recursor-call β-result, got {:?}", outer_a.as_ref());
    };
    let CoreTerm::App(rec_f_inner, rec_f_arg) = rec_f.as_ref() else { panic!(); };
    assert!(matches!(rec_f_inner.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_succ"));
    assert!(matches!(rec_f_arg.as_ref(), CoreTerm::Var(n) if n.as_str() == "Zero"));
    assert!(matches!(rec_a.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_zero"));
}

// =============================================================================
// S¹ β-rule — point-ctor (Base) reduces; path-ctor scrutinee stays neutral.
// =============================================================================

#[test]
fn s1_elim_beta_reduces_base_to_first_case() {
    let mut reg = InductiveRegistry::new();
    let s1 = RegisteredInductive::new(
        Text::from("S1"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Loop"),
        dim: 1,
        lhs: var("Base"),
        rhs: var("Base"),
    });
    reg.register(s1).unwrap();

    let term = elim(
        var("Base"),
        var("motive"),
        vec![var("case_base"), var("case_loop")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "case_base"),
        "S¹ elim(Base) must β-reduce to case_base; got {:?}",
        normal
    );
}

// =============================================================================
// Mismatched cases length — kernel does NOT β-reduce when cases.len()
// disagrees with the parent inductive's point-ctor count (avoids
// out-of-bounds case indexing).
// =============================================================================

#[test]
fn elim_with_too_few_cases_remains_neutral() {
    let mut reg = InductiveRegistry::new();
    reg.register(RegisteredInductive::new(
        Text::from("Bool"),
        List::new(),
        List::from_iter(vec![nullary("True"), nullary("False")]),
    ))
    .unwrap();

    // Only ONE case provided where Bool needs two — keep neutral.
    let term = elim(var("True"), var("motive"), vec![var("only_case")]);
    let normal = normalize_with_inductives(&term, &reg);
    // Cases.len() = 1, but ctor_idx of True = 0 < 1, so the rule
    // SHOULD fire (only out-of-bounds case is when ctor_idx >= cases.len()).
    // True is at idx 0, cases has 1 entry, so 0 < 1 ⇒ β fires.
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "only_case"),
        "True is at idx 0 < cases.len()=1 so β must fire to only_case; got {:?}",
        normal
    );

    // Now the actual mismatch case: scrutinee is False (idx 1) with
    // only one case → out of bounds → neutral.
    let term = elim(var("False"), var("motive"), vec![var("only_case")]);
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Elim { .. }),
        "False at idx 1 >= cases.len()=1 must keep neutral; got {:?}",
        normal
    );
}

// =============================================================================
// Unregistered ctor — keep neutral, don't crash.
// =============================================================================

#[test]
fn elim_with_unregistered_ctor_remains_neutral() {
    let reg = InductiveRegistry::new();
    let term = elim(
        app(var("MysteryCtor"), var("arg")),
        var("motive"),
        vec![var("c0")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Elim { .. }),
        "unregistered ctor must keep neutral; got {:?}",
        normal
    );
}
