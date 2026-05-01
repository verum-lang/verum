//! # Kernel reflection — the meta-theory escape hatch (#158, current slice)
//!

//! ## Architectural role
//!

//! Reflective theorem provers let the kernel reason about ITS OWN
//! syntax + semantics from inside the language. ACL2's metafunctions,
//! Coq's `Reflection` library, and Lean's `decide` / `native_decide`
//! are the canonical examples. Pre-this-module Verum has no
//! reflective surface: every consumer of [`proof_checker::Term`] —
//! the elaborator, the audit gates, future Verum-side meta-tactics —
//! has to either re-import `proof_checker.rs` (widening the trusted
//! base) or hand-roll its own term mirror (introducing a drift hazard).
//!

//! This module ships the **V0 reflective surface**: a serializable
//! mirror of [`proof_checker::Term`] that carries the kernel's term
//! grammar + judgment shape as **data**, exposed to non-trusted
//! callers without dragging the trusted base across the API
//! boundary. A future Verum-side meta-tactic can then pattern-match
//! on [`ReflectedTerm`] / [`ReflectedKernelRule`] without ever
//! importing `proof_checker.rs` directly.
//!

//! ## scope (this slice)
//!

//! 1. [`ReflectedTerm`] — serializable mirror of [`proof_checker::Term`].
//!  One variant per kernel term constructor. `From<&Term>` +
//!  `TryFrom<&ReflectedTerm>` give a total round-trip.
//! 2. [`ReflectedJudgment`] — `Γ ⊢ t : T` reified as data: context
//!  depth + reflected term + reflected expected type.
//! 3. [`ReflectedKernelRule`] — name + premise/conclusion judgments
//!  for one of the six kernel rules (T-Var, T-Univ, T-Pi-Form,
//!  T-Lam-Intro, T-App-Elim, T-Conv).
//! 4. [`reflect_kernel_rule`] — the lookup `rule_name -> Option<…>`.
//! 5. [`is_reflected_well_formed`] — surface-level sanity check
//!  (de Bruijn indices in range, reflected types reference live
//!  binders, …).
//!

//! ## non-goals (deferred to V1)
//!

//! - **Wiring into the elaborator.** Reflection is a *data layer*
//!  today; downstream callers can read the surface but no
//!  elaborator path consumes it. Wiring follows once the V0
//!  shape stabilises through use.
//! - **Verum-source surface.** `core/verify/reflection.vr` is a
//!  future deliverable. V0 is Rust-only.
//! - **Decision procedures over [`ReflectedTerm`].** No
//!  `reflected_def_eq` / `reflected_normalize` / `reflected_type_check`
//!  yet — those compose with V1.
//!

//! ## Structural sketch — what each kernel rule reflects to
//!

//! The `reflect_kernel_rule` builders ship **abstract structural
//! sketches** of each rule. These are intentionally *schematic*:
//! they record the SHAPE of premises + conclusion using stand-in
//! `Var(0)` / `Universe(0)` placeholders, not the full higher-order
//! quantification a kernel rule classically carries (e.g., `T-Pi-Form`
//! quantifies over universe levels `n`, `m` and types `A`, `B`; the
//! sketch shows that with a two-premise / one-conclusion shape but
//! plugs in concrete universe levels).
//!

//! The structural sketches are sufficient for V0 meta-tactics that
//! enumerate "the kernel has six rules, here's their arity and
//! premise count". Future work will lift the sketches to fully-quantified
//! schemata once the pattern-matching DSL on the meta-tactic side
//! lands.
//!

//! ## Mirror invariant
//!

//! The mirror invariant is:
//!

//! ```text
//!  for every t : Term ,
//!  Term::try_from(&ReflectedTerm::from(&t)) == Ok(t)
//! ```
//!

//! Tests pin this for every variant. Drift between [`Term`] and
//! [`ReflectedTerm`] is the failure mode: adding a variant to one
//! without the other breaks the round-trip.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

use crate::proof_checker::Term;

// =============================================================================
// ReflectedTerm — serializable mirror of proof_checker::Term
// =============================================================================

/// Serializable mirror of [`proof_checker::Term`].
///

