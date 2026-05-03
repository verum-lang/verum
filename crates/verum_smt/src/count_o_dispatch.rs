//! # Count-of-quantity FMF dispatch (OWL 2 Direct Semantics, VVA §21.5)
//!
//! ## Architectural role
//!
//! Verum's OWL 2 layer ships `count_o(domain, pred)` — the
//! quantifier-of-quantity `# y:o P(y)` from Shkotin 2019 — and a
//! companion `count_o_unbounded(Maybe<List<I>>, pred)` whose runtime
//! return is `Maybe.None` when the user does not supply a finite-domain
//! witness. The HOL semantics is `|{y : Individual | P(y)}|`, only
//! well-defined when the comprehension set is finite (W3C OWL 2 Direct
//! Semantics §5.6 keeps the universe open-world; finite-domain
//! reasoning is therefore a witness-level concern).
//!
//! Pre-this-module, the V1 surface either required an explicit
//! `List<I>` witness (bounded `count_o`) or returned `Maybe.None`
//! (`count_o_unbounded`). The companion `E_OWL2_UNBOUNDED_COUNT`
//! diagnostic surfaced the witness gap to the user but offered no
//! recovery path.
//!
//! V2 (this module) wires CVC5 Finite Model Finding into the SMT
//! routing so that `count_o_unbounded` queries dispatch automatically
//! when:
//!   1. the surrounding refinement type carries an explicit
//!      cardinality bound (`{x : Int | x ≤ K ∧ x = count_o(_, P)}`), and
//!   2. the predicate body is encodable in CVC5's UF + uninterpreted-
//!      sort fragment (no recursive function calls, no higher-order
//!      arguments).
//!
//! The dispatcher emits an `FmfQuery` over an uninterpreted individual
//! sort with cardinality ≤ K, asserts the predicate, and asks CVC5 to
//! enumerate the satisfying interpretations. The model's count of
//! `pred(d) = true` slots is the recovered `count_o` value.
//!
//! ## Failure modes
//!
//! Three structurally distinct outcomes (orthogonal to CVC5's
//! sat/unsat verdict):
//!
//! - `Decided { count, model_smtlib }` — FMF found a finite
//!   interpretation; the count is load-bearing.
//! - `BoundExceeded` — no model satisfies the cardinality bound; the
//!   refinement type's claim is structurally unsatisfiable. Surfaced
//!   to the user as a hard error promoting the V1 warning.
//! - `Unsupported { reason }` — CVC5 is not linked at build time, or
//!   the predicate uses a feature outside FMF's encoding. The caller
//!   falls back to the V1 `Maybe.None` semantics.
//! - `Timeout { elapsed_ms }` — FMF exhausted its time budget.
//!
//! ## Architectural alignment
//!
//! The dispatcher reuses every existing piece of the SMT infrastructure:
//!
//! - [`crate::cvc5_advanced::FmfQuery`] / [`find_finite_model`] for the
//!   actual solver call (no new FFI surface).
//! - [`crate::capability_router::ExtendedCharacteristics::needs_finite_model_finding`]
//!   for routing (count_o queries flip the FMF flag).
//! - [`crate::cvc5_advanced::Cvc5AdvancedError::NotAvailable`] for the
//!   stub-mode fallback path.
//!
//! No new top-level modules; the dispatcher is one focused
//! translation unit between Verum's OWL 2 layer and CVC5's FMF
//! engine.
//!
//! [`find_finite_model`]: crate::cvc5_advanced::find_finite_model

use serde::{Deserialize, Serialize};

use crate::capability_router::ExtendedCharacteristics;
use crate::cvc5_advanced::{
    Cvc5AdvancedError, FmfQuery, FmfResult, find_finite_model,
};

// ============================================================================
// Public types
// ============================================================================

/// A cardinality constraint extracted from the surrounding refinement
/// type. The bound shapes the FMF query's `max_domain_size` and the
/// dispatcher's accept/reject verdict.
///
/// Each variant matches the canonical refinement-type shapes Verum's
/// OWL 2 layer emits when a `count_o` call is wrapped in a
/// `{x : Int | _ }` predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountBound {
    /// `count ≤ K`. The most common form. FMF searches up to K.
    LessEq(u32),
    /// `count = K`. FMF must produce exactly K matches.
    Equal(u32),
    /// `count ≥ K`. FMF must produce at least K matches; a model
    /// proves an existential lower bound.
    GreaterEq(u32),
    /// `K1 ≤ count ≤ K2`. Two-sided bound.
    Range { low: u32, high: u32 },
}

