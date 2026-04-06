//! Postcondition handling and verification.
//!
//! Postconditions are constraints that the function guarantees to hold
//! at exit. They are what we verify using the SMT solver.
//!
//! Verum uses contract literals (`contract#"requires ...; ensures ...;"`) embedded
//! in functions annotated with `@verify(proof)`. Postconditions (ensures clauses) are
//! the proof obligations verified by the SMT solver. The weakest-precondition calculus
//! transforms postconditions backward through the function body to generate verification
//! conditions. Three verification modes exist: `@verify(runtime)` inserts runtime checks,
//! `@verify(static)` uses dataflow analysis, and `@verify(proof)` uses SMT solvers
//! (Z3/CVC5) for full formal verification with zero runtime cost when proven.

use crate::counterexample::{CounterExample, CounterExampleExtractor};
use crate::rsl_parser::RslClause;
use crate::translate::{TranslationError, Translator};
use verum_ast::Expr;
use verum_common::{List, Map, Maybe, Text};
use verum_common::ToText;

/// Result type for postcondition operations.
pub type PostconditionResult<T> = Result<T, PostconditionError>;

/// Errors that can occur during postcondition handling.
#[derive(Debug, thiserror::Error)]
pub enum PostconditionError {
    /// Translation error
    #[error("translation error: {0}")]
    Translation(#[from] TranslationError),

    /// Postcondition violated
    #[error("postcondition violated: {message}")]
    Violated {
        message: String,
        counterexample: Option<CounterExample>,
    },

    /// Invalid postcondition
    #[error("invalid postcondition: {0}")]
    Invalid(Text),

    /// Type error
    #[error("postcondition type error: {0}")]
    TypeError(Text),

    /// Solver error
    #[error("solver error: {0}")]
    SolverError(Text),
}

/// Verify a postcondition using the SMT solver.
///
/// Postconditions must be proven to hold given the preconditions and
/// function semantics. We check if there exists an input where the
/// preconditions hold but the postcondition doesn't.
///
/// # Arguments
/// * `translator` - The SMT translator
/// * `solver` - The Z3 solver (with preconditions already asserted)
/// * `clause` - The postcondition clause to verify
/// * `input_vars` - Names of input variables for counterexample extraction
///
/// # Returns
/// * `Ok(())` if the postcondition is proven
/// * `Err(PostconditionError)` if verification fails
pub fn verify_postcondition(
    translator: &Translator<'_>,
    solver: &z3::Solver,
    clause: &RslClause,
    input_vars: &[Text],
) -> PostconditionResult<()> {
    // Push a new solver scope (so we can pop after checking)
    solver.push();

    // Translate the postcondition expression to Z3
    let z3_expr = translator.translate_expr(&clause.expr)?;

    // Ensure it's a boolean expression
    let z3_bool = z3_expr
        .as_bool()
        .ok_or_else(|| PostconditionError::TypeError("postcondition must be boolean".to_text()))?;

    // Assert the NEGATION of the postcondition
    // If SAT, we found a counterexample where the postcondition is violated
    solver.assert(z3_bool.not());

    // Check satisfiability
    let check_result = solver.check();

    match check_result {
        z3::SatResult::Unsat => {
            // No counterexample exists - postcondition always holds!
            solver.pop(1);
            Ok(())
        }

        z3::SatResult::Sat => {
            // Found a counterexample - postcondition can be violated
            let model = solver
                .get_model()
                .ok_or_else(|| PostconditionError::SolverError("no model available".to_text()))?;

            // Extract counterexample
            let extractor = CounterExampleExtractor::new(&model);
            let counterexample = extractor.extract(input_vars, &format!("{:?}", clause.expr));

            solver.pop(1);

            Err(PostconditionError::Violated {
                message: format!("postcondition violated: {:?}", clause.expr),
                counterexample: Some(counterexample),
            })
        }

        z3::SatResult::Unknown => {
            // Solver couldn't determine result (timeout or too complex)
            solver.pop(1);

            Err(PostconditionError::SolverError(
                "solver returned unknown (timeout or undecidable)".to_text(),
            ))
        }
    }
}

/// Verify multiple postconditions.
///
/// Returns the first violation found, or Ok if all pass.
pub fn verify_postconditions(
    translator: &Translator<'_>,
    solver: &z3::Solver,
    clauses: &[RslClause],
    input_vars: &[Text],
) -> PostconditionResult<()> {
    for clause in clauses {
        verify_postcondition(translator, solver, clause, input_vars)?;
    }
    Ok(())
}

/// Validate that a postcondition uses valid constructs.
///
/// Postconditions can reference:
/// - `result` (the return value)
/// - `old(expr)` (values at function entry)
/// - All input parameters
pub fn validate_postcondition(_expr: &Expr) -> PostconditionResult<()> {
    // Postconditions can use result and old() - no restrictions
    Ok(())
}

/// Handle `old()` expressions in postconditions.
///
/// The `old()` function captures the value of an expression at function entry.
/// During verification, we need to substitute `old(x)` with the original value
/// of `x` before any modifications.
///
/// This struct tracks the mapping from expressions to their "old" values.
#[derive(Debug, Clone)]
pub struct OldValueTracker {
    /// Map from variable names to their "old" Z3 variables
    old_values: Map<Text, z3::ast::Dynamic>,
}

impl OldValueTracker {
    /// Create a new old value tracker.
    pub fn new() -> Self {
        Self {
            old_values: Map::new(),
        }
    }

