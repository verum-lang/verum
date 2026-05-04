//! `@derive(MysqlTypedRow)` — auto-generate the
//! `core.database.mysql.typed_row.MysqlTypedRow` protocol impl
//! for record / newtype types.
//!
//! Mirror of `derives::typed_row::DeriveTypedRow` (which targets
//! the PG protocol). Closes the parity gap: every Verum struct
//! that wants to be lifted from a MySQL prepared/binlog row can
//! now `@derive(MysqlTypedRow)` instead of hand-writing the
//! per-cell `match` cascade against `RowValue` variants.
//!
//! # Generated code shape
//!
//! For a record:
//!
//! ```verum
//! @derive(MysqlTypedRow)
//! type User is { id: Int, name: Text, age: Maybe<Int> };
//! ```
//!
//! the macro emits:
//!
//! ```verum
//! implement MysqlTypedRow for User {
//!     fn from_mysql_row(row: &List<RowValue>) -> Result<User, DbError> {
//!         core.database.mysql.typed_row.expect_arity(row, 3)?;
//!         let id: Int =
//!             core.database.mysql.typed_row.decode_field(row, 0)?;
//!         let name: Text =
//!             core.database.mysql.typed_row.decode_field(row, 1)?;
//!         let age: Maybe<Int> =
//!             core.database.mysql.typed_row.decode_field(row, 2)?;
//!         Ok(User { id: id, name: name, age: age })
//!     }
//! }
//! ```
//!
//! For a newtype `type UserId is (Int);`:
//!
//! ```verum
//! implement MysqlTypedRow for UserId {
//!     fn from_mysql_row(row: &List<RowValue>) -> Result<UserId, DbError> {
//!         core.database.mysql.typed_row.expect_arity(row, 1)?;
//!         let inner: Int =
//!             core.database.mysql.typed_row.decode_field(row, 0)?;
//!         Ok(UserId(inner))
//!     }
//! }
//! ```
//!
//! # Differences from PG `@derive(TypedRow)`
//!
//! - Method name `from_mysql_row` (NOT `from_typed_row`).
//! - Row type `&List<RowValue>` (NOT `&List<TypedValue>`).
//! - No `desc` parameter — MySQL's typed-row protocol decodes
//!   purely from the column-aligned `RowValue` list.
//! - Helper paths in `core.database.mysql.typed_row` (NOT
//!   `core.database.postgres.typed_row`).
//!
//! Otherwise architecturally identical: bound inference enabled
//! (generic records get `where T: MysqlTypedRow`), sum types
//! rejected with the same friendly hint.

use super::common::{DeriveContext, DeriveError, FieldInfo, TypeInfo};
use super::{DeriveMacro, DeriveResult, ident_expr, int_lit};
use verum_ast::Span;
use verum_ast::decl::{FunctionDecl, FunctionParam, FunctionParamKind, Item, Visibility};
use verum_ast::expr::{Block, Expr, ExprKind, FieldInit};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_ast::ty::{GenericArg, Ident, Path, PathSegment, Type, TypeKind};
use verum_common::{Heap, List, Maybe, Text};

/// `@derive(MysqlTypedRow)` macro implementation.
pub struct DeriveMysqlTypedRow;

impl DeriveMacro for DeriveMysqlTypedRow {
    fn name(&self) -> &'static str {
        "MysqlTypedRow"
    }

    fn protocol_name(&self) -> &'static str {
        "MysqlTypedRow"
    }

    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item> {
        let span = ctx.span;
        let type_info = &ctx.type_info;

        if type_info.is_enum {
            return Err(DeriveError::UnsupportedTypeKind {
                kind: Text::from("enum"),
                hint: Text::from(
                    "Cannot derive MysqlTypedRow for sum types — they need a \
                     hand-rolled impl that picks a discriminator column. \
                     Implement MysqlTypedRow manually.",
                ),
                span,
            });
        }

        let body = if type_info.is_newtype {
            self.generate_newtype_body(type_info, span)?
        } else {
            self.generate_struct_body(type_info, span)?
        };

        let method = self.create_from_mysql_row_method(ctx, body, span);
        Ok(ctx.generate_impl_with_field_bounds(
            "MysqlTypedRow",
            List::from(vec![method]),
            span,
        ))
    }

    fn doc_comment(&self) -> &'static str {
        "Auto-generated MysqlTypedRow implementation — positional column \
         decode via `core.database.mysql.typed_row.decode_field`."
    }
}