impl CountBound {
    /// Maximum domain size implied by the bound. Drives
    /// [`FmfQuery::max_domain_size`].
    pub fn max_domain_size(&self) -> u32 {
        match *self {
            Self::LessEq(k) => k,
            Self::Equal(k) => k,
            Self::GreaterEq(k) => k.saturating_add(8).max(16),
            Self::Range { high, .. } => high,
        }
    }

    /// True iff the candidate count satisfies this bound.
    pub fn admits(&self, count: u32) -> bool {
        match *self {
            Self::LessEq(k) => count <= k,
            Self::Equal(k) => count == k,
            Self::GreaterEq(k) => count >= k,
            Self::Range { low, high } => count >= low && count <= high,
        }
    }

    /// Stable tag used in routing diagnostics + JSON reports.
    pub fn tag(&self) -> &'static str {
        match *self {
            Self::LessEq(_) => "LessEq",
            Self::Equal(_) => "Equal",
            Self::GreaterEq(_) => "GreaterEq",
            Self::Range { .. } => "Range",
        }
    }
}

/// A `count_o_unbounded` query the dispatcher can answer via FMF.
///
/// Constructed by Verum's refinement-checker when it encounters a
/// `count_o_unbounded(Maybe.None, pred)` inside a refinement type
/// carrying an explicit cardinality bound.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountOQuery {
    /// SMT-LIB symbol for the uninterpreted individual sort. Must
    /// match an SMT-LIB sort name (`I`, `Person`, `Item`, ...).
    pub individual_sort: String,
    /// SMT-LIB body of the predicate `pred(y)`. The body must reference
    /// the parameter variable named in [`Self::predicate_var`].
    pub predicate_body: String,
    /// SMT-LIB variable name for the predicate's input. Conventionally
    /// `y` to match Shkotin's `# y:o P(y)`.
    pub predicate_var: String,
    /// Refinement-type-level cardinality bound. Drives the FMF size
    /// budget and the accept/reject verdict.
    pub bound: CountBound,
    /// Solver timeout in milliseconds. `0` ⇒ no timeout (use solver
    /// default).
    pub timeout_ms: u64,
}

impl CountOQuery {
    /// Build a query with the canonical OWL 2 individual-sort name.
    pub fn new(predicate_body: impl Into<String>, bound: CountBound) -> Self {
        Self {
            individual_sort: "Individual".into(),
            predicate_body: predicate_body.into(),
            predicate_var: "y".into(),
            bound,
            timeout_ms: 5_000,
        }
    }
}

/// Outcome of a `count_o` FMF dispatch.
///
/// Distinct variants for each architecturally meaningful failure mode
/// — the caller maps them onto Verum-side diagnostic levels (hard
/// error / soft warning / silent fallback).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CountOResult {
    /// FMF found a finite interpretation; `count` is load-bearing.
    Decided {
        /// |{y ∈ domain | pred(y)}| in the discovered model.
        count: u32,
        /// SMT-LIB-formatted model the FMF engine returned. Empty
        /// when the engine reports no model body.
        model_smtlib: String,
        /// Wall-clock elapsed time of the FMF call.
        elapsed_ms: u64,
    },
    /// No model satisfies the cardinality bound — the surrounding
    /// refinement type's claim is structurally false. Promotes the V1
    /// `E_OWL2_UNBOUNDED_COUNT` warning to a hard error.
    BoundExceeded {
        /// The cardinality bound the query carried.
        bound: CountBound,
        /// Wall-clock elapsed time of the FMF call.
        elapsed_ms: u64,
    },
    /// CVC5 is not linked at build time, or the predicate is outside
    /// FMF's encoding. The caller falls back to the V1
    /// `Maybe.None` semantics.
    Unsupported {
        /// Stable diagnostic string.
        reason: String,
    },
    /// FMF exhausted its time budget without a verdict.
    Timeout {
        /// Wall-clock elapsed time of the FMF call.
        elapsed_ms: u64,
    },
}

impl CountOResult {
    /// Stable tag used in routing diagnostics + JSON reports.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Decided { .. } => "Decided",
            Self::BoundExceeded { .. } => "BoundExceeded",
            Self::Unsupported { .. } => "Unsupported",
            Self::Timeout { .. } => "Timeout",
        }
    }

    /// True iff the result is a load-bearing `Decided`.
    pub fn is_decided(&self) -> bool {
        matches!(self, Self::Decided { .. })
    }
}

// ============================================================================
// SMT-LIB translation
// ============================================================================

