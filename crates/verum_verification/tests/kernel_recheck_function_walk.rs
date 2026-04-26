//! Integration tests for `KernelRecheck::recheck_function` (#186 V3).
//!
//! These tests exercise the production wiring path: a synthetic
//! `FunctionDecl` is built with refinement types in parameters and
//! return position, the public `recheck_function` is called, and
//! the per-site (label, outcome) pairs are inspected. This is the
//! same entry point that `SmtVerificationPass::verify_function`
//! invokes as its kernel-recheck preamble before any SMT round.

use verum_ast::Ident;
use verum_ast::Span;
use verum_ast::Visibility;
use verum_ast::decl::{FunctionDecl, FunctionParam, FunctionParamKind};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::ty::{RefinementPredicate, Type, TypeKind};
use verum_common::{Heap, List, Maybe, Text};
use verum_verification::kernel_recheck::KernelRecheck;

fn span() -> Span {
    Span::default()
}

fn ident(name: &str) -> Ident {
    Ident {
        name: Text::from(name),
        span: span(),
    }
}

fn path_expr(name: &str) -> Expr {
    Expr::ident(ident(name))
}

fn method_call_expr(receiver: Expr, method_name: &str) -> Expr {
    Expr::new(
        ExprKind::MethodCall {
            receiver: Heap::new(receiver),
            method: ident(method_name),
            args: List::new(),
            type_args: List::new(),
        },
        span(),
    )
}

fn refined_int(predicate_expr: Expr, binder: Maybe<Ident>) -> Type {
    Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span())),
            predicate: Heap::new(RefinementPredicate {
                expr: predicate_expr,
                binding: binder,
                span: span(),
            }),
        },
        span(),
    )
}

fn wildcard_pattern() -> Pattern {
    Pattern {
        kind: PatternKind::Wildcard,
        span: span(),
    }
}

fn regular_param(ty: Type) -> FunctionParam {
    FunctionParam {
        kind: FunctionParamKind::Regular {
            pattern: wildcard_pattern(),
            ty,
            default_value: Maybe::None,
        },
        attributes: List::new(),
        span: span(),
    }
}

/// Build a minimal `FunctionDecl` with the supplied parameter types
/// and return type. Many fields default to neutral values; the
/// recheck walker only consults `params` + `return_type` + `name`.
fn make_function(
    name: &str,
    params: Vec<Type>,
    return_type: Maybe<Type>,
) -> FunctionDecl {
    let mut p_list: List<FunctionParam> = List::new();
    for ty in params {
        p_list.push(regular_param(ty));
    }
    FunctionDecl {
        visibility: Visibility::Private,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: ident(name),
        generics: List::new(),
        params: p_list,
        return_type,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::None,
        span: span(),
    }
}

#[test]
fn recheck_function_no_refinements_returns_empty() {
    let func = make_function("plain", vec![Type::int(span())], Maybe::Some(Type::int(span())));
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 0);
}

#[test]
fn recheck_function_well_formed_param_refinement_passes() {
    // `fn f(x: Int{p}) -> Int` — predicate is a bare path
    // (rank 0); base is Int (rank 0); 0 < 0+1 ⇒ Ok.
    let pred = path_expr("p");
    let refined = refined_int(pred, Maybe::None);
    let func = make_function("f", vec![refined], Maybe::Some(Type::int(span())));
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1);
    let (label, outcome) = outcomes.get(0).unwrap();
    assert!(label.as_str().contains("K-Refine-omega"));
    assert!(label.as_str().contains("f"));
    assert!(label.as_str().contains("param"));
    assert!(outcome.is_ok(), "expected Ok, got {:?}", outcome);
}

#[test]
fn recheck_function_modal_overshoot_param_refinement_rejected() {
    // `fn g(x: Int{p.box().box()}) -> Int` — predicate has md^ω = 2,
    // base has md^ω = 0; 2 < 0+1 = 1 is false ⇒ Err.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let refined = refined_int(boxed, Maybe::None);
    let func = make_function("g", vec![refined], Maybe::Some(Type::int(span())));
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1);
    let (label, outcome) = outcomes.get(0).unwrap();
    assert!(label.as_str().contains("g"));
    assert!(label.as_str().contains("param"));
    assert!(outcome.is_err(), "expected Err, got {:?}", outcome);
}

#[test]
fn recheck_function_return_type_refinement_label_correct() {
    // Refinement on the return position only. Label must read
    // "return", not "param".
    let pred = path_expr("p");
    let refined = refined_int(pred, Maybe::None);
    let func = make_function("h", vec![Type::int(span())], Maybe::Some(refined));
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1);
    let (label, outcome) = outcomes.get(0).unwrap();
    assert!(label.as_str().contains("h"));
    assert!(label.as_str().contains("return"));
    assert!(outcome.is_ok());
}

