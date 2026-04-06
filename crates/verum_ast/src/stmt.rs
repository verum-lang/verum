//! Statement nodes in the AST.
//!
//! Statements are executed for their side effects and do not produce values
//! (except in the case of expression statements in tail position).

use crate::expr::{Block, Expr};
use crate::pattern::Pattern;
use crate::span::{Span, Spanned};
use crate::ty::Type;
use serde::{Deserialize, Serialize};
use verum_common::{Heap, Maybe, Text};

/// A statement in Verum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
    /// Attributes attached to this statement (e.g., @unroll, @parallel, @likely)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<crate::decl::Attribute>,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Self { kind, span, attributes: Vec::new() }
    }

    pub fn with_attributes(kind: StmtKind, span: Span, attributes: Vec<crate::decl::Attribute>) -> Self {
        Self { kind, span, attributes }
    }

    pub fn let_stmt(pattern: Pattern, ty: Maybe<Type>, value: Maybe<Expr>, span: Span) -> Self {
        Self::new(StmtKind::Let { pattern, ty, value }, span)
    }

    pub fn expr(expr: Expr, has_semi: bool) -> Self {
        let span = expr.span;
        Self::new(StmtKind::Expr { expr, has_semi }, span)
    }

    pub fn item(item: Item) -> Self {
        let span = item.span();
        Self::new(StmtKind::Item(item), span)
    }
}

impl Spanned for Stmt {
    fn span(&self) -> Span {
        self.span
    }
}

/// The kind of statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StmtKind {
    /// Let binding: let x = expr or let x: T = expr
    Let {
        pattern: Pattern,
        ty: Maybe<Type>,
        value: Maybe<Expr>,
    },

    /// Let-else statement: let pattern = expr else { diverging_block }
    LetElse {
        pattern: Pattern,
        ty: Maybe<Type>,
        value: Expr,
        else_block: Block,
    },

    /// Expression statement: expr; or expr (tail position)
    Expr {
        expr: Expr,
        /// Whether the statement has a semicolon
        has_semi: bool,
    },

    /// Item declaration (function, type, etc.) within a block
    Item(Item),

    /// Defer statement for RAII cleanup: defer expr;
    Defer(Expr),

    /// Errdefer statement: errdefer expr;
    /// Only executes when the scope exits via an error path (error return or panic).
    /// Unlike `defer` which always runs on scope exit, `errdefer` is conditional:
    /// it runs only when the enclosing scope exits due to an error (via `?` propagation,
    /// explicit `return Err(...)`, or panic). This is used for cleanup that should only
    /// happen on failure, e.g., `errdefer conn.abort()` to roll back a transaction.
    Errdefer(Expr),

    /// Provide statement for context injection: provide ContextName = expr;
    /// Supports alias syntax: provide ContextName as alias = expr;
    /// Installs a context provider into the current task-local context environment (theta).
    /// The provider is lexically scoped and available to all `using [ContextName]` functions
    /// called within this scope. Aliases enable multiple instances of the same context type.
    Provide {
        context: Text,
        /// Optional alias for the context (enables multiple instances of same context type)
        alias: Maybe<Text>,
        value: Heap<Expr>,
    },

    /// Block-scoped provide statement: provide ContextName = expr in { block }
    /// Supports alias syntax: provide ContextName as alias = expr in { block }
    /// Like `Provide`, but the context is only available within the specified block.
    /// After the block exits, the previous context (if any) is restored.
    ProvideScope {
        context: Text,
        /// Optional alias for the context
        alias: Maybe<Text>,
        value: Heap<Expr>,
        block: Heap<Expr>,
    },

    /// Empty statement (just a semicolon)
    Empty,
}

// Re-export Item from decl.rs to avoid duplication
pub use crate::decl::Item;
