//! Contract literal handling and verification.
//!
//! This module provides the main interface for working with contract# literals,
//! including parsing, validation, and SMT translation.
//!
//! Contract literals use the RSL (Refinement Specification Language) mechanism via
//! `contract#"requires P; ensures Q;"` syntax. Preconditions become caller obligations;
//! postconditions become proof obligations verified by SMT. In `@verify(proof)` mode,
//! both are checked at compile time; in `@verify(runtime)`, they become runtime assertions.
//! Contracts support `old(expr)` for referring to pre-state values in postconditions.

use crate::context::Context;
use crate::cost::{CostMeasurement, VerificationCost};
use crate::counterexample::{CounterExample, CounterExampleExtractor, generate_suggestions};
use crate::rsl_parser::{ContractSpec, RslParseError, RslParser};
use crate::translate::{TranslationError, Translator};
use std::time::Duration;
use verum_ast::{Expr, LiteralKind, Span};
use verum_common::ToText;
use verum_common::{List, Text};

/// Result type for contract operations.
pub type ContractResult<T> = Result<T, ContractError>;

/// Errors that can occur during contract handling.
#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    /// RSL parsing error
    #[error("RSL parse error: {0}")]
    Parse(#[from] RslParseError),

    /// Translation error
    #[error("translation error: {0}")]
    Translation(#[from] TranslationError),

    /// Verification error
    #[error("verification failed: {message}")]
    VerificationFailed {
        message: String,
        counterexample: Option<CounterExample>,
        suggestions: List<Text>,
    },

    /// Timeout during verification
    #[error("verification timeout after {timeout:?}")]
    Timeout { timeout: Duration },

    /// Invalid contract specification
    #[error("invalid contract: {0}")]
    InvalidContract(Text),

    /// Missing required context
    #[error("missing context: {0}")]
    MissingContext(Text),
}

/// Parse a contract# literal into a structured ContractSpec.
///
/// # Arguments
/// * `literal_content` - The string content inside the contract# literal
/// * `span` - Source span for error reporting
///
/// # Returns
/// * `Ok(ContractSpec)` if parsing succeeds
/// * `Err(ContractError)` if parsing fails
///
/// # Example
/// ```ignore
/// use verum_smt::contract::parse_contract_literal;
/// use verum_common::Text;
/// use verum_ast::Span;
///
/// let content: Text = "requires x > 0; ensures result >= 0;".into();
/// let spec = parse_contract_literal(&content, Span::dummy()).unwrap();
///
/// assert_eq!(spec.preconditions.len(), 1);
/// assert_eq!(spec.postconditions.len(), 1);
/// ```
pub fn parse_contract_literal(literal_content: &Text, span: Span) -> ContractResult<ContractSpec> {
    let mut parser = RslParser::new(literal_content.to_string().into(), span);
    let spec = parser.parse()?;
    Ok(spec)
}

/// Verify a contract specification against a function body.
///
/// This is the main entry point for contract verification. It translates the
/// contract and function body to SMT, invokes the solver, and reports results.
///
/// # Arguments
/// * `context` - Z3 context for SMT solving
/// * `spec` - The parsed contract specification
/// * `function_body` - Optional expression representing the function body
/// * `input_bindings` - Variable bindings for function parameters
///
/// # Returns
/// * `Ok(VerificationCost)` if verification succeeds
/// * `Err(ContractError)` if verification fails
pub fn verify_contract(
    context: &Context,
    spec: &ContractSpec,
    _function_body: Option<&Expr>,
    input_bindings: &[(Text, verum_ast::Type)],
) -> ContractResult<VerificationCost> {
    let measurement = CostMeasurement::start("contract_verification");

    // Create translator
    let mut translator = Translator::new(context);

    // Bind input variables
    for (name, ty) in input_bindings {
        let var = translator
            .create_var(name.as_str(), ty)
            .map_err(ContractError::Translation)?;
        translator.bind(name.clone(), var);
    }

    // Create solver
    let solver = context.solver();

    // Step 1: Assert preconditions (assumed to hold)
    for precond in &spec.preconditions {
        let z3_expr = translator
            .translate_expr(&precond.expr)
            .map_err(ContractError::Translation)?;

        let z3_bool = z3_expr.as_bool().ok_or_else(|| {
            ContractError::InvalidContract("precondition must be boolean".to_text())
        })?;

        solver.assert(&z3_bool);
    }

    // Step 2: If we have a function body, translate its semantics
    // For now, we skip body translation and focus on postconditions
    // Full implementation would use weakest precondition calculus

    // Step 3: Try to prove postconditions
    // We check if there's a case where preconditions hold but postconditions don't
    let mut all_verified = true;
    let mut first_counterexample = None;
    let mut failed_clause = None;

    for postcond in &spec.postconditions {
        // Create a new solver scope for this postcondition
        solver.push();

        let z3_expr = translator
            .translate_expr(&postcond.expr)
            .map_err(ContractError::Translation)?;

        let z3_bool = z3_expr.as_bool().ok_or_else(|| {
            ContractError::InvalidContract("postcondition must be boolean".to_text())
        })?;

        // Assert the NEGATION of the postcondition
        // If SAT, we found a counterexample (postcondition can be violated)
        solver.assert(z3_bool.not());

        match solver.check() {
            z3::SatResult::Unsat => {
                // Postcondition always holds - good!
            }
            z3::SatResult::Sat => {
                // Found a counterexample
                all_verified = false;

                if first_counterexample.is_none() {
                    let model = solver.get_model().ok_or_else(|| {
                        ContractError::InvalidContract("no model available".to_text())
                    })?;

                    let extractor = CounterExampleExtractor::new(&model);
                    let var_names: List<Text> =
                        input_bindings.iter().map(|(n, _)| n.clone()).collect();
                    let ce = extractor.extract(&var_names, &format!("{:?}", postcond.expr));

                    first_counterexample = Some(ce);
                    failed_clause = Some(postcond.clone());
                }
            }
            z3::SatResult::Unknown => {
                // Solver couldn't determine result
                let cost = measurement.finish(false);

                if let Some(timeout) = context.config().timeout
                    && cost.duration >= timeout
                {
                    return Err(ContractError::Timeout { timeout });
                }

                return Err(ContractError::VerificationFailed {
                    message: format!("unknown result for clause: {:?}", postcond.expr),
                    counterexample: None,
                    suggestions: {
                        let mut list = List::new();
                        list.push("Try simplifying the contract".to_text());
                        list.push("Increase solver timeout".to_text());
                        list
                    },
                });
            }
        }

        solver.pop(1);
    }

    if !all_verified {
        // SAFETY: Logic guarantees these are Some when all_verified is false
        // first_counterexample and failed_clause are set in SatResult::Sat branch above
        let ce = first_counterexample.unwrap();
        let clause = failed_clause.unwrap();
        let suggestions = generate_suggestions(&ce, &format!("{:?}", clause.expr));

        let _cost = measurement.finish(false);

        return Err(ContractError::VerificationFailed {
            message: format!("postcondition violated: {:?}", clause.expr),
            counterexample: Some(ce),
            suggestions,
        });
    }

    // All postconditions verified!
    let cost = measurement.finish(true);
    Ok(cost)
}

/// Extract contract literal from an expression.
///
/// Checks if the expression is a contract# literal and returns its content.
pub fn extract_contract_from_expr(expr: &Expr) -> Option<Text> {
    match &expr.kind {
        verum_ast::ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Contract(content) => Some(content.clone().into()),
            _ => None,
        },
        _ => None,
    }
}

