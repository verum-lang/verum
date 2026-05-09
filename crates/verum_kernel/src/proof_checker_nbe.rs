//! Normalisation-by-Evaluation (NbE) proof-term checker — second
//! independent algorithmic kernel for #159 differential testing.
//!
//! ## Why a SECOND kernel
//!
//! The existing trusted base [`crate::proof_checker`] does
//! bidirectional type-checking with explicit substitution +
//! weak-head normalisation + structural definitional-equality.
//! Pre-this-module the differential-kernel audit gate (#159 V0)
//! had nothing to differential against — both slots ran the same
//! algorithm.
//!
//! This module ships a STRUCTURALLY DISTINCT algorithm — NbE — that
//! computes the same input/output relation via a different evaluation
//! strategy. Disagreements between the two implementations are bugs
//! in EITHER, surfacing through the audit gate's `Disagreement`
//! verdict.
//!
//! ## NbE in 5 lines
//!
//! Evaluation: `Term × Env → Value`. The semantic domain `Value`
//! has VLam (closure), VPi (closure + domain value), VApp (neutral
//! application), VVar (Levels — level-indexed free vars in
//! readback), VUniverse(n). Closures capture an environment + a
//! term to evaluate when applied.
//!
//! Type-check: bidirectional, but uses NbE internally for
//! normalisation. To check def-eq, evaluate both sides to Values
//! then compare structurally (with η-equivalence for Lam vs neutral).
//!
//! ## Soundness vs the existing kernel
//!
//! The two kernels MUST agree on every closed certificate. Pin
//! tests in this module's tests submodule + the differential audit
//! gate enforce the agreement at CI time. Any disagreement is a
//! bug to be tracked and fixed, never silently accepted.
//!
//! ## Architectural pattern
//!
//! Mirrors the established #159 pattern:
//!   * `proof_checker::infer / check` — Algorithm A (bidirectional +
//!     explicit substitution).
//!   * `proof_checker_nbe::infer / check` — Algorithm B (NbE).
//!   * `differential::run_differential_test_with_verum` — runs both
//!     on a Certificate; agreement classifier reports
//!     `BothAccept` / `BothReject` / `Disagreement` /
//!     `NotYetSelfHosting` (the last unused once this lands).

use crate::proof_checker::{CheckError, Level, Term};

// =============================================================================
// Semantic domain — the load-bearing distinguishing feature of NbE
// =============================================================================

/// Closure capturing an environment + a term body. Used for both
/// `VLam` and `VPi`'s codomain part. Application of a closure
/// extends the captured environment with the argument and evaluates
/// the body.
///
/// **Invariant**: closures are first-class semantic values; readback
/// to a syntactic Lam produces a fresh-de-Bruijn body via [`quote`].
#[derive(Debug, Clone)]
pub struct Closure {
    /// Environment captured at closure-construction time. Outer-first
    /// (level 0 is the outermost binder).
    pub env: Env,
    /// Body to evaluate when the closure is applied.
    pub body: Term,
}

impl Closure {
    /// Apply this closure to a value: extend its environment with
    /// the argument and evaluate the body.
    pub fn apply(&self, arg: Value) -> Value {
        let mut new_env = self.env.clone();
        new_env.push(arg);
        eval(&self.body, &new_env)
    }
}

/// Semantic value — the result of evaluating a term in an
/// environment. The fundamental data layer of NbE.
///
/// **Distinguishing feature** (vs Term):
///   * `Lam` and `Pi` carry CLOSURES, not raw bodies — capturing
///     the environment at evaluation time gives NbE its compositional
///     evaluation strategy.
///   * `VVar` uses LEVELS (counted from the outside) instead of
///     INDICES (counted from the inside). Levels are stable under
///     binder addition; indices are not. Conversion between the
///     two is the readback function [`quote`]'s primary job.
///   * `VApp` represents NEUTRAL application — `n x` where `n` is a
///     stuck term (e.g. a free variable applied to args).
///     Neutrals can't be reduced further.
#[derive(Debug, Clone)]
pub enum Value {
    /// Universe at the given [`Level`] — concrete or polymorphic.
    /// The carrier shape mirrors `Term::Universe`'s post-FV-19
    /// universe-polymorphic representation.
    VUniverse(Level),
    /// `Π(x : A). B` as a closure-bearing semantic value.
    VPi {
        /// Domain value (already evaluated).
        domain: Box<Value>,
        /// Codomain closure: applied to a value to compute the body.
        codomain: Closure,
    },
    /// `λ(x : A). body` as a closure-bearing semantic value.
    VLam {
        /// Domain value (already evaluated).
        domain: Box<Value>,
        /// Body closure.
        body: Closure,
    },
    /// Neutral term: a free variable or stuck application that
    /// cannot reduce further.
    VNeutral(Neutral),
    /// `Σ(x : A). B` as a closure-bearing semantic value.
    VSigma {
        /// Domain value (already evaluated).
        domain: Box<Value>,
        /// Codomain closure.
        codomain: Closure,
    },
    /// Pair value `(a, b)`.
    VPair(Box<Value>, Box<Value>),
}