/// Variants are 1:1 with [`Term`]; the payloads are recursively
/// reflected. This indirection lets non-trusted callers (audit
/// gates, future meta-tactics) consume the kernel's term grammar as
/// data — *without* importing `proof_checker.rs` directly.
///

/// The lossless round-trip [`Term`] ↔ [`ReflectedTerm`] is pinned by
/// the test suite below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReflectedTerm {
    /// Mirror of [`Term::Var`].
    Var {
        /// de Bruijn index — `0` is the innermost binder.
        index: usize,
    },

    /// Mirror of [`Term::Universe`].
    Universe {
        /// Universe level (`Type` is `level = 0`).
        level: u32,
    },

    /// Mirror of [`Term::Pi`].
    Pi {
        /// Reflected domain `A`.
        domain: Box<ReflectedTerm>,
        /// Reflected codomain `B` under the binder.
        body: Box<ReflectedTerm>,
    },

    /// Mirror of [`Term::Lam`].
    Lam {
        /// Reflected binder annotation `A`.
        domain: Box<ReflectedTerm>,
        /// Reflected body under the binder.
        body: Box<ReflectedTerm>,
    },

    /// Mirror of [`Term::App`].
    App {
        /// Reflected function part.
        function: Box<ReflectedTerm>,
        /// Reflected argument.
        argument: Box<ReflectedTerm>,
    },
}

// =============================================================================
// Reflection error
// =============================================================================

/// Errors reported when reflecting (or de-reflecting) kernel data.
///

/// current surface is intentionally narrow: the only failure mode today
/// is a reflected term whose internal structure is malformed. The
/// `From<&Term>` direction is total and never errors; the
/// `TryFrom<&ReflectedTerm>` direction inherits the shape so it too
/// is total *for V0 variants* — the error type exists for
/// forward-compatibility (V1+ may carry typed extensions where
/// reflection becomes lossy on unknown shapes).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReflectionError {
    /// The reflected term references a de Bruijn index that exceeds
    /// the surrounding binder depth. Surfaced by
    /// [`is_reflected_well_formed`] but not by `TryFrom` — the
    /// `TryFrom` direction is purely structural.
    #[error("de Bruijn index {index} out of range (max {max})")]
    DeBruijnOutOfRange {
        /// The offending index.
        index: usize,
        /// The maximum permitted index given context depth.
        max: usize,
    },

    /// Reserved for V1: reflected term carries a constructor unknown
    /// to this kernel version.
    #[error("unknown reflected-term constructor: {0}")]
    UnknownConstructor(String),
}

// =============================================================================
// From / TryFrom round-trip
// =============================================================================

impl From<&Term> for ReflectedTerm {
    fn from(term: &Term) -> Self {
        match term {
            Term::Var(i) => ReflectedTerm::Var { index: *i },
            Term::Universe(n) => ReflectedTerm::Universe { level: *n },
            Term::Pi(a, b) => ReflectedTerm::Pi {
                domain: Box::new(ReflectedTerm::from(a.as_ref())),
                body: Box::new(ReflectedTerm::from(b.as_ref())),
            },
            Term::Lam(a, b) => ReflectedTerm::Lam {
                domain: Box::new(ReflectedTerm::from(a.as_ref())),
                body: Box::new(ReflectedTerm::from(b.as_ref())),
            },
            Term::App(f, x) => ReflectedTerm::App {
                function: Box::new(ReflectedTerm::from(f.as_ref())),
                argument: Box::new(ReflectedTerm::from(x.as_ref())),
            },
        }
    }
}

impl TryFrom<&ReflectedTerm> for Term {
    type Error = ReflectionError;

    fn try_from(reflected: &ReflectedTerm) -> Result<Self, Self::Error> {
        match reflected {
            ReflectedTerm::Var { index } => Ok(Term::Var(*index)),
            ReflectedTerm::Universe { level } => Ok(Term::Universe(*level)),
            ReflectedTerm::Pi { domain, body } => Ok(Term::pi(
                Term::try_from(domain.as_ref())?,
                Term::try_from(body.as_ref())?,
            )),
            ReflectedTerm::Lam { domain, body } => Ok(Term::lam(
                Term::try_from(domain.as_ref())?,
                Term::try_from(body.as_ref())?,
            )),
            ReflectedTerm::App { function, argument } => Ok(Term::app(
                Term::try_from(function.as_ref())?,
                Term::try_from(argument.as_ref())?,
            )),
        }
    }
}

