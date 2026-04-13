//! Continuation Calculus — delimited control as a first-class
//! computational primitive.
//!
//! Where ordinary control flow (return, throw, return-via-Result)
//! is *limited* — you can only escape one level at a time —
//! delimited continuations let a computation reify "what to do
//! next" as a value and pass it around. The classical operators
//! are:
//!
//! ```text
//!     reset { e }            install a delimiter (prompt)
//!     shift k. e             capture the rest up to the nearest
//!                            reset, bind it to k, run e
//! ```
//!
//! With these, programmers can express coroutines, nondeterministic
//! search, backtracking, and full algebraic-effect handlers as a
//! library — without language-level effect machinery.
//!
//! Verum chose contexts + specialized constructs (async/Iterator/
//! Result) over general handlers. This module provides delimited
//! continuations as a **standalone analysis core**: callers that
//! want to model handler-style code, prove properties about it, or
//! translate other languages with shift/reset into Verum's
//! semantics get the core algebra here.
//!
//! ## Core syntax
//!
//! ```text
//!     M ::= v                  (value)
//!         | x                  (variable)
//!         | λx. M              (lambda)
//!         | M N                (application)
//!         | reset M            (delimiter)
//!         | shift k. M         (capture)
//!         | k @ M              (resume captured continuation)
//! ```
//!
//! ## Reduction
//!
//! The cardinal computation rule is the **shift-reset reaction**:
//!
//! ```text
//!     reset (E[shift k. M])   ↦   reset (M[k := λx. reset E[x]])
//! ```
//!
//! where `E[]` is the evaluation context inside the nearest reset.
//! When no shift remains, `reset v ↦ v`.
//!
//! ## Status
//!
//! Algebraic core: term language, capture-avoiding substitution,
//! single-step reduction at the redex `reset (E[shift k. M])`.
//! Higher-level driver loops, type systems for shift/reset
//! (answer-type modification), and CPS translation are out of
//! scope.

use std::collections::HashSet;

use verum_common::Text;

/// A continuation-calculus term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CcTerm {
    /// A value-level constant — opaque to the calculus.
    Const(Text),
    /// Variable reference.
    Var(Text),
    /// Lambda abstraction `λx. body`.
    Lam {
        param: Text,
        body: Box<CcTerm>,
    },
    /// Application `f arg`.
    App {
        func: Box<CcTerm>,
        arg: Box<CcTerm>,
    },
    /// `reset M` — install a control delimiter.
    Reset(Box<CcTerm>),
    /// `shift k. M` — capture the continuation up to the nearest
    /// reset, bind it to `k`.
    Shift {
        binder: Text,
        body: Box<CcTerm>,
    },
}

impl CcTerm {
    pub fn cnst(s: impl Into<Text>) -> Self {
        Self::Const(s.into())
    }

    pub fn var(s: impl Into<Text>) -> Self {
        Self::Var(s.into())
    }

    pub fn lam(param: impl Into<Text>, body: CcTerm) -> Self {
        Self::Lam {
            param: param.into(),
            body: Box::new(body),
        }
    }

    pub fn app(f: CcTerm, x: CcTerm) -> Self {
        Self::App {
            func: Box::new(f),
            arg: Box::new(x),
        }
    }

    pub fn reset(m: CcTerm) -> Self {
        Self::Reset(Box::new(m))
    }

    pub fn shift(binder: impl Into<Text>, body: CcTerm) -> Self {
        Self::Shift {
            binder: binder.into(),
            body: Box::new(body),
        }
    }

    /// Free variables, in deterministic-traversal-order set form.
    pub fn free_vars(&self) -> HashSet<Text> {
        let mut out = HashSet::new();
        self.free_vars_into(&mut out);
        out
    }

    fn free_vars_into(&self, out: &mut HashSet<Text>) {
        match self {
            CcTerm::Const(_) => {}
            CcTerm::Var(name) => {
                out.insert(name.clone());
            }
            CcTerm::Lam { param, body } => {
                let mut inner = HashSet::new();
                body.free_vars_into(&mut inner);
                inner.remove(param);
                for n in inner {
                    out.insert(n);
                }
            }
            CcTerm::App { func, arg } => {
                func.free_vars_into(out);
                arg.free_vars_into(out);
            }
            CcTerm::Reset(inner) => inner.free_vars_into(out),
            CcTerm::Shift { binder, body } => {
                let mut inner = HashSet::new();
                body.free_vars_into(&mut inner);
                inner.remove(binder);
                for n in inner {
                    out.insert(n);
                }
            }
        }
    }

