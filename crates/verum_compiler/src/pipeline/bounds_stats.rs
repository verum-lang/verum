//! AST-level bounds-statistics gathering for `phase_verify`.
//!
//! Pure read-only walks over the typed AST that count `Expr::Index`
//! sites for diagnostic statistics — never elimination. Actual
//! bounds-check elimination happens at MIR level in
//! `phases/verification_phase.rs` via dataflow analysis on the
//! Control Flow Graph (full def-use chains, dominance, etc.).
//!
//! # Why a separate file
//!
//! Extracted from `pipeline.rs` (#106 crate-split) so the
//! bounds-statistics surface is independently reviewable and
//! `pipeline.rs` shrinks toward a thin orchestration layer. All
//! functions here are pure: no `&mut self`, no `self.*` field
//! mutation. The single instance method `run_bounds_elimination_analysis`
//! is included here as part of the same cohesive concern; it
//! borrows `&self` only to dispatch through the static helpers.

use std::time::Instant;

use anyhow::Result;
use tracing::debug;

use verum_ast::{
    decl::{FunctionBody, FunctionDecl, ItemKind},
    expr::{Block, Expr, ExprKind},
    stmt::StmtKind,
    Module,
};

use super::CompilationPipeline;

impl<'s> CompilationPipeline<'s> {
    /// Run bounds elimination analysis at AST level (statistics gathering).
    ///
    /// This AST-level analysis collects statistics about array index
    /// accesses. The actual bounds check elimination happens at MIR
    /// level in `verification_phase.rs` which has access to full CFG
    /// and dataflow analysis.
    ///
    /// This pass is retained for early statistics and potential
    /// future AST-level optimisations.
    pub(super) fn run_bounds_elimination_analysis(
        &self,
        module: &Module,
    ) -> Result<()> {
        debug!("Running AST-level bounds statistics collection");
        let start = Instant::now();

        let mut total_checks = 0usize;
        let mut eliminated = 0usize;

        for item in module.items.iter() {
            if let ItemKind::Function(func) = &item.kind {
                if func.is_meta {
                    continue;
                }
                let func_stats = analyze_function_bounds_checks(func);
                total_checks += func_stats.0;
                eliminated += func_stats.1;
            }
        }

        let elapsed = start.elapsed();

        if total_checks > 0 {
            debug!(
                "Bounds elimination: {} / {} checks eliminated ({:.1}%) in {:.2}ms",
                eliminated,
                total_checks,
                (eliminated as f64 / total_checks as f64) * 100.0,
                elapsed.as_millis()
            );
        }

        Ok(())
    }
}

/// Count index accesses in a function for statistics.
///
/// Returns `(total_index_accesses, eliminated)`. The eliminated
/// component is always 0 at AST level — actual bounds-check
/// elimination is implemented at MIR level via
/// `BoundsCheckEliminator` and SMT-based proofs.
pub fn analyze_function_bounds_checks(func: &FunctionDecl) -> (usize, usize) {
    let mut total = 0;
    let eliminated = 0;

    if let Some(ref body) = func.body {
        let index_count = match body {
            FunctionBody::Block(block) => count_index_accesses(block),
            FunctionBody::Expr(expr) => count_index_in_expr(expr),
        };
        total = index_count;
    }

    (total, eliminated)
}

/// Count index-access expressions in a statement block.
pub fn count_index_accesses(block: &Block) -> usize {
    let mut count = 0;

    for stmt in &block.stmts {
        match &stmt.kind {
            StmtKind::Expr { expr, .. } => {
                count += count_index_in_expr(expr);
            }
            StmtKind::Let { value, .. } => {
                if let Some(init_expr) = value {
                    count += count_index_in_expr(init_expr);
                }
            }
            _ => {}
        }
    }

    if let Some(tail) = &block.expr {
        count += count_index_in_expr(tail);
    }

    count
}

/// Recursively count `ExprKind::Index` expressions.
pub fn count_index_in_expr(expr: &Expr) -> usize {
    let mut count = 0;

    match &expr.kind {
        ExprKind::Index { expr: inner, index } => {
            count += 1;
            count += count_index_in_expr(inner);
            count += count_index_in_expr(index);
        }
        ExprKind::Binary { left, right, .. } => {
            count += count_index_in_expr(left);
            count += count_index_in_expr(right);
        }
        ExprKind::Unary { expr: inner, .. } => {
            count += count_index_in_expr(inner);
        }
        ExprKind::Block(block) => {
            count += count_index_accesses(block);
        }
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            // Note: condition is IfCondition, not Expr, so we skip it for counting.
            count += count_index_accesses(then_branch);
            if let Some(else_expr) = else_branch {
                count += count_index_in_expr(else_expr);
            }
        }
        ExprKind::Call { args, .. } => {
            for arg in args {
                count += count_index_in_expr(arg);
            }
        }
        _ => {}
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::Block;
    use verum_common::List;

    fn empty_block() -> Block {
        Block {
            stmts: List::new(),
            expr: None,
            span: verum_ast::Span::dummy(),
        }
    }

    #[test]
    fn count_index_in_empty_block_is_zero() {
        let b = empty_block();
        assert_eq!(count_index_accesses(&b), 0);
    }
}
