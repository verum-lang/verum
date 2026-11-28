//! Core Derives Implementation for Verum Meta-System
//!
//! This module implements the 8 core derives required for production use:
//! - Debug: Development, testing, debugging output
//! - Display: Human-readable formatting for end-users
//! - Clone: Deep copy and memory management patterns
//! - PartialEq: Structural equality comparisons
//! - Default: Zero/empty value initialization
//! - Serialize: Data format encoding (JSON, binary, etc.)
//! - Deserialize: Data format decoding with validation
//! - Error: Error protocol implementation with source chaining
//!
//! ## Specification Compliance
//!
//! These derives are P0 (must-have) for initial release of the unified meta-system.
//! Without Serialize/Deserialize, no web frameworks, APIs, or data persistence
//! can be implemented, blocking ecosystem development.
//!
//! ## Semantic Honesty (v5.1 Compliance)
//!
//! - All generated code is fully inspectable via `--show-expansions`
//! - CBGR costs (~15ns per reference) are documented in generated code
//! - No hidden "magic" - everything is explicit
//! - Performance characteristics identical to hand-written code
//!
//! ## Reference Type Awareness
//!
//! Generated code handles both ThinRef (16 bytes) and FatRef (24 bytes)
//! transparently, with explicit CBGR validation costs documented.
//!
//! The unified meta-system is the ONLY compile-time computation mechanism in Verum.
//! All derive macros generate code via the meta-system with full inspectability
//! (--show-expansions flag). The 9 core derives (Debug, Display, Clone, PartialEq,
//! Default, Serialize, Deserialize, Error, Builder) cover the essential code
//! generation needs. Generated code works transparently with both ThinRef (16 bytes)
//! and FatRef (24 bytes) reference types.

pub mod builder;
pub mod cli_command;
pub mod clone;
pub mod debug;
pub mod default;
pub mod deserialize;
pub mod display;
pub mod error;
pub mod hash;
pub mod mysql_typed_row;
pub mod partial_eq;
pub mod serialize;
pub mod shell_render;
pub mod typed_row;

mod common;

use verum_ast::decl::{Item, ItemKind, TypeDecl};
use verum_ast::expr::{Block, Expr, ExprKind};
use verum_ast::literal::IntLit;
use verum_ast::ty::{Ident, Path};
use verum_ast::{Literal, LiteralKind, Span};
use verum_common::{Heap, List, Maybe, Text};

pub use builder::DeriveBuilder;
pub use cli_command::DeriveCommand;
pub use clone::DeriveClone;
pub use common::{DeriveContext, DeriveError, FieldInfo, TypeInfo, VariantInfo, path_from_str};
pub use debug::DeriveDebug;
pub use default::DeriveDefault;
pub use deserialize::DeriveDeserialize;
pub use display::DeriveDisplay;
pub use error::DeriveError as DeriveErrorMacro;
pub use hash::DeriveHash;
pub use mysql_typed_row::DeriveMysqlTypedRow;
pub use partial_eq::{DeriveEq, DerivePartialEq};
pub use serialize::DeriveSerialize;
pub use typed_row::DeriveTypedRow;

/// Result type for derive operations
pub type DeriveResult<T> = Result<T, DeriveError>;

/// Expand built-in derives supported on the ordinary source path. An applied
/// argument is consumed so an already-prepared project or inline body can
/// safely pass through selection again. Other derive arguments keep their spans.
pub(crate) fn expand_source_derives(items: &mut List<Item>, enabled: bool) -> anyhow::Result<()> {
    static REGISTRY: std::sync::LazyLock<DeriveRegistry> =
        std::sync::LazyLock::new(DeriveRegistry::new);
    fn structural_name(arg: &Expr) -> Option<&str> {
        let ExprKind::Path(path) = &arg.kind else { return None };
        let name = path.as_ident()?.as_str();
        matches!(name, "Hash" | "Eq" | "PartialEq").then_some(name)
    }

    let mut pending = List::from(vec![items]);
    while let Some(items) = pending.pop() {
        let mut generated = List::new();
        for item in &mut *items {
            let ItemKind::Type(decl) = &mut item.kind else { continue };
            let mut requested: List<(Text, Span)> = List::new();
            for arg in decl.attributes.iter().filter(|attr| attr.name.as_str() == "derive")
                .flat_map(|attr| attr.args.iter().flatten())
            {
                let Some(name) = structural_name(arg) else { continue };
                anyhow::ensure!(enabled, "`@derive({name})` on `{}` is disabled by `[meta] derive` at {:?}", decl.name, arg.span);
                anyhow::ensure!(!requested.iter().any(|(seen, _)| seen.as_str() == name),
                    "duplicate `@derive({name})` on `{}` at {:?}", decl.name, arg.span);
                requested.push((name.into(), arg.span));
            }
            if requested.is_empty() { continue; }
            generated.extend(REGISTRY.expand_requested(decl, &requested)?);
            decl.attributes.retain_mut(|attr| {
                if attr.name.as_str() == "derive" && let Some(args) = &mut attr.args {
                    args.retain(|arg| structural_name(arg).is_none());
                    return !args.is_empty();
                }
                true
            });
        }
        items.extend(generated);
        for item in items {
            if let ItemKind::Module(decl) = &mut item.kind
                && let Some(children) = &mut decl.items
            {
                pending.push(children);
            }
        }
    }
    Ok(())
}

