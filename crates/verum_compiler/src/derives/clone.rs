//! Clone derive macro implementation
//!
//! Generates `implement Clone for Type { fn clone(&self) -> Self }`
//!
//! For record types: clones each field recursively via field.clone().
//! For sum types: matches each variant and clones payload fields.
//! Generated code has identical performance to hand-written Clone implementations.
//! Works transparently with both ThinRef (16 bytes) and FatRef (24 bytes).

use super::common::DeriveContext;
use super::{DeriveMacro, DeriveResult, ident_expr, method_call, self_ref};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, Item};
use verum_ast::expr::{Block, Expr, ExprKind, FieldInit};
use verum_ast::pattern::{FieldPattern, MatchArm, Pattern, PatternKind, VariantPatternData};
use verum_ast::ty::{Ident, Path, PathSegment, Type, TypeKind};
use verum_common::{List, Text};

/// Clone derive implementation
pub struct DeriveClone;

impl DeriveMacro for DeriveClone {
    fn name(&self) -> &'static str {
        "Clone"
    }

    fn protocol_name(&self) -> &'static str {
        "Clone"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Generate clone method body based on type kind
        let body = if type_info.is_enum {
            self.generate_enum_clone(type_info, span)?
        } else if type_info.is_newtype {
            self.generate_newtype_clone(type_info, span)?
        } else {
            self.generate_struct_clone(type_info, span)?
        };

        // Create the clone method
        let clone_method = self.create_clone_method(ctx, body, span);

        // Generate impl block
        Ok(ctx.generate_impl("Clone", List::from(vec![clone_method]), span))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated Clone implementation with CBGR-aware deep copy (~15ns per field)."
    }
}