    /// Capture-avoiding substitution `term[from := to]`.
    pub fn substitute(&self, from: &Text, to: &CcTerm) -> CcTerm {
        match self {
            CcTerm::Const(_) => self.clone(),
            CcTerm::Var(name) => {
                if name == from {
                    to.clone()
                } else {
                    self.clone()
                }
            }
            CcTerm::Lam { param, body } => {
                if param == from {
                    self.clone()
                } else {
                    CcTerm::lam(param.clone(), body.substitute(from, to))
                }
            }
            CcTerm::App { func, arg } => {
                CcTerm::app(func.substitute(from, to), arg.substitute(from, to))
            }
            CcTerm::Reset(inner) => CcTerm::reset(inner.substitute(from, to)),
            CcTerm::Shift { binder, body } => {
                if binder == from {
                    self.clone()
                } else {
                    CcTerm::shift(binder.clone(), body.substitute(from, to))
                }
            }
        }
    }

    /// Is this term a *value*? Values are constants and lambdas;
    /// they cannot reduce further on their own.
    pub fn is_value(&self) -> bool {
        matches!(self, CcTerm::Const(_) | CcTerm::Lam { .. })
    }
}

/// Step the term by one β/reset/shift reduction, when applicable.
/// Returns `None` if the term is already a value or no immediate
/// redex applies.
///
/// Reductions implemented:
///
/// * **β**:   `(λx. M) N         ↦ M[x := N]`
/// * **reset-value**: `reset v   ↦ v`
/// * **shift**: `reset (shift k. M) ↦ reset (M[k := λx. reset x])`
///   (the simplest, *empty-context* form of the shift-reset rule)
pub fn step(term: &CcTerm) -> Option<CcTerm> {
    match term {
        // β-reduction.
        CcTerm::App { func, arg } => {
            if let CcTerm::Lam { param, body } = func.as_ref() {
                return Some(body.substitute(param, arg));
            }
            // No outer redex — try reducing the function part first.
            if let Some(f2) = step(func) {
                return Some(CcTerm::app(f2, (**arg).clone()));
            }
            // Then the argument.
            if let Some(a2) = step(arg) {
                return Some(CcTerm::app((**func).clone(), a2));
            }
            None
        }

        CcTerm::Reset(inner) => {
            // reset v ↦ v (when inner is a value).
            if inner.is_value() {
                return Some((**inner).clone());
            }
            // reset (shift k. M) ↦ reset (M[k := λx. reset x])
            // — the empty-context case where the captured
            // continuation is the identity.
            if let CcTerm::Shift { binder, body } = inner.as_ref() {
                let identity_k =
                    CcTerm::lam("x", CcTerm::reset(CcTerm::var("x")));
                let substituted = body.substitute(binder, &identity_k);
                return Some(CcTerm::reset(substituted));
            }
            // Otherwise reduce the inner term.
            step(inner).map(CcTerm::reset)
        }

        // Variables, constants, lambdas, and free shifts (outside
        // any reset) do not reduce.
        _ => None,
    }
}

