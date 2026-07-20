//! The two judgments a refinement type admits, and the single solver
//! routine that discharges them.
//!

//! A refinement type `T{P}` admits exactly two questions, and
//! conflating them is what made every non-tautological parameter
//! refinement report as "unsatisfiable" (T0457):
//!

//!  * **Inhabitation** — asked at a *declaration* site, where no value
//!    is in hand (`fn f(max_level: Int{>= 0 && <= 5})`). The claim is
//!    existential: `∃x:T. P(x)`. Discharged by asserting `P` and
//!    checking SAT. `Unsat` is the only outcome that makes the
//!    declaration an error: the declared type has no inhabitants, so no
//!    call can ever be well-typed.
//!

//!  * **Membership** — asked at a *use* site, where a concrete value
//!    expression `e` is in hand. The claim is universal over the free
//!    variables of `e`: `∀. P[it := e]`. Discharged by the repo-wide
//!    negate-and-check-Unsat convention: assert `¬P[it := e]` and
//!    require `Unsat`. A `Sat` here is a counterexample, never a proof.
//!

//! Before T0457 both call paths asked a third question that nobody
//! wants — `∀x:T. P(x)`, "is *every* inhabitant of the base type inside
//! the refinement" — and reported its `Sat` as "unsatisfiable
//! refinement constraint". Under that judgment `Int{> 0}`, `Int{>= 0}`,
//! `Int{!= 0}` and every other useful refinement are errors; only a
//! tautology such as `Int{it == it}` passes. (`T <: T{P}` is a real
//! question, but it belongs to [`crate::subsumption`], which already
//! owns it.)
//!

//! This module is the ONE authority for discharging either judgment.
//! [`crate::refinement::RefinementVerifier::verify_refinement`] (the
//! compiler pipeline and LSP entry point) and
//! [`crate::verify::verify_refinement`] (the free-function entry point)
//! both route through [`discharge_refinement_judgment`]; they differ
//! only in the result decoration they layer on top (proof extraction
//! and caching respectively).

use verum_ast::{Expr, Type};
use verum_common::{Text, ToText};
use z3::ast::{Bool, Dynamic, Int, Real};

use crate::context::Context;
use crate::counterexample::{CounterExample, CounterExampleExtractor};
use crate::translate::Translator;
use crate::verify::VerificationError;

/// The conventional name a refinement predicate uses for the value
/// under refinement.
pub const REFINEMENT_VALUE_VAR: &str = "it";

/// Which of the two refinement judgments to discharge.
#[derive(Debug, Clone, Copy)]
pub enum RefinementJudgment<'a> {
    /// Declaration-site well-formedness: `∃x:T. P(x)`.
    ///
    /// Refuted only when the predicate is contradictory over the base
    /// type, i.e. the declared type is uninhabited.
    Inhabited,

    /// Use-site membership: `∀. P[it := e]` for the given value
    /// expression `e`.
    Satisfies(&'a Expr),
}

impl RefinementJudgment<'_> {
    /// A short label for diagnostics and cost records.
    pub fn label(&self) -> &'static str {
        match self {
            RefinementJudgment::Inhabited => "refinement_inhabited",
            RefinementJudgment::Satisfies(_) => "refinement_membership",
        }
    }
}

/// Outcome of discharging a [`RefinementJudgment`].
#[derive(Debug, Clone)]
pub enum JudgmentOutcome {
    /// The judgment was discharged.
    Holds,

    /// The judgment was refuted.
    ///
    /// `counterexample` carries the refuting assignment when one
    /// exists. Only [`RefinementJudgment::Satisfies`] can produce one —
    /// a refuted inhabitation claim is an `Unsat`, which has no model by
    /// construction.
    Refuted {
        /// The refuting assignment, when the solver produced a model.
        counterexample: Option<CounterExample>,
    },

    /// The solver returned `unknown`.
    Unknown {
        /// The solver's own reason string, when it gave one.
        reason: String,
    },
}