/// Neutral term — a stuck reduction. Represents level-indexed free
/// variables and applications headed by neutrals.
#[derive(Debug, Clone)]
pub enum Neutral {
    /// Free variable at the given level. Levels are stable under
    /// binder addition (unlike indices).
    NVar(usize),
    /// Application `n x` where `n` is neutral. Cannot reduce.
    NApp(Box<Neutral>, Box<Value>),
    /// Stuck first projection `fst(n)`.
    NFst(Box<Neutral>),
    /// Stuck second projection `snd(n)`.
    NSnd(Box<Neutral>),
    /// **FV-21 soundness gate**: a stuck head that is itself a
    /// non-function, non-pair value (e.g. `Universe(_)` applied to
    /// an argument or projected via `fst`/`snd`).  Wrapping the
    /// offending value here preserves structural distinctness so
    /// that `def_eq` does NOT silently equate `App(Universe(MAX), x)`
    /// with `Universe(MAX)`.  In a sound run the type-checker
    /// rejects BEFORE reaching this branch (NotAFunction /
    /// NotASigma / UniverseOverflow), but the kernel runs on
    /// adversarial fuzz input via the differential harness, and the
    /// stuck-head form keeps the kernel's reject/accept verdicts in
    /// sync with `proof_checker.rs`'s structural rejection.
    NStuck(Box<Value>),
}

/// Environment — outer-first list of values. Pushed at binder
/// entry (eval extends env on Pi/Lam recursion); read by index
/// during Var evaluation.
pub type Env = Vec<Value>;

// =============================================================================
// Eval — Term → Value
// =============================================================================

/// Evaluate a term in an environment.  This is NbE's compositional
/// core — every term reduces to a Value with closures capturing
/// the env state at the binding site.
pub fn eval(term: &Term, env: &Env) -> Value {
    match term {
        Term::Universe(level) => Value::VUniverse(level.clone()),

        Term::Var(i) => {
            // env is outer-first; Var(i) means "i'th binder from
            // the inside". So lookup index = env.len() - 1 - i.
            let len = env.len();
            if *i >= len {
                // Free variable — produce a neutral. The level is
                // stable: it's the binder's count from the outside,
                // computed as i - len + (current binding-depth-out).
                // We use `len` directly as the level here because
                // free vars at the boundary of `env` correspond to
                // levels equal to `env.len()` (the next free level).
                // For a closed term this branch is unreachable; left
                // for robustness on partial terms used in tests.
                return Value::VNeutral(Neutral::NVar(*i));
            }
            env[len - 1 - *i].clone()
        }

        Term::Pi(domain, body) => {
            let dom_val = eval(domain, env);
            Value::VPi {
                domain: Box::new(dom_val),
                codomain: Closure {
                    env: env.clone(),
                    body: (**body).clone(),
                },
            }
        }

        Term::Lam(domain, body) => {
            let dom_val = eval(domain, env);
            Value::VLam {
                domain: Box::new(dom_val),
                body: Closure {
                    env: env.clone(),
                    body: (**body).clone(),
                },
            }
        }

        Term::App(f, x) => {
            let f_val = eval(f, env);
            let x_val = eval(x, env);
            apply_value(f_val, x_val)
        }

        Term::Sigma(domain, body) => {
            let dom_val = eval(domain, env);
            Value::VSigma {
                domain: Box::new(dom_val),
                codomain: Closure {
                    env: env.clone(),
                    body: (**body).clone(),
                },
            }
        }

        Term::Pair(a, b) => {
            Value::VPair(Box::new(eval(a, env)), Box::new(eval(b, env)))
        }

        Term::Fst(p) => apply_fst(eval(p, env)),
        Term::Snd(p) => apply_snd(eval(p, env)),
    }
}

