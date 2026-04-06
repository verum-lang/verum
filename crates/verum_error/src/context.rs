//! Error Context Chain Management
//!
//! Provides **zero-cost error context chains** that preserve the full error history
//! while adding contextual information at each level of the call stack. This helps
//! developers understand what the code was doing when an error occurred.
//!
//! # Core Concept
//!
//! Errors don't occur in isolation - they happen in a context. By adding context
//! at each level, we create a breadcrumb trail that helps debugging:
//!
//! ```text
//! Level 1 (bottom): "connection refused"
//! Level 2:          "failed to connect to database"
//! Level 3:          "failed to load user profile"
//! Level 4 (top):    "failed to render home page"
//! ```
//!
//! # Key Features
//!
//! - **Zero-cost on success path** - Context closures only execute on error
//! - **Automatic chain preservation** - Contexts are preserved through `?` operator
//! - **Lazy evaluation** - Context values computed only on error
//! - **Type safety** - Context operations are type-checked and composable
//! - **Nested contexts** - Arbitrary depth of contextual information
//! - **Easy integration** - Works naturally with `?` operator
//!
//! # Example Flow
//!
//! ```rust,ignore
//! fn render_page() -> Result<(), ContextError<VerumError>> {
//!     load_user().context("rendering home page")?;
//!     //          ↓
//!     //    added context here
//!     Ok(())
//! }
//!
//! fn load_user() -> Result<User> {
//!     fetch_from_db().context("loading user from database")?;
//!     //              ↓
//!     //        added context here
//!     Ok(user)
//! }
//!
//! fn fetch_from_db() -> Result<User> {
//!     connect_to_db().context("connecting to database")?;
//!     //              ↓
//!     //        added context here
//!     Ok(user)
//! }
//!
//! fn connect_to_db() -> Result<Connection> {
//!     Err(VerumError::new("connection refused", ErrorKind::Network))
//! }
//! ```
//!
//! Result when error reaches top:
//! ```text
//! Error chain:
//! - "connection refused" (original error)
//! - "connecting to database" (context from fetch_from_db)
//! - "loading user from database" (context from load_user)
//! - "rendering home page" (context from render_page)
//! ```

use crate::structured_context::{ContextValue, ToContextValue};
use crate::{ErrorKind, Result, VerumError};
use std::fmt;
use verum_common::{List, Map, Text};

/// Error with context chain
///
/// Wraps an error with additional contextual information, allowing errors
/// to accumulate context as they propagate up the call stack.
///
/// Supports both text contexts (human-readable breadcrumbs) and structured
/// contexts (machine-readable key-value pairs) for rich diagnostics.
///
/// # Examples
///
/// ```rust
/// use verum_error::{VerumError, ErrorKind};
/// use verum_error::context::{ErrorContext, ContextError};
///
/// fn inner() -> Result<(), VerumError> {
///     Err(VerumError::io("connection refused"))
/// }
///
/// fn outer() -> Result<(), ContextError<VerumError>> {
///     inner().context("Failed to connect to database")?;
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct ContextError<E> {
    /// The underlying error
    error: E,
    /// Context message
    context: Text,
    /// Optional source context (for chaining)
    source: Option<Box<ContextError<E>>>,
    /// Structured context data (key-value pairs)
    structured_contexts: Map<Text, ContextValue>,
}

impl<E> ContextError<E> {
    /// Create a new context error
    pub fn new(error: E, context: impl Into<Text>) -> Self {
        Self {
            error,
            context: context.into(),
            source: None,
            structured_contexts: Map::new(),
        }
    }

    /// Add another layer of context
    pub fn with_context(mut self, context: impl Into<Text>) -> Self
    where
        E: Clone,
    {
        let error_clone1 = self.error.clone();
        let error_clone2 = self.error.clone();
        let old_context = self.context.clone();
        let old_source = self.source.take();
        let old_structured = self.structured_contexts.clone();

        Self {
            error: error_clone1,
            context: context.into(),
            source: Some(Box::new(ContextError {
                error: error_clone2,
                context: old_context,
                source: old_source,
                structured_contexts: old_structured,
            })),
            structured_contexts: Map::new(),
        }
    }

    /// Add a single structured context key-value pair
    ///
    /// This allows adding machine-readable data to the error for logging
    /// and monitoring systems.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_error::{VerumError, ErrorKind};
    /// use verum_error::context::ContextError;
    ///
    /// let error = VerumError::new("Connection failed", ErrorKind::Network);
    /// let ctx_error = ContextError::new(error, "Failed to connect")
    ///     .add_structured("user_id", 12345)
    ///     .add_structured("retry_count", 3);
    /// ```
    pub fn add_structured<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<Text>,
        V: ToContextValue,
    {
        self.structured_contexts
            .insert(key.into(), value.to_context_value());
        self
    }

    /// Add multiple structured context key-value pairs from a map
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_common::{Map, Text};
    /// use verum_error::{VerumError, ErrorKind};
    /// use verum_error::context::ContextError;
    /// use verum_error::structured_context::{ContextValue, ToContextValue};
    ///
    /// let error = VerumError::new("Error", ErrorKind::Other);
    /// let mut map: Map<Text, ContextValue> = Map::new();
    /// map.insert("key1".to_string().into(), 42.to_context_value());
    /// map.insert("key2".to_string().into(), "value".to_context_value());
    ///
    /// let ctx_error = ContextError::new(error, "Context")
    ///     .add_structured_map(map);
    /// ```
    pub fn add_structured_map(mut self, map: Map<Text, ContextValue>) -> Self {
        for (k, v) in map {
            self.structured_contexts.insert(k, v);
        }
        self
    }

