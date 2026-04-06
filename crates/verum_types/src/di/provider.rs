//! Context Providers - Concrete implementations for dependency injection
//!
//! Context groups: reusable sets defined as "using GroupName = [Ctx1, Ctx2, ...]", composable with other contexts — Provider Interface Definition
//! Context resolution: resolving context names to declarations, expanding groups, checking provision — .2 - The `provide` Keyword
//!
//! This module implements context providers, which bind concrete implementations
//! to context interfaces. Providers are installed using the `provide` keyword.
//!
//! # Examples
//!
//! ```verum
//! provide Logger = console_logger();
//! provide Database = postgres_connection().await;
//! ```

use serde::{Deserialize, Serialize};
use std::any::TypeId;
use std::fmt;
use verum_ast::expr::Expr;
use verum_ast::span::Span;
#[allow(unused_imports)]
use verum_common::{Maybe, Text};

use super::requirement::ContextRef;

/// Provider expression - the AST node or source text for the provider
///
/// Context resolution: resolving context names to declarations, expanding groups, checking provision — .2 - The `provide` Keyword
///
/// This enum allows storing provider expressions either as:
/// - An actual AST `Expr` node for full semantic analysis
/// - Source text for backward compatibility and simple cases
///
/// Using the AST node enables:
/// - Type checking of provider expressions
/// - Semantic analysis and validation
/// - Code generation from the actual expression
/// - Better error reporting with source locations
#[derive(Debug, Clone, PartialEq)]
pub enum ProviderExpr {
    /// Full AST expression node for semantic analysis
    ///
    /// This is the preferred representation, enabling:
    /// - Type inference on the provider expression
    /// - Validation that the expression produces the correct type
    /// - Code generation with proper CBGR handling
    Ast(Expr),

    /// Source text representation for backward compatibility
    ///
    /// Used when:
    /// - Parsing hasn't completed yet
    /// - The expression is simple and doesn't need full analysis
    /// - For serialization/deserialization (Expr doesn't impl Serialize)
    Source {
        /// The source text of the provider expression
        text: Text,
        /// Source location for error reporting
        span: Span,
    },
}

impl ProviderExpr {
    /// Create a new AST-based provider expression
    pub fn from_ast(expr: Expr) -> Self {
        ProviderExpr::Ast(expr)
    }

    /// Create a new source-based provider expression
    pub fn from_source(text: impl Into<Text>, span: Span) -> Self {
        ProviderExpr::Source {
            text: text.into(),
            span,
        }
    }

    /// Get the source text representation
    pub fn as_text(&self) -> Text {
        match self {
            ProviderExpr::Ast(expr) => {
                // Format the expression as source text
                format!("{:?}", expr.kind).into()
            }
            ProviderExpr::Source { text, .. } => text.clone(),
        }
    }

    /// Get the span for error reporting
    pub fn span(&self) -> Span {
        match self {
            ProviderExpr::Ast(expr) => expr.span,
            ProviderExpr::Source { span, .. } => *span,
        }
    }

    /// Check if this is an AST expression
    pub fn is_ast(&self) -> bool {
        matches!(self, ProviderExpr::Ast(_))
    }

    /// Get the AST expression if available
    pub fn as_ast(&self) -> Option<&Expr> {
        match self {
            ProviderExpr::Ast(expr) => Some(expr),
            ProviderExpr::Source { .. } => None,
        }
    }

    /// Check if the provider expression is empty
    pub fn is_empty(&self) -> bool {
        match self {
            ProviderExpr::Ast(_) => false,
            ProviderExpr::Source { text, .. } => text.is_empty(),
        }
    }
}

impl fmt::Display for ProviderExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_text())
    }
}

// Implement Serialize/Deserialize manually since Expr doesn't support them
impl Serialize for ProviderExpr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as source text
        self.as_text().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProviderExpr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Deserialize as source text
        let text = Text::deserialize(deserializer)?;
        Ok(ProviderExpr::Source {
            text,
            span: Span::default(),
        })
    }
}

impl Eq for ProviderExpr {}

/// Context provider binding
///
/// Context resolution: resolving context names to declarations, expanding groups, checking provision — .2 - The `provide` Keyword
///
/// A context provider binds a concrete implementation to a context interface.
/// The provider expression is evaluated when the `provide` statement executes,
/// and the result is stored in the context environment.
///
/// # Properties
///
/// - **context**: The context being provided
/// - **provider_expr**: Expression that creates the provider (AST node or source)
/// - **scope**: Where this provider is active (local, module, global)
/// - **type_id**: Runtime type identifier for the provider value
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextProvider {
    /// The context being provided
    pub context: ContextRef,

    /// Provider expression - the AST node or source text
    ///
    /// This can be either:
    /// - `ProviderExpr::Ast(expr)` - Full AST for semantic analysis
    /// - `ProviderExpr::Source { text, span }` - Source text for simple cases
    ///
    /// Using an AST node enables type checking, semantic analysis,
    /// and proper code generation.
    pub provider_expr: ProviderExpr,

    /// Scope of this provider
    pub scope: ProviderScope,

    /// Runtime type ID for this provider
    #[serde(skip, default = "default_type_id")]
    pub type_id: TypeId,

    /// Whether this provider is async (requires .await)
    pub is_async: bool,
}

