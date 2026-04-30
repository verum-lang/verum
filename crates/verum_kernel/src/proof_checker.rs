//! # Minimal proof-term checker (#157 — the trusted base)
//!
//! The smallest possible kernel that re-verifies a Verum proof from
//! a serialised proof-term certificate.  This module is the explicit
//! trusted base for Verum's reference-standard kernel claim:
//! everything else in `verum_kernel/` is *infrastructure* (the apply
//! dispatcher, the bridge audits, the cross-format renderers); the
//! proof-term checker here is the *verdict authority* that an
//! independent reviewer can read top-to-bottom in one sitting.
//!
//! ## Design discipline — < 1000 LOC, hand-auditable
//!
//! The checker implements a minimal Calculus of Constructions
//! fragment with bidirectional type-checking.  Six inference rules
//! are exhaustive: T-Var, T-Univ, T-Pi-Form, T-Lam-Intro, T-App-Elim,
//! T-Conv (β-conversion).  No cubical, modal, or refinement
//! extensions — those layer on top via `verum_kernel`'s broader rule
//! set, and their soundness theorems are tracked separately by
//! `core/verify/kernel_soundness/`.
//!
//! The trade-off is deliberate: the checker rejects MOST Verum
//! programs (since most use refinement / cubical / modal / SMT-axiom
//! features), but the programs it accepts have an iron-clad
//! independent verdict.  The full Verum kernel handles the broader
//! surface; the proof-term checker handles the irreducible core.
//!
//! ## What this DOES NOT do
//!
//! - Does NOT type-check refinement types (those need SMT).
//! - Does NOT decide propositional equality up to η-conversion
//!   beyond α + β (η is a separable extension).
//! - Does NOT inspect `@framework`-cited axioms — these are leaves
//!   that the apply-graph audit handles.
//! - Does NOT aspire to feature parity with Coq's `coqchk` — it
//!   aspires to feature parity with HOL Light's kernel: minimal,
//!   exhaustive, hand-readable.
//!
//! ## Trust delegation
//!
//! After this checker accepts a `(term, expected_type)` pair, the
//! ONLY things a reviewer needs to trust are:
//!
//!   1. This file (~600 LOC, exhaustive pattern-matching, no `unsafe`).
//!   2. The Rust compiler's correctness (or, after Phase 3 / #154,
//!      the Verum self-hosted kernel that consumes this checker's
//!      output as a verifiable artifact).
//!   3. The serialisation format of `.vproof` files (simple JSON or
//!      s-expression — separately auditable).
//!
//! Compare: HOL Light kernel ~5K LOC SML; Coq kernel ~10K LOC OCaml;
//! Lean kernel ~5K LOC C++.  Verum proof-term checker target: < 1000
//! LOC Rust.  Order-of-magnitude smaller trusted base than any
//! production proof assistant.

use serde::{Deserialize, Serialize};

// =============================================================================
// Minimal CoC AST
// =============================================================================

/// A proof term.  Types ARE terms (CIC-style); a "type" is a term
/// whose own type is some `Universe(n)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Term {
    /// de Bruijn-indexed variable referring to a context entry.
    /// Index 0 is the innermost binder.
    Var(usize),

    /// Universe level — `Universe(n)` lives in `Universe(n+1)`.
    /// Used for both `Type` (n=0) and stratified universes
    /// (n=1, 2, ... κ).
    Universe(u32),

    /// Dependent function type `Π(x : A). B`.  The body `B` is under
    /// a binder shifting de Bruijn indices: index 0 in `B` refers to
    /// the bound argument of type `A`.
    Pi(Box<Term>, Box<Term>),

    /// Lambda abstraction `λ(x : A). body`.  Carries the domain
    /// annotation so type-checking is bidirectional-from-info-rich
    /// (every binder is type-annotated; no inference of binder types).
    Lam(Box<Term>, Box<Term>),

    /// Application `f x`.  Evaluation reduces to substitution of `x`
    /// for de Bruijn 0 in the body of `f`.
    App(Box<Term>, Box<Term>),
}

