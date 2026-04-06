//! Default derive macro implementation
//!
//! Generates `implement Default for Type { fn default() -> Self }`
//!
//! @derive(Default): generates Default protocol implementation with
//! sensible defaults for each field type (0 for Int, "" for Text, etc.).

use super::common::{DeriveContext, DeriveError};
use super::{DeriveMacro, DeriveResult, ident_expr, method_call};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, Item};
use verum_ast::expr::{Block, Expr, ExprKind, FieldInit};
use verum_ast::ty::{Ident, Path, Type, TypeKind};
use verum_common::{List, Text};

/// Default derive implementation
pub struct DeriveDefault;

impl DeriveMacro for DeriveDefault {
    fn name(&self) -> &'static str {
        "Default"
    }

    fn protocol_name(&self) -> &'static str {
        "Default"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        // Generate default method body
        let body = if type_info.is_enum {
            return Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("enum"),
                hint: Text::from(
                    "Cannot derive Default for enums without specifying default variant.",
                ),
                span,
            });
        } else if type_info.is_newtype {
            self.generate_newtype_default(type_info, span)?
        } else {
            self.generate_struct_default(type_info, span)?
        };

        // Create the default method
        let default_method = self.create_default_method(ctx, body, span);

        // Generate impl block
        Ok(ctx.generate_impl("Default", List::from(vec![default_method]), span))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated Default implementation with zero-cost initialization."
    }
}

impl DeriveDefault {
    /// Generate default body for struct types
    fn generate_struct_default(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        let type_name = type_info.name.as_str();

        // Build struct fields: field: Default::default()
        let mut fields = List::new();
        for field in type_info.fields.iter() {
            let default_call =
                method_call(ident_expr("Default", span), "default", List::new(), span);
            fields.push(FieldInit {
                attributes: List::new(),
                name: field.ident(span),
                value: Some(default_call),
                span,
            });
        }

        // TypeName { field1: Default::default(), ... }
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

    /// Generate default body for newtype
    fn generate_newtype_default(
        &self,
        type_info: &super::common::TypeInfo,
        span: Span,
    ) -> DeriveResult<Block> {
        let type_name = type_info.name.as_str();

        // TypeName(Default::default())
        let default_call = method_call(ident_expr("Default", span), "default", List::new(), span);
        let call_expr = Expr::new(
            ExprKind::Call {
                func: Box::new(ident_expr(type_name, span)),
                type_args: List::new(),
                args: List::from(vec![default_call]),
            },
            span,
        );

        Ok(Block {
            stmts: List::new(),
            expr: Some(Box::new(call_expr)),
            span,
        })
    }

    /// Create the default method declaration
    fn create_default_method(&self, ctx: &DeriveContext, body: Block, span: Span) -> FunctionDecl {
        // Return type: Self
        let return_type = Type::new(TypeKind::Path(Path::single(Ident::new("Self", span))), span);

        ctx.method(
            "default",
            List::new(), // No parameters for default()
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
            name: Ident::new("Config", span),
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
    fn test_derive_default() {
        let decl = create_test_struct();
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let derive = DeriveDefault;

        let result = derive.expand(&ctx);
        assert!(result.is_ok());
    }
}