    /// Get the underlying error
    pub fn error(&self) -> &E {
        &self.error
    }

    /// Get the context message
    pub fn context(&self) -> &Text {
        &self.context
    }

    /// Get the source context (if any)
    pub fn source_context(&self) -> Option<&ContextError<E>> {
        self.source.as_ref().map(|b| b.as_ref())
    }

    /// Iterate over all contexts in the chain
    pub fn contexts(&self) -> ContextIterator<'_, E> {
        ContextIterator {
            current: Some(self),
        }
    }

    /// Get the full context chain as a list
    pub fn context_chain(&self) -> List<&Text> {
        self.contexts().map(|ctx| &ctx.context).collect()
    }

    /// Get all structured contexts
    ///
    /// Returns a reference to the map of structured context data.
    pub fn structured_contexts(&self) -> &Map<Text, ContextValue> {
        &self.structured_contexts
    }

    /// Get a specific structured context value by key
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_error::{VerumError, ErrorKind};
    /// use verum_error::context::ContextError;
    ///
    /// let error = VerumError::new("Error", ErrorKind::Other);
    /// let ctx_error = ContextError::new(error, "Context")
    ///     .add_structured("user_id", 12345);
    ///
    /// assert!(ctx_error.get_structured_context("user_id").is_some());
    /// assert_eq!(ctx_error.get_structured_context("user_id").unwrap().as_int(), Some(12345));
    /// ```
    pub fn get_structured_context(&self, key: &str) -> Option<&ContextValue> {
        self.structured_contexts.get(&Text::from(key))
    }

    /// Get all structured contexts from all layers of the context chain
    ///
    /// This merges structured contexts from all layers, with values from
    /// outer layers taking precedence over inner layers when keys conflict.
    pub fn all_structured_contexts(&self) -> Map<Text, ContextValue> {
        let mut result = Map::new();

        // Collect from innermost to outermost (reversed iteration)
        let layers: Vec<&ContextError<E>> = self.contexts().collect();
        for layer in layers.iter().rev() {
            for (k, v) in &layer.structured_contexts {
                result.insert(k.clone(), v.clone());
            }
        }

        result
    }
}

/// Iterator over context chain
pub struct ContextIterator<'a, E> {
    current: Option<&'a ContextError<E>>,
}

impl<'a, E> Iterator for ContextIterator<'a, E> {
    type Item = &'a ContextError<E>;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current?;
        self.current = current.source_context();
        Some(current)
    }
}

impl<E: fmt::Display> fmt::Display for ContextError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Error: {}", self.context)?;

        // Show all context layers
        for ctx in self.contexts().skip(1) {
            writeln!(f, "Caused by: {}", ctx.context)?;
        }

        // Show the root error
        write!(f, "Caused by: {}", self.error)
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ContextError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error as &(dyn std::error::Error + 'static))
    }
}

/// Extension trait for adding context to Results
///
/// Provides ergonomic methods for adding context to error values:
/// - `context()` - Static context string
/// - `with_context()` - Lazy context via closure (zero-cost on success)
pub trait ErrorContext<T, E> {
    /// Add static context to an error
    ///
    /// # Performance
    /// The context string is always evaluated, even on success.
    /// Use `with_context()` for expensive formatting.
    fn context(self, context: impl Into<Text>) -> Result<T, ContextError<E>>;

    /// Add lazy context to an error
    ///
    /// # Performance
    /// **Zero-cost on success path** - The closure is only called if there's an error.
    /// This is the preferred method for expensive formatting operations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_common::Text;
    /// use verum_error::{VerumError, ErrorKind};
    /// use verum_error::context::ErrorContext;
    ///
    /// fn load_data(id: u64) -> Result<Text, verum_error::context::ContextError<VerumError>> {
    ///     Err(VerumError::new("Database error", ErrorKind::Database))
    ///         .with_context(|| format!("Failed to load data {}", id).into())
    /// }
    /// ```
    fn with_context<F>(self, context: F) -> Result<T, ContextError<E>>
    where
        F: FnOnce() -> Text;
}

impl<T, E> ErrorContext<T, E> for Result<T, E> {
    fn context(self, context: impl Into<Text>) -> Result<T, ContextError<E>> {
        self.map_err(|e| ContextError::new(e, context))
    }

    fn with_context<F>(self, context: F) -> Result<T, ContextError<E>>
    where
        F: FnOnce() -> Text,
    {
        self.map_err(|e| ContextError::new(e, context()))
    }
}

/// Conversion from ContextError<E> to VerumError
///
/// This allows ContextError chains to be converted to the unified VerumError type
/// while preserving the context information in the error message.
impl<E: fmt::Display> From<ContextError<E>> for VerumError {
    fn from(ctx_err: ContextError<E>) -> Self {
        // Build the full error message with context chain
        let mut message = Text::new();
        message.push_str(ctx_err.context().as_str());

        for ctx in ctx_err.contexts().skip(1) {
            message.push_str("\n  Caused by: ");
            message.push_str(ctx.context().as_str());
        }

        message.push_str("\n  Caused by: ");
        message.push_str(&ctx_err.error().to_string());

        VerumError::new(message, ErrorKind::Other)
    }
}

/// Specialized context errors for different subsystems
///
/// These provide domain-specific context with appropriate error kinds.
/// Lexer context error
pub type LexContextError = ContextError<VerumError>;

/// Parser context error
pub type ParseContextError = ContextError<VerumError>;

/// Codegen context error
pub type CodegenContextError = ContextError<VerumError>;

/// Runtime context error
pub type RuntimeContextError = ContextError<VerumError>;
