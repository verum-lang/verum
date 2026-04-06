//! Display derive macro implementation
//!
//! Generates `implement Display for Type { fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> }`
//!
//! The Display implementation generates human-readable output suitable for end-users.
//! For error types, consider using @derive(Error) which auto-generates Display.
//!
//! # Attribute Support
//!
//! - `@display("custom format {field}")` - Custom display message for variants
//! - Default behavior: shows variant/type name with field values
//!
//! # Example
//!
//! ```verum
//! @derive(Display)
//! type Status is
//!     | @display("Pending approval")
//!       Pending
//!     | @display("Completed at {timestamp}")
//!       Completed { timestamp: DateTime }
//!     | @display("Error: {reason}")
//!       Failed { reason: Text };
//! ```
//!
//! Generates human-readable text representation. For record types: shows type name
//! and field values. For sum types: matches each variant with optional @display("...")
//! attribute for custom formatting. Generated code is fully inspectable via
//! --show-expansions compiler flag.

use super::common::{DeriveContext, FieldInfo, path_from_str};
use super::{DeriveMacro, DeriveResult, ident_expr, method_call, string_lit, self_ref};
use verum_ast::attr::{Attribute, AttributeListExt};
use verum_ast::decl::{FunctionDecl, Item};
use verum_ast::expr::{Block, Expr, ExprKind, UnOp};
use verum_ast::pattern::{FieldPattern, MatchArm, Pattern, PatternKind, VariantPatternData};
use verum_ast::ty::{Ident, Path, PathSegment, Type, TypeKind};
use verum_ast::Span;
use verum_common::{List, Text};

/// Display derive implementation
pub struct DeriveDisplay;

impl DeriveMacro for DeriveDisplay {
    fn name(&self) -> &'static str {
        "Display"
    }

    fn protocol_name(&self) -> &'static str {
        "Display"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Generate fmt method body based on type kind
        let body = if type_info.is_enum {
            self.generate_enum_fmt(ctx, span)?
        } else if type_info.is_newtype {
            self.generate_newtype_fmt(type_info, span)?
        } else {
            self.generate_struct_fmt(type_info, span)?
        };

        // Create the fmt method
        let fmt_method = self.create_fmt_method(ctx, body, span);

        // Generate impl block
        Ok(ctx.generate_impl("Display", vec![fmt_method].into(), span))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated Display implementation for human-readable output."
    }
}

impl DeriveDisplay {
    /// Extract custom display format from @display attribute on a variant
    fn get_variant_display_format(attrs: &List<Attribute>) -> Option<Text> {
        // Look for @display attribute
        if let Some(attr) = attrs.find_by_name("display") {
            if let Some(ref args) = attr.args {
                if let Some(first_arg) = args.first() {
                    // Extract string literal from the expression
                    if let ExprKind::Literal(lit) = &first_arg.kind {
                        if let verum_ast::LiteralKind::Text(string_lit) = &lit.kind {
                            return Some(Text::from(string_lit.as_str()));
                        }
                    }
                }
            }
        }
        None
    }

