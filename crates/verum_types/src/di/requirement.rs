//! Context Requirements - Function dependency declarations
//!
//! Context group expansion: resolving context group names to their constituent contexts recursively — Context Requirements
//!
//! This module implements context requirements, which specify what contexts
//! a function needs to execute. Requirements are declared with `using [Ctx1, Ctx2]`.
//!
//! # Examples
//!
//! ```verum
//! fn process() using [Logger, Database] {
//!     Logger.log(Level.Info, "Processing...");
//!     Database.save(...);
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::any::TypeId;
use std::fmt;
#[allow(unused_imports)]
use verum_common::{List, Map, Maybe, Set, Text};

use super::env::ContextEnv;
use crate::ty::TypeVar;

// =============================================================================
// CONTEXT EXPRESSION (Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.5 - Context Polymorphism)
// =============================================================================

/// Context expression - either a concrete set of contexts or a type variable.
///
/// This enables context polymorphism where higher-order functions can propagate
/// context requirements from callbacks:
///
/// ```verum
/// fn map<T, U, using C>(iter: I, f: fn(T) -> U using C) -> MapIter<T, U> using C
/// ```
///
/// In this example, `C` is a context variable that unifies with the actual context
/// requirements of the callback `f`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextExpr {
    /// Concrete context requirement - a fixed set of contexts
    Concrete(ContextRequirement),

    /// Context type variable - unifies with actual context requirements
    /// Type system improvements: refinement evidence tracking, flow-sensitive propagation, prototype mode — Section 17.2 - Context Polymorphism
    Variable(TypeVar),
}

impl ContextExpr {
    /// Create a concrete context expression from a requirement
    pub fn concrete(req: ContextRequirement) -> Self {
        ContextExpr::Concrete(req)
    }

    /// Create a context variable expression
    pub fn variable(var: TypeVar) -> Self {
        ContextExpr::Variable(var)
    }

    /// Create an empty concrete context expression
    pub fn empty() -> Self {
        ContextExpr::Concrete(ContextRequirement::empty())
    }

    /// Check if this is a variable
    pub fn is_variable(&self) -> bool {
        matches!(self, ContextExpr::Variable(_))
    }

    /// Check if this is a concrete expression
    pub fn is_concrete(&self) -> bool {
        matches!(self, ContextExpr::Concrete(_))
    }

    /// Get the type variable if this is a variable expression
    pub fn as_variable(&self) -> Option<TypeVar> {
        match self {
            ContextExpr::Variable(v) => Some(*v),
            ContextExpr::Concrete(_) => None,
        }
    }

    /// Get the concrete requirement if this is a concrete expression
    pub fn as_concrete(&self) -> Option<&ContextRequirement> {
        match self {
            ContextExpr::Concrete(req) => Some(req),
            ContextExpr::Variable(_) => None,
        }
    }

    /// Check if this expression requires any contexts (false for variables)
    pub fn is_empty(&self) -> bool {
        match self {
            ContextExpr::Concrete(req) => req.is_empty(),
            ContextExpr::Variable(_) => false, // Variables may bind to non-empty requirements
        }
    }

    /// Get the number of required contexts (0 for variables)
    pub fn len(&self) -> usize {
        match self {
            ContextExpr::Concrete(req) => req.len(),
            ContextExpr::Variable(_) => 0,
        }
    }

    /// Iterate over required contexts (empty iterator for variables)
    pub fn iter(&self) -> Box<dyn Iterator<Item = &ContextRef> + '_> {
        match self {
            ContextExpr::Concrete(req) => Box::new(req.iter()),
            ContextExpr::Variable(_) => Box::new(std::iter::empty()),
        }
    }

    /// Get context names (empty for variables)
    pub fn context_names(&self) -> List<&Text> {
        match self {
            ContextExpr::Concrete(req) => req.context_names(),
            ContextExpr::Variable(_) => List::new(),
        }
    }

    /// Check if a specific context is required (always false for variables)
    pub fn requires(&self, name: &str) -> bool {
        match self {
            ContextExpr::Concrete(req) => req.requires(name),
            ContextExpr::Variable(_) => false,
        }
    }

    /// Apply a context substitution, resolving variables to their bound values.
    ///
    /// Follows chains: if v1 -> v2 -> Concrete, resolves transitively.
    pub fn apply_context_subst(&self, subst: &indexmap::IndexMap<crate::ty::TypeVar, ContextExpr>) -> ContextExpr {
        match self {
            ContextExpr::Concrete(_) => self.clone(),
            ContextExpr::Variable(var) => {
                if let Some(bound) = subst.get(var) {
                    // Follow chain transitively
                    bound.apply_context_subst(subst)
                } else {
                    self.clone()
                }
            }
        }
    }

    /// Check if this context expression has async contexts
    pub fn has_async_contexts(&self) -> bool {
        match self {
            ContextExpr::Concrete(req) => req.has_async_contexts(),
            ContextExpr::Variable(_) => false, // Unknown until unified
        }
    }
}