#[test]
fn recheck_function_walks_into_tuple_types() {
    // `fn t(x: (Int, Int{p})) -> Int` — refinement is nested
    // inside a Tuple; the walker must recurse and pick it up.
    let pred = path_expr("p");
    let refined = refined_int(pred, Maybe::None);
    let mut tuple_elems: List<Type> = List::new();
    tuple_elems.push(Type::int(span()));
    tuple_elems.push(refined);
    let tuple_ty = Type::new(TypeKind::Tuple(tuple_elems), span());
    let func = make_function("t", vec![tuple_ty], Maybe::Some(Type::int(span())));
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1, "expected one refinement found, got {:?}", outcomes);
    let (label, outcome) = outcomes.get(0).unwrap();
    assert!(label.as_str().contains("t"));
    assert!(outcome.is_ok());
}

#[test]
fn recheck_function_walks_into_slice_types() {
    // `fn s(xs: [Int{p}]) -> Int` — refinement nested in a slice.
    let pred = path_expr("p");
    let refined = refined_int(pred, Maybe::None);
    let slice_ty = Type::new(TypeKind::Slice(Heap::new(refined)), span());
    let func = make_function("s", vec![slice_ty], Maybe::Some(Type::int(span())));
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1);
    let (_, outcome) = outcomes.get(0).unwrap();
    assert!(outcome.is_ok());
}

// =============================================================================
// V4 (#191) — function-body walker for let-binding refinements
// =============================================================================

fn make_function_with_body(
    name: &str,
    params: Vec<Type>,
    return_type: Maybe<Type>,
    body: verum_ast::decl::FunctionBody,
) -> FunctionDecl {
    let mut p_list: List<FunctionParam> = List::new();
    for ty in params {
        p_list.push(regular_param(ty));
    }
    FunctionDecl {
        visibility: Visibility::Private,
        is_async: false,
        is_meta: false,
        stage_level: 0,
        is_pure: false,
        is_generator: false,
        is_cofix: false,
        is_unsafe: false,
        is_transparent: false,
        is_variadic: false,
        extern_abi: Maybe::None,
        name: ident(name),
        generics: List::new(),
        params: p_list,
        return_type,
        throws_clause: Maybe::None,
        std_attr: Maybe::None,
        contexts: List::new(),
        generic_where_clause: Maybe::None,
        meta_where_clause: Maybe::None,
        requires: List::new(),
        ensures: List::new(),
        attributes: List::new(),
        body: Maybe::Some(body),
        span: span(),
    }
}

fn let_stmt_with_type(name: &str, ty: Type) -> verum_ast::stmt::Stmt {
    let pattern = verum_ast::pattern::Pattern {
        kind: verum_ast::pattern::PatternKind::Ident {
            by_ref: false,
            mutable: false,
            name: ident(name),
            subpattern: Maybe::None,
        },
        span: span(),
    };
    verum_ast::stmt::Stmt::let_stmt(pattern, Maybe::Some(ty), Maybe::None, span())
}

fn block_with_stmts(stmts: Vec<verum_ast::stmt::Stmt>) -> verum_ast::expr::Block {
    let mut s: List<verum_ast::stmt::Stmt> = List::new();
    for st in stmts {
        s.push(st);
    }
    verum_ast::expr::Block::new(s, Maybe::None, span())
}

#[test]
fn recheck_function_walks_into_body_let_binding() {
    // `fn f() { let x: Int{p} = ... }` — the let-binding type
    // carries a refinement that V4 must surface.
    let pred = path_expr("p");
    let let_stmt = let_stmt_with_type("x", refined_int(pred, Maybe::None));
    let body = verum_ast::decl::FunctionBody::Block(block_with_stmts(vec![let_stmt]));
    let func = make_function_with_body("f", vec![], Maybe::Some(Type::int(span())), body);
    let outcomes = KernelRecheck::recheck_function(&func);
    // Without V4 the walker would only see the (empty) signature
    // and return zero outcomes. With V4, the body's let-binding
    // refinement IS visible.
    assert_eq!(outcomes.len(), 1, "let-binding refinement must be walked");
    let (label, outcome) = outcomes.get(0).unwrap();
    assert!(label.as_str().contains("let"), "label should mention 'let' context: {}", label.as_str());
    assert!(outcome.is_ok(), "well-formed predicate accepted");
}

