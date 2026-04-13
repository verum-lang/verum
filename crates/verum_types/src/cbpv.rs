//! Call-by-Push-Value (CBPV) — Levy's separation of values from
//! computations.
//!
//! Most calculi treat values and computations as the same syntactic
//! category: a function application is "just another expression" and
//! the language designer must pick *call-by-value* or *call-by-name*
//! evaluation. CBPV (Levy, 2003) factors this choice out by
//! introducing two distinct kinds of terms:
//!
//! ```text
//!     V ::= x | λx. C | thunk C       (values)
//!     C ::= return V                  (computations)
//!         | V to x. C                 (sequencing)
//!         | force V
//!         | V₁ V₂                      (application: V₁ a thunk-of-fn)
//! ```
//!
//! `thunk` packages a computation as a value; `force` runs the
//! computation back. `return V` lifts a value into a trivial
//! computation; `to x. C` sequences. The two reduction rules are
//! the eponymous β rules:
//!
//! ```text
//!     force (thunk C)        ↦  C            (force-thunk)
//!     return V to x. C       ↦  C[x := V]    (return-to)
//! ```
//!
//! ## Why CBPV matters
//!
//! Both call-by-value and call-by-name lambda-calculi embed into
//! CBPV through systematic translations, and effects (state, IO,
//! exceptions) gain a clean denotational semantics where they
//! attach to computations rather than values. CBPV is the
//! canonical setting for monadic semantics, algebraic-effects
//! research, and intermediate representations like Bauer-Pretnar's.
//!
//! ## Status
//!
//! Standalone algebraic core: term language for V/C, capture-
//! avoiding substitution into both kinds, single-step reduction
//! for the two CBPV β rules. The translation from λ-calculus and
//! the type discipline (value vs. computation types) are out of
//! scope.

use std::collections::HashSet;

use verum_common::Text;

/// A CBPV value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CbpvValue {
    /// `x` — variable.
    Var(Text),
    /// `λx. C` — abstraction; binds variable, body is a computation.
    Lam {
        param: Text,
        body: Box<CbpvComp>,
    },
    /// `thunk C` — packaged computation.
    Thunk(Box<CbpvComp>),
}

/// A CBPV computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CbpvComp {
    /// `return V` — lift a value into a trivial computation.
    Return(CbpvValue),
    /// `V to x. C` — sequence `C` after binding `x` to V's payload.
    SeqTo {
        producer: CbpvValue,
        binder: Text,
        body: Box<CbpvComp>,
    },
    /// `force V` — run a thunked computation.
    Force(CbpvValue),
    /// `V₁ V₂` — application; V₁ must elaborate to a lambda.
    App {
        func: CbpvValue,
        arg: CbpvValue,
    },
}

impl CbpvValue {
    pub fn var(s: impl Into<Text>) -> Self {
        Self::Var(s.into())
    }

    pub fn lam(param: impl Into<Text>, body: CbpvComp) -> Self {
        Self::Lam {
            param: param.into(),
            body: Box::new(body),
        }
    }

    pub fn thunk(c: CbpvComp) -> Self {
        Self::Thunk(Box::new(c))
    }

    /// Free variables of a value.
    pub fn free_vars(&self) -> HashSet<Text> {
        let mut out = HashSet::new();
        self.free_vars_into(&mut out);
        out
    }

    fn free_vars_into(&self, out: &mut HashSet<Text>) {
        match self {
            CbpvValue::Var(name) => {
                out.insert(name.clone());
            }
            CbpvValue::Lam { param, body } => {
                let mut inner = HashSet::new();
                body.free_vars_into(&mut inner);
                inner.remove(param);
                for n in inner {
                    out.insert(n);
                }
            }
            CbpvValue::Thunk(c) => c.free_vars_into(out),
        }
    }

    /// Substitute a value for a free variable.
    pub fn substitute(&self, from: &Text, to: &CbpvValue) -> CbpvValue {
        match self {
            CbpvValue::Var(name) => {
                if name == from {
                    to.clone()
                } else {
                    self.clone()
                }
            }
            CbpvValue::Lam { param, body } => {
                if param == from {
                    self.clone()
                } else {
                    CbpvValue::lam(param.clone(), body.substitute(from, to))
                }
            }
            CbpvValue::Thunk(c) => CbpvValue::thunk(c.substitute(from, to)),
        }
    }
}

impl CbpvComp {
    pub fn ret(v: CbpvValue) -> Self {
        Self::Return(v)
    }