    /// Generate fmt body for struct types
    ///
    /// Generates: write!(f, "TypeName {{ field: {}, ... }}", self.field, ...)
    fn generate_struct_fmt(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        let type_name = type_info.name.as_str();
        let f_expr = ident_expr("f", span);

        if type_info.fields.is_empty() {
            // Empty struct: just write the name
            let write_call = method_call(
                f_expr,
                "write_str",
                vec![string_lit(type_name, span)].into(),
                span,
            );
            return Ok(Block {
                stmts: List::new(),
                expr: Some(Box::new(write_call)),
                span,
            });
        }

        // Create write! equivalent: f.write_fmt(format_args!(...))
        // For simplicity, we'll use a chain of write_str and Display::fmt calls
        let write_str_call = method_call(
            f_expr.clone(),
            "write_str",
            vec![string_lit(&format!("{} {{ ", type_name), span)].into(),
            span,
        );

        // Build chain of field writes
        let mut stmts = vec![verum_ast::Stmt::new(
            verum_ast::StmtKind::Expr {
                expr: write_str_call,
                has_semi: true,
            },
            span,
        )];

        for (i, field) in type_info.fields.iter().enumerate() {
            if i > 0 {
                // Write ", "
                let comma_write = method_call(
                    f_expr.clone(),
                    "write_str",
                    vec![string_lit(", ", span)].into(),
                    span,
                );
                stmts.push(verum_ast::Stmt::new(
                    verum_ast::StmtKind::Expr {
                        expr: comma_write,
                        has_semi: true,
                    },
                    span,
                ));
            }

            // Write "field_name: "
            let field_name_write = method_call(
                f_expr.clone(),
                "write_str",
                vec![string_lit(&format!("{}: ", field.name.as_str()), span)].into(),
                span,
            );
            stmts.push(verum_ast::Stmt::new(
                verum_ast::StmtKind::Expr {
                    expr: field_name_write,
                    has_semi: true,
                },
                span,
            ));

            // Write field value using Display::fmt
            let field_ref = Expr::new(
                ExprKind::Unary {
                    op: UnOp::Ref,
                    expr: Box::new(field.access_on_self(span)),
                },
                span,
            );
            let display_fmt = method_call(
                field_ref,
                "fmt",
                vec![f_expr.clone()].into(),
                span,
            );
            // Use ? operator for propagation
            let try_expr = Expr::new(
                ExprKind::Try(Box::new(display_fmt)),
                span,
            );
            stmts.push(verum_ast::Stmt::new(
                verum_ast::StmtKind::Expr {
                    expr: try_expr,
                    has_semi: true,
                },
                span,
            ));
        }

        // Final write " }"
        let close_write = method_call(
            f_expr,
            "write_str",
            vec![string_lit(" }", span)].into(),
            span,
        );

        Ok(Block {
            stmts: stmts.into(),
            expr: Some(Box::new(close_write)),
            span,
        })
    }