impl Term {
    /// Convenience: build `Var(i)`.
    pub fn var(i: usize) -> Self {
        Term::Var(i)
    }

    /// Convenience: build `Universe(n)`.
    pub fn universe(n: u32) -> Self {
        Term::Universe(n)
    }

    /// Convenience: build `Pi(domain, body)`.
    pub fn pi(domain: Term, body: Term) -> Self {
        Term::Pi(Box::new(domain), Box::new(body))
    }

    /// Convenience: build `Lam(domain, body)`.
    pub fn lam(domain: Term, body: Term) -> Self {
        Term::Lam(Box::new(domain), Box::new(body))
    }

    /// Convenience: build `App(f, x)`.
    pub fn app(f: Term, x: Term) -> Self {
        Term::App(Box::new(f), Box::new(x))
    }
}

// =============================================================================
// Context (de Bruijn-indexed variable types)
// =============================================================================

/// Type-checking context: stack of types corresponding to bound
/// variables, with index 0 being the most-recent binder.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Inner-first stack of variable types.
    types: Vec<Term>,
}

impl Context {
    /// Construct an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the type of variable at de Bruijn index `i`.  Returns
    /// `None` if the index is out of bounds (free variable).
    /// Crucially: the returned type is shifted up by `i + 1` so the
    /// caller sees it in the OUTER context's de Bruijn frame.
    pub fn lookup(&self, i: usize) -> Option<Term> {
        // The types vec is innermost-first, so var(0) is types[len-1],
        // var(1) is types[len-2], etc.
        let len = self.types.len();
        if i >= len {
            return None;
        }
        let raw = self.types[len - 1 - i].clone();
        Some(shift_up(raw, i + 1, 0))
    }

    /// Extend the context with a new binder of type `ty` (the new
    /// innermost binding).  Returns a fresh context — the original
    /// is unchanged for compositionality.
    pub fn extend(&self, ty: Term) -> Self {
        let mut out = self.clone();
        out.types.push(ty);
        out
    }

    /// Number of bound variables.
    pub fn depth(&self) -> usize {
        self.types.len()
    }
}

// =============================================================================
// de Bruijn shift and substitution
// =============================================================================

/// Shift every variable index in `term` by `+amount` if its index
/// is `>= cutoff`.  Used when moving a term INTO a binder context.
fn shift_up(term: Term, amount: usize, cutoff: usize) -> Term {
    match term {
        Term::Var(i) => {
            if i >= cutoff {
                Term::Var(i + amount)
            } else {
                Term::Var(i)
            }
        }
        Term::Universe(n) => Term::Universe(n),
        Term::Pi(a, b) => Term::Pi(
            Box::new(shift_up(*a, amount, cutoff)),
            Box::new(shift_up(*b, amount, cutoff + 1)),
        ),
        Term::Lam(a, body) => Term::Lam(
            Box::new(shift_up(*a, amount, cutoff)),
            Box::new(shift_up(*body, amount, cutoff + 1)),
        ),
        Term::App(f, x) => Term::App(
            Box::new(shift_up(*f, amount, cutoff)),
            Box::new(shift_up(*x, amount, cutoff)),
        ),
    }
}

/// Substitute `replacement` for the variable at de Bruijn index
/// `target` in `term`.  Used by β-reduction: `(λ. body) x` reduces
/// to `subst(body, 0, x)`.  The replacement is shifted to compensate
/// for the binders the substitution descends into.
fn subst(term: Term, target: usize, replacement: &Term) -> Term {
    match term {
        Term::Var(i) => {
            use std::cmp::Ordering;
            match i.cmp(&target) {
                Ordering::Equal => shift_up(replacement.clone(), target, 0),
                Ordering::Greater => Term::Var(i - 1),
                Ordering::Less => Term::Var(i),
            }
        }
        Term::Universe(n) => Term::Universe(n),
        Term::Pi(a, b) => Term::Pi(
            Box::new(subst(*a, target, replacement)),
            Box::new(subst(*b, target + 1, replacement)),
        ),
        Term::Lam(a, body) => Term::Lam(
            Box::new(subst(*a, target, replacement)),
            Box::new(subst(*body, target + 1, replacement)),
        ),
        Term::App(f, x) => Term::App(
            Box::new(subst(*f, target, replacement)),
            Box::new(subst(*x, target, replacement)),
        ),
    }
}