impl Default for ContextExpr {
    fn default() -> Self {
        ContextExpr::empty()
    }
}

impl fmt::Display for ContextExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextExpr::Concrete(req) => write!(f, "{}", req),
            ContextExpr::Variable(var) => write!(f, "{}", var),
        }
    }
}

impl From<ContextRequirement> for ContextExpr {
    fn from(req: ContextRequirement) -> Self {
        ContextExpr::Concrete(req)
    }
}

impl From<TypeVar> for ContextExpr {
    fn from(var: TypeVar) -> Self {
        ContextExpr::Variable(var)
    }
}

// Serialization support for ContextExpr
impl serde::Serialize for ContextExpr {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        match self {
            ContextExpr::Concrete(req) => {
                let mut s = serializer.serialize_struct("ContextExpr", 2)?;
                s.serialize_field("kind", "concrete")?;
                s.serialize_field("value", req)?;
                s.end()
            }
            ContextExpr::Variable(var) => {
                let mut s = serializer.serialize_struct("ContextExpr", 2)?;
                s.serialize_field("kind", "variable")?;
                s.serialize_field("value", &var.id())?;
                s.end()
            }
        }
    }
}

impl<'de> serde::Deserialize<'de> for ContextExpr {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // For simplicity, deserialize as concrete by default
        // Full deserialization would need a custom visitor
        let req: ContextRequirement = serde::Deserialize::deserialize(deserializer)?;
        Ok(ContextExpr::Concrete(req))
    }
}

/// Context requirement - specifies what contexts a function needs
///
/// Context group expansion: resolving context group names to their constituent contexts recursively — Context Requirements
///
/// A context requirement is a set of contexts that must be provided
/// for a function to execute. The requirement is checked at compile-time
/// and satisfied at runtime via the context environment (θ).
///
/// # Examples
///
/// ```no_run
/// use verum_types::di::requirement::{ContextRequirement, ContextRef};
/// # let logger_type_id = std::any::TypeId::of::<()>();
/// # let db_type_id = std::any::TypeId::of::<()>();
///
/// // Function requires Logger and Database
/// let mut req = ContextRequirement::empty();
/// req.add_context(ContextRef::new("Logger".into(), logger_type_id));
/// req.add_context(ContextRef::new("Database".into(), db_type_id));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRequirement {
    /// Set of required contexts
    contexts: Set<ContextRef>,
}

// Custom Serialize/Deserialize for ContextRequirement because Set doesn't implement these traits
impl serde::Serialize for ContextRequirement {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Convert to List for serialization
        let list: List<ContextRef> = self.contexts.iter().cloned().collect();
        list.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for ContextRequirement {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let list: List<ContextRef> = serde::Deserialize::deserialize(deserializer)?;
        let mut contexts = Set::new();
        for ctx in list {
            contexts.insert(ctx);
        }
        Ok(ContextRequirement { contexts })
    }
}

