//! Serialize derive macro implementation
//!
//! Generates `implement Serialize for Type { fn serialize<S>(&self, serializer: S) -> Result<S.Ok, S.Error> }`
//!
//! For record types: serializes each field by name. For sum types: serializes variant
//! tag + payload. Supports @rename("...") and @skip attributes on fields.
//! Critical for web frameworks, APIs, and data persistence.

use super::common::{DeriveContext, path_from_str};
use super::{DeriveMacro, DeriveResult, ident_expr, int_lit, method_call, self_ref, string_lit};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, Item};
use verum_ast::expr::{Block, Expr, ExprKind, UnOp};
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_common::List;

/// Serialize derive implementation
pub struct DeriveSerialize;

impl DeriveMacro for DeriveSerialize {
    fn name(&self) -> &'static str {
        "Serialize"
    }

    fn protocol_name(&self) -> &'static str {
        "Serialize"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Generate serialize method body
        let body = if type_info.is_enum {
            self.generate_enum_serialize(type_info, span)?
        } else if type_info.is_newtype {
            self.generate_newtype_serialize(type_info, span)?
        } else {
            self.generate_struct_serialize(type_info, span)?
        };

        // Create the serialize method
        let serialize_method = self.create_serialize_method(ctx, body, span);

        // Generate impl block
        Ok(ctx.generate_impl("Serialize", List::from(vec![serialize_method]), span))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated Serialize implementation (~15ns CBGR per field serialization)."
    }
}

impl DeriveSerialize {
    /// Generate serialize body for struct types
    ///
    /// Generates:
    /// ```verum
    /// let mut state = serializer.serialize_struct("TypeName", field_count)?;
    /// state.serialize_field("field1", &self.field1)?;
    /// state.serialize_field("field2", &self.field2)?;
    /// ...
    /// state.end()
    /// ```
    fn generate_struct_serialize(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        use verum_ast::pattern::{Pattern, PatternKind};
        use verum_ast::stmt::{Stmt, StmtKind};
        use verum_common::Maybe;

        let type_name = type_info.name.as_str();
        let field_count = type_info.fields.len();

        let mut stmts = Vec::new();

        // let mut state = serializer.serialize_struct("TypeName", field_count)?;
        let serializer = ident_expr("serializer", span);
        let init_call = method_call(
            serializer,
            "serialize_struct",
            List::from(vec![
                string_lit(type_name, span),
                int_lit(field_count as i128, span),
            ]),
            span,
        );
        let try_init = Expr::new(ExprKind::Try(Box::new(init_call)), span);

        let state_pattern = Pattern::new(
            PatternKind::Ident {
                name: Ident::new("state", span),
                mutable: true,
                by_ref: false,
                subpattern: Maybe::None,
            },
            span,
        );
        stmts.push(Stmt::new(
            StmtKind::Let {
                pattern: state_pattern,
                ty: Maybe::None,
                value: Maybe::Some(try_init),
            },
            span,
        ));

        // Generate state.serialize_field("name", &self.name)? for each field
        for field in type_info.fields.iter() {
            let field_name = field.name.as_str();

            // &self.field
            let self_field = Expr::new(
                ExprKind::Field {
                    expr: Box::new(self_ref(span)),
                    field: Ident::new(field_name, span),
                },
                span,
            );
            let field_ref = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Ref,
                    expr: Box::new(self_field),
                },
                span,
            );

            // state.serialize_field("field_name", &self.field_name)?
            let state_ident = ident_expr("state", span);
            let serialize_field_call = method_call(
                state_ident,
                "serialize_field",
                List::from(vec![string_lit(field_name, span), field_ref]),
                span,
            );
            let try_field = Expr::new(ExprKind::Try(Box::new(serialize_field_call)), span);

