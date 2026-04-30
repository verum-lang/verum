//! `@cfg(...)` predicate evaluation utilities.
//!
//! Extracted from `pipeline.rs` (#106 crate-split foundation) so the
//! cfg-evaluation surface is independently reviewable / testable and
//! pipeline.rs shrinks toward a thin orchestration layer. All
//! functions here are pure: no `&self`, no `&mut state`. They take
//! AST nodes + a target spec and return diagnostics-ready data.
//!
//! # Surface
//!
//!   * [`extract_cfg_predicates`] — walk an item's `@cfg(...)`
//!     attributes plus the module-path platform inference, return
//!     the list of predicate strings.
//!   * [`target_predicates_satisfied`] — evaluate that list against
//!     a `TargetSpec`. Non-target-axis predicates pass through so
//!     feature-flag / custom evaluators can compose downstream.
//!   * [`cfg_expr_to_predicate`] — single-expression conversion
//!     (used by both `@cfg` attributes and `if cfg!(...)` style).
//!   * Helpers `expr_to_ident_string` / `expr_to_string_literal`
//!     handle the two literal-shape arms.

use verum_ast::Expr;
use verum_common::{List, Text};
use verum_modules::ModulePath;

use crate::target_spec::TargetSpec;

/// Extract `@cfg(...)` predicates from item attributes and module
/// path. Returns a list of predicate strings (e.g.,
/// `target_os = "linux"`).
///
/// Also infers platform cfg from module paths containing platform
/// segments (e.g., `sys.darwin.io` implies `target_os = "macos"`).
pub fn extract_cfg_predicates(
    attributes: &[verum_ast::attr::Attribute],
    module_path: &ModulePath,
) -> List<Text> {
    let mut predicates = List::new();

    // Extract from @cfg(...) attributes on the item.
    for attr in attributes {
        if attr.name.as_str() == "cfg" {
            if let verum_common::Maybe::Some(ref args) = attr.args {
                for arg in args.iter() {
                    if let Some(pred) = cfg_expr_to_predicate(arg) {
                        predicates.push(pred);
                    }
                }
            }
        }
    }

    // Infer platform cfg from module path segments.
    let path_str = module_path.to_string();
    if path_str.contains(".darwin.") || path_str.ends_with(".darwin") {
        predicates.push(Text::from("target_os = \"macos\""));
    } else if path_str.contains(".linux.") || path_str.ends_with(".linux") {
        predicates.push(Text::from("target_os = \"linux\""));
    } else if path_str.contains(".windows.") || path_str.ends_with(".windows") {
        predicates.push(Text::from("target_os = \"windows\""));
    }

    predicates
}

/// Audit-E2: evaluate a list of `@cfg(...)` predicates against the
/// active compilation target. Returns `true` when every predicate
/// either matches the target (target_os / target_arch /
/// target_pointer_width / target_endian) or is a non-target-axis
/// predicate this evaluator doesn't recognise (kept conservative —
/// callers can opt in to a custom evaluator for feature flags via
/// `feature = "..."`).
pub fn target_predicates_satisfied(predicates: &[Text], target: &TargetSpec) -> bool {
    for pred in predicates.iter() {
        let textual = pred.as_str();
        if let Some(matched) = target.matches_textual(textual) {
            if !matched {
                return false;
            }
        }
        // Non-target predicates pass through; downstream
        // feature-flag evaluators can sharpen the decision.
    }
    true
}

/// Convert a `@cfg` expression argument to a predicate string.
///
/// Handles two shapes the grammar allows:
///   * `target_os = "linux"` — `Binary` with `BinOp::Assign`
///   * `unix` / `windows` — bare `Path` identifier
pub fn cfg_expr_to_predicate(expr: &Expr) -> Option<Text> {
    use verum_ast::expr::{BinOp, ExprKind};
    match &expr.kind {
        ExprKind::Binary { op, left, right } => {
            if matches!(op, BinOp::Assign) {
                let key = expr_to_ident_string(left)?;
                let val = expr_to_string_literal(right)?;
                Some(Text::from(format!("{} = \"{}\"", key, val)))
            } else {
                None
            }
        }
        ExprKind::Path(path) => {
            use verum_ast::ty::PathSegment;
            match path.segments.last()? {
                PathSegment::Name(ident) => Some(Text::from(ident.as_str())),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Extract identifier name from an expression. Used by
/// `cfg_expr_to_predicate` for the `key = "value"` left-hand side.
pub fn expr_to_ident_string(expr: &Expr) -> Option<String> {
    use verum_ast::ty::PathSegment;
    match &expr.kind {
        verum_ast::expr::ExprKind::Path(path) => match path.segments.last()? {
            PathSegment::Name(ident) => Some(ident.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// Extract string literal value from an expression. Used by
/// `cfg_expr_to_predicate` for the `key = "value"` right-hand side.
pub fn expr_to_string_literal(expr: &Expr) -> Option<String> {
    use verum_ast::literal::LiteralKind;
    match &expr.kind {
        verum_ast::expr::ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Text(string_lit) => Some(string_lit.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_predicates_satisfied_empty_list_passes() {
        let target = TargetSpec::host();
        let preds: Vec<Text> = Vec::new();
        assert!(target_predicates_satisfied(&preds, &target));
    }

    #[test]
    fn target_predicates_satisfied_unknown_predicate_passes() {
        let target = TargetSpec::host();
        let preds = vec![Text::from("feature = \"experimental\"")];
        assert!(target_predicates_satisfied(&preds, &target));
    }

    #[test]
    fn target_predicates_satisfied_target_os_match() {
        let target = TargetSpec::parse_triple("aarch64-unknown-linux-gnu");
        let preds = vec![Text::from("target_os = \"linux\"")];
        assert!(target_predicates_satisfied(&preds, &target));
        let preds_macos = vec![Text::from("target_os = \"macos\"")];
        assert!(!target_predicates_satisfied(&preds_macos, &target));
    }

    #[test]
    fn target_predicates_satisfied_arch_match() {
        let target = TargetSpec::parse_triple("riscv32-unknown-linux");
        let preds = vec![Text::from("target_arch = \"riscv32\"")];
        assert!(target_predicates_satisfied(&preds, &target));
        let preds_x86 = vec![Text::from("target_arch = \"x86_64\"")];
        assert!(!target_predicates_satisfied(&preds_x86, &target));
    }

    #[test]
    fn target_predicates_satisfied_pointer_width_match() {
        let target = TargetSpec::parse_triple("aarch64-unknown-linux-gnu");
        let preds = vec![Text::from("target_pointer_width = \"64\"")];
        assert!(target_predicates_satisfied(&preds, &target));
        let preds32 = vec![Text::from("target_pointer_width = \"32\"")];
        assert!(!target_predicates_satisfied(&preds32, &target));
    }

    #[test]
    fn target_predicates_satisfied_all_must_match() {
        let target = TargetSpec::parse_triple("aarch64-unknown-linux-gnu");
        // OS matches but arch doesn't → overall false.
        let preds = vec![
            Text::from("target_os = \"linux\""),
            Text::from("target_arch = \"x86_64\""),
        ];
        assert!(!target_predicates_satisfied(&preds, &target));
    }
}
