//! Structural equality visits matching variant payloads and declared fields in source order.

use super::common::{DeriveContext, path_from_str};
use super::{
    DeriveError, DeriveMacro, DeriveResult, binary_op, bool_lit, ident_expr, method_call, self_ref,
};
use verum_ast::decl::{Item, ItemKind, Visibility};
use verum_ast::pattern::{MatchArm, Pattern, PatternKind};
use verum_ast::ty::{
    GenericParam, GenericParamKind, Ident, Path, Type, TypeBound, TypeBoundKind, TypeKind,
    WhereClause, WherePredicate, WherePredicateKind,
};
use verum_ast::{BinOp, Block, Expr, ExprKind, Span, Stmt, UnOp};
use verum_common::{Heap, List};

const PARTIAL_EQ: &str = "core.base.protocols.PartialEq";
const EQ: &str = "core.base.protocols.Eq";

/// Generate the structural implementation of the comparison operation.
pub struct DerivePartialEq;
/// Generate reflexivity evidence together with its structural comparison.
pub struct DeriveEq;

impl DeriveMacro for DerivePartialEq {
    fn name(&self) -> &'static str {
        "PartialEq"
    }
    fn protocol_name(&self) -> &'static str {
        "PartialEq"
    }
    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        Ok(comparison_impl(ctx, false))
    }
    fn doc_comment(&self) -> &'static str {
        "Structural equality using the field implementations of PartialEq."
    }
}

impl DeriveMacro for DeriveEq {
    fn name(&self) -> &'static str {
        "Eq"
    }
    fn protocol_name(&self) -> &'static str {
        "Eq"
    }
    fn companion_protocol(&self) -> Option<&'static str> {
        Some("PartialEq")
    }
    fn expand_group(&self, ctx: &DeriveContext) -> DeriveResult<List<Item>> {
        // The marker owns the derive argument. Its companion implementation
        // has a distinct source anchor on the nominal declaration.
        let comparison = DeriveContext::from_type_decl(&ctx.type_decl, ctx.type_decl.name.span)?;
        Ok(List::from_iter([
            comparison_impl(&comparison, true),
            ctx.generate_impl_inner(EQ, List::new(), component_conditions(ctx, EQ), ctx.span),
        ]))
    }
    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        Err(DeriveError::RequiresGroup {
            name: self.name().into(),
            span: ctx.span,
        })
    }
    fn doc_comment(&self) -> &'static str {
        "Reflexive structural equality for fields implementing Eq."
    }
}

// Importers consult both declarations before the source owner is checked.
// Require equality of the stored types, including wrappers that can implement
// equality independently of their type arguments.
fn component_conditions(ctx: &DeriveContext, protocol: &str) -> Option<WhereClause> {
    let info = &ctx.type_info;
    let mut predicates = ctx
        .type_decl
        .generic_where_clause
        .as_ref()
        .map(|clause| clause.predicates.clone())
        .unwrap_or_default();
    for field in info
        .fields
        .iter()
        .chain(info.variants.iter().flat_map(|variant| &variant.fields))
    {
        predicates.push(field_obligation(field.ty.clone(), protocol));
    }
    (!predicates.is_empty()).then(|| WhereClause::new(predicates, ctx.span))
}

fn field_obligation(ty: Type, protocol: &str) -> WherePredicate {
    let span = ty.span;
    WherePredicate {
        kind: WherePredicateKind::Type {
            ty,
            bounds: List::from_iter([protocol_bound(protocol, span)]),
        },
        span,
    }
}

fn protocol_bound(protocol: &str, span: Span) -> TypeBound {
    TypeBound {
        kind: TypeBoundKind::Protocol(path_from_str(protocol, span)),
        span,
    }
}

fn reference(ty: Type, span: Span) -> Type {
    Type::new(
        TypeKind::Reference {
            mutable: false,
            inner: Heap::new(ty),
        },
        span,
    )
}

fn borrow(value: Expr) -> Expr {
    let span = value.span;
    Expr::new(
        ExprKind::Unary {
            op: UnOp::Ref,
            expr: Heap::new(value),
        },
        span,
    )
}

fn compare(helper: &str, left: Expr, right: Expr, span: Span) -> Expr {
    Expr::new(
        ExprKind::Call {
            func: Heap::new(ident_expr(helper, span)),
            type_args: List::new(),
            args: List::from_iter([left, right]),
        },
        span,
    )
}

