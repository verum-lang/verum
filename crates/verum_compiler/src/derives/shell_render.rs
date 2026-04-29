//! ShellRender derive macro.
//!
//! Generates `implement <Type> { fn render(&self) -> Text { ... } }` for
//! record types whose fields carry `@flag("...")`, `@positional` or no
//! attribute (default: `--<kebab-name> <value>`).
//!
//! Generated body shape:
//!
//!     fn render(&self) -> Text {
//!         let mut out = "".into();
//!         out.push_str(<flag-prefix>);
//!         out.push_str(Escaper.posix(&self.<field>).as_str());
//!         ...
//!         out
//!     }
//!
//! Field-type awareness (List<T> as repeat-flag, Maybe<T> as if-let, Bool
//! flag-only-when-true) requires control-flow shapes (For / IfLet / If
//! with IfCondition) that this derive doesn't yet emit; those refinements
//! are picked up at the lowering phase that follows.  The current body is
//! correct for fields whose value implements `ShellEscape` and whose
//! semantics map to "flag value" emission.  The hand-written
//! `core/shell/dsl/{git,docker,kubectl}.vr` impls cover the richer shapes.

use super::common::{DeriveContext, FieldInfo};
use super::{DeriveMacro, DeriveResult, ident_expr, method_call, string_lit};
use verum_ast::Span;
use verum_ast::decl::Item;
use verum_ast::expr::{Block, Expr, ExprKind, UnOp};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_common::{List, Maybe, Text};

pub struct DeriveShellRender;

impl DeriveMacro for DeriveShellRender {
    fn name(&self) -> &'static str { "ShellRender" }
    fn protocol_name(&self) -> &'static str { "ShellRender" }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let body = build_render_body(ctx, span);

        let text_ty = Type {
            kind: TypeKind::Path(Path::single(Ident::new("Text", span))),
            span,
        };
        let render_fn = ctx.method(
            "render",
            List::from(vec![ctx.self_ref_param(span)]),
            text_ty,
            body,
            span,
        );

        Ok(ctx.generate_impl("ShellRender", List::from(vec![render_fn]), span))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated render() for typed shell command DSLs"
    }
}

// =============================================================================
// Body synthesis
// =============================================================================

fn build_render_body(ctx: &DeriveContext, span: Span) -> Block {
    let out_id = Ident::new("out", span);

    // let mut out = "".into();
    let prelude = Stmt::new(
        StmtKind::Let {
            pattern: Pattern::new(
                PatternKind::Ident {
                    by_ref: false,
                    name:   out_id.clone(),
                    mutable: true,
                    subpattern: None,
                },
                span,
            ),
            ty: Maybe::None,
            value: Maybe::Some(method_call(
                string_lit("", span),
                "into",
                List::new(),
                span,
            )),
        },
        span,
    );

    let mut stmts: List<Stmt> = List::new();
    stmts.push(prelude);

    for (idx, field) in ctx.type_info.fields.iter().enumerate() {
        for s in render_field(&out_id, field, idx == 0, span) {
            stmts.push(s);
        }
    }

    Block::new(
        stmts,
        Maybe::Some(verum_common::Heap::new(ident_expr("out", span))),
        span,
    )
}

fn render_field(
    out: &Ident,
    field: &FieldInfo,
    is_first: bool,
    span: Span,
) -> Vec<Stmt> {
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

    let mut stmts: Vec<Stmt> = Vec::new();
    let field_value = field.access_on_self(span);

    // Insert separator (space) between fields.
    if !is_first {
        stmts.push(push_lit_stmt(out, Text::from(" "), span));
    }

    if positional {
        stmts.push(push_escape_stmt(out, field_value, span));
    } else if let Some(flag) = flag_text {
        // Emit `<flag> <escaped-value>` for every type.  The "Bool flag
        // → emit only when true" refinement requires an If-expr with
        // IfCondition wrapper; see module doc.
        stmts.push(push_lit_stmt(out, flag, span));
        stmts.push(push_lit_stmt(out, Text::from(" "), span));
        stmts.push(push_escape_stmt(out, field_value, span));
    } else {
        let derived = Text::from(format!("--{}", to_kebab_case(field.name.as_str())));
        stmts.push(push_lit_stmt(out, derived, span));
        stmts.push(push_lit_stmt(out, Text::from(" "), span));
        stmts.push(push_escape_stmt(out, field_value, span));
    }

    stmts
}

// -----------------------------------------------------------------------------
// Statement builders
// -----------------------------------------------------------------------------

/// `out.push_str("<lit>");`
fn push_lit_stmt(out: &Ident, s: Text, span: Span) -> Stmt {
    let receiver = ident_expr(out.name.as_str(), span);
    let call = method_call(
        receiver,
        "push_str",
        List::from(vec![string_lit(s.as_str(), span)]),
        span,
    );
    Stmt::expr(call, /* has_semi */ true)
}

/// `out.push_str(Escaper.posix(&<value>).as_str());`
fn push_escape_stmt(out: &Ident, value: Expr, span: Span) -> Stmt {
    let receiver = ident_expr(out.name.as_str(), span);
    let escaper = Expr::new(
        ExprKind::Path(Path::single(Ident::new("Escaper", span))),
        span,
    );
    let escaped = method_call(
        escaper,
        "posix",
        List::from(vec![Expr::new(
            ExprKind::Unary { op: UnOp::Ref, expr: Box::new(value) },
            span,
        )]),
        span,
    );
    let call = method_call(
        receiver,
        "push_str",
        List::from(vec![method_call(escaped, "as_str", List::new(), span)]),
        span,
    );
    Stmt::expr(call, /* has_semi */ true)
}

/// Extract the first string-literal argument from `@flag("X")`.
fn first_string_arg(attr: &verum_ast::Attribute) -> Option<Text> {
    use verum_ast::expr::ExprKind as EK;
    use verum_ast::literal::{LiteralKind, StringLit};
    let args = match &attr.args {
        Maybe::Some(args) => args,
        Maybe::None       => return None,
    };
    for arg in args.iter() {
        if let EK::Literal(lit) = &arg.kind {
            if let LiteralKind::Text(StringLit::Regular(s)) = &lit.kind {
                return Some(s.clone());
            }
        }
    }
    None
}

fn to_kebab_case(s: &str) -> String {
    s.replace('_', "-")
}