/// Apply one value to another.
///
/// β-reduces for `VLam`; builds a stuck `NApp` for neutrals; **for
/// non-function heads (FV-21 soundness gate)** wraps the head in
/// `NStuck` so the resulting `App` is structurally distinct from
/// the bare head.  This closes the disagreement found by
/// `multi_kernel_agreement_on_arbitrary_cert`: pre-FV-21 the
/// fallback `_ => f` silently dropped the application, making
/// `App(Universe(MAX), x)` evaluate to `Universe(MAX)` and unsoundly
/// matching the term's inferred type.
pub fn apply_value(f: Value, x: Value) -> Value {
    match f {
        Value::VLam { body, .. } => body.apply(x),
        Value::VNeutral(n) => {
            Value::VNeutral(Neutral::NApp(Box::new(n), Box::new(x)))
        }
        other => {
            // FV-21: wrap the non-function head in NStuck so def_eq
            // sees the application structurally and doesn't equate
            // `App(other, x)` with `other`.
            Value::VNeutral(Neutral::NApp(
                Box::new(Neutral::NStuck(Box::new(other))),
                Box::new(x),
            ))
        }
    }
}

/// First projection on a value (FV-20).  Reduces `VPair(a, _)` → `a`;
/// stays stuck on neutrals via `NFst`; falls through `NStuck` for
/// non-pair, non-neutral heads (FV-21).
pub fn apply_fst(p: Value) -> Value {
    match p {
        Value::VPair(a, _) => *a,
        Value::VNeutral(n) => Value::VNeutral(Neutral::NFst(Box::new(n))),
        other => Value::VNeutral(Neutral::NFst(Box::new(Neutral::NStuck(
            Box::new(other),
        )))),
    }
}

/// Second projection on a value (FV-20).  Symmetric to [`apply_fst`].
pub fn apply_snd(p: Value) -> Value {
    match p {
        Value::VPair(_, b) => *b,
        Value::VNeutral(n) => Value::VNeutral(Neutral::NSnd(Box::new(n))),
        other => Value::VNeutral(Neutral::NSnd(Box::new(Neutral::NStuck(
            Box::new(other),
        )))),
    }
}

// =============================================================================
// Quote — Value → Term (readback)
// =============================================================================

/// Read back a value into a normal-form Term.  The `level` parameter
/// is the current binding depth; quote uses it to generate fresh
/// de Bruijn indices for Lam bodies.
pub fn quote(value: &Value, level: usize) -> Term {
    match value {
        Value::VUniverse(level) => Term::Universe(level.clone()),

        Value::VPi { domain, codomain } => {
            let dom_term = quote(domain, level);
            // Open the closure with a fresh neutral var at the
            // current level, then quote the result at level+1.
            let fresh = Value::VNeutral(Neutral::NVar(level));
            let body_val = codomain.apply(fresh);
            let body_term = quote(&body_val, level + 1);
            Term::Pi(Box::new(dom_term), Box::new(body_term))
        }

        Value::VLam { domain, body } => {
            let dom_term = quote(domain, level);
            let fresh = Value::VNeutral(Neutral::NVar(level));
            let body_val = body.apply(fresh);
            let body_term = quote(&body_val, level + 1);
            Term::Lam(Box::new(dom_term), Box::new(body_term))
        }

        Value::VNeutral(n) => quote_neutral(n, level),

        Value::VSigma { domain, codomain } => {
            let dom_term = quote(domain, level);
            let fresh = Value::VNeutral(Neutral::NVar(level));
            let body_val = codomain.apply(fresh);
            let body_term = quote(&body_val, level + 1);
            Term::Sigma(Box::new(dom_term), Box::new(body_term))
        }

        Value::VPair(a, b) => {
            Term::Pair(Box::new(quote(a, level)), Box::new(quote(b, level)))
        }
    }
}

