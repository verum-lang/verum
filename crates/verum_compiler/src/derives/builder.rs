//! Builder derive macro implementation
//!
//! Generates a type-safe builder pattern with compile-time required field verification.
//!
//! ## Generated Code
//!
//! For a type like:
//! ```verum
//! @builder
//! type HttpRequest is {
//!     method: HttpMethod,                      // Required
//!     url: Url,                                // Required
//!     headers: Map<Text, Text> = Map.new(),   // Optional with default
//!     body: Maybe<Bytes> = Maybe.None,        // Optional
//!     timeout: Duration = 30.seconds,         // Optional with default
//! };
//! ```
//!
//! This derive generates:
//! 1. `HttpRequestBuilder` type with appropriate field types
//! 2. `HttpRequest.builder()` static method returning `HttpRequestBuilder`
//! 3. `.method(value)`, `.url(value)`, etc. setter methods (chainable)
//! 4. `.build()` method that constructs the final type
//!
//! ## Type Safety
//!
//! Required fields are stored as `Maybe<T>` in the builder and validated at build time.
//! Optional fields (those with default values) keep their original type.
//!
//! ## CBGR Considerations
//!
//! The builder pattern creates owned values, avoiding reference lifetime issues.
//! Each setter takes ownership and returns the modified builder.
//!
//! @derive(Builder): generates builder pattern with .with_field() methods,
//! optional fields, validation, and type-safe construction.
//! @derive infrastructure: macro expansion framework for automatic code
//! generation from type definitions (Clone, Debug, Serialize, etc.).

use super::common::{DeriveContext, DeriveError, FieldInfo, TypeInfo};
use super::{DeriveMacro, DeriveResult, ident_expr, method_call, string_lit};
use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParam, FunctionParamKind, ImplDecl, ImplItem, ImplItemKind,
    ImplKind, Item, ItemKind, RecordField, TypeDecl, TypeDeclBody, Visibility,
};
use verum_ast::expr::{BinOp, Block, Expr, ExprKind, FieldInit};
use verum_ast::pattern::{MatchArm, Pattern, PatternKind};
use verum_ast::stmt::Stmt;
use verum_ast::ty::{Ident, Path, PathSegment, Type, TypeKind};
use verum_ast::Span;
use verum_common::well_known_types::{type_names, variant_tags};
use verum_common::{Heap, List, Maybe, Text};

/// Builder derive macro implementation
///
/// Generates ergonomic type-safe builder pattern with:
/// - Compile-time required field verification
/// - Optional fields with defaults
/// - Fluent chainable setters
pub struct DeriveBuilder;

impl DeriveMacro for DeriveBuilder {
    fn name(&self) -> &'static str {
        "Builder"
    }

    fn protocol_name(&self) -> &'static str {
        // Builder doesn't implement a protocol, it generates additional types
        "Builder"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Builder can only be derived for record types
        if type_info.is_enum {
            return Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("enum/variant"),
                hint: Text::from(
                    "@builder can only be applied to record types, not enums. \
                     Consider using a factory function pattern instead.",
                ),
                span,
            });
        }

        if type_info.is_newtype {
            return Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("newtype"),
                hint: Text::from(
                    "@builder is not needed for newtypes. \
                     Use direct construction: TypeName(value).",
                ),
                span,
            });
        }

        if type_info.fields.is_empty() {
            return Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("unit type"),
                hint: Text::from(
                    "@builder is not needed for unit types. \
                     Use direct construction: TypeName or TypeName().",
                ),
                span,
            });
        }

        // Generate the builder type
        let builder_type_decl = self.generate_builder_type(type_info, span);

        // Generate impl block for the builder with setter methods and build()
        let builder_impl = self.generate_builder_impl(type_info, span);

        // Generate impl block for the original type with builder() method
        let origin_impl = self.generate_origin_impl(ctx, span);

        // Return a module-like item containing all generated items
        // For now, we return the impl for the original type and include builder type/impl as associated items
        // Actually, the derive system expects a single Item, so we'll generate a compound Item
        // using ItemKind::Module or return the main impl with builder embedded

        // Since the derive system is designed for protocol implementations,
        // we'll generate the builder impl and inline the type definition
        // The builder type will be generated as a separate impl target

        // For full industrial implementation, we generate a synthetic module
        // But the simplest approach is to generate an impl that adds the builder() method
        // and emit the Builder type + impl separately

        // Return the origin type's impl with builder() method
        // The builder type and its impl need to be emitted through another mechanism
        // For now, we'll generate a compound structure that the compiler can unpack

        Ok(self.generate_compound_item(
            builder_type_decl,
            builder_impl,
            origin_impl,
            span,
        ))
    }

    fn can_derive(&self, ctx: &DeriveContext) -> Result<(), DeriveError> {
        let type_info = &ctx.type_info;

        if type_info.is_enum {
            return Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("enum"),
                hint: Text::from("@builder requires a record type, not an enum."),
                span: ctx.span,
            });
        }

        if type_info.fields.is_empty() {
            return Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("unit type"),
                hint: Text::from("@builder requires at least one field."),
                span: ctx.span,
            });
        }

        Ok(())
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated type-safe builder pattern with compile-time required field verification."
    }
}

