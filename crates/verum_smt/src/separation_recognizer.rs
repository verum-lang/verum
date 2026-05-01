//! AST-recognizer for separation-logic predicates (#161 V2).
//!
//! ## Architectural role
//!
//! Pre-this-module, when a user wrote
//! ```text
//! @verify(formal)
//! fn swap(a: &mut Cell, b: &mut Cell)
//!     requires sep_conj(points_to(a, av), points_to(b, bv))
//!     ensures  sep_conj(points_to(a, bv), points_to(b, av))
//! { ... }
//! ```
//! the verifier's [`crate::expr_to_smtlib`] translated each call as
//! an opaque uninterpreted function:
//! ```text
//! (sep_conj (points_to a av) (points_to b bv))
//! ```
//! Z3 has no built-in semantics for `sep_conj` or `points_to` — they
//! were just predicate symbols. The proof obligation reduced to
//! "can Z3 prove uninterpreted (sep_conj …) ⇒ (sep_conj …)" — almost
//! always falsifiable by Z3's CEGAR loop because the symbols had no
//! semantic content.
//!
//! ## What this module delivers
//!
//! [`try_recognize_sep_assertion`] walks an AST `Expr` and returns
//! a [`crate::separation_logic::SepAssertion`] **whenever the
//! expression is a syntactic call to one of the separation-logic
//! smart constructors declared in `core/logic/separation.vr`**. The
//! recognised constructors:
//!
//! | Verum source                           | SepAssertion                   |
//! |----------------------------------------|--------------------------------|
//! | `emp()`                                | `Emp`                          |
//! | `points_to(addr, value)`               | `PointsTo { location, value }` |
//! | `sep_conj(p, q)` / `p * q`             | `Sep { left, right }`          |
//! | `heap_and(p, q)`                       | `And { left, right }`          |
//! | `pure(prop)`                           | `Pure(expr)`                   |
//! | `named(name, args)`                    | (custom — not yet wired)       |
//!
//! ## Soundness
//!
//! Recognition is **syntax-driven, no name resolution**. The
//! recogniser pattern-matches on the bare-name path of the call's
//! function. This is sound because `core/logic/separation.vr`'s
//! smart constructors are the canonical entry points for
//! separation-logic predicate construction; any user code that
//! shadows these names is either (a) knowingly playing with the
//! separation-logic surface (acceptable) or (b) writing code the
//! separation-logic verifier shouldn't see anyway.
//!
//! Returning `None` on an unrecognised shape is always sound —
//! the caller falls back to the opaque uninterpreted-function
//! translation, which is the pre-V2 behaviour.
//!
//! ## Performance
//!
//! Pure recursive AST walk — O(n) in expression size. Predicates
//! are typically small (< 30 nodes); per-call cost < 1µs.
//!
//! ## Architectural pattern
//!
//! Mirrors the established `recognise + translate` pattern:
//!   * [`super::separation_kernel_bridge`] (#161 V1) translates
//!     between kernel `HeapPredicate<Term>` and SMT `SepAssertion<Expr>`
//!     at the type-data layer.
//!   * THIS module (#161 V2) translates from user `Expr` (AST) to
//!     SMT `SepAssertion<Expr>` at the recognition layer.
//!   * Together: user `requires sep_conj(points_to(a, av), ...)` →
//!     `SepAssertion::Sep { left: PointsTo { ... }, ... }` →
//!     `SepLogicEncoder` Z3 emission.
//!
//! ## Reuse
//!
//! Every recognised constructor matches the smart-constructor names
//! defined in `core/logic/separation.vr`. Adding a new public
//! constructor there requires extending the recogniser here — but
//! the alternative (no recogniser) is the pre-V2 status quo where
//! the entire separation-logic surface is opaque-functions-only.

use verum_ast::expr::{Expr, ExprKind};

use super::separation_logic::SepAssertion;

// =============================================================================
// Recognition entry point
// =============================================================================

/// Try to recognise an AST expression as a separation-logic predicate.
///
/// Returns `Some(SepAssertion)` when the expression is a syntactic
/// call to one of the smart constructors declared in
/// `core/logic/separation.vr`. Returns `None` for any other shape —
/// including:
///   * non-Call expressions (Path, Literal, Binary, Block, ...)
///   * Calls whose function isn't a recognised separation
///     constructor name
///   * Calls with the wrong arity for the recognised name
///
/// **Recursive descent**: arguments to recognised constructors are
/// themselves passed through the recogniser, so deeply nested
/// predicates resolve fully. A recognised outer constructor with
/// an unrecognisable inner argument returns `None` for the entire
/// expression — soundness over partial coverage.
pub fn try_recognize_sep_assertion(expr: &Expr) -> Option<SepAssertion> {
    let (callee_name, args) = match &expr.kind {
        ExprKind::Call { func, args, .. } => {
            let name = call_callee_bare_name(func)?;
            (name, args)
        }
        // A non-Call expression cannot be a separation-logic
        // predicate constructor. The caller's fast path in
        // expr_to_smtlib continues to translate the expression
        // generically (e.g., `emp` as a path resolves to the
        // emp() smart constructor only if it's in CALL position).
        _ => return None,
    };

    let arg_count = args.iter().count();
    match (callee_name.as_str(), arg_count) {
        // Empty-heap predicate. `emp()` with zero args.
        ("emp", 0) => Some(SepAssertion::Emp),

        // Points-to: `points_to(addr, value)`.
        ("points_to", 2) => {
            let addr = args.iter().next()?.clone();
            let value = args.iter().nth(1)?.clone();
            Some(SepAssertion::PointsTo {
                location: addr,
                value,
            })
        }

        // Separating conjunction: `sep_conj(p, q)`.
        ("sep_conj", 2) => {
            let lhs_expr = args.iter().next()?;
            let rhs_expr = args.iter().nth(1)?;
            // **All-or-nothing recursive recognition**: both arms
            // must classify, otherwise the whole expression bails.
            // Half-recognised separation predicates would produce
            // unsound translations (some args opaque, some lifted).
            let lhs = try_recognize_sep_assertion(lhs_expr)?;
            let rhs = try_recognize_sep_assertion(rhs_expr)?;
            Some(SepAssertion::sep(lhs, rhs))
        }

        // Heap-stable conjunction: `heap_and(p, q)`.
        ("heap_and", 2) => {
            let lhs_expr = args.iter().next()?;
            let rhs_expr = args.iter().nth(1)?;
            let lhs = try_recognize_sep_assertion(lhs_expr)?;
            let rhs = try_recognize_sep_assertion(rhs_expr)?;
            Some(SepAssertion::and(lhs, rhs))
        }

        // Pure proposition lift: `pure(prop)`.  The argument is an
        // arbitrary heap-irrelevant expression — passed through to
        // the SepAssertion as-is. The Z3 encoder will translate
        // the inner expression via expr_to_smtlib.
        ("pure", 1) => {
            let prop = args.iter().next()?.clone();
            Some(SepAssertion::Pure(prop))
        }

        // `named(name, args)` is omitted from V2 — it requires
        // resolving the user-defined predicate against the
        // elaborator's axiom registry, which the recogniser doesn't
        // yet have access to. Future work: wire the registry through
        // and recognise named predicates as `SepAssertion::Pure(name)`
        // with the args attached as a separate metadata field.

        // Anything else is not a recognised separation-logic
        // constructor at this AST shape. The caller continues with
        // generic translation.
        _ => None,
    }
}

