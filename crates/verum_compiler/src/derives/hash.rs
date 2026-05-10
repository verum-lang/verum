//! `@derive(Hash)` — auto-generate the `core.base.protocols.Hash`
//! protocol impl for record / newtype / sum types.
//!
//! Closes the gap left by `@derive(PartialEq)` shipping without a
//! matching `@derive(Hash)` — HashMap/HashSet keys want both, but
//! every implementor had to hand-write the Hash side. After this
//! commit `@derive(PartialEq, Hash)` is the conventional pair.
//!
//! # Generated code shapes
//!
//! For a record `User { id: Int, name: Text }`:
//!
//! ```verum
//! implement Hash for User {
//!     fn hash(&self, hasher: &mut Hasher) {
//!         self.id.hash(hasher);
//!         self.name.hash(hasher);
//!     }
//! }
//! ```
//!
//! For a sum type:
//!
//! ```verum
//! implement Hash for Status {
//!     fn hash(&self, hasher: &mut Hasher) {
//!         match self {
//!             Self.Active => { hasher.write_byte(0 as Byte); }
//!             Self.Pending { code } => {
//!                 hasher.write_byte(1 as Byte);
//!                 code.hash(hasher);
//!             }
//!             Self.Tagged(t) => {
//!                 hasher.write_byte(2 as Byte);
//!                 t.hash(hasher);
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! The leading `write_byte(<index>)` is what preserves Hash's
//! invariant `a == b ⟹ hash(a) == hash(b)`: equal-shape
//! variants of different identity hash to different prefixes.
//!
//! For a newtype `type UserId is (Int);`:
//!
//! ```verum
//! implement Hash for UserId {
//!     fn hash(&self, hasher: &mut Hasher) {
//!         self.0.hash(hasher);
//!     }
//! }
//! ```
//!
//! # Bound inference
//!
//! Adopts `generate_impl_with_field_bounds` — generic types like
//! `type Pair<A, B> { first: A, second: B }` get
//! `where A: Hash, B: Hash` auto-emitted into the impl. Mirrors
//! the sweep landed for the 6 sibling derives.