    /// Capture the current value of a variable as its "old" value.
    pub fn capture(&mut self, name: Text, value: z3::ast::Dynamic) {
        self.old_values.insert(name, value);
    }

    /// Get the "old" value of a variable.
    pub fn get(&self, name: &str) -> Maybe<&z3::ast::Dynamic> {
        self.old_values.get(&name.to_text())
    }

    /// Check if an "old" value exists for a variable.
    pub fn contains(&self, name: &str) -> bool {
        self.old_values.contains_key(&name.to_text())
    }
}

impl Default for OldValueTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract `result` references from a postcondition.
///
/// Returns true if the expression references `result`.
pub fn references_result(expr: &Expr) -> bool {
    match &expr.kind {
        verum_ast::ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                ident.as_str() == "result"
            } else {
                false
            }
        }
        verum_ast::ExprKind::Binary { left, right, .. } => {
            references_result(left) || references_result(right)
        }
        verum_ast::ExprKind::Unary { expr, .. } => references_result(expr),
        verum_ast::ExprKind::Call { func, args, .. } => {
            references_result(func) || args.iter().any(references_result)
        }
        verum_ast::ExprKind::MethodCall { receiver, args, .. } => {
            references_result(receiver) || args.iter().any(references_result)
        }
        verum_ast::ExprKind::Field { expr, .. } => references_result(expr),
        verum_ast::ExprKind::Index { expr, index } => {
            references_result(expr) || references_result(index)
        }
        verum_ast::ExprKind::Paren(inner) => references_result(inner),
        verum_ast::ExprKind::Attenuate { context, .. } => {
            // Attenuate expressions restrict capability, recursively process the context
            references_result(context)
        }
        _ => false,
    }
}

/// Extract all `old()` calls from a postcondition.
///
/// Returns a list of expressions wrapped in `old()`.
pub fn extract_old_calls(expr: &Expr) -> List<Expr> {
    let mut old_calls = List::new();
    extract_old_calls_recursive(expr, &mut old_calls);
    old_calls
}

fn extract_old_calls_recursive(expr: &Expr, result: &mut List<Expr>) {
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

            if is_old && args.len() == 1 {
                result.push(args[0].clone());
            } else {
                // Recursively search arguments
                for arg in args {
                    extract_old_calls_recursive(arg, result);
                }
            }
        }
        verum_ast::ExprKind::Binary { left, right, .. } => {
            extract_old_calls_recursive(left, result);
            extract_old_calls_recursive(right, result);
        }
        verum_ast::ExprKind::Unary { expr, .. } => {
            extract_old_calls_recursive(expr, result);
        }
        verum_ast::ExprKind::MethodCall { receiver, args, .. } => {
            extract_old_calls_recursive(receiver, result);
            for arg in args {
                extract_old_calls_recursive(arg, result);
            }
        }
        verum_ast::ExprKind::Field { expr, .. } => {
            extract_old_calls_recursive(expr, result);
        }
        verum_ast::ExprKind::Index { expr, index } => {
            extract_old_calls_recursive(expr, result);
            extract_old_calls_recursive(index, result);
        }
        verum_ast::ExprKind::Paren(inner) => {
            extract_old_calls_recursive(inner, result);
        }
        verum_ast::ExprKind::Attenuate { context, .. } => {
            // Attenuate expressions restrict capability, recursively process the context
            extract_old_calls_recursive(context, result);
        }
        _ => {}
    }
}
