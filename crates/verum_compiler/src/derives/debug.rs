//! Debug derive macro implementation
//!
//! Generates `implement Debug for Type { fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> }`
//!
//! For record types: generates "TypeName { field1: {val1}, field2: {val2} }" format.
//! For sum types: matches each variant and formats with variant name and payload.
//! Output is for development/testing use, not end-user display (use Display for that).

use super::common::{DeriveContext, path_from_str};
use super::{DeriveMacro, DeriveResult, ident_expr, method_call, string_lit};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, Item};
use verum_ast::expr::{Block, Expr, ExprKind, UnOp};
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_common::List;

/// Debug derive implementation
pub struct DeriveDebug;

impl DeriveMacro for DeriveDebug {
    fn name(&self) -> &'static str {
        "Debug"
    }

    fn protocol_name(&self) -> &'static str {
        "Debug"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Generate fmt method body based on type kind
        let body = if type_info.is_enum {
            self.generate_enum_fmt(type_info, span)?
        } else if type_info.is_newtype {
            self.generate_newtype_fmt(type_info, span)?
        } else {
            self.generate_struct_fmt(type_info, span)?
        };

        // Create the fmt method
        let fmt_method = self.create_fmt_method(ctx, body, span);

        // Generate impl block
        Ok(ctx.generate_impl("Debug", List::from(vec![fmt_method]), span))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated Debug implementation with CBGR-aware formatting (~15ns overhead per reference check)."
    }
}

impl DeriveDebug {
    /// Generate fmt body for struct types
    fn generate_struct_fmt(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        let type_name = type_info.name.as_str();

        // Start building: f.debug_struct("TypeName")
        let f_expr = ident_expr("f", span);
        let debug_struct_call = method_call(
            f_expr,
            "debug_struct",
            List::from(vec![string_lit(type_name, span)]),
            span,
        );

        // Chain .field("name", &self.name) for each field
        let mut chain = debug_struct_call;
        for field in type_info.fields.iter() {
            // Create &self.field_name
            let field_ref = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Ref,
                    expr: Box::new(field.access_on_self(span)),
                },
                span,
            );

            // .field("field_name", &self.field_name)
            chain = method_call(
                chain,
                "field",
                List::from(vec![string_lit(field.name.as_str(), span), field_ref]),
                span,
            );
        }

        // Add .finish()
        let finish_call = method_call(chain, "finish", List::new(), span);

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(finish_call)),
            span,
        })
    }

    /// Generate fmt body for newtype
    fn generate_newtype_fmt(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        let type_name = type_info.name.as_str();

        // f.debug_tuple("TypeName").finish()
        let f_expr = ident_expr("f", span);
        let debug_tuple_call = method_call(
            f_expr,
            "debug_tuple",
            List::from(vec![string_lit(type_name, span)]),
            span,
        );
        let finish_call = method_call(debug_tuple_call, "finish", List::new(), span);

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(finish_call)),
            span,
        })
    }

    /// Generate fmt body for enum types
    ///
    /// Generates match expression:
    /// ```verum
    /// match self {
    ///     Self::Unit => f.debug_struct("Unit").finish(),
    ///     Self::Tuple(v0, v1) => f.debug_tuple("Tuple").field(&v0).field(&v1).finish(),
    ///     Self::Struct { field } => f.debug_struct("Struct").field("field", &field).finish(),
    /// }
    /// ```
    fn generate_enum_fmt(
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
                // Unit variant: Self::Variant => f.debug_struct("Variant").finish()
                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::None,
                    },
                    span,
                );

                let f_expr = ident_expr("f", span);
                let debug_struct_call = method_call(
                    f_expr,
                    "debug_struct",
                    List::from(vec![string_lit(variant_name, span)]),
                    span,
                );
                let finish_call = method_call(debug_struct_call, "finish", List::new(), span);
                (pattern, finish_call)
            } else if variant.fields.iter().all(|f| {
                let name = f.name.as_str();
                name.starts_with('_')
                    || name.chars().next().map(|c| c.is_numeric()).unwrap_or(false)
                    || name.is_empty()
            }) {
                // Tuple variant: Self::Variant(v0, v1) => f.debug_tuple("Variant").field(&v0).field(&v1).finish()
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

                // f.debug_tuple("Variant")
                let f_expr = ident_expr("f", span);
                let mut chain = method_call(
                    f_expr,
                    "debug_tuple",
                    List::from(vec![string_lit(variant_name, span)]),
                    span,
                );

                // Chain .field(&v0), .field(&v1), ...
                for i in 0..variant.fields.len() {
                    let field_var = ident_expr(&format!("v{}", i), span);
                    let field_ref = Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Ref,
                            expr: Box::new(field_var),
                        },
                        span,
                    );
                    chain = method_call(chain, "field", List::from(vec![field_ref]), span);
                }

                let finish_call = method_call(chain, "finish", List::new(), span);
                (pattern, finish_call)
            } else {
                // Struct variant: Self::Variant { field } => f.debug_struct("Variant").field("field", &field).finish()
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

                // f.debug_struct("Variant")
                let f_expr = ident_expr("f", span);
                let mut chain = method_call(
                    f_expr,
                    "debug_struct",
                    List::from(vec![string_lit(variant_name, span)]),
                    span,
                );

                // Chain .field("name", &name) for each field
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
                    chain = method_call(
                        chain,
                        "field",
                        List::from(vec![string_lit(field_name, span), field_ref]),
                        span,
                    );
                }

                let finish_call = method_call(chain, "finish", List::new(), span);
                (pattern, finish_call)
            };

            arms.push(MatchArm::new(pattern, Maybe::None, Heap::new(body), span));
        }

        // If no variants, just return Ok(())
        if arms.is_empty() {
            let f_expr = ident_expr("f", span);
            let write_call = method_call(
                f_expr,
                "write_str",
                List::from(vec![string_lit(type_info.name.as_str(), span)]),
                span,
            );
            return Ok(Block {
                stmts: List::new(),
                expr: Some(Box::new(write_call)),
                span,
            });
        }

        // Build match expression
        let self_expr = Expr::new(ExprKind::Path(Path::single(Ident::new("self", span))), span);
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Heap::new(self_expr),
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

    /// Create the fmt method declaration
    fn create_fmt_method(&self, ctx: &DeriveContext, body: Block, span: Span) -> FunctionDecl {
        // Return type: Result<(), FormatError>
        let return_type = Type::new(TypeKind::Path(path_from_str("Result", span)), span);

        // Parameter: f: &mut Formatter
        let formatter_type = Type::new(
            TypeKind::Reference {
                mutable: true,
                inner: Box::new(Type::new(
                    TypeKind::Path(Path::single(Ident::new("Formatter", span))),
                    span,
                )),
            },
            span,
        );
        let f_param = ctx.param("f", formatter_type, span);

        ctx.method(
            "fmt",
            List::from(vec![ctx.self_ref_param(span), f_param]),
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
    fn test_derive_debug_unit_struct() {
        let decl = create_test_struct();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveDebug;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }
}
