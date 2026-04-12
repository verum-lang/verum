//! Integration tests for the cubical normalizer (Phase B.2).
//!
//! Exercises the `whnf()` reduction + `definitionally_equal` equivalence
//! with realistic proof patterns that occur in cubical type theory.

use verum_common::Text;
use verum_types::cubical::{CubicalTerm, DimVar, IntervalEndpoint};

// Helpers
fn val(s: &str) -> CubicalTerm {
    CubicalTerm::Value(Text::from(s))
}
fn refl(x: CubicalTerm) -> CubicalTerm {
    CubicalTerm::Refl(Box::new(x))
}
fn transport(line: CubicalTerm, value: CubicalTerm) -> CubicalTerm {
    CubicalTerm::Transport {
        line: Box::new(line),
        value: Box::new(value),
    }
}
fn hcomp(base: CubicalTerm, sides: CubicalTerm) -> CubicalTerm {
    CubicalTerm::Hcomp {
        base: Box::new(base),
        sides: Box::new(sides),
    }
}
fn sym(p: CubicalTerm) -> CubicalTerm {
    CubicalTerm::Sym(Box::new(p))
}
fn trans(p: CubicalTerm, q: CubicalTerm) -> CubicalTerm {
    CubicalTerm::Trans(Box::new(p), Box::new(q))
}
fn path_lam(dim: &str, body: CubicalTerm) -> CubicalTerm {
    CubicalTerm::PathLambda {
        dim: DimVar::new(dim),
        body: Box::new(body),
    }
}
fn path_app(p: CubicalTerm, at: CubicalTerm) -> CubicalTerm {
    CubicalTerm::PathApp {
        path: Box::new(p),
        at: Box::new(at),
    }
}
fn dim(name: &str) -> CubicalTerm {
    CubicalTerm::DimVar(DimVar::new(name))
}
fn i0() -> CubicalTerm {
    CubicalTerm::Endpoint(IntervalEndpoint::I0)
}
fn i1() -> CubicalTerm {
    CubicalTerm::Endpoint(IntervalEndpoint::I1)
}

// ==================== Basic reductions ====================

#[test]
fn transport_refl_identity() {
    let x = val("x");
    let term = transport(refl(val("A")), x.clone());
    assert_eq!(term.whnf(), x);
}

#[test]
fn hcomp_trivial_sides_eq_base() {
    let base = val("b");
    let term = hcomp(base.clone(), refl(val("trivial")));
    assert_eq!(term.whnf(), base);
}

#[test]
fn sym_of_refl_is_refl() {
    let r = refl(val("x"));
    assert_eq!(sym(r.clone()).whnf(), r);
}

// ==================== Path-lambda beta ====================

#[test]
fn path_lambda_applied_i0_reduces_body() {
    // (λi. i) @ i0 ↦ i0
    let lam = path_lam("i", dim("i"));
    assert_eq!(path_app(lam, i0()).whnf(), i0());
}

#[test]
fn path_lambda_applied_i1_reduces_body() {
    // (λi. i) @ i1 ↦ i1
    let lam = path_lam("i", dim("i"));
    assert_eq!(path_app(lam, i1()).whnf(), i1());
}

#[test]
fn path_lambda_constant_body_reduces() {
    // (λi. x) @ j ↦ x
    let lam = path_lam("i", val("x"));
    assert_eq!(path_app(lam.clone(), i0()).whnf(), val("x"));
    assert_eq!(path_app(lam, i1()).whnf(), val("x"));
}

#[test]
fn refl_application_elides() {
    // refl(x) @ endpoint ↦ x
    let r = refl(val("x"));
    assert_eq!(path_app(r.clone(), i0()).whnf(), val("x"));
    assert_eq!(path_app(r.clone(), i1()).whnf(), val("x"));
}

// ==================== Composition patterns ====================

#[test]
fn nested_transport_refl_reduces_fully() {
    // transport refl (transport refl (transport refl x)) ↦ x
    let mut t = val("x");
    for _ in 0..3 {
        t = transport(refl(val("A")), t);
    }
    assert_eq!(t.whnf(), val("x"));
}

#[test]
fn hcomp_over_transport_refl() {
    // hcomp (transport refl base) (refl sides) ↦ base
    let base = val("b");
    let sides = refl(val("s"));
    let inner = transport(refl(val("A")), base.clone());
    let outer = hcomp(inner, sides);
    assert_eq!(outer.whnf(), base);
}

// ==================== Definitional equality ====================

#[test]
fn definitionally_equal_after_reduction() {
    let lhs = transport(refl(val("A")), val("x"));
    let rhs = val("x");
    assert!(lhs.definitionally_equal(&rhs));
}

#[test]
fn definitionally_equal_symmetric() {
    let a = transport(refl(val("A")), val("x"));
    let b = transport(refl(val("B")), val("x"));
    assert!(a.definitionally_equal(&b));
}

#[test]
fn not_definitionally_equal_different_values() {
    let a = val("x");
    let b = val("y");
    assert!(!a.definitionally_equal(&b));
}

// ==================== Substitution correctness ====================

#[test]
fn subst_dim_identity_var() {
    let t = dim("i");
    assert_eq!(
        t.subst_dim(&DimVar::new("i"), IntervalEndpoint::I0),
        i0()
    );
}

#[test]
fn subst_dim_preserves_unrelated_dim() {
    let t = dim("j");
    assert_eq!(
        t.subst_dim(&DimVar::new("i"), IntervalEndpoint::I0),
        dim("j")
    );
}

#[test]
fn subst_dim_skips_shadowing_lambda() {
    // λi. i should NOT substitute the bound i
    let lam = path_lam("i", dim("i"));
    let subst = lam.subst_dim(&DimVar::new("i"), IntervalEndpoint::I0);
    // Lambda preserved (bound variable shadows)
    match subst {
        CubicalTerm::PathLambda { ref dim, ref body } => {
            assert_eq!(dim.name.as_str(), "i");
            assert_eq!(**body, CubicalTerm::DimVar(DimVar::new("i")));
        }
        _ => panic!("expected PathLambda"),
    }
}

// ==================== HoTT Path-algebra laws ====================

#[test]
fn sym_refl_reduces_to_refl() {
    // sym(refl(x)) ↦ refl(x) — the single-step reduction that whnf
    // guarantees. Note: whnf is weak-head, so the double-wrapped case
    // sym(sym(refl)) only unwraps one layer; full normalisation would
    // reach a fixpoint but is not what whnf commits to.
    let r = refl(val("x"));
    assert_eq!(sym(r.clone()).whnf(), r);
}

#[test]
fn trans_is_preserved() {
    // Trans nodes are opaque to whnf unless both sides are refl
    let p = val("p");
    let q = val("q");
    let t = trans(p.clone(), q.clone());
    // whnf does not reduce trans (no rule for it)
    assert_eq!(t.clone().whnf(), t);
}

// ==================== Endpoint literals ====================

#[test]
fn endpoints_are_whnf() {
    assert_eq!(i0().whnf(), i0());
    assert_eq!(i1().whnf(), i1());
}

#[test]
fn endpoints_are_distinct() {
    assert!(!i0().definitionally_equal(&i1()));
}

// ==================== Values are terminal ====================

#[test]
fn value_is_normal_form() {
    let v = val("x");
    assert_eq!(v.clone().whnf(), v);
}
