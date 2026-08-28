//! Every binding in a `provide` parses its value the same way.
//!
//! The first binding peeked for an identifier and used full expression
//! parsing, so a named record literal came through:
//!
//! ```text
//! provide Logger = ConsoleLogger { level: 1 } { ... }
//!                  ^^^^^^^^^^^^^^^^^^^^^^^^^ value  ^^^^^ scope
//! ```
//!
//! The second and later bindings of a multi-provide called
//! `parse_expr_bp_no_struct` directly, so the `{` after the name read as
//! the SCOPE BLOCK instead of the record:
//!
//! ```text
//! provide A = X { n: 1 }, B = Y { k: 2 } { body }
//!                             ^^^^^^^^^^ taken as the scope
//!                                        ^^^^^^^^ orphaned statement
//! ```
//!
//! With a non-empty record that is a parse error, which is loud. With an
//! EMPTY one it was SILENT: `B = T { }` took `{ }` as the scope, `{ body }`
//! became a separate statement after the provide, and codegen emitted
//!
//! ```text
//! CtxProvide A / CtxProvide B / CtxEnd / CtxEnd / Call body
//! ```
//!
//! — both contexts popped before the body ran. The program compiled with
//! zero errors and panicked at runtime with "Context T not provided".
//!
//! Two spellings of one construct with the rule written on only one of
//! them; the fix is one carrier, `parse_provide_value`, used by both.
//!
//! Task: T0937.

use verum_ast::span::FileId;
use verum_fast_parser::VerumParser;
use verum_lexer::Lexer;

fn parse(src: &str) -> verum_ast::Module {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(src, file_id);
    VerumParser::new()
        .parse_module(lexer, file_id)
        .unwrap_or_else(|e| panic!("parse failed for {src:?}: {e:?}"))
}

/// Names of the contexts every `ProvideScope` in the module provides, in
/// the order the walk meets them.
///
/// The multi form desugars to NESTED `ProvideScope`s wrapped in block
/// expressions, so this recurses through statements and expressions
/// rather than scanning the top level — a shallow scan would report one
/// binding for a three-binding provide and read as a pass.
fn provided_contexts(module: &verum_ast::Module) -> Vec<String> {
    use verum_ast::expr::ExprKind;
    use verum_ast::stmt::StmtKind;

    fn walk_expr(e: &verum_ast::Expr, out: &mut Vec<String>) {
        if let ExprKind::Block(b) = &e.kind {
            for s in b.stmts.iter() {
                walk_stmt(s, out);
            }
            if let verum_common::Maybe::Some(tail) = &b.expr {
                walk_expr(tail, out);
            }
        }
    }

    fn walk_stmt(s: &verum_ast::Stmt, out: &mut Vec<String>) {
        match &s.kind {
            StmtKind::ProvideScope { context, block, .. } => {
                out.push(context.to_string());
                walk_expr(block, out);
            }
            StmtKind::Provide { context, .. } => out.push(context.to_string()),
            StmtKind::Expr { expr, .. } => walk_expr(expr, out),
            _ => {}
        }
    }

    let mut out = Vec::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            if let verum_common::Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) = &f.body {
                for s in b.stmts.iter() {
                    walk_stmt(s, &mut out);
                }
                if let verum_common::Maybe::Some(tail) = &b.expr {
                    walk_expr(tail, &mut out);
                }
            }
        }
    }
    out
}

/// Units at the top of `main`'s body — statements plus a tail expression.
///
/// This is the count the misparse moved. The desugared multi-provide is a
/// single tail expression, so a correct parse gives 1; when `{ body }` was
/// mistaken for a value's record and orphaned, it became a SECOND unit.
/// Counting `stmts` alone would report 0 either way and pass vacuously.
fn top_level_unit_count(module: &verum_ast::Module) -> usize {
    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            if let verum_common::Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) = &f.body {
                return b.stmts.len() + usize::from(b.expr.is_some());
            }
        }
    }
    0
}

