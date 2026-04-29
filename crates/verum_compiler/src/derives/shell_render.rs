//! ShellRender derive macro implementation
//!
//! Generates `implement <Type> { fn render(&self) -> Text }` that produces
//! a quoted shell command line by walking record fields.
//!
//! Field semantics:
//!   * `@flag("-x")` on a `Bool` field — emit `-x` when the value is true.
//!   * `@flag("-x")` on any other field — emit `-x <escaped-value>`.
//!   * `@flag("--key")` — same, with the supplied flag string.
//!   * `@positional` — emit the value as a bare positional argument.
//!   * No attribute — falls back to `--<field-name> <escaped-value>`
//!     (with kebab-case conversion).
//!
//! All values are escaped through `core.shell.escape.Escaper.posix(&value)`
//! before being concatenated. Lists of `Text` produce one flag per element.
//!
//! Example:
//!
//!     @derive(ShellRender)
//!     public type RsyncCmd is {
//!         @flag("-a") archive: Bool,
//!         @flag("-v") verbose: Bool,
//!         @flag("--exclude") excludes: List<Text>,
//!         @positional source: Path,
//!         @positional dest: Path,
//!     };
//!
//! generates a `render(&self) -> Text` whose runtime output is a properly
//! quoted command line.

use super::common::DeriveContext;
use super::{DeriveMacro, DeriveResult};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, Item};
use verum_ast::expr::{Block, Expr, ExprKind, Statement, BinOp, UnOp};
use verum_ast::ty::{Ident, Path, PathSegment, Type, TypeKind};
use verum_ast::literal::{Literal, LiteralKind, StringLit};
use verum_common::{List, Text, Heap};

pub struct DeriveShellRender;

impl DeriveMacro for DeriveShellRender {
    fn name(&self) -> &'static str {
        "ShellRender"
    }

    fn protocol_name(&self) -> &'static str {
        "ShellRender"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        // Build the function body — concatenate quoted segments.
        let body = build_render_body(ctx, span);
        // implement <TypeName> { fn render(&self) -> Text { body } }
        let render_fn = FunctionDecl {
            name:   Ident { name: Text::from("render"), span },
            generics: List::new(),
            params: List::from(vec![ /* &self */ ]),
            return_type: Some(Type {
                kind: TypeKind::Named { path: Path::from_segments(vec![Text::from("Text")]) },
                span,
            }),
            where_clauses: List::new(),
            body: Some(verum_ast::expr::FunctionBody::Block(body)),
            attributes: List::new(),
            visibility: verum_ast::decl::Visibility::Public,
            is_async: false,
            is_meta:  false,
            stage_level: 0,
            is_generator: false,
            using_contexts: List::new(),
            span,
            ..Default::default()
        };
        // Wrap in implement block — relies on the emit-helpers used by other
        // derives; keep API symmetric with `DeriveClone` etc.
        Ok(super::common::wrap_in_implement(ctx, vec![render_fn], span))
    }
}

/// Build the body that produces the joined shell command line.
fn build_render_body(ctx: &DeriveContext, span: Span) -> Block {
    // let mut out = Text.with_capacity(64);
    // ...append per field...
    // out
    let out_ident = Ident { name: Text::from("out"), span };
    let prelude = Statement::LetBind {
        pattern: verum_ast::pattern::Pattern::ident(out_ident.clone(), span),
        init: Some(call_path(
            &["Text", "with_capacity"],
            vec![int_lit(64, span)],
            span,
        )),
        ty: None,
        attributes: List::new(),
        is_mutable: true,
        span,
    };

    let mut stmts: List<Statement> = List::from(vec![prelude]);

    for (idx, field) in ctx.field_iter().enumerate() {
        let segment = render_field(&out_ident, field, idx == 0, span);
        for s in segment { stmts.push(s); }
    }

    // Trailing expression: `out`
    Block {
        statements: stmts,
        last_expr: Some(Heap::new(ident_expr(&out_ident, span))),
        span,
    }
}