// =============================================================================
// ReflectedJudgment — Γ ⊢ t : T as data
// =============================================================================

/// Reflected typing judgment `Γ ⊢ t : T`.
///

/// The context `Γ` is summarised by its **depth** rather than its
/// full reified contents — for V0 meta-tactics, the depth is what
/// matters for de Bruijn validity (rule sketches don't yet need to
/// inspect individual context entries, just their count). V1 may
/// promote `context_depth` to `context: Vec<ReflectedTerm>` when the
/// meta-tactic surface needs entry-level inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflectedJudgment {
    /// Number of binders in the surrounding context. Used by
    /// [`is_reflected_well_formed`] to bound legal de Bruijn indices.
    pub context_depth: usize,

    /// The term being judged.
    pub term: ReflectedTerm,

    /// The expected type of `term`.
    pub expected_type: ReflectedTerm,
}

impl ReflectedJudgment {
    /// Construct a reflected judgment at the empty context (depth 0).
    pub fn closed(term: ReflectedTerm, expected_type: ReflectedTerm) -> Self {
        Self {
            context_depth: 0,
            term,
            expected_type,
        }
    }

    /// Construct a reflected judgment with explicit context depth.
    pub fn at_depth(
        depth: usize,
        term: ReflectedTerm,
        expected_type: ReflectedTerm,
    ) -> Self {
        Self {
            context_depth: depth,
            term,
            expected_type,
        }
    }
}

// =============================================================================
// ReflectedKernelRule — the meta-data shape of one kernel rule
// =============================================================================

/// Reflected kernel inference rule.
///

/// Carries the rule's stable name + a structural sketch of its
/// premises and conclusion as [`ReflectedJudgment`] values. This is
/// the data a Verum-side meta-tactic enumerates when asking "what
/// rules does the kernel know about?" — without ever touching
/// `proof_checker.rs`.
///

/// V0 sketches use stand-in `Var(0)` / `Universe(0)` placeholders
/// where a fully-quantified schema would carry meta-variables; see
/// the module-level docs for the Future-work promotion path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflectedKernelRule {
    /// Stable rule name — one of `"T-Var"`, `"T-Univ"`, `"T-Pi-Form"`,
    /// `"T-Lam-Intro"`, `"T-App-Elim"`, `"T-Conv"`.
    pub name: String,

    /// Premise judgments. `T-Var` / `T-Univ` have zero premises;
    /// `T-Pi-Form` has two; the rest have one.
    pub premises: Vec<ReflectedJudgment>,

    /// Conclusion judgment.
    pub conclusion: ReflectedJudgment,
}

// =============================================================================
// reflect_kernel_rule — the rule-name -> shape lookup
// =============================================================================

/// Stable list of kernel-rule names this V0 reflection surface
/// recognises. Mirrors the six rules implemented in
/// [`crate::proof_checker`].
///

/// Adding a rule to `proof_checker.rs` requires extending this
/// constant + adding a new branch in [`reflect_kernel_rule`].
pub const KERNEL_RULE_NAMES: &[&str] = &[
    "T-Var",
    "T-Univ",
    "T-Pi-Form",
    "T-Lam-Intro",
    "T-App-Elim",
    "T-Conv",
];

/// Reflect one of the six kernel rules into its abstract structural
/// sketch. Returns `None` for any name not in [`KERNEL_RULE_NAMES`].
///

/// ## V0 sketch shapes
///

