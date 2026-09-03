//! Context Group Resolution for Type Checking
//!
//! This module integrates context group expansion into the type checking phase.
//! It bridges the parser's ContextGroupDecl AST nodes with the type checker's
//! context requirement system.
//!
//! # Overview
//!
//! The parser can parse context group declarations:
//!
//! ```verum
//! using WebContext = [Database, Logger, FileSystem];
//! ```
//!
//! Or the alternative syntax:
//!
//! ```verum
//! context group WebContext {
//!  Database,
//!  Logger,
//!  FileSystem
//! }
//! ```
//!
//! When type checking a function that uses a context group:
//!
//! ```verum
//! fn handler() using WebContext { }
//! ```
//!
//! This module expands `WebContext` to `[Database, Logger, FileSystem]` and
//! validates that all referenced contexts exist.
//!
//! # Architecture
//!
//! 1. **Registration Phase**: During type checking of program items, context group
//!  declarations are registered in a ContextGroupRegistry.
//!
//! 2. **Resolution Phase**: When type checking function signatures, context
//!  requirements are resolved. If a requirement references a group name,
//!  it's expanded to the full list of contexts.
//!
//! 3. **Validation Phase**: Each expanded context is validated to ensure it
//!  references a valid context type.
//!
//! # Integration with Type Checker
//!
//! The TypeChecker maintains a ContextGroupRegistry field that stores all
//! defined context groups. During the check_item phase, ContextGroup items
//! populate this registry. During check_function, context requirements are
//! expanded using this registry.