fn render_field(
    out: &Ident,
    field: &super::common::FieldInfo,
    is_first: bool,
    span: Span,
) -> Vec<Statement> {
    // Determine semantics: @flag("..."), @positional, or default
    let mut flag_text: Option<Text> = None;
    let mut positional: bool = false;
    for attr in field.attributes.iter() {
        match attr.name.as_str() {
            "flag" => {
                if let Some(arg) = first_string_arg(attr) {
                    flag_text = Some(arg);
                }
            }
            "positional" => positional = true,
            _ => {}
        }
    }

    let mut stmts: Vec<Statement> = Vec::new();
    let field_value = field_access(&field.name, span);

    // Type-aware rendering paths:
    //
    // 1. List<T>     — repeat the flag once per element.  If positional, just
    //                  splat the values space-separated.
    // 2. Maybe<T>    — emit only when the value is `Some(_)`; nothing on `None`.
    // 3. Bool + flag — emit just the flag when true; skip when false.
    // 4. Other       — emit `<flag-or-default> <escaped-value>`.

    if field.is_list() {
        // for x in &self.<field> { ... emit flag + escape(x) ... }
        let body_stmts = if positional {
            vec![
                append_lit(out, Text::from(" "), span),
                append_escape(out, ident_expr(&Ident::new("__elem", span), span), span),
            ]
        } else {
            let flag = flag_text.unwrap_or_else(||
                Text::from(format!("--{}", to_kebab_case(field.name.as_str()))));
            vec![
                append_lit(out, Text::from(" "), span),
                append_lit(out, flag, span),
                append_lit(out, Text::from(" "), span),
                append_escape(out, ident_expr(&Ident::new("__elem", span), span), span),
            ]
        };
        let for_loop = Statement::Expr(Expr::new(
            ExprKind::For {
                pattern: verum_ast::pattern::Pattern::ident(Ident::new("__elem", span), span),
                iter: Heap::new(field_value),
                body: Box::new(Block {
                    statements: List::from(body_stmts),
                    last_expr: None,
                    span,
                }),
                label: None,
                span,
            },
            span,
        ));
        stmts.push(for_loop);
        return stmts;
    }

    if field.is_maybe() {
        // if let Some(x) = &self.<field> { ... }
        let inner_value = ident_expr(&Ident::new("__some", span), span);
        let inner_stmts = if positional {
            vec![
                append_lit(out, Text::from(" "), span),
                append_escape(out, inner_value, span),
            ]
        } else {
            let flag = flag_text.unwrap_or_else(||
                Text::from(format!("--{}", to_kebab_case(field.name.as_str()))));
            vec![
                append_lit(out, Text::from(" "), span),
                append_lit(out, flag, span),
                append_lit(out, Text::from(" "), span),
                append_escape(out, inner_value, span),
            ]
        };
        let if_let = Statement::Expr(Expr::new(
            ExprKind::IfLet {
                pattern: verum_ast::pattern::Pattern::variant(
                    "Some".to_string().into(),
                    vec![Ident::new("__some", span).into()],
                    span,
                ),
                expr: Heap::new(field_value),
                then_block: Box::new(Block {
                    statements: List::from(inner_stmts),
                    last_expr: None,
                    span,
                }),
                else_block: None,
                span,
            },
            span,
        ));
        stmts.push(if_let);
        return stmts;
    }

    // Non-collection field — single emission point. Insert separator first.
    if !is_first {
        stmts.push(append_lit(out, Text::from(" "), span));
    }

    if positional {
        stmts.push(append_escape(out, field_value, span));
    } else if let Some(flag) = flag_text {
        if field.is_bool() {
            // if self.<field> { out.push_str("<flag>"); }
            let then_stmt = append_lit(out, flag.clone(), span);
            stmts.push(Statement::Expr(Expr::new(
                ExprKind::If(Heap::new(verum_ast::expr::IfExpr {
                    condition: Heap::new(field_value),
                    then_block: Block {
                        statements: List::from(vec![then_stmt]),
                        last_expr: None,
                        span,
                    },
                    else_block: None,
                    span,
                })),
                span,
            )));
        } else {
            stmts.push(append_lit(out, flag, span));
            stmts.push(append_lit(out, Text::from(" "), span));
            stmts.push(append_escape(out, field_value, span));
        }
    } else {
        let derived_flag = Text::from(format!("--{}", to_kebab_case(field.name.as_str())));
        stmts.push(append_lit(out, derived_flag, span));
        stmts.push(append_lit(out, Text::from(" "), span));
        stmts.push(append_escape(out, field_value, span));
    }

    stmts
}