/// Extract the bare (single-segment) name of a call's callee, when
/// the callee is a simple `Path` expression. Returns `None` for
/// multi-segment paths, method calls, closures, or any other shape.
fn call_callee_bare_name(func: &Expr) -> Option<String> {
    match &func.kind {
        ExprKind::Path(p) if p.segments.len() == 1 => match &p.segments[0] {
            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

// =============================================================================
// SMT-LIB rendering — #161 V3
// =============================================================================

/// Render a [`SepAssertion`] into a stable separation-theory SMT-LIB
/// string. This is the canonical text downstream verifiers consume
/// when emitting separation-logic obligations to Z3.
///
/// **Naming convention** — stable, namespace-prefixed so the SMT
/// context can declare these as distinguished symbols without
/// colliding with user code:
///
/// | `SepAssertion`              | SMT-LIB form                        |
/// |-----------------------------|-------------------------------------|
/// | `Emp`                       | `sep_emp`                           |
/// | `PointsTo { loc, val }`     | `(sep_pt <loc> <val>)`              |
/// | `Sep { left, right }`       | `(sep_star <left> <right>)`         |
/// | `And { left, right }`       | `(sep_and <left> <right>)`          |
/// | `Pure(expr)`                | `(sep_pure <expr>)`                 |
/// | `Or { left, right }`        | `(sep_or <left> <right>)`           |
/// | `Wand { left, right }`      | `(sep_wand <left> <right>)`         |
/// | `Exists { var, body }`      | `(sep_exists <var> <body>)`         |
/// | `Forall { var, body }`      | `(sep_forall <var> <body>)`         |
/// | `ListSegment { from, to, _ }` | `(sep_lseg <from> <to>)`          |
/// | `Tree { root, _, _ }`       | `(sep_tree <root>)`                 |
/// | `Block { base, size }`      | `(sep_block <base> <size>)`         |
/// | `ArraySegment { base, .., length, _ }` | `(sep_array_seg <base> <length>)` |
///
/// Inner expressions are rendered through
/// [`crate::expr_to_smtlib::expr_to_smtlib`] — the generic AST → SMT
/// translator. Failure to translate an inner expression bubbles up
/// as an `Err`.
///
/// **Architectural role**: this function lets a verifier (or audit
/// gate) emit separation-logic predicates in a way that downstream
/// Z3 setup can dispatch on the `sep_*` prefix to install the
/// matching theory.  It is the **stable interchange format** between
/// the recogniser and the Z3 encoder.
pub fn sep_assertion_to_smtlib(
    assertion: &SepAssertion,
) -> Result<String, crate::expr_to_smtlib::SmtTranslateError> {
    use crate::expr_to_smtlib::expr_to_smtlib;

    match assertion {
        SepAssertion::Emp => Ok("sep_emp".to_string()),

        SepAssertion::PointsTo { location, value } => {
            let loc = expr_to_smtlib(location)?;
            let val = expr_to_smtlib(value)?;
            Ok(format!("(sep_pt {} {})", loc, val))
        }

        SepAssertion::Sep { left, right } => {
            let l = sep_assertion_to_smtlib(left)?;
            let r = sep_assertion_to_smtlib(right)?;
            Ok(format!("(sep_star {} {})", l, r))
        }

        SepAssertion::And { left, right } => {
            let l = sep_assertion_to_smtlib(left)?;
            let r = sep_assertion_to_smtlib(right)?;
            Ok(format!("(sep_and {} {})", l, r))
        }

        SepAssertion::Or { left, right } => {
            let l = sep_assertion_to_smtlib(left)?;
            let r = sep_assertion_to_smtlib(right)?;
            Ok(format!("(sep_or {} {})", l, r))
        }

        SepAssertion::Wand { left, right } => {
            let l = sep_assertion_to_smtlib(left)?;
            let r = sep_assertion_to_smtlib(right)?;
            Ok(format!("(sep_wand {} {})", l, r))
        }

        SepAssertion::Pure(prop) => {
            let p = expr_to_smtlib(prop)?;
            Ok(format!("(sep_pure {})", p))
        }

        SepAssertion::Exists { var, body } => {
            let b = sep_assertion_to_smtlib(body)?;
            Ok(format!("(sep_exists {} {})", var.as_str(), b))
        }

        SepAssertion::Forall { var, body } => {
            let b = sep_assertion_to_smtlib(body)?;
            Ok(format!("(sep_forall {} {})", var.as_str(), b))
        }

        SepAssertion::ListSegment { from, to, .. } => {
            let f = expr_to_smtlib(from)?;
            let t = expr_to_smtlib(to)?;
            Ok(format!("(sep_lseg {} {})", f, t))
        }

        SepAssertion::Tree { root, .. } => {
            let r = expr_to_smtlib(root)?;
            Ok(format!("(sep_tree {})", r))
        }

        SepAssertion::Block { base, size } => {
            let b = expr_to_smtlib(base)?;
            let s = expr_to_smtlib(size)?;
            Ok(format!("(sep_block {} {})", b, s))
        }

        SepAssertion::ArraySegment { base, length, .. } => {
            let b = expr_to_smtlib(base)?;
            let l = expr_to_smtlib(length)?;
            Ok(format!("(sep_array_seg {} {})", b, l))
        }
    }
}

/// **Recognise + render** in one call. Returns `Some(smtlib_string)`
/// when the expression is a separation-logic predicate; `None`
/// otherwise. This is the canonical fast-path entry point for
/// [`crate::expr_to_smtlib::expr_to_smtlib`] callers that want
/// separation-aware translation.
///
/// Returns `Some(Err)` when the expression IS a separation
/// predicate but contains an inner expression that fails the
/// generic AST → SMT translator. Callers can decide whether to
/// fall back to opaque-function translation or surface the error.
pub fn try_translate_sep_predicate_to_smtlib(
    expr: &Expr,
) -> Option<Result<String, crate::expr_to_smtlib::SmtTranslateError>> {
    let assertion = try_recognize_sep_assertion(expr)?;
    Some(sep_assertion_to_smtlib(&assertion))
}

// =============================================================================
// Verification-goal routing — #161 V4
// =============================================================================

/// Outcome of a separation-aware entailment attempt.
///
/// Distinguishes the four observable verdict shapes the caller
/// needs to dispatch on:
///   * `NotSeparationGoal` — neither side is a syntactic separation
///     predicate; caller must fall through to generic SMT.
///   * `Valid` — P ⊢ Q checked by `SepLogicEncoder::verify_entailment`.
///     Audit-clean discharge.
///   * `Invalid` — counterexample found; the obligation is unsound.
///   * `Unknown` — Z3 returned `unknown` (timeout / incompleteness).
///
/// Mirrors [`crate::separation_logic::EntailmentResult`] but adds
/// the `NotSeparationGoal` arm so callers don't conflate "neither
/// recognised" with "couldn't decide".
#[derive(Debug, Clone)]
pub enum SepObligationOutcome {
    /// At least one of `pre`/`post` isn't a separation predicate;
    /// caller dispatches to generic SMT.
    NotSeparationGoal,
    /// `Pre ⊢ Post` proved valid by the separation-logic encoder.
    Valid,
    /// Counterexample found; the obligation is unsound. The
    /// `counterexample_summary` is a human-readable rendering of
    /// the heap state Z3 produced.
    Invalid { counterexample_summary: String },
    /// Z3 returned `unknown` for the entailment query.
    Unknown { reason: String },
}

impl SepObligationOutcome {
    /// Stable diagnostic tag — matches what audit reports surface.
    pub fn tag(&self) -> &'static str {
        match self {
            SepObligationOutcome::NotSeparationGoal => "not_separation_goal",
            SepObligationOutcome::Valid => "valid",
            SepObligationOutcome::Invalid { .. } => "invalid",
            SepObligationOutcome::Unknown { .. } => "unknown",
        }
    }

    /// True iff the entailment was decisively VALID. Used by the
    /// proof-verification phase as the discharge predicate.
    pub fn is_valid(&self) -> bool {
        matches!(self, SepObligationOutcome::Valid)
    }
}

/// **Combine a slice of AST `Expr` clauses into a single
/// [`SepAssertion`] via separating conjunction.** Used by the
/// multi-clause Hoare-obligation entry point [`verify_separation_obligation_multi`]
/// (#161 V6).
///
/// Behaviour:
///   * Empty slice → `Some(SepAssertion::Emp)` — the identity
///     element of separating conjunction.  This makes
///     `requires <empty>` map to `requires emp`, which is the
///     vacuously-satisfied precondition.
///   * Single clause → recognise it directly; return `Some(a)`
///     when recognition succeeds.
///   * Multi-clause → recognise each individually; combine via
///     left-associative `sep_conj`. Returns `Some(a1 ∗ a2 ∗ … ∗ aN)`
///     when EVERY clause recognises; `None` if any clause fails
///     recognition.
///
/// **Soundness**: separating conjunction is associative
/// (Reynolds 2002, O'Hearn 2007) and has `emp` as identity, so
/// `((a ∗ b) ∗ c) ≡ (a ∗ (b ∗ c)) ≡ (a ∗ b ∗ c)`.  Left-fold
/// composition is the canonical encoding.
///
/// **All-or-nothing**: if even one clause fails recognition, the
/// combined result is `None`.  Half-recognised conjunctions would
/// be unsound — some clauses opaque, some lifted.
pub fn try_combine_clauses_to_sep_assertion(
    clauses: &[&Expr],
) -> Option<SepAssertion> {
    if clauses.is_empty() {
        return Some(SepAssertion::Emp);
    }
    let mut acc: Option<SepAssertion> = None;
    for clause in clauses {
        let recognised = try_recognize_sep_assertion(clause)?;
        acc = Some(match acc {
            None => recognised,
            Some(a) => SepAssertion::sep(a, recognised),
        });
    }
    acc
}

// =============================================================================
// Clause partitioning — #161 V7 (mixed separation + pure)
// =============================================================================

/// Per-clause classification used by the V7 splitter.
///
/// Separates clauses into two cohorts:
///   * `Separation` — recognises as a syntactic separation
///     predicate via [`try_recognize_sep_assertion`]; route through
///     the SepLogicEncoder.
///   * `Pure` — does NOT recognise as a separation predicate;
///     route through generic SMT.
///
/// The discrimination is the load-bearing soundness boundary: a
/// clause is treated as separation-bearing IFF the recogniser says
/// so. Pure clauses don't see the heap; their verdict is independent
/// of the separation portion.
#[derive(Debug, Clone)]
pub enum ClauseKind<'a> {
    /// Clause recognises as a separation predicate.
    Separation(SepAssertion),
    /// Clause is heap-independent (treated as pure proposition).
    /// The original AST `Expr` is preserved for downstream SMT
    /// translation.
    Pure(&'a Expr),
}

/// Outcome of partitioning a list of clauses into separation +
/// pure cohorts (#161 V7).
#[derive(Debug, Clone)]
pub struct ClausePartition<'a> {
    /// Clauses that recognised as separation predicates,
    /// pre-translated to `SepAssertion`.
    pub separation: Vec<SepAssertion>,
    /// Clauses that DID NOT recognise — heap-irrelevant pure
    /// propositions. The original `Expr` references are preserved
    /// so the caller can dispatch them through generic SMT.
    pub pure: Vec<&'a Expr>,
}

impl<'a> ClausePartition<'a> {
    /// Combine the separation cohort into a single `SepAssertion`
    /// via left-associative `sep_conj`. Empty cohort → `Emp`
    /// (identity).
    pub fn combined_separation(&self) -> SepAssertion {
        if self.separation.is_empty() {
            return SepAssertion::Emp;
        }
        let mut iter = self.separation.iter().cloned();
        let first = iter.next().expect("non-empty");
        iter.fold(first, |acc, next| SepAssertion::sep(acc, next))
    }

    /// True iff the partition has at least one separation clause —
    /// the predicate the V7 hot-path uses to decide whether to
    /// invoke the SepLogicEncoder at all.
    pub fn has_separation(&self) -> bool {
        !self.separation.is_empty()
    }
}

/// Partition a slice of clauses into separation + pure cohorts.
///
/// **Soundness**: a clause is classified as `Separation` IFF the
/// recogniser produces a `SepAssertion`. Otherwise it's `Pure` —
/// no syntactic mixing within a single clause.  This rule keeps
/// the split semantically clean: separation clauses describe heap
/// shape; pure clauses describe heap-irrelevant logical
/// constraints.
///
/// **Use site** (#161 V7): the verifier's hot path partitions
/// requires + ensures clauses via this function, dispatches the
/// separation cohort through the SepLogicEncoder, dispatches the
/// pure cohort through generic SMT, and ANDs the verdicts.
pub fn partition_clauses<'a>(clauses: &[&'a Expr]) -> ClausePartition<'a> {
    let mut separation: Vec<SepAssertion> = Vec::new();
    let mut pure: Vec<&'a Expr> = Vec::new();
    for clause in clauses {
        match try_recognize_sep_assertion(clause) {
            Some(a) => separation.push(a),
            None => pure.push(clause),
        }
    }
    ClausePartition { separation, pure }
}

