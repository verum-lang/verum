//! `@derive(Hash)` — auto-generate the `core.base.protocols.Hash`
//! protocol impl for record / newtype / sum types.
//!
//! The implementation visits record and tuple fields in declaration order.
//! Sum variants first feed their discriminator. A local generic function
//! borrows each payload with the canonical Hash bound, so its body selects
//! the protocol method and each caller must supply a conformance witness.

use super::common::{DeriveContext, TypeInfo, VariantInfo, path_from_str};
use super::{DeriveMacro, DeriveResult, ident_expr, int_lit, method_call, self_ref};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, FunctionParam, FunctionParamKind, Item, ItemKind, Visibility};
use verum_ast::expr::{Block, Expr, ExprKind, UnOp};
use verum_ast::pattern::{MatchArm, Pattern, PatternKind};
use verum_ast::stmt::Stmt;
use verum_ast::ty::{
    GenericParam, GenericParamKind, Ident, Path, Type, TypeBound, TypeBoundKind,
    TypeKind,
};
use verum_common::{Heap, List, Maybe};

/// `@derive(Hash)` macro implementation.
pub struct DeriveHash;

impl DeriveMacro for DeriveHash {
    fn name(&self) -> &'static str {
        "Hash"
    }

    fn protocol_name(&self) -> &'static str {
        "Hash"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;
        // Nested methods may share a leaf name in one compilation unit. Keep
        // the generated local outside the source identifier alphabet.
        let helper = format!(
            "$hash_field_{}_{}_{}",
            type_info.name,
            span.file_id.raw(),
            span.start
        );

        let mut body = if type_info.is_enum {
            self.generate_enum_body(type_info, &helper, span)?
        } else {
            self.generate_struct_body(type_info, &helper, span)?
        };

        if !type_info.fields.is_empty()
            || type_info
                .variants
                .iter()
                .any(|variant| !variant.fields.is_empty())
        {
            body.stmts.insert(0, self.field_hasher(ctx, &helper, span));
        }
        let method = self.create_hash_method(ctx, body, span);
        Ok(ctx.generate_impl_with_field_bounds(
            "core.base.protocols.Hash",
            List::from(vec![method]),
            span,
        ))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated Hash implementation — chains per-field .hash(hasher) \
         calls; sum variants prefix with a 1-byte discriminator."
    }
}

impl DeriveHash {
    // ------------------------------------------------------------------
    // Body generators
    // ------------------------------------------------------------------