/// Extract the first string-literal argument from an attribute, if any.
fn first_string_arg(attr: &verum_ast::Attribute) -> Option<Text> {
    use verum_ast::expr::ExprKind as EK;
    use verum_ast::literal::{LiteralKind, StringLit};
    for arg in attr.args.iter() {
        if let EK::Literal(lit) = &arg.kind {
            if let LiteralKind::Text(StringLit::Regular(s)) = &lit.kind {
                return Some(s.clone());
            }
        }
    }
    None
}

// ----- AST helpers ----------------------------------------------------------

fn ident_expr(id: &Ident, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Ident(id.clone()),
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn field_access(field_name: &Text, span: Span) -> Expr {
    let self_id = Ident { name: Text::from("self"), span };
    Expr {
        kind: ExprKind::Field {
            expr: Heap::new(ident_expr(&self_id, span)),
            field: Ident { name: field_name.clone(), span },
        },
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn int_lit(n: i64, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Literal(Literal {
            kind: LiteralKind::Int(verum_ast::literal::IntLit { value: n as i128, suffix: None }),
            span,
        }),
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn text_lit(s: Text, span: Span) -> Expr {
    Expr {
        kind: ExprKind::Literal(Literal {
            kind: LiteralKind::Text(StringLit::Regular(s)),
            span,
        }),
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

fn call_path(segs: &[&str], args: Vec<Expr>, span: Span) -> Expr {
    let path = Path::from_segments(segs.iter().map(|s| Text::from(*s)).collect());
    let func = Expr {
        kind: ExprKind::Path(path),
        span,
        ref_kind: None,
        check_eliminated: false,
    };
    Expr {
        kind: ExprKind::Call {
            func: Heap::new(func),
            type_args: List::new(),
            args: List::from(args),
        },
        span,
        ref_kind: None,
        check_eliminated: false,
    }
}

/// Append a literal to `out` via `out.push_str("<lit>")`.
fn append_lit(out: &Ident, s: Text, span: Span) -> Statement {
    let call = Expr {
        kind: ExprKind::MethodCall {
            receiver: Heap::new(ident_expr(out, span)),
            method: Ident { name: Text::from("push_str"), span },
            type_args: List::new(),
            args: List::from(vec![text_lit(s, span)]),
        },
        span,
        ref_kind: None,
        check_eliminated: false,
    };
    Statement::Expr(call)
}

/// Append `Escaper.posix(&value)` to `out`.
fn append_escape(out: &Ident, value: Expr, span: Span) -> Statement {
    let borrowed = Expr {
        kind: ExprKind::Unary { op: UnOp::Ref, expr: Heap::new(value) },
        span,
        ref_kind: None,
        check_eliminated: false,
    };
    let escaped = call_path(
        &["core", "shell", "escape", "Escaper", "posix"],
        vec![borrowed],
        span,
    );
    let call = Expr {
        kind: ExprKind::MethodCall {
            receiver: Heap::new(ident_expr(out, span)),
            method: Ident { name: Text::from("push_str"), span },
            type_args: List::new(),
            args: List::from(vec![Expr {
                kind: ExprKind::Unary {
                    op: UnOp::Ref,
                    expr: Heap::new(escaped),
                },
                span,
                ref_kind: None,
                check_eliminated: false,
            }]),
        },
        span,
        ref_kind: None,
        check_eliminated: false,
    };
    Statement::Expr(call)
}

fn to_kebab_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '_' { out.push('-'); }
        else         { out.push(ch); }
    }
    out
}
