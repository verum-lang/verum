//! Context System (Dependency Injection) for Verum
//!
//! **THIS IS DEPENDENCY INJECTION, NOT ALGEBRAIC EFFECTS**
//!
//! Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Context System (Capability-based Dependency Injection)
//!
//! # Overview
//!
//! The Context System provides type-safe dependency injection for cross-cutting
//! concerns like logging, metrics, authentication, and database access. It is
//! designed as a modern, capability-based DI framework with minimal runtime overhead.
//!
//! **Key Design Principles:**
//!
//! 1. **Orthogonality**: Contexts manage *what* you need (database, logger),
//!    async/await manages *when* operations complete
//! 2. **Dependency Injection**: Functions declare required capabilities with `using [Context]`
//! 3. **Lexical Scoping**: Dependencies installed with `provide` keyword
//! 4. **Task-Local Storage**: Context environment (θ) stored in task-local storage
//! 5. **Async Integration**: Context methods can be async - async is orthogonal to contexts
//!
//! # Core Components
//!
//! ## Context Declaration (`context` keyword)
//!
//! Contexts are interface-like specifications that define operations:
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
//!
//! See [`ContextDecl`] for the implementation.
//!
//! ## Context Requirements (`using` keyword)
//!
//! Functions declare what contexts they need:
//!
//! ```verum
//! fn process_user(id: UserId) using [Logger, Database] {
//!     Logger.log(Level.Info, f"Processing user {id}");
//!     let user = Database.query(sql"SELECT * FROM users WHERE id = {id}");
//!     // ...
//! }
//! ```
//!
//! See [`ContextRequirement`] and [`ContextRef`] for the implementation.
//!
//! ## Context Providers (`provide` keyword)
//!
//! Concrete implementations are bound using `provide`:
//!
//! ```verum
//! fn main() {
//!     provide Logger = console_logger();
//!     provide Database = postgres_connection().await;
//!
//!     // Now contexts are available for all subsequent code
//!     process_user(UserId(42));
//! }
//! ```
//!
//! See [`ContextProvider`] and [`ProviderScope`] for the implementation.
//!
//! ## Context Environment (θ, theta)
//!
//! Runtime storage for context providers using task-local storage:
//!
//! - **Fast lookup**: ~5-30ns (Tier 1-3), ~100ns (Tier 0)
//! - **Lexical scoping**: Parent chain for nested scopes
//! - **Thread-safe**: Via `Arc<Mutex<ContextEnv>>`
//!
//! See [`ContextEnv`] for the implementation.
//!
//! ## Context Groups
//!
//! Reusable context sets for common patterns:
//!
//! ```verum
//! using WebContext = [Database, Logger, Auth, Metrics]
//!
//! fn handle_request() using WebContext {
//!     // Has access to all WebContext contexts
//! }
//! ```
//!
//! See [`ContextGroup`] and [`ContextGroupRegistry`] for the implementation.
//!
//! # Performance Characteristics
//!
//! Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — Section 6 - Performance Characteristics
//!
//! | Operation | Cost | Notes |
//! |-----------|------|-------|
//! | Context lookup | ~5-30ns (Tier 1-3), ~100ns (Tier 0) | HashMap lookup + vtable dispatch |
//! | Context provision | ~50ns | Insert into HashMap |
//! | Parent chain lookup | +10-20ns per level | Rare, typically 1-2 levels |
//! | Scope creation | ~100ns | Allocate new environment |
//!
//! # Memory Overhead
//!
//! - **ContextEnv**: ~48 bytes base + 24 bytes per context
//! - **Parent chain**: 16 bytes per level
//! - **Total**: < 200 bytes for typical 3-5 context application
//!
//! # Two-Level Context Model
//!
//! Verum provides TWO orthogonal dependency injection mechanisms:
//!
//! **Level 1 (Static Dependencies)**: `@injectable`/`@inject` attributes
//! - Resolution: Compile-time (AOT) or startup (JIT/Interpreter)
//! - Cost: **0ns** - direct struct field access
//! - Use case: Services, repositories, infrastructure components
//!
//! **Level 2 (Dynamic Contexts)**: `provide`/`using` keywords **(THIS MODULE)**
//! - Resolution: Runtime via task-local storage (θ)
//! - Cost: **~5-30ns lookup** (Tier 1-3), **~100ns** (Tier 0)
//! - Use case: Cross-cutting concerns (logging, metrics, auth, tracing)
//!
//! # NOT Algebraic Effects
//!
//! **Important**: Despite the name "Context System", this is **NOT** an algebraic
//! effects system. It does not provide:
//!
//! - ❌ Effect handlers
//! - ❌ Delimited continuations
//! - ❌ Resumable computations
//! - ❌ Effect polymorphism
//!
//! Instead, it provides:
//!
//! - ✅ Type-safe dependency injection
//! - ✅ Capability-based security
//! - ✅ Lexically-scoped providers
//! - ✅ Task-local storage
//! - ✅ Native async support
//!
//! # Distinction from Computational Properties
//!
//! **CRITICAL**: Do NOT confuse the Context System with computational properties tracking!
//!
//! | Aspect | Context System (DI) | Computational Properties |
//! |--------|---------------------|-------------------------|
//! | **Purpose** | Dependency injection | Track side effects (Pure/IO/Async) |
//! | **Keywords** | `context`, `provide`, `using` | None (inferred) |
//! | **When** | Runtime dependency resolution | Compile-time optimization |
//! | **Examples** | Logger, Database, Auth | Can this be memoized? Is it async? |
//! | **Module** | `verum_types::di` | `verum_types::contexts` |
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```rust
//! use verum_types::di::*;
//! use verum_types::Type;
//! use std::any::TypeId;
//!
//! // Create a context declaration
//! let mut logger_ctx = ContextDecl::new("Logger".into());
//! logger_ctx.add_operation(ContextOperation::new(
//!     "log".into(),
//!     vec![("level".into(), Type::Int), ("message".into(), Type::Text)],
//!     Type::Unit,
//!     false,
//! ));
//!
//! // Create a context requirement
//! let logger_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());
//! let requirement = ContextRequirement::single(logger_ref);
//!
//! // Create a context provider
//! let provider = ContextProvider::new(
//!     requirement.iter().next().unwrap().clone(),
//!     "console_logger()".into(),
//!     TypeId::of::<String>(),
//! );
//!
//! // Create a context environment
//! let mut env = ContextEnv::new();
//! // In real usage, you would insert the actual logger implementation
//! ```
//!
//! ## Context Groups
//!
//! ```rust
//! use verum_types::di::*;
//! use std::any::TypeId;
//!
//! let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
//! let database = ContextRef::new("Database".into(), TypeId::of::<String>());
//! let auth = ContextRef::new("Auth".into(), TypeId::of::<i32>());
//!
//! let web_context = ContextGroup::new(
//!     "WebContext".into(),
//!     vec![logger, database, auth]
//! );
//!
//! let requirement = web_context.expand();
//! assert_eq!(requirement.len(), 3);
//! ```
//!
//! ## Lexical Scoping
//!
//! ```rust
//! use verum_types::di::*;
//! use std::sync::Arc;
//!
//! // Parent scope
//! let mut parent = ContextEnv::new();
//! parent.insert("ParentLogger".to_string());
//!
//! // Child scope inherits from parent
//! let mut child = ContextEnv::with_parent(Arc::new(parent));
//! child.insert(42i32); // Different type
//!
//! // Can access parent's context
//! assert!(child.get_or_parent::<String>().is_some());
//! // And own context
//! assert!(child.get::<i32>().is_some());
//! ```
//!
//! # Module Structure
//!
//! - [`decl`]: Context declarations (`context Logger { ... }`)
//! - [`requirement`]: Context requirements (`using [Logger, Database]`)
//! - [`provider`]: Context providers (`provide Logger = ...`)
//! - [`env`]: Runtime context environment (θ, task-local storage)
//! - [`group`]: Context groups (`using WebContext = [...]`)