    fn generate_struct_body(
        &self,
        type_info: &TypeInfo,
        helper: &str,
        span: Span,
    ) -> DeriveResult<Block> {
        Ok(Block {
            stmts: type_info
                .fields
                .iter()
                .map(|field| {
                    let span = field.ty.span;
                    let value = field.access_on_self(span);
                    let borrowed = Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Ref,
                            expr: Heap::new(value),
                        },
                        span,
                    );
                    self.hash_field(helper, borrowed, span)
                })
                .collect(),
            expr: None,
            span,
        })
    }

    /// `match self { Self.<V> {...} => { write_byte(i); <field>.hash(hasher); ... } }`
    fn generate_enum_body(
        &self,
        type_info: &TypeInfo,
        helper: &str,
        span: Span,
    ) -> DeriveResult<Block> {
        let mut arms: Vec<MatchArm> = Vec::new();
        for (idx, variant) in type_info.variants.iter().enumerate() {
            arms.push(self.variant_arm(type_info.name.as_str(), variant, idx, helper, variant.span));
        }
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Heap::new(self_ref(span)),
                arms: List::from(arms),
            },
            span,
        );
        Ok(Block {
            stmts: List::from(vec![Stmt::expr(match_expr, true)]),
            expr: Maybe::None,
            span,
        })
    }

    fn variant_arm(
        &self,
        owner: &str,
        variant: &VariantInfo,
        index: usize,
        helper: &str,
        span: Span,
    ) -> MatchArm {
        let (pattern, names) = variant.binding_pattern(owner, "field");
        let calls = names
            .iter()
            .zip(&variant.fields)
            .map(|(name, field)| {
                self.hash_field(
                    helper,
                    ident_expr(name.as_str(), field.ty.span),
                    field.ty.span,
                )
            })
            .collect();
        let body = self.write_index_block(index, calls, span);

        MatchArm::new(pattern, Maybe::None, Heap::new(body), span)
    }

    /// `{ hasher.write_byte(<index> as Byte); <stmts> }`
    fn write_index_block(&self, index: usize, mut stmts: Vec<Stmt>, span: Span) -> Expr {
        let mut all: Vec<Stmt> = Vec::with_capacity(1 + stmts.len());
        let byte_expr = Expr::new(
            ExprKind::Cast {
                expr: Heap::new(int_lit(index as i128, span)),
                ty: Type::new(TypeKind::Path(Path::single(Ident::new("Byte", span))), span),
            },
            span,
        );
        let write_call = method_call(
            ident_expr("hasher", span),
            "write_byte",
            List::from(vec![byte_expr]),
            span,
        );
        all.push(Stmt::expr(write_call, true));
        all.append(&mut stmts);
        Expr::new(
            ExprKind::Block(Block {
                stmts: List::from(all),
                expr: Maybe::None,
                span,
            }),
            span,
        )
    }

    // ------------------------------------------------------------------
    // Per-field hash calls
    // ------------------------------------------------------------------

    fn hash_field(&self, helper: &str, borrowed: Expr, span: Span) -> Stmt {
        let call = Expr::new(
            ExprKind::Call {
                func: Heap::new(ident_expr(helper, span)),
                type_args: List::new(),
                args: List::from(vec![borrowed, ident_expr("hasher", span)]),
            },
            span,
        );
        Stmt::expr(call, true)
    }

    fn field_hasher(&self, ctx: &DeriveContext, name: &str, span: Span) -> Stmt {
        let parameter = Ident::new("Member", span);
        let ty = Type::new(TypeKind::Path(Path::single(parameter.clone())), span);
        let member = self.regular_param(
            "member",
            Type::new(
                TypeKind::Reference {
                    mutable: false,
                    inner: Heap::new(ty),
                },
                span,
            ),
            span,
        );
        let feed = method_call(
            ident_expr("member", span),
            "hash",
            List::from(vec![ident_expr("hasher", span)]),
            span,
        );
        let body = Block {
            stmts: List::from(vec![Stmt::expr(feed, true)]),
            expr: None,
            span,
        };
        let mut helper = ctx.method(
            name,
            List::from(vec![member, self.hasher_param(span)]),
            Type::new(TypeKind::Unit, span),
            body,
            span,
        );
        helper.visibility = Visibility::Private;
        helper.generics.push(GenericParam {
            kind: GenericParamKind::Type {
                name: parameter,
                default: None,
                bounds: List::from(vec![TypeBound {
                    kind: TypeBoundKind::Protocol(path_from_str(
                        "core.base.protocols.Hash",
                        span,
                    )),
                    span,
                }]),
            },
            is_implicit: false,
            span,
        });
        Stmt::item(Item::new(ItemKind::Function(helper), span))
    }

    // ------------------------------------------------------------------
    // Method declaration
    // ------------------------------------------------------------------

    /// `fn hash(&self, hasher: &mut Hasher)`
    fn create_hash_method(&self, ctx: &DeriveContext, body: Block, span: Span) -> FunctionDecl {
        let unit_ty = Type::new(TypeKind::Unit, span);
        let self_param = ctx.self_ref_param(span);
        let hasher_param = self.hasher_param(span);
        ctx.method(
            "hash",
            List::from(vec![self_param, hasher_param]),
            unit_ty,
            body,
            span,
        )
    }

    fn hasher_param(&self, span: Span) -> FunctionParam {
        let hasher_ty = Type::new(
            TypeKind::Reference {
                mutable: true,
                inner: Heap::new(Type::new(
                    TypeKind::Path(path_from_str("core.base.protocols.Hasher", span)),
                    span,
                )),
            },
            span,
        );
        self.regular_param("hasher", hasher_ty, span)
    }

    fn regular_param(&self, name: &str, ty: Type, span: Span) -> FunctionParam {
        FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        name: Ident::new(name, span),
                        mutable: false,
                        by_ref: false,
                        subpattern: Maybe::None,
                    },
                    span,
                ),
                ty,
                default_value: Maybe::None,
            },
            span,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{
        RecordField, TypeDecl, TypeDeclBody, Variant, VariantData, Visibility,
    };

    fn make_record_decl(name: &str, fields: Vec<(&str, &str)>) -> TypeDecl {
        let span = Span::default();
        let record_fields: Vec<RecordField> = fields
            .into_iter()
            .map(|(fname, fty)| RecordField {
                attributes: List::new(),
                visibility: Visibility::Public,
                name: Ident::new(fname, span),
                ty: Type::new(
                    TypeKind::Path(Path::single(Ident::new(fty, span))),
                    span,
                ),
                default_value: Maybe::None,
                bit_spec: Maybe::None,
                span,
            })
            .collect();
        TypeDecl {
            value_params: List::new(),
            visibility: Visibility::Public,
            name: Ident::new(name, span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Record(List::from(record_fields)),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        }
    }

    #[test]
    fn record_two_fields_compiles() {
        let decl = make_record_decl("Pair", vec![("first", "Int"), ("second", "Text")]);
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let item = DeriveHash.expand(&ctx).expect("expand should succeed");
        match item.kind {
            verum_ast::decl::ItemKind::Impl(_) => {}
            other => panic!("expected ItemKind::Impl, got {:?}", other),
        }
    }

    #[test]
    fn newtype_compiles() {
        let span = Span::default();
        let inner = Type::new(
            TypeKind::Path(Path::single(Ident::new("Int", span))),
            span,
        );
        let decl = TypeDecl {
            value_params: List::new(),
            visibility: Visibility::Public,
            name: Ident::new("UserId", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Newtype(inner),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        };
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let item = DeriveHash.expand(&ctx).expect("newtype expand should succeed");
        match item.kind {
            verum_ast::decl::ItemKind::Impl(_) => {}
            other => panic!("expected ItemKind::Impl, got {:?}", other),
        }
    }

    /// Sum type with mixed variants (unit + tuple + record) compiles.
    #[test]
    fn sum_type_mixed_variants_compiles() {
        let span = Span::default();
        let unit_v = Variant::new(Ident::new("Unit", span), Maybe::None, span);
        let tuple_v = Variant::new(
            Ident::new("Tuple", span),
            Maybe::Some(VariantData::Tuple(List::from(vec![Type::new(
                TypeKind::Path(Path::single(Ident::new("Int", span))),
                span,
            )]))),
            span,
        );
        let record_v = Variant::new(
            Ident::new("Record", span),
            Maybe::Some(VariantData::Record(List::from(vec![RecordField {
                attributes: List::new(),
                visibility: Visibility::Public,
                name: Ident::new("code", span),
                ty: Type::new(
                    TypeKind::Path(Path::single(Ident::new("Int", span))),
                    span,
                ),
                default_value: Maybe::None,
                bit_spec: Maybe::None,
                span,
            }]))),
            span,
        );

        let decl = TypeDecl {
            value_params: List::new(),
            visibility: Visibility::Public,
            name: Ident::new("Status", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Variant(List::from(vec![unit_v, tuple_v, record_v])),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        };
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let item = DeriveHash.expand(&ctx).expect("sum type expand should succeed");
        let impl_decl = match item.kind {
            verum_ast::decl::ItemKind::Impl(i) => i,
            other => panic!("expected Impl, got {:?}", other),
        };
        // The bounded helper precedes the match over the sum.
        let func = match &impl_decl.items.iter().next().unwrap().kind {
            verum_ast::decl::ImplItemKind::Function(f) => f.clone(),
            other => panic!("expected Function impl item, got {:?}", other),
        };
        let body = match &func.body {
            Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => b.clone(),
            other => panic!("expected Block body, got {:?}", other),
        };
        assert!(matches!(&body.stmts.last().unwrap().kind, verum_ast::stmt::StmtKind::Expr { expr, .. } if matches!(expr.kind, ExprKind::Match { .. })));
    }
}
