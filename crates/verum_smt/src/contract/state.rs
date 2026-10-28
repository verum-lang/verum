//! Loop and frame obligations over a checked, ordered source transition.
use super::{ContractError, ContractResult};
use crate::context::Context;
use crate::cost::{CostMeasurement, VerificationCost};
use crate::ranking::{RankingFunction, RankingTransition};
use crate::scalar_transition::ScalarState;
use verum_ast::{Expr, Literal, Span, Type};
use verum_common::{List, Text};
use z3::{
    SatResult,
    ast::{Ast, Bool},
};

fn truth(value: bool) -> Expr {
    Expr::literal(Literal::bool(value, Span::dummy()))
}

/// Verify a loop invariant with no entry assumptions beyond declared scalar types.
/// Use [`verify_loop_invariant_with_precondition`] for a constrained entry state.
pub fn verify_loop_invariant(
    context: &Context,
    invariant: &Expr,
    state_variables: &[(Text, Type)],
    loop_body: &Expr,
    exit_condition: &Expr,
    postcondition: &Expr,
) -> ContractResult<VerificationCost> {
    verify_loop_invariant_with_precondition(
        context,
        &truth(true),
        invariant,
        state_variables,
        loop_body,
        exit_condition,
        postcondition,
    )
}

/// Check initialization, guarded body safety and invariant preservation, and
/// exit sufficiency. The source subset is finite structured Int/Bool code;
/// unsupported effects return errors, including on syntactically dead paths.
/// Specification arithmetic is mathematical; executed Int intermediates must
/// stay within signed 64-bit bounds. This returns solver evidence, not a kernel proof.
pub fn verify_loop_invariant_with_precondition(
    context: &Context,
    precondition: &Expr,
    invariant: &Expr,
    state_variables: &[(Text, Type)],
    loop_body: &Expr,
    exit_condition: &Expr,
    postcondition: &Expr,
) -> ContractResult<VerificationCost> {
    let measurement = CostMeasurement::start("loop_invariant");
    let before = ScalarState::new(state_variables)?;
    let entry = before.predicate(precondition)?;
    let invariant = before.predicate(invariant)?;
    let (exit, exit_safe) = before.condition(exit_condition)?;
    let post = before.predicate(postcondition)?;
    let (after, body_safe) = before.execute(loop_body)?;
    let substitutions: List<_> = before.values.iter().zip(after.values.iter()).collect();
    let invariant_after = invariant.substitute(&substitutions);
    let domain = Bool::and(&[before.bounds(), invariant.clone()]);
    // Lower every path before solving. Failed translations and discarded effects
    // must never turn into inconsistent transition assumptions.
    prove(
        context,
        Bool::and(&[before.bounds(), entry]).implies(&invariant),
        "loop invariant does not hold initially",
    )?;
    prove(
        context,
        domain.implies(exit_safe),
        "loop exit condition can overflow",
    )?;
    prove(
        context,
        Bool::and(&[domain.clone(), exit.not()]).implies(Bool::and(&[body_safe, invariant_after])),
        "loop body can overflow or violate the invariant",
    )?;
    prove(
        context,
        Bool::and(&[domain, exit]).implies(post),
        "invariant and exit do not imply postcondition",
    )?;
    Ok(measurement.finish(true))
}

/// Verify an unconditional loop over all declared states. No domain or exit
/// condition is inferred from the candidate rank or from assignments.
pub fn verify_termination(
    context: &Context,
    ranking_function: &Expr,
    loop_vars: &[(Text, Type)],
    loop_body: &Expr,
) -> ContractResult<VerificationCost> {
    verify_termination_in_domain(
        context,
        ranking_function,
        loop_vars,
        loop_body,
        &truth(true),
        &truth(false),
    )
}

