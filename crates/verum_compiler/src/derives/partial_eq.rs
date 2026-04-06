//! PartialEq derive macro implementation
//!
//! Generates `implement PartialEq for Type { fn eq(&self, other: &Self) -> Bool }`
//!
//! For record types: compares all fields pairwise with &&. For sum types: matches
//! variant tags first, then compares payloads field-by-field. Returns false for
//! mismatched variants. NaN != NaN for float fields (IEEE 754 semantics).

use super::common::DeriveContext;
use super::{DeriveMacro, DeriveResult, binary_op, bool_lit, ident_expr, self_ref};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, Item};
use verum_ast::expr::{BinOp, Block, Expr, ExprKind};
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_common::List;
use verum_common::well_known_types::type_names;

/// PartialEq derive implementation
pub struct DerivePartialEq;

impl DeriveMacro for DerivePartialEq {
    fn name(&self) -> &'static str {
        "PartialEq"
    }

    fn protocol_name(&self) -> &'static str {
        "PartialEq"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Generate eq method body
        let body = if type_info.is_enum {
            self.generate_enum_eq(type_info, span)?
        } else if type_info.is_newtype {
            self.generate_newtype_eq(type_info, span)?
        } else {
            self.generate_struct_eq(type_info, span)?
        };

        // Create the eq method
        let eq_method = self.create_eq_method(ctx, body, span);

        // Generate impl block
        Ok(ctx.generate_impl("PartialEq", List::from(vec![eq_method]), span))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated PartialEq implementation (~30ns CBGR per field comparison)."
    }
}

impl DerivePartialEq {
    /// Generate eq body for struct types
    fn generate_struct_eq(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        if type_info.fields.is_empty() {
            // Empty struct: always equal
            return Ok(Block {
                stmts: vec![].into(),
                expr: Some(Box::new(bool_lit(true, span))),
                span,
            });
        }

        // Build: self.f1 == other.f1 && self.f2 == other.f2 && ...
        let mut result: Option<Expr> = None;

        for field in type_info.fields.iter() {
            let self_field = field.access_on_self(span);
            let other_field = field.access_on_other(span);

            let field_eq = binary_op(self_field, BinOp::Eq, other_field, span);

            result = Some(match result {
                None => field_eq,
                Some(prev) => binary_op(prev, BinOp::And, field_eq, span),
            });
        }

        let expr = result.unwrap_or_else(|| bool_lit(true, span));

        Ok(Block {
            stmts: vec![].into(),
            expr: Some(Box::new(expr)),
            span,
        })
    }

    /// Generate eq body for newtype
    fn generate_newtype_eq(
        &self,
        _type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        // Compare inner values: self.0 == other.0
        let self_inner = Expr::new(
            ExprKind::TupleIndex {
                expr: Box::new(self_ref(span)),
                index: 0,
            },
            span,
        );
        let other_inner = Expr::new(
            ExprKind::TupleIndex {
                expr: Box::new(ident_expr("other", span)),
                index: 0,
            },
            span,
        );

        let eq_expr = binary_op(self_inner, BinOp::Eq, other_inner, span);

        Ok(Block {
            stmts: vec![].into(),
            expr: Some(Box::new(eq_expr)),
            span,
        })
    }