/// Registry for all derive macros
pub struct DeriveRegistry {
    /// Registered derive implementations
    derives: verum_common::Map<Text, Box<dyn DeriveMacro>>,
}

impl DeriveRegistry {
    /// Create a new registry with all core derives registered
    pub fn new() -> Self {
        let mut registry = Self {
            derives: verum_common::Map::new(),
        };

        // Structural and formatting derives
        registry.register("Debug", Box::new(DeriveDebug));
        registry.register("Clone", Box::new(DeriveClone));
        registry.register("PartialEq", Box::new(DerivePartialEq));
        registry.register("Eq", Box::new(DeriveEq));
        registry.register("Default", Box::new(DeriveDefault));
        registry.register("Serialize", Box::new(DeriveSerialize));
        registry.register("Deserialize", Box::new(DeriveDeserialize));
        registry.register("Display", Box::new(DeriveDisplay));
        registry.register("Error", Box::new(DeriveErrorMacro));
        // Builder pattern derive for fluent construction APIs
        registry.register("Builder", Box::new(DeriveBuilder));
        // ShellRender — auto-generate `render()` for typed command DSLs
        registry.register(
            "ShellRender",
            Box::new(self::shell_render::DeriveShellRender),
        );
        // TypedRow — auto-generate `core.database.postgres.typed_row.TypedRow`
        // impl for record / newtype types.
        registry.register("TypedRow", Box::new(DeriveTypedRow));
        // Hash — auto-generate `core.base.protocols.Hash` impl,
        // closes the Eq+Hash pair (PartialEq exists since v0).
        registry.register("Hash", Box::new(DeriveHash));
        // MysqlTypedRow — symmetric to TypedRow but targets the
        // MySQL-side `core.database.mysql.typed_row.MysqlTypedRow`
        // protocol (different method name, different cell type).
        registry.register("MysqlTypedRow", Box::new(DeriveMysqlTypedRow));
        // Command — vcli @command derive: lower a `@command`-decorated
        // record type to a CommandSpec factory function. Triggered by
        // the `@command(...)` attribute itself (see `extract_derive_names`)
        // or via explicit `@derive(Command)`. Spec:
        // cli-framework spec §4.
        registry.register("Command", Box::new(DeriveCommand));

        registry
    }

    /// Register a custom derive macro
    pub fn register(&mut self, name: &str, derive: Box<dyn DeriveMacro>) {
        self.derives.insert(Text::from(name), derive);
    }

    /// Look up a derive by name
    pub fn get(&self, name: &str) -> Option<&dyn DeriveMacro> {
        self.derives.get(&Text::from(name)).map(|d| d.as_ref())
    }

    /// Expand a derive attribute on a type
    pub fn expand(
        &self,
        derive_name: &str,
        type_decl: &TypeDecl,
        span: Span,
    ) -> DeriveResult<Item> {
        let derive = self
            .get(derive_name)
            .ok_or_else(|| DeriveError::UnknownDerive {
                name: Text::from(derive_name),
                span,
            })?;

        let ctx = DeriveContext::from_type_decl(type_decl, span)?;
        derive.expand(&ctx)
    }

    /// Expand all derives on a type declaration
    pub fn expand_all(
        &self,
        type_decl: &TypeDecl,
        derives: &[Text],
        span: Span,
    ) -> DeriveResult<List<Item>> {
        let source_args = type_decl
            .attributes
            .iter()
            .filter(|attr| attr.name.as_str() == "derive")
            .flat_map(|attr| attr.args.iter().flatten());
        let requested: List<_> = derives
            .iter()
            .map(|name| {
                let source_span = source_args.clone().find_map(|arg| {
                    let ExprKind::Path(path) = &arg.kind else {
                        return None;
                    };
                    (path.as_ident()?.as_str() == name.as_str()).then_some(arg.span)
                });
                (name.clone(), source_span.unwrap_or(span))
            })
            .collect();
        self.expand_requested(type_decl, &requested)
    }

    pub(crate) fn expand_requested(
        &self,
        decl: &TypeDecl,
        requested: &[(Text, Span)],
    ) -> DeriveResult<List<Item>> {
        let companions: List<_> = requested
            .iter()
            .filter_map(|(name, _)| self.get(name.as_str())?.companion_protocol())
            .collect();
        let mut items = List::new();
        for (name, span) in requested {
            if companions.contains(&name.as_str()) {
                continue;
            }
            let generator = self
                .get(name.as_str())
                .ok_or_else(|| DeriveError::UnknownDerive {
                    name: name.clone(),
                    span: *span,
                })?;
            let context = DeriveContext::from_type_decl(decl, *span)?;
            items.extend(generator.expand_group(&context)?);
        }
        Ok(items)
    }
}