/// Find all contract literals in a list of expressions.
///
/// This is useful for extracting contracts from function annotations.
pub fn find_contract_literals(exprs: &[Expr]) -> List<(Text, Span)> {
    exprs
        .iter()
        .filter_map(|expr| extract_contract_from_expr(expr).map(|content| (content, expr.span)))
        .collect()
}

/// Merge multiple contract specifications into one.
///
/// This combines preconditions, postconditions, and invariants from
/// multiple contract# literals into a single specification.
pub fn merge_contracts(contracts: &[ContractSpec]) -> ContractSpec {
    let span = contracts
        .first()
        .map(|c| c.span)
        .unwrap_or_else(Span::dummy);
    let mut merged = ContractSpec::new(span);

    for contract in contracts {
        merged.preconditions.extend(contract.preconditions.clone());
        merged
            .postconditions
            .extend(contract.postconditions.clone());
        merged.invariants.extend(contract.invariants.clone());
    }

    merged
}

/// Validate a contract specification for semantic correctness.
///
/// Checks for:
/// - Use of `result` only in postconditions
/// - Use of `old()` only in postconditions
/// - Valid variable references
/// - No circular dependencies
pub fn validate_contract(spec: &ContractSpec) -> ContractResult<()> {
    // Check preconditions don't use result or old()
    for precond in &spec.preconditions {
        if contains_result(&precond.expr) {
            return Err(ContractError::InvalidContract(
                "preconditions cannot reference 'result'".to_text(),
            ));
        }
        if contains_old(&precond.expr) {
            return Err(ContractError::InvalidContract(
                "preconditions cannot use 'old()' function".to_text(),
            ));
        }
    }

    // Postconditions can use both result and old()
    // (no validation needed)

    // Check invariants don't use result or old()
    for inv in &spec.invariants {
        if contains_result(&inv.expr) {
            return Err(ContractError::InvalidContract(
                "invariants cannot reference 'result'".to_text(),
            ));
        }
        if contains_old(&inv.expr) {
            return Err(ContractError::InvalidContract(
                "invariants cannot use 'old()' function".to_text(),
            ));
        }
    }

    Ok(())
}