pub mod decl;
pub mod env;
pub mod group;
pub mod provider;
pub mod requirement;

// Re-export main types for convenience
pub use decl::{ContextDecl, ContextError, ContextOperation, TypeParam};
pub use env::{ContextEnv, SharedContextEnv};
pub use group::{ContextGroup, ContextGroupRegistry, GroupError};
pub use provider::{ContextProvider, ProviderError, ProviderScope};
pub use requirement::{ContextExpr, ContextRef, ContextRequirement};

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::ty::Type;
    use std::any::TypeId;

    /// Test complete workflow: declaration -> requirement -> provider -> environment
    #[test]
    fn test_complete_context_workflow() {
        // 1. Declare a context
        let mut logger_ctx = ContextDecl::new("Logger".into());
        logger_ctx.add_operation(ContextOperation::new(
            "log".into(),
            vec![("level".into(), Type::Int), ("message".into(), Type::Text)],
            Type::Unit,
            false,
        ));

        assert!(logger_ctx.validate().is_ok());

        // 2. Create a context requirement
        let logger_ref = ContextRef::new("Logger".into(), TypeId::of::<String>());
        let requirement = ContextRequirement::single(logger_ref.clone());

        assert_eq!(requirement.len(), 1);
        assert!(requirement.requires("Logger"));

        // 3. Create a provider
        let provider = ContextProvider::new(
            logger_ref,
            "console_logger()".into(),
            TypeId::of::<String>(),
        );

        assert!(provider.validate().is_ok());

        // 4. Set up runtime environment
        let mut env = ContextEnv::new();
        env.insert("ConsoleLogger".to_string());

        // 5. Check requirement is satisfied
        assert!(requirement.satisfies(&env));
    }

    /// Test context groups expanding to requirements
    #[test]
    fn test_group_to_requirement() {
        let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let database = ContextRef::new("Database".into(), TypeId::of::<String>());
        let auth = ContextRef::new("Auth".into(), TypeId::of::<i32>());

        let web_context = ContextGroup::new("WebContext".into(), vec![logger, database, auth]);

        assert_eq!(web_context.len(), 3);
        assert!(web_context.validate().is_ok());

        let requirement = web_context.expand();
        assert_eq!(requirement.len(), 3);
        assert!(requirement.requires("Logger"));
        assert!(requirement.requires("Database"));
        assert!(requirement.requires("Auth"));
    }

    /// Test lexical scoping with parent environments
    #[test]
    fn test_lexical_scoping() {
        use std::sync::Arc;

        // Global scope
        let mut global = ContextEnv::new();
        global.insert("GlobalLogger".to_string());

        // Module scope
        let mut module = ContextEnv::with_parent(Arc::new(global));
        module.insert(42i32); // Module-specific context

        // Function scope
        let function = ContextEnv::with_parent(Arc::new(module));

        // Function can access contexts from all scopes
        assert!(function.get_or_parent::<String>().is_some()); // Global logger
        assert!(function.get_or_parent::<i32>().is_some()); // Module context

        assert_eq!(function.depth(), 2); // Two parent levels
        assert_eq!(function.total_len(), 2); // Total contexts available
    }

    /// Test provider scope levels
    #[test]
    fn test_provider_scopes() {
        let logger_ref = ContextRef::new("Logger".into(), TypeId::of::<()>());

        let local = ContextProvider::new(
            logger_ref.clone(),
            "local_logger()".into(),
            TypeId::of::<String>(),
        );
        assert!(local.is_local());

        let module = ContextProvider::with_scope(
            logger_ref.clone(),
            "module_logger()".into(),
            TypeId::of::<String>(),
            ProviderScope::Module,
        );
        assert!(module.is_module());

        let global = ContextProvider::with_scope(
            logger_ref,
            "global_logger()".into(),
            TypeId::of::<String>(),
            ProviderScope::Global,
        );
        assert!(global.is_global());
    }

    /// Test async contexts
    #[test]
    fn test_async_contexts() {
        let mut db_ctx = ContextDecl::new("Database".into());
        db_ctx.add_operation(ContextOperation::new(
            "query".into(),
            vec![("sql".into(), Type::Text)],
            Type::Text,
            true, // async operation
        ));

        assert!(db_ctx.is_async);

        let db_ref = ContextRef::new("Database".into(), TypeId::of::<()>()).as_async();
        assert!(db_ref.is_async);

        let provider = ContextProvider::new(
            db_ref,
            "postgres_connection()".into(),
            TypeId::of::<String>(),
        )
        .as_async();
        assert!(provider.is_async);
        assert!(provider.validate().is_ok());
    }

    /// Test context group registry
    #[test]
    fn test_group_registry() {
        let mut registry = ContextGroupRegistry::new();

        let logger = ContextRef::new("Logger".into(), TypeId::of::<()>());
        let database = ContextRef::new("Database".into(), TypeId::of::<String>());

        let web_context = ContextGroup::new("WebContext".into(), vec![logger, database]);

        assert!(registry.register(web_context).is_ok());
        assert!(registry.has_group("WebContext"));

        let requirement = registry.expand("WebContext").unwrap();
        assert_eq!(requirement.len(), 2);
    }

    /// Test missing context detection
    #[test]
    fn test_missing_contexts() {
        let logger = ContextRef::new("Logger".into(), TypeId::of::<String>());
        let database = ContextRef::new("Database".into(), TypeId::of::<i32>());

        let requirement = ContextRequirement::from_contexts(vec![logger, database]);

        let mut env = ContextEnv::new();
        env.insert("ConsoleLogger".to_string()); // Only Logger provided

        assert!(!requirement.satisfies(&env)); // Database missing

        let missing = requirement.missing_contexts(&env);
        assert_eq!(missing.len(), 1);
        assert!(missing.iter().any(|n| n.as_str() == "Database"));
    }
}