/// Read back a neutral.  Levels translate into INDICES at readback
/// time: a neutral at level `k` becomes `Var(level - 1 - k)` at the
/// current binding depth.
fn quote_neutral(neutral: &Neutral, level: usize) -> Term {
    match neutral {
        Neutral::NVar(k) => {
            // Level → index conversion: level k at depth `level`
            // becomes index `level - 1 - k`.
            if *k >= level {
                // Free variable at boundary — emit as Var(k) directly.
                // Sound for closed terms (this branch unreachable).
                Term::Var(*k)
            } else {
                Term::Var(level - 1 - *k)
            }
        }
        Neutral::NApp(n, x) => {
            let f_term = quote_neutral(n, level);
            let x_term = quote(x, level);
            Term::App(Box::new(f_term), Box::new(x_term))
        }
        Neutral::NFst(n) => Term::Fst(Box::new(quote_neutral(n, level))),
        Neutral::NSnd(n) => Term::Snd(Box::new(quote_neutral(n, level))),
        // FV-21: read back the wrapped value directly — quote sees
        // exactly what the offending term was, so structural
        // distinctness across `App(Stuck, x)` vs `bare_value` is
        // preserved at readback time.
        Neutral::NStuck(v) => quote(v, level),
    }
}

// =============================================================================
// Definitional equality via NbE
// =============================================================================

/// Definitional equality: two values are equal iff they normalise
/// to the same term.  NbE collapses α/β/η equivalence into pure
/// syntactic comparison after normalisation.
pub fn def_eq(a: &Value, b: &Value, level: usize) -> bool {
    match (a, b) {
        (Value::VUniverse(n), Value::VUniverse(m)) => crate::proof_checker::level_eq(n, m),

        (
            Value::VPi {
                domain: d1,
                codomain: c1,
            },
            Value::VPi {
                domain: d2,
                codomain: c2,
            },
        ) => {
            if !def_eq(d1, d2, level) {
                return false;
            }
            // Open both codomains with the same fresh level.
            let fresh = Value::VNeutral(Neutral::NVar(level));
            let b1 = c1.apply(fresh.clone());
            let b2 = c2.apply(fresh);
            def_eq(&b1, &b2, level + 1)
        }

        (
            Value::VLam {
                domain: d1,
                body: b1,
            },
            Value::VLam {
                domain: d2,
                body: b2,
            },
        ) => {
            if !def_eq(d1, d2, level) {
                return false;
            }
            let fresh = Value::VNeutral(Neutral::NVar(level));
            let body1 = b1.apply(fresh.clone());
            let body2 = b2.apply(fresh);
            def_eq(&body1, &body2, level + 1)
        }

        // η-equivalence: λx. (f x) ≡ f when x ∉ FV(f).
        // NbE handles this naturally: Lam vs Neutral compares by
        // applying the lam's body to a fresh var and checking
        // equality with `neutral applied to same var`.
        (Value::VLam { body, .. }, other) => eta_match(body, other, level),
        (other, Value::VLam { body, .. }) => eta_match(body, other, level),

        (Value::VNeutral(n1), Value::VNeutral(n2)) => def_eq_neutral(n1, n2, level),

        (
            Value::VSigma { domain: d1, codomain: c1 },
            Value::VSigma { domain: d2, codomain: c2 },
        ) => {
            if !def_eq(d1, d2, level) {
                return false;
            }
            let fresh = Value::VNeutral(Neutral::NVar(level));
            let b1 = c1.apply(fresh.clone());
            let b2 = c2.apply(fresh);
            def_eq(&b1, &b2, level + 1)
        }

        (Value::VPair(a1, b1), Value::VPair(a2, b2)) => {
            def_eq(a1, a2, level) && def_eq(b1, b2, level)
        }

        _ => false,
    }
}

/// η-match helper: open the lam's body with a fresh variable and
/// compare to `other applied to that fresh variable`.
fn eta_match(body: &Closure, other: &Value, level: usize) -> bool {
    let fresh = Value::VNeutral(Neutral::NVar(level));
    let body_val = body.apply(fresh.clone());
    let other_app = apply_value(other.clone(), fresh);
    def_eq(&body_val, &other_app, level + 1)
}

fn def_eq_neutral(a: &Neutral, b: &Neutral, level: usize) -> bool {
    match (a, b) {
        (Neutral::NVar(i), Neutral::NVar(j)) => i == j,
        (Neutral::NApp(f1, x1), Neutral::NApp(f2, x2)) => {
            def_eq_neutral(f1, f2, level) && def_eq(x1, x2, level)
        }
        (Neutral::NFst(n1), Neutral::NFst(n2)) => def_eq_neutral(n1, n2, level),
        (Neutral::NSnd(n1), Neutral::NSnd(n2)) => def_eq_neutral(n1, n2, level),
        // FV-21: two stuck heads are equal iff their wrapped values
        // are equal — preserves structural distinctness w.r.t. bare
        // values (which compare via their own arms in `def_eq`).
        (Neutral::NStuck(v1), Neutral::NStuck(v2)) => def_eq(v1, v2, level),
        _ => false,
    }
}