/// Reference to a context in a requirement
///
/// Each context reference includes:
/// - **name**: The context name (e.g., "Logger")
/// - **type_id**: Runtime type identifier for lookup
/// - **type_args**: Optional type arguments (e.g., State<Int>)
///
/// Extended with advanced context patterns (Advanced context patterns (negative contexts, call graph verification, module aliases)):
/// - **alias**: Optional alias name for the context
/// - **is_negative**: Whether this is a negative context (`!Database`)
/// - **transforms**: Applied transforms (`.transactional()`, `.traced()`)
/// - **condition**: Compile-time condition (`if cfg.enabled`)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContextRef {
    /// Context name (e.g., "Logger", "Database")
    pub name: Text,

    /// Runtime type ID for this context
    #[serde(skip, default = "default_type_id")]
    pub type_id: TypeId,

    /// Type arguments for parameterized contexts (e.g., State<Int>)
    pub type_args: List<Text>,

    /// Whether this context is async
    pub is_async: bool,

    // ---- Advanced Context Pattern Fields (Advanced context patterns (negative contexts, call graph verification, module aliases)) ----

    /// Optional alias for the context (`Database as db`)
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
    #[serde(default)]
    pub alias: Maybe<Text>,

    /// Whether this is a negative context (`!Database`)
    /// Negative contexts are excluded and their usage is forbidden
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    #[serde(default)]
    pub is_negative: bool,

    /// Applied context transforms (`.transactional()`, `.traced()`, etc.)
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.3 - Context Transformations
    #[serde(default)]
    pub transforms: List<ContextTransformRef>,

    /// Compile-time condition for conditional contexts
    /// e.g., `if cfg.analytics_enabled` or `if T: Protocol`
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    #[serde(default)]
    pub condition: Maybe<Text>, // Serialized form of the condition expression
}

/// Reference to a context transform (e.g., `.transactional()`)
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.3 - Context Transformations
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContextTransformRef {
    /// Transform name (e.g., "transactional", "traced", "scoped")
    pub name: Text,
    /// Transform arguments (serialized as strings)
    pub args: List<Text>,
}

/// Default TypeId for deserialization (uses unit type as placeholder)
fn default_type_id() -> TypeId {
    TypeId::of::<()>()
}

impl ContextRequirement {
    /// Create an empty context requirement (pure function)
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_types::di::requirement::ContextRequirement;
    ///
    /// let req = ContextRequirement::empty();
    /// assert!(req.is_empty());
    /// ```
    pub fn empty() -> Self {
        ContextRequirement {
            contexts: Set::new(),
        }
    }

    /// Create a context requirement with a single context
    ///
    /// # Arguments
    ///
    /// * `context` - The context reference
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_types::di::requirement::{ContextRequirement, ContextRef};
    /// # let type_id = std::any::TypeId::of::<()>();
    ///
    /// let logger_ref = ContextRef::new("Logger".into(), type_id);
    /// let req = ContextRequirement::single(logger_ref);
    /// assert_eq!(req.len(), 1);
    /// ```
    pub fn single(context: ContextRef) -> Self {
        let mut contexts = Set::new();
        contexts.insert(context);
        ContextRequirement { contexts }
    }

    /// Create a context requirement from multiple contexts
    ///
    /// # Arguments
    ///
    /// * `contexts` - Iterator of context references
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::requirement::{ContextRequirement, ContextRef};
    /// # let logger_ref = ContextRef::new("Logger".into(), std::any::TypeId::of::<()>());
    /// # let db_ref = ContextRef::new("Database".into(), std::any::TypeId::of::<()>());
    ///
    /// let req = ContextRequirement::from_contexts(vec![logger_ref, db_ref]);
    /// assert_eq!(req.len(), 2);
    /// ```
    pub fn from_contexts(contexts: impl IntoIterator<Item = ContextRef>) -> Self {
        ContextRequirement {
            contexts: contexts.into_iter().collect(),
        }
    }

    /// Add a context to this requirement
    ///
    /// # Arguments
    ///
    /// * `context` - The context to add
    pub fn add_context(&mut self, context: ContextRef) {
        self.contexts.insert(context);
    }

    /// Remove a context from this requirement
    ///
    /// # Arguments
    ///
    /// * `name` - The context name to remove
    ///
    /// # Returns
    ///
    /// `true` if the context was removed, `false` if not found
    pub fn remove_context(&mut self, name: &str) -> bool {
        // Find the context first, clone it to avoid borrow issues
        let ctx_to_remove = self
            .contexts
            .iter()
            .find(|c| c.name.as_str() == name)
            .cloned();
        if let Some(ctx) = ctx_to_remove {
            self.contexts.remove(&ctx);
            true
        } else {
            false
        }
    }