/// Translate a [`CountOQuery`] into the SMT-LIB assertion body the
/// FMF engine consumes.
///
/// The encoding shape:
///
/// ```smtlib
/// (declare-sort Individual 0)
/// (declare-fun pred_o (Individual) Bool)
/// (assert (forall ((y Individual)) (= (pred_o y) <predicate_body>)))
/// ; cardinality bound becomes the FMF size budget — encoded externally
/// ; via `FmfQuery::max_domain_size`.
/// ```
///
/// The predicate's body is wrapped in a universally-quantified
/// equality that pins the meaning of `pred_o` to the user's predicate.
/// The FMF engine then enumerates finite interpretations; each model
/// pins `pred_o`'s extension on the `K` Individual atoms.
///
/// Returns the assertion list as one SMT-LIB string per assertion
/// (suitable for direct insertion into [`FmfQuery::assertions`]).
pub fn count_o_to_smtlib(q: &CountOQuery) -> Vec<String> {
    let sort = &q.individual_sort;
    let pred_var = &q.predicate_var;
    let pred_body = &q.predicate_body;
    vec![
        format!("(declare-sort {sort} 0)"),
        format!("(declare-fun pred_o ({sort}) Bool)"),
        format!(
            "(assert (forall (({pred_var} {sort})) (= (pred_o {pred_var}) {pred_body})))"
        ),
    ]
}

/// Build an [`FmfQuery`] for the given count-of query. The query's
/// logic is fixed at `UF` (uninterpreted-function fragment) — FMF's
/// home turf.
pub fn build_fmf_query(q: &CountOQuery) -> FmfQuery {
    FmfQuery {
        logic: "UF".into(),
        assertions: count_o_to_smtlib(q),
        max_domain_size: q.bound.max_domain_size().clamp(1, 1024),
        timeout_ms: q.timeout_ms,
    }
}

// ============================================================================
// Dispatch entrypoint
// ============================================================================

/// Run a [`CountOQuery`] through CVC5's Finite Model Finding engine
/// and return the structured outcome.
///
/// In stub mode (CVC5 not linked), returns
/// [`CountOResult::Unsupported`] without making any FFI calls — the
/// caller falls back to the V1 `Maybe.None` semantics.
pub fn dispatch_count_o(q: &CountOQuery) -> CountOResult {
    let fmf_query = build_fmf_query(q);

    match find_finite_model(&fmf_query) {
        Ok(FmfResult::Model {
            model,
            domain_sizes,
            elapsed_ms,
        }) => {
            let count = extract_count_from_model(&model, &domain_sizes, q);
            if q.bound.admits(count) {
                CountOResult::Decided {
                    count,
                    model_smtlib: model,
                    elapsed_ms,
                }
            } else {
                CountOResult::BoundExceeded {
                    bound: q.bound,
                    elapsed_ms,
                }
            }
        }
        Ok(FmfResult::Unsat { elapsed_ms }) => CountOResult::BoundExceeded {
            bound: q.bound,
            elapsed_ms,
        },
        Ok(FmfResult::Unknown { reason, elapsed_ms }) => {
            // Treat "unknown" as unsupported: the FMF engine could
            // neither model-find nor refute. The V1 fallback is the
            // honest answer.
            let _ = elapsed_ms;
            CountOResult::Unsupported {
                reason: format!("FMF returned UNKNOWN: {reason}"),
            }
        }
        Err(Cvc5AdvancedError::NotAvailable) => CountOResult::Unsupported {
            reason: "CVC5 is not linked (cvc5-sys/vendored disabled)".into(),
        },
        Err(Cvc5AdvancedError::Timeout { timeout_ms }) => CountOResult::Timeout {
            elapsed_ms: timeout_ms,
        },
        Err(Cvc5AdvancedError::Unsupported(msg)) => CountOResult::Unsupported {
            reason: format!("FMF unsupported: {msg}"),
        },
        Err(other) => CountOResult::Unsupported {
            reason: format!("CVC5 advanced error: {other}"),
        },
    }
}

