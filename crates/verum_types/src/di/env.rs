//! Context Environment (θ) - Runtime task-local storage for DI
//!
//! Context runtime: task-local storage (theta) for context lookup, ~5-30ns overhead per access — Contexts + Async Integration
//! Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Section 6 - Performance Characteristics
//!
//! This module implements the context environment (θ, theta), which provides
//! task-local storage for context providers. The environment supports:
//! - Fast context lookup (<50ns target, ~5-30ns typical)
//! - Lexical scoping with parent chain
//! - Thread-safe access via Arc/Mutex when needed
//! - Integration with async runtime
//!
//! # Performance Target
//!
//! Context lookup: **< 50ns** (typically ~5-30ns with optimizations)
//!
//! # Examples
//!
//! ```ignore
//! use verum_types::di::env::ContextEnv;
//! # struct Logger;
//! # impl Logger { fn log(&self, _: &str) {} }
//! # let logger = Logger;
//!
//! let mut env = ContextEnv::new();
//! env.insert(logger);
//!
//! if let Some(logger) = env.get::<Logger>() {
//!     logger.log("Hello");
//! }
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use verum_common::{Maybe, Text};

/// Context environment (θ) - task-local storage for context providers
///
/// Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Context System Runtime
///
/// The context environment stores context provider instances and provides
/// fast lookup by type. It supports lexical scoping through parent chains.
///
/// # Implementation Notes
///
/// - Uses `HashMap<TypeId, Box<dyn Any>>` for fast O(1) lookup
/// - Parent chain for lexical scoping (provide blocks)
/// - Thread-safe variant available via `Arc<Mutex<ContextEnv>>`
/// - Performance: ~5-30ns per lookup (Tier 1-3), ~100ns (Tier 0)
///
/// # Memory Layout
///
/// ```text
/// ContextEnv {
///     contexts: HashMap<TypeId, Box<dyn Any>>  // ~24 bytes overhead per entry
///     parent: Option<Arc<ContextEnv>>           // 8 bytes when None, 16 when Some
/// }
/// ```
pub struct ContextEnv {
    /// Map from TypeId to context provider instance
    /// Using HashMap for O(1) lookup performance
    contexts: HashMap<TypeId, Box<dyn Any + Send + Sync>>,

    /// Map from alias name to (TypeId, context instance)
    /// Supports `using [Database as primary_db]` syntax
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
    aliased_contexts: HashMap<Text, (TypeId, Box<dyn Any + Send + Sync>)>,

    /// Parent environment for lexical scoping
    /// When a context is not found locally, search in parent
    parent: Option<Arc<ContextEnv>>,
}

/// Thread-safe context environment wrapper
///
/// Used when contexts need to be shared across threads or modified concurrently.
pub type SharedContextEnv = Arc<Mutex<ContextEnv>>;

impl ContextEnv {
    /// Create a new empty context environment
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_types::di::env::ContextEnv;
    ///
    /// let env = ContextEnv::new();
    /// assert!(env.is_empty());
    /// ```
    pub fn new() -> Self {
        ContextEnv {
            contexts: HashMap::new(),
            aliased_contexts: HashMap::new(),
            parent: None,
        }
    }

    /// Create a context environment with a parent
    ///
    /// The parent chain enables lexical scoping:
    /// - Child scopes can override parent contexts
    /// - Lookups fall back to parent if not found locally
    ///
    /// # Arguments
    ///
    /// * `parent` - The parent environment
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use verum_types::di::env::ContextEnv;
    ///
    /// let parent = Arc::new(ContextEnv::new());
    /// let child = ContextEnv::with_parent(parent);
    /// ```
    pub fn with_parent(parent: Arc<ContextEnv>) -> Self {
        ContextEnv {
            contexts: HashMap::new(),
            aliased_contexts: HashMap::new(),
            parent: Some(parent),
        }
    }