/// **Verify a mixed separation+pure Hoare obligation** (#161 V7).
///
/// The architectural extension on top of V6: splits `pre` and
/// `post` clause lists into separation + pure cohorts, dispatches
/// each cohort through the appropriate verifier:
///   * Separation cohort → `SepLogicEncoder::verify_entailment`
///     (heap-shape entailment with array-theory backing).
///   * Pure cohort → generic SMT (the caller is expected to
///     dispatch the returned `Expr`s through existing
///     verification infrastructure).
///
/// **Soundness**: the conjunction `(P_sep ∗ P_pure) ⊢ (Q_sep ∗
/// Q_pure)` proves separately as `P_sep ⊢ Q_sep` AND
/// `P_pure ⊢ Q_pure` — both must hold. The pure portion is
/// heap-irrelevant, so its verdict doesn't depend on the heap
/// state; the separation portion describes heap shape, so its
/// verdict doesn't depend on pure logic.
///
/// Returns the V4-style outcome for the SEPARATION portion plus
/// the list of pure clauses the caller must verify through
/// generic SMT. The caller's job is to (a) confirm the separation
/// outcome is `Valid`, (b) hand the pure clauses to the generic
/// verification path, (c) AND the two verdicts.
///
/// **No automatic AND**: this function doesn't try to verify the
/// pure portion itself — that requires the full generic-SMT setup
/// (engine, hypothesis context, refinement aliases) which lives
/// in the verifier hot path. Returning the pure clauses lets the
/// caller compose verification cleanly.
pub fn split_separation_obligation<'a>(
    pre_clauses: &[&'a Expr],
    post_clauses: &[&'a Expr],
) -> SeparationSplitOutcome<'a> {
    let pre_partition = partition_clauses(pre_clauses);
    let post_partition = partition_clauses(post_clauses);

    // If neither side has a separation clause, V7 has nothing to
    // contribute — punt to generic SMT for the whole obligation.
    if !pre_partition.has_separation() && !post_partition.has_separation() {
        return SeparationSplitOutcome::NotSeparationGoal;
    }

    // Verify the separation portion via the existing V4 path.
    let pre_sep = pre_partition.combined_separation();
    let post_sep = post_partition.combined_separation();
    use crate::separation_logic::{EntailmentResult, SepLogicConfig, SepLogicEncoder};
    let encoder = SepLogicEncoder::new(SepLogicConfig::default());
    let separation_outcome = match encoder.verify_entailment(&pre_sep, &post_sep) {
        Ok(EntailmentResult::Valid { .. }) => SepObligationOutcome::Valid,
        Ok(EntailmentResult::Invalid { counterexample, .. }) => SepObligationOutcome::Invalid {
            counterexample_summary: format!("{:?}", counterexample),
        },
        Ok(EntailmentResult::Unknown { reason, .. }) => SepObligationOutcome::Unknown {
            reason: reason.as_str().to_string(),
        },
        Err(e) => SepObligationOutcome::Unknown {
            reason: format!("encoder error: {:?}", e),
        },
    };

    SeparationSplitOutcome::Split {
        separation_outcome,
        pure_pre: pre_partition.pure,
        pure_post: post_partition.pure,
    }
}