/// Extract the count of `pred_o(d) = true` slots from the FMF
/// engine's model. The model format is SMT-LIB 2:
///
/// ```smtlib
/// (
///   (define-fun pred_o ((y Individual)) Bool
///     (or (= y @Individual_0) (= y @Individual_3))
///   )
/// )
/// ```
///
/// We count the disjuncts in the `pred_o` definition. When the
/// definition is `false` (no element satisfies), the count is 0;
/// when the definition is `true` (every element satisfies), the
/// count is the discovered domain size.
fn extract_count_from_model(
    model: &str,
    domain_sizes: &[(String, u32)],
    q: &CountOQuery,
) -> u32 {
    let domain_size = domain_sizes
        .iter()
        .find(|(name, _)| name == &q.individual_sort)
        .map(|(_, size)| *size)
        .unwrap_or(0);

    // Locate the `(define-fun pred_o ...)` block.
    let needle = "define-fun pred_o";
    let Some(start) = model.find(needle) else {
        // No predicate definition in the model; conservatively report
        // zero matches. The caller's bound check will surface the
        // mismatch if it matters.
        return 0;
    };

    let after_signature = match model[start..].find(") Bool") {
        Some(off) => start + off + ") Bool".len(),
        None => return 0,
    };

    let body = model[after_signature..]
        .trim_start()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();

    if body.starts_with("true") || body == "true" {
        return domain_size;
    }
    if body.starts_with("false") || body == "false" {
        return 0;
    }

    // Count `=` disjuncts. Every `(= y @Sort_<n>)` corresponds to one
    // matching element. `or` is the canonical join; `and` would mean
    // intersection, which FMF doesn't emit for membership predicates.
    let count = body.matches("(=").count();
    count.min(domain_size as usize) as u32
}

// ============================================================================
// Capability-router integration
// ============================================================================