    /// Insert a context provider into this environment
    ///
    /// # Type Parameters
    ///
    /// * `T` - The provider type (must be `Any + Send + Sync + 'static`)
    ///
    /// # Arguments
    ///
    /// * `value` - The provider instance
    ///
    /// # Performance
    ///
    /// O(1) insertion via HashMap
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::env::ContextEnv;
    /// # struct ConsoleLogger;
    /// # impl ConsoleLogger { fn new() -> Self { ConsoleLogger } }
    /// # struct PostgresDatabase;
    /// # impl PostgresDatabase { fn new() -> Self { PostgresDatabase } }
    ///
    /// let mut env = ContextEnv::new();
    /// env.insert(ConsoleLogger::new());
    /// env.insert(PostgresDatabase::new());
    /// ```
    pub fn insert<T: Any + Send + Sync + 'static>(&mut self, value: T) {
        let type_id = TypeId::of::<T>();
        self.contexts.insert(type_id, Box::new(value));
    }

    /// Insert a context provider with explicit TypeId
    ///
    /// Used when the type is not known at compile time.
    ///
    /// # Arguments
    ///
    /// * `type_id` - The type identifier
    /// * `value` - The provider instance (boxed)
    pub fn insert_boxed(&mut self, type_id: TypeId, value: Box<dyn Any + Send + Sync>) {
        self.contexts.insert(type_id, value);
    }

    /// Insert a context provider with an alias
    ///
    /// Aliased contexts allow multiple instances of the same type to coexist,
    /// distinguished by their alias name.
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
    ///
    /// # Type Parameters
    ///
    /// * `T` - The provider type (must be `Any + Send + Sync + 'static`)
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias name for this context instance
    /// * `value` - The provider instance
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use verum_types::di::env::ContextEnv;
    /// # struct Database;
    /// # impl Database { fn new(_url: &str) -> Self { Database } }
    ///
    /// let mut env = ContextEnv::new();
    /// env.insert_with_alias("primary_db", Database::new("postgres://primary"));
    /// env.insert_with_alias("replica_db", Database::new("postgres://replica"));
    /// ```
    pub fn insert_with_alias<T: Any + Send + Sync + 'static>(
        &mut self,
        alias: impl Into<Text>,
        value: T,
    ) {
        let type_id = TypeId::of::<T>();
        let alias_text = alias.into();
        self.aliased_contexts.insert(alias_text, (type_id, Box::new(value)));
    }

    /// Insert a boxed context provider with an alias
    ///
    /// Used when the type is not known at compile time.
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias name for this context instance
    /// * `type_id` - The type identifier
    /// * `value` - The provider instance (boxed)
    pub fn insert_boxed_with_alias(
        &mut self,
        alias: impl Into<Text>,
        type_id: TypeId,
        value: Box<dyn Any + Send + Sync>,
    ) {
        let alias_text = alias.into();
        self.aliased_contexts.insert(alias_text, (type_id, value));
    }

    /// Get a context provider by alias (local only)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
    ///
    /// # Type Parameters
    ///
    /// * `T` - The expected provider type
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias name to look up
    ///
    /// # Returns
    ///
    /// `Some(&T)` if found locally with matching type, `None` otherwise
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use verum_types::di::env::ContextEnv;
    /// # struct Database;
    /// # impl Database { fn query(&self, _: &str) {} }
    ///
    /// let env = ContextEnv::new();
    /// if let Some(db) = env.get_by_alias::<Database>("primary_db") {
    ///     db.query("SELECT * FROM users");
    /// }
    /// ```
    pub fn get_by_alias<T: Any + 'static>(&self, alias: &str) -> Maybe<&T> {
        let alias_text = Text::from(alias);
        if let Some((stored_type_id, boxed)) = self.aliased_contexts.get(&alias_text) {
            // Verify the type matches
            if *stored_type_id == TypeId::of::<T>() {
                return boxed.downcast_ref::<T>().and_then(Maybe::Some);
            }
        }
        Maybe::None
    }

    /// Get a mutable reference to a context provider by alias (local only)
    ///
    /// # Type Parameters
    ///
    /// * `T` - The expected provider type
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias name to look up
    ///
    /// # Returns
    ///
    /// `Some(&mut T)` if found locally with matching type, `None` otherwise
    pub fn get_by_alias_mut<T: Any + 'static>(&mut self, alias: &str) -> Maybe<&mut T> {
        let alias_text = Text::from(alias);
        if let Some((stored_type_id, boxed)) = self.aliased_contexts.get_mut(&alias_text) {
            // Verify the type matches
            if *stored_type_id == TypeId::of::<T>() {
                return boxed.downcast_mut::<T>().and_then(Maybe::Some);
            }
        }
        Maybe::None
    }

    /// Get a context provider by alias, searching parent chain if needed
    ///
    /// This is the primary aliased lookup method used at runtime.
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
    ///
    /// # Type Parameters
    ///
    /// * `T` - The expected provider type
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias name to look up
    ///
    /// # Returns
    ///
    /// `Some(&T)` if found in this env or any parent, `None` otherwise
    ///
    /// # Performance
    ///
    /// - Local hit: ~5-10ns (HashMap lookup)
    /// - Parent chain: +10-20ns per level
    pub fn get_by_alias_or_parent<T: Any + 'static>(&self, alias: &str) -> Maybe<&T> {
        // Try local lookup first (fast path)
        if let result @ Maybe::Some(_) = self.get_by_alias::<T>(alias) {
            return result;
        }

        // Walk parent chain
        let mut current_parent = &self.parent;
        while let Some(parent) = current_parent {
            if let result @ Maybe::Some(_) = parent.get_by_alias::<T>(alias) {
                return result;
            }
            current_parent = &parent.parent;
        }

        Maybe::None
    }

    /// Check if an alias is defined (locally or in parent chain)
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias name to check
    ///
    /// # Returns
    ///
    /// `true` if the alias exists, `false` otherwise
    pub fn has_alias(&self, alias: &str) -> bool {
        let alias_text = Text::from(alias);

        // Check locally
        if self.aliased_contexts.contains_key(&alias_text) {
            return true;
        }

        // Check parent chain
        let mut current_parent = &self.parent;
        while let Some(parent) = current_parent {
            if parent.aliased_contexts.contains_key(&alias_text) {
                return true;
            }
            current_parent = &parent.parent;
        }

        false
    }

    /// Get all alias names defined in this environment (local only)
    ///
    /// # Returns
    ///
    /// Iterator over alias names
    pub fn aliases(&self) -> impl Iterator<Item = &Text> {
        self.aliased_contexts.keys()
    }

    /// Get the number of aliased contexts in this environment (local only)
    pub fn aliases_len(&self) -> usize {
        self.aliased_contexts.len()
    }

    /// Remove an aliased context from this environment
    ///
    /// Only removes from local environment, not from parent.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The provider type
    ///
    /// # Arguments
    ///
    /// * `alias` - The alias name to remove
    ///
    /// # Returns
    ///
    /// `Some(T)` if the alias was present with matching type, `None` otherwise
    pub fn remove_alias<T: Any + 'static>(&mut self, alias: &str) -> Maybe<T> {
        let alias_text = Text::from(alias);
        if let Some((stored_type_id, boxed)) = self.aliased_contexts.remove(&alias_text) {
            if stored_type_id == TypeId::of::<T>() {
                if let Ok(typed_box) = boxed.downcast::<T>() {
                    return Maybe::Some(*typed_box);
                }
            }
        }
        Maybe::None
    }

    /// Get a context provider from this environment (local only)
    ///
    /// Does NOT search parent chain. Use `get_or_parent` for full lookup.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The provider type
    ///
    /// # Returns
    ///
    /// `Some(&T)` if found locally, `None` otherwise
    ///
    /// # Performance
    ///
    /// O(1) lookup via HashMap: ~5-10ns
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use verum_types::di::env::ContextEnv;
    /// # struct Logger;
    /// # impl Logger { fn log(&self, _: &str) {} }
    ///
    /// let env = ContextEnv::new();
    /// if let Some(logger) = env.get::<Logger>() {
    ///     logger.log("Hello");
    /// }
    /// ```
    pub fn get<T: Any + 'static>(&self) -> Maybe<&T> {
        let type_id = TypeId::of::<T>();
        self.contexts
            .get(&type_id)
            .and_then(|boxed| boxed.downcast_ref::<T>()).and_then(Maybe::Some)
    }

    /// Get a mutable reference to a context provider (local only)
    ///
    /// # Type Parameters
    ///
    /// * `T` - The provider type
    ///
    /// # Returns
    ///
    /// `Some(&mut T)` if found locally, `None` otherwise
    pub fn get_mut<T: Any + 'static>(&mut self) -> Maybe<&mut T> {
        let type_id = TypeId::of::<T>();
        self.contexts
            .get_mut(&type_id)
            .and_then(|boxed| boxed.downcast_mut::<T>()).and_then(Maybe::Some)
    }

    /// Get a context provider, searching parent chain if needed
    ///
    /// This is the primary lookup method used at runtime.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The provider type
    ///
    /// # Returns
    ///
    /// `Some(&T)` if found in this env or any parent, `None` otherwise
    ///
    /// # Performance
    ///
    /// - Local hit: ~5-10ns (HashMap lookup)
    /// - Parent chain: +10-20ns per level
    /// - Target: < 50ns for typical 2-3 level chains
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use verum_types::di::env::ContextEnv;
    /// # struct Logger;
    /// # impl Logger { fn log(&self, _: &str) {} }
    ///
    /// let env = ContextEnv::new();
    /// if let Some(logger) = env.get_or_parent::<Logger>() {
    ///     logger.log("Hello from child or parent");
    /// }
    /// ```
    pub fn get_or_parent<T: Any + 'static>(&self) -> Maybe<&T> {
        // Try local lookup first (fast path)
        if let Maybe::Some(value) = self.get::<T>() {
            return Maybe::Some(value);
        }

        // Walk parent chain
        let mut current_parent = &self.parent;
        while let Some(parent) = current_parent {
            if let Maybe::Some(value) = parent.get::<T>() {
                return Maybe::Some(value);
            }
            current_parent = &parent.parent;
        }

        Maybe::None
    }

    /// Check if a context is available (by TypeId)
    ///
    /// Used for requirement checking during compilation/runtime.
    ///
    /// # Arguments
    ///
    /// * `type_id` - The type identifier to check
    ///
    /// # Returns
    ///
    /// `true` if the context is available (locally or in parent)
    pub fn has_context(&self, type_id: TypeId) -> bool {
        // Check locally
        if self.contexts.contains_key(&type_id) {
            return true;
        }

        // Check parent chain
        let mut current_parent = &self.parent;
        while let Some(parent) = current_parent {
            if parent.contexts.contains_key(&type_id) {
                return true;
            }
            current_parent = &parent.parent;
        }

        false
    }

    /// Remove a context provider from this environment
    ///
    /// Only removes from local environment, not from parent.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The provider type
    ///
    /// # Returns
    ///
    /// `Some(T)` if the context was present, `None` otherwise
    pub fn remove<T: Any + 'static>(&mut self) -> Maybe<T> {
        let type_id = TypeId::of::<T>();
        self.contexts
            .remove(&type_id)
            .and_then(|boxed| boxed.downcast::<T>().ok())
            .map(|boxed| *boxed)
    }

    /// Clear all contexts from this environment
    ///
    /// Does not affect parent environments.
    /// Clears both typed contexts and aliased contexts.
    pub fn clear(&mut self) {
        self.contexts.clear();
        self.aliased_contexts.clear();
    }

    /// Check if this environment is empty (no contexts and no aliases)
    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty() && self.aliased_contexts.is_empty()
    }

    /// Get the number of typed contexts in this environment (local only)
    ///
    /// Does not include aliased contexts. Use `len_with_aliases()` for total.
    pub fn len(&self) -> usize {
        self.contexts.len()
    }

    /// Get the number of all contexts including aliased (local only)
    pub fn len_with_aliases(&self) -> usize {
        self.contexts.len() + self.aliased_contexts.len()
    }

    /// Get the total number of contexts including parent chain
    ///
    /// Includes both typed and aliased contexts from all levels.
    pub fn total_len(&self) -> usize {
        let mut count = self.contexts.len() + self.aliased_contexts.len();
        let mut current_parent = &self.parent;
        while let Some(parent) = current_parent {
            count += parent.contexts.len() + parent.aliased_contexts.len();
            current_parent = &parent.parent;
        }
        count
    }

    /// Get the depth of the parent chain
    ///
    /// Useful for debugging and performance analysis.
    ///
    /// # Returns
    ///
    /// 0 for root environment, 1+ for nested scopes
    pub fn depth(&self) -> usize {
        let mut depth = 0;
        let mut current_parent = &self.parent;
        while let Some(parent) = current_parent {
            depth += 1;
            current_parent = &parent.parent;
        }
        depth
    }

    /// Clone this environment with a new parent
    ///
    /// Creates a child scope that inherits from this environment.
    ///
    /// # Examples
    ///
    /// ```
    /// use verum_types::di::env::ContextEnv;
    ///
    /// let parent = ContextEnv::new();
    /// let child = parent.create_child();
    /// assert_eq!(child.depth(), 1);
    /// ```
    pub fn create_child(self) -> Self {
        ContextEnv {
            contexts: HashMap::new(),
            aliased_contexts: HashMap::new(),
            parent: Some(Arc::new(self)),
        }
    }

    /// Create a child scope from a shared reference
    ///
    /// # Arguments
    ///
    /// * `parent` - Arc to the parent environment
    pub fn create_child_from(parent: Arc<ContextEnv>) -> Self {
        ContextEnv::with_parent(parent)
    }
}