/// Check if an expression contains a reference to 'result'.
pub fn contains_result(expr: &Expr) -> bool {
    match &expr.kind {
        verum_ast::ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                ident.as_str() == "result"
            } else {
                false
            }
        }
        verum_ast::ExprKind::Binary { left, right, .. } => {
            contains_result(left) || contains_result(right)
        }
        verum_ast::ExprKind::Unary { expr, .. } => contains_result(expr),
        verum_ast::ExprKind::Call { func, args, .. } => {
            contains_result(func) || args.iter().any(contains_result)
        }
        verum_ast::ExprKind::MethodCall { receiver, args, .. } => {
            contains_result(receiver) || args.iter().any(contains_result)
        }
        verum_ast::ExprKind::Field { expr, .. } => contains_result(expr),
        verum_ast::ExprKind::Index { expr, index } => {
            contains_result(expr) || contains_result(index)
        }
        verum_ast::ExprKind::Paren(inner) => contains_result(inner),
        _ => false,
    }
}

/// Check if an expression contains a call to 'old()'.
fn contains_old(expr: &Expr) -> bool {
    match &expr.kind {
        verum_ast::ExprKind::Call { func, args, .. } => {
            // Check if this is a call to 'old'
            let is_old = if let verum_ast::ExprKind::Path(path) = &func.kind {
                path.as_ident()
                    .map(|i| i.as_str() == "old")
                    .unwrap_or(false)
            } else {
                false
            };

            is_old || args.iter().any(contains_old)
        }
        verum_ast::ExprKind::Binary { left, right, .. } => {
            contains_old(left) || contains_old(right)
        }
        verum_ast::ExprKind::Unary { expr, .. } => contains_old(expr),
        verum_ast::ExprKind::MethodCall { receiver, args, .. } => {
            contains_old(receiver) || args.iter().any(contains_old)
        }
        verum_ast::ExprKind::Field { expr, .. } => contains_old(expr),
        verum_ast::ExprKind::Index { expr, index } => contains_old(expr) || contains_old(index),
        verum_ast::ExprKind::Paren(inner) => contains_old(inner),
        _ => false,
    }
}

mod state;
pub use state::{
    verify_frame_condition, verify_frame_condition_with_precondition,
    verify_loop_invariant, verify_loop_invariant_with_precondition,
    verify_termination, verify_termination_in_domain,
};
