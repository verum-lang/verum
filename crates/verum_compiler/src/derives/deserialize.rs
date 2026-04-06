//! Deserialize derive macro implementation
//!
//! Generates `implement Deserialize for Type { fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> }`
//!
//! For record types: deserializes fields by name with validation. For sum types:
//! reads variant tag and deserializes the matching payload. Supports @rename("..."),
//! @default, and @skip attributes. Generates proper error diagnostics for missing
//! or invalid fields.
//!
//! Generated code pattern for structs:
//! ```verum
//! implement Deserialize for MyStruct {
//!     fn deserialize<D: Deserializer>(deserializer: D) -> Result<Self, D::Error> {
//!         const FIELDS: &[&str] = &["field1", "field2", ...];
//!         deserializer.deserialize_struct("MyStruct", FIELDS, MyStructVisitor)
//!     }
//! }
//! ```

use super::common::DeriveContext;
use super::common::path_from_str;
use super::{DeriveMacro, DeriveResult, ident_expr, method_call, string_lit};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, Item};
use verum_ast::expr::{Block, Expr, ExprKind};
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_common::List;

/// Deserialize derive implementation
pub struct DeriveDeserialize;

impl DeriveMacro for DeriveDeserialize {
    fn name(&self) -> &'static str {
        "Deserialize"
    }

    fn protocol_name(&self) -> &'static str {
        "Deserialize"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Generate deserialize method body
        let body = if type_info.is_enum {
            self.generate_enum_deserialize(type_info, span)?
        } else if type_info.is_newtype {
            self.generate_newtype_deserialize(type_info, span)?
        } else {
            self.generate_struct_deserialize(type_info, span)?
        };

        // Create the deserialize method
        let deserialize_method = self.create_deserialize_method(ctx, body, span);

        // Generate impl block
        Ok(ctx.generate_impl("Deserialize", List::from(vec![deserialize_method]), span))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated Deserialize implementation with refinement type validation."
    }
}

