//! K-PathTy β-normalization integration tests (V8, #216).
//!
//! Pre-V8 the PathTy formation rule used `structural_eq` (byte-
//! identity) to compare endpoint types against the carrier.
//! Definitionally-equal-but-syntactically-different terms (e.g.
//! `App(Lam(x, _, x), Nat) ≡_β Nat`) FALSELY REJECTED.
//!
//! V8 ships `support::normalize` (β-normaliser to fixed point or
//! `NORMALIZE_STEP_LIMIT` steps) and `support::definitional_eq`
//! (normalise-then-compare). PathTy formation now uses the latter.
//! These tests exercise both the β-aware completeness gain and the
//! corner cases (depth limits, neutral terms, recursive descent).

use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, FrameworkId, KernelError, NORMALIZE_STEP_LIMIT,
    UniverseLevel, definitional_eq, infer, normalize,
};

fn empty() -> (Context, AxiomRegistry) {
    (Context::new(), AxiomRegistry::new())
}

fn nat_ind() -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    }
}

// =============================================================================
// normalize() — β-reduction at the head + recursive descent
// =============================================================================

#[test]
fn normalize_atomic_var_is_idempotent() {
    let t = CoreTerm::Var(Text::from("x"));
    assert_eq!(normalize(&t), t);
}

#[test]
fn normalize_universe_is_idempotent() {
    let t = CoreTerm::Universe(UniverseLevel::Concrete(0));
    assert_eq!(normalize(&t), t);
}

#[test]
fn normalize_simple_beta_redex_reduces() {
    // (λx:Nat. x) y  →  y
    let id_lam = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let app = CoreTerm::App(
        Heap::new(id_lam),
        Heap::new(CoreTerm::Var(Text::from("y"))),
    );
    let normalized = normalize(&app);
    assert_eq!(normalized, CoreTerm::Var(Text::from("y")));
}

#[test]
fn normalize_nested_beta_redex_reduces_to_fixed_point() {
    // (λx. (λy. x) z) w  →  (λy. w) z  →  w
    let inner = CoreTerm::Lam {
        binder: Text::from("y"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let inner_app = CoreTerm::App(
        Heap::new(inner),
        Heap::new(CoreTerm::Var(Text::from("z"))),
    );
    let outer = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(inner_app),
    };
    let app = CoreTerm::App(
        Heap::new(outer),
        Heap::new(CoreTerm::Var(Text::from("w"))),
    );
    let normalized = normalize(&app);
    assert_eq!(normalized, CoreTerm::Var(Text::from("w")));
}

#[test]
fn normalize_neutral_app_keeps_application() {
    // F y where F is a free var (not a λ) — no β-redex, App
    // remains in the normal form (with both sides normalised
    // recursively).
    let app = CoreTerm::App(
        Heap::new(CoreTerm::Var(Text::from("F"))),
        Heap::new(CoreTerm::Var(Text::from("y"))),
    );
    let normalized = normalize(&app);
    assert_eq!(normalized, app);
}

#[test]
fn normalize_recurses_into_pi_codomain() {
    // Π(_: Nat). (λx. x) y — codomain has β-redex.
    let id_lam = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let app = CoreTerm::App(
        Heap::new(id_lam),
        Heap::new(CoreTerm::Var(Text::from("y"))),
    );
    let pi = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(nat_ind()),
        codomain: Heap::new(app),
    };
    let normalized = normalize(&pi);
    let expected = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(nat_ind()),
        codomain: Heap::new(CoreTerm::Var(Text::from("y"))),
    };
    assert_eq!(normalized, expected);
}

#[test]
fn normalize_sigma_projections_reduce_via_pair_beta() {
    // Fst((a, b))  →  a; Snd((a, b))  →  b.
    let pair = CoreTerm::Pair(
        Heap::new(CoreTerm::Var(Text::from("a"))),
        Heap::new(CoreTerm::Var(Text::from("b"))),
    );
    let fst = CoreTerm::Fst(Heap::new(pair.clone()));
    let snd = CoreTerm::Snd(Heap::new(pair));
    assert_eq!(normalize(&fst), CoreTerm::Var(Text::from("a")));
    assert_eq!(normalize(&snd), CoreTerm::Var(Text::from("b")));
}