/// Default TypeId for deserialization (uses unit type as placeholder)
fn default_type_id() -> TypeId {
    TypeId::of::<()>()
}

/// Scope where a context provider is active
///
/// Context resolution: resolving context names to declarations, expanding groups, checking provision — .2 - Scoped Context Providers
///
/// Providers can be scoped at different levels:
/// - **Local**: Function or block scope
/// - **Module**: Module-wide (@using attribute)
/// - **Global**: Application-wide (rare, for system services)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderScope {
    /// Local scope - function or block level
    /// Provider is active within the current lexical scope
    Local,

    /// Module scope - module-wide default
    /// Provider is active for all functions in the module
    /// Declared with @using([Context]) attribute
    Module,

    /// Global scope - application-wide
    /// Provider is active for the entire application
    /// Used for system-level services (Runtime, Net, etc.)
    Global,
}

impl ContextProvider {
    /// Create a new local-scope context provider from source text
    ///
    /// # Arguments
    ///
    /// * `context` - The context being provided
    /// * `provider_expr` - Source text expression that creates the provider
    /// * `type_id` - Runtime type ID for the provider value
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::provider::ContextProvider;
    /// use verum_types::di::requirement::ContextRef;
    /// use std::any::TypeId;
    /// # struct ConsoleLogger;
    /// # let logger_ctx = ContextRef::new("Logger".into(), TypeId::of::<()>());
    ///
    /// let provider = ContextProvider::new(
    ///     logger_ctx,
    ///     "console_logger()".into(),
    ///     TypeId::of::<ConsoleLogger>()
    /// );
    /// ```
    pub fn new(context: ContextRef, provider_expr: Text, type_id: TypeId) -> Self {
        ContextProvider {
            context,
            provider_expr: ProviderExpr::from_source(provider_expr, Span::default()),
            scope: ProviderScope::Local,
            type_id,
            is_async: false,
        }
    }

    /// Create a new local-scope context provider from an AST expression
    ///
    /// This is the preferred constructor when you have a parsed AST expression,
    /// as it enables full semantic analysis, type checking, and code generation.
    ///
    /// # Arguments
    ///
    /// * `context` - The context being provided
    /// * `expr` - The AST expression that creates the provider
    /// * `type_id` - Runtime type ID for the provider value
    pub fn from_ast(context: ContextRef, expr: Expr, type_id: TypeId) -> Self {
        ContextProvider {
            context,
            provider_expr: ProviderExpr::Ast(expr),
            scope: ProviderScope::Local,
            type_id,
            is_async: false,
        }
    }

    /// Create a context provider with a specific scope
    ///
    /// # Arguments
    ///
    /// * `context` - The context being provided
    /// * `provider_expr` - Expression that creates the provider
    /// * `type_id` - Runtime type ID for the provider value
    /// * `scope` - The scope for this provider
    pub fn with_scope(
        context: ContextRef,
        provider_expr: Text,
        type_id: TypeId,
        scope: ProviderScope,
    ) -> Self {
        ContextProvider {
            context,
            provider_expr: ProviderExpr::from_source(provider_expr, Span::default()),
            scope,
            type_id,
            is_async: false,
        }
    }

    /// Create a context provider with AST expression and specific scope
    pub fn with_ast_and_scope(
        context: ContextRef,
        expr: Expr,
        type_id: TypeId,
        scope: ProviderScope,
    ) -> Self {
        ContextProvider {
            context,
            provider_expr: ProviderExpr::Ast(expr),
            scope,
            type_id,
            is_async: false,
        }
    }

    /// Mark this provider as async
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::provider::ContextProvider;
    /// use verum_types::di::requirement::ContextRef;
    /// use std::any::TypeId;
    /// # let logger_ctx = ContextRef::new("Logger".into(), TypeId::of::<()>());
    ///
    /// let provider = ContextProvider::new(logger_ctx, "async_logger()".into(), TypeId::of::<()>())
    ///     .as_async();
    /// ```
    pub fn as_async(mut self) -> Self {
        self.is_async = true;
        self
    }

    /// Get the AST expression if available
    ///
    /// Returns `Some(&Expr)` if the provider was created with `from_ast()`,
    /// or `None` if it was created with source text.
    pub fn ast_expr(&self) -> Option<&Expr> {
        self.provider_expr.as_ast()
    }

    /// Get the source text representation of the provider expression
    pub fn expr_text(&self) -> Text {
        self.provider_expr.as_text()
    }

    /// Get the span of the provider expression
    pub fn expr_span(&self) -> Span {
        self.provider_expr.span()
    }

    /// Set the scope for this provider
    pub fn set_scope(&mut self, scope: ProviderScope) {
        self.scope = scope;
    }

    /// Check if this provider is local scope
    pub fn is_local(&self) -> bool {
        self.scope == ProviderScope::Local
    }

    /// Check if this provider is module scope
    pub fn is_module(&self) -> bool {
        self.scope == ProviderScope::Module
    }

