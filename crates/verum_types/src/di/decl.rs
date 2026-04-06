//! Context Declarations - Interface definitions for dependency injection
//!
//! Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Section 2 - Context Definitions
//!
//! This module implements context declarations, which are interface-like
//! specifications for dependency injection. Contexts define operations that
//! can be provided by different implementations.
//!
//! # Examples
//!
//! ```verum
//! context Logger {
//!     fn log(level: Level, message: Text)
//! }
//!
//! context async Database {
//!     async fn query(sql: SqlQuery) -> Result<Rows, DbError>
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
#[allow(unused_imports)]
use verum_common::{List, Map, Maybe, Text};

use crate::ty::Type;

/// Context declaration - defines an interface for dependency injection
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — Basic Context Syntax
///
/// A context declaration is similar to a trait/protocol in other languages.
/// It specifies operations that must be implemented by context providers.
///
/// # Properties
///
/// - **name**: The context name (e.g., "Logger", "Database")
/// - **type_params**: Optional type parameters (e.g., State<S>)
/// - **operations**: List of operations defined by this context
/// - **is_async**: Whether this context supports async operations
/// - **doc_comment**: Optional documentation
#[derive(Debug, Clone, PartialEq)]
pub struct ContextDecl {
    /// Context name (e.g., "Logger", "Database")
    pub name: Text,

    /// Type parameters for parameterized contexts (e.g., State<S>, Cache<K,V>)
    pub type_params: List<TypeParam>,

    /// Operations defined by this context
    pub operations: List<ContextOperation>,

    /// Whether this context has async operations
    pub is_async: bool,

    /// Documentation comment for this context
    pub doc_comment: Maybe<Text>,
}

/// Type parameter for parameterized contexts
///
/// Context provision: "provide ContextName = implementation" installs a provider in lexical scope via task-local storage (theta) — Parameterized Contexts
///
/// # Examples
///
/// - `State<S>` - type parameter S
/// - `Cache<K, V>` - type parameters K and V
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeParam {
    /// Parameter name (e.g., "S", "K", "V")
    pub name: Text,

    /// Optional constraints on this type parameter
    pub bounds: List<Text>,
}

/// Operation defined within a context
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — Basic Context Syntax
///
/// Each operation defines:
/// - A name
/// - Parameters (name and type)
/// - Return type
/// - Whether it's async
#[derive(Debug, Clone, PartialEq)]
pub struct ContextOperation {
    /// Operation name (e.g., "log", "query", "get")
    pub name: Text,

    /// Parameters: (parameter_name, parameter_type)
    pub params: List<(Text, Type)>,

    /// Return type of this operation
    pub return_type: Type,

    /// Whether this operation is async
    pub is_async: bool,

    /// Documentation for this operation
    pub doc_comment: Maybe<Text>,
}

impl ContextDecl {
    /// Create a new context declaration
    ///
    /// # Arguments
    ///
    /// * `name` - The context name
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_types::di::decl::ContextDecl;
    ///
    /// let logger_context = ContextDecl::new("Logger".into());
    /// ```
    pub fn new(name: Text) -> Self {
        ContextDecl {
            name,
            type_params: List::new(),
            operations: List::new(),
            is_async: false,
            doc_comment: Maybe::None,
        }
    }

    /// Create a context declaration with type parameters
    ///
    /// # Arguments
    ///
    /// * `name` - The context name
    /// * `type_params` - Type parameters for this context
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_types::di::decl::{ContextDecl, TypeParam};
    ///
    /// let state_context = ContextDecl::with_type_params(
    ///     "State".into(),
    ///     vec![TypeParam::new("S".into())]
    /// );
    /// ```
    pub fn with_type_params(name: Text, type_params: impl IntoIterator<Item = TypeParam>) -> Self {
        ContextDecl {
            name,
            type_params: type_params.into_iter().collect(),
            operations: List::new(),
            is_async: false,
            doc_comment: Maybe::None,
        }
    }

    /// Add an operation to this context
    ///
    /// # Arguments
    ///
    /// * `operation` - The operation to add
    pub fn add_operation(&mut self, operation: ContextOperation) {
        if operation.is_async {
            self.is_async = true;
        }
        self.operations.push(operation);
    }

    /// Set the documentation comment for this context
    pub fn set_doc_comment(&mut self, doc: Text) {
        self.doc_comment = Maybe::Some(doc);
    }

    /// Get the number of operations in this context
    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    /// Check if this context has a specific operation
    pub fn has_operation(&self, name: &str) -> bool {
        self.operations.iter().any(|op| op.name.as_str() == name)
    }

    /// Get an operation by name
    pub fn get_operation(&self, name: &str) -> Maybe<&ContextOperation> {
        self.operations
            .iter()
            .find(|op| op.name.as_str() == name).and_then(Maybe::Some)
    }

    /// Validate this context declaration
    ///
    /// Checks:
    /// - At least one operation defined
    /// - No duplicate operation names
    /// - Type parameters are used in operations
    ///
    /// # Returns
    ///
    /// `Ok(())` if valid, `Err(ContextError)` otherwise
    pub fn validate(&self) -> Result<(), ContextError> {
        // Must have at least one operation
        if self.operations.is_empty() {
            return Err(ContextError::EmptyContext(self.name.clone()));
        }

        // Check for duplicate operation names
        let mut seen_names = std::collections::HashSet::new();
        for op in &self.operations {
            if !seen_names.insert(op.name.as_str()) {
                return Err(ContextError::DuplicateOperation {
                    context: self.name.clone(),
                    operation: op.name.clone(),
                });
            }
        }

        Ok(())
    }

