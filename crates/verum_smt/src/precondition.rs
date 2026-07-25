//! Precondition handling and verification.
//!
//! Preconditions are constraints that must hold at function entry.
//! They represent caller obligations and are assumed during verification.
//!
//! Verum uses contract literals (`contract#"requires ...; ensures ...;"`) embedded
//! in functions annotated with `@verify(proof)`. Preconditions (requires clauses) are
//! translated to SMT assertions that the caller must satisfy. At verification time,
//! preconditions are assumed true in the function body, while postconditions become
//! proof obligations. Three verification modes exist: `@verify(runtime)` inserts
//! runtime checks, `@verify(static)` uses dataflow analysis, and `@verify(proof)`
//! uses SMT solvers (Z3/CVC5) for full formal verification with zero runtime cost.

use crate::rsl_parser::RslClause;
use crate::translate::{TranslationError, Translator};
use verum_ast::Expr;
use verum_common::Text;
use verum_common::ToText;

/// Result type for precondition operations.
pub type PreconditionResult<T> = Result<T, PreconditionError>;

/// Errors that can occur during precondition handling.
#[derive(Debug, thiserror::Error)]
pub enum PreconditionError {
    /// Translation error
    #[error("translation error: {0}")]
    Translation(#[from] TranslationError),

    /// Invalid precondition
    #[error("invalid precondition: {0}")]
    Invalid(Text),

    /// Type error
    #[error("precondition type error: {0}")]
    TypeError(Text),
}

/// Assert a precondition in the SMT solver.
///
/// Preconditions are assumed to hold - they constrain the input space
/// but are not verified. It's the caller's responsibility to ensure them.
///
/// # Arguments
/// * `translator` - The SMT translator
/// * `solver` - The Z3 solver
/// * `clause` - The precondition clause to assert
///
/// # Returns
/// * `Ok(())` if the precondition was successfully asserted
/// * `Err(PreconditionError)` if translation or assertion failed
pub fn assert_precondition(
    translator: &Translator<'_>,
    solver: &z3::Solver,
    clause: &RslClause,
) -> PreconditionResult<()> {
    // Translate the precondition expression to Z3
    let z3_expr = translator.translate_expr(&clause.expr)?;

    // Ensure it's a boolean expression
    let z3_bool = z3_expr
        .as_bool()
        .ok_or_else(|| PreconditionError::TypeError("precondition must be boolean".to_text()))?;

    // Assert the precondition (assumed to hold)
    solver.assert(&z3_bool);

    Ok(())
}

/// Assert multiple preconditions in the SMT solver.
///
/// This is a convenience function for asserting all preconditions from a
/// contract specification.
pub fn assert_preconditions(
    translator: &Translator<'_>,
    solver: &z3::Solver,
    clauses: &[RslClause],
) -> PreconditionResult<()> {
    for clause in clauses {
        assert_precondition(translator, solver, clause)?;
    }
    Ok(())
}

/// Validate that a precondition doesn't use forbidden constructs.
///
/// Preconditions must not reference:
/// - `result` (not available at function entry)
/// - `old()` (meaningless for preconditions)
pub fn validate_precondition(expr: &Expr) -> PreconditionResult<()> {
    if contains_result(expr) {
        return Err(PreconditionError::Invalid(
            "precondition cannot reference 'result'".to_text(),
        ));
    }

    if contains_old(expr) {
        return Err(PreconditionError::Invalid(
            "precondition cannot use 'old()' function".to_text(),
        ));
    }

    Ok(())
}

/// Check if an expression references 'result'.
///
/// This is used to validate preconditions, which cannot reference `result`
/// since it's not available at function entry.
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
        verum_ast::ExprKind::Attenuate { context, .. } => {
            // Attenuate expressions restrict capability, recursively process the context
            contains_result(context)
        }
        _ => false,
    }
}

/// Check if an expression uses 'old()'.
///
/// This is used to validate preconditions, which cannot use `old()` since
/// it's meaningless at function entry (there is no prior state).
pub fn contains_old(expr: &Expr) -> bool {
    match &expr.kind {
        verum_ast::ExprKind::Call { func, args, .. } => {
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
        verum_ast::ExprKind::Attenuate { context, .. } => {
            // Attenuate expressions restrict capability, recursively process the context
            contains_old(context)
        }
        _ => false,
    }
}

/// Generate an informative error message for a violated precondition.
///
/// This is used when a caller fails to satisfy a precondition.
pub fn format_precondition_violation(clause: &RslClause, function_name: &str) -> Text {
    format!(
        "Precondition violated in call to '{}': {}",
        function_name,
        format_expr(&clause.expr)
    )
    .into()
}

/// Format an expression for display in error messages.
fn format_expr(expr: &Expr) -> Text {
    // Simplified formatting - in production, use proper pretty-printer
    format!("{:?}", expr).into()
}