    /// Check if this requirement is empty (pure function)
    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }

    /// Get the number of required contexts
    pub fn len(&self) -> usize {
        self.contexts.len()
    }

    /// Check if a specific context is required
    ///
    /// # Arguments
    ///
    /// * `name` - The context name to check
    pub fn requires(&self, name: &str) -> bool {
        self.contexts.iter().any(|c| c.name.as_str() == name)
    }

    /// Get a context reference by name
    ///
    /// # Arguments
    ///
    /// * `name` - The context name
    ///
    /// # Returns
    ///
    /// `Some(&ContextRef)` if found, `None` otherwise
    pub fn get_context(&self, name: &str) -> Maybe<&ContextRef> {
        self.contexts
            .iter()
            .find(|c| c.name.as_str() == name).and_then(Maybe::Some)
    }

    /// Merge this requirement with another
    ///
    /// Returns a new requirement containing all contexts from both.
    ///
    /// # Arguments
    ///
    /// * `other` - The other requirement to merge
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::requirement::{ContextRequirement, ContextRef};
    /// # let logger_ref = ContextRef::new("Logger".into(), std::any::TypeId::of::<()>());
    /// # let db_ref = ContextRef::new("Database".into(), std::any::TypeId::of::<()>());
    ///
    /// let req1 = ContextRequirement::single(logger_ref);
    /// let req2 = ContextRequirement::single(db_ref);
    /// let merged = req1.merge(&req2);
    /// assert_eq!(merged.len(), 2);
    /// ```
    pub fn merge(&self, other: &Self) -> Self {
        let mut contexts = self.contexts.clone();
        for ctx in other.contexts.iter() {
            contexts.insert(ctx.clone());
        }
        ContextRequirement { contexts }
    }

    /// Check if this requirement is satisfied by an environment
    ///
    /// Context resolution: resolving context names to declarations, expanding groups, checking provision — .2 - Context Provision
    ///
    /// # Arguments
    ///
    /// * `env` - The context environment to check
    ///
    /// # Returns
    ///
    /// `true` if all required contexts are available in the environment
    pub fn satisfies(&self, env: &ContextEnv) -> bool {
        self.contexts.iter().all(|ctx| env.has_context(ctx.type_id))
    }

    /// Get missing contexts from an environment
    ///
    /// Returns a list of context names that are required but not provided.
    ///
    /// # Arguments
    ///
    /// * `env` - The context environment to check
    ///
    /// # Returns
    ///
    /// List of missing context names
    pub fn missing_contexts(&self, env: &ContextEnv) -> List<Text> {
        self.contexts
            .iter()
            .filter(|ctx| !env.has_context(ctx.type_id))
            .map(|ctx| ctx.name.clone())
            .collect()
    }

    /// Check if this requirement is a subset of another
    ///
    /// Returns `true` if all contexts in this requirement are also in `other`.
    ///
    /// # Arguments
    ///
    /// * `other` - The other requirement to compare against
    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.contexts.iter().all(|ctx| other.contexts.contains(ctx))
    }

    /// Iterate over required contexts
    pub fn iter(&self) -> impl Iterator<Item = &ContextRef> {
        self.contexts.iter()
    }

    /// Get all context names
    pub fn context_names(&self) -> List<&Text> {
        self.contexts.iter().map(|c| &c.name).collect()
    }

    /// Check if any required context is async
    pub fn has_async_contexts(&self) -> bool {
        self.contexts.iter().any(|c| c.is_async)
    }

    // ========================================================================
    // Advanced Context Pattern Methods (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // ========================================================================

    /// Get all negative contexts (excluded contexts)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    pub fn negative_contexts(&self) -> List<&ContextRef> {
        self.contexts.iter().filter(|c| c.is_negative).collect()
    }

    /// Get all positive contexts (required contexts)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    pub fn positive_contexts(&self) -> List<&ContextRef> {
        self.contexts.iter().filter(|c| !c.is_negative).collect()
    }

    /// Check if a context name is excluded (negative)
    ///
    /// # Arguments
    ///
    /// * `name` - The context name to check
    ///
    /// # Returns
    ///
    /// `true` if the context is explicitly excluded (`!Context`)
    pub fn is_excluded(&self, name: &str) -> bool {
        self.contexts.iter().any(|c| c.is_negative && c.name.as_str() == name)
    }

    /// Get a context by its alias
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias name
    ///
    /// # Returns
    ///
    /// `Some(&ContextRef)` if found, `None` otherwise
    pub fn get_by_alias(&self, alias: &str) -> Maybe<&ContextRef> {
        self.contexts
            .iter()
            .find(|c| c.effective_name().as_str() == alias).and_then(Maybe::Some)
    }

    /// Get all aliased contexts
    ///
    /// Returns contexts that have an alias set (via `as alias` or `name:` syntax)
    pub fn aliased_contexts(&self) -> List<&ContextRef> {
        self.contexts
            .iter()
            .filter(|c| matches!(c.alias, Maybe::Some(_)))
            .collect()
    }

    /// Get all conditional contexts
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    pub fn conditional_contexts(&self) -> List<&ContextRef> {
        self.contexts.iter().filter(|c| c.is_conditional()).collect()
    }

    /// Get all transformed contexts
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.3 - Context Transformations
    pub fn transformed_contexts(&self) -> List<&ContextRef> {
        self.contexts.iter().filter(|c| c.has_transforms()).collect()
    }

    /// Validate that using a context name is allowed
    ///
    /// Returns an error if the context is excluded (negative)
    ///
    /// # Arguments
    ///
    /// * `name` - The context name being used
    ///
    /// # Returns
    ///
    /// `Ok(())` if allowed, `Err(message)` if excluded
    pub fn validate_usage(&self, name: &str) -> std::result::Result<(), Text> {
        if self.is_excluded(name) {
            Err(format!(
                "Context '{}' is explicitly excluded in function signature. Cannot use it.",
                name
            ).into())
        } else {
            Ok(())
        }
    }
}