impl DeriveMysqlTypedRow {
    /// Body for a record:
    ///
    /// ```text
    /// expect_arity(row, N)?;
    /// let f1: T1 = decode_field(row, 0)?;
    /// ...
    /// Ok(TypeName { f1: f1, ... })
    /// ```
    fn generate_struct_body(&self, type_info: &TypeInfo, span: Span) -> DeriveResult<Block> {
        let arity = type_info.fields.len() as u32;
        let mut stmts: Vec<Stmt> = Vec::new();

        // expect_arity(row, N)?;
        stmts.push(Stmt::expr(
            self.try_expr(
                self.qualified_call(
                    "expect_arity",
                    List::from(vec![
                        ident_expr("row", span),
                        int_lit(arity as i128, span),
                    ]),
                    span,
                ),
                span,
            ),
            true,
        ));

        for field in type_info.fields.iter() {
            stmts.push(self.field_decode_stmt(field, span));
        }

        // Ok(TypeName { f1: f1, ... })
        let record_lit = Expr::new(
            ExprKind::Record {
                path: type_info.as_path(span),
                fields: type_info
                    .fields
                    .iter()
                    .map(|f| FieldInit {
                        attributes: List::new(),
                        name: f.ident(span),
                        value: Maybe::Some(ident_expr(f.name.as_str(), span)),
                        span,
                    })
                    .collect(),
                base: Maybe::None,
            },
            span,
        );
        let ok_call = Expr::new(
            ExprKind::Call {
                func: Heap::new(ident_expr("Ok", span)),
                type_args: List::new(),
                args: List::from(vec![record_lit]),
            },
            span,
        );

        Ok(Block {
            stmts: List::from(stmts),
            expr: Maybe::Some(Heap::new(ok_call)),
            span,
        })
    }

    fn generate_newtype_body(&self, type_info: &TypeInfo, span: Span) -> DeriveResult<Block> {
        let mut stmts: Vec<Stmt> = Vec::new();
        stmts.push(Stmt::expr(
            self.try_expr(
                self.qualified_call(
                    "expect_arity",
                    List::from(vec![ident_expr("row", span), int_lit(1, span)]),
                    span,
                ),
                span,
            ),
            true,
        ));

        let field = type_info
            .fields
            .iter()
            .next()
            .ok_or_else(|| DeriveError::Internal {
                message: Text::from("newtype TypeInfo missing inner field"),
                span,
            })?;

        stmts.push(self.let_stmt_with_type(
            "inner",
            field.ty.clone(),
            self.try_expr(
                self.qualified_call(
                    "decode_field",
                    List::from(vec![ident_expr("row", span), int_lit(0, span)]),
                    span,
                ),
                span,
            ),
            span,
        ));

        let ctor = Expr::new(
            ExprKind::Call {
                func: Heap::new(ident_expr(type_info.name.as_str(), span)),
                type_args: List::new(),
                args: List::from(vec![ident_expr("inner", span)]),
            },
            span,
        );
        let ok_call = Expr::new(
            ExprKind::Call {
                func: Heap::new(ident_expr("Ok", span)),
                type_args: List::new(),
                args: List::from(vec![ctor]),
            },
            span,
        );

        Ok(Block {
            stmts: List::from(stmts),
            expr: Maybe::Some(Heap::new(ok_call)),
            span,
        })
    }

    // ------------------------------------------------------------------
    // Method declaration
    // ------------------------------------------------------------------

    /// `fn from_mysql_row(row: &List<RowValue>) -> Result<Self, DbError>`
    fn create_from_mysql_row_method(
        &self,
        ctx: &DeriveContext,
        body: Block,
        span: Span,
    ) -> FunctionDecl {
        let row_ty = self.ref_type(self.generic_type(
            "List",
            vec![self.qualified_type(
                &[
                    "core",
                    "database",
                    "mysql",
                    "binlog_rows",
                    "RowValue",
                ],
                span,
            )],
            span,
        ));
        let return_ty = self.generic_type(
            "Result",
            vec![
                Type::new(TypeKind::Path(Path::single(Ident::new("Self", span))), span),
                self.qualified_type(
                    &[
                        "core",
                        "database",
                        "common",
                        "error",
                        "DbError",
                    ],
                    span,
                ),
            ],
            span,
        );

        let row_param = self.param("row", row_ty, span);

        ctx.method(
            "from_mysql_row",
            List::from(vec![row_param]),
            return_ty,
            body,
            span,
        )
    }