// =============================================================================
// Type-checker (NbE-based)
// =============================================================================

/// NbE type-check context: a stack of (value, type) pairs at each
/// binder. value is a fresh neutral at the binder's level; type is
/// the binder's domain (as a value).
#[derive(Debug, Clone, Default)]
pub struct NbeContext {
    /// Stack of binder types as values, outermost-first.
    types: Vec<Value>,
    /// Stack of binder values (used as the env for eval).
    /// Each entry is a fresh neutral at the binder's level.
    env: Env,
}

impl NbeContext {
    /// Construct an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current binding depth (level for next fresh neutral).
    pub fn level(&self) -> usize {
        self.env.len()
    }

    /// Extend the context with a new binder of the given domain
    /// type.  The new binder's value is a fresh neutral at the
    /// current level.
    pub fn extend(&self, domain_value: Value) -> Self {
        let mut new_types = self.types.clone();
        new_types.push(domain_value);
        let mut new_env = self.env.clone();
        new_env.push(Value::VNeutral(Neutral::NVar(self.env.len())));
        Self {
            types: new_types,
            env: new_env,
        }
    }

    /// Look up the type of variable at de Bruijn index `i`.
    pub fn lookup(&self, i: usize) -> Option<&Value> {
        let len = self.types.len();
        if i >= len {
            return None;
        }
        Some(&self.types[len - 1 - i])
    }

    /// The current evaluation environment.
    pub fn env(&self) -> &Env {
        &self.env
    }
}

/// Infer the type of `term` under `ctx`. NbE-based — uses semantic
/// values internally, returns the inferred type as a Term (quoted).
///
/// **Architectural twin** of `proof_checker::infer`: same input/
/// output behavior, different evaluation strategy.
pub fn infer(ctx: &NbeContext, term: &Term) -> Result<Term, CheckError> {
    let inferred_value = infer_value(ctx, term)?;
    Ok(quote(&inferred_value, ctx.level()))
}

