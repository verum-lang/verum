//! Coinduction by checking a candidate relation against a typed observation model.
use crate::{
    context::{Context, ContextConfig},
    verify::VerificationError,
};
use verum_common::{List, Maybe, Text};
use z3::{
    DeclKind, SatResult,
    ast::{Ast, Bool, Dynamic},
};

/// A deterministic observation model over a pair of state spaces.
///
/// `variables` are distinct symbolic state constants. The two state sorts may
/// differ. Expressions in `observations` and `successors` use these variables.
/// For a candidate R, the observation operator is
/// F(R)(x,y) = conjunction(observations agree) ∧ conjunction(R(successor pairs)).
/// R occurs only positively, so this operator is monotone by construction.
///
/// Verification proves initial ∈ R and R ⊆ F(R), which imply initial ∈ νF.
/// This establishes a theorem about the supplied model. It does not establish
/// that source-language observers implement that model, prove productivity, or
/// produce a proof that has been replayed by the Verum kernel.
#[derive(Debug, Clone)]
pub struct CoinductionModel {
    pub variables: [Dynamic; 2],
    pub initial: [Dynamic; 2],
    pub relation: Bool,
    pub observations: List<(Dynamic, Dynamic)>,
    pub successors: List<(Dynamic, Dynamic)>,
}

/// Which universal obligation a solver model refutes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoinductionObligation {
    InitialPair,
    Observation(usize),
    Successor(usize),
}

/// SMT verification of the supplied model, not a portable proof certificate.
#[derive(Debug)]
pub enum CoinductionResult {
    Verified,
    /// A candidate obligation failed; this does not prove initial states inequivalent.
    CandidateRefuted {
        obligation: CoinductionObligation,
        counterexample: Maybe<Text>,
    },
}

impl CoinductionModel {
    /// Verify all coinduction obligations, with the configured per-query budget.
    /// Satisfiability of a negated obligation refutes it. Only UNSAT discharges
    /// an obligation; Unknown is propagated as an error.
    pub fn verify(&self, config: ContextConfig) -> Result<CoinductionResult, VerificationError> {
        self.validate()?;
        let context = Context::with_config(config);
        let solver = context.solver();
        let check = |obligation, proposition: Bool| {
            solver.push();
            context.assert(&solver, &proposition.not());
            let outcome = match context.check(&solver) {
                SatResult::Unsat => Ok(None),
                SatResult::Sat => Ok(Some(CoinductionResult::CandidateRefuted {
                    obligation,
                    counterexample: solver.get_model().map(|m| Text::from(m.to_string())),
                })),
                SatResult::Unknown => Err(VerificationError::Unknown(Text::from(format!(
                    "Coinduction {obligation:?}: {}",
                    solver.get_reason_unknown().unwrap_or_default()
                )))),
            };
            solver.pop(1);
            outcome
        };
        let initial = self.relation.substitute(&[
            (&self.variables[0], &self.initial[0]),
            (&self.variables[1], &self.initial[1]),
        ]);
        if let Some(result) = check(CoinductionObligation::InitialPair, initial)? {
            return Ok(result);
        }
        for (index, (left, right)) in self.observations.iter().enumerate() {
            // validate() established both context and exact sort equality.
            let equality = left.eq(right);
            if let Some(result) = check(
                CoinductionObligation::Observation(index),
                self.relation.implies(equality),
            )? {
                return Ok(result);
            }
        }
        for (index, (left, right)) in self.successors.iter().enumerate() {
            let next = self
                .relation
                .substitute(&[(&self.variables[0], left), (&self.variables[1], right)]);
            if let Some(result) = check(
                CoinductionObligation::Successor(index),
                self.relation.implies(next),
            )? {
                return Ok(result);
            }
        }
        Ok(CoinductionResult::Verified)
    }

    fn validate(&self) -> Result<(), VerificationError> {
        let invalid = |message: &str| VerificationError::SolverError(Text::from(message));
        let active = z3::Context::thread_local();
        let same_context = |value: &Dynamic| value.get_ctx() == &active;
        if self.relation.get_ctx() != &active
            || !self
                .variables
                .iter()
                .chain(self.initial.iter())
                .all(same_context)
            || self
                .observations
                .iter()
                .chain(self.successors.iter())
                .any(|(l, r)| !same_context(l) || !same_context(r))
        {
            return Err(invalid("Coinduction model belongs to another Z3 context"));
        }
        for variable in &self.variables {
            if !variable.is_const() || variable.decl().kind() != DeclKind::Uninterpreted {
                return Err(invalid(
                    "Coinduction state variables must be symbolic constants",
                ));
            }
        }
        if self.variables[0].get_z3_ast() == self.variables[1].get_z3_ast() {
            return Err(invalid("Coinduction state variables must be distinct"));
        }
        for side in 0..2 {
            if self.initial[side].get_sort() != self.variables[side].get_sort() {
                return Err(invalid("Coinduction initial state has the wrong sort"));
            }
        }
        for (left, right) in &self.observations {
            if left.get_sort() != right.get_sort() {
                return Err(invalid("Coinduction observations must have matching sorts"));
            }
        }
        for (left, right) in &self.successors {
            if left.get_sort() != self.variables[0].get_sort()
                || right.get_sort() != self.variables[1].get_sort()
            {
                return Err(invalid("Coinduction successor has the wrong state sort"));
            }
        }
        Ok(())
    }
}
