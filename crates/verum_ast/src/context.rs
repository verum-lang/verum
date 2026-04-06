//! Context system types for Verum's dependency injection.
//!
//! This module defines types for the context system, which provides
//! compile-time verified dependency injection.
//!
//! # Context System Overview
//!
//! Verum provides a two-level context model for dependency injection:
//! - Level 1 (Static): `@injectable`/`@inject` for compile-time/startup resolution (0ns overhead)
//! - Level 2 (Dynamic): `provide`/`using` keywords for runtime-varying dependencies (~5-30ns overhead)
//!
//! Contexts are NOT types -- they are declared with `context Name { }` syntax.
//! Functions declare required contexts after the return type: `fn foo() -> T using [Ctx]`.
//! Context groups bundle multiple contexts: `using WebContext = [Database, Logger, Auth]`.
//! All contexts must be explicitly provided with `provide` statements in lexical scope.
//!
//! This is dependency injection, NOT algebraic effects. Context environment (theta) is stored
//! in task-local storage and inherited on spawn.
//!
//! # Context Requirements
//!
//! Context requirements specify what contexts a function needs:
//!
//! ```verum
//! fn query() using [Database, Logger] -> Data { ... }
//! fn pure_fn() using [!IO, !State<_>] -> Int { ... }  // Negative contexts
//! ```

use crate::expr::Expr;
use crate::span::{Span, Spanned};
use crate::ty::{Ident, Path, Type};
use serde::{Deserialize, Serialize};
use verum_common::{Heap, List, Maybe};

/// A context requirement in a function signature or type.
///
/// # Variants
///
/// Context requirements can be:
/// - Simple: `Database`
/// - With type args: `Cache<User>`
/// - Negative: `!Database`
/// - Aliased: `Database as db`
/// - Named: `db: Database`
/// - Conditional: `Analytics if cfg.enabled`
/// - Transformed: `Database.transactional()`
///
/// Context requirements can be simple (`Database`), parameterized (`Cache<User>`),
/// negative (`!Database` -- asserts absence), aliased (`Database as db`),
/// named (`db: Database`), conditional (`Analytics if cfg.enabled`), or
/// transformed (`Database.transactional()`). Contexts are declared with `using [...]`
/// after the return type and provided with `provide Context = expr` in lexical scope.
/// Resolution is via task-local storage (theta) with ~5-30ns lookup overhead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextRequirement {
    pub path: Path,
    pub args: List<Type>,
    /// Whether this is a negative context (`!Database`)
    pub is_negative: bool,
    /// Optional alias (`Database as db`)
    pub alias: Maybe<Ident>,
    /// Optional name binding (`db: Database`)
    pub name: Maybe<Ident>,
    /// Compile-time condition (`if cfg.enabled`)
    pub condition: Maybe<Heap<Expr>>,
    /// Context transforms (`.transactional()`)
    pub transforms: List<ContextTransform>,
    pub span: Span,
}

impl ContextRequirement {
    /// Create a simple context requirement (backward compatible)
    pub fn simple(path: Path, args: List<Type>, span: Span) -> Self {
        Self {
            path,
            args,
            is_negative: false,
            alias: Maybe::None,
            name: Maybe::None,
            condition: Maybe::None,
            transforms: List::new(),
            span,
        }
    }

    /// Create a negative context requirement (`!Database`)
    pub fn negative(path: Path, span: Span) -> Self {
        Self {
            path,
            args: List::new(),
            is_negative: true,
            alias: Maybe::None,
            name: Maybe::None,
            condition: Maybe::None,
            transforms: List::new(),
            span,
        }
    }

    /// Create a context requirement with alias (`Database as db`)
    pub fn with_alias(path: Path, alias: Ident, span: Span) -> Self {
        Self {
            path,
            args: List::new(),
            is_negative: false,
            alias: Maybe::Some(alias),
            name: Maybe::None,
            condition: Maybe::None,
            transforms: List::new(),
            span,
        }
    }
}

impl Spanned for ContextRequirement {
    fn span(&self) -> Span {
        self.span
    }
}

/// A transform applied to a context (e.g., `.transactional()`)
///
/// # Examples
///
/// ```verum
/// fn query() using [Database.transactional()] -> Data { ... }
/// fn cached() using [Cache.scoped("user")] -> User { ... }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextTransform {
    pub name: Ident,
    pub args: List<Expr>,
    pub span: Span,
}

impl Spanned for ContextTransform {
    fn span(&self) -> Span {
        self.span
    }
}

/// A list of context requirements, used in function types and declarations.
///
/// This is a thin wrapper around `List<ContextRequirement>` that provides
/// convenient constructors and utility methods.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ContextList {
    pub requirements: List<ContextRequirement>,
}

impl ContextList {
    /// Create an empty context list
    pub fn empty() -> Self {
        Self {
            requirements: List::new(),
        }
    }

    /// Create a context list from requirements
    pub fn new(requirements: List<ContextRequirement>) -> Self {
        Self { requirements }
    }

    /// Check if the context list is empty
    pub fn is_empty(&self) -> bool {
        self.requirements.is_empty()
    }

    /// Get the number of context requirements
    pub fn len(&self) -> usize {
        self.requirements.len()
    }

    /// Iterate over context requirements
    pub fn iter(&self) -> impl Iterator<Item = &ContextRequirement> {
        self.requirements.iter()
    }
}

impl From<List<ContextRequirement>> for ContextList {
    fn from(requirements: List<ContextRequirement>) -> Self {
        Self { requirements }
    }
}

impl From<Vec<ContextRequirement>> for ContextList {
    fn from(requirements: Vec<ContextRequirement>) -> Self {
        Self {
            requirements: requirements.into(),
        }
    }
}