    fn param(&self, name: &str, ty: Type, span: Span) -> FunctionParam {
        FunctionParam::new(
            FunctionParamKind::Regular {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new(name, span),
                        mutable: false,
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

    // ------------------------------------------------------------------
    // Body-level helpers
    // ------------------------------------------------------------------

    fn field_decode_stmt(&self, field: &FieldInfo, span: Span) -> Stmt {
        let call = self.qualified_call(
            "decode_field",
            List::from(vec![
                ident_expr("row", span),
                int_lit(field.index as i128, span),
            ]),
            span,
        );
        let value = self.try_expr(call, span);
        self.let_stmt_with_type(field.name.as_str(), field.ty.clone(), value, span)
    }

    fn let_stmt_with_type(&self, name: &str, ty: Type, value: Expr, span: Span) -> Stmt {
        Stmt::new(
            StmtKind::Let {
                pattern: Pattern::new(
                    PatternKind::Ident {
                        by_ref: false,
                        name: Ident::new(name, span),
                        mutable: false,
                        subpattern: Maybe::None,
                    },
                    span,
                ),
                ty: Maybe::Some(ty),
                value: Maybe::Some(value),
            },
            span,
        )
    }

    fn try_expr(&self, inner: Expr, span: Span) -> Expr {
        Expr::new(ExprKind::Try(Heap::new(inner)), span)
    }

    /// `core.database.mysql.typed_row.<fname>(args)`
    fn qualified_call(&self, fname: &str, args: List<Expr>, span: Span) -> Expr {
        let path = Path::new(
            List::from(vec![
                PathSegment::Name(Ident::new("core", span)),
                PathSegment::Name(Ident::new("database", span)),
                PathSegment::Name(Ident::new("mysql", span)),
                PathSegment::Name(Ident::new("typed_row", span)),
                PathSegment::Name(Ident::new(fname, span)),
            ]),
            span,
        );
        let func = Expr::new(ExprKind::Path(path), span);
        Expr::new(
            ExprKind::Call {
                func: Heap::new(func),
                type_args: List::new(),
                args,
            },
            span,
        )
    }

    // ------------------------------------------------------------------
    // Type-construction helpers
    // ------------------------------------------------------------------

    fn ref_type(&self, inner: Type) -> Type {
        let span = inner.span;
        Type::new(
            TypeKind::Reference {
                mutable: false,
                inner: Heap::new(inner),
            },
            span,
        )
    }

    fn generic_type(&self, base_name: &str, args: Vec<Type>, span: Span) -> Type {
        let base = Type::new(
            TypeKind::Path(Path::single(Ident::new(base_name, span))),
            span,
        );
        let generic_args: Vec<GenericArg> = args.into_iter().map(GenericArg::Type).collect();
        Type::new(
            TypeKind::Generic {
                base: Heap::new(base),
                args: List::from(generic_args),
            },
            span,
        )
    }

    fn qualified_type(&self, segments: &[&str], span: Span) -> Type {
        let path_segments: Vec<PathSegment> = segments
            .iter()
            .map(|s| PathSegment::Name(Ident::new(*s, span)))
            .collect();
        Type::new(
            TypeKind::Path(Path::new(List::from(path_segments), span)),
            span,
        )
    }
}

#[allow(dead_code)]
const _: Option<Visibility> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::{RecordField, TypeDecl, TypeDeclBody, Visibility};

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
        let item = DeriveMysqlTypedRow.expand(&ctx).expect("expand should succeed");
        match item.kind {
            verum_ast::decl::ItemKind::Impl(_) => {}
            other => panic!("expected ItemKind::Impl, got {:?}", other),
        }
    }

    #[test]
    fn enum_rejected() {
        let span = Span::default();
        let decl = TypeDecl {
            visibility: Visibility::Public,
            name: Ident::new("Tag", span),
            generics: List::new(),
            attributes: List::new(),
            body: TypeDeclBody::Variant(List::from(vec![verum_ast::decl::Variant::new(
                Ident::new("A", span),
                Maybe::None,
                span,
            )])),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: None,
            span,
        };
        let ctx = DeriveContext::from_type_decl(&decl, Span::default()).unwrap();
        let err = DeriveMysqlTypedRow.expand(&ctx).expect_err("enum rejected");
        match err {
            DeriveError::UnsupportedTypeKind { kind, .. } => {
                assert_eq!(kind.as_str(), "enum");
            }
            other => panic!("expected UnsupportedTypeKind, got {:?}", other),
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
        let item = DeriveMysqlTypedRow.expand(&ctx).expect("newtype expand");
        match item.kind {
            verum_ast::decl::ItemKind::Impl(_) => {}
            other => panic!("expected Impl, got {:?}", other),
        }
    }
}