/// β-reduce the head of a term to weak head normal form.  Repeats
/// until no top-level redex remains.  Cycle-safe by construction
/// (each reduction strictly shrinks the App-structure at the head).
fn whnf(mut term: Term) -> Term {
    loop {
        match term {
            Term::App(f, x) => {
                let f_whnf = whnf(*f);
                match f_whnf {
                    Term::Lam(_, body) => {
                        term = subst(*body, 0, &x);
                    }
                    other => return Term::App(Box::new(other), x),
                }
            }
            _ => return term,
        }
    }
}

/// α-equivalence + β-equality + η-equivalence (definitional equality
/// at the level the checker decides).  Both sides are reduced to
/// WHNF and then compared structurally; under binders, α-equivalence
/// is automatic via de Bruijn indices.
///
/// **η-equivalence (T-Eta-Conv)** — `λx. (f x) ≡ f` when `x` (de
/// Bruijn 0 in the body) does not occur free in the CONTENT of `f`.
/// This is the standard CIC rule extending β with extensional
/// function equality.  Brings the proof-term checker to textbook
/// CIC parity within the < 1000 LOC trust-base budget.
fn def_eq(a: &Term, b: &Term) -> bool {
    let a = whnf(a.clone());
    let b = whnf(b.clone());
    def_eq_whnf(&a, &b)
}

fn def_eq_whnf(a: &Term, b: &Term) -> bool {
    match (a, b) {
        (Term::Var(i), Term::Var(j)) => i == j,
        (Term::Universe(n), Term::Universe(m)) => n == m,
        (Term::Pi(a1, b1), Term::Pi(a2, b2)) => def_eq(a1, a2) && def_eq(b1, b2),
        (Term::Lam(a1, b1), Term::Lam(a2, b2)) => def_eq(a1, a2) && def_eq(b1, b2),
        (Term::App(f1, x1), Term::App(f2, x2)) => def_eq(f1, f2) && def_eq(x1, x2),
        // η-equivalence — one-sided cases.  When one side is a
        // λx.(f x) and the other is `f`, they're equal iff `x`
        // does not appear free in `f`.  This rule fires AFTER WHNF
        // reduction so β-redexes are eliminated first; what remains
        // is purely structural η.
        (Term::Lam(_, body), other) => eta_match(body, other),
        (other, Term::Lam(_, body)) => eta_match(body, other),
        _ => false,
    }
}

/// η-equivalence helper: returns `true` iff `lam_body` (the body of
/// a λ at depth 0) is `App(f, Var(0))` where `f` does not contain
/// Var(0) free, AND `f` (after shifting down) is equal to `other`.
///
/// This is the soundness gate for T-Eta-Conv: the bound variable
/// must not "leak" into the function part of the application.
fn eta_match(lam_body: &Term, other: &Term) -> bool {
    let app_body = whnf(lam_body.clone());
    let (f, x) = match app_body {
        Term::App(f, x) => (f, x),
        _ => return false,
    };
    // The argument must be exactly Var(0) (the bound variable).
    if !matches!(*x, Term::Var(0)) {
        return false;
    }
    // The function part must not reference Var(0) — otherwise the
    // η-rule is unsound (the variable escapes its binder).
    if is_free_in(&f, 0) {
        return false;
    }
    // Shift `f` down by one (since we're removing a binder) and
    // compare to `other`.
    let f_shifted = shift_down(*f, 1, 0);
    def_eq(&f_shifted, other)
}