/// Internal: infer the type as a semantic Value (avoids quote/eval
/// roundtrips during recursive checking).
fn infer_value(ctx: &NbeContext, term: &Term) -> Result<Value, CheckError> {
    match term {
        Term::Var(i) => ctx
            .lookup(*i)
            .cloned()
            .ok_or_else(|| CheckError::UnboundVariable(*i)),

        // T-Univ — universe-polymorphic mirror of the trusted base.
        // Concrete-overflow rejected via [`Level::checked_succ`];
        // symbolic carriers pass through structurally.  Mirrors
        // `proof_checker.rs::infer` Term::Universe arm.
        Term::Universe(level) => {
            let level = level.clone().normalize();
            match level.checked_succ() {
                Some(next) => Ok(Value::VUniverse(next)),
                None => Err(CheckError::UniverseOverflow {
                    level: match level {
                        Level::Concrete(n) => n,
                        _ => u32::MAX,
                    },
                }),
            }
        }

        Term::Pi(a, b) => {
            let a_ty = infer_value(ctx, a)?;
            let n = expect_universe(&a_ty).ok_or_else(|| CheckError::NotAType((**a).clone()))?;
            let a_value = eval(a, ctx.env());
            let extended = ctx.extend(a_value);
            let b_ty = infer_value(&extended, b)?;
            let m = expect_universe(&b_ty).ok_or_else(|| CheckError::NotAType((**b).clone()))?;
            Ok(Value::VUniverse(n.max_with(m)))
        }

        Term::Lam(domain, body) => {
            let dom_ty = infer_value(ctx, domain)?;
            expect_universe(&dom_ty).ok_or_else(|| CheckError::NotAType((**domain).clone()))?;
            let dom_value = eval(domain, ctx.env());
            let extended = ctx.extend(dom_value.clone());
            let body_ty_value = infer_value(&extended, body)?;
            // Build the result type: Π(x : domain). body_ty
            let body_ty_term = quote(&body_ty_value, extended.level());
            let result = Term::Pi(domain.clone(), Box::new(body_ty_term));
            Ok(eval(&result, ctx.env()))
        }

        Term::App(f, x) => {
            let f_ty = infer_value(ctx, f)?;
            let (dom, codom) = match f_ty {
                Value::VPi { domain, codomain } => (domain, codomain),
                other => {
                    return Err(CheckError::NotAFunction(quote(&other, ctx.level())));
                }
            };
            let x_ty = infer_value(ctx, x)?;
            if !def_eq(&dom, &x_ty, ctx.level()) {
                return Err(CheckError::DomainMismatch {
                    expected: quote(&dom, ctx.level()),
                    actual: quote(&x_ty, ctx.level()),
                });
            }
            let x_value = eval(x, ctx.env());
            Ok(codom.apply(x_value))
        }

        // T-Sigma-Form (NbE): Σ(x:A).B : Universe(max(level(A), level(B)))
        Term::Sigma(a, b) => {
            let a_ty = infer_value(ctx, a)?;
            let n = expect_universe(&a_ty).ok_or_else(|| CheckError::NotAType((**a).clone()))?;
            let a_value = eval(a, ctx.env());
            let extended = ctx.extend(a_value);
            let b_ty = infer_value(&extended, b)?;
            let m = expect_universe(&b_ty).ok_or_else(|| CheckError::NotAType((**b).clone()))?;
            Ok(Value::VUniverse(n.max_with(m)))
        }

        // T-Pair-Intro (NbE): synthesis-mode mirror.  The synthesised
        // Σ is non-dependent (`B` doesn't reference de Bruijn 0);
        // dependent claims are reached via `check`.  The closure
        // body is the quoted snd-type at the OUTER level; we
        // produce it via `Term::Pair`'s ad-hoc shift through
        // re-evaluation below.
        Term::Pair(a, b) => {
            let a_ty = infer_value(ctx, a)?;
            let b_ty = infer_value(ctx, b)?;
            let b_ty_term = quote(&b_ty, ctx.level());
            // Build a Σ-value using the existing Lam/Pi closure
            // infrastructure: we evaluate `Sigma(quote(a_ty),
            // shift_up(b_ty_term))` so the closure captures the
            // current env at the binding site (the trick used by
            // T-Lam-Intro above for Pi-from-Lam).
            let a_ty_term = quote(&a_ty, ctx.level());
            let b_ty_shifted =
                crate::proof_checker::lift_term_one_binder(b_ty_term);
            let result = Term::Sigma(Box::new(a_ty_term), Box::new(b_ty_shifted));
            Ok(eval(&result, ctx.env()))
        }

        // T-Fst-Elim (NbE): Fst(p) : A where p : Σ(A, B).
        Term::Fst(p) => {
            let p_ty = infer_value(ctx, p)?;
            match p_ty {
                Value::VSigma { domain, .. } => Ok(*domain),
                other => Err(CheckError::NotASigma(quote(&other, ctx.level()))),
            }
        }

        // T-Snd-Elim (NbE): Snd(p) : B[Fst(p)/0] where p : Σ(A, B).
        // Discharge the dependency by applying the codomain closure
        // to `Fst(p)`'s VALUE — this is the NbE realisation of the
        // term-level substitution `B[Fst(p)/0]`.
        Term::Snd(p) => {
            let p_ty = infer_value(ctx, p)?;
            match p_ty {
                Value::VSigma { codomain, .. } => {
                    let p_value = eval(p, ctx.env());
                    let fst_value = apply_fst(p_value);
                    Ok(codomain.apply(fst_value))
                }
                other => Err(CheckError::NotASigma(quote(&other, ctx.level()))),
            }
        }
    }
}

/// Check that `term` has type `expected` under `ctx`. NbE-based.
///
/// **Architectural twin** of `proof_checker::check`. The two
/// implementations MUST agree on every well-typed certificate;
/// disagreements are bugs in either side.
pub fn check(ctx: &NbeContext, term: &Term, expected: &Term) -> Result<(), CheckError> {
    let inferred = infer_value(ctx, term)?;
    let expected_value = eval(expected, ctx.env());
    if def_eq(&inferred, &expected_value, ctx.level()) {
        Ok(())
    } else {
        Err(CheckError::TypeMismatch {
            expected: expected.clone(),
            actual: quote(&inferred, ctx.level()),
        })
    }
}

/// If `value` is `VUniverse(level)`, return the normalised level;
/// else `None`.  Universe-polymorphic — the returned level may be
/// symbolic.
fn expect_universe(value: &Value) -> Option<Level> {
    match value {
        Value::VUniverse(level) => Some(level.clone().normalize()),
        _ => None,
    }
}