impl ContextRef {
    /// Create a new context reference
    ///
    /// # Arguments
    ///
    /// * `name` - Context name
    /// * `type_id` - Runtime type ID
    pub fn new(name: Text, type_id: TypeId) -> Self {
        ContextRef {
            name,
            type_id,
            type_args: List::new(),
            is_async: false,
            // Advanced pattern defaults
            alias: Maybe::None,
            is_negative: false,
            transforms: List::new(),
            condition: Maybe::None,
        }
    }

    /// Create a negative context reference (`!Database`)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    pub fn negative(name: Text, type_id: TypeId) -> Self {
        ContextRef {
            name,
            type_id,
            type_args: List::new(),
            is_async: false,
            alias: Maybe::None,
            is_negative: true,
            transforms: List::new(),
            condition: Maybe::None,
        }
    }

    /// Create an aliased context reference (`Database as db`)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
    pub fn aliased(name: Text, type_id: TypeId, alias: Text) -> Self {
        ContextRef {
            name,
            type_id,
            type_args: List::new(),
            is_async: false,
            alias: Maybe::Some(alias),
            is_negative: false,
            transforms: List::new(),
            condition: Maybe::None,
        }
    }

    /// Create a conditional context reference (`Analytics if cfg.enabled`)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    pub fn conditional(name: Text, type_id: TypeId, condition: Text) -> Self {
        ContextRef {
            name,
            type_id,
            type_args: List::new(),
            is_async: false,
            alias: Maybe::None,
            is_negative: false,
            transforms: List::new(),
            condition: Maybe::Some(condition),
        }
    }

    /// Create a context reference with transforms (`Database.transactional()`)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.3 - Context Transformations
    pub fn with_transforms(name: Text, type_id: TypeId, transforms: List<ContextTransformRef>) -> Self {
        ContextRef {
            name,
            type_id,
            type_args: List::new(),
            is_async: false,
            alias: Maybe::None,
            is_negative: false,
            transforms,
            condition: Maybe::None,
        }
    }

    /// Get the effective name for this context (alias if set, otherwise name)
    pub fn effective_name(&self) -> &Text {
        match &self.alias {
            Maybe::Some(alias) => alias,
            Maybe::None => &self.name,
        }
    }

    /// Check if this context is conditional
    pub fn is_conditional(&self) -> bool {
        matches!(self.condition, Maybe::Some(_))
    }

    /// Check if this context has transforms
    pub fn has_transforms(&self) -> bool {
        !self.transforms.is_empty()
    }

    /// Create a context reference with type arguments
    ///
    /// # Arguments
    ///
    /// * `name` - Context name
    /// * `type_id` - Runtime type ID
    /// * `type_args` - Type arguments (e.g., ["Int"] for State<Int>)
    pub fn with_type_args(
        name: Text,
        type_id: TypeId,
        type_args: impl IntoIterator<Item = Text>,
    ) -> Self {
        ContextRef {
            name,
            type_id,
            type_args: type_args.into_iter().collect(),
            is_async: false,
            alias: Maybe::None,
            is_negative: false,
            transforms: List::new(),
            condition: Maybe::None,
        }
    }

    /// Mark this context as async
    pub fn as_async(mut self) -> Self {
        self.is_async = true;
        self
    }

    /// Get the qualified name including type arguments
    ///
    /// # Examples
    ///
    /// - `Logger` -> "Logger"
    /// - `State<Int>` -> "State<Int>"
    /// - `Cache<Text, Data>` -> "Cache<Text, Data>"
    pub fn qualified_name(&self) -> Text {
        if self.type_args.is_empty() {
            self.name.clone()
        } else {
            format!("{}<{}>", self.name, self.type_args.join(", ")).into()
        }
    }
}