impl Default for ContextEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ContextEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextEnv")
            .field("local_contexts", &self.contexts.len())
            .field("local_aliases", &self.aliased_contexts.len())
            .field("total_contexts", &self.total_len())
            .field("depth", &self.depth())
            .finish()
    }
}

// Helper for creating thread-safe shared environments
impl ContextEnv {
    /// Create a thread-safe shared context environment
    pub fn shared() -> SharedContextEnv {
        Arc::new(Mutex::new(ContextEnv::new()))
    }

    /// Create a thread-safe shared environment with parent
    pub fn shared_with_parent(parent: Arc<ContextEnv>) -> SharedContextEnv {
        Arc::new(Mutex::new(ContextEnv::with_parent(parent)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct TestLogger {
        name: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct TestDatabase {
        url: String,
    }

    #[test]
    fn test_new_environment() {
        let env = ContextEnv::new();
        assert!(env.is_empty());
        assert_eq!(env.len(), 0);
        assert_eq!(env.depth(), 0);
    }

    #[test]
    fn test_insert_and_get() {
        let mut env = ContextEnv::new();
        let logger = TestLogger {
            name: "test".to_string(),
        };

        env.insert(logger.clone());

        let retrieved = env.get::<TestLogger>();
        assert!(matches!(retrieved, Maybe::Some(_)));
        if let Maybe::Some(l) = retrieved {
            assert_eq!(l.name, "test");
        }
    }

    #[test]
    fn test_get_mut() {
        let mut env = ContextEnv::new();
        env.insert(TestLogger {
            name: "test".to_string(),
        });

        if let Maybe::Some(logger) = env.get_mut::<TestLogger>() {
            logger.name = "modified".to_string();
        }

        if let Maybe::Some(logger) = env.get::<TestLogger>() {
            assert_eq!(logger.name, "modified");
        }
    }

    #[test]
    fn test_multiple_contexts() {
        let mut env = ContextEnv::new();

        env.insert(TestLogger {
            name: "logger".to_string(),
        });
        env.insert(TestDatabase {
            url: "postgres://localhost".to_string(),
        });

        assert_eq!(env.len(), 2);
        assert!(matches!(env.get::<TestLogger>(), Maybe::Some(_)));
        assert!(matches!(env.get::<TestDatabase>(), Maybe::Some(_)));
    }

    #[test]
    fn test_parent_chain() {
        let mut parent = ContextEnv::new();
        parent.insert(TestLogger {
            name: "parent_logger".to_string(),
        });

        let mut child = ContextEnv::with_parent(Arc::new(parent));
        child.insert(TestDatabase {
            url: "postgres://localhost".to_string(),
        });

        // Child has 1 local context
        assert_eq!(child.len(), 1);
        // But 2 total (including parent)
        assert_eq!(child.total_len(), 2);
        assert_eq!(child.depth(), 1);

        // Can access parent's logger
        assert!(matches!(
            child.get_or_parent::<TestLogger>(),
            Maybe::Some(_)
        ));
        // And own database
        assert!(matches!(child.get::<TestDatabase>(), Maybe::Some(_)));
    }

    #[test]
    fn test_parent_chain_override() {
        let mut parent = ContextEnv::new();
        parent.insert(TestLogger {
            name: "parent_logger".to_string(),
        });

        let mut child = ContextEnv::with_parent(Arc::new(parent));
        child.insert(TestLogger {
            name: "child_logger".to_string(),
        });

        // Child's logger shadows parent's
        if let Maybe::Some(logger) = child.get_or_parent::<TestLogger>() {
            assert_eq!(logger.name, "child_logger");
        }
    }

    #[test]
    fn test_remove() {
        let mut env = ContextEnv::new();
        env.insert(TestLogger {
            name: "test".to_string(),
        });

        assert!(matches!(env.get::<TestLogger>(), Maybe::Some(_)));

        let removed = env.remove::<TestLogger>();
        assert!(matches!(removed, Maybe::Some(_)));

        assert!(matches!(env.get::<TestLogger>(), Maybe::None));
    }

    #[test]
    fn test_clear() {
        let mut env = ContextEnv::new();
        env.insert(TestLogger {
            name: "test".to_string(),
        });
        env.insert(TestDatabase {
            url: "test".to_string(),
        });

        assert_eq!(env.len(), 2);

        env.clear();

        assert!(env.is_empty());
        assert_eq!(env.len(), 0);
    }

    #[test]
    fn test_has_context() {
        let mut parent = ContextEnv::new();
        parent.insert(TestLogger {
            name: "parent".to_string(),
        });

        let child = ContextEnv::with_parent(Arc::new(parent));

        let logger_id = TypeId::of::<TestLogger>();
        let db_id = TypeId::of::<TestDatabase>();

        assert!(child.has_context(logger_id));
        assert!(!child.has_context(db_id));
    }

    #[test]
    fn test_create_child() {
        let mut parent = ContextEnv::new();
        parent.insert(TestLogger {
            name: "parent".to_string(),
        });

        let child = parent.create_child();

        assert_eq!(child.depth(), 1);
        assert!(matches!(
            child.get_or_parent::<TestLogger>(),
            Maybe::Some(_)
        ));
    }

    #[test]
    fn test_deep_parent_chain() {
        let mut env1 = ContextEnv::new();
        env1.insert(TestLogger {
            name: "level1".to_string(),
        });

        let mut env2 = ContextEnv::with_parent(Arc::new(env1));
        env2.insert(TestDatabase {
            url: "level2".to_string(),
        });

        let env3 = ContextEnv::with_parent(Arc::new(env2));

        assert_eq!(env3.depth(), 2);
        assert_eq!(env3.total_len(), 2);

        // Can access contexts from both levels
        assert!(matches!(env3.get_or_parent::<TestLogger>(), Maybe::Some(_)));
        assert!(matches!(
            env3.get_or_parent::<TestDatabase>(),
            Maybe::Some(_)
        ));
    }

    #[test]
    fn test_shared_environment() {
        let env = ContextEnv::shared();

        {
            let mut locked = env.lock().unwrap();
            locked.insert(TestLogger {
                name: "shared".to_string(),
            });
        }

        {
            let locked = env.lock().unwrap();
            assert!(matches!(locked.get::<TestLogger>(), Maybe::Some(_)));
        }
    }

    // ========================================================================
    // Alias Tests - context alias validation: ensuring aliases don't create contradictions
    // ========================================================================

    #[test]
    fn test_insert_and_get_by_alias() {
        let mut env = ContextEnv::new();

        env.insert_with_alias("primary", TestDatabase { url: "postgres://primary".to_string() });
        env.insert_with_alias("replica", TestDatabase { url: "postgres://replica".to_string() });

        assert_eq!(env.aliases_len(), 2);

        // Get by alias
        let primary = env.get_by_alias::<TestDatabase>("primary");
        assert!(matches!(primary, Maybe::Some(_)));
        if let Maybe::Some(db) = primary {
            assert_eq!(db.url, "postgres://primary");
        }

        let replica = env.get_by_alias::<TestDatabase>("replica");
        assert!(matches!(replica, Maybe::Some(_)));
        if let Maybe::Some(db) = replica {
            assert_eq!(db.url, "postgres://replica");
        }
    }

    #[test]
    fn test_alias_with_different_types() {
        let mut env = ContextEnv::new();

        env.insert_with_alias("main_logger", TestLogger { name: "main".to_string() });
        env.insert_with_alias("backup_db", TestDatabase { url: "postgres://backup".to_string() });

        // Get with correct type
        assert!(matches!(env.get_by_alias::<TestLogger>("main_logger"), Maybe::Some(_)));
        assert!(matches!(env.get_by_alias::<TestDatabase>("backup_db"), Maybe::Some(_)));

        // Get with wrong type should return None
        assert!(matches!(env.get_by_alias::<TestDatabase>("main_logger"), Maybe::None));
        assert!(matches!(env.get_by_alias::<TestLogger>("backup_db"), Maybe::None));
    }

    #[test]
    fn test_has_alias() {
        let mut env = ContextEnv::new();
        env.insert_with_alias("my_alias", TestLogger { name: "aliased".to_string() });

        assert!(env.has_alias("my_alias"));
        assert!(!env.has_alias("unknown"));
    }

    #[test]
    fn test_alias_parent_chain() {
        let mut parent = ContextEnv::new();
        parent.insert_with_alias("parent_db", TestDatabase { url: "postgres://parent".to_string() });

        let mut child = ContextEnv::with_parent(Arc::new(parent));
        child.insert_with_alias("child_db", TestDatabase { url: "postgres://child".to_string() });

        // Child can access its own alias
        assert!(matches!(child.get_by_alias::<TestDatabase>("child_db"), Maybe::Some(_)));

        // Child can access parent's alias via parent chain lookup
        assert!(matches!(child.get_by_alias_or_parent::<TestDatabase>("parent_db"), Maybe::Some(_)));

        // has_alias checks parent chain
        assert!(child.has_alias("parent_db"));
        assert!(child.has_alias("child_db"));
    }

    #[test]
    fn test_alias_shadowing() {
        let mut parent = ContextEnv::new();
        parent.insert_with_alias("db", TestDatabase { url: "parent_url".to_string() });

        let mut child = ContextEnv::with_parent(Arc::new(parent));
        child.insert_with_alias("db", TestDatabase { url: "child_url".to_string() });

        // Child's alias shadows parent's
        if let Maybe::Some(db) = child.get_by_alias_or_parent::<TestDatabase>("db") {
            assert_eq!(db.url, "child_url");
        }
    }

    #[test]
    fn test_get_by_alias_mut() {
        let mut env = ContextEnv::new();
        env.insert_with_alias("mutable_db", TestDatabase { url: "original".to_string() });

        // Modify via mutable reference
        if let Maybe::Some(db) = env.get_by_alias_mut::<TestDatabase>("mutable_db") {
            db.url = "modified".to_string();
        }

        // Verify modification
        if let Maybe::Some(db) = env.get_by_alias::<TestDatabase>("mutable_db") {
            assert_eq!(db.url, "modified");
        }
    }

    #[test]
    fn test_remove_alias() {
        let mut env = ContextEnv::new();
        env.insert_with_alias("removable", TestLogger { name: "to_remove".to_string() });

        assert!(env.has_alias("removable"));

        let removed = env.remove_alias::<TestLogger>("removable");
        assert!(matches!(removed, Maybe::Some(_)));

        assert!(!env.has_alias("removable"));
        assert!(matches!(env.get_by_alias::<TestLogger>("removable"), Maybe::None));
    }

    #[test]
    fn test_clear_includes_aliases() {
        let mut env = ContextEnv::new();
        env.insert(TestLogger { name: "typed".to_string() });
        env.insert_with_alias("alias1", TestDatabase { url: "url1".to_string() });

        assert_eq!(env.len(), 1);
        assert_eq!(env.aliases_len(), 1);

        env.clear();

        assert!(env.is_empty());
        assert_eq!(env.len(), 0);
        assert_eq!(env.aliases_len(), 0);
    }

    #[test]
    fn test_len_with_aliases() {
        let mut env = ContextEnv::new();
        env.insert(TestLogger { name: "typed".to_string() });
        env.insert_with_alias("alias1", TestDatabase { url: "url1".to_string() });
        env.insert_with_alias("alias2", TestDatabase { url: "url2".to_string() });

        assert_eq!(env.len(), 1);            // Only typed contexts
        assert_eq!(env.aliases_len(), 2);    // Only aliased contexts
        assert_eq!(env.len_with_aliases(), 3); // Both
    }

    #[test]
    fn test_total_len_with_aliases() {
        let mut parent = ContextEnv::new();
        parent.insert(TestLogger { name: "parent".to_string() });
        parent.insert_with_alias("parent_alias", TestDatabase { url: "parent".to_string() });

        let mut child = ContextEnv::with_parent(Arc::new(parent));
        child.insert_with_alias("child_alias", TestDatabase { url: "child".to_string() });

        // 1 typed in parent + 1 alias in parent + 1 alias in child = 3
        assert_eq!(child.total_len(), 3);
    }

    #[test]
    fn test_aliases_iterator() {
        let mut env = ContextEnv::new();
        env.insert_with_alias("alias_a", TestLogger { name: "a".to_string() });
        env.insert_with_alias("alias_b", TestDatabase { url: "b".to_string() });

        let aliases: Vec<&Text> = env.aliases().collect();
        assert_eq!(aliases.len(), 2);
        assert!(aliases.iter().any(|a| a.as_str() == "alias_a"));
        assert!(aliases.iter().any(|a| a.as_str() == "alias_b"));
    }

    #[test]
    fn test_typed_and_aliased_independent() {
        let mut env = ContextEnv::new();

        // Insert typed context
        env.insert(TestDatabase { url: "typed_url".to_string() });

        // Insert same type with alias - they should be independent
        env.insert_with_alias("aliased", TestDatabase { url: "aliased_url".to_string() });

        // Both should be accessible independently
        if let Maybe::Some(typed) = env.get::<TestDatabase>() {
            assert_eq!(typed.url, "typed_url");
        }
        if let Maybe::Some(aliased) = env.get_by_alias::<TestDatabase>("aliased") {
            assert_eq!(aliased.url, "aliased_url");
        }
    }
}
