//! Bounded search for existential universe metavariables.
//!
//! Z3 proposes a nonnegative integer assignment for the original structural
//! constraints. Acceptance independently evaluates every original constraint
//! with checked arithmetic, converts every assigned value without truncation,
//! and rechecks the resulting substitution. Solver status alone is not evidence
//! of a valid universe assignment. Unknown, limits and unsupported model values
//! are errors. Universal level ordering is a separate `UniverseLevel` judgment.

pub use crate::context::{UniverseConstraint, UniverseContext, UniverseSubstitution};
pub use crate::ty::UniverseLevel;

use verum_common::{List, Map, Set, Text};
use z3::{Params, SatResult, Solver, ast::Int};

#[derive(Debug, Clone, Copy)]
pub struct SolverLimits {
    pub max_constraints: usize,
    pub max_expression_nodes: usize,
    pub max_variables: usize,
    pub timeout_ms: u32,
    pub solver_steps: u32,
}

impl Default for SolverLimits {
    fn default() -> Self {
        Self {
            max_constraints: 4096,
            max_expression_nodes: 65_536,
            max_variables: 4096,
            timeout_ms: 2000,
            solver_steps: 2_000_000,
        }
    }
}

pub fn solve_universe_constraints(
    constraints: &[UniverseConstraint],
) -> Result<UniverseSubstitution, Text> {
    solve_with_limits(constraints, SolverLimits::default())
}

pub fn solve_with_limits(
    constraints: &[UniverseConstraint],
    limits: SolverLimits,
) -> Result<UniverseSubstitution, Text> {
    if constraints.len() > limits.max_constraints
        || limits.timeout_ms == 0
        || limits.solver_steps == 0
    {
        return Err("universe constraint search limit exceeded".into());
    }
    let solver = Solver::new();
    let mut parameters = Params::new();
    parameters.set_u32("timeout", limits.timeout_ms);
    parameters.set_u32("rlimit", limits.solver_steps);
    solver.set_params(&parameters);
    let mut translation = Translation {
        variables: Map::new(),
        remaining_nodes: limits.max_expression_nodes,
        max_variables: limits.max_variables,
    };
    for constraint in constraints {
        let assertion = match constraint {
            UniverseConstraint::LessOrEqual(u, v) => {
                translation.level(u)?.le(translation.level(v)?)
            }
            UniverseConstraint::StrictlyLess(u, v) => {
                translation.level(u)?.lt(translation.level(v)?)
            }
            UniverseConstraint::Equal(u, v) => translation.level(u)?.eq(translation.level(v)?),
            UniverseConstraint::Max(w, u, v) => {
                let w = translation.level(w)?;
                let u = translation.level(u)?;
                let v = translation.level(v)?;
                w.eq(u.ge(&v).ite(&u, &v))
            }
            UniverseConstraint::Successor(v, u) => {
                let v = translation.level(v)?;
                let u = translation.level(u)?;
                v.eq(Int::add(&[&u, &Int::from_u64(1)]))
            }
        };
        solver.assert(assertion);
    }
    for variable in translation.variables.values() {
        solver.assert(variable.ge(Int::from_u64(0)));
        // Finite search bounds are explicit. A model outside the independently
        // evaluable u64 range is unsupported, never narrowed into that range.
        solver.assert(variable.le(Int::from_u64(u64::MAX)));
    }
    match solver.check() {
        SatResult::Unsat => {
            return Err("universe constraints have no model within search bounds".into());
        }
        SatResult::Unknown => {
            return Err("universe constraint search is undetermined or exhausted".into());
        }
        SatResult::Sat => {}
    }
    let model = solver
        .get_model()
        .ok_or_else(|| Text::from("universe solver returned no model"))?;
    let mut assignment = Map::new();
    for (id, variable) in &translation.variables {
        let value = model
            .eval(variable, true)
            .and_then(|value| value.as_u64())
            .ok_or_else(|| {
                Text::from("universe model is not an exact nonnegative u64 assignment")
            })?;
        assignment.insert(*id, value);
    }
    validate_assignment_with_limits(constraints, &assignment, limits)
}

/// Independent acceptance boundary, also usable for externally supplied models.
/// No SMT terms, model completion or constraint rewriting is used here.
pub fn validate_assignment(
    constraints: &[UniverseConstraint],
    assignment: &Map<u32, u64>,
) -> Result<UniverseSubstitution, Text> {
    validate_assignment_with_limits(constraints, assignment, SolverLimits::default())
}