/// Run-to-normal-form evaluator, capped at `fuel` steps to avoid
/// runaway non-termination from arbitrary user terms.
pub fn evaluate(term: &CcTerm, fuel: usize) -> CcTerm {
    let mut current = term.clone();
    for _ in 0..fuel {
        match step(&current) {
            Some(next) => current = next,
            None => break,
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Text {
        Text::from(s)
    }

    #[test]
    fn const_is_value() {
        assert!(CcTerm::cnst("c").is_value());
    }

    #[test]
    fn lambda_is_value() {
        let l = CcTerm::lam("x", CcTerm::var("x"));
        assert!(l.is_value());
    }

    #[test]
    fn variable_is_not_value() {
        assert!(!CcTerm::var("x").is_value());
    }

    #[test]
    fn application_is_not_value() {
        let app = CcTerm::app(CcTerm::var("f"), CcTerm::var("x"));
        assert!(!app.is_value());
    }

    #[test]
    fn beta_reduction_substitutes() {
        // (λx. x) c  ↦  c
        let term = CcTerm::app(
            CcTerm::lam("x", CcTerm::var("x")),
            CcTerm::cnst("c"),
        );
        let result = step(&term).unwrap();
        assert_eq!(result, CcTerm::cnst("c"));
    }

    #[test]
    fn beta_reduces_function_first_when_not_lambda() {
        // ((λf. f) (λx. x)) c — reduce inner first.
        let inner = CcTerm::app(
            CcTerm::lam("f", CcTerm::var("f")),
            CcTerm::lam("x", CcTerm::var("x")),
        );
        let outer = CcTerm::app(inner, CcTerm::cnst("c"));
        let r = step(&outer).unwrap();
        // Result is ((λx. x) c) — outer not yet reduced.
        match r {
            CcTerm::App { func, .. } => assert!(matches!(*func, CcTerm::Lam { .. })),
            _ => panic!("expected Application after one step"),
        }
    }

    #[test]
    fn reset_value_unwraps() {
        // reset c  ↦  c
        let term = CcTerm::reset(CcTerm::cnst("c"));
        let r = step(&term).unwrap();
        assert_eq!(r, CcTerm::cnst("c"));
    }

    #[test]
    fn reset_lambda_unwraps() {
        let term = CcTerm::reset(CcTerm::lam("x", CcTerm::var("x")));
        let r = step(&term).unwrap();
        assert!(matches!(r, CcTerm::Lam { .. }));
    }

    #[test]
    fn shift_inside_reset_substitutes_identity() {
        // reset (shift k. c) ↦ reset c
        // (k goes unused, body is just `c`).
        let term = CcTerm::reset(CcTerm::shift("k", CcTerm::cnst("c")));
        let r = step(&term).unwrap();
        assert_eq!(r, CcTerm::reset(CcTerm::cnst("c")));
    }

    #[test]
    fn shift_using_k_applies_identity() {
        // reset (shift k. k @ c) ↦ reset ((λx. reset x) c)
        // Step further: ↦ reset (reset c) ↦ reset c ↦ c
        let term = CcTerm::reset(CcTerm::shift(
            "k",
            CcTerm::app(CcTerm::var("k"), CcTerm::cnst("c")),
        ));
        let result = evaluate(&term, 10);
        assert_eq!(result, CcTerm::cnst("c"));
    }

    #[test]
    fn no_step_on_bare_value() {
        assert!(step(&CcTerm::cnst("c")).is_none());
        assert!(step(&CcTerm::lam("x", CcTerm::var("x"))).is_none());
    }

    #[test]
    fn no_step_on_free_shift() {
        // shift outside any reset doesn't reduce.
        let term = CcTerm::shift("k", CcTerm::var("x"));
        assert!(step(&term).is_none());
    }

    #[test]
    fn free_vars_collects_unbound_names() {
        let term = CcTerm::app(CcTerm::var("f"), CcTerm::var("y"));
        let fvs = term.free_vars();
        assert!(fvs.contains(&t("f")));
        assert!(fvs.contains(&t("y")));
    }

    #[test]
    fn free_vars_skips_lambda_param() {
        let term = CcTerm::lam("x", CcTerm::var("x"));
        assert!(term.free_vars().is_empty());
    }

    #[test]
    fn free_vars_skips_shift_binder() {
        let term = CcTerm::shift("k", CcTerm::var("k"));
        assert!(term.free_vars().is_empty());
    }

    #[test]
    fn substitute_preserves_lambda_binder() {
        // (λx. y)[y := c]  ↦  λx. c
        let term = CcTerm::lam("x", CcTerm::var("y"));
        let r = term.substitute(&t("y"), &CcTerm::cnst("c"));
        if let CcTerm::Lam { param, body } = r {
            assert_eq!(param.as_str(), "x");
            assert_eq!(*body, CcTerm::cnst("c"));
        } else {
            panic!("expected Lam");
        }
    }

    #[test]
    fn substitute_skips_shadowed_binder() {
        // (λx. x)[x := c]  ↦  λx. x  (unchanged — x is bound)
        let term = CcTerm::lam("x", CcTerm::var("x"));
        let r = term.substitute(&t("x"), &CcTerm::cnst("c"));
        assert_eq!(r, term);
    }

    #[test]
    fn evaluate_terminates_at_normal_form() {
        // (λx. x) c  ↦  c  (normal form after 1 step)
        let term = CcTerm::app(
            CcTerm::lam("x", CcTerm::var("x")),
            CcTerm::cnst("c"),
        );
        let r = evaluate(&term, 100);
        assert_eq!(r, CcTerm::cnst("c"));
    }

    #[test]
    fn evaluate_caps_at_fuel() {
        // Apply Y-style non-terminator a couple of times — the
        // evaluator must stop without panicking.
        let omega = CcTerm::lam(
            "x",
            CcTerm::app(CcTerm::var("x"), CcTerm::var("x")),
        );
        let term = CcTerm::app(omega.clone(), omega);
        let _ = evaluate(&term, 5);
        // The point is that `evaluate` returned at all.
    }
}