use verum_ast::decl::{ContextGroupDecl, ContextRequirement as AstContextRequirement};
use verum_ast::span::{Span, Spanned};
use verum_common::well_known_types::WellKnownType as WKT;
use verum_common::{List, Map, Maybe, Set, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

use crate::di::group::{ContextGroup, ContextGroupRegistry, GroupError};
use crate::di::requirement::{ContextRef, ContextRequirement};
use crate::{Result, Type, TypeError};

/// Context resolution engine for type checking
///
/// This integrates the DI module's ContextGroupRegistry with the type checker.
/// It provides methods to:
/// - Register context groups from AST declarations
/// - Expand context references (including groups) into full requirements
/// - Validate that all referenced contexts exist
/// - Evaluate compile-time conditions for conditional contexts
///
/// Context system: capability-based dependency injection with "context" declarations, "using" requirements, "provide" injection, ~5-30ns runtime overhead via task-local storage — (advanced context patterns: negative contexts, transitive verification)
pub struct ContextResolver {
    /// Registry of all defined context groups
    registry: ContextGroupRegistry,

    /// Set of all defined context types (for validation)
    ///
    /// During type checking, we track context names (not runtime TypeIds) because:
    /// - TypeId is a runtime concept resolved during code generation
    /// - Type checking only needs to validate that context names are defined
    /// - Actual TypeIds are assigned when contexts are instantiated at runtime
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — Context Declarations
    defined_contexts: Set<Text>,

    /// Set of all defined constraint protocols (non-context protocols)
    /// Used to provide better error messages when a constraint protocol
    /// is mistakenly used in a `using [...]` clause
    constraint_protocols: Set<Text>,

    /// Protocol kind registry: maps protocol names to their ProtocolKind.
    /// Used for kind-based validation: Injectable cannot be used as type bound,
    /// Constraint cannot be used in using clause.
    protocol_kinds: verum_common::Map<Text, crate::protocol::ProtocolKind>,

    /// Map of context names to their types (for type environment)
    /// This stores the actual Type for each context so we can add them
    /// to the type environment when a function uses them
    context_types: Map<Text, Type>,

    /// Configuration environment for compile-time condition evaluation
    ///
    /// Stores configuration flags that can be used in conditional contexts:
    /// - `using [Analytics if cfg.analytics_enabled]`
    /// - `using [Profiler if DEBUG]`
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    config: condition_eval::ConfigEnv,

    /// When true, undefined contexts produce warnings instead of errors.
    /// This is enabled for files with `@test` annotations, since test
    /// scaffolding may provide contexts (Database, Logger, etc.) at runtime
    /// that are not visible during compilation.
    lenient_contexts: bool,
}

impl ContextResolver {
    /// Create a new context resolver
    pub fn new() -> Self {
        ContextResolver {
            registry: ContextGroupRegistry::new(),
            defined_contexts: Set::new(),
            constraint_protocols: Set::new(),
            protocol_kinds: verum_common::Map::new(),
            context_types: Map::new(),
            config: condition_eval::ConfigEnv::new(),
            lenient_contexts: false,
        }
    }

    /// Create a new context resolver with a specific configuration
    pub fn with_config(config: condition_eval::ConfigEnv) -> Self {
        ContextResolver {
            registry: ContextGroupRegistry::new(),
            defined_contexts: Set::new(),
            constraint_protocols: Set::new(),
            protocol_kinds: verum_common::Map::new(),
            context_types: Map::new(),
            config,
            lenient_contexts: false,
        }
    }

    /// Enable lenient context mode where undefined contexts produce warnings
    /// instead of errors. Used for `@test` annotated files where test harnesses
    /// may provide contexts (Database, Logger, Benchmark, etc.) at runtime.
    pub fn set_lenient_contexts(&mut self, lenient: bool) {
        self.lenient_contexts = lenient;
    }

    /// Check if lenient context mode is enabled.
    pub fn is_lenient_contexts(&self) -> bool {
        self.lenient_contexts
    }

    /// Check if a protocol name can be used as a type bound.
    /// Returns Err if the protocol is Injectable-only (context without protocol keyword).
    pub fn validate_as_type_bound(&self, name: &str, span: verum_ast::Span) -> Result<()> {
        let name_text = verum_common::Text::from(name);
        if let verum_common::Maybe::Some(kind) = self.protocol_kinds.get(&name_text) {
            if *kind == crate::protocol::ProtocolKind::Injectable {
                return Err(TypeError::Other(verum_common::Text::from(format!(
                    "context '{}' cannot be used as a type bound; \
                     contexts declared with `context {}` are injectable only.\n  \
                     help: use `using [{}]` for dependency injection, \
                     or declare as `context protocol {}` to enable both uses",
                    name, name, name, name
                ))));
            }
        }
        Ok(())
    }

    /// Get a reference to the configuration environment
    pub fn config(&self) -> &condition_eval::ConfigEnv {
        &self.config
    }

    /// Get a mutable reference to the configuration environment
    pub fn config_mut(&mut self) -> &mut condition_eval::ConfigEnv {
        &mut self.config
    }

    /// Set a configuration flag
    ///
    /// # Arguments
    ///
    /// * `name` - The flag name (e.g., "analytics_enabled")
    /// * `value` - The boolean value
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    pub fn set_config_flag(&mut self, name: impl Into<Text>, value: bool) {
        self.config.set_flag(name, value);
    }

    /// Check if a configuration flag is enabled
    ///
    /// # Arguments
    ///
    /// * `name` - The flag name to check
    ///
    /// # Returns
    ///
    /// `Some(true)` if enabled, `Some(false)` if explicitly disabled,
    /// `None` if the flag is not defined
    pub fn is_config_flag_enabled(&self, name: &str) -> Option<bool> {
        self.config.get_flag(name)
    }

    /// Register a context type as defined
    ///
    /// This should be called when processing context declarations:
    ///
    /// ```verum
    /// context Database {
    ///  fn query(...) -> ...;
    /// }
    /// ```
    ///
    /// # Arguments
    ///
    /// * `name` - The context type name (e.g., "Database")
    /// * `ty` - The type of the context (typically a Record with method fields)
    pub fn register_context_type(&mut self, name: Text, ty: Type) {
        self.defined_contexts.insert(name.clone());
        self.context_types.insert(name, ty);
    }

    /// Register a protocol as a valid context type.
    ///
    /// In Verum's dependency injection system, protocols can serve as context types.
    /// This enables patterns like:
    ///
    /// ```verum
    /// type Database is protocol {
    ///  async fn query(self, sql: Text) -> Result<List<Row>, Error>;
    /// }
    ///
    /// pub async fn get_user(id: Int) -> Result<User, Error>
    ///  using [Database] // Database protocol as context
    /// {
    ///  Database.query(f"SELECT * FROM users WHERE id = {id}").await
    /// }
    /// ```
    ///
    /// This is essential for cross-file context resolution where protocols are
    /// defined in one module and used in `using` clauses in another.
    ///
    /// # Arguments
    ///
    /// * `name` - The protocol name (e.g., "Database", "Storage", "Auth")
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
    pub fn register_protocol_as_context(&mut self, name: Text) {
        self.defined_contexts.insert(name.clone());
        self.protocol_kinds
            .insert(name, crate::protocol::ProtocolKind::ConstraintAndInjectable);
    }

    /// Register multiple protocols as valid context types.
    ///
    /// Convenience method for registering multiple protocols from module exports.
    ///
    /// # Arguments
    ///
    /// * `names` - Iterator of protocol names to register
    pub fn register_protocols_as_contexts<I>(&mut self, names: I)
    where
        I: IntoIterator<Item = Text>,
    {
        for name in names {
            self.register_protocol_as_context(name);
        }
    }

    /// Register a protocol as a constraint protocol (not injectable).
    ///
    /// Constraint protocols are used in `where T: Protocol` bounds but cannot be
    /// used in `using [Protocol]` dependency injection clauses.
    ///
    /// This is used to provide better error messages when a constraint protocol
    /// is mistakenly used in a `using [...]` clause.
    ///
    /// # Arguments
    ///
    /// * `name` - The protocol name (e.g., "Comparable", "Hashable")
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Context Protocol Validation
    pub fn register_constraint_protocol(&mut self, name: Text) {
        self.constraint_protocols.insert(name.clone());
        self.protocol_kinds
            .insert(name, crate::protocol::ProtocolKind::Constraint);
    }

    /// Check if a name is a known constraint protocol (not injectable).
    ///
    /// # Arguments
    ///
    /// * `name` - The name to check
    ///
    /// # Returns
    ///
    /// `true` if the name is a constraint protocol, `false` otherwise
    pub fn is_constraint_protocol(&self, name: &Text) -> bool {
        self.constraint_protocols.contains(name)
    }

    /// Check if a context name is registered (either as context or protocol).
    ///
    /// # Arguments
    ///
    /// * `name` - The context name to check
    ///
    /// # Returns
    ///
    /// `true` if the context is registered, `false` otherwise
    pub fn is_context_defined(&self, name: &Text) -> bool {
        self.defined_contexts.contains(name)
    }

    /// Get all registered context names (for diagnostics).
    ///
    /// # Returns
    ///
    /// List of all registered context and protocol names
    pub fn all_context_names(&self) -> List<&Text> {
        self.defined_contexts.iter().collect()
    }

    /// Get the type of a registered context
    ///
    /// # Arguments
    ///
    /// * `name` - The context name to look up
    ///
    /// # Returns
    ///
    /// `Maybe::Some(Type)` if the context is registered, `Maybe::None` otherwise
    pub fn get_context_type(&self, name: &Text) -> Maybe<&Type> {
        self.context_types.get(name)
    }

    /// Register a context group from an AST declaration
    ///
    /// Converts a parser-level ContextGroupDecl into a runtime ContextGroup
    /// and registers it in the registry.
    ///
    /// # Arguments
    ///
    /// * `decl` - The AST context group declaration
    ///
    /// # Returns
    ///
    /// `Ok(())` if successful, `Err(TypeError)` if the group is invalid
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The group name is already defined
    /// - The group is empty
    /// - The group contains duplicate contexts
    pub fn register_group(&mut self, decl: &ContextGroupDecl) -> Result<()> {
        // Convert AST context requirements to ContextRefs
        //

        // During type checking phase, we use a sentinel TypeId (unit type) because:
        // 1. Type checking validates context names exist in the scope
        // 2. Actual TypeIds are runtime artifacts created during code generation
        // 3. The ContextRef structure requires a TypeId for runtime dispatch,
        //  but type checking only needs name-based validation
        //

        // At runtime, the context provider resolves actual TypeIds via:
        //  provider.get::<T>() where T is the concrete context type
        //

        // Context requirements: functions declare needed contexts with "using [Ctx1, Ctx2]" after return type, callers must provide all — Context Resolution
        let sentinel_type_id = std::any::TypeId::of::<()>();
        let contexts: List<ContextRef> = decl
            .contexts
            .iter()
            .map(|ctx| {
                // Extract the context name from the path
                // For paths like "Database" or "State<Int>", get the first segment name
                use verum_ast::ty::PathSegment;
                let name = ctx
                    .path
                    .segments
                    .first()
                    .map(|seg| match seg {
                        PathSegment::Name(ident) => ident.name.to_string(),
                        PathSegment::SelfValue => "self".to_string(),
                        PathSegment::Super => "super".to_string(),
                        PathSegment::Cog => "cog".to_string(),
                        PathSegment::Relative => ".".to_string(),
                    })
                    .unwrap_or_else(|| "unknown".to_string());

                // Extract type arguments from the context requirement
                // e.g., State<Int> -> type_args = ["Int"], Cache<User> -> type_args = ["User"]
                let type_args: List<Text> = ctx.args.iter().map(type_to_text).collect();

                // Build the ContextRef with appropriate flags for negation and type args
                if !type_args.is_empty() {
                    // Parameterized context (e.g., State<Int>, Cache<User>)
                    let mut ctx_ref =
                        ContextRef::with_type_args(name.into(), sentinel_type_id, type_args);
                    // Propagate negation flag for parameterized negative contexts
                    // e.g., `!State<_>` in `using Pure = [!IO, !State<_>]`
                    ctx_ref.is_negative = ctx.is_negative;
                    ctx_ref
                } else if ctx.is_negative {
                    // Simple negative context (e.g., !IO, !Database)
                    ContextRef::negative(name.into(), sentinel_type_id)
                } else {
                    // Simple positive context (e.g., Database, Logger)
                    ContextRef::new(name.into(), sentinel_type_id)
                }
            })
            .collect();

        // Create the context group
        let group = ContextGroup::new(decl.name.name.clone(), contexts);

        // Validate the group
        if let Err(err) = group.validate() {
            return Err(TypeError::UndefinedContext {
                name: format!("Invalid context group: {}", err).into(),
                span: decl.span,
            });
        }

        // Register the group
        if let Err(err) = self.registry.register(group) {
            return Err(TypeError::UndefinedContext {
                name: format!("Failed to register context group: {}", err).into(),
                span: decl.span,
            });
        }

        Ok(())
    }

    /// Resolve a context requirement, expanding any group references
    ///
    /// If the requirement references a single identifier that matches a
    /// registered group name, expand it to the group's contexts.
    /// Otherwise, return the contexts as-is.
    ///
    /// # Arguments
    ///
    /// * `req` - The AST context requirement to resolve
    ///
    /// # Returns
    ///
    /// A fully expanded `ContextRequirement` with all groups resolved
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A referenced group doesn't exist
    /// - A referenced context type doesn't exist
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Given: using WebContext = [Database, Logger]
    /// // Input: using WebContext
    /// // Output: ContextRequirement { Database, Logger }
    ///
    /// // Input: using [Database, Logger]
    /// // Output: ContextRequirement { Database, Logger }
    /// ```
    pub fn resolve_requirement(
        &self,
        contexts: &[AstContextRequirement],
        span: Span,
    ) -> Result<ContextRequirement> {
        // If the list has exactly one context and it's a simple path (no generics),
        // check if it's a group name
        if contexts.len() == 1 {
            let ctx = &contexts[0];
            if ctx.args.is_empty() {
                // Simple path - might be a group name
                let name = path_to_string(&ctx.path);

                // Try to expand as a group first
                if let Option::Some(group) = self.registry.get(name.as_str()) {
                    // It's a group - expand it
                    let requirement = group.expand();

                    // Validate all contexts in the group
                    // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
                    // Both signs are validated. An exclusion names a
                    // context just as a requirement does, and a
                    // misspelled exclusion (`!Loggr`) forbids nothing
                    // while reading as a guarantee — the failure mode
                    // that only exists once exclusions are enforced.
                    for ctx_ref in requirement.iter() {
                        self.validate_context(&ctx_ref.name, span)?;
                    }

                    return Ok(requirement);
                }
            }
        }

        // Not a group reference - process as individual contexts
        // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1 - Advanced context patterns
        let mut requirement = ContextRequirement::empty();
        let mut used_aliases: Set<Text> = Set::new(); // Track aliases for duplicate detection

        for ctx in contexts {
            let name = path_to_string(&ctx.path);

            // Check if this is a group (groups cannot be used in lists)
            if self.registry.has_group(name.as_str()) {
                return Err(TypeError::UndefinedContext {
                    name: format!(
                        "Context group '{}' cannot be used in a list. Use it directly: 'using {}'",
                        name, name
                    )
                    .into(),
                    span,
                });
            }

            // Validate existence for BOTH signs. Skipping the
            // negative side was free while exclusions were inert; now
            // that they are enforced, an unchecked name turns a typo
            // into a silent permission of exactly what the author
            // wrote the exclusion to forbid.
            // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
            self.validate_context(&name, span)?;

            // Validate alias uniqueness
            // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
            let effective_alias: Option<Text> = match (&ctx.alias, &ctx.name) {
                (Some(alias), _) => Some(alias.name.clone()),
                (_, Some(name_binding)) => Some(name_binding.name.clone()),
                _ => None,
            };

            if let Some(ref alias_name) = effective_alias {
                if used_aliases.contains(alias_name) {
                    return Err(TypeError::Other(
                        format!(
                            "Duplicate context alias '{}'. Each alias must be unique.",
                            alias_name
                        )
                        .into(),
                    ));
                }
                used_aliases.insert(alias_name.clone());
            }

            // Create a ContextRef with all the advanced pattern fields
            let type_id = std::any::TypeId::of::<()>();
            let mut ctx_ref = ContextRef::new(name.clone(), type_id);

            // ============================================================================
            // CRITICAL FIX: Copy type arguments for generic contexts
            // ============================================================================
            // For generic contexts like `Cache<Text, User>` in using clauses,
            // we need to preserve the type arguments so they can be used for:
            // 1. Type instantiation when the context is provided
            // 2. Method resolution with substituted type parameters
            //

            // Context provision: "provide ContextName = implementation" installs a provider in lexical scope via task-local storage (theta) — Parameterized Contexts
            // ============================================================================
            if !ctx.args.is_empty() {
                // Convert AST types to text representation for storage
                // At runtime, these will be resolved to actual types
                let type_args: List<Text> = ctx.args.iter().map(type_to_text).collect();
                ctx_ref.type_args = type_args;
            }

            // Set negative flag
            ctx_ref.is_negative = ctx.is_negative;

            // Set alias from either `as alias` or `name:` syntax
            if let Some(ref alias) = ctx.alias {
                ctx_ref.alias = Maybe::Some(alias.name.clone());
            } else if let Some(ref name_binding) = ctx.name {
                ctx_ref.alias = Maybe::Some(name_binding.name.clone());
            }

            // Process transforms
            // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.3 - Context Transformations
            if !ctx.transforms.is_empty() {
                // NOTE: transform names are NOT validated here. The
                // check that used to live at this line consulted a
                // hardcoded nine-name vocabulary rather than the context,
                // and is gone (see the transforms module, T0938). The
                // honest check — does the context DECLARE a method of
                // this name — needs a populated context registry (T0939).

                // Store validated transforms
                let transforms: List<crate::di::requirement::ContextTransformRef> = ctx
                    .transforms
                    .iter()
                    .map(|t| crate::di::requirement::ContextTransformRef {
                        name: t.name.name.clone(),
                        args: t.args.iter().map(|a| format!("{:?}", a).into()).collect(),
                    })
                    .collect();
                ctx_ref.transforms = transforms;
            }

            // Process condition - evaluate at compile-time
            // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
            if let Some(ref cond_expr) = ctx.condition {
                // Evaluate the condition at compile-time using the configuration environment
                match condition_eval::should_include_context(&ctx.condition, &self.config) {
                    Ok(true) => {
                        // Condition is true - include this context
                        // Store condition for diagnostics/debugging
                        ctx_ref.condition = Maybe::Some(format!("{:?}", cond_expr).into());
                    }
                    Ok(false) => {
                        // Condition is false - skip this context entirely
                        // The context is not required when the condition is not met
                        continue;
                    }
                    Err(error_msg) => {
                        // Error evaluating condition - report as type error
                        return Err(TypeError::Other(
                            format!(
                                "Failed to evaluate compile-time condition for context '{}': {}",
                                name, error_msg
                            )
                            .into(),
                        ));
                    }
                }
            }

            requirement.add_context(ctx_ref);
        }

        Ok(requirement)
    }

    /// Validate that a context type is defined
    ///
    /// # Arguments
    ///
    /// * `name` - The context name to validate
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// `Ok(())` if the context exists, `Err(TypeError)` otherwise
    ///
    /// # Errors
    ///
    /// Returns:
    /// - `TypeError::NonContextProtocolInUsing` if the name is a constraint protocol
    ///  (not declared with `context protocol`)
    /// - `TypeError::UndefinedContext` if the name is not a known context or protocol
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Context Protocol Validation
    fn validate_context(&self, name: &Text, span: Span) -> Result<()> {
        if !self.defined_contexts.contains(name) {
            // Check if this is a constraint protocol that was incorrectly used
            if self.constraint_protocols.contains(name) {
                return Err(TypeError::NonContextProtocolInUsing {
                    name: name.clone(),
                    span,
                });
            }
            // In lenient mode (e.g., @test annotated files), treat undefined
            // contexts as warnings rather than errors. Test harnesses typically
            // provide common contexts (Database, Logger, Benchmark, etc.) at
            // runtime that are not visible during compilation.
            if self.lenient_contexts {
                // Silently accept — the context will be provided at runtime
                return Ok(());
            }
            return Err(TypeError::UndefinedContext {
                name: name.clone(),
                span,
            });
        }
        Ok(())
    }

    /// True when `name` is already a registered context (any kind).
    pub fn is_defined_context(&self, name: &str) -> bool {
        self.defined_contexts.contains(&Text::from(name))
    }

    /// Get all registered group names (for diagnostics)
    pub fn group_names(&self) -> List<&Text> {
        self.registry.group_names()
    }

    /// Check if a name is a registered group
    pub fn is_group(&self, name: &str) -> bool {
        self.registry.has_group(name)
    }

    /// Get a group by name (for diagnostics)
    pub fn get_group(&self, name: &str) -> Maybe<&ContextGroup> {
        self.registry.get(name)
    }

    /// Clear all registered groups (for testing)
    #[cfg(test)]
    pub fn clear(&mut self) {
        self.registry.clear();
        self.defined_contexts.clear();
        self.context_types.clear();
    }
}

impl Default for ContextResolver {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Context Transform Validation
// =============================================================================

// =============================================================================
// Context transform validation — REMOVED (T0938)
// =============================================================================
//
// What stood here: a `transforms` module of ~490 lines holding a
// nine-name vocabulary (`STANDARD_TRANSFORMS`), a table mapping three of
// those names to the context NAMES they were allowed on, matched by
// SUBSTRING, and per-name argument signatures naming stdlib types
// (`Duration`, `EncryptionKey`, `CacheConfig`) — plus two validators
// built on it.
//
// It violated the standing rule in crates/verum_types/src/CLAUDE.md: the
// compiler must hold ZERO hardcoded knowledge of stdlib types. It also
// gave three wrong answers, each measured on a release binary before
// removal:
//
//   * `Db.readonly()` was REFUSED as an "unknown transform", though a
//     user context is free to declare a `readonly` method.
//   * `Db.cached()` was ACCEPTED even though `Db` declares no `cached`
//     method — the check consulted the vocabulary, not the context.
//   * `Db.transactional()` was REFUSED because the string "Db" does not
//     contain "Database", while a context named `MyDatabaseThing` would
//     have passed.
//
// And it contradicted the LOWERING. verum_vbc's `emit_context_transforms`
// treats a transform as an ordinary method — CtxGet the context, call the
// named method, CtxProvide the result — and special-cases none of the
// nine names. Nothing downstream ever needed the list.
//
// Two of its three validators were reachable ONLY from tests
// (`validate_transforms_with_args`, `validate_ast_transform`): a rule
// nobody asks, kept alive by its own test.
//
// The correct check is that the context DECLARES a method of that name.
// It is not written here because neither context registry is populated on
// the production compile path — `register_context_type` and
// `ContextChecker::register_context` are called only from tests. That is
// T0939.





// =============================================================================
// Conditional Context Compile-Time Evaluation
// =============================================================================

/// Compile-time condition evaluator for conditional contexts
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
///
/// Evaluates conditions like:
/// - `cfg.feature_enabled` - Configuration flags
/// - `T: Protocol` - Type constraint checking
/// - Boolean literals and expressions
pub mod condition_eval {
    use super::*;
    use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
    use verum_ast::literal::{Literal, LiteralKind};
    use verum_common::Heap;

    /// Configuration environment for compile-time evaluation
    #[derive(Debug, Clone, Default)]
    pub struct ConfigEnv {
        /// Configuration flags (key -> value)
        flags: Map<Text, bool>,
        /// String configuration values
        values: Map<Text, Text>,
    }

    impl ConfigEnv {
        pub fn new() -> Self {
            Self::default()
        }

        /// Set a boolean configuration flag
        pub fn set_flag(&mut self, name: impl Into<Text>, value: bool) {
            self.flags.insert(name.into(), value);
        }

        /// Get a boolean configuration flag
        pub fn get_flag(&self, name: &str) -> Option<bool> {
            self.flags.get(&Text::from(name)).copied()
        }

        /// Set a string configuration value
        pub fn set_value(&mut self, name: impl Into<Text>, value: impl Into<Text>) {
            self.values.insert(name.into(), value.into());
        }

        /// Get a string configuration value
        pub fn get_value(&self, name: &str) -> Option<&Text> {
            self.values.get(&Text::from(name))
        }
    }

    // =========================================================================
    // Type Constraint Environment (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // =========================================================================

    /// Type constraint environment for compile-time type bound evaluation.
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    ///
    /// This environment tracks type parameters and their protocol bounds,
    /// enabling compile-time evaluation of conditions like `T: Protocol`.
    ///
    /// # Example
    ///
    /// ```verum
    /// fn foo<T>() using [Validator if T: Validatable] {
    ///  // Validator only available when T implements Validatable
    /// }
    /// ```
    ///
    /// During type checking, when T is instantiated with a concrete type,
    /// we can evaluate `T: Validatable` at compile time.
    #[derive(Debug, Clone, Default)]
    pub struct TypeConstraintEnv {
        /// Type parameter to protocol bounds mapping.
        /// Key: type parameter name (e.g., "T")
        /// Value: list of protocol names the type is known to implement
        type_bounds: Map<Text, List<Text>>,
        /// Concrete type substitutions for type parameters.
        /// Key: type parameter name (e.g., "T")
        /// Value: concrete type name (e.g., "User")
        concrete_types: Map<Text, Text>,
        /// Protocol checker reference for checking protocol implementations
        /// on concrete types. Uses interior mutability pattern.
        protocol_impls: Map<Text, Set<Text>>,
    }

    impl TypeConstraintEnv {
        /// Create a new empty type constraint environment
        pub fn new() -> Self {
            Self::default()
        }

        /// Register a type parameter with its protocol bounds.
        ///
        /// Called when entering a generic function scope with type parameters.
        ///
        /// # Arguments
        ///
        /// * `type_param` - Name of the type parameter (e.g., "T")
        /// * `bounds` - List of protocol names the type is bounded by
        pub fn register_type_param(&mut self, type_param: impl Into<Text>, bounds: List<Text>) {
            self.type_bounds.insert(type_param.into(), bounds);
        }

        /// Register a concrete type substitution for a type parameter.
        ///
        /// Called during monomorphization when a generic function is
        /// instantiated with concrete types.
        ///
        /// # Arguments
        ///
        /// * `type_param` - Name of the type parameter (e.g., "T")
        /// * `concrete_type` - Name of the concrete type (e.g., "User")
        pub fn substitute_type(
            &mut self,
            type_param: impl Into<Text>,
            concrete_type: impl Into<Text>,
        ) {
            self.concrete_types
                .insert(type_param.into(), concrete_type.into());
        }

        /// Register a protocol implementation for a concrete type.
        ///
        /// This allows the constraint checker to know which protocols
        /// a concrete type implements.
        ///
        /// # Arguments
        ///
        /// * `type_name` - Name of the concrete type (e.g., "User")
        /// * `protocol` - Name of the protocol implemented (e.g., "Validatable")
        pub fn register_impl(&mut self, type_name: impl Into<Text>, protocol: impl Into<Text>) {
            let type_key = type_name.into();
            let protocol_name = protocol.into();

            if let Some(impls) = self.protocol_impls.get_mut(&type_key) {
                impls.insert(protocol_name);
            } else {
                let mut impls = Set::new();
                impls.insert(protocol_name);
                self.protocol_impls.insert(type_key, impls);
            }
        }

        /// Check if a type parameter satisfies a protocol bound.
        ///
        /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
        ///
        /// # Arguments
        ///
        /// * `type_param` - Name of the type parameter to check
        /// * `protocol` - Protocol name to check against
        ///
        /// # Returns
        ///
        /// - `Some(true)` if the bound is definitely satisfied
        /// - `Some(false)` if the bound is definitely not satisfied
        /// - `None` if the bound cannot be determined at compile time
        pub fn check_type_bound(&self, type_param: &str, protocol: &str) -> Option<bool> {
            let type_key = Text::from(type_param);
            let protocol_key = Text::from(protocol);

            // Case 1: Type parameter has explicit bounds
            if let Some(bounds) = self.type_bounds.get(&type_key) {
                // If protocol is in the bounds, the constraint is satisfied
                if bounds.iter().any(|b| b.as_str() == protocol) {
                    return Some(true);
                }
            }

            // Case 2: Type parameter has been substituted with a concrete type
            if let Some(concrete_type) = self.concrete_types.get(&type_key) {
                // Check if the concrete type implements the protocol
                if let Some(impls) = self.protocol_impls.get(concrete_type) {
                    return Some(impls.contains(&protocol_key));
                }
                // Concrete type exists but we don't know its implementations
                // This could happen if protocol registration is incomplete
                return None;
            }

            // Case 3: Type parameter exists but bound is not in its declared bounds
            if self.type_bounds.contains_key(&type_key) {
                // Type param exists but doesn't have this bound declared
                // Cannot determine without concrete type instantiation
                return None;
            }

            // Case 4: Unknown type parameter - treat as runtime
            None
        }

        /// Check if a type parameter is known (registered)
        pub fn has_type_param(&self, name: &str) -> bool {
            self.type_bounds.contains_key(&Text::from(name))
                || self.concrete_types.contains_key(&Text::from(name))
        }

        /// Get the concrete type for a type parameter if substituted
        pub fn get_concrete_type(&self, type_param: &str) -> Option<&Text> {
            self.concrete_types.get(&Text::from(type_param))
        }

        /// Get the declared bounds for a type parameter
        pub fn get_bounds(&self, type_param: &str) -> Option<&List<Text>> {
            self.type_bounds.get(&Text::from(type_param))
        }

        /// Clear all registered type parameters (for reuse)
        #[cfg(test)]
        pub fn clear(&mut self) {
            self.type_bounds.clear();
            self.concrete_types.clear();
            self.protocol_impls.clear();
        }
    }

    /// Result of compile-time condition evaluation
    #[derive(Debug, Clone)]
    pub enum ConditionResult {
        /// Condition evaluated to a known value
        Known(bool),
        /// Condition depends on runtime values (cannot evaluate at compile time)
        Runtime,
        /// Condition could not be evaluated due to an error
        Error(Text),
    }

    impl ConditionResult {
        pub fn is_known_true(&self) -> bool {
            matches!(self, ConditionResult::Known(true))
        }

        pub fn is_known_false(&self) -> bool {
            matches!(self, ConditionResult::Known(false))
        }

        pub fn is_runtime(&self) -> bool {
            matches!(self, ConditionResult::Runtime)
        }
    }

    /// Evaluate a condition expression at compile time
    ///
    /// # Arguments
    ///
    /// * `expr` - The condition expression to evaluate
    /// * `config` - Configuration environment for cfg lookups
    ///
    /// # Returns
    ///
    /// `ConditionResult` indicating if the condition is known true/false,
    /// or if it depends on runtime values
    pub fn evaluate_condition(expr: &Expr, config: &ConfigEnv) -> ConditionResult {
        // Delegate to the full version with an empty type constraint environment
        evaluate_condition_with_types(expr, config, &TypeConstraintEnv::new())
    }

    /// Evaluate a condition expression at compile time with type constraint checking.
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    ///
    /// This is the full version of condition evaluation that supports:
    /// - Configuration flags (`cfg.feature_enabled`)
    /// - Type constraint conditions (`T: Protocol`)
    /// - Boolean expressions
    ///
    /// # Arguments
    ///
    /// * `expr` - The condition expression to evaluate
    /// * `config` - Configuration environment for cfg lookups
    /// * `type_env` - Type constraint environment for protocol bound checking
    ///
    /// # Returns
    ///
    /// `ConditionResult` indicating if the condition is known true/false,
    /// or if it depends on runtime values
    ///
    /// # Example
    ///
    /// ```verum
    /// fn foo<T>() using [Validator if T: Validatable] {
    ///  // Validator only available when T implements Validatable
    /// }
    /// ```
    ///
    /// When `foo` is instantiated with `User` which implements `Validatable`,
    /// the condition evaluates to `Known(true)`.
    pub fn evaluate_condition_with_types(
        expr: &Expr,
        config: &ConfigEnv,
        type_env: &TypeConstraintEnv,
    ) -> ConditionResult {
        match &expr.kind {
            // Boolean literals: true, false
            ExprKind::Literal(Literal {
                kind: LiteralKind::Bool(value),
                ..
            }) => ConditionResult::Known(*value),

            // Path expressions: could be cfg.something or a type constraint
            ExprKind::Path(path) => evaluate_path_condition(path, config),

            // Field access: cfg.feature or config.enabled
            ExprKind::Field { expr, field } => {
                if is_cfg_access(expr) {
                    let flag_name = field.name.as_str();
                    match config.get_flag(flag_name) {
                        Some(value) => ConditionResult::Known(value),
                        // ================================================================
                        // CRITICAL FIX: Treat unknown config flags as Runtime conditions
                        // ================================================================
                        // Unknown configuration flags are deferred to runtime evaluation.
                        // This allows code to typecheck even when config values aren't
                        // known at compile time (e.g., during testing or when configs
                        // are provided at deployment time).
                        //

                        // At runtime, if the flag is not set, the conditional context
                        // will not be included.
                        //

                        // Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
                        // ================================================================
                        None => ConditionResult::Runtime,
                    }
                } else {
                    // Unknown field access - treat as runtime
                    ConditionResult::Runtime
                }
            }

            // Binary operators: and, or
            ExprKind::Binary { op, left, right } => {
                match op {
                    BinOp::And => {
                        let left_result = evaluate_condition_with_types(left, config, type_env);
                        let right_result = evaluate_condition_with_types(right, config, type_env);

                        // Short-circuit: false && _ = false
                        if left_result.is_known_false() {
                            return ConditionResult::Known(false);
                        }
                        // true && x = x
                        if left_result.is_known_true() {
                            return right_result;
                        }
                        // Unknown && false = false
                        if right_result.is_known_false() {
                            return ConditionResult::Known(false);
                        }
                        ConditionResult::Runtime
                    }
                    BinOp::Or => {
                        let left_result = evaluate_condition_with_types(left, config, type_env);
                        let right_result = evaluate_condition_with_types(right, config, type_env);

                        // Short-circuit: true || _ = true
                        if left_result.is_known_true() {
                            return ConditionResult::Known(true);
                        }
                        // false || x = x
                        if left_result.is_known_false() {
                            return right_result;
                        }
                        // Unknown || true = true
                        if right_result.is_known_true() {
                            return ConditionResult::Known(true);
                        }
                        ConditionResult::Runtime
                    }
                    _ => ConditionResult::Runtime,
                }
            }

            // Unary not
            ExprKind::Unary { op, expr: inner } => {
                if matches!(op, UnOp::Not) {
                    match evaluate_condition_with_types(inner, config, type_env) {
                        ConditionResult::Known(value) => ConditionResult::Known(!value),
                        other => other,
                    }
                } else {
                    ConditionResult::Runtime
                }
            }

            // Is expression: T is Protocol (type constraint check)
            // This is an alternative syntax for type bounds
            ExprKind::Is {
                expr: type_expr,
                pattern,
                negated,
            } => evaluate_is_expression(type_expr, pattern, *negated, type_env),

            // Parenthesized expression
            ExprKind::Paren(inner) => evaluate_condition_with_types(inner, config, type_env),

            // All other expressions are treated as runtime
            _ => ConditionResult::Runtime,
        }
    }

    /// Evaluate a type constraint expression: `T: Protocol`
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    ///
    /// # Arguments
    ///
    /// * `type_expr` - The type parameter expression (e.g., `T`)
    /// * `protocol_expr` - The protocol expression (e.g., `Validatable`)
    /// * `type_env` - Type constraint environment
    /// * `negated` - Whether the constraint is negated (`T: !Protocol`)
    ///
    /// # Returns
    ///
    /// `ConditionResult` based on whether the type satisfies the protocol bound
    fn evaluate_type_constraint(
        type_expr: &Expr,
        protocol_expr: &Expr,
        type_env: &TypeConstraintEnv,
        negated: bool,
    ) -> ConditionResult {
        // Extract type parameter name from left side
        let type_param = match &type_expr.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        ident.name.as_str()
                    } else {
                        return ConditionResult::Runtime;
                    }
                } else {
                    return ConditionResult::Runtime;
                }
            }
            _ => return ConditionResult::Runtime,
        };

        // Extract protocol name from right side
        let protocol = match &protocol_expr.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        ident.name.as_str()
                    } else {
                        return ConditionResult::Runtime;
                    }
                } else {
                    // Multi-segment path like std::ops::Add
                    match extract_path_name(path) {
                        Some(name) => name,
                        None => return ConditionResult::Runtime,
                    }
                }
            }
            _ => return ConditionResult::Runtime,
        };

        // Check the type constraint in the environment
        match type_env.check_type_bound(type_param, protocol) {
            Some(result) => {
                let final_result = if negated { !result } else { result };
                ConditionResult::Known(final_result)
            }
            None => {
                // Cannot determine at compile time
                ConditionResult::Runtime
            }
        }
    }

    /// Evaluate an `is` expression: `T is Protocol`
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.1 - Conditional Contexts
    ///
    /// This function handles type constraint conditions in conditional contexts.
    /// The pattern can be a Variant pattern (for protocol/type names) or an
    /// identifier pattern.
    ///
    /// # Arguments
    ///
    /// * `type_expr` - The type expression being tested
    /// * `pattern` - The pattern (protocol) being matched
    /// * `negated` - Whether the expression is negated (`T is not Protocol`)
    /// * `type_env` - Type constraint environment
    ///
    /// # Returns
    ///
    /// `ConditionResult` based on whether the type matches the pattern
    fn evaluate_is_expression(
        type_expr: &Expr,
        pattern: &verum_ast::pattern::Pattern,
        negated: bool,
        type_env: &TypeConstraintEnv,
    ) -> ConditionResult {
        use verum_ast::pattern::PatternKind;

        // Extract type parameter name from the expression
        let type_param = match &type_expr.kind {
            ExprKind::Path(path) => {
                if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        ident.name.as_str()
                    } else {
                        return ConditionResult::Runtime;
                    }
                } else {
                    return ConditionResult::Runtime;
                }
            }
            _ => return ConditionResult::Runtime,
        };

        // Extract protocol name from the pattern
        // In Verum, `T is Validatable` uses Variant pattern for the protocol name
        let protocol = match &pattern.kind {
            // Variant pattern: Some(x), Validatable, etc.
            PatternKind::Variant { path, .. } => {
                if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        ident.name.as_str()
                    } else {
                        return ConditionResult::Runtime;
                    }
                } else {
                    match extract_path_name(path) {
                        Some(name) => name,
                        None => return ConditionResult::Runtime,
                    }
                }
            }
            // Record pattern (for protocol types): Protocol { method1, method2 }
            PatternKind::Record { path, .. } => {
                if path.segments.len() == 1 {
                    if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                        ident.name.as_str()
                    } else {
                        return ConditionResult::Runtime;
                    }
                } else {
                    match extract_path_name(path) {
                        Some(name) => name,
                        None => return ConditionResult::Runtime,
                    }
                }
            }
            // Identifier pattern: just a name like Validatable
            PatternKind::Ident { name, .. } => name.name.as_str(),
            // All other patterns are not type constraints
            _ => return ConditionResult::Runtime,
        };

        // Check the type constraint in the environment
        match type_env.check_type_bound(type_param, protocol) {
            Some(result) => {
                let final_result = if negated { !result } else { result };
                ConditionResult::Known(final_result)
            }
            None => {
                // Cannot determine at compile time
                ConditionResult::Runtime
            }
        }
    }

    /// Extract the name from a multi-segment path
    fn extract_path_name(path: &verum_ast::ty::Path) -> Option<&str> {
        // For paths like std::ops::Add, return "Add"
        if let Some(last_segment) = path.segments.last() {
            if let verum_ast::ty::PathSegment::Name(ident) = last_segment {
                return Some(ident.name.as_str());
            }
        }
        None
    }

    /// Extract the name from an AST type path
    fn extract_ast_type_path_name(path: &verum_ast::ty::Path) -> Option<&str> {
        // For paths like std::ops::Add, return "Add"
        if let Some(last_segment) = path.segments.last() {
            if let verum_ast::ty::PathSegment::Name(ident) = last_segment {
                return Some(ident.name.as_str());
            }
        }
        None
    }

    /// Check if an expression is `cfg`
    fn is_cfg_access(expr: &Expr) -> bool {
        if let ExprKind::Path(path) = &expr.kind {
            if path.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    return ident.name.as_str() == "cfg";
                }
            }
        }
        false
    }

    /// Evaluate a path-based condition
    fn evaluate_path_condition(path: &verum_ast::ty::Path, config: &ConfigEnv) -> ConditionResult {
        // Single-segment path might be a cfg flag or constant
        if path.segments.len() == 1 {
            if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                let name = ident.name.as_str();
                // Check if it's a known config flag
                if let Some(value) = config.get_flag(name) {
                    return ConditionResult::Known(value);
                }
                // Check for well-known constants
                if name == "true" {
                    return ConditionResult::Known(true);
                }
                if name == "false" {
                    return ConditionResult::Known(false);
                }
            }
        }
        // Multi-segment or unknown paths are runtime
        ConditionResult::Runtime
    }

    /// Evaluate a condition and determine if the context should be included
    ///
    /// # Arguments
    ///
    /// * `condition` - Optional condition expression
    /// * `config` - Configuration environment
    ///
    /// # Returns
    ///
    /// - `Ok(true)` - Include the context
    /// - `Ok(false)` - Exclude the context (condition evaluated to false)
    /// - `Err(_)` - Error evaluating the condition
    pub fn should_include_context(
        condition: &Option<Heap<Expr>>,
        config: &ConfigEnv,
    ) -> std::result::Result<bool, Text> {
        match condition {
            None => Ok(true), // No condition means always include
            Some(cond_expr) => match evaluate_condition(cond_expr, config) {
                ConditionResult::Known(value) => Ok(value),
                ConditionResult::Runtime => {
                    // Runtime conditions are included (checked at runtime)
                    Ok(true)
                }
                ConditionResult::Error(msg) => Err(msg),
            },
        }
    }
}

