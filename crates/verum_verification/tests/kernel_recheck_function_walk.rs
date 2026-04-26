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