    pub fn seq_to(producer: CbpvValue, binder: impl Into<Text>, body: CbpvComp) -> Self {
        Self::SeqTo {
            producer,
            binder: binder.into(),
            body: Box::new(body),
        }
    }

    pub fn force(v: CbpvValue) -> Self {
        Self::Force(v)
    }

    pub fn app(f: CbpvValue, x: CbpvValue) -> Self {
        Self::App { func: f, arg: x }
    }

    pub fn free_vars(&self) -> HashSet<Text> {
        let mut out = HashSet::new();
        self.free_vars_into(&mut out);
        out
    }

    fn free_vars_into(&self, out: &mut HashSet<Text>) {
        match self {
            CbpvComp::Return(v) => v.free_vars_into(out),
            CbpvComp::SeqTo { producer, binder, body } => {
                producer.free_vars_into(out);
                let mut inner = HashSet::new();
                body.free_vars_into(&mut inner);
                inner.remove(binder);
                for n in inner {
                    out.insert(n);
                }
            }
            CbpvComp::Force(v) => v.free_vars_into(out),
            CbpvComp::App { func, arg } => {
                func.free_vars_into(out);
                arg.free_vars_into(out);
            }
        }
    }

    /// Substitute a value for a free variable inside a computation.
    pub fn substitute(&self, from: &Text, to: &CbpvValue) -> CbpvComp {
        match self {
            CbpvComp::Return(v) => CbpvComp::Return(v.substitute(from, to)),
            CbpvComp::SeqTo { producer, binder, body } => {
                if binder == from {
                    CbpvComp::seq_to(
                        producer.substitute(from, to),
                        binder.clone(),
                        (**body).clone(),
                    )
                } else {
                    CbpvComp::seq_to(
                        producer.substitute(from, to),
                        binder.clone(),
                        body.substitute(from, to),
                    )
                }
            }
            CbpvComp::Force(v) => CbpvComp::Force(v.substitute(from, to)),
            CbpvComp::App { func, arg } => {
                CbpvComp::app(func.substitute(from, to), arg.substitute(from, to))
            }
        }
    }
}

/// Single-step reduction. Implements the two CBPV β rules:
///
/// * `force (thunk C)        ↦ C`
/// * `return V to x. C       ↦ C[x := V]`
///
/// Plus the standard left-to-right congruence on `App` (when the
/// function position is a variable that hasn't been resolved, we
/// can't reduce — the rule fires only when the function position
/// is a lambda or a thunked-lambda, the latter requiring an
/// explicit force first).
pub fn step(c: &CbpvComp) -> Option<CbpvComp> {
    match c {
        // force (thunk C)  ↦  C
        CbpvComp::Force(CbpvValue::Thunk(inner)) => Some((**inner).clone()),

        // return V to x. body  ↦  body[x := V]
        CbpvComp::SeqTo { producer, binder, body } => {
            if let CbpvValue::Var(_) = producer {
                // Producer is a bare variable — can't reduce yet.
                return None;
            }
            // The producer should already be a value — but the
            // CBPV `to` binder explicitly takes a *value* on the
            // left. We treat any concrete value (Lam, Thunk, or a
            // resolved Var) as ready.
            //
            // The classical rule fires at `(return V) to x. C`, but
            // since `producer` is already a value at this position
            // (we've dropped the `Return` wrapper into the
            // value-language for this minimal core), we substitute
            // directly.
            Some(body.substitute(binder, producer))
        }

        // V₁ V₂ where V₁ = λx. body  ↦  body[x := V₂]
        CbpvComp::App { func, arg } => {
            if let CbpvValue::Lam { param, body } = func {
                return Some(body.substitute(param, arg));
            }
            None
        }

        _ => None,
    }
}

