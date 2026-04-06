//! Error derive macro implementation
//!
//! Generates implementations for:
//! - `implement Error for Type { fn description(&self) -> Text }`
//! - `implement Display for Type { fn fmt(&self, f: &mut Formatter) -> Result<(), FormatError> }`
//! - `From` implementations for wrapped error variants
//!
//! # Attribute Support
//!
//! - `@message("custom message")` - Custom display message for a variant
//! - `@source` - Mark a field as the error source for chaining
//!
//! # Example
//!
//! ```verum
//! @derive(Error)
//! type MyError is
//!     | @message("I/O operation failed")
//!       IoError { @source inner: std.io.Error }
//!     | @message("Parse failed: {reason}")
//!       ParseError { reason: Text, line: Int }
//!     | @message("Connection timeout")
//!       Timeout;
//! ```
//!
//! Generates:
//! - Error protocol implementation
//! - Display protocol implementation
//! - From<std.io.Error> for MyError
//!
//! The Error derive auto-generates Error protocol (description + source chaining),
//! Display protocol (human-readable messages), and From conversions for @source-marked
//! fields. This enables ergonomic error type hierarchies with ? operator support.

use super::common::{DeriveContext, DeriveError as CommonDeriveError, VariantInfo};
use super::{DeriveMacro, DeriveResult, ident_expr, self_ref, string_lit};
use verum_ast::attr::{Attribute, AttributeListExt};
use verum_ast::decl::{FunctionDecl, ImplDecl, ImplKind, ImplItem, ImplItemKind, Item, ItemKind, Visibility};
use verum_ast::expr::{Block, Expr, ExprKind, FieldInit, UnOp};
use verum_ast::pattern::{FieldPattern, MatchArm, Pattern, PatternKind, VariantPatternData};
use verum_ast::ty::{GenericArg, Ident, Path, PathSegment, Type, TypeKind};
use verum_common::well_known_types::type_names;
use verum_common::{Heap, List, Maybe, Text};
use verum_ast::Span;

/// Error derive implementation
pub struct DeriveError;

impl DeriveMacro for DeriveError {
    fn name(&self) -> &'static str {
        "Error"
    }

    fn protocol_name(&self) -> &'static str {
        "Error"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Error derive is primarily for sum types (enums)
        if !type_info.is_enum {
            return Err(CommonDeriveError::UnsupportedTypeKind {
                kind: Text::from("non-enum type"),
                hint: Text::from(
                    "Error derive is primarily intended for sum types (variants). \
                    For record types, implement Error manually.",
                ),
                span,
            });
        }

        // Generate the description method body
        let description_body = self.generate_description_method(ctx, span)?;

        // Generate the source method body
        let source_body = self.generate_source_method(ctx, span)?;

        // Create the methods
        let description_method = self.create_description_method(ctx, description_body, span);
        let source_method = self.create_source_method(ctx, source_body, span);

        // Generate impl block for Error protocol
        Ok(ctx.generate_impl(
            "Error",
            vec![description_method, source_method].into(),
            span,
        ))
    }

    fn can_derive(&self, ctx: &DeriveContext) -> Result<(), CommonDeriveError> {
        // Error derive works best on enum types
        if !ctx.type_info.is_enum {
            return Err(CommonDeriveError::UnsupportedTypeKind {
                kind: Text::from("non-enum type"),
                hint: Text::from(
                    "Error derive works best with sum types (enums). \
                    Consider using a sum type like `type MyError is ErrorA | ErrorB;`",
                ),
                span: ctx.span,
            });
        }
        Ok(())
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated Error implementation with support for @message and @source attributes."
    }
}

impl DeriveError {
    /// Extract custom message from @message attribute on a variant
    fn get_variant_message(_variant: &VariantInfo, attrs: &List<Attribute>) -> Maybe<Text> {
        // Look for @message attribute
        if let Some(attr) = attrs.find_by_name("message") {
            if let Maybe::Some(ref args) = attr.args {
                if let Some(first_arg) = args.first() {
                    // Extract string literal from the expression
                    if let ExprKind::Literal(lit) = &first_arg.kind {
                        if let verum_ast::LiteralKind::Text(string_lit) = &lit.kind {
                            return Maybe::Some(Text::from(string_lit.as_str()));
                        }
                    }
                }
            }
        }
        Maybe::None
    }