// =============================================================================
// Certificate verification (parity with proof_checker::Certificate::verify)
// =============================================================================

/// Verify a [`Certificate`](crate::proof_checker::Certificate) using
/// the NbE kernel.  Architectural twin of
/// [`crate::proof_checker::Certificate::verify`] — mirrors its
/// claimed-type well-formedness check (claimed_type must itself be
/// a type) plus the universe-tower-top escape hatch (a
/// claimed_type at the very top of the universe tower triggers
/// `UniverseOverflow` on its successor's kind-check, but is still a
/// valid type — swallow the overflow there and let the structural
/// check downstream catch any genuine type mismatch).
pub fn verify_certificate(
    cert: &crate::proof_checker::Certificate,
) -> Result<(), CheckError> {
    let ctx = NbeContext::new();
    match infer(&ctx, &cert.claimed_type) {
        Ok(claimed_kind) => {
            let claimed_kind_value = eval(&claimed_kind, ctx.env());
            if expect_universe(&claimed_kind_value).is_none() {
                return Err(CheckError::ClaimedTypeNotAType {
                    claimed_type: cert.claimed_type.clone(),
                    actual: claimed_kind,
                });
            }
        }
        Err(CheckError::UniverseOverflow { .. }) => {
            // claimed_type lives at the top of the universe tower
            // — still a type.  Mirrors proof_checker.rs::verify.
        }
        Err(other) => return Err(other),
    }
    check(&ctx, &cert.term, &cert.claimed_type)
}