/// | Rule | Premise count | Sketch |
/// |---------------|---------------|--------------------------------------------------------|
/// | `T-Var` | 0 | `Γ ⊢ Var(0) : Var(0)` (axiomatic — context lookup) |
/// | `T-Univ` | 0 | `Γ ⊢ Universe(0) : Universe(1)` |
/// | `T-Pi-Form` | 2 | `Γ⊢A:U(0)`, `Γ,A⊢B:U(0)` ⇒ `Γ⊢Π(A).B : U(0)` |
/// | `T-Lam-Intro` | 1 | `Γ,A ⊢ b : B` ⇒ `Γ ⊢ λ(A).b : Π(A).B` |
/// | `T-App-Elim` | 1 | `Γ ⊢ f : Π(A).B` ⇒ `Γ ⊢ App(f, Var(0)) : B` |
/// | `T-Conv` | 1 | `Γ ⊢ t : A`, `A ≡_β B` ⇒ `Γ ⊢ t : B` |
///

/// The placeholders use de Bruijn `Var(0)` to mean "the freshest
/// binder in scope"; `Universe(0)` is `Type` and `Universe(1)` is
/// `Type+1`. The full quantified schemata land in V1.
pub fn reflect_kernel_rule(rule_name: &str) -> Option<ReflectedKernelRule> {
    match rule_name {
        "T-Var" => Some(reflect_t_var()),
        "T-Univ" => Some(reflect_t_univ()),
        "T-Pi-Form" => Some(reflect_t_pi_form()),
        "T-Lam-Intro" => Some(reflect_t_lam_intro()),
        "T-App-Elim" => Some(reflect_t_app_elim()),
        "T-Conv" => Some(reflect_t_conv()),
        _ => None,
    }
}

// -----------------------------------------------------------------------------
// Per-rule structural sketches
// -----------------------------------------------------------------------------

fn reflect_t_var() -> ReflectedKernelRule {
    // T-Var: under a context with at least one binder, Var(0) has the
    // type recorded for it. Schematic placeholder uses Var(0) twice
    // to encode "term and type are both reified placeholders".
    let conclusion = ReflectedJudgment::at_depth(
        1,
        ReflectedTerm::Var { index: 0 },
        ReflectedTerm::Var { index: 0 },
    );
    ReflectedKernelRule {
        name: "T-Var".to_string(),
        premises: Vec::new(),
        conclusion,
    }
}

fn reflect_t_univ() -> ReflectedKernelRule {
    // T-Univ: Universe(n) : Universe(n+1). V0 sketch pins n=0.
    let conclusion = ReflectedJudgment::closed(
        ReflectedTerm::Universe { level: 0 },
        ReflectedTerm::Universe { level: 1 },
    );
    ReflectedKernelRule {
        name: "T-Univ".to_string(),
        premises: Vec::new(),
        conclusion,
    }
}

fn reflect_t_pi_form() -> ReflectedKernelRule {
    // T-Pi-Form: Γ ⊢ A : U(n), Γ,A ⊢ B : U(m) ⇒ Γ ⊢ Π(A).B : U(max n m).
    // V0 sketch pins n = m = 0.
    let premise_a = ReflectedJudgment::closed(
        ReflectedTerm::Var { index: 0 },
        ReflectedTerm::Universe { level: 0 },
    );
    let premise_b = ReflectedJudgment::at_depth(
        1,
        ReflectedTerm::Var { index: 0 },
        ReflectedTerm::Universe { level: 0 },
    );
    let conclusion = ReflectedJudgment::closed(
        ReflectedTerm::Pi {
            domain: Box::new(ReflectedTerm::Var { index: 0 }),
            body: Box::new(ReflectedTerm::Var { index: 0 }),
        },
        ReflectedTerm::Universe { level: 0 },
    );
    ReflectedKernelRule {
        name: "T-Pi-Form".to_string(),
        premises: vec![premise_a, premise_b],
        conclusion,
    }
}

fn reflect_t_lam_intro() -> ReflectedKernelRule {
    // T-Lam-Intro: Γ,A ⊢ b : B ⇒ Γ ⊢ λ(A).b : Π(A).B.
    let premise = ReflectedJudgment::at_depth(
        1,
        ReflectedTerm::Var { index: 0 },
        ReflectedTerm::Var { index: 0 },
    );
    let conclusion = ReflectedJudgment::closed(
        ReflectedTerm::Lam {
            domain: Box::new(ReflectedTerm::Var { index: 0 }),
            body: Box::new(ReflectedTerm::Var { index: 0 }),
        },
        ReflectedTerm::Pi {
            domain: Box::new(ReflectedTerm::Var { index: 0 }),
            body: Box::new(ReflectedTerm::Var { index: 0 }),
        },
    );
    ReflectedKernelRule {
        name: "T-Lam-Intro".to_string(),
        premises: vec![premise],
        conclusion,
    }
}