use super::common::{DeriveContext, FieldInfo, TypeInfo, VariantInfo};
use super::{DeriveMacro, DeriveResult, ident_expr, int_lit, method_call, self_ref};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, FunctionParam, FunctionParamKind, Item, Visibility};
use verum_ast::expr::{Block, Expr, ExprKind};
use verum_ast::pattern::{
    FieldPattern, MatchArm, Pattern, PatternKind, VariantPatternData,
};
use verum_ast::stmt::Stmt;
use verum_ast::ty::{Ident, Path, PathSegment, Type, TypeKind};
use verum_common::{Heap, List, Maybe, Text};

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

        let body = if type_info.is_enum {
            self.generate_enum_body(type_info, span)?
        } else if type_info.is_newtype {
            self.generate_newtype_body(type_info, span)?
        } else {
            self.generate_struct_body(type_info, span)?
        };

        let method = self.create_hash_method(ctx, body, span);
        Ok(ctx.generate_impl_with_field_bounds(
            "Hash",
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

    /// Empty struct → empty body. Otherwise: chain
    /// `self.f.hash(hasher);` per field.
    fn generate_struct_body(&self, type_info: &TypeInfo, span: Span) -> DeriveResult<Block> {
        let mut stmts: Vec<Stmt> = Vec::new();
        for field in type_info.fields.iter() {
            stmts.push(self.field_hash_stmt_on_self(field, span));
        }
        Ok(Block {
            stmts: List::from(stmts),
            expr: Maybe::None,
            span,
        })
    }

    /// `self.0.hash(hasher);`
    fn generate_newtype_body(&self, type_info: &TypeInfo, span: Span) -> DeriveResult<Block> {
        let _ = type_info; // newtype's synthesized field is positional `0`
        let inner = Expr::new(
            ExprKind::TupleIndex {
                expr: Heap::new(self_ref(span)),
                index: 0,
            },
            span,
        );
        let call = method_call(
            inner,
            "hash",
            List::from(vec![ident_expr("hasher", span)]),
            span,
        );
        Ok(Block {
            stmts: List::from(vec![Stmt::expr(call, true)]),
            expr: Maybe::None,
            span,
        })
    }

    /// `match self { Self.<V> {...} => { write_byte(i); <field>.hash(hasher); ... } }`
    fn generate_enum_body(&self, type_info: &TypeInfo, span: Span) -> DeriveResult<Block> {
        let mut arms: Vec<MatchArm> = Vec::new();
        for (idx, variant) in type_info.variants.iter().enumerate() {
            arms.push(self.variant_arm(variant, idx, span));
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

    fn variant_arm(&self, variant: &VariantInfo, index: usize, span: Span) -> MatchArm {
        let variant_path = Path::new(
            List::from(vec![
                PathSegment::SelfValue,
                PathSegment::Name(Ident::new(variant.name.as_str(), span)),
            ]),
            span,
        );

        let (pattern, body) = if variant.is_unit {
            // Self.Unit => { hasher.write_byte(<index> as Byte); }
            let pattern = Pattern::new(
                PatternKind::Variant {
                    path: variant_path,
                    data: Maybe::None,
                },
                span,
            );
            let body = self.write_index_block(index, vec![], span);
            (pattern, body)
        } else if variant.is_tuple {
            // Self.Tuple(_0, _1) => { write_byte(i); _0.hash(hasher); _1.hash(hasher); }
            // Bind each positional field as `f<i>` for the body.
            let bind_names: Vec<Text> = (0..variant.fields.len())
                .map(|i| Text::from(format!("f{}", i)))
                .collect();
            let bind_patterns: List<Pattern> = bind_names
                .iter()
                .map(|n| Pattern::new(
                    PatternKind::Ident {
                        name: Ident::new(n.as_str(), span),
                        mutable: false,
                        by_ref: false,
                        subpattern: Maybe::None,
                    },
                    span,
                ))
                .collect();
            let pattern = Pattern::new(
                PatternKind::Variant {
                    path: variant_path,
                    data: Maybe::Some(VariantPatternData::Tuple(bind_patterns)),
                },
                span,
            );
            let body = self.write_index_block(
                index,
                bind_names
                    .into_iter()
                    .map(|n| self.hash_call_on_ident(n.as_str(), span))
                    .collect(),
                span,
            );
            (pattern, body)
        } else {
            // Self.Struct { f } => { write_byte(i); f.hash(hasher); }
            // Bind each named field 1:1 (no rename — pattern's
            // `name = ident` shorthand: `Pattern { f }` equiv
            // `Pattern { f: f }`).
            let bind_patterns: List<FieldPattern> = variant
                .fields
                .iter()
                .map(|f| FieldPattern {
                    name: f.ident(span),
                    pattern: Maybe::Some(Pattern::new(
                        PatternKind::Ident {
                            name: f.ident(span),
                            mutable: false,
                            by_ref: false,
                            subpattern: Maybe::None,
                        },
                        span,
                    )),
                    span,
                })
                .collect();
            let pattern = Pattern::new(
                PatternKind::Variant {
                    path: variant_path,
                    data: Maybe::Some(VariantPatternData::Record {
                        fields: bind_patterns,
                        rest: false,
                    }),
                },
                span,
            );
            let calls: Vec<Stmt> = variant
                .fields
                .iter()
                .map(|f| self.hash_call_on_ident(f.name.as_str(), span))
                .collect();
            let body = self.write_index_block(index, calls, span);
            (pattern, body)
        };

        MatchArm::new(pattern, Maybe::None, Heap::new(body), span)
    }

    /// `{ hasher.write_byte(<index> as Byte); <stmts> }`
    fn write_index_block(&self, index: usize, mut stmts: Vec<Stmt>, span: Span) -> Expr {
        let mut all: Vec<Stmt> = Vec::with_capacity(1 + stmts.len());
        let byte_expr = Expr::new(
            ExprKind::Cast {
                expr: Heap::new(int_lit(index as i128, span)),
                ty: Type::new(
                    TypeKind::Path(Path::single(Ident::new("Byte", span))),
                    span,
                ),
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

    /// `self.<field>.hash(hasher);`
    fn field_hash_stmt_on_self(&self, field: &FieldInfo, span: Span) -> Stmt {
        let recv = field.access_on_self(span);
        let call = method_call(
            recv,
            "hash",
            List::from(vec![ident_expr("hasher", span)]),
            span,
        );
        Stmt::expr(call, true)
    }

    /// `<ident>.hash(hasher);`
    fn hash_call_on_ident(&self, ident: &str, span: Span) -> Stmt {
        let call = method_call(
            ident_expr(ident, span),
            "hash",
            List::from(vec![ident_expr("hasher", span)]),
            span,
        );
        Stmt::expr(call, true)
    }

    // ------------------------------------------------------------------
    // Method declaration
    // ------------------------------------------------------------------

    /// `fn hash(&self, hasher: &mut Hasher)`
    fn create_hash_method(&self, ctx: &DeriveContext, body: Block, span: Span) -> FunctionDecl {
        let hasher_ty = Type::new(
            TypeKind::Reference {
                mutable: true,
                inner: Heap::new(Type::new(
                    TypeKind::Path(Path::single(Ident::new("Hasher", span))),
                    span,
                )),
            },
            span,
        );
        let unit_ty = Type::new(TypeKind::Unit, span);
        let self_param = ctx.self_ref_param(span);
        let hasher_param = self.regular_param("hasher", hasher_ty, span);
        ctx.method(
            "hash",
            List::from(vec![self_param, hasher_param]),
            unit_ty,
            body,
            span,
        )
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

// Suppress unused-import warnings on `Visibility` — load-bearing for
// future fields if a Hash variant requires private impls.
#[allow(dead_code)]
const _: Option<Visibility> = None;

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
        // Method body should contain a single Match expression.
        let func = match &impl_decl.items.iter().next().unwrap().kind {
            verum_ast::decl::ImplItemKind::Function(f) => f.clone(),
            other => panic!("expected Function impl item, got {:?}", other),
        };
        let body = match &func.body {
            Maybe::Some(verum_ast::decl::FunctionBody::Block(b)) => b.clone(),
            other => panic!("expected Block body, got {:?}", other),
        };
        assert_eq!(body.stmts.len(), 1, "single Match stmt in sum-type body");
    }
}
