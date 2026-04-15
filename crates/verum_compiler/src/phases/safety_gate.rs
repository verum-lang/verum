//! Pre-typecheck safety-feature gates.
//!
//! Walks the parsed AST looking for language constructs that are
//! disabled by the current `[safety]` feature set, emitting clean
//! diagnostics before type-checking runs. Keeping these checks in
//! their own walker avoids threading feature flags through the
//! ~52K-line `TypeChecker` and keeps the gate logic auditable.
//!
//! Currently gates:
//!   - `unsafe { … }` expressions when `safety.unsafe_allowed = false`.

use verum_ast::decl::{FunctionBody, ImplItemKind, Item, ItemKind};
use verum_ast::expr::{Block, ConditionKind};
use verum_ast::stmt::StmtKind;
use verum_ast::{Expr, ExprKind, Module};
use verum_common::List;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

/// Scan `modules` for `unsafe { ... }` blocks and return a diagnostic
/// for each one. Returns an empty list when `unsafe_allowed` is `true`
/// or when no `unsafe` blocks are present.
pub fn check_unsafe_usage(
    modules: &[Module],
    unsafe_allowed: bool,
) -> List<Diagnostic> {
    let mut diagnostics = List::new();
    if unsafe_allowed {
        return diagnostics;
    }
    for module in modules {
        for item in module.items.iter() {
            walk_item(item, &mut diagnostics);
        }
    }
    diagnostics
}

fn walk_item(item: &Item, out: &mut List<Diagnostic>) {
    match &item.kind {
        ItemKind::Function(func) => {
            if let Some(body) = &func.body {
                walk_function_body(body, out);
            }
        }
        ItemKind::Impl(impl_decl) => {
            for impl_item in &impl_decl.items {
                if let ImplItemKind::Function(func) = &impl_item.kind {
                    if let Some(body) = &func.body {
                        walk_function_body(body, out);
                    }
                }
            }
        }
        _ => {}
    }
}

fn walk_function_body(body: &FunctionBody, out: &mut List<Diagnostic>) {
    match body {
        FunctionBody::Block(blk) => walk_block(blk, out),
        FunctionBody::Expr(expr) => walk_expr(expr, out),
    }
}

fn walk_block(block: &Block, out: &mut List<Diagnostic>) {
    for stmt in &block.stmts {
        walk_stmt(stmt, out);
    }
    if let verum_common::Maybe::Some(e) = &block.expr {
        walk_expr(e, out);
    }
}

fn walk_stmt(stmt: &verum_ast::stmt::Stmt, out: &mut List<Diagnostic>) {
    match &stmt.kind {
        StmtKind::Expr { expr, .. } => walk_expr(expr, out),
        StmtKind::Let { value, .. } => {
            if let verum_common::Maybe::Some(init) = value {
                walk_expr(init, out);
            }
        }
        _ => {}
    }
}