fn reflect_t_app_elim() -> ReflectedKernelRule {
    // T-App-Elim: Γ ⊢ f : Π(A).B ⇒ Γ ⊢ App(f, x) : B[x/0].
    let premise = ReflectedJudgment::at_depth(
        1,
        ReflectedTerm::Var { index: 0 },
        ReflectedTerm::Pi {
            domain: Box::new(ReflectedTerm::Var { index: 0 }),
            body: Box::new(ReflectedTerm::Var { index: 0 }),
        },
    );
    let conclusion = ReflectedJudgment::at_depth(
        1,
        ReflectedTerm::App {
            function: Box::new(ReflectedTerm::Var { index: 0 }),
            argument: Box::new(ReflectedTerm::Var { index: 0 }),
        },
        ReflectedTerm::Var { index: 0 },
    );
    ReflectedKernelRule {
        name: "T-App-Elim".to_string(),
        premises: vec![premise],
        conclusion,
    }
}

fn reflect_t_conv() -> ReflectedKernelRule {
    // T-Conv: Γ ⊢ t : A, A ≡_β B ⇒ Γ ⊢ t : B.
    // V0 sketch elides the convertibility premise — it's a meta-side
    // condition, not a typing judgment. Only the typed premise +
    // conclusion appear as data.
    let premise = ReflectedJudgment::closed(
        ReflectedTerm::Var { index: 0 },
        ReflectedTerm::Var { index: 0 },
    );
    let conclusion = ReflectedJudgment::closed(
        ReflectedTerm::Var { index: 0 },
        ReflectedTerm::Var { index: 0 },
    );
    ReflectedKernelRule {
        name: "T-Conv".to_string(),
        premises: vec![premise],
        conclusion,
    }
}

// =============================================================================
// is_reflected_well_formed — surface-level sanity check
// =============================================================================

/// Surface-level sanity check on a [`ReflectedJudgment`].
///

/// Verifies:
///

///  * Every de Bruijn `Var(i)` inside `term` and `expected_type`
///  satisfies `i < context_depth + d`, where `d` is the number of
///  binders crossed when descending into the term.
///

/// Returns `true` if the reflected judgment is syntactically
/// well-formed under its declared context depth. Returns `false`
/// if any sub-term references an out-of-range de Bruijn index.
///

/// **Not a type check.** This routine doesn't run the kernel — it
/// only catches the cheapest class of malformed reflected data.
/// A well-formed reflected judgment may still be ill-typed; conversely
/// the kernel's own `infer` is the verdict authority for actual
/// typing correctness.
pub fn is_reflected_well_formed(judgment: &ReflectedJudgment) -> bool {
    debruijn_in_range(&judgment.term, judgment.context_depth)
        && debruijn_in_range(&judgment.expected_type, judgment.context_depth)
}

/// Recursive helper: every `Var(i)` inside `term` must satisfy
/// `i < depth + d` where `d` is the number of binders crossed since
/// the outer call.
fn debruijn_in_range(term: &ReflectedTerm, depth: usize) -> bool {
    match term {
        ReflectedTerm::Var { index } => *index < depth,
        ReflectedTerm::Universe { .. } => true,
        ReflectedTerm::Pi { domain, body } => {
            debruijn_in_range(domain, depth)
                && debruijn_in_range(body, depth + 1)
        }
        ReflectedTerm::Lam { domain, body } => {
            debruijn_in_range(domain, depth)
                && debruijn_in_range(body, depth + 1)
        }
        ReflectedTerm::App { function, argument } => {
            debruijn_in_range(function, depth)
                && debruijn_in_range(argument, depth)
        }
    }
}