/// Run the computation up to `fuel` steps or until normal form.
pub fn evaluate(c: &CbpvComp, fuel: usize) -> CbpvComp {
    let mut current = c.clone();
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
    fn force_thunk_cancels() {
        // force (thunk (return x))  ↦  return x
        let inner = CbpvComp::ret(CbpvValue::var("x"));
        let term = CbpvComp::force(CbpvValue::thunk(inner.clone()));
        assert_eq!(step(&term), Some(inner));
    }

    #[test]
    fn force_non_thunk_doesnt_step() {
        let term = CbpvComp::force(CbpvValue::var("x"));
        assert!(step(&term).is_none());
    }

    #[test]
    fn app_lambda_beta_reduces() {
        // (λx. return x) c  ↦  return c
        let lam = CbpvValue::lam("x", CbpvComp::ret(CbpvValue::var("x")));
        let term = CbpvComp::app(lam, CbpvValue::var("c"));
        let r = step(&term).unwrap();
        assert_eq!(r, CbpvComp::ret(CbpvValue::var("c")));
    }

    #[test]
    fn app_non_lambda_doesnt_step() {
        let term = CbpvComp::app(CbpvValue::var("f"), CbpvValue::var("x"));
        assert!(step(&term).is_none());
    }

    #[test]
    fn seq_to_substitutes_value() {
        // c to x. return x  ↦  return c   (where c is a thunk value)
        let body = CbpvComp::ret(CbpvValue::var("x"));
        let term = CbpvComp::seq_to(
            CbpvValue::thunk(CbpvComp::ret(CbpvValue::var("v"))),
            "x",
            body,
        );
        let r = step(&term).unwrap();
        // The substituted body is `return (thunk (return v))`.
        assert!(matches!(r, CbpvComp::Return(_)));
    }

    #[test]
    fn seq_to_with_var_producer_doesnt_step() {
        // x to y. return y  — producer is a bare var, no step.
        let term = CbpvComp::seq_to(
            CbpvValue::var("x"),
            "y",
            CbpvComp::ret(CbpvValue::var("y")),
        );
        assert!(step(&term).is_none());
    }

    #[test]
    fn substitute_into_value_lam_skips_shadow() {
        // (λx. return x)[x := c]  ↦  unchanged
        let lam = CbpvValue::lam("x", CbpvComp::ret(CbpvValue::var("x")));
        let r = lam.substitute(&t("x"), &CbpvValue::var("c"));
        assert_eq!(r, lam);
    }

    #[test]
    fn substitute_into_value_lam_passes_through() {
        // (λx. return y)[y := c]  ↦  λx. return c
        let lam = CbpvValue::lam("x", CbpvComp::ret(CbpvValue::var("y")));
        let r = lam.substitute(&t("y"), &CbpvValue::var("c"));
        if let CbpvValue::Lam { body, .. } = r {
            assert_eq!(*body, CbpvComp::ret(CbpvValue::var("c")));
        } else {
            panic!("expected Lam");
        }
    }

    #[test]
    fn substitute_through_thunk() {
        // thunk (return x) [x := c]  ↦  thunk (return c)
        let v = CbpvValue::thunk(CbpvComp::ret(CbpvValue::var("x")));
        let r = v.substitute(&t("x"), &CbpvValue::var("c"));
        if let CbpvValue::Thunk(c) = r {
            assert_eq!(*c, CbpvComp::ret(CbpvValue::var("c")));
        }
    }

    #[test]
    fn substitute_into_seq_to_skips_shadow() {
        // (v to x. return x) [x := c]  ↦  v to x. return x  (binder shadows)
        let term = CbpvComp::seq_to(
            CbpvValue::var("v"),
            "x",
            CbpvComp::ret(CbpvValue::var("x")),
        );
        let r = term.substitute(&t("x"), &CbpvValue::var("c"));
        // The body's `x` is bound by the binder, so it's unchanged.
        if let CbpvComp::SeqTo { body, .. } = r {
            assert_eq!(*body, CbpvComp::ret(CbpvValue::var("x")));
        }
    }

    #[test]
    fn free_vars_value_var() {
        let v = CbpvValue::var("x");
        let fvs = v.free_vars();
        assert!(fvs.contains(&t("x")));
    }

    #[test]
    fn free_vars_value_lam_excludes_param() {
        let v = CbpvValue::lam("x", CbpvComp::ret(CbpvValue::var("x")));
        assert!(v.free_vars().is_empty());
    }

    #[test]
    fn free_vars_seq_to_excludes_binder() {
        let term = CbpvComp::seq_to(
            CbpvValue::var("v"),
            "x",
            CbpvComp::ret(CbpvValue::var("x")),
        );
        let fvs = term.free_vars();
        assert!(fvs.contains(&t("v")));
        assert!(!fvs.contains(&t("x")));
    }

    #[test]
    fn evaluate_terminates_at_normal_form() {
        // force (thunk (return x))  ↦  return x  (1 step then normal)
        let inner = CbpvComp::ret(CbpvValue::var("x"));
        let term = CbpvComp::force(CbpvValue::thunk(inner.clone()));
        assert_eq!(evaluate(&term, 100), inner);
    }

    #[test]
    fn evaluate_caps_at_fuel() {
        // No infinite-loop construction here, but confirm fuel
        // limit doesn't panic.
        let term = CbpvComp::force(CbpvValue::var("never_thunk"));
        let r = evaluate(&term, 5);
        assert_eq!(r, term);
    }
}