fn walk_expr(expr: &Expr, out: &mut List<Diagnostic>) {
    match &expr.kind {
        ExprKind::Unsafe(block) => {
            out.push(
                DiagnosticBuilder::error()
                    .message(
                        "`unsafe` blocks are not allowed: \
                         `[safety] unsafe_allowed` is disabled",
                    )
                    .span(super::ast_span_to_diagnostic_span(expr.span, None))
                    .help(
                        "set `unsafe_allowed = true` under `[safety]` in \
                         verum.toml, or remove `-Z safety.unsafe_allowed=false`",
                    )
                    .build(),
            );
            // Recurse — nested unsafe still worth reporting.
            walk_block(block, out);
        }
        ExprKind::Block(block) => walk_block(block, out),
        ExprKind::Async(block) | ExprKind::Meta(block) => walk_block(block, out),
        ExprKind::If { condition, then_branch, else_branch } => {
            for cond in &condition.conditions {
                match cond {
                    ConditionKind::Expr(e) => walk_expr(e, out),
                    ConditionKind::Let { value, .. } => walk_expr(value, out),
                }
            }
            walk_block(then_branch, out);
            if let verum_common::Maybe::Some(else_b) = else_branch {
                walk_expr(else_b, out);
            }
        }
        ExprKind::Match { expr: scrutinee, arms } => {
            walk_expr(scrutinee, out);
            for arm in arms {
                if let verum_common::Maybe::Some(guard) = &arm.guard {
                    walk_expr(guard, out);
                }
                walk_expr(&arm.body, out);
            }
        }
        ExprKind::Loop { body, .. } => walk_block(body, out),
        ExprKind::While { condition, body, .. } => {
            walk_expr(condition, out);
            walk_block(body, out);
        }
        ExprKind::For { iter, body, .. } => {
            walk_expr(iter, out);
            walk_block(body, out);
        }
        ExprKind::Call { func, args, .. } => {
            walk_expr(func, out);
            for a in args {
                walk_expr(a, out);
            }
        }
        ExprKind::Binary { left, right, .. } => {
            walk_expr(left, out);
            walk_expr(right, out);
        }
        ExprKind::Unary { expr, .. } => walk_expr(expr, out),
        ExprKind::Return(maybe) => {
            if let verum_common::Maybe::Some(e) = maybe {
                walk_expr(e, out);
            }
        }
        ExprKind::Paren(inner) => walk_expr(inner, out),
        _ => {
            // Other expression kinds (Literal, Path, Lambda, …) are
            // either leaves or have shapes we don't need to descend
            // into for this check. A miss here is a false negative —
            // the safe failure mode.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{FunctionDecl, ItemKind};
    use verum_ast::expr::{Block, Expr, ExprKind};
    use verum_ast::{FileId, Span};
    use verum_common::{List, Maybe};

    fn mk_unsafe_expr() -> Expr {
        let blk = Block {
            stmts: List::new(),
            expr: Maybe::None,
            span: Span::dummy(),
        };
        Expr::new(ExprKind::Unsafe(blk), Span::dummy())
    }

    fn mk_module_with_unsafe() -> Module {
        // Build a minimal main fn with { unsafe {} } in its body.
        let inner_block = Block {
            stmts: List::new(),
            expr: Maybe::Some(Box::new(mk_unsafe_expr())),
            span: Span::dummy(),
        };
        let func = FunctionDecl {
            visibility: Default::default(),
            name: verum_ast::ty::Ident::new("main", Span::dummy()),
            generics: List::new(),
            params: List::new(),
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            body: Some(FunctionBody::Block(inner_block)),
            attributes: List::new(),
            is_async: false,
            is_meta: false,
            is_unsafe: false,
            span: Span::dummy(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_transparent: false,
            extern_abi: Maybe::None,
            is_variadic: false,
            std_attr: Maybe::None,
            contexts: List::new(),
        };
        let mut items = List::new();
        items.push(Item::new(ItemKind::Function(func), Span::dummy()));
        Module {
            items,
            attributes: List::new(),
            file_id: FileId::new(0),
            span: Span::dummy(),
        }
    }

    #[test]
    fn allowed_yields_no_diagnostics_even_with_unsafe() {
        let module = mk_module_with_unsafe();
        let diags = check_unsafe_usage(&[module], true);
        assert_eq!(diags.len(), 0);
    }

    #[test]
    fn disabled_flags_unsafe_block() {
        let module = mk_module_with_unsafe();
        let diags = check_unsafe_usage(&[module], false);
        assert_eq!(diags.len(), 1, "one unsafe block → one diagnostic");
        let msg = diags.iter().next().unwrap().message();
        assert!(
            msg.contains("unsafe") && msg.contains("[safety]"),
            "diag must point at the config key (got: {})",
            msg
        );
    }

    #[test]
    fn disabled_with_empty_module_yields_nothing() {
        let empty = Module {
            items: List::new(),
            attributes: List::new(),
            file_id: FileId::new(0),
            span: Span::dummy(),
        };
        let diags = check_unsafe_usage(&[empty], false);
        assert_eq!(diags.len(), 0);
    }
}
