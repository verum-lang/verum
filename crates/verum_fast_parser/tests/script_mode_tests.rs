#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Tests for script-mode parsing (P1.2).
//!
//! Verifies that [`FastParser::parse_module_script_str`] accepts top-
//! level statements alongside items and folds every collected
//! statement into a single synthesised `__verum_script_main`
//! `FunctionDecl`. Library-mode parsing keeps the stricter grammar
//! (decls only) — the script flag must be off by default and reject
//! statements via the regular item-parser error path.

use verum_ast::{FileId, ItemKind};
use verum_fast_parser::FastParser;

fn fid() -> FileId {
    FileId::new(0)
}

#[test]
fn library_mode_rejects_top_level_defer() {
    // `defer { … }` is statement-only; the default library parser
    // has no item rule that accepts it. Note: `let NAME = …;` at
    // top level is intentionally accepted as a `const` shorthand
    // (decl.rs ~431), which is why we use `defer` as the statement-
    // only canary here.
    let parser = FastParser::new();
    let result = parser.parse_module_str("defer cleanup();\n", fid());
    assert!(
        result.is_err(),
        "library mode must reject top-level `defer` statements"
    );
}

#[test]
fn script_mode_accepts_top_level_defer() {
    let parser = FastParser::new();
    let module = parser
        .parse_module_script_str("defer cleanup();\n", fid())
        .expect("script mode must accept top-level `defer`");

    assert_eq!(module.items.len(), 1, "expected one synthesised wrapper");
    let item = &module.items[0];
    let func = match &item.kind {
        ItemKind::Function(f) => f,
        other => panic!("expected Function item, got {:?}", other),
    };
    assert_eq!(
        func.name.name.as_str(),
        "__verum_script_main",
        "wrapper must be named __verum_script_main"
    );
    let body = match &func.body {
        verum_common::Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => b,
        _ => panic!("wrapper must have a block body"),
    };
    assert_eq!(body.stmts.len(), 1, "expected 1 collected statement");
}

#[test]
fn script_mode_intermixes_items_and_statements() {
    // P1.2 contract: `decl ; stmt ; decl ; stmt` survives unchanged.
    // The two top-level fns appear before the wrapper; the two
    // statements get folded into the wrapper's body in source order.
    let src = r#"
fn helper() -> Int { 42 }

let x = helper();

fn other() -> Int { 7 }

let y = x + other();
"#;
    let parser = FastParser::new();
    let module = parser
        .parse_module_script_str(src, fid())
        .expect("script mode must accept mixed decls + stmts");

    // Three items: two `fn` decls + the synthesised wrapper.
    assert_eq!(module.items.len(), 3, "expected helper, other, wrapper");

    // First two items are the user-written fns in source order.
    let names: Vec<&str> = module
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Function(f) => Some(f.name.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["helper", "other", "__verum_script_main"]);

    // The wrapper holds exactly the two `let` stmts.
    let wrapper = module
        .items
        .iter()
        .find_map(|i| match &i.kind {
            ItemKind::Function(f) if f.name.name.as_str() == "__verum_script_main" => Some(f),
            _ => None,
        })
        .expect("wrapper must exist");
    let body = match &wrapper.body {
        verum_common::Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => b,
        _ => panic!("wrapper body must be a block"),
    };
    assert_eq!(
        body.stmts.len(),
        2,
        "expected 2 collected let-statements in wrapper"
    );
}

#[test]
fn script_mode_with_no_statements_emits_no_wrapper() {
    // A pure-decl source compiled in script mode must NOT emit a
    // wrapper — synthesising an empty `__verum_script_main` would
    // pollute the symbol table for source files that just happen to
    // be parsed as scripts (e.g., a build-tool-driven dispatch).
    let parser = FastParser::new();
    let module = parser
        .parse_module_script_str("fn main() -> Int { 0 }\n", fid())
        .expect("script mode must accept pure-decl source");
    assert_eq!(module.items.len(), 1);
    let names: Vec<&str> = module
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Function(f) => Some(f.name.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["main"]);
}

#[test]
fn script_mode_expression_statement() {
    // Expression statements don't start with let/defer/etc., so they
    // hit the parse-item-then-fallback-to-stmt path. Common case:
    // a side-effect call in script form.
    let parser = FastParser::new();
    let module = parser
        .parse_module_script_str("print(42);\n", fid())
        .expect("script mode must accept top-level call expressions");

    // Single wrapper item.
    assert_eq!(module.items.len(), 1);
    let func = match &module.items[0].kind {
        ItemKind::Function(f) => f,
        _ => panic!("expected wrapper Function"),
    };
    assert_eq!(func.name.name.as_str(), "__verum_script_main");
    let body = match &func.body {
        verum_common::Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => b,
        _ => panic!("wrapper body must be a block"),
    };
    assert_eq!(body.stmts.len(), 1);
}