fn comparator(ctx: &DeriveContext, name: &str, total: bool) -> Stmt {
    let span = ctx.span;
    let member = Ident::new("Member", span);
    let member_ty = Type::new(TypeKind::Path(Path::single(member.clone())), span);
    let borrowed = reference(member_ty.clone(), span);
    let call = method_call(
        ident_expr("left", span),
        "eq",
        List::from_iter([ident_expr("right", span)]),
        span,
    );
    let mut function = ctx.method(
        name,
        List::from_iter([
            ctx.param("left", borrowed.clone(), span),
            ctx.param("right", borrowed, span),
        ]),
        Type::bool(span),
        Block {
            stmts: List::new(),
            expr: Some(Heap::new(call)),
            span,
        },
        span,
    );
    function.visibility = Visibility::Private;
    function.generics.push(GenericParam {
        kind: GenericParamKind::Type {
            name: member,
            default: None,
            bounds: List::from_iter([protocol_bound(PARTIAL_EQ, span)]),
        },
        is_implicit: false,
        span,
    });
    if total {
        function.generic_where_clause = Some(WhereClause::new(
            List::from_iter([field_obligation(member_ty, EQ)]),
            span,
        ));
    }
    Stmt::item(Item::new(ItemKind::Function(function), span))
}

// Pair adjacent expressions at each level. Source-order short circuiting is
// preserved and a record with many fields has logarithmic expression depth.
fn conjunction(mut values: List<Expr>, span: Span) -> Expr {
    while values.len() > 1 {
        let mut next = List::with_capacity(values.len().div_ceil(2));
        let mut current = values.into_iter();
        while let Some(left) = current.next() {
            next.push(match current.next() {
                Some(right) => binary_op(left, BinOp::And, right, span),
                None => left,
            });
        }
        values = next;
    }
    values.pop().unwrap_or_else(|| bool_lit(true, span))
}

fn sum_comparison(ctx: &DeriveContext, helper: &str) -> Expr {
    let span = ctx.span;
    let owner = ctx.type_info.name.as_str();
    let arms = ctx
        .type_info
        .variants
        .iter()
        .map(|variant| {
            let span = variant.span;
            let (left, left_fields) = variant.binding_pattern(owner, "left");
            let (right, right_fields) = variant.binding_pattern(owner, "right");
            let fields = left_fields
                .iter()
                .zip(&right_fields)
                .map(|(a, b)| {
                    compare(
                        helper,
                        ident_expr(a.as_str(), a.span),
                        ident_expr(b.as_str(), b.span),
                        a.span,
                    )
                })
                .collect();
            let same = MatchArm::new(right, None, Heap::new(conjunction(fields, span)), span);
            let different = MatchArm::new(
                Pattern::new(PatternKind::Wildcard, span),
                None,
                Heap::new(bool_lit(false, span)),
                span,
            );
            let other = Expr::new(
                ExprKind::Match {
                    expr: Heap::new(ident_expr("other", span)),
                    arms: List::from_iter([same, different]),
                },
                span,
            );
            MatchArm::new(left, None, Heap::new(other), span)
        })
        .collect();
    Expr::new(
        ExprKind::Match {
            expr: Heap::new(self_ref(span)),
            arms,
        },
        span,
    )
}

fn comparison_impl(ctx: &DeriveContext, total: bool) -> Item {
    let span = ctx.span;
    let info = &ctx.type_info;
    let name = format!(
        "$eq_field_{}_{}_{}",
        info.name,
        span.file_id.raw(),
        span.start
    );
    let equal = if info.is_enum {
        sum_comparison(ctx, &name)
    } else {
        let fields = info
            .fields
            .iter()
            .map(|field| {
                let span = field.ty.span;
                compare(
                    &name,
                    borrow(field.access_on_self(span)),
                    borrow(field.access_on_other(span)),
                    span,
                )
            })
            .collect();
        conjunction(fields, span)
    };
    let has_fields = !info.fields.is_empty()
        || info
            .variants
            .iter()
            .any(|variant| !variant.fields.is_empty());
    let stmts = has_fields
        .then(|| comparator(ctx, &name, total))
        .into_iter()
        .collect();
    let method = ctx.method(
        "eq",
        List::from_iter([
            ctx.self_ref_param(span),
            ctx.param(
                "other",
                reference(DeriveContext::self_type(span), span),
                span,
            ),
        ]),
        Type::bool(span),
        Block {
            stmts,
            expr: Some(Heap::new(equal)),
            span,
        },
        span,
    );
    ctx.generate_impl_inner(
        PARTIAL_EQ,
        List::from_iter([method]),
        component_conditions(ctx, if total { EQ } else { PARTIAL_EQ }),
        span,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{TypeDecl, TypeDeclBody, Visibility};
    use verum_common::List;

    fn create_test_struct() -> TypeDecl {
        let span = Span::default();
        TypeDecl {
            value_params: List::new(),
            visibility: Visibility::Public,
            name: Ident::new("Point", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Unit,
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        }
    }

    #[test]
    fn test_derive_partial_eq() {
        let decl = create_test_struct();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DerivePartialEq;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }
}