impl Default for DeriveRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for derive macro implementations
pub trait DeriveMacro: Send + Sync {
    /// The name of this derive
    fn name(&self) -> &'static str;

    /// The protocol this derive implements
    fn protocol_name(&self) -> &'static str;

    /// Expand the derive for the given type
    fn expand(&self, ctx: &DeriveContext) -> DeriveResult<Item>;

    /// A companion supplied by this generator also satisfies a separate request for it.
    fn companion_protocol(&self) -> Option<&'static str> {
        None
    }

    /// Expand all declarations this generator supplies for a source attribute.
    fn expand_group(&self, ctx: &DeriveContext) -> DeriveResult<List<Item>> {
        Ok(List::from_iter([self.expand(ctx)?]))
    }

    /// Check if this derive can be applied to the given type
    fn can_derive(&self, _ctx: &DeriveContext) -> Result<(), DeriveError> {
        // Default: can derive for any type with accessible fields
        Ok(())
    }

    /// Get documentation for the generated code
    fn doc_comment(&self) -> &'static str {
        "Auto-generated implementation"
    }
}

/// Helper function to create a method call expression
pub fn method_call(receiver: Expr, method: &str, args: List<Expr>, span: Span) -> Expr {
    Expr::new(
        ExprKind::MethodCall {
            receiver: Box::new(receiver),
            method: Ident::new(method, span),
            type_args: List::new(),
            args: args.clone(),
        },
        span,
    )
}

/// Helper function to create a field access expression
pub fn field_access(receiver: Expr, field: &str, span: Span) -> Expr {
    Expr::new(
        ExprKind::Field {
            expr: Box::new(receiver),
            field: Ident::new(field, span),
        },
        span,
    )
}

/// Helper function to create a self reference expression
pub fn self_ref(span: Span) -> Expr {
    Expr::new(ExprKind::Path(Path::single(Ident::new("self", span))), span)
}

/// Helper function to create an identifier expression
pub fn ident_expr(name: &str, span: Span) -> Expr {
    Expr::new(ExprKind::Path(Path::single(Ident::new(name, span))), span)
}

/// Helper function to create a string literal expression
pub fn string_lit(value: &str, span: Span) -> Expr {
    use verum_ast::literal::StringLit;
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Text(StringLit::Regular(value.to_string().into())),
            span,
        }),
        span,
    )
}

/// Helper function to create an integer literal expression
pub fn int_lit(value: i128, span: Span) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Int(IntLit {
                value,
                suffix: None,
            }),
            span,
        }),
        span,
    )
}

/// Helper function to create a bool literal expression
pub fn bool_lit(value: bool, span: Span) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Bool(value),
            span,
        }),
        span,
    )
}

/// Helper function to create a binary operation expression
pub fn binary_op(left: Expr, op: verum_ast::BinOp, right: Expr, span: Span) -> Expr {
    Expr::new(
        ExprKind::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        },
        span,
    )
}

/// Helper function to create a block expression
pub fn block_expr(stmts: List<verum_ast::Stmt>, expr: Maybe<Heap<Expr>>, span: Span) -> Expr {
    Expr::new(
        ExprKind::Block(Block {
            stmts: stmts.clone(),
            expr: expr.map(|e| Box::new((*e).clone())).into(),
            span,
        }),
        span,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = DeriveRegistry::new();

        // All 9 core derives should be registered (8 essential + Builder)
        assert!(registry.get("Debug").is_some());
        assert!(registry.get("Clone").is_some());
        assert!(registry.get("PartialEq").is_some());
        assert!(registry.get("Default").is_some());
        assert!(registry.get("Serialize").is_some());
        assert!(registry.get("Deserialize").is_some());
        assert!(registry.get("Display").is_some());
        assert!(registry.get("Error").is_some());
        // Builder derive for fluent construction
        assert!(registry.get("Builder").is_some());
        // TypedRow derive for postgres typed-row protocol
        assert!(registry.get("TypedRow").is_some());
        // Hash derive — completes the Eq+Hash pair
        assert!(registry.get("Hash").is_some());
        // MysqlTypedRow derive — symmetric to TypedRow
        assert!(registry.get("MysqlTypedRow").is_some());

        // Unknown derives should return None
        assert!(registry.get("Unknown").is_none());
    }

    #[test]
    fn test_helper_functions() {
        let span = Span::default();

        // Test self_ref
        let self_expr = self_ref(span);
        assert!(matches!(self_expr.kind, ExprKind::Path(_)));

        // Test string_lit
        let str_expr = string_lit("test", span);
        assert!(matches!(str_expr.kind, ExprKind::Literal(_)));

        // Test int_lit
        let int_expr = int_lit(42, span);
        assert!(matches!(int_expr.kind, ExprKind::Literal(_)));

        // Test bool_lit
        let bool_expr = bool_lit(true, span);
        assert!(matches!(bool_expr.kind, ExprKind::Literal(_)));
    }
}