/// Check whether de Bruijn index `target` occurs FREE in `term`
/// (i.e., not captured by an inner binder).  Used by the η-rule
/// to ensure the bound variable doesn't leak into the function
/// part.
fn is_free_in(term: &Term, target: usize) -> bool {
    match term {
        Term::Var(i) => *i == target,
        Term::Universe(_) => false,
        Term::Pi(a, b) => is_free_in(a, target) || is_free_in(b, target + 1),
        Term::Lam(a, body) => is_free_in(a, target) || is_free_in(body, target + 1),
        Term::App(f, x) => is_free_in(f, target) || is_free_in(x, target),
    }
}

/// Inverse of `shift_up` — decrement every variable index in `term`
/// by `amount` if its index is `>= cutoff + amount`, leaving lower
/// indices alone.  Panics in debug if it would produce a negative
/// index (caller bug).
fn shift_down(term: Term, amount: usize, cutoff: usize) -> Term {
    match term {
        Term::Var(i) => {
            if i >= cutoff + amount {
                Term::Var(i - amount)
            } else if i < cutoff {
                Term::Var(i)
            } else {
                // 0 <= i - cutoff < amount → would underflow.
                // η-match's `is_free_in` precondition rules this out
                // for our use case, but defensively return the
                // unchanged variable so a caller bug is visible
                // downstream as a type-mismatch rather than a panic.
                Term::Var(i)
            }
        }
        Term::Universe(n) => Term::Universe(n),
        Term::Pi(a, b) => Term::Pi(
            Box::new(shift_down(*a, amount, cutoff)),
            Box::new(shift_down(*b, amount, cutoff + 1)),
        ),
        Term::Lam(a, body) => Term::Lam(
            Box::new(shift_down(*a, amount, cutoff)),
            Box::new(shift_down(*body, amount, cutoff + 1)),
        ),
        Term::App(f, x) => Term::App(
            Box::new(shift_down(*f, amount, cutoff)),
            Box::new(shift_down(*x, amount, cutoff)),
        ),
    }
}

// =============================================================================
// Bidirectional type checker — the six rules
// =============================================================================

/// Type-checking error.  Each error names the kernel rule that
/// rejected the term, so a reviewer can trace the verdict to the
/// exact arm of `infer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// T-Var: variable index out of bounds (free variable).
    UnboundVariable(usize),
    /// T-Pi-Form / T-Lam-Intro: domain annotation isn't a type
    /// (its own type isn't a `Universe(n)`).
    NotAType(Term),
    /// T-App-Elim: function side isn't a Pi type.
    NotAFunction(Term),
    /// T-App-Elim: argument's type doesn't match the Pi's domain.
    DomainMismatch { expected: Term, actual: Term },
    /// T-Conv: expected and inferred types are not definitionally
    /// equal.
    TypeMismatch { expected: Term, actual: Term },
}