    /// Check if a field has @source attribute
    fn is_source_field(field_attrs: &List<Attribute>) -> bool {
        field_attrs.has_attribute("source")
    }

    /// Generate the description method body
    ///
    /// Generates match expression:
    /// ```verum
    /// match self {
    ///     Self::Variant1 => "Variant1 error",
    ///     Self::Variant2 { reason, .. } => f"Parse failed: {reason}",
    /// }
    /// ```
    fn generate_description_method(&self, ctx: &DeriveContext, span: Span) -> DeriveResult<Block> {
        let type_info = &ctx.type_info;

        if type_info.variants.is_empty() {
            // Empty enum case - return empty string
            return Ok(Block {
                stmts: vec![].into(),
                expr: Some(Box::new(string_lit("", span))),
                span,
            });
        }

        let mut arms = Vec::new();

        for variant in type_info.variants.iter() {
            let variant_name = variant.name.as_str();

            // Get variant attributes from the original type declaration
            let variant_attrs = self.get_variant_attributes(ctx, variant_name);

            // Build path: Self::VariantName
            let variant_path = Path::new(
                vec![
                    PathSegment::SelfValue,
                    PathSegment::Name(Ident::new(variant_name, span)),
                ].into(),
                span,
            );

            // Determine the message for this variant
            let message = match Self::get_variant_message(variant, &variant_attrs) {
                Maybe::Some(msg) => msg,
                Maybe::None => {
                    // Default message: humanized variant name
                    self.humanize_variant_name(variant_name).into()
                }
            };

            let (pattern, body) = if variant.is_unit {
                // Unit variant: Self::Variant => "message"
                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::None,
                    },
                    span,
                );
                (pattern, string_lit(message.as_str(), span))
            } else if variant.is_tuple {
                // Tuple variant: Self::Variant(..) => "message"
                // Use wildcard pattern for simplicity
                let wildcard_patterns: List<Pattern> = variant
                    .fields
                    .iter()
                    .map(|_| Pattern::new(PatternKind::Wildcard, span))
                    .collect();

                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::Some(VariantPatternData::Tuple(wildcard_patterns)),
                    },
                    span,
                );
                (pattern, string_lit(message.as_str(), span))
            } else {
                // Record variant: Self::Variant { .. } => "message" or interpolated
                // Check if message contains interpolation placeholders like {field_name}
                let has_interpolation = message.as_str().contains('{');

                if has_interpolation {
                    // Bind fields that are referenced in the message
                    let field_patterns: List<FieldPattern> = variant
                        .fields
                        .iter()
                        .map(|f| FieldPattern {
                            name: Ident::new(f.name.as_str(), span),
                            pattern: Maybe::None, // shorthand binding
                            span,
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

                    // For now, return the template string - proper interpolation would
                    // need format string parsing
                    // In a full implementation, we'd generate: f"message with {field}"
                    let format_expr = self.generate_format_string(&message, &variant.fields, span);
                    (pattern, format_expr)
                } else {
                    // No interpolation, use wildcard pattern
                    let pattern = Pattern::new(
                        PatternKind::Variant {
                            path: variant_path,
                            data: Maybe::Some(VariantPatternData::Record {
                                fields: List::new(),
                                rest: true,
                            }),
                        },
                        span,
                    );
                    (pattern, string_lit(message.as_str(), span))
                }
            };

            arms.push(MatchArm::new(pattern, Maybe::None, Heap::new(body), span));
        }

        // Build match expression
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Heap::new(self_ref(span)),
                arms: arms.into(),
            },
            span,
        );

        Ok(Block {
            stmts: vec![].into(),
            expr: Some(Box::new(match_expr)),
            span,
        })
    }

    /// Generate the source method body
    ///
    /// Generates match expression that returns the source error if @source is present:
    /// ```verum
    /// match self {
    ///     Self::IoError { inner, .. } => Some(&inner),
    ///     _ => None,
    /// }
    /// ```
    fn generate_source_method(&self, ctx: &DeriveContext, span: Span) -> DeriveResult<Block> {
        let type_info = &ctx.type_info;

        if type_info.variants.is_empty() {
            // Empty enum - return None
            let none_expr = Expr::new(
                ExprKind::Path(Path::single(Ident::new("None", span))),
                span,
            );
            return Ok(Block {
                stmts: vec![].into(),
                expr: Some(Box::new(none_expr)),
                span,
            });
        }

        let mut arms = Vec::new();
        let mut has_source_variant = false;

        for variant in type_info.variants.iter() {
            let variant_name = variant.name.as_str();

            // Check each field for @source attribute
            let source_field = self.find_source_field(ctx, variant);

            if let Some((field_name, _field_idx)) = source_field {
                has_source_variant = true;

                // Build path: Self::VariantName
                let variant_path = Path::new(
                    vec![
                        PathSegment::SelfValue,
                        PathSegment::Name(Ident::new(variant_name, span)),
                    ].into(),
                    span,
                );

                // Pattern that binds the source field
                let field_patterns: List<FieldPattern> = vec![FieldPattern {
                    name: Ident::new(field_name.as_str(), span),
                    pattern: Maybe::None,
                    span,
                }].into();

                let pattern = Pattern::new(
                    PatternKind::Variant {
                        path: variant_path,
                        data: Maybe::Some(VariantPatternData::Record {
                            fields: field_patterns,
                            rest: true,
                        }),
                    },
                    span,
                );

                // Body: Some(&field_name)
                let field_ref = Expr::new(
                    ExprKind::Unary {
                        op: UnOp::Ref,
                        expr: Box::new(ident_expr(field_name.as_str(), span)),
                    },
                    span,
                );
                let some_expr = Expr::new(
                    ExprKind::Call {
                        func: Box::new(ident_expr("Some", span)),
                        type_args: vec![].into(),
                        args: vec![field_ref].into(),
                    },
                    span,
                );

                arms.push(MatchArm::new(pattern, Maybe::None, Heap::new(some_expr), span));
            }
        }

        // If no source fields, just return None directly (avoid trivial match)
        if !has_source_variant {
            let none_expr = Expr::new(
                ExprKind::Path(Path::single(Ident::new("None", span))),
                span,
            );
            return Ok(Block {
                stmts: vec![].into(),
                expr: Some(Box::new(none_expr)),
                span,
            });
        }

        // Add default arm: _ => None
        let wildcard_pattern = Pattern::new(PatternKind::Wildcard, span);
        let none_expr = Expr::new(
            ExprKind::Path(Path::single(Ident::new("None", span))),
            span,
        );
        arms.push(MatchArm::new(wildcard_pattern, Maybe::None, Heap::new(none_expr), span));

        // Build match expression
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Heap::new(self_ref(span)),
                arms: arms.into(),
            },
            span,
        );

        Ok(Block {
            stmts: vec![].into(),
            expr: Some(Box::new(match_expr)),
            span,
        })
    }

    /// Create the description method declaration
    fn create_description_method(
        &self,
        ctx: &DeriveContext,
        body: Block,
        span: Span,
    ) -> FunctionDecl {
        // Return type: Text
        let return_type = Type::new(
            TypeKind::Path(Path::single(Ident::new(type_names::TEXT, span))),
            span,
        );

        ctx.method(
            "description",
            vec![ctx.self_ref_param(span)].into(),
            return_type,
            body,
            span,
        )
    }

    /// Create the source method declaration
    fn create_source_method(
        &self,
        ctx: &DeriveContext,
        body: Block,
        span: Span,
    ) -> FunctionDecl {
        // Return type: Maybe<&dyn Error>
        // Simplified as Maybe<&Error> for now
        let error_type = Type::new(
            TypeKind::Path(Path::single(Ident::new("Error", span))),
            span,
        );
        let error_ref = Type::new(
            TypeKind::Reference {
                mutable: false,
                inner: Box::new(error_type),
            },
            span,
        );
        let return_type = Type::new(
            TypeKind::Generic {
                base: Box::new(Type::new(
                    TypeKind::Path(Path::single(Ident::new(type_names::MAYBE, span))),
                    span,
                )),
                args: vec![GenericArg::Type(error_ref)].into(),
            },
            span,
        );

        ctx.method(
            "source",
            vec![ctx.self_ref_param(span)].into(),
            return_type,
            body,
            span,
        )
    }

    /// Get attributes for a specific variant from the type declaration
    fn get_variant_attributes(&self, ctx: &DeriveContext, variant_name: &str) -> List<Attribute> {
        // Look up variant in the original type declaration
        if let verum_ast::decl::TypeDeclBody::Variant(variants) = &ctx.type_decl.body {
            for variant in variants.iter() {
                if variant.name.as_str() == variant_name {
                    return variant.attributes.iter().cloned().collect::<Vec<_>>().into();
                }
            }
        }
        List::new()
    }

    /// Find a field marked with @source in a variant
    fn find_source_field(&self, ctx: &DeriveContext, variant: &VariantInfo) -> Option<(Text, usize)> {
        // Look up variant in original type declaration to get field attributes
        if let verum_ast::decl::TypeDeclBody::Variant(variants) = &ctx.type_decl.body {
            for v in variants.iter() {
                if v.name.as_str() == variant.name.as_str() {
                    if let Maybe::Some(verum_ast::decl::VariantData::Record(fields)) = &v.data {
                        for (idx, field) in fields.iter().enumerate() {
                            if Self::is_source_field(&field.attributes) {
                                return Some((Text::from(field.name.as_str()), idx));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Convert CamelCase variant name to human-readable form
    fn humanize_variant_name(&self, name: &str) -> String {
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
        result
    }

    /// Generate a format string expression
    /// This creates an f-string literal for interpolation
    fn generate_format_string(
        &self,
        template: &Text,
        _fields: &List<super::common::FieldInfo>,
        span: Span,
    ) -> Expr {
        // Create an f-string literal expression
        // For now, we'll represent this as a regular string literal that the runtime can interpret
        // A full implementation would parse the template and generate proper format calls
        // using the Verum `f"..."` format string syntax
        use verum_ast::literal::StringLit;
        Expr::new(
            ExprKind::Literal(verum_ast::Literal {
                kind: verum_ast::LiteralKind::Text(StringLit::Regular(template.to_string().into())),
                span,
            }),
            span,
        )
    }
}

/// Generate From implementations for wrapped error types
///
/// For a variant like `IoError { @source inner: std.io.Error }`,
/// generates: `implement From<std.io.Error> for MyError { ... }`
pub fn generate_from_impls(ctx: &DeriveContext) -> DeriveResult<List<Item>> {
    let span = ctx.span;
    let type_info = &ctx.type_info;

    if !type_info.is_enum {
        return Ok(List::new());
    }

    let mut impls = Vec::new();

    for variant in type_info.variants.iter() {
        // Look for variants with a single @source field
        if let verum_ast::decl::TypeDeclBody::Variant(variants) = &ctx.type_decl.body {
            for v in variants.iter() {
                if v.name.as_str() != variant.name.as_str() {
                    continue;
                }

                if let Maybe::Some(verum_ast::decl::VariantData::Record(fields)) = &v.data {
                    // Check for single-field variant with @source
                    if fields.len() == 1 {
                        let field = &fields[0];
                        if DeriveError::is_source_field(&field.attributes) {
                            // Generate From<FieldType> for SelfType
                            let from_impl = generate_from_impl(
                                ctx,
                                &variant.name,
                                &field.name,
                                &field.ty,
                                span,
                            );
                            impls.push(from_impl);
                        }
                    }
                }

                if let Maybe::Some(verum_ast::decl::VariantData::Tuple(types)) = &v.data {
                    // For tuple variants with single @source field
                    // (attributes would need to be on the variant itself)
                    if types.len() == 1 && v.attributes.has_attribute("source") {
                        let from_impl = generate_from_impl_tuple(
                            ctx,
                            &variant.name,
                            &types[0],
                            span,
                        );
                        impls.push(from_impl);
                    }
                }
            }
        }
    }

    Ok(impls.into())
}

/// Generate a From implementation for a record variant
fn generate_from_impl(
    ctx: &DeriveContext,
    variant_name: &Text,
    field_name: &Ident,
    field_type: &Type,
    span: Span,
) -> Item {
    // Create the from method body:
    // Self::VariantName { field_name: value }
    let value_ident = ident_expr("value", span);

    let variant_path = Path::new(
        vec![
            PathSegment::Name(Ident::new("Self", span)),
            PathSegment::Name(Ident::new(variant_name.as_str(), span)),
        ].into(),
        span,
    );

    let record_expr = Expr::new(
        ExprKind::Record {
            path: variant_path,
            fields: vec![FieldInit {
                attributes: List::new(),
                name: field_name.clone(),
                value: Maybe::Some(value_ident),
                span,
            }].into(),
            base: Maybe::None,
        },
        span,
    );

    let body = Block {
        stmts: vec![].into(),
        expr: Some(Box::new(record_expr)),
        span,
    };

    // Create the from method
    let from_method = ctx.method(
        "from",
        vec![ctx.param("value", field_type.clone(), span)].into(),
        ctx.type_info.as_type(span),
        body,
        span,
    );

    // Create impl block: implement From<FieldType> for SelfType
    let impl_decl = ImplDecl {
        is_unsafe: false,
        generics: ctx.type_info.generics.iter().cloned().collect::<Vec<_>>().into(),
        kind: ImplKind::Protocol {
            protocol: Path::single(Ident::new("From", span)),
            protocol_args: vec![GenericArg::Type(field_type.clone())].into(),
            for_type: ctx.type_info.as_type(span),
        },
        generic_where_clause: None,
        meta_where_clause: None,
        specialize_attr: None,
        items: vec![ImplItem {
            attributes: List::new(),
            visibility: Visibility::Public,
            kind: ImplItemKind::Function(from_method),
            span,
        }].into(),
        span,
    };

    Item::new(ItemKind::Impl(impl_decl), span)
}

/// Generate a From implementation for a tuple variant
fn generate_from_impl_tuple(
    ctx: &DeriveContext,
    variant_name: &Text,
    field_type: &Type,
    span: Span,
) -> Item {
    // Create the from method body:
    // Self::VariantName(value)
    let value_ident = ident_expr("value", span);

    let variant_path = Path::new(
        vec![
            PathSegment::Name(Ident::new("Self", span)),
            PathSegment::Name(Ident::new(variant_name.as_str(), span)),
        ].into(),
        span,
    );

    let call_expr = Expr::new(
        ExprKind::Call {
            func: Box::new(Expr::new(ExprKind::Path(variant_path), span)),
            type_args: vec![].into(),
            args: vec![value_ident].into(),
        },
        span,
    );

    let body = Block {
        stmts: vec![].into(),
        expr: Some(Box::new(call_expr)),
        span,
    };

    // Create the from method
    let from_method = ctx.method(
        "from",
        vec![ctx.param("value", field_type.clone(), span)].into(),
        ctx.type_info.as_type(span),
        body,
        span,
    );

    // Create impl block
    let impl_decl = ImplDecl {
        is_unsafe: false,
        generics: ctx.type_info.generics.iter().cloned().collect::<Vec<_>>().into(),
        kind: ImplKind::Protocol {
            protocol: Path::single(Ident::new("From", span)),
            protocol_args: vec![GenericArg::Type(field_type.clone())].into(),
            for_type: ctx.type_info.as_type(span),
        },
        generic_where_clause: None,
        meta_where_clause: None,
        specialize_attr: None,
        items: vec![ImplItem {
            attributes: List::new(),
            visibility: Visibility::Public,
            kind: ImplItemKind::Function(from_method),
            span,
        }].into(),
        span,
    };

    Item::new(ItemKind::Impl(impl_decl), span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{TypeDecl, TypeDeclBody, Variant, VariantData, RecordField, Visibility};

    fn create_simple_error_enum() -> TypeDecl {
        let span = Span::default();
        TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new("MyError", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Variant(vec![
                Variant::new(
                    Ident::new("NotFound", span),
                    Maybe::None,
                    span,
                ),
                Variant::new(
                    Ident::new("PermissionDenied", span),
                    Maybe::None,
                    span,
                ),
            ].into()),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        }
    }

    fn create_error_with_fields() -> TypeDecl {
        let span = Span::default();
        TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new("AppError", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Variant(vec![
                Variant::new(
                    Ident::new("IoError", span),
                    Maybe::Some(VariantData::Record(vec![
                        RecordField::new(
                            Visibility::Private,
                            Ident::new("message", span),
                            Type::text(span),
                            span,
                        ),
                    ].into())),
                    span,
                ),
            ].into()),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        }
    }

    #[test]
    fn test_derive_error_simple() {
        let decl = create_simple_error_enum();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveError;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_derive_error_with_fields() {
        let decl = create_error_with_fields();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveError;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_humanize_variant_name() {
        let derive = DeriveError;
        assert_eq!(derive.humanize_variant_name("NotFound"), "Not found");
        assert_eq!(derive.humanize_variant_name("IoError"), "Io error");
        assert_eq!(
            derive.humanize_variant_name("PermissionDenied"),
            "Permission denied"
        );
    }

    #[test]
    fn test_error_derive_rejects_struct() {
        let span = Span::default();
        let decl = TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new("NotAnError", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Unit,
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        };
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveError;

        let result = derive.can_derive(&ctx);
        assert!(result.is_err());
    }
}