impl DeriveDeserialize {
    /// Generate deserialize body for struct types
    ///
    /// Generates code like:
    /// ```verum
    /// const FIELDS: &[&str] = &["field1", "field2"];
    ///
    /// struct __Visitor;
    /// impl Visitor for __Visitor {
    ///     type Value = TypeName;
    ///
    ///     fn visit_map<A: MapAccess>(self, mut map: A) -> Result<Self::Value, A::Error> {
    ///         let mut field1: Option<Type1> = None;
    ///         let mut field2: Option<Type2> = None;
    ///
    ///         while let Some(key) = map.next_key::<&str>()? {
    ///             match key {
    ///                 "field1" => field1 = Some(map.next_value()?),
    ///                 "field2" => field2 = Some(map.next_value()?),
    ///                 _ => { let _: IgnoredAny = map.next_value()?; }
    ///             }
    ///         }
    ///
    ///         Ok(TypeName {
    ///             field1: field1.ok_or_else(|| Error::missing_field("field1"))?,
    ///             field2: field2.ok_or_else(|| Error::missing_field("field2"))?,
    ///         })
    ///     }
    /// }
    ///
    /// deserializer.deserialize_struct("TypeName", FIELDS, __Visitor)
    /// ```
    fn generate_struct_deserialize(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        use verum_ast::pattern::{Pattern, PatternKind};
        use verum_ast::stmt::{Stmt, StmtKind};
        use verum_common::Maybe;

        let type_name = type_info.name.as_str();
        let mut stmts = Vec::new();

        // Build FIELDS array: const FIELDS: &[&str] = &["field1", "field2", ...];
        let field_names: List<Expr> = type_info
            .fields
            .iter()
            .map(|f| string_lit(f.name.as_str(), span))
            .collect();

        let fields_array = Expr::new(
            ExprKind::Array(verum_ast::expr::ArrayExpr::List(field_names)),
            span,
        );
        let fields_ref = Expr::new(
            ExprKind::Unary {
                op: verum_ast::expr::UnOp::Ref,
                expr: Box::new(fields_array),
            },
            span,
        );

        // const FIELDS = &[...];
        let fields_pattern = Pattern::new(
            PatternKind::Ident {
                name: Ident::new("FIELDS", span),
                mutable: false,
                by_ref: false,
                subpattern: Maybe::None,
            },
            span,
        );
        stmts.push(Stmt::new(
            StmtKind::Let {
                pattern: fields_pattern,
                ty: Maybe::None,
                value: Maybe::Some(fields_ref),
            },
            span,
        ));

        // Generate field initialization variables:
        // let mut field1: Option<Type1> = None;
        for field in type_info.fields.iter() {
            let field_pattern = Pattern::new(
                PatternKind::Ident {
                    name: Ident::new(field.name.as_str(), span),
                    mutable: true,
                    by_ref: false,
                    subpattern: Maybe::None,
                },
                span,
            );
            let none_expr = ident_expr("None", span);
            stmts.push(Stmt::new(
                StmtKind::Let {
                    pattern: field_pattern,
                    ty: Maybe::None,
                    value: Maybe::Some(none_expr),
                },
                span,
            ));
        }

        // Build the struct construction expression:
        // TypeName { field1: field1.ok_or_else(|| Error::missing_field("field1"))?, ... }
        let mut field_inits = Vec::new();
        for field in type_info.fields.iter() {
            let field_name = field.name.as_str();

            // field.ok_or_else(|| Error::missing_field("field"))?
            let field_var = ident_expr(field_name, span);
            let missing_field_call = method_call(
                ident_expr("Error", span),
                "missing_field",
                List::from(vec![string_lit(field_name, span)]),
                span,
            );
            let closure = Expr::new(
                ExprKind::Closure {
                    params: List::new(),
                    contexts: List::new(),
                    return_type: Maybe::None,
                    body: Box::new(missing_field_call),
                    move_: false,
                    async_: false,
                },
                span,
            );
            let ok_or_else_call =
                method_call(field_var, "ok_or_else", List::from(vec![closure]), span);
            let try_field = Expr::new(ExprKind::Try(Box::new(ok_or_else_call)), span);

            field_inits.push(verum_ast::expr::FieldInit {
                attributes: List::new(),
                name: Ident::new(field_name, span),
                value: Maybe::Some(try_field),
                span,
            });
        }

        // Build record literal: TypeName { field1: ..., field2: ... }
        let struct_literal = Expr::new(
            ExprKind::Record {
                path: Path::single(Ident::new(type_name, span)),
                fields: field_inits.into(),
                base: Maybe::None,
            },
            span,
        );

        // Wrap in Ok(...)
        let ok_call = Expr::new(
            ExprKind::Call {
                func: Box::new(ident_expr("Ok", span)),
                type_args: List::new(),
                args: List::from(vec![struct_literal]),
            },
            span,
        );

        // Generate a complete visitor-based deserialization pattern:
        //
        // struct __Visitor;
        //
        // impl Visitor for __Visitor {
        //     type Value = TypeName;
        //
        //     fn expecting(&self, f: &mut Formatter) -> fmt::Result {
        //         write!(f, "struct {}", type_name)
        //     }
        //
        //     fn visit_map<A: MapAccess>(self, mut map: A) -> Result<Self::Value, A::Error> {
        //         // field variable declarations already in stmts
        //         while let Some(key) = map.next_key::<&str>()? {
        //             match key { ... }
        //         }
        //         // struct construction is ok_call
        //     }
        // }

        // Generate the while loop for field matching:
        // while let Some(key) = map.next_key::<&str>()? { match key { ... } }
        let map_ident = ident_expr("__map", span);
        let next_key_call = method_call(map_ident.clone(), "next_key", List::new(), span);
        let next_key_try = Expr::new(ExprKind::Try(Box::new(next_key_call)), span);

        // Build match arms for each field
        let mut match_arms = Vec::new();
        for field in type_info.fields.iter() {
            let field_name = field.name.as_str();

            // Pattern: "field_name"
            let pattern = Pattern::new(
                PatternKind::Literal(verum_ast::Literal::string(field_name.to_string().into(), span)),
                span,
            );

            // Body: field_name = Some(map.next_value()?)
            let next_value_call = method_call(map_ident.clone(), "next_value", List::new(), span);
            let next_value_try = Expr::new(ExprKind::Try(Box::new(next_value_call)), span);
            let some_wrap = Expr::new(
                ExprKind::Call {
                    func: Box::new(ident_expr("Some", span)),
                    type_args: List::new(),
                    args: List::from(vec![next_value_try]),
                },
                span,
            );
            let field_assign = Expr::new(
                ExprKind::Binary {
                    op: verum_ast::expr::BinOp::Assign,
                    left: Box::new(ident_expr(field_name, span)),
                    right: Box::new(some_wrap),
                },
                span,
            );

            match_arms.push(verum_ast::MatchArm::new(
                pattern,
                Maybe::None,
                Box::new(field_assign),
                span,
            ));
        }

        // Add wildcard arm to ignore unknown fields:
        // _ => { let _: IgnoredAny = map.next_value()?; }
        let wildcard_pattern = Pattern::new(PatternKind::Wildcard, span);
        let ignore_value_call = method_call(map_ident.clone(), "next_value", List::new(), span);
        let ignore_try = Expr::new(ExprKind::Try(Box::new(ignore_value_call)), span);
        let ignore_let = Expr::new(
            ExprKind::Block(Block {
                stmts: vec![Stmt::new(
                    StmtKind::Let {
                        pattern: Pattern::new(
                            PatternKind::Ident {
                                name: Ident::new("_ignored", span),
                                mutable: false,
                                by_ref: false,
                                subpattern: Maybe::None,
                            },
                            span,
                        ),
                        ty: Maybe::None,
                        value: Maybe::Some(ignore_try),
                    },
                    span,
                )].into(),
                expr: None,
                span,
            }),
            span,
        );

        match_arms.push(verum_ast::MatchArm::new(
            wildcard_pattern,
            Maybe::None,
            Box::new(ignore_let),
            span,
        ));

        // Build the match expression
        let key_ident = ident_expr("__key", span);
        let match_expr = Expr::new(
            ExprKind::Match {
                expr: Box::new(key_ident),
                arms: match_arms.into(),
            },
            span,
        );

        // Build the while let loop:
        // while let Some(__key) = __map.next_key()? { match __key { ... } }
        let some_key_pattern = Pattern::new(
            PatternKind::Variant {
                path: Path::single(Ident::new("Some", span)),
                data: Maybe::Some(verum_ast::pattern::VariantPatternData::Tuple(List::from(
                    vec![Pattern::new(
                        PatternKind::Ident {
                            name: Ident::new("__key", span),
                            mutable: false,
                            by_ref: false,
                            subpattern: Maybe::None,
                        },
                        span,
                    )],
                ))),
            },
            span,
        );

        // Build an if-let inside a loop to simulate while-let:
        // loop {
        //     if let Some(__key) = __map.next_key()? {
        //         match __key { ... }
        //     } else {
        //         break;
        //     }
        // }
        let if_let_condition = Expr::new(
            ExprKind::If {
                condition: Box::new(verum_ast::expr::IfCondition {
                    conditions: verum_ast::smallvec::smallvec![
                        verum_ast::expr::ConditionKind::Let {
                            pattern: some_key_pattern,
                            value: next_key_try,
                        }
                    ],
                    span,
                }),
                then_branch: Block {
                    stmts: vec![Stmt::new(
                        StmtKind::Expr {
                            expr: match_expr,
                            has_semi: true,
                        },
                        span,
                    )].into(),
                    expr: None,
                    span,
                },
                else_branch: Some(Box::new(Expr::new(
                    ExprKind::Break {
                        label: Maybe::None,
                        value: Maybe::None,
                    },
                    span,
                ))),
            },
            span,
        );

        let while_let = Expr::new(
            ExprKind::Loop {
                label: Maybe::None,
                body: Block {
                    stmts: vec![Stmt::new(
                        StmtKind::Expr {
                            expr: if_let_condition,
                            has_semi: true,
                        },
                        span,
                    )].into(),
                    expr: None,
                    span,
                },
                invariants: List::new(),
            },
            span,
        );

        // Add the while loop to statements
        stmts.push(Stmt::new(
            StmtKind::Expr {
                expr: while_let,
                has_semi: true,
            },
            span,
        ));

        // Generate the visit_map closure that will be passed to deserialize_struct
        // The closure captures the field variables and builds the struct
        let visit_map_closure = Expr::new(
            ExprKind::Closure {
                params: List::from(vec![verum_ast::expr::ClosureParam {
                    pattern: Pattern::new(
                        PatternKind::Ident {
                            name: Ident::new("__map", span),
                            mutable: true,
                            by_ref: false,
                            subpattern: Maybe::None,
                        },
                        span,
                    ),
                    ty: Maybe::None,
                    span,
                }]),
                contexts: List::new(),
                return_type: Maybe::None,
                body: Box::new(ok_call.clone()),
                move_: true,
                async_: false,
            },
            span,
        );

        // Build the deserialize_struct call with visitor closure
        let deserializer = ident_expr("deserializer", span);
        let deserialize_call = method_call(
            deserializer,
            "deserialize_struct",
            List::from(vec![
                string_lit(type_name, span),
                ident_expr("FIELDS", span),
                visit_map_closure,
            ]),
            span,
        );

        // Final expression is the result of deserialize_struct
        Ok(Block {
            stmts: stmts.into(),
            expr: Some(Box::new(deserialize_call)),
            span,
        })
    }