    /// Generate eq body for enum types
    ///
    /// Generates match expression:
    /// ```verum
    /// match (self, other) {
    ///     (Self::Unit, Self::Unit) => true,
    ///     (Self::Tuple(a0, a1), Self::Tuple(b0, b1)) => a0 == b0 && a1 == b1,
    ///     (Self::Struct { field: a }, Self::Struct { field: b }) => a == b,
    ///     _ => false,
    /// }
    /// ```
    fn generate_enum_eq(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        use verum_ast::pattern::{
            FieldPattern, MatchArm, Pattern, PatternKind, VariantPatternData,
        };
        use verum_ast::ty::PathSegment;
        use verum_common::{Heap, Maybe};

        let mut arms = Vec::new();

        for variant in type_info.variants.iter() {
            let variant_name = variant.name.as_str();

            // Build path: Self::VariantName
            let variant_path = Path::new(
                List::from(vec![
                    PathSegment::SelfValue,
                    PathSegment::Name(Ident::new(variant_name, span)),
                ]),
                span,
            );

            let (pattern, body) = if variant.fields.is_empty() {
                // Unit variant: (Self::Variant, Self::Variant) => true
                let self_pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path.clone(),
                        data: Maybe::None,
                    },
                    span,
                );
                let other_pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::None,
                    },
                    span,
                );
                let tuple_pattern = Pattern::new(
                    PatternKind::Tuple(List::from(vec![self_pattern, other_pattern])),
                    span,
                );
                (tuple_pattern, bool_lit(true, span))
            } else if variant.fields.iter().all(|f| {
                let name = f.name.as_str();
                name.starts_with('_')
                    || name.chars().next().map(|c| c.is_numeric()).unwrap_or(false)
                    || name.is_empty()
            }) {
                // Tuple variant: (Self::Variant(a0, a1), Self::Variant(b0, b1)) => a0 == b0 && a1 == b1
                let self_field_patterns: List<Pattern> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        Pattern::new(
                            PatternKind::Ident {
                                name: Ident::new(format!("a{}", i), span),
                                mutable: false,
                                by_ref: false,
                                subpattern: Maybe::None,
                            },
                            span,
                        )
                    })
                    .collect();
                let other_field_patterns: List<Pattern> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        Pattern::new(
                            PatternKind::Ident {
                                name: Ident::new(format!("b{}", i), span),
                                mutable: false,
                                by_ref: false,
                                subpattern: Maybe::None,
                            },
                            span,
                        )
                    })
                    .collect();

                let self_pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path.clone(),
                        data: Maybe::Some(VariantPatternData::Tuple(self_field_patterns)),
                    },
                    span,
                );
                let other_pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::Some(VariantPatternData::Tuple(other_field_patterns)),
                    },
                    span,
                );
                let tuple_pattern = Pattern::new(
                    PatternKind::Tuple(List::from(vec![self_pattern, other_pattern])),
                    span,
                );

                // Build: a0 == b0 && a1 == b1 && ...
                let mut result: Option<Expr> = None;
                for i in 0..variant.fields.len() {
                    let a = ident_expr(&format!("a{}", i), span);
                    let b = ident_expr(&format!("b{}", i), span);
                    let field_eq = binary_op(a, BinOp::Eq, b, span);
                    result = Some(match result {
                        None => field_eq,
                        Some(prev) => binary_op(prev, BinOp::And, field_eq, span),
                    });
                }
                let body = result.unwrap_or_else(|| bool_lit(true, span));
                (tuple_pattern, body)
            } else {
                // Struct variant: (Self::Variant { field: a }, Self::Variant { field: b }) => a == b
                let self_field_patterns: List<FieldPattern> = variant
                    .fields
                    .iter()
                    .map(|f| FieldPattern {
                        name: Ident::new(f.name.as_str(), span),
                        pattern: Maybe::Some(Pattern::new(
                            PatternKind::Ident {
                                name: Ident::new(format!("a_{}", f.name.as_str()), span),
                                mutable: false,
                                by_ref: false,
                                subpattern: Maybe::None,
                            },
                            span,
                        )),
                        span,
                    })
                    .collect();
                let other_field_patterns: List<FieldPattern> = variant
                    .fields
                    .iter()
                    .map(|f| FieldPattern {
                        name: Ident::new(f.name.as_str(), span),
                        pattern: Maybe::Some(Pattern::new(
                            PatternKind::Ident {
                                name: Ident::new(format!("b_{}", f.name.as_str()), span),
                                mutable: false,
                                by_ref: false,
                                subpattern: Maybe::None,
                            },
                            span,
                        )),
                        span,
                    })
                    .collect();

                let self_pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path.clone(),
                        data: Maybe::Some(VariantPatternData::Record {
                            fields: self_field_patterns,
                            rest: false,
                        }),
                    },
                    span,
                );
                let other_pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::Some(VariantPatternData::Record {
                            fields: other_field_patterns,
                            rest: false,
                        }),
                    },
                    span,
                );
                let tuple_pattern = Pattern::new(
                    PatternKind::Tuple(List::from(vec![self_pattern, other_pattern])),
                    span,
                );

                // Build: a_field1 == b_field1 && a_field2 == b_field2 && ...
                let mut result: Option<Expr> = None;
                for field in variant.fields.iter() {
                    let field_name = field.name.as_str();
                    let a = ident_expr(&format!("a_{}", field_name), span);
                    let b = ident_expr(&format!("b_{}", field_name), span);
                    let field_eq = binary_op(a, BinOp::Eq, b, span);
                    result = Some(match result {
                        None => field_eq,
                        Some(prev) => binary_op(prev, BinOp::And, field_eq, span),
                    });
                }
                let body = result.unwrap_or_else(|| bool_lit(true, span));
                (tuple_pattern, body)
            };

            arms.push(MatchArm::new(pattern, Maybe::None, Heap::new(body), span));
        }

        // Add wildcard arm for different variants: _ => false
        let wildcard_pattern = Pattern::new(PatternKind::Wildcard, span);
        arms.push(MatchArm::new(
            wildcard_pattern,
            Maybe::None,
            Heap::new(bool_lit(false, span)),
            span,
        ));

        // Build tuple: (self, other)
        let self_expr = self_ref(span);
        let other_expr = ident_expr("other", span);
        let tuple_expr = Expr::new(
            ExprKind::Tuple(List::from(vec![self_expr, other_expr])),
            span,
        );

        // Build match expression
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Heap::new(tuple_expr),
                arms: List::from(arms),
            },
            span,
        );

        Ok(Block {
            stmts: vec![].into(),
            expr: Some(Box::new(match_expr)),
            span,
        })
    }

    /// Create the eq method declaration
    fn create_eq_method(&self, ctx: &DeriveContext, body: Block, span: Span) -> FunctionDecl {
        // Return type: Bool
        let return_type = Type::new(TypeKind::Path(Path::single(Ident::new(type_names::BOOL, span))), span);

        // Parameter: other: &Self
        let other_type = Type::new(
            TypeKind::Reference {
                mutable: false,
                inner: Box::new(Type::new(
                    TypeKind::Path(Path::single(Ident::new("Self", span))),
                    span,
                )),
            },
            span,
        );
        let other_param = ctx.param("other", other_type, span);

        ctx.method(
            "eq",
            List::from(vec![ctx.self_ref_param(span), other_param]),
            return_type,
            body,
            span,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{TypeDecl, TypeDeclBody, Visibility};
    use verum_common::List;

    fn create_test_struct() -> TypeDecl {
        let span = Span::default();
        TypeDecl {
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