// =============================================================================
// Tests — round-trip + rule lookup + serde + well-formedness
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Round-trip: Term -> ReflectedTerm -> Term, every variant
    // -------------------------------------------------------------------------

    fn roundtrip(t: Term) {
        let reflected = ReflectedTerm::from(&t);
        let back = Term::try_from(&reflected).expect("round-trip failed");
        assert_eq!(t, back, "round-trip mismatch for {:?}", t);
    }

    #[test]
    fn roundtrip_var() {
        roundtrip(Term::Var(0));
        roundtrip(Term::Var(7));
    }

    #[test]
    fn roundtrip_universe() {
        roundtrip(Term::Universe(0));
        roundtrip(Term::Universe(42));
    }

    #[test]
    fn roundtrip_pi() {
        roundtrip(Term::pi(Term::Universe(0), Term::Var(0)));
        roundtrip(Term::pi(
            Term::pi(Term::Universe(0), Term::Var(0)),
            Term::Universe(1),
        ));
    }

    #[test]
    fn roundtrip_lam() {
        roundtrip(Term::lam(Term::Universe(0), Term::Var(0)));
        roundtrip(Term::lam(
            Term::Universe(0),
            Term::lam(Term::Var(0), Term::Var(1)),
        ));
    }

    #[test]
    fn roundtrip_app() {
        roundtrip(Term::app(Term::Var(0), Term::Var(1)));
        roundtrip(Term::app(
            Term::lam(Term::Universe(0), Term::Var(0)),
            Term::Universe(0),
        ));
    }

    #[test]
    fn roundtrip_polymorphic_identity() {
        // λ(A : Type). λ(x : A). x --- the polymorphic identity.
        let poly_id = Term::lam(
            Term::Universe(0),
            Term::lam(Term::Var(0), Term::Var(0)),
        );
        roundtrip(poly_id);
    }

    // -------------------------------------------------------------------------
    // Well-formedness — accept legal, reject out-of-range de Bruijn
    // -------------------------------------------------------------------------

    #[test]
    fn well_formed_closed_universe() {
        let j = ReflectedJudgment::closed(
            ReflectedTerm::Universe { level: 0 },
            ReflectedTerm::Universe { level: 1 },
        );
        assert!(is_reflected_well_formed(&j));
    }

    #[test]
    fn well_formed_var_in_range() {
        // depth 2, Var(0) and Var(1) both legal.
        let j = ReflectedJudgment::at_depth(
            2,
            ReflectedTerm::Var { index: 1 },
            ReflectedTerm::Var { index: 0 },
        );
        assert!(is_reflected_well_formed(&j));
    }

    #[test]
    fn malformed_var_out_of_range() {
        // depth 1, Var(2) is out of range.
        let j = ReflectedJudgment::at_depth(
            1,
            ReflectedTerm::Var { index: 2 },
            ReflectedTerm::Universe { level: 0 },
        );
        assert!(!is_reflected_well_formed(&j));
    }

    #[test]
    fn malformed_var_out_of_range_in_type() {
        let j = ReflectedJudgment::at_depth(
            1,
            ReflectedTerm::Universe { level: 0 },
            ReflectedTerm::Var { index: 5 },
        );
        assert!(!is_reflected_well_formed(&j));
    }

    #[test]
    fn well_formed_under_binder_legal() {
        // λ. Var(0) — Var(0) refers to the lambda's own binder, so
        // it's legal even at outer depth 0.
        let body = ReflectedTerm::Lam {
            domain: Box::new(ReflectedTerm::Universe { level: 0 }),
            body: Box::new(ReflectedTerm::Var { index: 0 }),
        };
        let j = ReflectedJudgment::closed(
            body,
            ReflectedTerm::Universe { level: 0 },
        );
        assert!(is_reflected_well_formed(&j));
    }

    #[test]
    fn malformed_under_binder_index_too_large() {
        // λ. Var(3) with no surrounding context — Var(3) escapes.
        let body = ReflectedTerm::Lam {
            domain: Box::new(ReflectedTerm::Universe { level: 0 }),
            body: Box::new(ReflectedTerm::Var { index: 3 }),
        };
        let j = ReflectedJudgment::closed(
            body,
            ReflectedTerm::Universe { level: 0 },
        );
        assert!(!is_reflected_well_formed(&j));
    }

    // -------------------------------------------------------------------------
    // reflect_kernel_rule — every name resolves; garbage returns None
    // -------------------------------------------------------------------------

    #[test]
    fn reflect_every_known_rule() {
        for name in KERNEL_RULE_NAMES {
            let rule = reflect_kernel_rule(name)
                .unwrap_or_else(|| panic!("rule {name} should reflect"));
            assert_eq!(&rule.name, name);
        }
    }

    #[test]
    fn reflect_unknown_rule_returns_none() {
        assert!(reflect_kernel_rule("T-Bogus").is_none());
        assert!(reflect_kernel_rule("").is_none());
        assert!(reflect_kernel_rule("t-var").is_none()); // case-sensitive
    }

    #[test]
    fn reflect_rule_premise_arity() {
        // T-Var, T-Univ are axiomatic (zero premises).
        assert_eq!(reflect_kernel_rule("T-Var").unwrap().premises.len(), 0);
        assert_eq!(reflect_kernel_rule("T-Univ").unwrap().premises.len(), 0);
        // T-Pi-Form has two premises (domain + codomain typing).
        assert_eq!(
            reflect_kernel_rule("T-Pi-Form").unwrap().premises.len(),
            2
        );
        // The remaining three rules each have one premise in V0.
        assert_eq!(
            reflect_kernel_rule("T-Lam-Intro").unwrap().premises.len(),
            1
        );
        assert_eq!(
            reflect_kernel_rule("T-App-Elim").unwrap().premises.len(),
            1
        );
        assert_eq!(reflect_kernel_rule("T-Conv").unwrap().premises.len(), 1);
    }

    #[test]
    fn kernel_rule_names_count_matches_proof_checker() {
        // The proof_checker module ships exactly six rules; the
        // KERNEL_RULE_NAMES roster must mirror them.
        assert_eq!(KERNEL_RULE_NAMES.len(), 6);
    }

    // -------------------------------------------------------------------------
    // Serde round-trip
    // -------------------------------------------------------------------------

    #[test]
    fn serde_roundtrip_judgment() {
        let j = ReflectedJudgment::closed(
            ReflectedTerm::Lam {
                domain: Box::new(ReflectedTerm::Universe { level: 0 }),
                body: Box::new(ReflectedTerm::Var { index: 0 }),
            },
            ReflectedTerm::Pi {
                domain: Box::new(ReflectedTerm::Universe { level: 0 }),
                body: Box::new(ReflectedTerm::Universe { level: 0 }),
            },
        );
        let json = serde_json::to_string(&j).expect("serialise");
        let back: ReflectedJudgment =
            serde_json::from_str(&json).expect("deserialise");
        assert_eq!(j, back);
    }

    #[test]
    fn serde_roundtrip_kernel_rule() {
        for name in KERNEL_RULE_NAMES {
            let rule = reflect_kernel_rule(name).unwrap();
            let json = serde_json::to_string(&rule).expect("serialise");
            let back: ReflectedKernelRule =
                serde_json::from_str(&json).expect("deserialise");
            assert_eq!(rule, back);
        }
    }

    #[test]
    fn serde_tag_is_snake_case_kind() {
        // Pin the wire format: ReflectedTerm uses tag = "kind"
        // with snake_case constructor names. Audit-gate / external
        // tooling may key on this format, so it's stable surface.
        let t = ReflectedTerm::Var { index: 7 };
        let json = serde_json::to_string(&t).expect("serialise");
        assert!(
            json.contains("\"kind\":\"var\""),
            "expected snake_case kind tag, got {json}"
        );
    }

    // -------------------------------------------------------------------------
    // Reflection error type — basic coverage
    // -------------------------------------------------------------------------

    #[test]
    fn reflection_error_display() {
        let err = ReflectionError::DeBruijnOutOfRange { index: 5, max: 2 };
        let s = format!("{err}");
        assert!(s.contains('5'));
        assert!(s.contains('2'));
    }

    #[test]
    fn reflection_error_unknown_constructor() {
        let err = ReflectionError::UnknownConstructor("Future".to_string());
        let s = format!("{err}");
        assert!(s.contains("Future"));
    }
}