#[test]
fn recheck_function_let_binding_modal_overshoot_rejected() {
    // `fn g() { let x: Int{p.box().box()} = ... }` — modal
    // overshoot inside a let-binding type. V4 walks the body,
    // K-Refine-omega rejects.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let let_stmt = let_stmt_with_type("x", refined_int(boxed, Maybe::None));
    let body = verum_ast::decl::FunctionBody::Block(block_with_stmts(vec![let_stmt]));
    let func = make_function_with_body("g", vec![], Maybe::Some(Type::int(span())), body);
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1);
    let (_, outcome) = outcomes.get(0).unwrap();
    assert!(outcome.is_err(), "modal overshoot in let-binding must reject");
}

#[test]
fn recheck_function_walks_signature_and_body() {
    // Signature has one refinement (param x: Int{p}), body has
    // another (let y: Int{q}). Walker must surface BOTH.
    let p = path_expr("p");
    let q = path_expr("q");
    let let_stmt = let_stmt_with_type("y", refined_int(q, Maybe::None));
    let body = verum_ast::decl::FunctionBody::Block(block_with_stmts(vec![let_stmt]));
    let func = make_function_with_body(
        "h",
        vec![refined_int(p, Maybe::None)],
        Maybe::Some(Type::int(span())),
        body,
    );
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 2, "both signature and body refinements visible");
    let labels: Vec<String> = outcomes
        .iter()
        .map(|(l, _)| l.as_str().to_string())
        .collect();
    assert!(labels.iter().any(|l| l.contains("param")), "signature refinement labelled");
    assert!(labels.iter().any(|l| l.contains("let")), "body refinement labelled");
}

#[test]
fn recheck_function_walks_into_nested_if_body() {
    // `fn k() { if cond { let z: Int{p.box().box()} = ... } }`.
    // Refinement is nested inside an if-then block; V4's expr
    // walker descends into Block / If / Match / Loop.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let let_stmt = let_stmt_with_type("z", refined_int(boxed, Maybe::None));
    let then_block = block_with_stmts(vec![let_stmt]);

    use smallvec::smallvec;
    use verum_ast::expr::{ConditionKind, IfCondition};
    let cond = path_expr("cond");
    let if_cond = IfCondition {
        conditions: smallvec![ConditionKind::Expr(cond)],
        span: span(),
    };
    let if_expr = verum_ast::expr::Expr::new(
        verum_ast::expr::ExprKind::If {
            condition: verum_common::Heap::new(if_cond),
            then_branch: then_block,
            else_branch: Maybe::None,
        },
        span(),
    );
    let if_stmt = verum_ast::stmt::Stmt::expr(if_expr, true);
    let body = verum_ast::decl::FunctionBody::Block(block_with_stmts(vec![if_stmt]));
    let func = make_function_with_body("k", vec![], Maybe::Some(Type::int(span())), body);
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1, "refinement inside if-then block must be walked");
    let (_, outcome) = outcomes.get(0).unwrap();
    assert!(outcome.is_err(), "modal overshoot inside if must still reject");
}

#[test]
fn recheck_function_with_no_body_unchanged() {
    // Backwards-compat: a function with no body (extern decl,
    // protocol stub) walks the signature only — V4 is a strict
    // extension.
    let pred = path_expr("p");
    let func = make_function(
        "no_body",
        vec![refined_int(pred, Maybe::None)],
        Maybe::Some(Type::int(span())),
    );
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1, "signature-only walk for body-less fn");
}

#[test]
fn recheck_function_walks_through_explicit_binder() {
    // Modal-overshoot inside a Lambda-style refinement should
    // bubble up the explicit binder name 'y'.
    let p = path_expr("p");
    let boxed = method_call_expr(method_call_expr(p, "box"), "box");
    let refined = Type::new(
        TypeKind::Refined {
            base: Heap::new(Type::int(span())),
            predicate: Heap::new(RefinementPredicate {
                expr: boxed,
                binding: Maybe::Some(ident("y")),
                span: span(),
            }),
        },
        span(),
    );
    let func = make_function("e", vec![refined], Maybe::Some(Type::int(span())));
    let outcomes = KernelRecheck::recheck_function(&func);
    assert_eq!(outcomes.len(), 1);
    let (_, outcome) = outcomes.get(0).unwrap();
    let err = outcome.as_ref().expect_err("modal overshoot must reject");
    let rendered = format!("{}", err);
    assert!(
        rendered.contains("y"),
        "diagnostic must surface explicit binder 'y'; got: {}",
        rendered,
    );
}