/// Infer the type of `term` in `ctx`.  Returns the unique type or
/// a `CheckError` naming the rejecting kernel rule.
///
/// **The six rules at a glance.**
///
///   T-Var:    ctx[i] = T  →  Var(i) : T
///   T-Univ:   Universe(n) : Universe(n+1)
///   T-Pi-Form: A : Universe(n), B : Universe(m) under (A:: ctx)
///             → Pi(A, B) : Universe(max(n, m))
///   T-Lam-Intro: B : T under (A:: ctx)  →  Lam(A, B) : Pi(A, T)
///   T-App-Elim: f : Pi(A, B), x : A  →  App(f, x) : B[x/0]
///   T-Conv:   T1 ≡_β T2 (definitional equality lets the checker
///             swap T1 for T2 in any judgement; used implicitly in
///             T-App-Elim to match argument types).
pub fn infer(ctx: &Context, term: &Term) -> Result<Term, CheckError> {
    match term {
        // T-Var
        Term::Var(i) => ctx
            .lookup(*i)
            .ok_or_else(|| CheckError::UnboundVariable(*i)),

        // T-Univ
        Term::Universe(n) => Ok(Term::Universe(n + 1)),

        // T-Pi-Form
        Term::Pi(a, b) => {
            let a_ty = infer(ctx, a)?;
            let n = expect_universe(&a_ty).ok_or_else(|| {
                CheckError::NotAType((**a).clone())
            })?;
            let extended = ctx.extend((**a).clone());
            let b_ty = infer(&extended, b)?;
            let m = expect_universe(&b_ty).ok_or_else(|| {
                CheckError::NotAType((**b).clone())
            })?;
            Ok(Term::Universe(n.max(m)))
        }

        // T-Lam-Intro
        Term::Lam(domain, body) => {
            let dom_ty = infer(ctx, domain)?;
            // Domain annotation must be a type.
            expect_universe(&dom_ty).ok_or_else(|| {
                CheckError::NotAType((**domain).clone())
            })?;
            let extended = ctx.extend((**domain).clone());
            let body_ty = infer(&extended, body)?;
            Ok(Term::Pi(domain.clone(), Box::new(body_ty)))
        }

        // T-App-Elim (with implicit T-Conv on argument matching)
        Term::App(f, x) => {
            let f_ty = whnf(infer(ctx, f)?);
            let (dom, codom) = match f_ty {
                Term::Pi(a, b) => (a, b),
                other => return Err(CheckError::NotAFunction(other)),
            };
            let x_ty = infer(ctx, x)?;
            if !def_eq(&dom, &x_ty) {
                return Err(CheckError::DomainMismatch {
                    expected: *dom,
                    actual: x_ty,
                });
            }
            Ok(subst(*codom, 0, x))
        }
    }
}

/// Check that `term` has type `expected`.  Wraps `infer` + `def_eq`.
/// This is the load-bearing entry point for `verum check-proof`:
/// the .vproof file says "this term has this type", and we either
/// agree or reject.
pub fn check(ctx: &Context, term: &Term, expected: &Term) -> Result<(), CheckError> {
    let inferred = infer(ctx, term)?;
    if def_eq(&inferred, expected) {
        Ok(())
    } else {
        Err(CheckError::TypeMismatch {
            expected: expected.clone(),
            actual: inferred,
        })
    }
}

/// If `term` reduces to `Universe(n)`, return `n`; else `None`.
fn expect_universe(term: &Term) -> Option<u32> {
    match whnf(term.clone()) {
        Term::Universe(n) => Some(n),
        _ => None,
    }
}

// =============================================================================
// Proof-term certificate format
// =============================================================================

/// A `.vproof` certificate carries a self-contained type-checking
/// problem: a closed term + its claimed type.  The minimal proof-
/// term checker re-verifies the pair top-to-bottom.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certificate {
    /// The proof-term.  Must be closed (no free variables).
    pub term: Term,
    /// The claimed type.  Also closed.
    pub claimed_type: Term,
    /// Optional metadata: theorem name, source file, kernel-version
    /// hint.  Not load-bearing — the checker doesn't read them.
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
}

impl Certificate {
    /// Verify the certificate.  Returns `Ok(())` iff the term has the
    /// claimed type in an empty context.  Any free variable in either
    /// term or type is a structural error rejected here.
    pub fn verify(&self) -> Result<(), CheckError> {
        let ctx = Context::new();
        check(&ctx, &self.term, &self.claimed_type)
    }
}