/// Is there a call inside the INNERMOST provide's block?
///
/// The count above says nothing landed outside; this says the body landed
/// INSIDE. Both are needed: a parse that dropped the body entirely would
/// satisfy the count on its own.
fn innermost_provide_body_has_a_call(module: &verum_ast::Module) -> bool {
    use verum_ast::expr::ExprKind;
    use verum_ast::stmt::StmtKind;

    fn deepest<'a>(e: &'a verum_ast::Expr, found: &mut Option<&'a verum_ast::Expr>) {
        if let ExprKind::Block(b) = &e.kind {
            for s in b.stmts.iter() {
                if let StmtKind::ProvideScope { block, .. } = &s.kind {
                    *found = Some(block);
                    deepest(block, found);
                }
            }
            if let verum_common::Maybe::Some(tail) = &b.expr {
                deepest(tail, found);
            }
        }
    }

    fn has_call(e: &verum_ast::Expr) -> bool {
        match &e.kind {
            ExprKind::Call { .. } => true,
            ExprKind::Block(b) => {
                b.stmts.iter().any(|s| match &s.kind {
                    StmtKind::Expr { expr, .. } => has_call(expr),
                    _ => false,
                }) || matches!(&b.expr, verum_common::Maybe::Some(t) if has_call(t))
            }
            _ => false,
        }
    }

    for item in &module.items {
        if let verum_ast::ItemKind::Function(f) = &item.kind {
            if let verum_common::Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) = &f.body {
                let mut found = None;
                for s in b.stmts.iter() {
                    if let StmtKind::ProvideScope { block, .. } = &s.kind {
                        found = Some(&**block);
                        deepest(block, &mut found);
                    }
                }
                if let verum_common::Maybe::Some(tail) = &b.expr {
                    deepest(tail, &mut found);
                }
                return found.is_some_and(has_call);
            }
        }
    }
    false
}

#[test]
fn the_first_binding_takes_a_record_literal() {
    // The control. This spelling always worked; if it stops working the
    // fix broke the case it was modelled on.
    let m = parse("fn main() { provide A = X { n: 1 } { work(); } }");
    assert_eq!(provided_contexts(&m), vec!["A"]);
}

#[test]
fn a_later_binding_takes_a_record_literal_too() {
    let m = parse("fn main() { provide A = X { n: 1 }, B = Y { k: 2 } { work(); } }");
    assert_eq!(provided_contexts(&m), vec!["A", "B"]);
}

#[test]
fn an_empty_record_in_a_later_binding_is_not_the_scope() {
    // The silent case: `Y { }` must be the VALUE, not the scope block.
    // If it is taken as the scope, `{ work(); }` becomes a second
    // top-level statement — which is exactly what this asserts against.
    let m = parse("fn main() { provide A = X { n: 1 }, B = Y { } { work(); } }");
    assert_eq!(provided_contexts(&m), vec!["A", "B"]);
    assert_eq!(
        top_level_unit_count(&m),
        1,
        "the body must be INSIDE the provide, not a unit after it"
    );
    assert!(
        innermost_provide_body_has_a_call(&m),
        "work() must sit in the innermost provide's block"
    );
}

#[test]
fn three_bindings_all_take_record_literals() {
    let m = parse("fn main() { provide A = X { n: 1 }, B = Y { k: 2 }, C = Z { } { work(); } }");
    assert_eq!(provided_contexts(&m), vec!["A", "B", "C"]);
}

#[test]
fn a_later_binding_still_takes_a_non_record_value() {
    // The `no_struct` branch must survive: a call, a literal and a path
    // are all values, and none of them may swallow the scope block.
    let m = parse("fn main() { provide A = X { n: 1 }, B = mk() { work(); } }");
    assert_eq!(provided_contexts(&m), vec!["A", "B"]);
    assert_eq!(top_level_unit_count(&m), 1);
    assert!(innermost_provide_body_has_a_call(&m));
}

#[test]
fn an_alias_on_a_later_binding_keeps_the_record_literal() {
    let m = parse("fn main() { provide A = X { n: 1 }, A as second = X { n: 9 } { work(); } }");
    assert_eq!(provided_contexts(&m), vec!["A", "A"]);
    assert_eq!(top_level_unit_count(&m), 1);
    assert!(innermost_provide_body_has_a_call(&m));
}

#[test]
fn a_bare_identifier_value_does_not_swallow_the_scope() {
    // The REGRESSION the first version of this fix introduced.
    //
    //     provide Logger = logger, Database = db { seed(); }
    //
    // With the identifier peek alone, `db { seed(); }` reads as a record
    // literal, the block disappears into it, and the whole statement is
    // a parse error at `seed()`. This shape is what the context-system
    // tutorial teaches, and it worked before the record-literal fix —
    // so the fix traded one broken spelling for another until the value
    // parse learned to rewind.
    let m = parse("fn main() { provide A = logger, B = db { seed(); } }");
    assert_eq!(provided_contexts(&m), vec!["A", "B"]);
    assert_eq!(top_level_unit_count(&m), 1);
    assert!(
        innermost_provide_body_has_a_call(&m),
        "seed() must sit in the innermost provide's block, not inside `db`"
    );
}

#[test]
fn a_bare_identifier_value_works_in_the_single_form_too() {
    let m = parse("fn main() { provide A = logger { seed(); } }");
    assert_eq!(provided_contexts(&m), vec!["A"]);
    assert!(innermost_provide_body_has_a_call(&m));
}

#[test]
fn a_record_literal_and_a_bare_identifier_can_be_mixed() {
    let m = parse("fn main() { provide A = X { n: 1 }, B = db { seed(); } }");
    assert_eq!(provided_contexts(&m), vec!["A", "B"]);
    assert!(innermost_provide_body_has_a_call(&m));
}
