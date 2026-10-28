//! Inductive models of the original rules, independent of reachability queries.
use super::{FixedPointEngine, FixedPointSolution, PredicateInterpretation};
use crate::context::{Context, ContextConfig};
use verum_common::{List, Set, Text};
use z3::{
    AstKind, DeclKind, SatResult,
    ast::{Ast, Bool, Dynamic},
};

impl FixedPointEngine {
    /// Prove every original rule, background assertion and candidate global
    /// property under complete, nonrecursive relation definitions. Only UNSAT
    /// of each negated obligation is acceptance; Unknown is an error.
    ///
    /// This checks an inductive overapproximation, not the least fixed point or
    /// correspondence to a source program. No proof certificate is extracted.
    pub fn validate_solution(&self, solution: &FixedPointSolution) -> Result<bool, Text> {
        self.validate_solution_with_config(solution, ContextConfig::default())
    }

    pub fn validate_solution_with_config(
        &self,
        solution: &FixedPointSolution,
        config: ContextConfig,
    ) -> Result<bool, Text> {
        if self.ctx != z3::Context::thread_local() {
            return Err("Solution validation requires the active Z3 context".into());
        }
        if self.predicates.is_empty() {
            return Err("Solution validation requires registered relations".into());
        }
        // Validate the complete candidate before discharging any obligation.
        let mut definitions = List::new();
        for (name, interpretation) in &solution.interpretations {
            definitions.push(self.interpretation_definition(name, interpretation)?);
        }
        for name in self.predicates.keys() {
            if !solution.interpretations.contains_key(name) {
                return Err(format!("Missing predicate interpretation: {name}").into());
            }
        }
        if solution.invariants.iter().any(|p| p.get_ctx() != &self.ctx) {
            return Err("Candidate property belongs to another Z3 context".into());
        }
        let context = Context::with_config(config);
        let solver = context.solver();
        for definition in definitions {
            context.assert(&solver, &definition);
        }
        for obligation in self
            .rules
            .iter()
            .chain(&self.axioms)
            .chain(&solution.invariants)
        {
            solver.push();
            context.assert(&solver, &obligation.not());
            let result: Result<bool, Text> = match context.check(&solver) {
                SatResult::Unsat => Ok(true),
                SatResult::Sat => Ok(false),
                SatResult::Unknown => Err(format!(
                    "Inductive solution obligation unknown: {}",
                    solver.get_reason_unknown().unwrap_or_default()
                )
                .into()),
            };
            solver.pop(1);
            if !result? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn interpretation_definition(
        &self,
        name: &Text,
        interpretation: &PredicateInterpretation,
    ) -> Result<Bool, Text> {
        let relation = self
            .predicates
            .get(name)
            .ok_or_else(|| Text::from(format!("Unknown candidate predicate: {name}")))?;
        if &interpretation.predicate != name {
            return Err("Interpretation key differs from its predicate".into());
        }
        self.check_relation(relation, Some(&interpretation.parameters))?;
        if interpretation.formula.get_ctx() != &self.ctx {
            return Err("Predicate interpretation belongs to another Z3 context".into());
        }
        let mut parameters = Set::new();
        for parameter in &interpretation.parameters {
            if !parameter.is_const()
                || parameter.decl().kind() != DeclKind::Uninterpreted
                || !parameters.insert(parameter.clone())
            {
                return Err("Interpretation parameters must be distinct symbolic constants".into());
            }
        }
        // A definition must be a theory formula over its explicit parameters.
        // Otherwise P := not P (including hidden as-array/recursive calls) could
        // make the checking context inconsistent and prove every original rule.
        let mut pending = List::from(vec![Dynamic::from_ast(&interpretation.formula)]);
        let mut visited = Set::new();
        while let Some(term) = pending.pop() {
            if !visited.insert(term.clone()) {
                continue;
            }
            match term.kind() {
                AstKind::Numeral => {}
                AstKind::App => {
                    match term.decl().kind() {
                        DeclKind::Uninterpreted if !parameters.contains(&term) => {
                            return Err(format!(
                                "Unbound symbol or predicate in interpretation: {term}"
                            )
                            .into());
                        }
                        DeclKind::AsArray | DeclKind::Recursive => {
                            return Err("Interpretations cannot contain function-valued or recursive definitions".into());
                        }
                        _ => {}
                    }
                    pending.extend(term.children());
                }
                _ => return Err("Interpretations require quantifier-free theory formulas".into()),
            }
        }
        let arguments: List<&dyn Ast> = interpretation
            .parameters
            .iter()
            .map(|p| p as &dyn Ast)
            .collect();
        let application = relation
            .apply(&arguments)
            .as_bool()
            .ok_or_else(|| Text::from("Relation interpretation must be Boolean"))?;
        Ok(z3::ast::forall_const(
            &arguments,
            &[],
            &application.eq(&interpretation.formula),
        ))
    }
}
