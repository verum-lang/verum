//! K-Quot-Form / K-Quot-Intro / K-Quot-Elim integration tests
//! (V8 #236, VVA §7.5).
//!
//! Quotient types `T / ~` collapse equivalence classes of T into
//! single elements. The kernel checks:
//!   • K-Quot-Form: `Quotient(T, ~)` is a type when T is a type
//!     and ~ is well-typed; result inhabits T's universe.
//!   • K-Quot-Intro: `[t]_~ : Quotient(T, ~)` when t : T.
//!   • K-Quot-Elim: `quot_elim(q, motive, case)` typed as
//!     `motive(q)` when q : Quotient(T, ~), motive well-typed,
//!     case well-typed (respect-of-equivalence is V2 deferred).
//! Plus the β-rule: `quot_elim([t]_~, m, case) ↦ case(t)` via
//! the normaliser.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, FrameworkId, KernelError, UniverseLevel,
    definitional_eq, infer, normalize,
};

fn empty() -> (Context, AxiomRegistry) {
    (Context::new(), AxiomRegistry::new())
}

fn nat_ty() -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    }
}

/// A trivial equivalence relation: `λa.λb. true` (every pair related).
/// For typing purposes this stands for any well-typed relation; the
/// kernel doesn't internally verify reflexivity / symmetry /
/// transitivity (those are framework-axiom-attestable).
fn trivial_equiv() -> CoreTerm {
    CoreTerm::Lam {
        binder: Text::from("a"),
        domain: Heap::new(nat_ty()),
        body: Heap::new(CoreTerm::Lam {
            binder: Text::from("b"),
            domain: Heap::new(nat_ty()),
            body: Heap::new(CoreTerm::Universe(UniverseLevel::Prop)),
        }),
    }
}

// =============================================================================
// K-Quot-Form
// =============================================================================

#[test]
fn quot_form_inhabits_base_universe() {
    let (ctx, reg) = empty();
    let q = CoreTerm::Quotient {
        base: Heap::new(nat_ty()),
        equiv: Heap::new(trivial_equiv()),
    };
    let ty = infer(&ctx, &q, &reg).expect("Quotient well-formed");
    // Nat's universe is Concrete(0); Quotient inherits.
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
}

#[test]
fn quot_form_with_universe_base_inhabits_higher_universe() {
    let (ctx, reg) = empty();
    let q = CoreTerm::Quotient {
        base: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        equiv: Heap::new(trivial_equiv()),
    };
    let ty = infer(&ctx, &q, &reg).expect("Quotient over Type_0");
    // Type_0 is at Type_1; Quotient inherits.
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(1)));
}

#[test]
fn quot_form_rejects_ill_typed_base() {
    let (ctx, reg) = empty();
    // Base is an unbound variable → infer fails.
    let q = CoreTerm::Quotient {
        base: Heap::new(CoreTerm::Var(Text::from("Undefined"))),
        equiv: Heap::new(trivial_equiv()),
    };
    let res = infer(&ctx, &q, &reg);
    assert!(matches!(res, Err(KernelError::UnboundVariable(_))));
}

// =============================================================================
// K-Quot-Intro
// =============================================================================

#[test]
fn quot_intro_lifts_value_to_quotient() {
    let mut reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    };
    reg.register(Text::from("zero"), nat_ty(), fw.clone()).expect("zero axiom");
    let zero = CoreTerm::Axiom {
        name: Text::from("zero"),
        ty: Heap::new(nat_ty()),
        framework: fw,
    };
    let intro = CoreTerm::QuotIntro {
        value: Heap::new(zero),
        base: Heap::new(nat_ty()),
        equiv: Heap::new(trivial_equiv()),
    };
    let ty = infer(&Context::new(), &intro, &reg).expect("intro well-formed");
    let expected = CoreTerm::Quotient {
        base: Heap::new(nat_ty()),
        equiv: Heap::new(trivial_equiv()),
    };
    assert_eq!(ty, expected);
}

#[test]
fn quot_intro_rejects_value_at_wrong_type() {
    let mut reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    };
    let bool_ty = CoreTerm::Inductive {
        path: Text::from("Bool"),
        args: List::new(),
    };
    reg.register(Text::from("tt"), bool_ty.clone(), fw.clone()).expect("tt");
    // Value at Bool, base says Nat — must reject.
    let intro = CoreTerm::QuotIntro {
        value: Heap::new(CoreTerm::Axiom {
            name: Text::from("tt"),
            ty: Heap::new(bool_ty),
            framework: fw,
        }),
        base: Heap::new(nat_ty()),
        equiv: Heap::new(trivial_equiv()),
    };
    let res = infer(&Context::new(), &intro, &reg);
    assert!(matches!(res, Err(KernelError::TypeMismatch { .. })));
}

// =============================================================================
// K-Quot-Elim
// =============================================================================

#[test]
fn quot_elim_typed_as_motive_applied_to_scrutinee() {
    let mut reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    };
    reg.register(Text::from("zero"), nat_ty(), fw.clone()).expect("zero");
    let zero = CoreTerm::Axiom {
        name: Text::from("zero"),
        ty: Heap::new(nat_ty()),
        framework: fw,
    };
    let q_term = CoreTerm::QuotIntro {
        value: Heap::new(zero.clone()),
        base: Heap::new(nat_ty()),
        equiv: Heap::new(trivial_equiv()),
    };
    let motive = CoreTerm::Lam {
        binder: Text::from("_"),
        domain: Heap::new(CoreTerm::Quotient {
            base: Heap::new(nat_ty()),
            equiv: Heap::new(trivial_equiv()),
        }),
        body: Heap::new(nat_ty()),
    };
    let case = CoreTerm::Lam {
        binder: Text::from("t"),
        domain: Heap::new(nat_ty()),
        body: Heap::new(zero),
    };
    let elim = CoreTerm::QuotElim {
        scrutinee: Heap::new(q_term.clone()),
        motive: Heap::new(motive.clone()),
        case: Heap::new(case),
    };
    let ty = infer(&Context::new(), &elim, &reg).expect("elim well-formed");
    // Result type: motive applied to scrutinee.
    assert_eq!(ty, CoreTerm::App(Heap::new(motive), Heap::new(q_term)));
}