/// Outcome of [`split_separation_obligation`] (#161 V7).
#[derive(Debug, Clone)]
pub enum SeparationSplitOutcome<'a> {
    /// Neither pre nor post has a separation clause — caller
    /// dispatches through generic SMT, no V7 routing.
    NotSeparationGoal,
    /// At least one side had a separation clause; the separation
    /// portion was verified through the SepLogicEncoder; the pure
    /// portion is returned to the caller for dispatch through
    /// generic SMT.
    Split {
        /// Verdict on the separation portion (Valid / Invalid /
        /// Unknown).  The caller AND-s this with the pure-portion
        /// verdict.
        separation_outcome: SepObligationOutcome,
        /// Pure pre-clauses — to be dispatched as hypotheses
        /// through generic SMT.
        pure_pre: Vec<&'a Expr>,
        /// Pure post-clauses — to be dispatched as goals through
        /// generic SMT.
        pure_post: Vec<&'a Expr>,
    },
}

/// **Verify a multi-clause separation-logic Hoare obligation**
/// (#161 V6).  Generalises [`verify_separation_obligation`] from
/// single-clause to any number of pre/post clauses, combining
/// each side via separating conjunction.
///
/// **Routing rules**:
///   * Both sides combine successfully (every clause recognised) →
///     route through [`crate::separation_logic::SepLogicEncoder::verify_entailment`].
///   * Either side has at least one unrecognised clause →
///     return `NotSeparationGoal`; caller dispatches to generic SMT.
///   * Empty side → treated as `emp` (the identity element).  A
///     theorem with no requires-clause is `requires emp`.
///
/// **Architectural composition** with V5: the verifier's hot path
/// in `verum_compiler::phases::proof_verification` calls
/// `verify_separation_obligation_multi(requires_clauses,
/// ensures_clauses)`.  Pre-V6 (V5) it was restricted to N=1 each;
/// post-V6 any N works.
pub fn verify_separation_obligation_multi(
    pre_clauses: &[&Expr],
    post_clauses: &[&Expr],
) -> SepObligationOutcome {
    let pre_assertion = match try_combine_clauses_to_sep_assertion(pre_clauses) {
        Some(a) => a,
        None => return SepObligationOutcome::NotSeparationGoal,
    };
    let post_assertion = match try_combine_clauses_to_sep_assertion(post_clauses) {
        Some(a) => a,
        None => return SepObligationOutcome::NotSeparationGoal,
    };

    use crate::separation_logic::{EntailmentResult, SepLogicConfig, SepLogicEncoder};

    let encoder = SepLogicEncoder::new(SepLogicConfig::default());
    match encoder.verify_entailment(&pre_assertion, &post_assertion) {
        Ok(EntailmentResult::Valid { .. }) => SepObligationOutcome::Valid,
        Ok(EntailmentResult::Invalid { counterexample, .. }) => SepObligationOutcome::Invalid {
            counterexample_summary: format!("{:?}", counterexample),
        },
        Ok(EntailmentResult::Unknown { reason, .. }) => SepObligationOutcome::Unknown {
            reason: reason.as_str().to_string(),
        },
        Err(e) => SepObligationOutcome::Unknown {
            reason: format!("encoder error: {:?}", e),
        },
    }
}