    /// Generate fmt body for newtype
    ///
    /// Just delegates to the inner type's Display implementation
    fn generate_newtype_fmt(
        &self,
        _type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        // Newtype: delegate to inner value
        // self.0.fmt(f)
        let self_expr = self_ref(span);
        let inner_access = Expr::new(
            ExprKind::Index {
                expr: Box::new(self_expr),
                index: Box::new(Expr::new(
                    ExprKind::Literal(verum_ast::Literal {
                        kind: verum_ast::LiteralKind::Int(verum_ast::literal::IntLit {
                            value: 0,
                            suffix: None,
                        }),
                        span,
                    }),
                    span,
                )),
            },
            span,
        );

        let inner_ref = Expr::new(
            ExprKind::Unary {
                op: UnOp::Ref,
                expr: Box::new(inner_access),
            },
            span,
        );

        let f_expr = ident_expr("f", span);
        let fmt_call = method_call(inner_ref, "fmt", vec![f_expr].into(), span);

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(fmt_call)),
            span,
        })
    }

    /// Generate fmt body for enum types
    ///
    /// Generates match expression with custom @display formats if present
    fn generate_enum_fmt(&self, ctx: &DeriveContext, span: Span) -> DeriveResult<Block> {
        let type_info = &ctx.type_info;

        if type_info.variants.is_empty() {
            // Empty enum - just return Ok(())
            let ok_expr = Expr::new(
                ExprKind::Call {
                    func: Box::new(ident_expr("Ok", span)),
                    type_args: List::new(),
                    args: vec![Expr::new(ExprKind::Tuple(List::new()), span)].into(),
                },
                span,
            );
            return Ok(Block {
                stmts: List::new(),
                expr: Some(Box::new(ok_expr)),
                span,
            });
        }

        let mut arms: Vec<MatchArm> = Vec::new();

        for variant in type_info.variants.iter() {
            let variant_name = variant.name.as_str();

            // Get variant attributes from the original type declaration
            let variant_attrs = self.get_variant_attributes(ctx, variant_name);

            // Build path: Self::VariantName
            let variant_path = Path::new(
                vec![
                    PathSegment::SelfValue,
                    PathSegment::Name(Ident::new(Text::from(variant_name), span)),
                ].into(),
                span,
            );

            // Check for custom @display format
            let custom_format = Self::get_variant_display_format(&variant_attrs);

            let (pattern, body) = if variant.is_unit {
                // Unit variant
                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: None,
                    },
                    span,
                );

                let message = match custom_format {
                    Some(msg) => msg,
                    None => self.humanize_variant_name(variant_name),
                };

                let f_expr = ident_expr("f", span);
                let write_call = method_call(
                    f_expr,
                    "write_str",
                    vec![string_lit(message.as_str(), span)].into(),
                    span,
                );

                (pattern, write_call)
            } else if variant.is_tuple {
                // Tuple variant
                let field_patterns: List<Pattern> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        Pattern::new(
                            PatternKind::Ident {
                                name: Ident::new(Text::from(format!("v{}", i)), span),
                                mutable: false,
                                by_ref: false,
                                subpattern: None,
                            },
                            span,
                        )
                    })
                    .collect();

                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Some(VariantPatternData::Tuple(field_patterns)),
                    },
                    span,
                );

                let message = match custom_format {
                    Some(msg) => msg,
                    None => self.humanize_variant_name(variant_name),
                };

                // For tuple variants, just show the message (custom format could have {0}, {1}, etc.)
                let f_expr = ident_expr("f", span);
                let write_call = method_call(
                    f_expr,
                    "write_str",
                    vec![string_lit(message.as_str(), span)].into(),
                    span,
                );

                (pattern, write_call)
            } else {
                // Record variant
                let field_patterns: List<FieldPattern> = variant
                    .fields
                    .iter()
                    .map(|f| FieldPattern {
                        name: Ident::new(Text::from(f.name.as_str()), span),
                        pattern: None, // shorthand binding
                        span,
                    })
                    .collect();

                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Some(VariantPatternData::Record {
                            fields: field_patterns,
                            rest: false,
                        }),
                    },
                    span,
                );

                let message = match custom_format {
                    Some(msg) => msg,
                    None => self.humanize_variant_name(variant_name),
                };

                // Check if message has interpolation placeholders
                let has_interpolation = message.as_str().contains('{');

                let body = if has_interpolation {
                    // Generate format string with field substitution
                    self.generate_format_string_body(&message, &variant.fields, span)
                } else {
                    let f_expr = ident_expr("f", span);
                    method_call(
                        f_expr,
                        "write_str",
                        vec![string_lit(message.as_str(), span)].into(),
                        span,
                    )
                };

                (pattern, body)
            };

            arms.push(MatchArm::new(pattern, None, Box::new(body), span));
        }

        // Build match expression
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Box::new(self_ref(span)),
                arms: arms.into(),
            },
            span,
        );

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(match_expr)),
            span,
        })
    }

    /// Get attributes for a specific variant from the type declaration
    fn get_variant_attributes(&self, ctx: &DeriveContext, variant_name: &str) -> List<Attribute> {
        if let verum_ast::decl::TypeDeclBody::Variant(variants) = &ctx.type_decl.body {
            for variant in variants.iter() {
                if variant.name.as_str() == variant_name {
                    return variant.attributes.iter().cloned().collect();
                }
            }
        }
        List::new()
    }

    /// Convert CamelCase variant name to human-readable form
    fn humanize_variant_name(&self, name: &str) -> Text {
        let mut result = String::new();
        for (i, ch) in name.chars().enumerate() {
            if ch.is_uppercase() && i > 0 {
                result.push(' ');
                result.push(ch.to_lowercase().next().unwrap());
            } else if i == 0 {
                result.push(ch);
            } else {
                result.push(ch);
            }
        }
        result.into()
    }

    /// Generate a format string body for interpolated display messages
    ///
    /// For templates like "Error: {reason}", generates code that writes the interpolated string
    fn generate_format_string_body(
        &self,
        template: &Text,
        fields: &List<FieldInfo>,
        span: Span,
    ) -> Expr {
        // Parse template to extract parts
        let template_str = template.as_str();
        let mut parts: Vec<(String, Option<String>)> = Vec::new();
        let mut current_text = String::new();
        let mut in_placeholder = false;
        let mut placeholder = String::new();

        for ch in template_str.chars() {
            match ch {
                '{' if !in_placeholder => {
                    if !current_text.is_empty() {
                        parts.push((current_text.clone(), None));
                        current_text.clear();
                    }
                    in_placeholder = true;
                    placeholder.clear();
                }
                '}' if in_placeholder => {
                    parts.push((String::new(), Some(placeholder.clone())));
                    in_placeholder = false;
                }
                _ if in_placeholder => {
                    placeholder.push(ch);
                }
                _ => {
                    current_text.push(ch);
                }
            }
        }
        if !current_text.is_empty() {
            parts.push((current_text, None));
        }

        // Generate write calls for each part
        let f_expr = ident_expr("f", span);
        let mut stmts = Vec::new();

        for (text, field_name) in parts.iter() {
            if let Some(field_name) = field_name {
                // Check if field exists
                let _field_exists = fields.iter().any(|f| f.name.as_str() == field_name.as_str());

                // Write the field using Display
                let field_var = ident_expr(field_name, span);
                let field_ref = Expr::new(
                    ExprKind::Unary {
                        op: UnOp::Ref,
                        expr: Box::new(field_var),
                    },
                    span,
                );
                let fmt_call = method_call(field_ref, "fmt", vec![f_expr.clone()].into(), span);
                let try_expr = Expr::new(ExprKind::Try(Box::new(fmt_call)), span);
                stmts.push(verum_ast::Stmt::new(
                    verum_ast::StmtKind::Expr {
                        expr: try_expr,
                        has_semi: true,
                    },
                    span,
                ));
            } else if !text.is_empty() {
                // Write literal text
                let write_call = method_call(
                    f_expr.clone(),
                    "write_str",
                    vec![string_lit(text, span)].into(),
                    span,
                );
                let try_expr = Expr::new(ExprKind::Try(Box::new(write_call)), span);
                stmts.push(verum_ast::Stmt::new(
                    verum_ast::StmtKind::Expr {
                        expr: try_expr,
                        has_semi: true,
                    },
                    span,
                ));
            }
        }

        // Return Ok(())
        let ok_unit = Expr::new(
            ExprKind::Call {
                func: Box::new(ident_expr("Ok", span)),
                type_args: List::new(),
                args: vec![Expr::new(ExprKind::Tuple(List::new()), span)].into(),
            },
            span,
        );

        // Create block with all statements and final Ok(())
        Expr::new(
            ExprKind::Block(verum_ast::expr::Block {
                stmts: stmts.into(),
                expr: Some(Box::new(ok_unit)),
                span,
            }),
            span,
        )
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
            vec![ctx.self_ref_param(span), f_param].into(),
            return_type,
            body,
            span,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{TypeDecl, TypeDeclBody, Variant, VariantData, RecordField, Visibility};
    use verum_common::List;

    fn create_simple_enum() -> TypeDecl {
        let span = Span::default();
        TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new("Status", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Variant(List::from(vec![
                Variant::new(Ident::new("Pending", span), None, span),
                Variant::new(Ident::new("Completed", span), None, span),
            ])),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        }
    }

    fn create_enum_with_fields() -> TypeDecl {
        let span = Span::default();
        TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new("Result", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Variant(List::from(vec![
                Variant::new(
                    Ident::new("Error", span),
                    Some(VariantData::Record(List::from(vec![RecordField::new(
                        Visibility::Private,
                        Ident::new("message", span),
                        Type::text(span),
                        span,
                    )]))),
                    span,
                ),
            ])),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        }
    }

    #[test]
    fn test_derive_display_simple_enum() {
        let decl = create_simple_enum();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveDisplay;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_derive_display_enum_with_fields() {
        let decl = create_enum_with_fields();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveDisplay;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_humanize_variant_name() {
        let derive = DeriveDisplay;
        assert_eq!(derive.humanize_variant_name("NotFound").as_str(), "Not found");
        // IOError becomes "I o error" due to camelCase splitting - for acronyms,
        // use @display("I/O Error") custom format instead
        assert_eq!(derive.humanize_variant_name("IoError").as_str(), "Io error");
        assert_eq!(
            derive.humanize_variant_name("ConnectionTimeout").as_str(),
            "Connection timeout"
        );
    }
}