/// Discharge one refinement judgment against `solver`.
///
/// `solver` is supplied by the caller (rather than created here) so the
/// caller can interrogate it afterwards — `get_proof()` on a discharged
/// membership claim, `get_model()` for a witness.
///
/// Returns `Err` only for translation failures; a refuted or unknown
/// judgment is a successful `Ok(JudgmentOutcome)`, and the caller
/// decides how to render it.
pub fn discharge_refinement_judgment(
    context: &Context,
    solver: &z3::Solver,
    base_type: &Type,
    predicate: &Expr,
    judgment: RefinementJudgment<'_>,
) -> Result<JudgmentOutcome, VerificationError> {
    let mut translator = Translator::new(context);

    // Bind the refinement variable. For a membership claim the
    // predicate must read the *value*, not a free variable, so tie the
    // two together; see `bind_value_var`.
    match judgment {
        RefinementJudgment::Inhabited => {
            let z3_var = translator.create_var(REFINEMENT_VALUE_VAR, base_type)?;
            translator.bind(REFINEMENT_VALUE_VAR.to_text(), z3_var);
        }
        RefinementJudgment::Satisfies(value_expr) => {
            bind_value_var(solver, &mut translator, value_expr)?;
        }
    }

    let z3_predicate = translator.translate_expr(predicate)?;
    let z3_bool = z3_predicate
        .as_bool()
        .ok_or_else(|| VerificationError::SolverError("predicate is not boolean".to_text()))?;

    match judgment {
        // ∃x. P(x) — assert the predicate itself and look for a witness.
        RefinementJudgment::Inhabited => solver.assert(z3_bool.clone()),
        // ∀. P[it := e] — assert the negation and require Unsat.
        RefinementJudgment::Satisfies(_) => solver.assert(z3_bool.not()),
    }

    let verdict = context.check(solver);

    Ok(match (judgment, verdict) {
        // A model of P is an inhabitant: the declared type is non-empty.
        (RefinementJudgment::Inhabited, z3::SatResult::Sat) => JudgmentOutcome::Holds,

        // No model of P exists over the base type: the declared type is
        // uninhabited. There is no counterexample to report — the
        // refutation *is* the absence of any model.
        (RefinementJudgment::Inhabited, z3::SatResult::Unsat) => {
            JudgmentOutcome::Refuted {
                counterexample: None,
            }
        }

        // ¬P is unsatisfiable, so P holds for the value.
        (RefinementJudgment::Satisfies(_), z3::SatResult::Unsat) => JudgmentOutcome::Holds,

        // A model of ¬P is a counterexample to membership.
        (RefinementJudgment::Satisfies(_), z3::SatResult::Sat) => {
            let counterexample = solver.get_model().map(|model| {
                CounterExampleExtractor::new(&model).extract(
                    &[Text::from(REFINEMENT_VALUE_VAR)],
                    &format!("{:?}", predicate),
                )
            });
            JudgmentOutcome::Refuted { counterexample }
        }

        (_, z3::SatResult::Unknown) => JudgmentOutcome::Unknown {
            reason: solver
                .get_reason_unknown()
                .unwrap_or_else(|| "unknown".to_string()),
        },
    })
}

/// Bind `it` for a membership claim so the predicate reads the value.
///
/// Creates a constant of the base type, ties it to the translated value
/// with an equality assertion, and binds `it` to the constant — that
/// keeps `it` visible in the model so a refuted claim can report a
/// concrete counterexample.
///
/// The equality can only be asserted for sorts the translator produces
/// (Bool / Int / Real). For any other sort the constant would be
/// unconstrained, which would let an unrelated value satisfy the
/// predicate and yield an unsound `Holds`; so in that case `it` is bound
/// directly to the translated value instead. That is exact substitution
/// — sound, just without an `it` row in the counterexample.
fn bind_value_var(
    solver: &z3::Solver,
    translator: &mut Translator<'_>,
    value_expr: &Expr,
) -> Result<(), VerificationError> {
    let z3_value = translator.translate_expr(value_expr)?;

    if let Some(v) = z3_value.as_bool() {
        let var = Bool::new_const(REFINEMENT_VALUE_VAR);
        solver.assert(var.eq(&v));
        translator.bind(REFINEMENT_VALUE_VAR.to_text(), Dynamic::from_ast(&var));
    } else if let Some(v) = z3_value.as_int() {
        let var = Int::new_const(REFINEMENT_VALUE_VAR);
        solver.assert(var.eq(&v));
        translator.bind(REFINEMENT_VALUE_VAR.to_text(), Dynamic::from_ast(&var));
    } else if let Some(v) = z3_value.as_real() {
        let var = Real::new_const(REFINEMENT_VALUE_VAR);
        solver.assert(var.eq(&v));
        translator.bind(REFINEMENT_VALUE_VAR.to_text(), Dynamic::from_ast(&var));
    } else {
        tracing::debug!(
            "refinement membership: value has a sort outside Bool/Int/Real; \
             substituting it directly (no `it` row in counterexamples)"
        );
        translator.bind(REFINEMENT_VALUE_VAR.to_text(), z3_value);
    }

    Ok(())
}

/// Suggestions for a refuted inhabitation claim.
///
/// An uninhabited refinement is a spec bug, not a proof gap, so the
/// advice differs from the counterexample-driven suggestions used for
/// membership failures.
pub fn uninhabited_suggestions() -> verum_common::List<Text> {
    let mut list = verum_common::List::new();
    list.push("The predicate has no solution over the base type — no value can ever inhabit this refinement".to_text());
    list.push(
        "Check for contradictory bounds (e.g. `{> 10 && < 5}`) or a mistyped comparison operator"
            .to_text(),
    );
    list.push("Widen the base type if the intended values fall outside it".to_text());
    list
}