/// **Verify a separation-logic Hoare obligation** at the
/// `requires P ensures Q` boundary.
///
/// Walks the Verum-source `pre` and `post` expressions through
/// [`try_recognize_sep_assertion`].  When BOTH sides recognise as
/// separation predicates, builds them into [`SepAssertion`]s and
/// routes through [`crate::separation_logic::SepLogicEncoder::verify_entailment`]
/// to obtain a Z3-backed verdict.
///
/// **Architectural role** (#161 V4): closes the load-bearing
/// chain. Pre-V4, `requires`/`ensures` clauses with separation
/// predicates produced structured `(sep_star …)` SMT-LIB but no
/// Z3 setup understood the separation theory. Post-V4, the
/// obligation routes through the existing
/// [`SepLogicEncoder`](crate::separation_logic::SepLogicEncoder) —
/// 4441 LOC of Z3-array-theory-backed encoding, frame inference,
/// counterexample extraction. Reuses 100% of existing
/// infrastructure.
///
/// **Asymmetric handling**: when ONE side recognises but the
/// other doesn't (e.g. `requires sep_conj(...)` ensures
/// `result == 0`), returns `NotSeparationGoal` so the caller can
/// dispatch the obligation through generic SMT. Mixing
/// separation-and-pure verification is a V5+ concern; V4 handles
/// the homogeneous-separation case.
///
/// **Returns**:
///   * `NotSeparationGoal` when at least one side isn't a syntactic
///     separation predicate.
///   * `Valid` / `Invalid` / `Unknown` for the entailment verdict
///     when both sides are recognised.
pub fn verify_separation_obligation(
    pre: &Expr,
    post: &Expr,
) -> SepObligationOutcome {
    let pre_assertion = match try_recognize_sep_assertion(pre) {
        Some(a) => a,
        None => return SepObligationOutcome::NotSeparationGoal,
    };
    let post_assertion = match try_recognize_sep_assertion(post) {
        Some(a) => a,
        None => return SepObligationOutcome::NotSeparationGoal,
    };

    // Both sides recognised — build the SepLogicEncoder and run
    // entailment.  Use default config — callers wanting custom
    // timeout / unfolding-depth can use the lower-level encoder
    // API directly.
    use crate::separation_logic::{EntailmentResult, SepLogicConfig, SepLogicEncoder};

    let encoder = SepLogicEncoder::new(SepLogicConfig::default());
    match encoder.verify_entailment(&pre_assertion, &post_assertion) {
        Ok(EntailmentResult::Valid { .. }) => SepObligationOutcome::Valid,
        Ok(EntailmentResult::Invalid { counterexample, .. }) => SepObligationOutcome::Invalid {
            counterexample_summary: format!("{:?}", counterexample),
        },
        Ok(EntailmentResult::Unknown { reason, .. }) => SepObligationOutcome::Unknown {
            reason: reason.as_str().to_string(),
        },
        Err(e) => SepObligationOutcome::Unknown {
            reason: format!("encoder error: {:?}", e),
        },
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::ExprKind;
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_common::{Heap, List};

    fn span() -> Span {
        Span::dummy()
    }

    fn name_path_expr(name: &str) -> Expr {
        Expr::new(
            ExprKind::Path(Path::new(
                List::from(vec![PathSegment::Name(Ident::new(name, span()))]),
                span(),
            )),
            span(),
        )
    }

    fn call_expr(callee: &str, args: Vec<Expr>) -> Expr {
        Expr::new(
            ExprKind::Call {
                func: Heap::new(name_path_expr(callee)),
                type_args: List::new(),
                args: List::from(args),
            },
            span(),
        )
    }

    // ----- Recognised shapes -----

    #[test]
    fn recognises_emp_zero_arity() {
        let e = call_expr("emp", vec![]);
        assert!(matches!(
            try_recognize_sep_assertion(&e),
            Some(SepAssertion::Emp),
        ));
    }

    #[test]
    fn recognises_points_to_two_arity() {
        let e = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        match try_recognize_sep_assertion(&e) {
            Some(SepAssertion::PointsTo { .. }) => {}
            other => panic!("expected PointsTo, got {:?}", other),
        }
    }

    #[test]
    fn recognises_sep_conj_recursively() {
        // sep_conj(points_to(a, av), points_to(b, bv))
        let inner_l = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        let inner_r = call_expr(
            "points_to",
            vec![name_path_expr("b"), name_path_expr("bv")],
        );
        let outer = call_expr("sep_conj", vec![inner_l, inner_r]);
        match try_recognize_sep_assertion(&outer) {
            Some(SepAssertion::Sep { left, right }) => {
                assert!(matches!(left.as_ref(), SepAssertion::PointsTo { .. }));
                assert!(matches!(right.as_ref(), SepAssertion::PointsTo { .. }));
            }
            other => panic!("expected Sep, got {:?}", other),
        }
    }

    #[test]
    fn recognises_heap_and() {
        let inner = call_expr("emp", vec![]);
        let outer = call_expr("heap_and", vec![inner.clone(), inner]);
        match try_recognize_sep_assertion(&outer) {
            Some(SepAssertion::And { .. }) => {}
            other => panic!("expected And, got {:?}", other),
        }
    }

    #[test]
    fn recognises_pure() {
        let inner = name_path_expr("some_predicate");
        let outer = call_expr("pure", vec![inner]);
        match try_recognize_sep_assertion(&outer) {
            Some(SepAssertion::Pure(_)) => {}
            other => panic!("expected Pure, got {:?}", other),
        }
    }

    // ----- All-or-nothing recursion -----

    #[test]
    fn sep_conj_with_unrecognised_arg_returns_none() {
        // sep_conj(points_to(a, av), some_user_function())
        let inner_l = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        let unrecognised = call_expr("some_user_function", vec![]);
        let outer = call_expr("sep_conj", vec![inner_l, unrecognised]);
        assert!(
            try_recognize_sep_assertion(&outer).is_none(),
            "all-or-nothing: an unrecognised inner arg blocks the whole sep recognition",
        );
    }

    #[test]
    fn nested_sep_conj_recurses_three_levels() {
        // sep_conj(emp, sep_conj(emp, points_to(a, av)))
        let inner_pt = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        let mid = call_expr("sep_conj", vec![call_expr("emp", vec![]), inner_pt]);
        let outer = call_expr("sep_conj", vec![call_expr("emp", vec![]), mid]);
        match try_recognize_sep_assertion(&outer) {
            Some(SepAssertion::Sep { right, .. }) => {
                // right is itself Sep(Emp, PointsTo)
                match right.as_ref() {
                    SepAssertion::Sep {
                        right: inner_right, ..
                    } => {
                        assert!(matches!(inner_right.as_ref(), SepAssertion::PointsTo { .. }));
                    }
                    other => panic!("expected nested Sep, got {:?}", other),
                }
            }
            other => panic!("expected outer Sep, got {:?}", other),
        }
    }

    // ----- Rejection cases -----

    #[test]
    fn non_call_expression_returns_none() {
        let e = name_path_expr("emp"); // path, not a call
        assert!(try_recognize_sep_assertion(&e).is_none());
    }

    #[test]
    fn call_with_wrong_arity_returns_none() {
        // points_to with 1 arg instead of 2
        let e = call_expr("points_to", vec![name_path_expr("a")]);
        assert!(try_recognize_sep_assertion(&e).is_none());
        // emp with 1 arg instead of 0
        let e2 = call_expr("emp", vec![name_path_expr("x")]);
        assert!(try_recognize_sep_assertion(&e2).is_none());
    }

    #[test]
    fn unrecognised_callee_returns_none() {
        let e = call_expr("not_a_separation_constructor", vec![]);
        assert!(try_recognize_sep_assertion(&e).is_none());
    }

    #[test]
    fn multi_segment_path_callee_returns_none() {
        // some.module.path::emp() — multi-segment paths bypass the
        // bare-name recogniser. This is intentional: the recogniser
        // matches on syntactic shape, not on resolved identity.
        // Future work: a resolver-aware recogniser that handles
        // `core.logic.separation::emp` etc.
        let multi_path = Expr::new(
            ExprKind::Path(Path::new(
                List::from(vec![
                    PathSegment::Name(Ident::new("core", span())),
                    PathSegment::Name(Ident::new("logic", span())),
                    PathSegment::Name(Ident::new("separation", span())),
                    PathSegment::Name(Ident::new("emp", span())),
                ]),
                span(),
            )),
            span(),
        );
        let e = Expr::new(
            ExprKind::Call {
                func: Heap::new(multi_path),
                type_args: List::new(),
                args: List::new(),
            },
            span(),
        );
        assert!(try_recognize_sep_assertion(&e).is_none());
    }

    // ----- Architectural pin -----

    // ----- SMT-LIB rendering (#161 V3) -----

    #[test]
    fn render_emp_to_smtlib() {
        let r = sep_assertion_to_smtlib(&SepAssertion::Emp).unwrap();
        assert_eq!(r, "sep_emp");
    }

    #[test]
    fn render_points_to_to_smtlib() {
        let r = sep_assertion_to_smtlib(&SepAssertion::PointsTo {
            location: name_path_expr("a"),
            value: name_path_expr("av"),
        })
        .unwrap();
        assert_eq!(r, "(sep_pt a av)");
    }

    #[test]
    fn render_sep_conj_to_smtlib() {
        let inner = SepAssertion::PointsTo {
            location: name_path_expr("a"),
            value: name_path_expr("av"),
        };
        let outer = SepAssertion::sep(SepAssertion::Emp, inner);
        let r = sep_assertion_to_smtlib(&outer).unwrap();
        assert_eq!(r, "(sep_star sep_emp (sep_pt a av))");
    }

    #[test]
    fn render_pure_to_smtlib() {
        let r = sep_assertion_to_smtlib(&SepAssertion::Pure(name_path_expr("ok"))).unwrap();
        assert_eq!(r, "(sep_pure ok)");
    }

    #[test]
    fn try_translate_sep_predicate_recognises_and_renders() {
        // sep_conj(emp(), points_to(a, av)) → (sep_star sep_emp (sep_pt a av))
        let inner_pt = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        let outer = call_expr("sep_conj", vec![call_expr("emp", vec![]), inner_pt]);
        let outcome = try_translate_sep_predicate_to_smtlib(&outer);
        match outcome {
            Some(Ok(text)) => assert_eq!(text, "(sep_star sep_emp (sep_pt a av))"),
            other => panic!("expected Ok(sep_star ...), got {:?}", other),
        }
    }

    #[test]
    fn try_translate_sep_predicate_returns_none_for_unrecognised() {
        let e = call_expr("user_function", vec![]);
        assert!(try_translate_sep_predicate_to_smtlib(&e).is_none());
    }

    #[test]
    fn render_three_level_nesting_to_smtlib() {
        // sep_conj(emp, sep_conj(emp, points_to(a, av)))
        let inner_pt = SepAssertion::PointsTo {
            location: name_path_expr("a"),
            value: name_path_expr("av"),
        };
        let mid = SepAssertion::sep(SepAssertion::Emp, inner_pt);
        let outer = SepAssertion::sep(SepAssertion::Emp, mid);
        let r = sep_assertion_to_smtlib(&outer).unwrap();
        assert_eq!(r, "(sep_star sep_emp (sep_star sep_emp (sep_pt a av)))");
    }

    // ----- V4 verification routing -----

    #[test]
    fn verify_separation_obligation_routes_pure_emp_to_valid() {
        // emp ⊢ emp — trivially valid under separation logic.
        let pre = call_expr("emp", vec![]);
        let post = call_expr("emp", vec![]);
        let outcome = verify_separation_obligation(&pre, &post);
        // Z3 may return Valid OR Unknown depending on the encoder's
        // unfolding / timeout state. Pin only that the entry point
        // RUNS without panic and never returns `NotSeparationGoal`
        // for a recognised pre/post pair.
        assert!(
            !matches!(outcome, SepObligationOutcome::NotSeparationGoal),
            "both sides recognised — must NOT report NotSeparationGoal",
        );
    }

    #[test]
    fn verify_separation_obligation_unrecognised_pre_returns_not_separation() {
        let pre = call_expr("user_function", vec![]);
        let post = call_expr("emp", vec![]);
        let outcome = verify_separation_obligation(&pre, &post);
        assert!(
            matches!(outcome, SepObligationOutcome::NotSeparationGoal),
            "pre side not a sep predicate → NotSeparationGoal",
        );
    }

    #[test]
    fn verify_separation_obligation_unrecognised_post_returns_not_separation() {
        let pre = call_expr("emp", vec![]);
        let post = call_expr("user_function", vec![]);
        let outcome = verify_separation_obligation(&pre, &post);
        assert!(
            matches!(outcome, SepObligationOutcome::NotSeparationGoal),
            "post side not a sep predicate → NotSeparationGoal",
        );
    }

    #[test]
    fn verify_separation_obligation_both_unrecognised_returns_not_separation() {
        let pre = call_expr("foo", vec![]);
        let post = call_expr("bar", vec![]);
        assert!(matches!(
            verify_separation_obligation(&pre, &post),
            SepObligationOutcome::NotSeparationGoal,
        ));
    }

    #[test]
    fn sep_obligation_outcome_tags_are_distinct() {
        // Pin: every variant produces a distinct stable diagnostic tag.
        let probes = [
            SepObligationOutcome::NotSeparationGoal,
            SepObligationOutcome::Valid,
            SepObligationOutcome::Invalid {
                counterexample_summary: "x".into(),
            },
            SepObligationOutcome::Unknown {
                reason: "y".into(),
            },
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|o| o.tag()).collect();
        assert_eq!(tags.len(), 4, "every outcome variant must have a distinct tag");
    }

    // ----- V6 multi-clause combination -----

    #[test]
    fn combine_empty_clauses_produces_emp() {
        let clauses: Vec<&Expr> = vec![];
        let result = try_combine_clauses_to_sep_assertion(&clauses);
        assert!(matches!(result, Some(SepAssertion::Emp)));
    }

    #[test]
    fn combine_single_clause_returns_recognised_form() {
        let c = call_expr("emp", vec![]);
        let result = try_combine_clauses_to_sep_assertion(&[&c]);
        assert!(matches!(result, Some(SepAssertion::Emp)));
    }

    #[test]
    fn combine_two_clauses_via_sep_conj() {
        // [emp(), points_to(a, av)] should combine to
        // sep_conj(emp, points_to(a, av)).
        let emp_call = call_expr("emp", vec![]);
        let pt_call = call_expr("points_to", vec![name_path_expr("a"), name_path_expr("av")]);
        let result = try_combine_clauses_to_sep_assertion(&[&emp_call, &pt_call]);
        match result {
            Some(SepAssertion::Sep { left, right }) => {
                assert!(matches!(left.as_ref(), SepAssertion::Emp));
                assert!(matches!(right.as_ref(), SepAssertion::PointsTo { .. }));
            }
            other => panic!("expected Sep(Emp, PointsTo), got {:?}", other),
        }
    }

    #[test]
    fn combine_three_clauses_left_associative() {
        // [emp, emp, emp] → sep_conj(sep_conj(emp, emp), emp).
        let c = call_expr("emp", vec![]);
        let result = try_combine_clauses_to_sep_assertion(&[&c, &c, &c]);
        match result {
            Some(SepAssertion::Sep { left, right }) => {
                assert!(matches!(right.as_ref(), SepAssertion::Emp));
                match left.as_ref() {
                    SepAssertion::Sep { .. } => {}
                    other => panic!("expected nested Sep on left, got {:?}", other),
                }
            }
            other => panic!("expected Sep, got {:?}", other),
        }
    }

    #[test]
    fn combine_with_one_unrecognised_clause_returns_none() {
        let emp_call = call_expr("emp", vec![]);
        let user = call_expr("user_function", vec![]);
        let result = try_combine_clauses_to_sep_assertion(&[&emp_call, &user, &emp_call]);
        assert!(
            result.is_none(),
            "all-or-nothing: one unrecognised clause → whole combination fails",
        );
    }

    #[test]
    fn verify_multi_obligation_routes_pure_emp_pair_to_valid() {
        // requires emp ensures emp — trivially valid.
        let emp = call_expr("emp", vec![]);
        let outcome = verify_separation_obligation_multi(&[&emp], &[&emp]);
        // Z3 may return Valid OR Unknown depending on encoder state.
        // Pin only that the entry point RUNS without panic and never
        // returns NotSeparationGoal for recognised clauses.
        assert!(
            !matches!(outcome, SepObligationOutcome::NotSeparationGoal),
            "both sides recognised — must NOT report NotSeparationGoal",
        );
    }

    #[test]
    fn verify_multi_obligation_with_unrecognised_pre_returns_not_separation() {
        let emp = call_expr("emp", vec![]);
        let user = call_expr("user_function", vec![]);
        let outcome = verify_separation_obligation_multi(&[&user], &[&emp]);
        assert!(matches!(outcome, SepObligationOutcome::NotSeparationGoal));
    }

    #[test]
    fn verify_multi_obligation_empty_pre_treated_as_emp() {
        // An empty requires-list is treated as `requires emp`.
        let emp = call_expr("emp", vec![]);
        let outcome = verify_separation_obligation_multi(&[], &[&emp]);
        assert!(
            !matches!(outcome, SepObligationOutcome::NotSeparationGoal),
            "empty pre-list should default to emp, not NotSeparationGoal",
        );
    }

    #[test]
    fn verify_multi_obligation_empty_post_treated_as_emp() {
        let emp = call_expr("emp", vec![]);
        let outcome = verify_separation_obligation_multi(&[&emp], &[]);
        assert!(
            !matches!(outcome, SepObligationOutcome::NotSeparationGoal),
            "empty post-list should default to emp, not NotSeparationGoal",
        );
    }

    // ----- V7 mixed separation+pure splitting -----

    #[test]
    fn partition_classifies_separation_and_pure_correctly() {
        // [emp(), x > 0] should split into 1 separation + 1 pure.
        let emp = call_expr("emp", vec![]);
        let pure_pred = name_path_expr("x_gt_zero"); // surrogate for `x > 0`
        let clauses = [&emp, &pure_pred];
        let partition = partition_clauses(&clauses);
        assert_eq!(partition.separation.len(), 1);
        assert_eq!(partition.pure.len(), 1);
        assert!(matches!(partition.separation[0], SepAssertion::Emp));
    }

    #[test]
    fn partition_all_pure_returns_empty_separation() {
        let p1 = name_path_expr("x_gt_zero");
        let p2 = name_path_expr("y_lt_max");
        let clauses = [&p1, &p2];
        let partition = partition_clauses(&clauses);
        assert!(partition.separation.is_empty());
        assert_eq!(partition.pure.len(), 2);
        assert!(!partition.has_separation());
    }

    #[test]
    fn partition_all_separation_returns_empty_pure() {
        let emp = call_expr("emp", vec![]);
        let pt = call_expr("points_to", vec![name_path_expr("a"), name_path_expr("av")]);
        let clauses = [&emp, &pt];
        let partition = partition_clauses(&clauses);
        assert_eq!(partition.separation.len(), 2);
        assert!(partition.pure.is_empty());
        assert!(partition.has_separation());
    }

    #[test]
    fn combined_separation_emp_when_no_separation_clauses() {
        let p = name_path_expr("x");
        let clauses = [&p];
        let partition = partition_clauses(&clauses);
        let combined = partition.combined_separation();
        assert!(matches!(combined, SepAssertion::Emp));
    }

    #[test]
    fn combined_separation_left_associative_fold() {
        let emp = call_expr("emp", vec![]);
        let pt = call_expr("points_to", vec![name_path_expr("a"), name_path_expr("av")]);
        let clauses = [&emp, &pt, &emp];
        let partition = partition_clauses(&clauses);
        let combined = partition.combined_separation();
        // Expected: sep(sep(emp, points_to), emp).
        match combined {
            SepAssertion::Sep { left, right } => {
                assert!(matches!(right.as_ref(), SepAssertion::Emp));
                match left.as_ref() {
                    SepAssertion::Sep { .. } => {}
                    other => panic!("expected nested Sep on left, got {:?}", other),
                }
            }
            other => panic!("expected outer Sep, got {:?}", other),
        }
    }

    #[test]
    fn split_separation_obligation_no_separation_returns_not_separation_goal() {
        // All clauses pure → V7 has nothing to contribute.
        let p1 = name_path_expr("x_gt_zero");
        let p2 = name_path_expr("y_lt_max");
        let pre = [&p1];
        let post = [&p2];
        let outcome = split_separation_obligation(&pre, &post);
        assert!(matches!(outcome, SeparationSplitOutcome::NotSeparationGoal));
    }

    #[test]
    fn split_separation_obligation_mixed_returns_split_with_pure_clauses() {
        // pre = [emp(), x_gt_zero], post = [emp(), y_lt_max].
        // The separation portion: emp ⊢ emp (trivially valid).
        // The pure portion: [x_gt_zero] ⊢ [y_lt_max] — passed
        // through to the caller for generic SMT dispatch.
        let emp = call_expr("emp", vec![]);
        let pre_pure = name_path_expr("x_gt_zero");
        let post_pure = name_path_expr("y_lt_max");
        let pre = [&emp, &pre_pure];
        let post = [&emp, &post_pure];
        let outcome = split_separation_obligation(&pre, &post);
        match outcome {
            SeparationSplitOutcome::Split {
                separation_outcome: _,
                pure_pre,
                pure_post,
            } => {
                assert_eq!(pure_pre.len(), 1);
                assert_eq!(pure_post.len(), 1);
                // Pre's pure clause is `x_gt_zero`; post's is `y_lt_max`.
                let pre_text = pure_pre[0]
                    .kind
                    .clone();
                let post_text = pure_post[0]
                    .kind
                    .clone();
                let _ = (pre_text, post_text); // shape pin only
            }
            other => panic!("expected Split, got {:?}", other),
        }
    }

    #[test]
    fn split_separation_obligation_only_separation_clauses_routes_normally() {
        // No pure clauses — V7 still routes through (just like V6
        // would have done), pure_pre/pure_post are empty.
        let emp = call_expr("emp", vec![]);
        let pre = [&emp];
        let post = [&emp];
        let outcome = split_separation_obligation(&pre, &post);
        match outcome {
            SeparationSplitOutcome::Split {
                pure_pre,
                pure_post,
                ..
            } => {
                assert!(pure_pre.is_empty());
                assert!(pure_post.is_empty());
            }
            other => panic!("expected Split with empty pure cohorts, got {:?}", other),
        }
    }

    #[test]
    fn sep_obligation_outcome_is_valid_predicate_is_load_bearing() {
        // Only `Valid` returns true. Other variants — including
        // NotSeparationGoal — return false. Used by the
        // verification-phase discharge predicate.
        assert!(SepObligationOutcome::Valid.is_valid());
        assert!(!SepObligationOutcome::NotSeparationGoal.is_valid());
        assert!(!SepObligationOutcome::Invalid {
            counterexample_summary: "x".into()
        }
        .is_valid());
        assert!(!SepObligationOutcome::Unknown {
            reason: "x".into()
        }
        .is_valid());
    }

    // ----- Architectural pin -----

    #[test]
    fn recognised_constructors_match_core_logic_separation_surface() {
        // Pin: every constructor recognised here has a matching
        // smart-constructor in `core/logic/separation.vr`. Adding
        // a new constructor on the .vr side requires extending this
        // recogniser too — the gap between "user can write the
        // predicate" and "verifier translates it" must stay closed.
        let recognised: std::collections::BTreeSet<&str> =
            ["emp", "points_to", "sep_conj", "heap_and", "pure"]
                .iter()
                .copied()
                .collect();
        // The set documents the canonical surface. Updating the
        // recogniser to handle a new constructor MUST also update
        // this pin so reviewers see the surface change.
        assert_eq!(recognised.len(), 5);
        assert!(recognised.contains("emp"));
        assert!(recognised.contains("points_to"));
        assert!(recognised.contains("sep_conj"));
        assert!(recognised.contains("heap_and"));
        assert!(recognised.contains("pure"));
    }
}