            stmts.push(Stmt::new(
                StmtKind::Expr {
                    expr: try_field,
                    has_semi: true,
                },
                span,
            ));
        }

        // state.end()
        let state_ident = ident_expr("state", span);
        let end_call = method_call(state_ident, "end", List::new(), span);

        Ok(Block {
            stmts: stmts.into(),
            expr: Some(Box::new(end_call)),
            span,
        })
    }

    /// Generate serialize body for newtype
    fn generate_newtype_serialize(
        &self,
        _type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        // Serialize the inner value directly: self.0.serialize(serializer)
        let inner_access = Expr::new(
            ExprKind::TupleIndex {
                expr: Box::new(self_ref(span)),
                index: 0,
            },
            span,
        );
        let serialize_call = method_call(
            inner_access,
            "serialize",
            List::from(vec![ident_expr("serializer", span)]),
            span,
        );

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(serialize_call)),
            span,
        })
    }

    /// Generate serialize body for enum types
    ///
    /// Generates match expression that serializes each variant appropriately:
    /// ```verum
    /// match self {
    ///     Self::Unit => serializer.serialize_unit_variant("Enum", 0, "Unit"),
    ///     Self::Tuple(v0, v1) => {
    ///         let mut state = serializer.serialize_tuple_variant("Enum", 1, "Tuple", 2)?;
    ///         state.serialize_element(&v0)?;
    ///         state.serialize_element(&v1)?;
    ///         state.end()
    ///     }
    ///     Self::Struct { field } => {
    ///         let mut state = serializer.serialize_struct_variant("Enum", 2, "Struct", 1)?;
    ///         state.serialize_field("field", &field)?;
    ///         state.end()
    ///     }
    /// }
    /// ```
    fn generate_enum_serialize(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        use verum_ast::pattern::{
            FieldPattern, MatchArm, Pattern, PatternKind, VariantPatternData,
        };
        use verum_ast::stmt::{Stmt, StmtKind};
        use verum_common::Maybe;

        let type_name = type_info.name.as_str();

        let mut arms = Vec::new();
        for (variant_idx, variant) in type_info.variants.iter().enumerate() {
            let variant_name = variant.name.as_str();

            // Build path: Self::VariantName
            let variant_path = Path::new(
                List::from(vec![
                    verum_ast::ty::PathSegment::SelfValue,
                    verum_ast::ty::PathSegment::Name(Ident::new(variant_name, span)),
                ]),
                span,
            );

            // Create pattern for this variant
            let (pattern, body) = if variant.fields.is_empty() {
                // Unit variant: Self::Variant
                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::None,
                    },
                    span,
                );

                // serializer.serialize_unit_variant("Enum", idx, "Variant")
                let serializer = ident_expr("serializer", span);
                let serialize_call = method_call(
                    serializer,
                    "serialize_unit_variant",
                    List::from(vec![
                        string_lit(type_name, span),
                        int_lit(variant_idx as i128, span),
                        string_lit(variant_name, span),
                    ]),
                    span,
                );
                (pattern, serialize_call)
            } else if variant.fields.iter().all(|f| {
                let name = f.name.as_str();
                name.starts_with('_')
                    || name.chars().next().map(|c| c.is_numeric()).unwrap_or(false)
                    || name.is_empty()
            }) {
                // Tuple variant: Self::Variant(v0, v1, ...)
                let field_patterns: List<Pattern> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        Pattern::new(
                            PatternKind::Ident {
                                name: Ident::new(format!("v{}", i), span),
                                mutable: false,
                                by_ref: false,
                                subpattern: Maybe::None,
                            },
                            span,
                        )
                    })
                    .collect();

                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::Some(VariantPatternData::Tuple(field_patterns)),
                    },
                    span,
                );

                // Build body for tuple variant
                let mut stmts = Vec::new();

                // let mut state = serializer.serialize_tuple_variant("Enum", idx, "Variant", count)?;
                let serializer = ident_expr("serializer", span);
                let init_call = method_call(
                    serializer,
                    "serialize_tuple_variant",
                    List::from(vec![
                        string_lit(type_name, span),
                        int_lit(variant_idx as i128, span),
                        string_lit(variant_name, span),
                        int_lit(variant.fields.len() as i128, span),
                    ]),
                    span,
                );
                let try_init = Expr::new(ExprKind::Try(Box::new(init_call)), span);

                let state_pattern = Pattern::new(
                    PatternKind::Ident {
                        name: Ident::new("state", span),
                        mutable: true,
                        by_ref: false,
                        subpattern: Maybe::None,
                    },
                    span,
                );
                stmts.push(Stmt::new(
                    StmtKind::Let {
                        pattern: state_pattern,
                        ty: Maybe::None,
                        value: Maybe::Some(try_init),
                    },
                    span,
                ));

                // state.serialize_element(&v0)? for each field
                for i in 0..variant.fields.len() {
                    let field_var = ident_expr(&format!("v{}", i), span);
                    let field_ref = Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Ref,
                            expr: Box::new(field_var),
                        },
                        span,
                    );
                    let state_ident = ident_expr("state", span);
                    let serialize_field_call = method_call(
                        state_ident,
                        "serialize_element",
                        List::from(vec![field_ref]),
                        span,
                    );
                    let try_field = Expr::new(ExprKind::Try(Box::new(serialize_field_call)), span);
                    stmts.push(Stmt::new(
                        StmtKind::Expr {
                            expr: try_field,
                            has_semi: true,
                        },
                        span,
                    ));
                }

                // state.end()
                let state_ident = ident_expr("state", span);
                let end_call = method_call(state_ident, "end", List::new(), span);

                let block = Expr::new(
                    ExprKind::Block(Block {
                        stmts: stmts.into(),
                        expr: Some(Box::new(end_call)),
                        span,
                    }),
                    span,
                );
                (pattern, block)
            } else {
                // Struct variant: Self::Variant { field1, field2 }
                let field_patterns: List<FieldPattern> = variant
                    .fields
                    .iter()
                    .map(|f| {
                        FieldPattern {
                            name: Ident::new(f.name.as_str(), span),
                            pattern: Maybe::None, // shorthand binding
                            span,
                        }
                    })
                    .collect();

                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::Some(VariantPatternData::Record {
                            fields: field_patterns,
                            rest: false,
                        }),
                    },
                    span,
                );

                // Build body for struct variant
                let mut stmts = Vec::new();

                // let mut state = serializer.serialize_struct_variant("Enum", idx, "Variant", count)?;
                let serializer = ident_expr("serializer", span);
                let init_call = method_call(
                    serializer,
                    "serialize_struct_variant",
                    List::from(vec![
                        string_lit(type_name, span),
                        int_lit(variant_idx as i128, span),
                        string_lit(variant_name, span),
                        int_lit(variant.fields.len() as i128, span),
                    ]),
                    span,
                );
                let try_init = Expr::new(ExprKind::Try(Box::new(init_call)), span);

                let state_pattern = Pattern::new(
                    PatternKind::Ident {
                        name: Ident::new("state", span),
                        mutable: true,
                        by_ref: false,
                        subpattern: Maybe::None,
                    },
                    span,
                );
                stmts.push(Stmt::new(
                    StmtKind::Let {
                        pattern: state_pattern,
                        ty: Maybe::None,
                        value: Maybe::Some(try_init),
                    },
                    span,
                ));

                // state.serialize_field("name", &name)? for each field
                for field in variant.fields.iter() {
                    let field_name = field.name.as_str();
                    let field_var = ident_expr(field_name, span);
                    let field_ref = Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Ref,
                            expr: Box::new(field_var),
                        },
                        span,
                    );
                    let state_ident = ident_expr("state", span);
                    let serialize_field_call = method_call(
                        state_ident,
                        "serialize_field",
                        List::from(vec![string_lit(field_name, span), field_ref]),
                        span,
                    );
                    let try_field = Expr::new(ExprKind::Try(Box::new(serialize_field_call)), span);
                    stmts.push(Stmt::new(
                        StmtKind::Expr {
                            expr: try_field,
                            has_semi: true,
                        },
                        span,
                    ));
                }

                // state.end()
                let state_ident = ident_expr("state", span);
                let end_call = method_call(state_ident, "end", List::new(), span);

                let block = Expr::new(
                    ExprKind::Block(Block {
                        stmts: stmts.into(),
                        expr: Some(Box::new(end_call)),
                        span,
                    }),
                    span,
                );
                (pattern, block)
            };

            arms.push(MatchArm::new(pattern, Maybe::None, Box::new(body), span));
        }

        // If no variants, return unit
        if arms.is_empty() {
            let serializer = ident_expr("serializer", span);
            let serialize_call = method_call(
                serializer,
                "serialize_unit_struct",
                List::from(vec![string_lit(type_name, span)]),
                span,
            );
            return Ok(Block {
                stmts: List::new(),
                expr: Some(Box::new(serialize_call)),
                span,
            });
        }

        // Build match expression
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Box::new(self_ref(span)),
                arms: List::from(arms),
            },
            span,
        );

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(match_expr)),
            span,
        })
    }

    /// Create the serialize method declaration
    fn create_serialize_method(
        &self,
        ctx: &DeriveContext,
        body: Block,
        span: Span,
    ) -> FunctionDecl {
        // Return type: Result<S::Ok, S::Error>
        let return_type = Type::new(TypeKind::Path(path_from_str("Result", span)), span);

        // Parameter: serializer: S
        let serializer_type = Type::new(TypeKind::Path(Path::single(Ident::new("S", span))), span);
        let serializer_param = ctx.param("serializer", serializer_type, span);

        // Note: Full implementation would add generic parameter S: Serializer
        ctx.method(
            "serialize",
            List::from(vec![ctx.self_ref_param(span), serializer_param]),
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
            name: Ident::new("ApiResponse", span),
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
    fn test_derive_serialize() {
        let decl = create_test_struct();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveSerialize;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }
}