impl DeriveClone {
    /// Generate clone body for struct types
    fn generate_struct_clone(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        let type_name = type_info.name.as_str();

        // Build struct fields: field: self.field.clone()
        let fields: List<FieldInit> = type_info
            .fields
            .iter()
            .map(|field| {
                let clone_call =
                    method_call(field.access_on_self(span), "clone", List::new(), span);
                FieldInit {
                    attributes: List::new(),
                    name: field.ident(span),
                    value: Some(clone_call),
                    span,
                }
            })
            .collect();

        // TypeName { field1: ..., field2: ... }
        let struct_lit = Expr::new(
            ExprKind::Record {
                path: Path::single(Ident::new(type_name, span)),
                fields,
                base: None,
            },
            span,
        );

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(struct_lit)),
            span,
        })
    }

    /// Generate clone body for newtype
    fn generate_newtype_clone(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        let type_name = type_info.name.as_str();

        // Clone the inner value: TypeName(self.0.clone())
        let inner_access = Expr::new(
            ExprKind::TupleIndex {
                expr: Box::new(self_ref(span)),
                index: 0,
            },
            span,
        );
        let clone_call = method_call(inner_access, "clone", List::new(), span);
        let call_expr = Expr::new(
            ExprKind::Call {
                func: Box::new(ident_expr(type_name, span)),
                type_args: List::new(),
                args: List::from(vec![clone_call]),
            },
            span,
        );

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(call_expr)),
            span,
        })
    }

    /// Generate clone body for enum types
    ///
    /// Generates a match expression that handles each variant:
    /// ```verum
    /// match self {
    ///     Self::Variant1 => Self::Variant1,
    ///     Self::Variant2(a) => Self::Variant2(a.clone()),
    ///     Self::Variant3 { x, y } => Self::Variant3 { x: x.clone(), y: y.clone() },
    /// }
    /// ```
    fn generate_enum_clone(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        // Handle empty enums (no variants)
        if type_info.variants.is_empty() {
            // For empty enums, produce an unreachable case
            let match_expr = Expr::new(
                ExprKind::Match {
                    expr: Box::new(self_ref(span)),
                    arms: List::new(),
                },
                span,
            );
            return Ok(Block {
                stmts: List::new(),
                expr: Some(Box::new(match_expr)),
                span,
            });
        }

        // Build match arms for each variant
        let mut arms: Vec<MatchArm> = Vec::new();

        for variant in type_info.variants.iter() {
            // Build the pattern for this variant
            let variant_path = Path::new(
                List::from(vec![
                    PathSegment::Name(Ident::new("Self", span)),
                    PathSegment::Name(variant.ident(span)),
                ]),
                span,
            );

            let (pattern, body_expr) = if variant.is_unit {
                // Unit variant: Self::Variant => Self::Variant
                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path.clone(),
                        data: None,
                    },
                    span,
                );
                let body = Expr::new(ExprKind::Path(variant_path), span);
                (pattern, body)
            } else if variant.is_tuple {
                // Tuple variant: Self::Variant(a, b) => Self::Variant(a.clone(), b.clone())
                let binding_names: Vec<Text> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, _)| Text::from(format!("_field{}", i)))
                    .collect();

                let inner_patterns: List<Pattern> = binding_names
                    .iter()
                    .map(|name| {
                        Pattern::new(
                            PatternKind::Ident {
                                by_ref: false,
                                name: Ident::new(name.as_str(), span),
                                mutable: false,
                                subpattern: None,
                            },
                            span,
                        )
                    })
                    .collect();

                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path.clone(),
                        data: Some(VariantPatternData::Tuple(inner_patterns)),
                    },
                    span,
                );

                // Build clone calls for each field
                let cloned_args: Vec<Expr> = binding_names
                    .iter()
                    .map(|name| {
                        let field_ref = ident_expr(name.as_str(), span);
                        method_call(field_ref, "clone", List::new(), span)
                    })
                    .collect();

                // Build constructor call: Self::Variant(arg1.clone(), arg2.clone(), ...)
                let constructor = Expr::new(ExprKind::Path(variant_path), span);
                let body = Expr::new(
                    ExprKind::Call {
                        func: Box::new(constructor),
                        type_args: List::new(),
                        args: cloned_args.into(),
                    },
                    span,
                );

                (pattern, body)
            } else {
                // Record variant: Self::Variant { x, y } => Self::Variant { x: x.clone(), y: y.clone() }
                let field_patterns: List<FieldPattern> = variant
                    .fields
                    .iter()
                    .map(|field| FieldPattern {
                        name: field.ident(span),
                        pattern: Some(Pattern::new(
                            PatternKind::Ident {
                                by_ref: false,
                                name: field.ident(span),
                                mutable: false,
                                subpattern: None,
                            },
                            span,
                        )),
                        span,
                    })
                    .collect();

                let pattern = Pattern::new(
                    PatternKind::Record {
                        path: variant_path.clone(),
                        fields: field_patterns,
                        rest: false,
                    },
                    span,
                );

                // Build cloned field initializers
                let cloned_fields: List<FieldInit> = variant
                    .fields
                    .iter()
                    .map(|field| {
                        let field_ref = ident_expr(field.name.as_str(), span);
                        let cloned = method_call(field_ref, "clone", List::new(), span);
                        FieldInit {
                            attributes: List::new(),
                            name: field.ident(span),
                            value: Some(cloned),
                            span,
                        }
                    })
                    .collect();

                let body = Expr::new(
                    ExprKind::Record {
                        path: variant_path,
                        fields: cloned_fields,
                        base: None,
                    },
                    span,
                );

                (pattern, body)
            };

            arms.push(MatchArm::new(
                pattern,
                None,
                Box::new(body_expr),
                span,
            ));
        }

        // Build the match expression: match self { ... }
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

    /// Create the clone method declaration
    fn create_clone_method(&self, ctx: &DeriveContext, body: Block, span: Span) -> FunctionDecl {
        // Return type: Self
        let return_type = Type::new(TypeKind::Path(Path::single(Ident::new("Self", span))), span);

        ctx.method(
            "clone",
            List::from(vec![ctx.self_ref_param(span)]),
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
    fn test_derive_clone_unit_struct() {
        let decl = create_test_struct();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveClone;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }
}