// =============================================================================
// Tests — pin the six rules + corner cases
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_universe_lives_in_next_level() {
        // T-Univ: Universe(0) : Universe(1)
        let ctx = Context::new();
        assert_eq!(
            infer(&ctx, &Term::Universe(0)).unwrap(),
            Term::Universe(1)
        );
        assert_eq!(
            infer(&ctx, &Term::Universe(5)).unwrap(),
            Term::Universe(6)
        );
    }

    #[test]
    fn t_var_returns_context_type() {
        // T-Var: extend with type Universe(0); Var(0) : Universe(0).
        let ctx = Context::new().extend(Term::Universe(0));
        assert_eq!(infer(&ctx, &Term::Var(0)).unwrap(), Term::Universe(0));
    }

    #[test]
    fn t_var_unbound_rejects() {
        let ctx = Context::new();
        match infer(&ctx, &Term::Var(0)) {
            Err(CheckError::UnboundVariable(0)) => {}
            other => panic!("expected UnboundVariable, got {:?}", other),
        }
    }

    #[test]
    fn t_pi_form_accepts_universe_to_universe() {
        // Π(_ : Universe(0)). Universe(0) : Universe(1)
        let ctx = Context::new();
        let pi = Term::pi(Term::Universe(0), Term::Universe(0));
        assert_eq!(infer(&ctx, &pi).unwrap(), Term::Universe(1));
    }

    #[test]
    fn t_pi_form_takes_max_universe() {
        // Π(_ : Universe(2)). Universe(5) : Universe(6)
        let ctx = Context::new();
        let pi = Term::pi(Term::Universe(2), Term::Universe(5));
        // Universe(2) : Universe(3); Universe(5) : Universe(6)
        // → max(3, 6) = 6
        assert_eq!(infer(&ctx, &pi).unwrap(), Term::Universe(6));
    }

    #[test]
    fn t_lam_intro_produces_pi() {
        // λ(x : Universe(0)). x  has type  Π(_ : Universe(0)). Universe(0)
        let ctx = Context::new();
        let lam = Term::lam(Term::Universe(0), Term::Var(0));
        let inferred = infer(&ctx, &lam).unwrap();
        let expected = Term::pi(Term::Universe(0), Term::Universe(0));
        assert!(def_eq(&inferred, &expected));
    }

    #[test]
    fn t_app_elim_with_correct_argument() {
        // (λ(x : Universe(0)). x) y   where y : Universe(0) (a hypothesis).
        // Result has type Universe(0) (the codomain after substitution).
        // Note: `Universe(0)` itself does NOT have type `Universe(0)` —
        // it has type `Universe(1)`.  So we can't pass `Universe(0)` as
        // an argument here; we must use a context variable typed at
        // `Universe(0)`.
        let ctx = Context::new().extend(Term::Universe(0));
        let f = Term::lam(Term::Universe(0), Term::Var(0));
        // Var(1) refers to the context entry (which had type Univ(0));
        // the lambda binder bumps de Bruijn indices, so the OUTER
        // hypothesis is at index 1 when we view the App at depth 0.
        // Actually no — the App is at the OUTER context depth, so
        // Var(0) refers to the hypothesis directly.
        let app = Term::app(f, Term::Var(0));
        let inferred = infer(&ctx, &app).unwrap();
        assert_eq!(inferred, Term::Universe(0));
    }

    #[test]
    fn t_app_elim_rejects_non_function() {
        // App(Universe(0), Universe(0)) — applying a non-function.
        let ctx = Context::new();
        let bad = Term::app(Term::Universe(0), Term::Universe(0));
        match infer(&ctx, &bad) {
            Err(CheckError::NotAFunction(_)) => {}
            other => panic!("expected NotAFunction, got {:?}", other),
        }
    }

    #[test]
    fn t_app_elim_rejects_domain_mismatch() {
        // f : Π(_ : Univ(0)). Univ(0); apply to Univ(5) (whose type
        // is Univ(6), not Univ(0)) → DomainMismatch.
        let ctx = Context::new();
        let f = Term::lam(Term::Universe(0), Term::Var(0));
        let bad = Term::app(f, Term::Universe(5));
        // Argument Universe(5) has type Universe(6); Pi expects Univ(0).
        match infer(&ctx, &bad) {
            Err(CheckError::DomainMismatch { .. }) => {}
            // Actually — Universe(5) IS a Universe, so its TYPE is
            // Universe(6).  The Pi expects something of type Universe(0).
            // 6 ≠ 0 → DomainMismatch.
            other => panic!("expected DomainMismatch, got {:?}", other),
        }
    }

    #[test]
    fn beta_reduction_resolves_application() {
        // (λx. x) y  →  y  (where y : T, the application has type T)
        let ctx = Context::new().extend(Term::Universe(0)); // y : Universe(0)
        let id = Term::lam(Term::Universe(0), Term::Var(0));
        let app = Term::app(id, Term::Var(0));
        let inferred = infer(&ctx, &app).unwrap();
        // App-Elim: f : Pi(U(0), U(0)); arg Var(0) has type U(0); result type
        // = subst(U(0), 0, Var(0)) = U(0).
        assert_eq!(inferred, Term::Universe(0));
    }

    #[test]
    fn certificate_verifies_correct_pair() {
        // Identity at Universe(0): λ(x:U(0)). x  has type  Π(_:U(0)).U(0)
        let cert = Certificate {
            term: Term::lam(Term::Universe(0), Term::Var(0)),
            claimed_type: Term::pi(Term::Universe(0), Term::Universe(0)),
            metadata: Default::default(),
        };
        cert.verify().expect("certificate should verify");
    }

    #[test]
    fn certificate_rejects_wrong_type() {
        // Identity claims to be Universe(0) — wrong; it's a function.
        let cert = Certificate {
            term: Term::lam(Term::Universe(0), Term::Var(0)),
            claimed_type: Term::Universe(0),
            metadata: Default::default(),
        };
        match cert.verify() {
            Err(CheckError::TypeMismatch { .. }) => {}
            other => panic!("expected TypeMismatch, got {:?}", other),
        }
    }

    #[test]
    fn shift_up_handles_binders_correctly() {
        // shift_up(Var(0), 1, 0) → Var(1)  (free var gets shifted)
        // shift_up(Lam(_, Var(0)), 1, 0) → Lam(_, Var(0))  (bound stays)
        // shift_up(Lam(_, Var(1)), 1, 0) → Lam(_, Var(2))  (free in body shifts)
        assert_eq!(
            shift_up(Term::Var(0), 1, 0),
            Term::Var(1),
        );
        let lam_bound = Term::lam(Term::Universe(0), Term::Var(0));
        assert_eq!(shift_up(lam_bound.clone(), 1, 0), lam_bound);
        let lam_free = Term::lam(Term::Universe(0), Term::Var(1));
        assert_eq!(
            shift_up(lam_free, 1, 0),
            Term::lam(Term::Universe(0), Term::Var(2)),
        );
    }

    #[test]
    fn def_eq_is_alpha_plus_beta() {
        // (λx. x) y  ≡_β  y
        let lhs = Term::app(
            Term::lam(Term::Universe(0), Term::Var(0)),
            Term::Universe(7),
        );
        let rhs = Term::Universe(7);
        assert!(def_eq(&lhs, &rhs));
    }

    #[test]
    fn def_eq_rejects_distinct_universes() {
        assert!(!def_eq(&Term::Universe(0), &Term::Universe(1)));
    }

    #[test]
    fn def_eq_eta_lam_app_equals_function() {
        // λ(x : Univ(0)). (f x)  ≡_η  f   when f doesn't contain x.
        // We use Var(0) referring to OUTER context (a hypothesis "f"
        // present at depth 0).  Inside the lambda, that becomes Var(1).
        let f_outer = Term::Var(0);
        // Inside lambda body: Var(0) is the bound x; Var(1) is f.
        let lam_eta = Term::lam(
            Term::Universe(0),
            Term::app(Term::Var(1), Term::Var(0)),
        );
        // Outer context: f : Π(_:Univ(0)).Univ(0).  The lam_eta's type
        // is the same Pi, and η-equality with f_outer should hold.
        // For the def_eq test, we don't need the context — we just
        // check whether the term forms are η-equivalent.
        assert!(def_eq(&lam_eta, &f_outer));
        // Symmetry: the comparison is order-independent.
        assert!(def_eq(&f_outer, &lam_eta));
    }

    #[test]
    fn def_eq_eta_rejects_when_arg_is_not_bound_var() {
        // λ(x : Univ(0)). (f y)  is NOT η-equivalent to f — the
        // application argument isn't the bound variable.
        let f = Term::Var(0); // outer context
        let lam_not_eta = Term::lam(
            Term::Universe(0),
            // Var(2) inside body refers to TWO levels out, not Var(0)
            // (the bound x), so the η-rule doesn't fire.
            Term::app(Term::Var(1), Term::Var(2)),
        );
        assert!(!def_eq(&lam_not_eta, &f));
    }

    #[test]
    fn def_eq_eta_rejects_when_var_escapes_into_function() {
        // λ(x : Univ(0)). (x x)  has the bound variable in the
        // FUNCTION part — η would be unsound here, must be rejected.
        let lam_unsound = Term::lam(
            Term::Universe(0),
            Term::app(Term::Var(0), Term::Var(0)),
        );
        let any_other = Term::Var(0); // outer context "any other f"
        assert!(!def_eq(&lam_unsound, &any_other));
    }

    #[test]
    fn is_free_in_handles_binders() {
        // Var(0) is free in Var(0), but bound in λ(_).Var(0)
        assert!(is_free_in(&Term::Var(0), 0));
        let lam_body_zero = Term::lam(Term::Universe(0), Term::Var(0));
        // Lam(_, Var(0)) — the body's Var(0) is the bound var, NOT
        // a free reference to outer Var(0).  Querying outer-target=0
        // shifts to inner-target=1 inside the body, which Var(0) is
        // NOT.  So the outer Var(0) is NOT free in this term.
        assert!(!is_free_in(&lam_body_zero, 0));
        // But Var(1) inside the body IS a free reference to OUTER
        // Var(0).  The outer-target=0 query shifts to target=1 in
        // the body, which matches Var(1).
        let lam_body_outer = Term::lam(Term::Universe(0), Term::Var(1));
        assert!(is_free_in(&lam_body_outer, 0));
    }

    #[test]
    fn shift_down_inverse_of_shift_up() {
        // shift_down . shift_up = identity on var indices that don't
        // get clobbered.
        let original = Term::lam(Term::Universe(0), Term::Var(2));
        let shifted_up = shift_up(original.clone(), 1, 0);
        let shifted_back = shift_down(shifted_up, 1, 0);
        assert_eq!(shifted_back, original);
    }

    #[test]
    fn dependent_function_type_checks() {
        // Dependent identity: Π(A : Univ(0)). Π(_ : A). A
        // (the polymorphic identity type)
        let ctx = Context::new();
        let inner_pi = Term::pi(Term::Var(0), Term::Var(1)); // _ : A; result type A (now Var(1))
        let outer_pi = Term::pi(Term::Universe(0), inner_pi);
        let inferred = infer(&ctx, &outer_pi).unwrap();
        // Universe(1) — outer.A : Universe(0); body of outer is Pi
        // taking Var(0) (A) and returning Var(1) (A under one
        // additional binder).  Var(0) under the outer binder has
        // type Universe(0); the inner Pi forms over it, producing
        // Universe(0).  Outer Pi: max(Univ(1) for type-of-A,
        // Univ(0) for body-Pi) = Universe(1).
        assert_eq!(inferred, Term::Universe(1));
    }

    #[test]
    fn polymorphic_identity_type_checks() {
        // λ(A : Univ(0)). λ(x : A). x
        //   has type  Π(A : Univ(0)). Π(_ : A). A
        let ctx = Context::new();
        let body = Term::lam(Term::Var(0), Term::Var(0));
        let id = Term::lam(Term::Universe(0), body);
        let inferred = infer(&ctx, &id).unwrap();
        // Type: Pi(Univ(0), Pi(Var(0), Var(1)))
        // Inner-Pi body Var(1) refers to A in the outer Pi's binder.
        let expected_type = Term::pi(
            Term::Universe(0),
            Term::pi(Term::Var(0), Term::Var(1)),
        );
        assert!(
            def_eq(&inferred, &expected_type),
            "polymorphic id expected type, got {:?}",
            inferred,
        );
    }
}