/// Convert an AST Path to a simple string name
///
/// For now, we only support simple identifiers in context requirements.
/// Future work: support module-qualified paths like `my_module.Database`
fn path_to_string(path: &verum_ast::ty::Path) -> Text {
    use verum_ast::ty::PathSegment;

    // For single-segment paths, return the segment name
    if path.segments.len() == 1 {
        match &path.segments[0] {
            PathSegment::Name(ident) => ident.name.clone(),
            PathSegment::SelfValue => "self".into(),
            PathSegment::Super => "super".into(),
            PathSegment::Cog => "cog".into(),
            PathSegment::Relative => ".".into(),
        }
    } else {
        // For multi-segment paths, join with '.'
        path.segments
            .iter()
            .map(|seg| match seg {
                PathSegment::Name(ident) => ident.name.as_str(),
                PathSegment::SelfValue => "self",
                PathSegment::Super => "super",
                PathSegment::Cog => "cog",
                PathSegment::Relative => ".",
            })
            .collect::<Vec<_>>()
            .join(".")
            .into()
    }
}

/// Convert an AST Type to a text representation for storage in context requirements.
///
/// This is used to preserve type arguments for generic contexts in using clauses.
/// The text representation can later be resolved to actual types during type checking.
///
/// # Examples
///
/// ```ignore
/// // Type::Named { path: "Text", args: [] } -> "Text"
/// // Type::Named { path: "List", args: [Int] } -> "List<Int>"
/// // Type::Named { path: "Result", args: [Ok, Err] } -> "Result<Ok, Err>"
/// ```
fn type_to_text(ty: &verum_ast::ty::Type) -> Text {
    use verum_ast::ty::{GenericArg, TypeKind};

    match &ty.kind {
        TypeKind::Path(path) => path_to_string(path),
        TypeKind::Generic { base, args } => {
            let base_text = type_to_text(base);
            if args.is_empty() {
                base_text
            } else {
                let args_str: Vec<Text> = args
                    .iter()
                    .map(|arg| match arg {
                        GenericArg::Type(t) => type_to_text(t),
                        GenericArg::Const(e) => format!("{:?}", e).into(),
                        GenericArg::Lifetime(_) => "'_".into(),
                        GenericArg::Binding(binding) => {
                            format!("{}={}", binding.name.name, type_to_text(&binding.ty)).into()
                        }
                    })
                    .collect();
                format!(
                    "{}<{}>",
                    base_text,
                    args_str
                        .iter()
                        .map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .into()
            }
        }
        TypeKind::Tuple(types) => {
            let types_str: Vec<Text> = types.iter().map(type_to_text).collect();
            format!(
                "({})",
                types_str
                    .iter()
                    .map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into()
        }
        TypeKind::Reference { mutable, inner } => {
            let mut_str = if *mutable { "mut " } else { "" };
            format!("&{}{}", mut_str, type_to_text(inner)).into()
        }
        TypeKind::CheckedReference { mutable, inner } => {
            let mut_str = if *mutable { "mut " } else { "" };
            format!("&checked {}{}", mut_str, type_to_text(inner)).into()
        }
        TypeKind::UnsafeReference { mutable, inner } => {
            let mut_str = if *mutable { "mut " } else { "" };
            format!("&unsafe {}{}", mut_str, type_to_text(inner)).into()
        }
        TypeKind::Array { element, size: _ } => format!("[{}]", type_to_text(element)).into(),
        TypeKind::Slice(inner) => format!("[{}]", type_to_text(inner)).into(),
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            let params_str: Vec<Text> = params.iter().map(type_to_text).collect();
            let params_joined = params_str
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("fn({}) -> {}", params_joined, type_to_text(return_type)).into()
        }
        TypeKind::Unit => "()".into(),
        TypeKind::Inferred => "_".into(),
        TypeKind::Bool => WKT::Bool.as_str().into(),
        TypeKind::Int => WKT::Int.as_str().into(),
        TypeKind::Float => WKT::Float.as_str().into(),
        TypeKind::Char => WKT::Char.as_str().into(),
        TypeKind::Text => WKT::Text.as_str().into(),
        // For other cases, fall back to debug format
        _ => format!("{:?}", ty.kind).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::decl::ContextGroupDecl;
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};

    fn make_span() -> Span {
        Span::default()
    }

    fn make_ident(name: &str) -> Ident {
        Ident::new(name.to_string(), make_span())
    }

    fn make_path(name: &str) -> Path {
        Path::new(
            vec![PathSegment::Name(Ident::new(name.to_string(), make_span()))].into(),
            make_span(),
        )
    }

    fn make_context_req(name: &str) -> AstContextRequirement {
        AstContextRequirement::simple(make_path(name), verum_common::List::new(), make_span())
    }

    #[test]
    fn test_register_context_type() {
        let mut resolver = ContextResolver::new();
        let db_type = Type::unit(); // Placeholder type for testing
        resolver.register_context_type("Database".into(), db_type);

        assert!(resolver.defined_contexts.contains(&"Database".into()));
        assert!(resolver.get_context_type(&"Database".into()).is_some());
    }

    #[test]
    fn test_register_protocol_as_context() {
        // Test that protocols can be registered as valid context types.
        // This enables patterns like:
        //  type Database is protocol { ... }
        //  fn handler() using [Database] { ... }
        let mut resolver = ContextResolver::new();

        // Register a protocol as a context (without a full Type - protocols
        // don't have a concrete type until runtime)
        resolver.register_protocol_as_context("Database".into());

        // The protocol should be recognized as a defined context
        assert!(resolver.is_context_defined(&"Database".into()));
        assert!(resolver.defined_contexts.contains(&"Database".into()));

        // Resolving a requirement with this protocol name should succeed
        let contexts = vec![make_context_req("Database")];
        let result = resolver.resolve_requirement(&contexts, make_span());
        assert!(result.is_ok());
    }

    #[test]
    fn test_register_multiple_protocols_as_contexts() {
        let mut resolver = ContextResolver::new();

        // Register multiple protocols at once
        let protocols = vec!["Database".into(), "Logger".into(), "Cache".into()];
        resolver.register_protocols_as_contexts(protocols);

        // All should be recognized as valid contexts
        assert!(resolver.is_context_defined(&"Database".into()));
        assert!(resolver.is_context_defined(&"Logger".into()));
        assert!(resolver.is_context_defined(&"Cache".into()));

        // Using them together in a context list should work
        let contexts = vec![make_context_req("Database"), make_context_req("Logger")];
        let result = resolver.resolve_requirement(&contexts, make_span());
        assert!(result.is_ok());
        let req = result.unwrap();
        assert_eq!(req.len(), 2);
        assert!(req.requires("Database"));
        assert!(req.requires("Logger"));
    }

    #[test]
    fn test_register_group_success() {
        let mut resolver = ContextResolver::new();

        // Register context types first
        resolver.register_context_type("Database".into(), Type::unit());
        resolver.register_context_type("Logger".into(), Type::unit());

        // Create a context group declaration
        let decl = ContextGroupDecl {
            visibility: verum_ast::decl::Visibility::Private,
            name: make_ident("WebContext"),
            contexts: vec![make_context_req("Database"), make_context_req("Logger")].into(),
            span: make_span(),
        };

        // Register the group
        assert!(resolver.register_group(&decl).is_ok());
        assert!(resolver.is_group("WebContext"));
    }

    #[test]
    fn test_register_empty_group_fails() {
        let mut resolver = ContextResolver::new();

        let decl = ContextGroupDecl {
            visibility: verum_ast::decl::Visibility::Private,
            name: make_ident("EmptyGroup"),
            contexts: verum_common::List::new(),
            span: make_span(),
        };

        // Should fail - groups cannot be empty
        assert!(resolver.register_group(&decl).is_err());
    }

    #[test]
    fn test_register_duplicate_group_fails() {
        let mut resolver = ContextResolver::new();

        resolver.register_context_type("Database".into(), Type::unit());

        let decl1 = ContextGroupDecl {
            visibility: verum_ast::decl::Visibility::Private,
            name: make_ident("WebContext"),
            contexts: vec![make_context_req("Database")].into(),
            span: make_span(),
        };

        let decl2 = decl1.clone();

        assert!(resolver.register_group(&decl1).is_ok());
        assert!(resolver.register_group(&decl2).is_err());
    }

    #[test]
    fn test_resolve_single_context() {
        let mut resolver = ContextResolver::new();
        resolver.register_context_type("Database".into(), Type::unit());

        let contexts = vec![make_context_req("Database")];

        let result = resolver.resolve_requirement(&contexts, make_span());
        assert!(result.is_ok());

        let req = result.unwrap();
        assert_eq!(req.len(), 1);
        assert!(req.requires("Database"));
    }

    #[test]
    fn test_resolve_group_expansion() {
        let mut resolver = ContextResolver::new();

        // Register contexts
        resolver.register_context_type("Database".into(), Type::unit());
        resolver.register_context_type("Logger".into(), Type::unit());

        // Register group
        let decl = ContextGroupDecl {
            visibility: verum_ast::decl::Visibility::Private,
            name: make_ident("WebContext"),
            contexts: vec![make_context_req("Database"), make_context_req("Logger")].into(),
            span: make_span(),
        };
        resolver.register_group(&decl).unwrap();

        // Resolve using the group name
        let contexts = vec![make_context_req("WebContext")];

        let result = resolver.resolve_requirement(&contexts, make_span());
        assert!(result.is_ok());

        let req = result.unwrap();
        assert_eq!(req.len(), 2);
        assert!(req.requires("Database"));
        assert!(req.requires("Logger"));
    }

    #[test]
    fn test_resolve_undefined_context_fails() {
        let resolver = ContextResolver::new();

        let contexts = vec![make_context_req("UndefinedContext")];

        let result = resolver.resolve_requirement(&contexts, make_span());
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_group_in_list_fails() {
        let mut resolver = ContextResolver::new();

        resolver.register_context_type("Database".into(), Type::unit());
        resolver.register_context_type("Logger".into(), Type::unit());

        // Register group
        let decl = ContextGroupDecl {
            visibility: verum_ast::decl::Visibility::Private,
            name: make_ident("WebContext"),
            contexts: vec![make_context_req("Database"), make_context_req("Logger")].into(),
            span: make_span(),
        };
        resolver.register_group(&decl).unwrap();

        // Try to use group in a list - should fail
        let contexts = vec![make_context_req("WebContext"), make_context_req("Database")];

        let result = resolver.resolve_requirement(&contexts, make_span());
        assert!(result.is_err());
    }

    #[test]
    fn test_path_to_string_simple() {
        let path = make_path("Database");
        assert_eq!(path_to_string(&path), "Database");
    }

    #[test]
    fn test_path_to_string_qualified() {
        let path = Path::new(
            vec![
                PathSegment::Name(Ident::new("module".to_string(), make_span())),
                PathSegment::Name(Ident::new("Database".to_string(), make_span())),
            ]
            .into(),
            make_span(),
        );
        assert_eq!(path_to_string(&path), "module.Database");
    }

    // ========================================================================
    // Compile-Time Condition Evaluation Tests (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // ========================================================================

    #[test]
    fn test_config_flag_set_and_get() {
        let mut resolver = ContextResolver::new();

        // Initially, flag is not set
        assert_eq!(resolver.is_config_flag_enabled("analytics_enabled"), None);

        // Set the flag to true
        resolver.set_config_flag("analytics_enabled", true);
        assert_eq!(
            resolver.is_config_flag_enabled("analytics_enabled"),
            Some(true)
        );

        // Set the flag to false
        resolver.set_config_flag("analytics_enabled", false);
        assert_eq!(
            resolver.is_config_flag_enabled("analytics_enabled"),
            Some(false)
        );
    }

    #[test]
    fn test_resolver_with_config() {
        use super::condition_eval::ConfigEnv;

        let mut config = ConfigEnv::new();
        config.set_flag("feature_enabled", true);
        config.set_flag("debug_mode", false);

        let resolver = ContextResolver::with_config(config);

        assert_eq!(
            resolver.is_config_flag_enabled("feature_enabled"),
            Some(true)
        );
        assert_eq!(resolver.is_config_flag_enabled("debug_mode"), Some(false));
    }

    #[test]
    fn test_config_env_boolean_evaluation() {
        use super::condition_eval::{ConditionResult, ConfigEnv, evaluate_condition};
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::literal::{Literal, LiteralKind};

        let config = ConfigEnv::new();

        // Test boolean literal: true
        let true_expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(true), make_span())),
            make_span(),
        );
        let result = evaluate_condition(&true_expr, &config);
        assert!(result.is_known_true());

        // Test boolean literal: false
        let false_expr = Expr::new(
            ExprKind::Literal(Literal::new(LiteralKind::Bool(false), make_span())),
            make_span(),
        );
        let result = evaluate_condition(&false_expr, &config);
        assert!(result.is_known_false());
    }

    #[test]
    fn test_condition_result_methods() {
        use super::condition_eval::ConditionResult;

        let known_true = ConditionResult::Known(true);
        assert!(known_true.is_known_true());
        assert!(!known_true.is_known_false());
        assert!(!known_true.is_runtime());

        let known_false = ConditionResult::Known(false);
        assert!(!known_false.is_known_true());
        assert!(known_false.is_known_false());
        assert!(!known_false.is_runtime());

        let runtime = ConditionResult::Runtime;
        assert!(!runtime.is_known_true());
        assert!(!runtime.is_known_false());
        assert!(runtime.is_runtime());
    }

    #[test]
    fn test_should_include_context_no_condition() {
        use super::condition_eval::{ConfigEnv, should_include_context};

        let config = ConfigEnv::new();

        // No condition means always include
        let result = should_include_context(&None, &config);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_config_env_flag_lookup() {
        use super::condition_eval::ConfigEnv;

        let mut config = ConfigEnv::new();

        // Test flag that doesn't exist
        assert_eq!(config.get_flag("nonexistent"), None);

        // Set and retrieve various flags
        config.set_flag("feature_a", true);
        config.set_flag("feature_b", false);
        config.set_flag("debug", true);

        assert_eq!(config.get_flag("feature_a"), Some(true));
        assert_eq!(config.get_flag("feature_b"), Some(false));
        assert_eq!(config.get_flag("debug"), Some(true));
    }

    #[test]
    fn test_config_env_value_storage() {
        use super::condition_eval::ConfigEnv;

        let mut config = ConfigEnv::new();

        // Test value that doesn't exist
        assert_eq!(config.get_value("nonexistent"), None);

        // Set and retrieve string values
        config.set_value("platform", "linux");
        config.set_value("version", "1.0.0");

        assert_eq!(config.get_value("platform"), Some(&"linux".into()));
        assert_eq!(config.get_value("version"), Some(&"1.0.0".into()));
    }

    // =========================================================================
    // Type Constraint Condition Tests (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // =========================================================================

    #[test]
    fn test_type_constraint_env_register_bounds() {
        use super::condition_eval::TypeConstraintEnv;

        let mut env = TypeConstraintEnv::new();

        // Register a type parameter with bounds
        let bounds: List<Text> = vec!["Validatable".into(), "Display".into()]
            .into_iter()
            .collect();
        env.register_type_param("T", bounds);

        // Check the bound is satisfied
        assert_eq!(env.check_type_bound("T", "Validatable"), Some(true));
        assert_eq!(env.check_type_bound("T", "Display"), Some(true));

        // Bound not in declared bounds - cannot determine
        assert_eq!(env.check_type_bound("T", "Clone"), None);

        // Unknown type parameter
        assert_eq!(env.check_type_bound("U", "Validatable"), None);
    }

    #[test]
    fn test_type_constraint_env_concrete_substitution() {
        use super::condition_eval::TypeConstraintEnv;

        let mut env = TypeConstraintEnv::new();

        // Register T with no bounds
        env.register_type_param("T", List::new());

        // Register concrete type "User" as implementing "Validatable"
        env.register_impl("User", "Validatable");
        env.register_impl("User", "Display");

        // Substitute T with User
        env.substitute_type("T", "User");

        // Now T: Validatable should resolve to true
        assert_eq!(env.check_type_bound("T", "Validatable"), Some(true));
        assert_eq!(env.check_type_bound("T", "Display"), Some(true));

        // User doesn't implement Clone
        assert_eq!(env.check_type_bound("T", "Clone"), Some(false));
    }

    #[test]
    fn test_type_constraint_env_has_type_param() {
        use super::condition_eval::TypeConstraintEnv;

        let mut env = TypeConstraintEnv::new();

        // Initially empty
        assert!(!env.has_type_param("T"));

        // Register type param
        env.register_type_param("T", List::new());
        assert!(env.has_type_param("T"));
        assert!(!env.has_type_param("U"));

        // Substitute creates a type param entry
        env.substitute_type("U", "String");
        assert!(env.has_type_param("U"));
    }

    #[test]
    fn test_evaluate_condition_with_type_constraints() {
        use super::condition_eval::{ConfigEnv, TypeConstraintEnv, evaluate_condition_with_types};
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::pattern::{Pattern, PatternKind};

        let config = ConfigEnv::new();
        let mut type_env = TypeConstraintEnv::new();

        // Register T: Validatable
        let bounds: List<Text> = vec!["Validatable".into()].into_iter().collect();
        type_env.register_type_param("T", bounds);

        // Create expression: T is Validatable (using Is expression)
        let t_path = make_path("T");
        let t_expr = Expr::new(ExprKind::Path(t_path), make_span());

        // Create an Ident pattern for Validatable
        let validatable_pattern = Pattern::new(
            PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: make_ident("Validatable"),
                subpattern: None,
            },
            make_span(),
        );

        let constraint_expr = Expr::new(
            ExprKind::Is {
                expr: verum_common::Heap::new(t_expr),
                pattern: validatable_pattern,
                negated: false,
            },
            make_span(),
        );

        // Evaluate: T is Validatable should be Known(true)
        let result = evaluate_condition_with_types(&constraint_expr, &config, &type_env);
        assert!(result.is_known_true());
    }

    #[test]
    fn test_evaluate_condition_type_constraint_false() {
        use super::condition_eval::{ConfigEnv, TypeConstraintEnv, evaluate_condition_with_types};
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::pattern::{Pattern, PatternKind};

        let config = ConfigEnv::new();
        let mut type_env = TypeConstraintEnv::new();

        // Register T without Validatable bound
        type_env.register_type_param("T", List::new());

        // Register User with some implementations, but NOT Validatable
        // This allows us to get a definitive false answer
        type_env.register_impl("User", "Display");
        type_env.register_impl("User", "Clone");

        // Substitute T with User
        type_env.substitute_type("T", "User");

        // Create expression: T is Validatable
        let t_path = make_path("T");
        let t_expr = Expr::new(ExprKind::Path(t_path), make_span());

        // Create an Ident pattern for Validatable
        let validatable_pattern = Pattern::new(
            PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: make_ident("Validatable"),
                subpattern: None,
            },
            make_span(),
        );

        let constraint_expr = Expr::new(
            ExprKind::Is {
                expr: verum_common::Heap::new(t_expr),
                pattern: validatable_pattern,
                negated: false,
            },
            make_span(),
        );

        // Evaluate: T is Validatable should be Known(false) because User implements
        // Display and Clone, but NOT Validatable
        let result = evaluate_condition_with_types(&constraint_expr, &config, &type_env);
        assert!(result.is_known_false());
    }

    // =========================================================================
    // Transform Argument Type Tests (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // =========================================================================












}