#[test]
fn quot_elim_rejects_non_quotient_scrutinee() {
    let mut reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    };
    reg.register(Text::from("zero"), nat_ty(), fw.clone()).expect("zero");
    let zero = CoreTerm::Axiom {
        name: Text::from("zero"),
        ty: Heap::new(nat_ty()),
        framework: fw,
    };
    // Scrutinee is a bare Nat value, not a Quotient — must reject.
    let elim = CoreTerm::QuotElim {
        scrutinee: Heap::new(zero),
        motive: Heap::new(CoreTerm::Lam {
            binder: Text::from("_"),
            domain: Heap::new(nat_ty()),
            body: Heap::new(nat_ty()),
        }),
        case: Heap::new(CoreTerm::Lam {
            binder: Text::from("t"),
            domain: Heap::new(nat_ty()),
            body: Heap::new(CoreTerm::Var(Text::from("t"))),
        }),
    };
    let res = infer(&Context::new(), &elim, &reg);
    assert!(matches!(res, Err(KernelError::TypeMismatch { .. })));
}

// =============================================================================
// β-rule: quot_elim([t]_~, m, case) ↦ case(t)
// =============================================================================

#[test]
fn quot_elim_beta_reduces_to_case_applied_to_value() {
    // case = λt.t (id on Nat)
    let case = CoreTerm::Lam {
        binder: Text::from("t"),
        domain: Heap::new(nat_ty()),
        body: Heap::new(CoreTerm::Var(Text::from("t"))),
    };
    let value = CoreTerm::Var(Text::from("z"));
    let intro = CoreTerm::QuotIntro {
        value: Heap::new(value.clone()),
        base: Heap::new(nat_ty()),
        equiv: Heap::new(trivial_equiv()),
    };
    let elim = CoreTerm::QuotElim {
        scrutinee: Heap::new(intro),
        motive: Heap::new(CoreTerm::Lam {
            binder: Text::from("_"),
            domain: Heap::new(nat_ty()),
            body: Heap::new(nat_ty()),
        }),
        case: Heap::new(case.clone()),
    };
    let normalised = normalize(&elim);
    // β: quot_elim([z]_~, _, λt.t) → (λt.t) z → z.
    assert_eq!(normalised, value);
}

#[test]
fn definitional_eq_recognises_quotient_beta() {
    // quot_elim([z]_~, m, λt.t) ≡_β z.
    let case = CoreTerm::Lam {
        binder: Text::from("t"),
        domain: Heap::new(nat_ty()),
        body: Heap::new(CoreTerm::Var(Text::from("t"))),
    };
    let z = CoreTerm::Var(Text::from("z"));
    let intro = CoreTerm::QuotIntro {
        value: Heap::new(z.clone()),
        base: Heap::new(nat_ty()),
        equiv: Heap::new(trivial_equiv()),
    };
    let elim = CoreTerm::QuotElim {
        scrutinee: Heap::new(intro),
        motive: Heap::new(CoreTerm::Lam {
            binder: Text::from("_"),
            domain: Heap::new(nat_ty()),
            body: Heap::new(nat_ty()),
        }),
        case: Heap::new(case),
    };
    assert!(definitional_eq(&elim, &z));
    // Symmetric.
    assert!(definitional_eq(&z, &elim));
}

// =============================================================================
// Round-trip: Z = ℕ × ℕ / ~ as the canonical setoid quotient
// =============================================================================

#[test]
fn setoid_z_construction_typechecks() {
    // Z = (Nat × Nat) / ~ where (a,b) ~ (c,d) iff a + d = b + c.
    // The trivial-equiv stand-in keeps the kernel typing rule's
    // soundness scope precise: the kernel checks structural
    // well-formedness, not the equivalence-respect obligation.
    let reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    };
    let nat2 = CoreTerm::Sigma {
        binder: Text::from("_"),
        fst_ty: Heap::new(nat_ty()),
        snd_ty: Heap::new(nat_ty()),
    };
    let z_equiv = CoreTerm::Lam {
        binder: Text::from("ab"),
        domain: Heap::new(nat2.clone()),
        body: Heap::new(CoreTerm::Lam {
            binder: Text::from("cd"),
            domain: Heap::new(nat2.clone()),
            body: Heap::new(CoreTerm::Universe(UniverseLevel::Prop)),
        }),
    };
    let z_ty = CoreTerm::Quotient {
        base: Heap::new(nat2.clone()),
        equiv: Heap::new(z_equiv),
    };
    let ty = infer(&Context::new(), &z_ty, &reg).expect("Z well-formed");
    // Result lives in Type_max(0, 0) = Universe(Max(Concrete(0),
    // Concrete(0))) per Sigma. Just verify it's at a Universe.
    let _ = (reg, fw);
    match ty {
        CoreTerm::Universe(_) => {}
        other => panic!("expected Universe, got {:?}", other),
    }
}