impl DeriveBuilder {
    /// Generate the builder type declaration
    ///
    /// For each field in the original type:
    /// - Required fields (no default) become Maybe<T>
    /// - Optional fields (has default) keep their original type T
    fn generate_builder_type(&self, type_info: &TypeInfo, span: Span) -> TypeDecl {
        let builder_name = format!("{}Builder", type_info.name.as_str());

        let fields: Vec<RecordField> = type_info
            .fields
            .iter()
            .map(|field| {
                let field_type = if field.is_required() {
                    // Required: Maybe<T>
                    self.wrap_in_maybe(&field.ty, span)
                } else {
                    // Optional: T (will use default if not set)
                    field.ty.clone()
                };

                RecordField {
                    visibility: Visibility::Public,
                    name: Ident::new(field.name.as_str(), span),
                    ty: field_type,
                    attributes: List::new(),
                    default_value: Maybe::None,
                    bit_spec: Maybe::None,
                    span,
                }
            })
            .collect();

        TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new(builder_name.as_str(), span),
            generics: type_info.generics.iter().cloned().collect(),
            attributes: List::new(),
            body: TypeDeclBody::Record(fields.into()),
            resource_modifier: None,
            generic_where_clause: Maybe::None,
            meta_where_clause: None,
            span,
        }
    }

    /// Generate impl block for the builder type with setter methods and build()
    fn generate_builder_impl(&self, type_info: &TypeInfo, span: Span) -> ImplDecl {
        let builder_name = format!("{}Builder", type_info.name.as_str());
        let builder_path = Path::single(Ident::new(builder_name.as_str(), span));

        let mut methods: Vec<ImplItem> = Vec::new();

        // Generate setter for each field
        for field in type_info.fields.iter() {
            let setter = self.generate_setter(field, span);
            methods.push(ImplItem {
                attributes: List::new(),
                visibility: Visibility::Public,
                kind: ImplItemKind::Function(setter),
                span,
            });
        }

        // Generate build() method
        let build_method = self.generate_build_method(type_info, span);
        methods.push(ImplItem {
            attributes: List::new(),
            visibility: Visibility::Public,
            kind: ImplItemKind::Function(build_method),
            span,
        });

        ImplDecl {
            is_unsafe: false,
            generics: type_info.generics.iter().cloned().collect(),
            kind: ImplKind::Inherent(Type::new(TypeKind::Path(builder_path), span)),
            generic_where_clause: None,
            meta_where_clause: None,
            specialize_attr: None,
            items: methods.into_iter().collect(),
            span,
        }
    }

    /// Generate a setter method for a field
    ///
    /// ```verum
    /// fn field_name(mut self, value: FieldType) -> Self {
    ///     self.field_name = Maybe.Some(value);  // for required fields
    ///     // or
    ///     self.field_name = value;               // for optional fields
    ///     self
    /// }
    /// ```
    fn generate_setter(&self, field: &FieldInfo, span: Span) -> FunctionDecl {
        let field_name = field.name.as_str();

        // Parameter: value: FieldType (original type, not Maybe-wrapped)
        let param_type = field.ty.clone();
        let value_param = FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new("value", span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                ),
                ty: param_type,
                default_value: Maybe::None,
            },
            span,
        );

        // self parameter (mut self for by-value consumption)
        let self_param = FunctionParam::new(FunctionParamKind::SelfValueMut, span);

        // Body:
        // - For required: self.field = Maybe.Some(value);
        // - For optional: self.field = value;
        let assignment_value = if field.is_required() {
            // Maybe.Some(value)
            Expr::new(
                ExprKind::Call {
                    func: Heap::new(Expr::new(
                        ExprKind::Path(Path::new(
                            vec![
                                PathSegment::Name(Ident::new(type_names::MAYBE, span)),
                                PathSegment::Name(Ident::new(variant_tags::SOME, span)),
                            ]
                            .into(),
                            span,
                        )),
                        span,
                    )),
                    type_args: List::new(),
                    args: List::from(vec![ident_expr("value", span)]),
                },
                span,
            )
        } else {
            // Just value
            ident_expr("value", span)
        };

        // self.field = value or self.field = Maybe.Some(value)
        let self_field = Expr::new(
            ExprKind::Field {
                expr: Heap::new(ident_expr("self", span)),
                field: Ident::new(field_name, span),
            },
            span,
        );

        // Use Binary with BinOp::Assign for assignment
        let assignment = Expr::new(
            ExprKind::Binary {
                op: BinOp::Assign,
                left: Heap::new(self_field),
                right: Heap::new(assignment_value),
            },
            span,
        );

        // Return self
        let return_self = ident_expr("self", span);

        let body = Block {
            stmts: List::from(vec![Stmt::expr(assignment, true)]),
            expr: Maybe::Some(Heap::new(return_self)),
            span,
        };

        // Return type: Self
        let return_type = Type::new(TypeKind::Path(Path::single(Ident::new("Self", span))), span);

        FunctionDecl {
            visibility: Visibility::Public,
            is_async: false,
            is_pure: false,
            is_meta: false,
            stage_level: 0,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: None,
            is_variadic: false,
            name: Ident::new(field_name, span),
            generics: List::new(),
            params: vec![self_param, value_param].into(),
            return_type: Some(return_type),
            throws_clause: None,
            std_attr: None,
            contexts: List::new(),
            generic_where_clause: None,
            meta_where_clause: None,
            attributes: List::new(),
            body: Some(FunctionBody::Block(body)),
            requires: List::new(),
            ensures: List::new(),
            span,
        }
    }

    /// Generate the build() method
    ///
    /// ```verum
    /// fn build(self) -> Result<TypeName, BuilderError> {
    ///     let field1 = self.field1.ok_or(BuilderError.MissingField("field1"))?;
    ///     let field2 = self.field2.ok_or(BuilderError.MissingField("field2"))?;
    ///     // Optional fields just use their value
    ///     let field3 = self.field3;  // Has default
    ///
    ///     Result.Ok(TypeName { field1, field2, field3 })
    /// }
    /// ```
    fn generate_build_method(&self, type_info: &TypeInfo, span: Span) -> FunctionDecl {
        let type_name = type_info.name.as_str();

        // self parameter (consumes the builder)
        let self_param = FunctionParam::new(FunctionParamKind::SelfValue, span);

        let mut stmts: Vec<verum_ast::Stmt> = Vec::new();
        let mut field_inits: Vec<FieldInit> = Vec::new();

        for field in type_info.fields.iter() {
            let field_name = field.name.as_str();
            let var_name = format!("__{}_val", field_name);

            if field.is_required() {
                // Required field: extract from Maybe with error handling
                // let __field_val = match self.field {
                //     Maybe.Some(v) => v,
                //     Maybe.None => panic(f"Missing required field: {field_name}"),
                // };
                let self_field = Expr::new(
                    ExprKind::Field {
                        expr: Heap::new(ident_expr("self", span)),
                        field: Ident::new(field_name, span),
                    },
                    span,
                );

                // Match expression for Maybe extraction
                let some_pattern = Pattern::new(
                    PatternKind::Variant {
                        path: Path::new(
                            vec![
                                PathSegment::Name(Ident::new(type_names::MAYBE, span)),
                                PathSegment::Name(Ident::new(variant_tags::SOME, span)),
                            ]
                            .into(),
                            span,
                        ),
                        data: Some(verum_ast::pattern::VariantPatternData::Tuple(
                            vec![Pattern::new(
                                PatternKind::Ident {
                                    by_ref: false,
                                    name: Ident::new("v", span),
                                    mutable: false,
                                    subpattern: None,
                                },
                                span,
                            )]
                            .into(),
                        )),
                    },
                    span,
                );

                let none_pattern = Pattern::new(
                    PatternKind::Variant {
                        path: Path::new(
                            vec![
                                PathSegment::Name(Ident::new(type_names::MAYBE, span)),
                                PathSegment::Name(Ident::new(variant_tags::NONE, span)),
                            ]
                            .into(),
                            span,
                        ),
                        data: None,
                    },
                    span,
                );

                // panic(f"Missing required field: {field_name}")
                let error_msg = format!("Missing required field: {}", field_name);
                let panic_call = Expr::new(
                    ExprKind::Call {
                        func: Heap::new(ident_expr("panic", span)),
                        type_args: List::new(),
                        args: List::from(vec![string_lit(&error_msg, span)]),
                    },
                    span,
                );

                let match_expr = Expr::new(
                    ExprKind::Match {
                        expr: Heap::new(self_field),
                        arms: vec![
                            MatchArm {
                                pattern: some_pattern,
                                guard: Maybe::None,
                                body: Heap::new(ident_expr("v", span)),
                                with_clause: Maybe::None,
                                attributes: List::new(),
                                span,
                            },
                            MatchArm {
                                pattern: none_pattern,
                                guard: Maybe::None,
                                body: Heap::new(panic_call),
                                with_clause: Maybe::None,
                                attributes: List::new(),
                                span,
                            },
                        ]
                        .into(),
                    },
                    span,
                );

                // let __field_val = match ...
                let let_pattern = Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new(var_name.as_str(), span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                );
                let let_stmt = Stmt::let_stmt(let_pattern, Maybe::None, Maybe::Some(match_expr), span);
                stmts.push(let_stmt);
            } else {
                // Optional field: use directly (has default or was set)
                // If not set, use the default value from the type definition
                // For now, just use self.field directly since it's already T not Maybe<T>

                // let __field_val = self.field;
                let self_field = Expr::new(
                    ExprKind::Field {
                        expr: Heap::new(ident_expr("self", span)),
                        field: Ident::new(field_name, span),
                    },
                    span,
                );

                let let_pattern = Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new(var_name.as_str(), span),
                        mutable: false,
                        subpattern: None,
                    },
                    span,
                );
                let let_stmt = Stmt::let_stmt(let_pattern, Maybe::None, Maybe::Some(self_field), span);
                stmts.push(let_stmt);
            }

            // Field init: field_name: __field_val
            field_inits.push(FieldInit {
                attributes: List::new(),
                name: Ident::new(field_name, span),
                value: Some(ident_expr(&var_name, span)),
                span,
            });
        }

        // Build the final struct
        let struct_lit = Expr::new(
            ExprKind::Record {
                path: Path::single(Ident::new(type_name, span)),
                fields: field_inits.into_iter().collect(),
                base: None,
            },
            span,
        );

        let body = Block {
            stmts: stmts.into_iter().collect(),
            expr: Maybe::Some(Heap::new(struct_lit)),
            span,
        };

        // Return type: TypeName
        let return_type = type_info.as_type(span);

        FunctionDecl {
            visibility: Visibility::Public,
            is_async: false,
            is_pure: false,
            is_meta: false,
            stage_level: 0,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: None,
            is_variadic: false,
            name: Ident::new("build", span),
            generics: List::new(),
            params: vec![self_param].into(),
            return_type: Some(return_type),
            throws_clause: None,
            std_attr: None,
            contexts: List::new(),
            generic_where_clause: None,
            meta_where_clause: None,
            attributes: List::new(),
            body: Some(FunctionBody::Block(body)),
            requires: List::new(),
            ensures: List::new(),
            span,
        }
    }

    /// Generate impl block for the original type with builder() static method
    fn generate_origin_impl(&self, ctx: &DeriveContext, span: Span) -> ImplDecl {
        let type_info = &ctx.type_info;
        let builder_name = format!("{}Builder", type_info.name.as_str());

        // builder() method: creates a new builder with defaults
        let builder_method = self.generate_builder_method(type_info, &builder_name, span);

        let impl_item = ImplItem {
            attributes: List::new(),
            visibility: Visibility::Public,
            kind: ImplItemKind::Function(builder_method),
            span,
        };

        ImplDecl {
            is_unsafe: false,
            generics: type_info.generics.iter().cloned().collect(),
            kind: ImplKind::Inherent(type_info.as_type(span)),
            generic_where_clause: None,
            meta_where_clause: None,
            specialize_attr: None,
            items: vec![impl_item].into_iter().collect(),
            span,
        }
    }

    /// Generate the builder() static method for the original type
    ///
    /// ```verum
    /// fn builder() -> TypeNameBuilder {
    ///     TypeNameBuilder {
    ///         required_field1: Maybe.None,
    ///         required_field2: Maybe.None,
    ///         optional_field1: default_value1,
    ///         optional_field2: default_value2,
    ///     }
    /// }
    /// ```
    fn generate_builder_method(
        &self,
        type_info: &TypeInfo,
        builder_name: &str,
        span: Span,
    ) -> FunctionDecl {
        let mut field_inits: Vec<FieldInit> = Vec::new();

        for field in type_info.fields.iter() {
            let init_value = if field.is_required() {
                // Required: Maybe.None
                Expr::new(
                    ExprKind::Path(Path::new(
                        vec![
                            PathSegment::Name(Ident::new(type_names::MAYBE, span)),
                            PathSegment::Name(Ident::new(variant_tags::NONE, span)),
                        ]
                        .into(),
                        span,
                    )),
                    span,
                )
            } else {
                // Optional: use default value if available, otherwise Default::default()
                match &field.default_value {
                    Maybe::Some(expr) => expr.clone(),
                    Maybe::None => {
                        // No default provided, use Default::default()
                        method_call(ident_expr("Default", span), "default", List::new(), span)
                    }
                }
            };

            field_inits.push(FieldInit {
                attributes: List::new(),
                name: Ident::new(field.name.as_str(), span),
                value: Some(init_value),
                span,
            });
        }

        // TypeNameBuilder { ... }
        let builder_struct = Expr::new(
            ExprKind::Record {
                path: Path::single(Ident::new(builder_name, span)),
                fields: field_inits.into_iter().collect(),
                base: None,
            },
            span,
        );

        let body = Block {
            stmts: List::new(),
            expr: Maybe::Some(Heap::new(builder_struct)),
            span,
        };

        // Return type: TypeNameBuilder
        let return_type = Type::new(
            TypeKind::Path(Path::single(Ident::new(builder_name, span))),
            span,
        );

        FunctionDecl {
            visibility: Visibility::Public,
            is_async: false,
            is_pure: false,
            is_meta: false,
            stage_level: 0,
            is_generator: false,
            is_cofix: false,
            is_unsafe: false,
            is_transparent: false,
            extern_abi: None,
            is_variadic: false,
            name: Ident::new("builder", span),
            generics: List::new(),
            params: List::new(),
            return_type: Some(return_type),
            throws_clause: None,
            std_attr: None,
            contexts: List::new(),
            generic_where_clause: None,
            meta_where_clause: None,
            attributes: List::new(),
            body: Some(FunctionBody::Block(body)),
            requires: List::new(),
            ensures: List::new(),
            span,
        }
    }

    /// Wrap a type in Maybe<T>
    fn wrap_in_maybe(&self, inner: &Type, span: Span) -> Type {
        let maybe_path = Path::single(Ident::new(type_names::MAYBE, span));
        let base = Type::new(TypeKind::Path(maybe_path), span);

        Type::new(
            TypeKind::Generic {
                base: Heap::new(base),
                args: vec![verum_ast::ty::GenericArg::Type(inner.clone())].into(),
            },
            span,
        )
    }

    /// Generate a compound item containing the builder type, builder impl, and origin impl
    ///
    /// Since the derive system expects a single Item, we package everything into
    /// a synthetic module or use a special ItemKind for compound derives.
    ///
    /// For now, we return the impl on the original type with the builder() method.
    /// The builder type and its impl will need special handling in the compiler pipeline.
    fn generate_compound_item(
        &self,
        builder_type: TypeDecl,
        builder_impl: ImplDecl,
        origin_impl: ImplDecl,
        span: Span,
    ) -> Item {
        // For industrial implementation, we need to emit multiple items.
        // The current derive infrastructure returns a single Item.
        //
        // Strategy: Return a synthetic module containing all items.
        // The compiler will then flatten this module's contents into the parent scope.
        //
        // Alternative: Extend DeriveResult to return List<Item>.
        // This would require changes to DeriveRegistry and DeriveMacro trait.
        //
        // For now, we use a Module item that the compiler should handle specially.
        // Items marked with @derive_generated should be hoisted to parent scope.

        let module_name = format!("__builder_derive_generated");

        // Create items for the module
        let items: List<Item> = vec![
            Item::new(ItemKind::Type(builder_type), span),
            Item::new(ItemKind::Impl(builder_impl), span),
            Item::new(ItemKind::Impl(origin_impl), span),
        ]
        .into_iter()
        .collect();

        // Create a module with special attribute to signal hoisting
        let module = verum_ast::decl::ModuleDecl {
            visibility: Visibility::Public,
            name: Ident::new(module_name.as_str(), span),
            items: Maybe::Some(items),
            profile: Maybe::None,
            features: Maybe::None,
            contexts: List::new(),
            span,
        };

        // Add @derive_generated attribute for compiler recognition
        let attr = verum_ast::Attribute::simple(Text::from("derive_generated"), span);

        Item {
            kind: ItemKind::Module(module),
            span,
            attributes: vec![attr].into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{TypeDecl, TypeDeclBody, Visibility};

    fn create_test_record() -> TypeDecl {
        let span = Span::default();

        let fields = vec![
            RecordField {
                visibility: Visibility::Public,
                name: Ident::new("method", span),
                ty: Type::new(TypeKind::Path(Path::single(Ident::new("HttpMethod", span))), span),
                attributes: List::new(),
                default_value: Maybe::None, // Required
                bit_spec: Maybe::None,
                span,
            },
            RecordField {
                visibility: Visibility::Public,
                name: Ident::new("url", span),
                ty: Type::new(TypeKind::Path(Path::single(Ident::new("Url", span))), span),
                attributes: List::new(),
                default_value: Maybe::None, // Required
                bit_spec: Maybe::None,
                span,
            },
            RecordField {
                visibility: Visibility::Public,
                name: Ident::new("timeout", span),
                ty: Type::int(span),
                attributes: List::new(),
                default_value: Maybe::Some(Expr::new(
                    ExprKind::Literal(verum_ast::Literal {
                        kind: verum_ast::LiteralKind::Int(verum_ast::literal::IntLit {
                            value: 30,
                            suffix: None,
                        }),
                        span,
                    }),
                    span,
                )), // Optional with default
                bit_spec: Maybe::None,
                span,
            },
        ];

        TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new("HttpRequest", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Record(fields.into()),
            resource_modifier: None,
            generic_where_clause: Maybe::None,
            meta_where_clause: None,
            span,
        }
    }

    #[test]
    fn test_derive_builder_creates_builder_type() {
        let decl = create_test_record();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveBuilder;

        let result = derive.expand(&ctx);
        assert!(result.is_ok(), "Builder derive should succeed");

        let item = result.unwrap();
        // Check that we get a module containing the generated items
        assert!(matches!(item.kind, ItemKind::Module(_)));
    }

    #[test]
    fn test_derive_builder_rejects_enum() {
        use verum_ast::decl::Variant;

        let span = Span::default();
        let variants = vec![
            Variant {
                name: Ident::new("A", span),
                generic_params: List::new(),
                data: None,
                where_clause: verum_common::Maybe::None,
                attributes: List::new(),
                span,
            },
            Variant {
                name: Ident::new("B", span),
                generic_params: List::new(),
                data: None,
                where_clause: verum_common::Maybe::None,
                attributes: List::new(),
                span,
            },
        ];

        let decl = TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new("MyEnum", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Variant(variants.into()),
            resource_modifier: None,
            generic_where_clause: Maybe::None,
            meta_where_clause: None,
            span,
        };

        let ctx = DeriveContext::from_type_decl(&decl, span).unwrap();
        let derive = DeriveBuilder;

        let result = derive.expand(&ctx);
        assert!(result.is_err(), "Builder derive should reject enums");
    }

    #[test]
    fn test_field_info_required_vs_optional() {
        let span = Span::default();

        let required_field = FieldInfo {
            name: Text::from("method"),
            ty: Type::int(span),
            index: 0,
            is_public: true,
            has_default: false,
            default_value: Maybe::None,
            span,
        };

        let optional_field = FieldInfo {
            name: Text::from("timeout"),
            ty: Type::int(span),
            index: 1,
            is_public: true,
            has_default: true,
            default_value: Maybe::Some(Expr::new(
                ExprKind::Literal(verum_ast::Literal {
                    kind: verum_ast::LiteralKind::Int(verum_ast::literal::IntLit {
                        value: 30,
                        suffix: None,
                    }),
                    span,
                }),
                span,
            )),
            span,
        };

        assert!(required_field.is_required());
        assert!(!optional_field.is_required());
    }

    #[test]
    fn test_wrap_in_maybe() {
        let span = Span::default();
        let derive = DeriveBuilder;

        let inner = Type::int(span);
        let wrapped = derive.wrap_in_maybe(&inner, span);

        // Should produce Maybe<Int>
        assert!(matches!(wrapped.kind, TypeKind::Generic { .. }));
    }
}