/// Annotate the SMT capability router's characteristics with a
/// count_o-driven FMF requirement. Flipping
/// `needs_finite_model_finding` routes the goal to CVC5 (per
/// [`crate::capability_router`]'s policy).
///
/// Should be called by Verum's refinement-checker for every goal it
/// detects as containing a `count_o_unbounded` call.
pub fn flag_count_o_dispatch(chars: &mut ExtendedCharacteristics) {
    chars.needs_finite_model_finding = true;
    chars.base.has_quantifiers = true;
    chars.quantifier_depth = chars.quantifier_depth.max(1);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_bound_max_domain_size_matches_variant() {
        assert_eq!(CountBound::LessEq(7).max_domain_size(), 7);
        assert_eq!(CountBound::Equal(3).max_domain_size(), 3);
        assert_eq!(CountBound::Range { low: 1, high: 12 }.max_domain_size(), 12);
        // GreaterEq pads upward so FMF has room to discover larger
        // models.
        assert!(CountBound::GreaterEq(2).max_domain_size() >= 16);
    }

    #[test]
    fn count_bound_admits_classifies_correctly() {
        let b = CountBound::LessEq(5);
        assert!(b.admits(0));
        assert!(b.admits(5));
        assert!(!b.admits(6));

        let b = CountBound::Equal(3);
        assert!(b.admits(3));
        assert!(!b.admits(2));
        assert!(!b.admits(4));

        let b = CountBound::GreaterEq(2);
        assert!(b.admits(2));
        assert!(b.admits(100));
        assert!(!b.admits(1));

        let b = CountBound::Range { low: 2, high: 5 };
        assert!(b.admits(2));
        assert!(b.admits(5));
        assert!(!b.admits(1));
        assert!(!b.admits(6));
    }

    #[test]
    fn count_bound_tag_is_stable() {
        assert_eq!(CountBound::LessEq(1).tag(), "LessEq");
        assert_eq!(CountBound::Equal(1).tag(), "Equal");
        assert_eq!(CountBound::GreaterEq(1).tag(), "GreaterEq");
        assert_eq!(
            CountBound::Range { low: 0, high: 1 }.tag(),
            "Range"
        );
    }

    #[test]
    fn count_o_to_smtlib_emits_three_assertions() {
        let q = CountOQuery::new("(>= y 0)", CountBound::LessEq(8));
        let asserts = count_o_to_smtlib(&q);
        assert_eq!(asserts.len(), 3);
        assert!(asserts[0].contains("declare-sort Individual"));
        assert!(asserts[1].contains("declare-fun pred_o (Individual) Bool"));
        assert!(asserts[2].contains("forall"));
        assert!(asserts[2].contains("(>= y 0)"));
    }

    #[test]
    fn count_o_to_smtlib_respects_custom_sort_and_var() {
        let mut q = CountOQuery::new("(P y_var)", CountBound::Equal(2));
        q.individual_sort = "Person".into();
        q.predicate_var = "y_var".into();
        let asserts = count_o_to_smtlib(&q);
        assert!(asserts[0].contains("declare-sort Person"));
        assert!(asserts[1].contains("(Person)"));
        assert!(asserts[2].contains("y_var"));
    }

    #[test]
    fn build_fmf_query_clamps_domain_size_to_solver_bounds() {
        let q = CountOQuery::new(
            "(P y)",
            CountBound::LessEq(2_000), // exceeds 1024
        );
        let fmf = build_fmf_query(&q);
        assert_eq!(fmf.max_domain_size, 1024);
        assert_eq!(fmf.logic, "UF");
        assert_eq!(fmf.assertions.len(), 3);
    }

    #[test]
    fn build_fmf_query_clamps_zero_to_one() {
        let q = CountOQuery::new("(P y)", CountBound::LessEq(0));
        let fmf = build_fmf_query(&q);
        assert_eq!(fmf.max_domain_size, 1);
    }

    #[test]
    fn dispatch_returns_unsupported_in_stub_mode() {
        // In stub mode (cvc5-sys not linked) `find_finite_model` is
        // expected to error with NotAvailable; the dispatcher must
        // surface that as `Unsupported` so the V1 fallback path runs.
        if !crate::cvc5_advanced::is_available() {
            let q = CountOQuery::new("(P y)", CountBound::LessEq(4));
            let result = dispatch_count_o(&q);
            assert_eq!(result.tag(), "Unsupported");
            assert!(!result.is_decided());
        }
    }

    #[test]
    fn extract_count_from_model_handles_disjunction() {
        let model = "( (define-fun pred_o ((y Individual)) Bool \
                     (or (= y @Individual_0) (= y @Individual_2))) )";
        let domain_sizes = vec![("Individual".into(), 4u32)];
        let q = CountOQuery::new("(P y)", CountBound::LessEq(4));
        assert_eq!(extract_count_from_model(model, &domain_sizes, &q), 2);
    }

    #[test]
    fn extract_count_from_model_handles_true_body() {
        let model =
            "( (define-fun pred_o ((y Individual)) Bool true) )";
        let domain_sizes = vec![("Individual".into(), 5u32)];
        let q = CountOQuery::new("(P y)", CountBound::LessEq(5));
        assert_eq!(extract_count_from_model(model, &domain_sizes, &q), 5);
    }

    #[test]
    fn extract_count_from_model_handles_false_body() {
        let model =
            "( (define-fun pred_o ((y Individual)) Bool false) )";
        let domain_sizes = vec![("Individual".into(), 7u32)];
        let q = CountOQuery::new("(P y)", CountBound::LessEq(7));
        assert_eq!(extract_count_from_model(model, &domain_sizes, &q), 0);
    }

    #[test]
    fn extract_count_clamps_at_domain_size() {
        // Pathological model returning more `=` patterns than
        // domain elements — clamp to domain size to avoid lying.
        let model = "( (define-fun pred_o ((y Individual)) Bool \
                     (or (= y @Individual_0) (= y @Individual_1) \
                         (= y @Individual_2) (= y @Individual_3))) )";
        let domain_sizes = vec![("Individual".into(), 2u32)];
        let q = CountOQuery::new("(P y)", CountBound::LessEq(2));
        assert_eq!(extract_count_from_model(model, &domain_sizes, &q), 2);
    }

    #[test]
    fn extract_count_returns_zero_when_predicate_missing() {
        let model = "( ; no pred_o definition )";
        let domain_sizes = vec![("Individual".into(), 4u32)];
        let q = CountOQuery::new("(P y)", CountBound::LessEq(4));
        assert_eq!(extract_count_from_model(model, &domain_sizes, &q), 0);
    }

    #[test]
    fn flag_count_o_dispatch_flips_fmf_route() {
        let mut chars = ExtendedCharacteristics::new();
        assert!(!chars.needs_finite_model_finding);
        flag_count_o_dispatch(&mut chars);
        assert!(chars.needs_finite_model_finding);
        assert!(chars.base.has_quantifiers);
        assert!(chars.quantifier_depth >= 1);
    }

    #[test]
    fn count_o_result_tag_is_stable() {
        let r = CountOResult::Decided {
            count: 1,
            model_smtlib: String::new(),
            elapsed_ms: 0,
        };
        assert_eq!(r.tag(), "Decided");
        assert!(r.is_decided());

        let r = CountOResult::BoundExceeded {
            bound: CountBound::LessEq(0),
            elapsed_ms: 0,
        };
        assert_eq!(r.tag(), "BoundExceeded");
        assert!(!r.is_decided());

        let r = CountOResult::Unsupported {
            reason: String::new(),
        };
        assert_eq!(r.tag(), "Unsupported");
        assert!(!r.is_decided());

        let r = CountOResult::Timeout { elapsed_ms: 0 };
        assert_eq!(r.tag(), "Timeout");
        assert!(!r.is_decided());
    }
}