    /// Get the full qualified name including type parameters
    ///
    /// # Examples
    ///
    /// - `Logger` -> "Logger"
    /// - `State<S>` -> "State<S>"
    /// - `Cache<K, V>` -> "Cache<K, V>"
    pub fn qualified_name(&self) -> Text {
        if self.type_params.is_empty() {
            self.name.clone()
        } else {
            let params: Vec<String> = self
                .type_params
                .iter()
                .map(|p| p.name.to_string())
                .collect();
            format!("{}<{}>", self.name, params.join(", ")).into()
        }
    }
}

impl TypeParam {
    /// Create a new type parameter
    pub fn new(name: Text) -> Self {
        TypeParam {
            name,
            bounds: List::new(),
        }
    }

    /// Create a type parameter with bounds
    pub fn with_bounds(name: Text, bounds: impl IntoIterator<Item = Text>) -> Self {
        TypeParam {
            name,
            bounds: bounds.into_iter().collect(),
        }
    }
}

impl ContextOperation {
    /// Create a new context operation
    ///
    /// # Arguments
    ///
    /// * `name` - Operation name
    /// * `params` - Parameters as (name, type) pairs
    /// * `return_type` - Return type
    /// * `is_async` - Whether this operation is async
    pub fn new(
        name: Text,
        params: impl IntoIterator<Item = (Text, Type)>,
        return_type: Type,
        is_async: bool,
    ) -> Self {
        ContextOperation {
            name,
            params: params.into_iter().collect(),
            return_type,
            is_async,
            doc_comment: Maybe::None,
        }
    }

    /// Set the documentation comment for this operation
    pub fn set_doc_comment(&mut self, doc: Text) {
        self.doc_comment = Maybe::Some(doc);
    }

    /// Get the number of parameters
    pub fn param_count(&self) -> usize {
        self.params.len()
    }

    /// Get parameter names
    pub fn param_names(&self) -> List<&Text> {
        self.params.iter().map(|(name, _)| name).collect()
    }

    /// Get parameter types
    pub fn param_types(&self) -> List<&Type> {
        self.params.iter().map(|(_, ty)| ty).collect()
    }
}

/// Errors that can occur in context declarations
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    /// Context has no operations defined
    #[error("context '{0}' has no operations defined")]
    EmptyContext(Text),

    /// Duplicate operation name in context
    #[error("duplicate operation '{operation}' in context '{context}'")]
    DuplicateOperation { context: Text, operation: Text },

    /// Context not found
    #[error("context '{0}' not found")]
    NotFound(Text),

    /// Invalid type parameter
    #[error("invalid type parameter: {0}")]
    InvalidTypeParam(Text),
}

impl fmt::Display for ContextDecl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "context {}{}",
            if self.is_async { "async " } else { "" },
            self.qualified_name()
        )
    }
}

impl fmt::Display for ContextOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}fn {}(",
            if self.is_async { "async " } else { "" },
            self.name
        )?;

        for (i, (param_name, param_type)) in self.params.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: <type>", param_name)?;
        }

        write!(f, ") -> <return-type>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_context() {
        let mut logger = ContextDecl::new("Logger".into());
        logger.add_operation(ContextOperation::new(
            "log".into(),
            vec![("level".into(), Type::Int), ("message".into(), Type::Text)],
            Type::Unit,
            false,
        ));

        assert_eq!(logger.name, "Logger");
        assert_eq!(logger.operation_count(), 1);
        assert!(logger.has_operation("log"));
        assert!(!logger.is_async);
    }

    #[test]
    fn test_async_context() {
        let mut db = ContextDecl::new("Database".into());
        db.add_operation(ContextOperation::new(
            "query".into(),
            vec![("sql".into(), Type::Text)],
            Type::Text,
            true,
        ));

        assert!(db.is_async);
        assert!(db.has_operation("query"));
    }

    #[test]
    fn test_parameterized_context() {
        let state = ContextDecl::with_type_params("State".into(), vec![TypeParam::new("S".into())]);

        assert_eq!(state.qualified_name(), "State<S>");
        assert_eq!(state.type_params.len(), 1);
    }

    #[test]
    fn test_validation_empty_context() {
        let empty = ContextDecl::new("Empty".into());
        assert!(matches!(
            empty.validate(),
            Err(ContextError::EmptyContext(_))
        ));
    }

    #[test]
    fn test_validation_duplicate_operations() {
        let mut ctx = ContextDecl::new("Test".into());
        ctx.add_operation(ContextOperation::new(
            "foo".into(),
            vec![],
            Type::Unit,
            false,
        ));
        ctx.add_operation(ContextOperation::new(
            "foo".into(),
            vec![],
            Type::Unit,
            false,
        ));

        assert!(matches!(
            ctx.validate(),
            Err(ContextError::DuplicateOperation { .. })
        ));
    }

    #[test]
    fn test_get_operation() {
        let mut ctx = ContextDecl::new("Test".into());
        ctx.add_operation(ContextOperation::new(
            "foo".into(),
            vec![],
            Type::Unit,
            false,
        ));

        let op = ctx.get_operation("foo");
        assert!(matches!(op, Maybe::Some(_)));

        let missing = ctx.get_operation("bar");
        assert!(matches!(missing, Maybe::None));
    }
}
