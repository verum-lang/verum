//! Integer ranking obligations over explicit, guarded state transitions.
use crate::context::{Context, ContextConfig};
use verum_common::{List, Text};
use z3::{
    DeclKind, SatResult,
    ast::{Ast, Bool, Dynamic, Int},
};

/// One possible recursive call or loop back-edge. Arguments are simultaneous
/// successor expressions in the current parameters; the guard is its path condition.
#[derive(Debug, Clone)]
pub struct RankingTransition {
    pub guard: Bool,
    pub arguments: List<Dynamic>,
}

/// A candidate integer rank and an additional domain obligation.
#[derive(Debug)]
pub struct RankingFunction {
    pub expression: Int,
    /// Must follow from the domain; it is never assumed to exclude counterexamples.
    pub well_founded_constraint: Bool,
}

impl RankingFunction {
    /// Prove domain nonnegativity and closure, and strict decrease on every
    /// guarded transition. Only UNSAT of each negated obligation is success.
    ///
    /// This proves termination of the supplied transition model. A source
    /// adapter must account for every back-edge and all intervening work;
    /// this API does not establish source correspondence or kernel replay.
    /// `false` refutes this candidate, not termination of the program itself.
    pub fn verify(
        &self,
        parameters: &[Dynamic],
        domain: &Bool,
        transitions: &[RankingTransition],
        config: ContextConfig,
    ) -> Result<bool, Text> {
        let active = z3::Context::thread_local();
        if self.expression.get_ctx() != &active
            || self.well_founded_constraint.get_ctx() != &active
            || domain.get_ctx() != &active
            || parameters.iter().any(|p| p.get_ctx() != &active)
            || transitions.iter().any(|t| {
                t.guard.get_ctx() != &active || t.arguments.iter().any(|a| a.get_ctx() != &active)
            })
        {
            return Err(Text::from("Ranking model belongs to another Z3 context"));
        }
        let mut seen = std::collections::HashSet::new();
        for p in parameters {
            if !p.is_const()
                || p.decl().kind() != DeclKind::Uninterpreted
                || !seen.insert(p.get_z3_ast())
            {
                return Err(Text::from(
                    "Ranking parameters must be distinct symbolic constants",
                ));
            }
        }
        // Validate the entire model before discharging any obligation.
        for t in transitions {
            if t.arguments.len() != parameters.len() {
                return Err(Text::from(
                    "Ranking transition argument count differs from parameters",
                ));
            }
            if parameters
                .iter()
                .zip(t.arguments.iter())
                .any(|(p, a)| p.get_sort() != a.get_sort())
            {
                return Err(Text::from("Ranking transition argument has the wrong sort"));
            }
        }
        let context = Context::with_config(config);
        let solver = context.solver();
        let proves = |proposition: Bool| {
            solver.push();
            context.assert(&solver, &proposition.not());
            let result = match context.check(&solver) {
                SatResult::Unsat => Ok(true),
                SatResult::Sat => Ok(false),
                SatResult::Unknown => Err(Text::from(format!(
                    "Ranking obligation unknown: {}",
                    solver.get_reason_unknown().unwrap_or_default()
                ))),
            };
            solver.pop(1);
            result
        };
        let valid_rank = Bool::and(&[&self.expression.ge(0), &self.well_founded_constraint]);
        if !proves(domain.implies(valid_rank))? {
            return Ok(false);
        }
        for t in transitions {
            let substitutions: Vec<_> = parameters.iter().zip(t.arguments.iter()).collect();
            let next_rank = self.expression.substitute(&substitutions);
            let next_domain = domain.substitute(&substitutions);
            let step = Bool::and(&[&next_domain, &next_rank.lt(&self.expression)]);
            if !proves(Bool::and(&[domain, &t.guard]).implies(step))? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