#[test]
fn normalize_idempotent_on_already_normal_term() {
    // Π(x: A). x — already in normal form.
    let t = CoreTerm::Pi {
        binder: Text::from("x"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        codomain: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    assert_eq!(normalize(&t), t);
}

#[test]
fn normalize_descends_into_path_endpoints() {
    // PathTy(carrier=A, lhs=(λx.x) a, rhs=(λx.x) b) →
    // PathTy(A, a, b).
    let id = || CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let path = CoreTerm::PathTy {
        carrier: Heap::new(CoreTerm::Var(Text::from("A"))),
        lhs: Heap::new(CoreTerm::App(
            Heap::new(id()),
            Heap::new(CoreTerm::Var(Text::from("a"))),
        )),
        rhs: Heap::new(CoreTerm::App(
            Heap::new(id()),
            Heap::new(CoreTerm::Var(Text::from("b"))),
        )),
    };
    let normalized = normalize(&path);
    let expected = CoreTerm::PathTy {
        carrier: Heap::new(CoreTerm::Var(Text::from("A"))),
        lhs: Heap::new(CoreTerm::Var(Text::from("a"))),
        rhs: Heap::new(CoreTerm::Var(Text::from("b"))),
    };
    assert_eq!(normalized, expected);
}

#[test]
fn normalize_step_limit_constant_documented() {
    // The constant is exposed as part of the public surface so
    // callers can size their proof obligations against it.
    assert!(NORMALIZE_STEP_LIMIT >= 1_000);
}

// =============================================================================
// definitional_eq() — normalise-then-compare contract
// =============================================================================

#[test]
fn definitional_eq_handles_byte_identical_terms() {
    let t = CoreTerm::Var(Text::from("x"));
    assert!(definitional_eq(&t, &t));
}

#[test]
fn definitional_eq_handles_beta_equivalent_terms() {
    // (λx.x) y  ≡  y
    let id_lam = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let app = CoreTerm::App(
        Heap::new(id_lam),
        Heap::new(CoreTerm::Var(Text::from("y"))),
    );
    let bare = CoreTerm::Var(Text::from("y"));
    assert!(definitional_eq(&app, &bare));
    assert!(definitional_eq(&bare, &app)); // symmetric
}

#[test]
fn definitional_eq_distinguishes_genuinely_different_terms() {
    let a = CoreTerm::Var(Text::from("a"));
    let b = CoreTerm::Var(Text::from("b"));
    assert!(!definitional_eq(&a, &b));
}

// =============================================================================
// PathTy formation — β-aware endpoint matching
// =============================================================================

fn refl_axiom_at_nat(reg: &mut AxiomRegistry, name: &str) -> CoreTerm {
    let fw = FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    };
    let _ = reg.register(Text::from(name), nat_ind(), fw.clone());
    CoreTerm::Axiom {
        name: Text::from(name),
        ty: Heap::new(nat_ind()),
        framework: fw,
    }
}

#[test]
fn pathty_accepts_byte_identical_carrier_endpoints() {
    let (ctx, mut reg) = empty();
    let n = refl_axiom_at_nat(&mut reg, "n");
    let path = CoreTerm::PathTy {
        carrier: Heap::new(nat_ind()),
        lhs: Heap::new(n.clone()),
        rhs: Heap::new(n),
    };
    let res = infer(&ctx, &path, &reg);
    assert!(res.is_ok(), "byte-identical types must accept: {:?}", res);
}

#[test]
fn v8_pathty_accepts_beta_equivalent_carrier() {
    // Pre-V8 this was rejected because structural_eq compared
    // `nat_ty` against `App(Lam, Nat)` (which β-reduces to
    // Nat) and the equality failed.
    //
    // V8 normalizes both sides; the equality holds.
    let (ctx, mut reg) = empty();
    let n = refl_axiom_at_nat(&mut reg, "n_beta");
    // carrier = (λT:Type. T) Nat  ≡_β  Nat
    let id_type_lam = CoreTerm::Lam {
        binder: Text::from("T"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        body: Heap::new(CoreTerm::Var(Text::from("T"))),
    };
    let beta_carrier =
        CoreTerm::App(Heap::new(id_type_lam), Heap::new(nat_ind()));
    let path = CoreTerm::PathTy {
        carrier: Heap::new(beta_carrier),
        lhs: Heap::new(n.clone()),
        rhs: Heap::new(n),
    };
    let res = infer(&ctx, &path, &reg);
    assert!(
        res.is_ok(),
        "V8 β-aware match accepts: {:?}",
        res,
    );
}

// =============================================================================
// App-rule β-aware domain match
// =============================================================================

#[test]
fn b221_app_accepts_beta_equivalent_domain() {
    // Pre-V8 the App rule used structural_eq for the domain
    // match. A Π whose domain has a β-redex (e.g., (λT. T) Nat
    // ≡_β Nat) would falsely reject any arg typed at Nat.
    //
    // V8 lifts to definitional_eq → both sides normalised before
    // comparison → application admitted.
    let (ctx, mut reg) = empty();
    let n = refl_axiom_at_nat(&mut reg, "n_app_beta");
    // f : Π(_: (λT:Type. T) Nat). Nat
    //   = (λu: Nat. u : Nat → Nat) wrapped via Lam over the
    //     β-redex domain.
    let beta_dom = CoreTerm::App(
        Heap::new(CoreTerm::Lam {
            binder: Text::from("T"),
            domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
            body: Heap::new(CoreTerm::Var(Text::from("T"))),
        }),
        Heap::new(nat_ind()),
    );
    let f = CoreTerm::Lam {
        binder: Text::from("u"),
        domain: Heap::new(beta_dom),
        body: Heap::new(CoreTerm::Var(Text::from("u"))),
    };
    let app = CoreTerm::App(Heap::new(f), Heap::new(n));
    let res = infer(&ctx, &app, &reg);
    assert!(
        res.is_ok(),
        "V8 β-aware App admits β-equivalent domain: {:?}",
        res,
    );
}

#[test]
fn b221_app_rejects_truly_different_domain() {
    // Sanity: if the domain truly differs (carrier = Bool, arg
    // type = Nat), the App rule still rejects — definitional_eq
    // is monotone-strengthening, not unsound.
    let (ctx, mut reg) = empty();
    let n = refl_axiom_at_nat(&mut reg, "n_app_mismatch");
    let bool_ind = CoreTerm::Inductive {
        path: Text::from("Bool"),
        args: List::new(),
    };
    // f : Bool → Bool — but we apply with a Nat-typed arg.
    let f = CoreTerm::Lam {
        binder: Text::from("b"),
        domain: Heap::new(bool_ind),
        body: Heap::new(CoreTerm::Var(Text::from("b"))),
    };
    let app = CoreTerm::App(Heap::new(f), Heap::new(n));
    let res = infer(&ctx, &app, &reg);
    assert!(
        matches!(res, Err(KernelError::TypeMismatch { .. })),
        "type-mismatched App must reject: {:?}",
        res,
    );
}

#[test]
fn pathty_rejects_endpoint_at_different_type() {
    // PathTy<Nat>(zero, true_value) — true_value is at Bool, not
    // Nat. Even after normalization, the types differ → reject.
    let (ctx, mut reg) = empty();
    let n = refl_axiom_at_nat(&mut reg, "n_mismatch");
    let bool_axiom = {
        let fw = FrameworkId {
            framework: Text::from("test"),
            citation: Text::from("test"),
        };
        let bool_ind = CoreTerm::Inductive {
            path: Text::from("Bool"),
            args: List::new(),
        };
        let _ = reg.register(Text::from("tt_bool"), bool_ind.clone(), fw.clone());
        CoreTerm::Axiom {
            name: Text::from("tt_bool"),
            ty: Heap::new(bool_ind),
            framework: fw,
        }
    };
    let path = CoreTerm::PathTy {
        carrier: Heap::new(nat_ind()),
        lhs: Heap::new(n),
        rhs: Heap::new(bool_axiom),
    };
    let res = infer(&ctx, &path, &reg);
    assert!(
        matches!(res, Err(KernelError::TypeMismatch { .. })),
        "type-mismatched endpoint must reject: {:?}",
        res,
    );
}