fn validate_assignment_with_limits(
    constraints: &[UniverseConstraint],
    assignment: &Map<u32, u64>,
    limits: SolverLimits,
) -> Result<UniverseSubstitution, Text> {
    // Refuse excessive outer collections before walking any expression or
    // constructing structural levels from externally supplied model values.
    if constraints.len() > limits.max_constraints || assignment.len() > limits.max_variables {
        return Err("universe model validation collection limit exceeded".into());
    }
    let mut remaining_nodes = limits.max_expression_nodes;
    let mut variables = Set::new();
    for constraint in constraints {
        for level in constraint.levels() {
            let mut work = List::from_single(level);
            let mut local_nodes = 0;
            while let Some(level) = work.pop() {
                if remaining_nodes == 0 || local_nodes >= crate::universe_level::MAX_LEVEL_NODES {
                    return Err("universe model validation expression limit exceeded".into());
                }
                remaining_nodes -= 1;
                local_nodes += 1;
                match level {
                    UniverseLevel::Concrete(_) => {}
                    UniverseLevel::Variable(id) => {
                        variables.insert(*id);
                        if variables.len() > limits.max_variables {
                            return Err("universe model validation variable limit exceeded".into());
                        }
                    }
                    UniverseLevel::Succ(inner) => work.push(inner),
                    UniverseLevel::Max(left, right) => {
                        work.push(left);
                        work.push(right);
                    }
                }
            }
        }
    }
    let mut model_nodes = limits.max_expression_nodes as u64;
    for value in assignment.values() {
        let nodes = value
            .saturating_sub(u64::from(u32::MAX))
            .checked_add(1)
            .ok_or_else(|| Text::from("universe model representation overflow"))?;
        if nodes > crate::universe_level::MAX_LEVEL_NODES as u64 || nodes > model_nodes {
            return Err("universe model representation limit exceeded".into());
        }
        model_nodes -= nodes;
    }
    if !constraints
        .iter()
        .all(|constraint| constraint.evaluate(assignment) == Some(true))
    {
        return Err("universe model failed exact constraint evaluation".into());
    }
    let mut substitution = UniverseSubstitution::new();
    for (id, value) in assignment {
        let level = UniverseLevel::from_value(*value).ok_or_else(|| {
            Text::from("universe model level exceeds structural representation limits")
        })?;
        substitution.insert(*id, level)?;
    }
    if !constraints
        .iter()
        .all(|constraint| constraint.is_satisfied(&substitution))
    {
        return Err("universe substitution failed exact constraint evaluation".into());
    }
    Ok(substitution)
}

pub fn check_constraint(constraint: &UniverseConstraint, subst: &UniverseSubstitution) -> bool {
    constraint.is_satisfied(subst)
}

struct Translation {
    variables: Map<u32, Int>,
    remaining_nodes: usize,
    max_variables: usize,
}

impl Translation {
    fn level(&mut self, level: &UniverseLevel) -> Result<Int, Text> {
        enum Work<'a> {
            Enter(&'a UniverseLevel),
            Successor,
            Maximum,
        }
        let mut work = List::from_single(Work::Enter(level));
        let mut values = List::new();
        let mut local_nodes = 0usize;
        while let Some(item) = work.pop() {
            match item {
                Work::Enter(level) => {
                    if self.remaining_nodes == 0
                        || local_nodes >= crate::universe_level::MAX_LEVEL_NODES
                    {
                        return Err("universe expression search limit exceeded".into());
                    }
                    self.remaining_nodes -= 1;
                    local_nodes += 1;
                    match level {
                        UniverseLevel::Concrete(value) => {
                            values.push(Int::from_u64(u64::from(*value)))
                        }
                        UniverseLevel::Variable(id) => {
                            if !self.variables.contains_key(id) {
                                if self.variables.len() >= self.max_variables {
                                    return Err("universe variable search limit exceeded".into());
                                }
                                // Z3 integer symbols accept only signed 31-bit IDs.
                                // Decimal names preserve every u32 identity injectively.
                                self.variables
                                    .insert(*id, Int::new_const(format!("universe!{id}")));
                            }
                            values.push(self.variables[id].clone());
                        }
                        UniverseLevel::Succ(inner) => {
                            work.push(Work::Successor);
                            work.push(Work::Enter(inner));
                        }
                        UniverseLevel::Max(left, right) => {
                            work.push(Work::Maximum);
                            work.push(Work::Enter(right));
                            work.push(Work::Enter(left));
                        }
                    }
                }
                Work::Successor => {
                    let inner = values
                        .pop()
                        .ok_or_else(|| Text::from("invalid universe expression"))?;
                    values.push(Int::add(&[&inner, &Int::from_u64(1)]));
                }
                Work::Maximum => {
                    let right = values
                        .pop()
                        .ok_or_else(|| Text::from("invalid universe expression"))?;
                    let left = values
                        .pop()
                        .ok_or_else(|| Text::from("invalid universe expression"))?;
                    values.push(left.ge(&right).ite(&left, &right));
                }
            }
        }
        if values.len() != 1 {
            return Err("invalid universe expression result".into());
        }
        values
            .pop()
            .ok_or_else(|| "missing universe expression result".into())
    }
}