    /// Check if this provider is global scope
    pub fn is_global(&self) -> bool {
        self.scope == ProviderScope::Global
    }

    /// Get the context name
    pub fn context_name(&self) -> &Text {
        &self.context.name
    }

    /// Validate this provider
    ///
    /// Checks:
    /// - Provider expression is not empty
    /// - Async context requires async provider
    ///
    /// # Returns
    ///
    /// `Ok(())` if valid, `Err(ProviderError)` otherwise
    pub fn validate(&self) -> Result<(), ProviderError> {
        // Provider expression must not be empty
        if self.provider_expr.is_empty() {
            return Err(ProviderError::EmptyExpression(self.context.name.clone()));
        }

        // Async context requires async provider (or vice versa)
        if self.context.is_async && !self.is_async {
            return Err(ProviderError::AsyncMismatch {
                context: self.context.name.clone(),
                context_async: true,
                provider_async: false,
            });
        }

        Ok(())
    }
}

/// Errors that can occur with context providers
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// Provider expression is empty
    #[error("provider expression for context '{0}' is empty")]
    EmptyExpression(Text),

    /// Async mismatch between context and provider
    #[error(
        "async mismatch for context '{context}': context is {}, provider is {}",
        if *.context_async { "async" } else { "sync" },
        if *.provider_async { "async" } else { "sync" }
    )]
    AsyncMismatch {
        context: Text,
        context_async: bool,
        provider_async: bool,
    },

    /// Provider type mismatch
    #[error("provider type mismatch for context '{0}': expected {1}, got {2}")]
    TypeMismatch(Text, Text, Text),

    /// Context already provided in this scope
    #[error("context '{0}' already provided in this scope")]
    AlreadyProvided(Text),
}

impl fmt::Display for ContextProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "provide {} = {}{}",
            self.context,
            self.provider_expr,
            if self.is_async { ".await" } else { "" }
        )
    }
}

impl fmt::Display for ProviderScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderScope::Local => write!(f, "local"),
            ProviderScope::Module => write!(f, "module"),
            ProviderScope::Global => write!(f, "global"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_provider() {
        let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let provider =
            ContextProvider::new(ctx_ref, "console_logger()".into(), TypeId::of::<String>());

        assert!(provider.is_local());
        assert!(!provider.is_module());
        assert!(!provider.is_global());
        assert!(!provider.is_async);
    }

    #[test]
    fn test_module_provider() {
        let ctx_ref = ContextRef::new("Database".into(), TypeId::of::<()>());
        let provider = ContextProvider::with_scope(
            ctx_ref,
            "postgres_connection()".into(),
            TypeId::of::<String>(),
            ProviderScope::Module,
        );

        assert!(!provider.is_local());
        assert!(provider.is_module());
        assert!(!provider.is_global());
    }

    #[test]
    fn test_global_provider() {
        let ctx_ref = ContextRef::new("Runtime".into(), TypeId::of::<()>());
        let provider = ContextProvider::with_scope(
            ctx_ref,
            "VerumNativeRuntime.new()".into(),
            TypeId::of::<String>(),
            ProviderScope::Global,
        );

        assert!(!provider.is_local());
        assert!(!provider.is_module());
        assert!(provider.is_global());
    }

    #[test]
    fn test_async_provider() {
        let ctx_ref = ContextRef::new("Database".into(), TypeId::of::<()>()).as_async();
        let provider = ContextProvider::new(
            ctx_ref,
            "Database.connect(url)".into(),
            TypeId::of::<String>(),
        )
        .as_async();

        assert!(provider.is_async);
        assert!(provider.context.is_async);
    }

    #[test]
    fn test_validate_empty_expression() {
        let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let provider = ContextProvider::new(ctx_ref, "".into(), TypeId::of::<String>());

        assert!(matches!(
            provider.validate(),
            Err(ProviderError::EmptyExpression(_))
        ));
    }

    #[test]
    fn test_validate_async_mismatch() {
        let ctx_ref = ContextRef::new("Database".into(), TypeId::of::<()>()).as_async();
        let provider =
            ContextProvider::new(ctx_ref, "sync_provider()".into(), TypeId::of::<String>());

        assert!(matches!(
            provider.validate(),
            Err(ProviderError::AsyncMismatch { .. })
        ));
    }

    #[test]
    fn test_validate_success() {
        let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let provider =
            ContextProvider::new(ctx_ref, "console_logger()".into(), TypeId::of::<String>());

        assert!(provider.validate().is_ok());
    }

    #[test]
    fn test_context_name() {
        let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let provider =
            ContextProvider::new(ctx_ref, "console_logger()".into(), TypeId::of::<String>());

        assert_eq!(provider.context_name(), "Logger");
    }

    #[test]
    fn test_set_scope() {
        let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let mut provider =
            ContextProvider::new(ctx_ref, "console_logger()".into(), TypeId::of::<String>());

        assert!(provider.is_local());

        provider.set_scope(ProviderScope::Module);
        assert!(provider.is_module());

        provider.set_scope(ProviderScope::Global);
        assert!(provider.is_global());
    }
}
