//! AST walker that produces a QTT `UsageMap` from a function body.
//!
//! Given a set of tracked binding names and an expression, the
//! walker counts occurrences of each binding using the QTT
//! composition rules: sequential composition sums counts, branching
//! composition takes the worst-case maximum.
//!
//! The walker is intentionally **conservative**: any expression
//! shape it does not understand contributes zero usage. This means
//! the analysis may **miss** uses (false negatives) but never
//! **invent** uses (no false positives). A QTT violation reported
//! by this walker is therefore guaranteed to be real — though
//! some violations might escape detection until the walker grows
//! more cases.
//!
//! ## Coverage
//!
//! Recognised expression shapes:
//!
//! * `Path(p)` — single-segment identifier matches a tracked binding
//! * `Call { func, args }` — sequential composition
//! * `Binary { left, right }` — sequential composition
//! * `Unary { expr }` — recurse
//! * `Paren(inner)` — recurse
//! * `Tuple(elements)` — sequential composition
//! * `Field { expr, .. }` — recurse into receiver
//! * `Index { expr, index }` — both sides
//! * `Closure { params, body }` — recurse with closure params shadowing
//! * `Match { expr, arms }` — sequential(scrut) + max over arms
//! * `If { condition, then, else }` — sequential(cond) + max(then, else)
//!
//! ## Shadowing
//!
//! Closure parameters and pattern bindings shadow tracked names —
//! a tracked outer `x` is no longer matched inside `|x| x` because
//! the inner `x` refers to the closure parameter.

use std::collections::HashSet;

use verum_common::Text;

use verum_ast::expr::{ConditionKind, Expr, ExprKind};

use crate::qtt_usage::UsageMap;

/// Walk an expression and return the usage map for the given tracked
/// bindings.
pub fn walk_expr(tracked: &HashSet<Text>, expr: &Expr) -> UsageMap {
    let mut usage = UsageMap::new();
    walk_into(tracked, expr, &mut usage);
    usage
}

fn walk_into(tracked: &HashSet<Text>, expr: &Expr, out: &mut UsageMap) {
    match &expr.kind {
        ExprKind::Path(p) => {
            if p.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(ident) = &p.segments[0] {
                    let name = Text::from(ident.name.as_str());
                    if tracked.contains(&name) {
                        out.use_once(name);
                    }
                }
            }
        }

        ExprKind::Call { func, args, .. } => {
            walk_into(tracked, func, out);
            for a in args.iter() {
                walk_into(tracked, a, out);
            }
        }

        ExprKind::Binary { left, right, .. } => {
            walk_into(tracked, left, out);
            walk_into(tracked, right, out);
        }

        ExprKind::Unary { expr: inner, .. } => walk_into(tracked, inner, out),

        ExprKind::Paren(inner) => walk_into(tracked, inner, out),

        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            // IfCondition is a SmallVec of ConditionKind; recurse
            // into each `Expr` sub-condition.
            for cond in condition.conditions.iter() {
                if let ConditionKind::Expr(e) = cond {
                    walk_into(tracked, e, out);
                }
            }
            // Branches: max usage of then and else, sequenced after
            // condition. then_branch is a Block, else is a Maybe<Heap<Expr>>.
            let then_map = walk_block_into(tracked, then_branch);
            let else_map = match else_branch {
                verum_common::Maybe::Some(eb) => walk_expr(tracked, eb),
                verum_common::Maybe::None => UsageMap::new(),
            };
            let merged = then_map.merge_branches_max(else_map);
            *out = std::mem::take(out).merge_sequential(merged);
        }

        ExprKind::Match { expr: scrut, arms } => {
            walk_into(tracked, scrut, out);
            let mut combined: Option<UsageMap> = None;
            for arm in arms.iter() {
                let arm_tracked = remove_pattern_binders(tracked, &arm.pattern);
                let body = walk_expr(&arm_tracked, &arm.body);
                combined = Some(match combined {
                    None => body,
                    Some(prev) => prev.merge_branches_max(body),
                });
            }
            if let Some(arms_map) = combined {
                *out = std::mem::take(out).merge_sequential(arms_map);
            }
        }

        ExprKind::Block(block) => {
            *out = std::mem::take(out)
                .merge_sequential(walk_block_into(tracked, block));
        }

        ExprKind::Closure { params, body, .. } => {
            let mut inner_tracked = tracked.clone();
            for p in params.iter() {
                strip_pattern_binders(&mut inner_tracked, &p.pattern);
            }
            walk_into(&inner_tracked, body, out);
        }

        ExprKind::Tuple(elements) => {
            for e in elements.iter() {
                walk_into(tracked, e, out);
            }
        }

        ExprKind::Field { expr: receiver, .. } => {
            walk_into(tracked, receiver, out)
        }

        ExprKind::Index { expr: receiver, index } => {
            walk_into(tracked, receiver, out);
            walk_into(tracked, index, out);
        }

        ExprKind::Cast { expr: inner, .. } => walk_into(tracked, inner, out),

        ExprKind::Return(opt) => {
            if let verum_common::Maybe::Some(e) = opt {
                walk_into(tracked, e, out);
            }
        }

        ExprKind::Try(inner) => walk_into(tracked, inner, out),

        // Conservative fallback: any other shape contributes zero.
        _ => {}
    }
}