impl Default for ContextRequirement {
    fn default() -> Self {
        Self::empty()
    }
}

impl fmt::Display for ContextRequirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return write!(f, "[]");
        }

        write!(f, "[")?;
        for (i, ctx) in self.contexts.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", ctx)?;
        }
        write!(f, "]")
    }
}

impl fmt::Display for ContextRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_async {
            write!(f, "async ")?;
        }
        write!(f, "{}", self.qualified_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_requirement() {
        let req = ContextRequirement::empty();
        assert!(req.is_empty());
        assert_eq!(req.len(), 0);
    }

    #[test]
    fn test_single_requirement() {
        let ctx_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let req = ContextRequirement::single(ctx_ref);

        assert!(!req.is_empty());
        assert_eq!(req.len(), 1);
        assert!(req.requires("Logger"));
    }

    #[test]
    fn test_multiple_requirements() {
        let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let database = ContextRef::new("Database".into(), TypeId::of::<String>());

        let req = ContextRequirement::from_contexts(vec![logger, database]);

        assert_eq!(req.len(), 2);
        assert!(req.requires("Logger"));
        assert!(req.requires("Database"));
        assert!(!req.requires("Config"));
    }

    #[test]
    fn test_add_remove_context() {
        let mut req = ContextRequirement::empty();

        let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
        req.add_context(logger);

        assert_eq!(req.len(), 1);
        assert!(req.requires("Logger"));

        assert!(req.remove_context("Logger"));
        assert!(req.is_empty());
        assert!(!req.remove_context("Logger"));
    }

    #[test]
    fn test_merge_requirements() {
        let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let database = ContextRef::new("Database".into(), TypeId::of::<String>());

        let req1 = ContextRequirement::single(logger);
        let req2 = ContextRequirement::single(database);

        let merged = req1.merge(&req2);

        assert_eq!(merged.len(), 2);
        assert!(merged.requires("Logger"));
        assert!(merged.requires("Database"));
    }

    #[test]
    fn test_subset() {
        let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let database = ContextRef::new("Database".into(), TypeId::of::<String>());

        let req_small = ContextRequirement::single(logger.clone());
        let req_large = ContextRequirement::from_contexts(vec![logger, database]);

        assert!(req_small.is_subset_of(&req_large));
        assert!(!req_large.is_subset_of(&req_small));
    }

    #[test]
    fn test_context_ref_with_type_args() {
        let state_ref =
            ContextRef::with_type_args("State".into(), TypeId::of::<()>(), vec!["Int".into()]);

        assert_eq!(state_ref.qualified_name(), "State<Int>");
        assert_eq!(state_ref.type_args.len(), 1);
    }

    #[test]
    fn test_async_context() {
        let db_ref = ContextRef::new("Database".into(), TypeId::of::<()>()).as_async();

        assert!(db_ref.is_async);

        let req = ContextRequirement::single(db_ref);
        assert!(req.has_async_contexts());
    }

    #[test]
    fn test_get_context() {
        let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let mut req = ContextRequirement::empty();
        req.add_context(logger);

        let ctx = req.get_context("Logger");
        assert!(matches!(ctx, Maybe::Some(_)));

        let missing = req.get_context("Database");
        assert!(matches!(missing, Maybe::None));
    }

    #[test]
    fn test_context_names() {
        let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let database = ContextRef::new("Database".into(), TypeId::of::<String>());

        let req = ContextRequirement::from_contexts(vec![logger, database]);
        let names = req.context_names();

        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|n| n.as_str() == "Logger"));
        assert!(names.iter().any(|n| n.as_str() == "Database"));
    }
}
