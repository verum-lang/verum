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
//! Unit tests for the `@command` derive macro.
//!
//! Spec: internal/specs/cli-framework.md §4
//!
//! These tests exercise the derive in isolation — given a constructed
//! `TypeDecl`, what `Item` does `DeriveCommand::expand` produce? Tests
//! assert on the AST shape (Module wrapper containing a freestanding
//! `__cli_spec_for_<Type>` function whose body is an AppBuilder chain).

use smallvec::smallvec;
use verum_ast::Span;
use verum_ast::decl::{
    FunctionBody, ItemKind, RecordField, TypeDecl, TypeDeclBody, Visibility,
};
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_ast::{Attribute, Literal, LiteralKind};
use verum_common::{List, Maybe, Text};

use verum_compiler::derives::{DeriveCommand, DeriveContext, DeriveMacro};

fn span() -> Span {
    Span::default()
}

fn text_lit(s: &str) -> Expr {
    use verum_ast::literal::StringLit;
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Text(StringLit::Regular(s.to_string().into())),
            span: span(),
        }),
        span(),
    )
}

fn named(key: &str, value: Expr) -> Expr {
    Expr::new(
        ExprKind::NamedArg {
            name: Ident::new(key, span()),
            value: verum_common::Heap::new(value),
        },
        span(),
    )
}

fn type_path(name: &str) -> Type {
    Type::new(
        TypeKind::Path(Path::single(Ident::new(name, span()))),
        span(),
    )
}

fn record_field(name: &str, ty: Type, attrs: Vec<Attribute>) -> RecordField {
    RecordField {
        visibility: Visibility::Public,
        name: Ident::new(name, span()),
        ty,
        default_value: Maybe::None,
        attributes: List::from(attrs),
        span: span(),
    }
}

fn make_command_attr(name: &str, version: Option<&str>, about: Option<&str>) -> Attribute {
    let mut args: Vec<Expr> = Vec::new();
    args.push(named("name", text_lit(name)));
    if let Some(v) = version {
        args.push(named("version", text_lit(v)));
    }
    if let Some(a) = about {
        args.push(named("about", text_lit(a)));
    }
    Attribute::new(
        Text::from("command"),
        Maybe::Some(List::from(args)),
        span(),
    )
}

#[test]
fn derive_emits_module_wrapper() {
    let bool_field = record_field(
        "verbose",
        type_path("Bool"),
        vec![Attribute::simple(Text::from("flag"), span())],
    );
    let cmd_attr = make_command_attr("mytool", Some("1.0.0"), Some("test cli"));

    let decl = TypeDecl {
        visibility: Visibility::Public,
        name: Ident::new("Cli", span()),
        generics: List::new(),
        attributes: List::from(vec![cmd_attr]),
        body: TypeDeclBody::Record(List::from(vec![bool_field])),
        meta_where_clause: Maybe::None,
        generic_where_clause: Maybe::None,
        span: span(),
    };

    let ctx = DeriveContext::from_type_decl(&decl, span()).expect("derive context");
    let item = DeriveCommand
        .expand(&ctx)
        .expect("derive command should expand");

    // Outer must be a Module — that's how the derive infrastructure
    // hoists the freestanding factory fn into the parent scope.
    let module = match &item.kind {
        ItemKind::Module(m) => m,
        other => panic!("expected Module wrapper, got {:?}", other),
    };

    // Module name follows the `__cli_command_derive_<Type>` convention.
    assert_eq!(module.name.as_str(), "__cli_command_derive_Cli");

    // Module must contain exactly one item — the factory fn.
    let items = match &module.items {
        Maybe::Some(items) => items,
        Maybe::None => panic!("module should have items"),
    };
    assert_eq!(items.len(), 1);

    let inner = &items[0];
    let factory_fn = match &inner.kind {
        ItemKind::Function(f) => f,
        other => panic!("inner item should be a Function, got {:?}", other),
    };
    assert_eq!(factory_fn.name.as_str(), "__cli_spec_for_Cli");
    assert!(matches!(factory_fn.body, Some(FunctionBody::Block(_))));
}

#[test]
fn derive_chain_starts_with_appbuilder_new() {
    // Check that the body of the generated fn ends in a method call
    // chain rooted at AppBuilder::new(...).
    let bool_field = record_field(
        "verbose",
        type_path("Bool"),
        vec![Attribute::simple(Text::from("flag"), span())],
    );
    let cmd_attr = make_command_attr("tool", None, Some("..."));

    let decl = TypeDecl {
        visibility: Visibility::Public,
        name: Ident::new("Tool", span()),
        generics: List::new(),
        attributes: List::from(vec![cmd_attr]),
        body: TypeDeclBody::Record(List::from(vec![bool_field])),
        meta_where_clause: Maybe::None,
        generic_where_clause: Maybe::None,
        span: span(),
    };
    let ctx = DeriveContext::from_type_decl(&decl, span()).expect("ctx");
    let item = DeriveCommand.expand(&ctx).expect("expand");

    let module = match &item.kind {
        ItemKind::Module(m) => m,
        _ => panic!("expected module"),
    };
    let items = match &module.items {
        Maybe::Some(items) => items,
        _ => panic!("expected items"),
    };
    let factory = match &items[0].kind {
        ItemKind::Function(f) => f,
        _ => panic!("expected fn"),
    };
    let body = match &factory.body {
        Some(FunctionBody::Block(b)) => b,
        _ => panic!("expected block body"),
    };
    let tail = match &body.expr {
        Maybe::Some(e) => &**e,
        _ => panic!("expected tail expr"),
    };

    // The tail expression is `<chain>.build()` — drill down past the
    // outermost MethodCall, then keep peeling MethodCalls until we
    // reach the AppBuilder receiver.
    fn root_receiver(e: &Expr) -> &Expr {
        match &e.kind {
            ExprKind::MethodCall { receiver, .. } => root_receiver(receiver),
            _ => e,
        }
    }
    let root = root_receiver(tail);
    let path = match &root.kind {
        ExprKind::Path(p) => p,
        other => panic!("expected Path receiver, got {:?}", other),
    };
    assert_eq!(path.last_segment_name(), "AppBuilder");
}