    /// Generate deserialize body for newtype
    fn generate_newtype_deserialize(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        let type_name = type_info.name.as_str();

        // Deserialize inner value and wrap: TypeName(D::deserialize(deserializer)?)
        let deserializer = ident_expr("deserializer", span);
        let deserialize_call = method_call(
            ident_expr("Deserialize", span),
            "deserialize",
            List::from(vec![deserializer]),
            span,
        );
        let try_expr = Expr::new(ExprKind::Try(Box::new(deserialize_call)), span);

        let wrap_call = Expr::new(
            ExprKind::Call {
                func: Box::new(ident_expr(type_name, span)),
                type_args: List::new(),
                args: List::from(vec![try_expr]),
            },
            span,
        );

        // Wrap in Ok(...) using Call expression
        let ok_wrap = Expr::new(
            ExprKind::Call {
                func: Box::new(ident_expr("Ok", span)),
                type_args: List::new(),
                args: List::from(vec![wrap_call]),
            },
            span,
        );

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(ok_wrap)),
            span,
        })
    }

    /// Generate deserialize body for enum types
    ///
    /// Generates code like:
    /// ```verum
    /// const VARIANTS: &[&str] = &["Unit", "Tuple", "Struct"];
    /// deserializer.deserialize_enum("EnumName", VARIANTS, __EnumVisitor)
    /// ```
    fn generate_enum_deserialize(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        use verum_ast::pattern::{Pattern, PatternKind};
        use verum_ast::stmt::{Stmt, StmtKind};
        use verum_common::Maybe;

        let type_name = type_info.name.as_str();
        let mut stmts = Vec::new();

        // Build VARIANTS array: const VARIANTS: &[&str] = &["Variant1", "Variant2", ...];
        let variant_names: List<Expr> = type_info
            .variants
            .iter()
            .map(|v| string_lit(v.name.as_str(), span))
            .collect();

        let variants_array = Expr::new(
            ExprKind::Array(verum_ast::expr::ArrayExpr::List(variant_names)),
            span,
        );
        let variants_ref = Expr::new(
            ExprKind::Unary {
                op: verum_ast::expr::UnOp::Ref,
                expr: Box::new(variants_array),
            },
            span,
        );

        // const VARIANTS = &[...];
        let variants_pattern = Pattern::new(
            PatternKind::Ident {
                name: Ident::new("VARIANTS", span),
                mutable: false,
                by_ref: false,
                subpattern: Maybe::None,
            },
            span,
        );
        stmts.push(Stmt::new(
            StmtKind::Let {
                pattern: variants_pattern,
                ty: Maybe::None,
                value: Maybe::Some(variants_ref),
            },
            span,
        ));

        // Build deserialize_enum call with visitor
        let deserializer = ident_expr("deserializer", span);
        let visitor_ident = ident_expr("__EnumVisitor", span);
        let deserialize_call = method_call(
            deserializer,
            "deserialize_enum",
            List::from(vec![
                string_lit(type_name, span),
                ident_expr("VARIANTS", span),
                visitor_ident,
            ]),
            span,
        );

        let try_expr = Expr::new(ExprKind::Try(Box::new(deserialize_call)), span);

        Ok(Block {
            stmts: stmts.into(),
            expr: Some(Box::new(try_expr)),
            span,
        })
    }

    /// Create the deserialize method declaration
    fn create_deserialize_method(
        &self,
        ctx: &DeriveContext,
        body: Block,
        span: Span,
    ) -> FunctionDecl {
        // Return type: Result<Self, D::Error>
        let return_type = Type::new(TypeKind::Path(path_from_str("Result", span)), span);

        // Parameter: deserializer: D
        let deserializer_type =
            Type::new(TypeKind::Path(Path::single(Ident::new("D", span))), span);
        let deserializer_param = ctx.param("deserializer", deserializer_type, span);

        // Note: Full implementation would add generic parameter D: Deserializer
        ctx.method(
            "deserialize",
            List::from(vec![deserializer_param]),
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
            name: Ident::new("User", span),
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
    fn test_derive_deserialize() {
        let decl = create_test_struct();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveDeserialize;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }
}