// =============================================================================
// Tests — parity + agreement pins
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proof_checker::{self, Certificate};

    /// Build the polymorphic-identity certificate (same as
    /// proof_checker::tests). Term: λ(A:Universe(0)). λ(x:Var(0)). Var(0).
    /// Type: Π(A:Universe(0)). Π(x:Var(0)). Var(1).
    fn polymorphic_identity() -> Certificate {
        let term = Term::lam(
            Term::universe(0),
            Term::lam(Term::var(0), Term::var(0)),
        );
        let claimed_type = Term::pi(
            Term::universe(0),
            Term::pi(Term::var(0), Term::var(1)),
        );
        Certificate {
            term,
            claimed_type,
            metadata: std::collections::BTreeMap::new(),
        }
    }

    /// Build the simpler identity at Universe(0): λ(x:Universe(0)). x.
    /// Type: Π(x:Universe(0)). Universe(0).
    fn identity_universe_0() -> Certificate {
        let term = Term::lam(Term::universe(0), Term::var(0));
        let claimed_type = Term::pi(Term::universe(0), Term::universe(0));
        Certificate {
            term,
            claimed_type,
            metadata: std::collections::BTreeMap::new(),
        }
    }

    // ----- Eval / Quote roundtrip -----

    #[test]
    fn eval_quote_universe_round_trip() {
        let env: Env = vec![];
        let v = eval(&Term::universe(0), &env);
        assert!(matches!(&v, Value::VUniverse(l) if l.as_concrete() == Some(0)));
        let t = quote(&v, 0);
        assert!(matches!(&t, Term::Universe(l) if l.as_concrete() == Some(0)));
    }

    #[test]
    fn eval_quote_lam_round_trip_preserves_alpha() {
        // λ(x : U_0). x — should round-trip identically.
        let term = Term::lam(Term::universe(0), Term::var(0));
        let v = eval(&term, &vec![]);
        let t = quote(&v, 0);
        // After eval/quote, the Lam structure is preserved.
        assert!(matches!(t, Term::Lam(..)));
    }

    // ----- def_eq agreement with proof_checker -----

    #[test]
    fn def_eq_alpha_equivalence() {
        // λ(x : U_0). x  vs  λ(y : U_0). y — same after de Bruijn.
        let a = Term::lam(Term::universe(0), Term::var(0));
        let b = Term::lam(Term::universe(0), Term::var(0));
        let v_a = eval(&a, &vec![]);
        let v_b = eval(&b, &vec![]);
        assert!(def_eq(&v_a, &v_b, 0));
    }

    #[test]
    fn def_eq_distinct_universes_rejected() {
        let v_a = Value::VUniverse(Level::Concrete(0));
        let v_b = Value::VUniverse(Level::Concrete(1));
        assert!(!def_eq(&v_a, &v_b, 0));
    }

    // ----- Certificate verification: NbE accepts known-good -----

    #[test]
    fn nbe_accepts_polymorphic_identity_certificate() {
        let cert = polymorphic_identity();
        let result = verify_certificate(&cert);
        assert!(
            result.is_ok(),
            "NbE must accept the polymorphic identity: {:?}",
            result,
        );
    }

    #[test]
    fn nbe_accepts_identity_universe_0_certificate() {
        let cert = identity_universe_0();
        let result = verify_certificate(&cert);
        assert!(
            result.is_ok(),
            "NbE must accept identity_universe_0: {:?}",
            result,
        );
    }

    // ----- Differential agreement (the load-bearing pin) -----

    #[test]
    fn nbe_and_proof_checker_agree_on_polymorphic_identity() {
        let cert = polymorphic_identity();
        let nbe_outcome = verify_certificate(&cert);
        let trusted_outcome = cert.verify();
        // Both kernels must agree on the polymorphic identity. This
        // is the load-bearing pin: any disagreement is a bug.
        assert_eq!(
            nbe_outcome.is_ok(),
            trusted_outcome.is_ok(),
            "NbE/trusted-base disagreement: nbe={:?}, trusted={:?}",
            nbe_outcome,
            trusted_outcome,
        );
    }

    #[test]
    fn nbe_and_proof_checker_agree_on_identity_universe_0() {
        let cert = identity_universe_0();
        let nbe_outcome = verify_certificate(&cert);
        let trusted_outcome = cert.verify();
        assert_eq!(
            nbe_outcome.is_ok(),
            trusted_outcome.is_ok(),
            "NbE/trusted-base disagreement on identity_universe_0",
        );
    }

    #[test]
    fn nbe_rejects_universe_mismatch() {
        // Mismatched universe levels — both kernels must reject.
        let term = Term::universe(0);
        let claimed_type = Term::universe(0); // Wrong: should be Universe(1).
        let cert = Certificate {
            term,
            claimed_type,
            metadata: std::collections::BTreeMap::new(),
        };
        let nbe_outcome = verify_certificate(&cert);
        let trusted_outcome = cert.verify();
        assert!(nbe_outcome.is_err());
        assert!(trusted_outcome.is_err());
    }

    #[test]
    fn nbe_rejects_unbound_variable() {
        // Var(0) at empty context — neither kernel should accept.
        let term = Term::Var(0);
        let claimed_type = Term::universe(0);
        let cert = Certificate {
            term,
            claimed_type,
            metadata: std::collections::BTreeMap::new(),
        };
        assert!(verify_certificate(&cert).is_err());
        assert!(cert.verify().is_err());
    }

    // ----- NbE-specific structural invariants -----

    #[test]
    fn closure_apply_is_associative_with_eval() {
        // Closure { env: [], body: Var(0) }.apply(VUniverse(0))
        // should produce VUniverse(0). The env-shift in apply is
        // the load-bearing piece.
        let closure = Closure {
            env: vec![],
            body: Term::Var(0),
        };
        let result = closure.apply(Value::VUniverse(Level::Concrete(0)));
        match result {
            Value::VUniverse(l) if l.as_concrete() == Some(0) => {}
            other => panic!("expected VUniverse(Concrete(0)), got {:?}", other),
        }
    }

    #[test]
    fn quote_uses_levels_not_indices() {
        // Verify that quote produces correct de Bruijn indices when
        // reading back a value with neutral variables. A neutral
        // at level 0 inside a binder at depth 1 should become Var(0).
        let neutral_at_level_0 = Value::VNeutral(Neutral::NVar(0));
        let term_at_depth_1 = quote(&neutral_at_level_0, 1);
        match term_at_depth_1 {
            Term::Var(0) => {}
            other => panic!("expected Var(0), got {:?}", other),
        }
    }

    #[test]
    fn nbe_context_extend_pushes_fresh_level() {
        let ctx = NbeContext::new();
        assert_eq!(ctx.level(), 0);
        let extended = ctx.extend(Value::VUniverse(Level::Concrete(0)));
        assert_eq!(extended.level(), 1);
        // The extended context's most-recent type binding is the
        // pushed value.
        let ty = extended.lookup(0).unwrap();
        assert!(matches!(ty, Value::VUniverse(l) if l.as_concrete() == Some(0)));
    }
}