/// Verify a candidate rank for a scalar loop with an explicit invariant domain
/// and exit condition. Establishing that execution enters this domain is a
/// separate obligation, supplied by `verify_loop_invariant_with_precondition`.
/// Proves intermediate arithmetic safety, domain closure and strict decrease;
/// successor nonnegativity is a conclusion, never an assumed filter.
pub fn verify_termination_in_domain(
    context: &Context,
    ranking_function: &Expr,
    loop_vars: &[(Text, Type)],
    loop_body: &Expr,
    invariant: &Expr,
    exit_condition: &Expr,
) -> ContractResult<VerificationCost> {
    let measurement = CostMeasurement::start("termination_check");
    let before = ScalarState::new(loop_vars)?;
    let domain = Bool::and(&[before.bounds(), before.predicate(invariant)?]);
    let (exit, exit_safe) = before.condition(exit_condition)?;
    let rank = before
        .term(ranking_function)?
        .as_int()
        .ok_or_else(|| ContractError::InvalidContract("ranking function must be integer".into()))?;
    let (after, safe) = before.execute(loop_body)?;
    prove(
        context,
        domain.implies(exit_safe),
        "loop exit condition can overflow",
    )?;
    prove(
        context,
        Bool::and(&[&domain, &exit.not()]).implies(safe),
        "loop body can overflow",
    )?;
    let verified = RankingFunction {
        expression: rank,
        well_founded_constraint: Bool::from_bool(true),
    }
    .verify(
        &before.values,
        &domain,
        &[RankingTransition {
            guard: exit.not(),
            arguments: after.values,
        }],
        context.config().clone(),
    )
    .map_err(|message| failure(message.to_string()))?;
    if !verified {
        return Err(failure(
            "loop rank is negative, fails to decrease, or leaves its domain".into(),
        ));
    }
    Ok(measurement.finish(true))
}

/// Prove that all state outside `modifies_vars` is unchanged on return, with
/// no entry assumptions beyond the declared scalar types.
pub fn verify_frame_condition(
    context: &Context,
    modifies_vars: &[Text],
    all_vars: &[(Text, Type)],
    function_body: &Expr,
) -> ContractResult<VerificationCost> {
    verify_frame_condition_with_precondition(
        context,
        &truth(true),
        modifies_vars,
        all_vars,
        function_body,
    )
}

/// Check a scalar frame against the complete ordered body under an explicit
/// entry predicate. Intermediate overflow and unsupported effects are refused
/// even if every supplied state variable appears in the modifies clause.
pub fn verify_frame_condition_with_precondition(
    context: &Context,
    precondition: &Expr,
    modifies_vars: &[Text],
    all_vars: &[(Text, Type)],
    function_body: &Expr,
) -> ContractResult<VerificationCost> {
    let measurement = CostMeasurement::start("frame_condition");
    if modifies_vars
        .iter()
        .any(|name| !all_vars.iter().any(|(declared, _)| name == declared))
    {
        return Err(ContractError::InvalidContract(
            "modifies clause names an undeclared state variable".into(),
        ));
    }
    let before = ScalarState::new(all_vars)?;
    let domain = Bool::and(&[before.bounds(), before.predicate(precondition)?]);
    let (after, safe) = before.execute(function_body)?;
    prove(context, domain.implies(safe), "frame body can overflow")?;
    for (index, (name, _)) in all_vars.iter().enumerate() {
        if !modifies_vars.contains(name) {
            prove(
                context,
                domain.implies(before.values[index].eq(&after.values[index])),
                &format!("variable '{name}' changes outside the modifies clause"),
            )?;
        }
    }
    Ok(measurement.finish(true))
}

fn failure(message: String) -> ContractError {
    ContractError::VerificationFailed {
        message,
        counterexample: None,
        suggestions: List::new(),
    }
}

fn prove(context: &Context, proposition: Bool, message: &str) -> ContractResult<()> {
    let solver = context.solver();
    context.assert(&solver, &proposition.not());
    match context.check(&solver) {
        SatResult::Unsat => Ok(()),
        SatResult::Sat => Err(failure(message.into())),
        SatResult::Unknown => Err(failure(format!(
            "loop obligation unknown: {}",
            solver.get_reason_unknown().unwrap_or_default()
        ))),
    }
}