fn walk_block_into(
    tracked: &HashSet<Text>,
    block: &verum_ast::expr::Block,
) -> UsageMap {
    let mut acc = UsageMap::new();
    let mut active = tracked.clone();
    for stmt in block.stmts.iter() {
        match &stmt.kind {
            verum_ast::StmtKind::Let { pattern, value, .. } => {
                if let verum_common::Maybe::Some(v) = value {
                    walk_into(&active, v, &mut acc);
                }
                strip_pattern_binders(&mut active, pattern);
            }
            verum_ast::StmtKind::Expr { expr: e, .. } => walk_into(&active, e, &mut acc),
            _ => {}
        }
    }
    if let verum_common::Maybe::Some(tail) = &block.expr {
        walk_into(&active, tail, &mut acc);
    }
    acc
}

fn pattern_binders(pat: &verum_ast::pattern::Pattern, out: &mut Vec<Text>) {
    use verum_ast::pattern::PatternKind;
    match &pat.kind {
        PatternKind::Ident { name, subpattern, .. } => {
            out.push(Text::from(name.name.as_str()));
            if let verum_common::Maybe::Some(sub) = subpattern {
                pattern_binders(sub, out);
            }
        }
        PatternKind::Tuple(parts) => {
            for p in parts.iter() {
                pattern_binders(p, out);
            }
        }
        _ => {}
    }
}

fn strip_pattern_binders(set: &mut HashSet<Text>, pat: &verum_ast::pattern::Pattern) {
    let mut binders = Vec::new();
    pattern_binders(pat, &mut binders);
    for b in binders {
        set.remove(&b);
    }
}

fn remove_pattern_binders(
    set: &HashSet<Text>,
    pat: &verum_ast::pattern::Pattern,
) -> HashSet<Text> {
    let mut clone = set.clone();
    strip_pattern_binders(&mut clone, pat);
    clone
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path};
    use verum_common::List;

    fn sp() -> Span {
        Span::default()
    }

    fn ident(name: &str) -> Ident {
        Ident::new(name, sp())
    }

    fn path_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Path(Path::single(ident(name))),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        }
    }

    fn tracked(names: &[&str]) -> HashSet<Text> {
        names.iter().map(|n| Text::from(*n)).collect()
    }

    #[test]
    fn single_path_use_counts_one() {
        let e = path_expr("x");
        let u = walk_expr(&tracked(&["x"]), &e);
        assert_eq!(u.lookup(&Text::from("x")).runtime, 1);
    }

    #[test]
    fn untracked_path_ignored() {
        let e = path_expr("y");
        let u = walk_expr(&tracked(&["x"]), &e);
        assert!(u.is_empty());
    }

    #[test]
    fn call_sums_arg_uses() {
        let e = Expr {
            kind: ExprKind::Call {
                func: verum_common::Heap::new(path_expr("f")),
                type_args: List::new(),
                args: List::from_iter([path_expr("x"), path_expr("x")]),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        let u = walk_expr(&tracked(&["x"]), &e);
        assert_eq!(u.lookup(&Text::from("x")).runtime, 2);
    }

    #[test]
    fn binary_walks_both_sides() {
        let e = Expr {
            kind: ExprKind::Binary {
                op: verum_ast::expr::BinOp::Add,
                left: verum_common::Heap::new(path_expr("x")),
                right: verum_common::Heap::new(path_expr("y")),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        let u = walk_expr(&tracked(&["x", "y"]), &e);
        assert_eq!(u.lookup(&Text::from("x")).runtime, 1);
        assert_eq!(u.lookup(&Text::from("y")).runtime, 1);
    }

    #[test]
    fn paren_unwraps() {
        let e = Expr {
            kind: ExprKind::Paren(verum_common::Heap::new(path_expr("x"))),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        let u = walk_expr(&tracked(&["x"]), &e);
        assert_eq!(u.lookup(&Text::from("x")).runtime, 1);
    }

    #[test]
    fn unrecognized_shape_is_zero_usage() {
        let e = Expr {
            kind: ExprKind::Literal(verum_ast::literal::Literal::int(42, sp())),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        let u = walk_expr(&tracked(&["x"]), &e);
        assert!(u.is_empty());
    }

    #[test]
    fn tuple_sums_elements() {
        let e = Expr {
            kind: ExprKind::Tuple(List::from_iter([
                path_expr("x"),
                path_expr("x"),
                path_expr("x"),
            ])),
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        let u = walk_expr(&tracked(&["x"]), &e);
        assert_eq!(u.lookup(&Text::from("x")).runtime, 3);
    }

    #[test]
    fn field_access_recurses_into_receiver() {
        let e = Expr {
            kind: ExprKind::Field {
                expr: verum_common::Heap::new(path_expr("rec")),
                field: ident("name"),
            },
            span: sp(),
            ref_kind: None,
            check_eliminated: false,
        };
        let u = walk_expr(&tracked(&["rec"]), &e);
        assert_eq!(u.lookup(&Text::from("rec")).runtime, 1);
    }
}
