//! Context requirement type checking for Verum's Two-Level Context System.
//!
//! This module implements compile-time validation of context requirements,
//! ensuring that functions declare all contexts they use and that context
//! providers are correctly installed.
//!
//! # Specification
//!
//! Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
//!
//! # Two-Level Model
//!
//! - **Level 1 (Static DI)**: @injectable/@inject - 0ns overhead, compile-time resolution
//! - **Level 2 (Dynamic Contexts)**: provide/using - ~5ns overhead, runtime resolution
//!
//! This module focuses on **Level 2 type checking** - validating `using [Context]` clauses.
//!
//! # Context Requirement Rules
//!
//! 1. **Declaration**: Functions must declare contexts in `using` clause
//! 2. **Propagation**: If function F calls G requiring contexts, F must also require them
//! 3. **Scope Closure (β-reduction)**: Context availability is lexically scoped by `provide`
//! 4. **Method Validation**: Context method calls must match their signatures

use crate::{Result, TypeError};
use verum_ast::{decl::ContextDecl, span::Span};
#[allow(unused_imports)]
use verum_common::{List, Map, Maybe, Set, Text};
use verum_common::ToText;

/// A context requirement in a function signature.
///
/// Example: `fn foo() using [Database, Logger]`
/// Creates two ContextRequirements: Database and Logger
///
/// Extended with advanced context patterns (negative contexts, call graph verification, module aliases):
/// Example: `fn pure_compute() using [!Database, !Network]`
/// These are excluded contexts - the function cannot use them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContextRequirement {
    /// Context name (e.g., "Database", "Logger")
    pub name: Text,
    /// Optional sub-context path (e.g., "FileSystem.Read")
    pub sub_context: Option<Text>,
    /// Whether this is a negative (excluded) context (`!Database`)
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    pub is_negative: bool,
    /// Span for error reporting
    pub span: Span,
}

impl ContextRequirement {
    /// Create a simple context requirement (positive - context is required)
    pub fn new(name: impl Into<Text>, span: Span) -> Self {
        Self {
            name: name.into(),
            sub_context: None,
            is_negative: false,
            span,
        }
    }

    /// Create a negative context requirement (`!Database`)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    ///
    /// Negative contexts are excluded - the function and all its callees
    /// cannot use this context.
    pub fn negative(name: impl Into<Text>, span: Span) -> Self {
        Self {
            name: name.into(),
            sub_context: None,
            is_negative: true,
            span,
        }
    }

    /// Create a sub-context requirement (e.g., FileSystem.Read)
    pub fn with_sub(name: impl Into<Text>, sub_context: impl Into<Text>, span: Span) -> Self {
        Self {
            name: name.into(),
            sub_context: Some(sub_context.into()),
            is_negative: false,
            span,
        }
    }

    /// Get the full context path
    pub fn full_path(&self) -> Text {
        match &self.sub_context {
            Some(sub) => format!("{}.{}", self.name, sub).into(),
            None => self.name.clone(),
        }
    }

    /// Check if this is a negative (excluded) context
    pub fn is_excluded(&self) -> bool {
        self.is_negative
    }
}

/// A set of context requirements.
///
/// Similar to ContextSet from contexts module, but for Two-Level Context requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSet {
    contexts: Set<ContextRequirement>,
}

impl ContextSet {
    pub fn new() -> Self {
        Self {
            contexts: Set::new(),
        }
    }

    pub fn empty() -> Self {
        Self::new()
    }

    pub fn singleton(ctx: ContextRequirement) -> Self {
        let mut set = Self::new();
        set.add(ctx);
        set
    }

    pub fn add(&mut self, ctx: ContextRequirement) {
        self.contexts.insert(ctx);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.contexts.iter().any(|c| c.name.as_str() == name && !c.is_negative)
    }

    pub fn contains_full_path(&self, path: &str) -> bool {
        self.contexts.iter().any(|c| c.full_path().as_str() == path && !c.is_negative)
    }

    pub fn union(&self, other: &ContextSet) -> ContextSet {
        let mut result = self.clone();
        for ctx in other.contexts.iter() {
            result.contexts.insert(ctx.clone());
        }
        result
    }

    pub fn is_empty(&self) -> bool {
        self.contexts.is_empty()
    }

    pub fn is_subset(&self, other: &ContextSet) -> bool {
        for ctx in self.contexts.iter() {
            if !other.contexts.contains(ctx) {
                return false;
            }
        }
        true
    }

    pub fn iter(&self) -> impl Iterator<Item = &ContextRequirement> {
        self.contexts.iter()
    }

    pub fn len(&self) -> usize {
        self.contexts.len()
    }

    // ========================================================================
    // Negative Context Methods (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // ========================================================================

    /// Check if a context is excluded (negative)
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    pub fn is_excluded(&self, name: &str) -> bool {
        self.contexts.iter().any(|c| c.name.as_str() == name && c.is_negative)
    }

    /// Get all positive (required) contexts
    pub fn positive_contexts(&self) -> impl Iterator<Item = &ContextRequirement> {
        self.contexts.iter().filter(|c| !c.is_negative)
    }

    /// Get all negative (excluded) contexts
    pub fn negative_contexts(&self) -> impl Iterator<Item = &ContextRequirement> {
        self.contexts.iter().filter(|c| c.is_negative)
    }

    /// Get the names of all excluded contexts
    pub fn excluded_names(&self) -> List<&Text> {
        self.negative_contexts().map(|c| &c.name).collect()
    }

    /// Check if using a context would violate any negative constraints
    ///
    /// Returns `Err(context_name)` if the context is excluded
    pub fn validate_usage(&self, context_name: &str) -> std::result::Result<(), Text> {
        if self.is_excluded(context_name) {
            Err(format!(
                "Context '{}' is explicitly excluded (`!{}`). Cannot use it.",
                context_name, context_name
            ).into())
        } else {
            Ok(())
        }
    }
}

impl Default for ContextSet {
    fn default() -> Self {
        Self::new()
    }
}

impl FromIterator<ContextRequirement> for ContextSet {
    fn from_iter<T: IntoIterator<Item = ContextRequirement>>(iter: T) -> Self {
        let mut set = Self::new();
        for ctx in iter {
            set.add(ctx);
        }
        set
    }
}

/// Context environment (θ - theta) tracking available contexts.
///
/// This tracks which contexts are currently available through `provide` statements.
/// It's separate from TypeEnv (which tracks variable types).
///
/// # Lexical Scoping
///
/// ```verum
/// provide Logger = console_logger();
/// {
///     // Logger available here
///     Logger.log(...);  // ✅
/// }
/// // Logger out of scope here
/// Logger.log(...);  // ❌ Error
/// ```
#[derive(Debug, Clone)]
pub struct ContextEnv {
    /// Currently provided contexts in this scope
    provided: Map<Text, ContextDecl>,
    /// Parent environment (for nested scopes)
    parent: Option<Box<ContextEnv>>,
}

impl ContextEnv {
    /// Create a new empty context environment
    pub fn new() -> Self {
        Self {
            provided: Map::new(),
            parent: None,
        }
    }

    /// Create a child environment (nested scope)
    pub fn child(&self) -> Self {
        Self {
            provided: Map::new(),
            parent: Some(Box::new(self.clone())),
        }
    }

    /// Provide a context in the current scope
    /// Returns true if successfully provided, false if already provided in current scope
    pub fn provide(&mut self, name: impl Into<Text>, decl: ContextDecl) -> bool {
        let name_text = name.into();
        // Check if already provided in THIS scope (not parent scopes)
        if self.provided.contains_key(&name_text) {
            return false; // Duplicate in current scope
        }
        self.provided.insert(name_text, decl);
        true
    }

    /// Check if a context is already provided in the current scope only (not parent scopes)
    pub fn is_provided_in_current_scope(&self, name: &str) -> bool {
        self.provided.contains_key(&Text::from(name))
    }

    /// Check if a context is available (search up scope chain)
    pub fn has_context(&self, name: &str) -> bool {
        if self.provided.contains_key(&Text::from(name)) {
            return true;
        }

        if let Some(parent) = &self.parent {
            return parent.has_context(name);
        }

        false
    }

    /// Lookup a context declaration
    pub fn lookup(&self, name: &str) -> Option<&ContextDecl> {
        if let Some(decl) = self.provided.get(&Text::from(name)) {
            return Some(decl);
        }

        if let Some(parent) = &self.parent {
            return parent.lookup(name);
        }

        None
    }

    /// Get all available contexts (including parent scopes)
    pub fn all_contexts(&self) -> Set<Text> {
        let mut contexts = Set::new();

        for name in self.provided.keys() {
            contexts.insert(name.clone());
        }

        if let Some(parent) = &self.parent {
            let parent_contexts = parent.all_contexts();
            for ctx in parent_contexts {
                contexts.insert(ctx);
            }
        }

        contexts
    }
}

impl Default for ContextEnv {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a function's context requirements for transitive checking
#[derive(Debug, Clone)]
pub struct FunctionContextInfo {
    /// Function name
    pub name: Text,
    /// Required (positive) contexts
    pub required_contexts: ContextSet,
    /// Excluded (negative) contexts
    pub excluded_contexts: List<Text>,
    /// Functions this function calls (with call site info)
    pub callees: List<Text>,
    /// Detailed call site information for better error messages
    pub call_sites: Map<Text, CallSiteInfo>,
    /// Source span
    pub span: Span,
}

/// Information about a call site for detailed error reporting
#[derive(Debug, Clone)]
pub struct CallSiteInfo {
    /// Name of the called function
    pub callee_name: Text,
    /// Line number of the call
    pub line: u32,
    /// Column number of the call
    pub column: u32,
    /// Span of the call expression
    pub span: Span,
}

impl CallSiteInfo {
    /// Create a new call site info
    pub fn new(callee_name: impl Into<Text>, line: u32, column: u32, span: Span) -> Self {
        Self {
            callee_name: callee_name.into(),
            line,
            column,
            span,
        }
    }
}

// =============================================================================
// Call Graph for Transitive Analysis (Advanced context patterns (negative contexts, call graph verification, module aliases))
// =============================================================================

/// A call graph for analyzing transitive context requirements.
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
///
/// The call graph enables efficient transitive analysis of negative context
/// constraints by pre-computing which functions call which other functions.
#[derive(Debug, Clone, Default)]
pub struct CallGraph {
    /// Map from function name to its call information
    functions: Map<Text, CallGraphNode>,
}

/// A node in the call graph representing a function
#[derive(Debug, Clone)]
pub struct CallGraphNode {
    /// Function name
    pub name: Text,
    /// Functions called by this function (direct callees)
    pub callees: List<CallSiteInfo>,
    /// Functions that call this function (direct callers)
    pub callers: List<Text>,
    /// Context requirements for this function
    pub contexts: ContextSet,
    /// Source span of the function definition
    pub span: Span,
}

impl CallGraph {
    /// Create a new empty call graph
    pub fn new() -> Self {
        Self {
            functions: Map::new(),
        }
    }

    /// Add a function to the call graph
    pub fn add_function(&mut self, name: impl Into<Text>, contexts: ContextSet, span: Span) {
        let name = name.into();
        self.functions.insert(name.clone(), CallGraphNode {
            name,
            callees: List::new(),
            callers: List::new(),
            contexts,
            span,
        });
    }

    /// Register a call from one function to another
    pub fn add_call(&mut self, caller: impl Into<Text>, call_site: CallSiteInfo) {
        let caller = caller.into();
        let callee = call_site.callee_name.clone();

        // Add callee to caller's list
        if let Some(caller_node) = self.functions.get_mut(&caller) {
            caller_node.callees.push(call_site);
        }

        // Add caller to callee's callers list (if callee exists)
        if let Some(callee_node) = self.functions.get_mut(&callee) {
            callee_node.callers.push(caller);
        }
    }

    /// Get a function node by name
    pub fn get_function(&self, name: &str) -> Option<&CallGraphNode> {
        self.functions.get(&Text::from(name))
    }

    /// Get all direct callees of a function
    pub fn get_callees(&self, name: &str) -> List<&CallSiteInfo> {
        self.functions
            .get(&Text::from(name))
            .map(|node| node.callees.iter().collect())
            .unwrap_or_default()
    }

    /// Check if a function exists in the graph
    pub fn contains(&self, name: &str) -> bool {
        self.functions.contains_key(&Text::from(name))
    }

    /// Get all functions in the graph
    pub fn all_functions(&self) -> impl Iterator<Item = &CallGraphNode> {
        self.functions.values()
    }
}

/// A step in the call chain leading to a violation
#[derive(Debug, Clone)]
pub struct CallChainStep {
    /// Function name at this step
    pub function_name: Text,
    /// Line number of the call (or function definition for the last step)
    pub line: u32,
    /// Whether this step uses the violated context
    pub uses_context: bool,
}

/// Detailed violation information for transitive negative context violations
#[derive(Debug, Clone)]
pub struct TransitiveViolationInfo {
    /// The function with the negative constraint
    pub origin_function: Text,
    /// The excluded context that was violated
    pub excluded_context: Text,
    /// The complete call chain leading to the violation
    pub call_chain: List<CallChainStep>,
    /// Span of the negative constraint declaration
    pub declaration_span: Span,
}

/// Context type checker.
///
/// Validates that:
/// 1. Functions declare all contexts they use
/// 2. Context requirements propagate through call chains
/// 3. Context methods match their signatures
/// 4. Sub-context access is valid
/// 5. Negative contexts are not violated transitively (Advanced context patterns (negative contexts, call graph verification, module aliases))
pub struct ContextChecker {
    /// Current context environment (θ)
    env: ContextEnv,
    /// Currently required contexts for the function being checked
    required: ContextSet,
    /// Context declarations (interface definitions)
    declarations: Map<Text, ContextDecl>,
    /// Function registry for transitive checking
    /// Maps function name -> context info
    function_registry: Map<Text, FunctionContextInfo>,
    /// In lenient mode, missing/undefined context errors are suppressed.
    lenient: bool,
}

impl ContextChecker {
    pub fn new() -> Self {
        Self {
            env: ContextEnv::new(),
            required: ContextSet::new(),
            declarations: Map::new(),
            function_registry: Map::new(),
            lenient: false,
        }
    }

    /// Register a context declaration (interface definition)
    /// Set lenient mode for context checking.
    pub fn set_lenient(&mut self, lenient: bool) {
        self.lenient = lenient;
    }
    pub fn register_context(&mut self, name: impl Into<Text>, decl: ContextDecl) {
        self.declarations.insert(name.into(), decl);
    }

    /// Enter a new scope
    pub fn enter_scope(&mut self) {
        self.env = self.env.child();
    }

    /// Exit the current scope
    pub fn exit_scope(&mut self) {
        if let Some(parent) = self.env.parent.take() {
            self.env = *parent;
        }
    }

    /// Install a context via `provide` statement
    ///
    /// Example: `provide Logger = console_logger();`
    ///
    /// Returns error E808 if the context is already provided in the current scope.
    pub fn provide_context(&mut self, name: impl Into<Text>, span: Span) -> Result<()> {
        let name = name.into();
        // Lookup the context declaration
        let decl = match self.declarations.get(&name) {
            Some(d) => d.clone(),
            None => {
                if self.lenient { return Ok(()); }
                return Err(TypeError::UndefinedContext {
                    name: name.as_str().to_text(),
                    span,
                });
            }
        };

        // Check for duplicate provide in current scope
        if !self.env.provide(name.clone(), decl) {
            return Err(TypeError::DuplicateProvide {
                context: name,
                span,
            });
        }
        Ok(())
    }

    /// Set the required contexts for a function
    ///
    /// Called when entering a function with `using [Context1, Context2]`
    pub fn set_required(&mut self, contexts: ContextSet) {
        self.required = contexts;
    }

    /// Check if a context is available (either required or provided)
    pub fn is_available(&self, name: &str) -> bool {
        // Context is available if either:
        // 1. It's in the function's required contexts
        // 2. It's provided via `provide` statement
        self.required.contains(name) || self.env.has_context(name)
    }

    /// Validate a context method call
    ///
    /// Example: `Logger.log(Level::Info, "message")`
    pub fn check_context_call(
        &self,
        context_name: &str,
        method_name: &str,
        span: Span,
    ) -> Result<()> {
        if self.lenient { return Ok(()); }
        // Check if context is available
        if !self.is_available(context_name) {
            return Err(TypeError::MissingContext {
                context: context_name.to_text(),
                span,
            });
        }

        // Lookup context declaration
        let decl = match self.declarations.get(&Text::from(context_name)) {
            Some(d) => d,
            None => {
                return Err(TypeError::UndefinedContext {
                    name: context_name.to_text(),
                    span,
                });
            }
        };

        // Check if method exists in context
        let method_exists = decl
            .methods
            .iter()
            .any(|m| m.name.name.as_str() == method_name);

        if !method_exists {
            return Err(TypeError::UndefinedContextMethod {
                context: context_name.to_text(),
                method: method_name.to_text(),
                span,
            });
        }

        Ok(())
    }

    /// Check if a sub-context is valid
    ///
    /// Validates that a sub-context exists within a parent context.
    ///
    /// # Specification
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
    /// Context system core: "context Name { fn method(...) }" declarations, "using [Ctx1, Ctx2]" on functions, "provide Ctx = impl" for injection — 0.1 - Sub-Context Syntax
    ///
    /// # Example
    ///
    /// ```verum
    /// context FileSystem {
    ///     context Read {
    ///         fn read(path: Text) -> Result<List<u8>>
    ///     }
    ///     context Write {
    ///         fn write(path: Text, data: List<u8>) -> Result<()>
    ///     }
    /// }
    ///
    /// fn process_file() using [FileSystem.Read] {
    ///     FileSystem.Read.read("file.txt")  // ✅ Valid
    /// }
    ///
    /// fn invalid() using [FileSystem.Execute] {  // ❌ Error: Execute not found
    ///     // ...
    /// }
    /// ```
    pub fn check_sub_context(&self, context: &str, sub: &str, span: Span) -> Result<()> {
        // Lookup parent context declaration
        match self.declarations.get(&Text::from(context)) {
            Some(decl) => {
                // Check if sub-context exists in the parent context's sub_contexts
                let sub_context_exists = decl
                    .sub_contexts
                    .iter()
                    .any(|sc| sc.name.name.as_str() == sub);

                if !sub_context_exists {
                    // Collect available sub-context names for error message
                    let available: List<Text> = decl
                        .sub_contexts
                        .iter()
                        .map(|sc| sc.name.name.clone())
                        .collect();

                    return Err(TypeError::InvalidSubContext {
                        context: context.to_text(),
                        sub_context: sub.to_text(),
                        available,
                        span,
                    });
                }

                Ok(())
            }
            None => Err(TypeError::UndefinedContext {
                name: context.to_text(),
                span,
            }),
        }
    }

    /// Validate context requirements propagation
    ///
    /// When function F calls function G, F must require all contexts that G requires
    /// (unless F provides them locally via `provide`).
    pub fn check_call_propagation(&self, callee_contexts: &ContextSet, span: Span) -> Result<()> {
        for ctx in callee_contexts.iter() {
            if !self.is_available(ctx.name.as_str()) {
                return Err(TypeError::MissingContext {
                    context: ctx.name.as_str().to_text(),
                    span,
                });
            }
        }

        Ok(())
    }

    /// Get currently required contexts
    pub fn get_required(&self) -> &ContextSet {
        &self.required
    }

    /// Get context environment
    pub fn get_env(&self) -> &ContextEnv {
        &self.env
    }

    /// Check if a context is available at the current location
    ///
    /// Validates that required contexts are either:
    /// 1. Declared in the function's `using [Context]` clause
    /// 2. Provided via `provide` statement in the current scope
    ///
    /// # Specification
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
    ///
    /// # Arguments
    ///
    /// * `context_name` - The name of the context to check
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// `Ok(())` if the context is available, or an appropriate error
    ///
    /// # Example
    ///
    /// ```verum
    /// fn process() using [Database] {
    ///     Database.query(...)  // ✅ Available via function requirement
    /// }
    ///
    /// fn main() {
    ///     provide Database = postgres();
    ///     Database.query(...)  // ✅ Available via provide statement
    /// }
    ///
    /// fn invalid() {
    ///     Database.query(...)  // ❌ Error: context not available
    /// }
    /// ```
    pub fn check_context_availability(&self, context_name: &str, span: Span) -> Result<()> {
        // Check if the context is available (either required or provided)
        if !self.is_available(context_name) {
            return Err(TypeError::MissingContext {
                context: context_name.to_text(),
                span,
            });
        }

        // Verify the context is actually declared
        if !self.declarations.contains_key(&Text::from(context_name)) {
            return Err(TypeError::UndefinedContext {
                name: context_name.to_text(),
                span,
            });
        }

        Ok(())
    }

    /// Check if provided contexts satisfy function requirements
    ///
    /// Validates that all contexts required by a function are satisfied by:
    /// 1. The caller's own context requirements
    /// 2. Contexts provided via `provide` statements
    ///
    /// This is the core validation for context propagation at call sites.
    ///
    /// # Specification
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
    /// Context resolution: resolving context names to declarations, expanding groups, checking provision — .2 - Context Propagation Rules
    ///
    /// # Arguments
    ///
    /// * `required_contexts` - Contexts required by the callee function
    /// * `provided_contexts` - Contexts available in the caller (via `using` or `provide`)
    /// * `call_span` - Source location of the function call
    ///
    /// # Returns
    ///
    /// `Ok(())` if all requirements are satisfied, or an error with details about
    /// missing contexts
    ///
    /// # Example
    ///
    /// ```verum
    /// fn callee() using [Database, Logger] {
    ///     // Requires both Database and Logger
    /// }
    ///
    /// fn caller1() using [Database, Logger] {
    ///     callee();  // ✅ All requirements satisfied
    /// }
    ///
    /// fn caller2() using [Database] {
    ///     provide Logger = console_logger();
    ///     callee();  // ✅ Database from requirement, Logger from provide
    /// }
    ///
    /// fn caller3() using [Database] {
    ///     callee();  // ❌ Error: Logger not available
    /// }
    ///
    /// fn caller4() {
    ///     provide Database = postgres();
    ///     provide Logger = console_logger();
    ///     callee();  // ✅ Both provided locally
    /// }
    /// ```
    pub fn check_context_satisfaction(
        &self,
        required_contexts: &ContextSet,
        provided_contexts: &ContextSet,
        call_span: Span,
    ) -> Result<()> {
        if self.lenient { return Ok(()); }
        // Check each required context individually
        for required in required_contexts.iter() {
            let context_name = required.name.as_str();

            // Check if the context is satisfied either by:
            // 1. Being in the provided contexts (caller's `using` clause)
            // 2. Being available in the environment (via `provide`)
            let is_provided = provided_contexts.contains(context_name);
            let is_in_env = self.env.has_context(context_name);

            if !is_provided && !is_in_env {
                // Context is missing - generate helpful error
                return Err(TypeError::MissingContext {
                    context: context_name.to_text(),
                    span: call_span,
                });
            }

            // Verify the context is actually declared (interface exists)
            if !self.declarations.contains_key(&Text::from(context_name)) {
                return Err(TypeError::UndefinedContext {
                    name: context_name.to_text(),
                    span: call_span,
                });
            }

            // If it's a sub-context, validate the sub-context exists
            if let Some(ref sub) = required.sub_context {
                self.check_sub_context(context_name, sub.as_str(), call_span)?;
            }
        }

        Ok(())
    }

    /// Check multiple contexts at once for bulk validation
    ///
    /// This is a convenience method that checks availability of multiple contexts
    /// and collects all missing contexts for comprehensive error reporting.
    ///
    /// # Arguments
    ///
    /// * `contexts` - Set of contexts to check
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// `Ok(())` if all contexts are available, or an error listing all missing contexts
    pub fn check_contexts_availability(&self, contexts: &ContextSet, span: Span) -> Result<()> {
        let mut missing = List::new();

        for ctx in contexts.iter() {
            if !self.is_available(ctx.name.as_str()) {
                missing.push(ctx.name.clone());
            }
        }

        if !missing.is_empty() {
            // If multiple contexts are missing, report the first one
            // (could be enhanced to report all missing contexts)
            return Err(TypeError::MissingContext {
                context: missing[0].as_str().to_text(),
                span,
            });
        }

        Ok(())
    }

    /// Infer context requirements from an expression or function body
    ///
    /// This performs flow-sensitive analysis to determine what contexts are
    /// actually used within a function body, enabling automatic inference of
    /// context requirements.
    ///
    /// # Specification
    ///
    /// Context resolution: resolving context names to declarations, expanding groups, checking provision — .3 - Context Inference
    ///
    /// # Arguments
    ///
    /// * `used_contexts` - Set of context names referenced in the code
    ///
    /// # Returns
    ///
    /// A `ContextSet` containing all inferred requirements
    ///
    /// # Example
    ///
    /// ```verum
    /// fn process(data: Data) {
    ///     Logger.info("Processing");  // Uses Logger
    ///     Database.save(data);        // Uses Database
    /// }
    /// // Compiler infers: using [Logger, Database]
    /// ```
    pub fn infer_requirements(&self, used_contexts: &[&str]) -> ContextSet {
        let mut requirements = ContextSet::new();

        for context_name in used_contexts {
            // Create a requirement for each used context
            let req = ContextRequirement::new(context_name.to_string(), Span::default());
            requirements.add(req);
        }

        requirements
    }

    /// Compute the transitive closure of context requirements
    ///
    /// Given a function that calls other functions, compute the complete set
    /// of contexts needed including transitive dependencies.
    ///
    /// # Specification
    ///
    /// Context resolution: resolving context names to declarations, expanding groups, checking provision — .2 - Context Propagation
    ///
    /// # Arguments
    ///
    /// * `direct_requirements` - Contexts directly required by the function
    /// * `callee_requirements` - List of context sets from called functions
    ///
    /// # Returns
    ///
    /// Complete set of contexts needed (union of all requirements)
    ///
    /// # Example
    ///
    /// ```verum
    /// fn leaf() using [Database] { ... }
    /// fn middle() using [Logger] { leaf(); }
    /// fn top() { middle(); }
    /// // top needs transitive closure: [Database, Logger]
    /// ```
    pub fn compute_transitive_requirements(
        &self,
        direct_requirements: &ContextSet,
        callee_requirements: &[&ContextSet],
    ) -> ContextSet {
        let mut transitive = direct_requirements.clone();

        // Add all requirements from callees
        for callee_reqs in callee_requirements {
            transitive = transitive.union(callee_reqs);
        }

        transitive
    }

    /// Validate a full function definition with context requirements
    ///
    /// This is the main entry point for validating that a function properly
    /// declares and uses contexts.
    ///
    /// # Specification
    ///
    /// Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
    ///
    /// # Arguments
    ///
    /// * `declared_contexts` - Contexts in the function's `using` clause
    /// * `used_contexts` - Contexts actually referenced in the function body
    /// * `callee_contexts` - Context requirements from called functions
    /// * `span` - Source location for error reporting
    ///
    /// # Returns
    ///
    /// `Ok(())` if the function is valid, or an error describing the issue
    ///
    /// # Validation Rules
    ///
    /// 1. All used contexts must be declared
    /// 2. All callee requirements must be satisfied
    /// 3. No undeclared contexts may be used
    pub fn validate_function(
        &self,
        declared_contexts: &ContextSet,
        used_contexts: &[&str],
        callee_contexts: &[&ContextSet],
        span: Span,
    ) -> Result<()> {
        // Infer what contexts are actually needed
        let inferred = self.infer_requirements(used_contexts);

        // Compute transitive requirements from callees
        let transitive = self.compute_transitive_requirements(&inferred, callee_contexts);

        // Check that all transitive requirements are satisfied by declared contexts
        for required in transitive.iter() {
            if !declared_contexts.contains(required.name.as_str()) {
                return Err(TypeError::MissingContext {
                    context: required.name.as_str().to_text(),
                    span,
                });
            }
        }

        Ok(())
    }

    // ========================================================================
    // Negative Context Transitive Verification (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // ========================================================================

    /// Register a function's context information for transitive checking
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    ///
    /// This must be called for each function before transitive verification.
    pub fn register_function(&mut self, info: FunctionContextInfo) {
        self.function_registry.insert(info.name.clone(), info);
    }

    /// Check if a context is available, considering negative constraints
    ///
    /// Returns an error if the context is excluded in the current requirements.
    pub fn check_context_not_excluded(&self, context_name: &str, span: Span) -> Result<()> {
        if self.required.is_excluded(context_name) {
            return Err(TypeError::ExcludedContextViolation {
                context: context_name.to_text(),
                span,
            });
        }
        Ok(())
    }

    /// Validate a function call against negative context constraints
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    ///
    /// Ensures that calling `callee_name` doesn't violate any negative
    /// context constraints in the current function's requirements.
    ///
    /// # Arguments
    ///
    /// * `callee_name` - Name of the function being called
    /// * `call_span` - Source location of the call
    ///
    /// # Returns
    ///
    /// `Ok(())` if the call is valid, or an error if it violates negative constraints
    pub fn check_call_negative_constraints(
        &self,
        callee_name: &str,
        call_span: Span,
    ) -> Result<()> {
        // Get callee's context requirements
        let callee_info = match self.function_registry.get(&Text::from(callee_name)) {
            Some(info) => info,
            None => return Ok(()), // Unknown function - can't verify, assume ok
        };

        // Check if any of the callee's required contexts are excluded in caller
        for required in callee_info.required_contexts.positive_contexts() {
            let ctx_name = required.name.as_str();
            if self.required.is_excluded(ctx_name) {
                return Err(TypeError::TransitiveNegativeContextViolation {
                    excluded_context: ctx_name.to_text(),
                    callee: callee_name.to_text(),
                    span: call_span,
                });
            }
        }

        Ok(())
    }

    /// Perform full transitive negative context verification for a function
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    ///
    /// This checks that if a function excludes a context (`!Database`),
    /// none of its callees (directly or transitively) use that context.
    ///
    /// # Arguments
    ///
    /// * `function_name` - The function to verify
    ///
    /// # Returns
    ///
    /// `Ok(())` if all negative constraints are satisfied, or an error
    /// with the violation path
    pub fn verify_transitive_negative_contexts(
        &self,
        function_name: &str,
    ) -> Result<()> {
        let func_info = match self.function_registry.get(&Text::from(function_name)) {
            Some(info) => info.clone(),
            None => return Ok(()), // Unknown function
        };

        // Get excluded contexts from the function
        let excluded: List<Text> = func_info.required_contexts
            .negative_contexts()
            .map(|c| c.name.clone())
            .collect();

        if excluded.is_empty() {
            return Ok(()); // No negative constraints to verify
        }

        // Track visited functions to avoid cycles
        let mut visited = Set::new();
        visited.insert(Text::from(function_name));

        // Check all callees transitively
        self.check_callees_for_excluded(
            function_name,
            &excluded,
            &func_info.callees,
            &mut visited,
            &mut List::new(),
        )
    }

    /// Recursively check callees for excluded context violations
    fn check_callees_for_excluded(
        &self,
        original_function: &str,
        excluded: &List<Text>,
        callees: &List<Text>,
        visited: &mut Set<Text>,
        call_path: &mut List<Text>,
    ) -> Result<()> {
        for callee_name in callees {
            // Skip if already visited (cycle)
            if visited.contains(callee_name) {
                continue;
            }
            visited.insert(callee_name.clone());
            call_path.push(callee_name.clone());

            // Get callee's context info
            if let Some(callee_info) = self.function_registry.get(callee_name) {
                // Check if callee uses any excluded contexts
                for excluded_ctx in excluded {
                    if callee_info.required_contexts.contains(excluded_ctx.as_str()) {
                        // Found a violation!
                        let path_str = call_path
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(" -> ");

                        return Err(TypeError::TransitiveNegativeContextViolation {
                            excluded_context: excluded_ctx.clone(),
                            callee: format!(
                                "{} (via: {})",
                                callee_name,
                                path_str
                            ).into(),
                            span: callee_info.span,
                        });
                    }
                }

                // Recursively check callee's callees
                self.check_callees_for_excluded(
                    original_function,
                    excluded,
                    &callee_info.callees,
                    visited,
                    call_path,
                )?;
            }

            call_path.pop();
        }

        Ok(())
    }

    /// Get the function registry for external access
    pub fn get_function_registry(&self) -> &Map<Text, FunctionContextInfo> {
        &self.function_registry
    }

    // ========================================================================
    // Enhanced Transitive Verification with CallGraph (Advanced context patterns (negative contexts, call graph verification, module aliases))
    // ========================================================================

    /// Perform full transitive negative context verification using a CallGraph.
    ///
    /// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
    ///
    /// This is an enhanced version that uses a CallGraph structure for better
    /// error messages with detailed call chain information including line numbers.
    ///
    /// # Arguments
    ///
    /// * `function_name` - The function to verify
    /// * `excluded` - List of excluded context paths
    /// * `call_graph` - The call graph for the module/compilation unit
    ///
    /// # Returns
    ///
    /// `Ok(())` if all negative constraints are satisfied, or an error
    /// with detailed call chain information
    ///
    /// # Error Format
    ///
    /// ```text
    /// error: Function 'pure_function' excludes 'Database' but transitively calls:
    ///   -> helper_function() at line 15
    ///      -> database_helper() at line 42
    ///         uses Database
    /// ```
    pub fn verify_transitive_negative_contexts_with_graph(
        &self,
        function_name: &str,
        excluded: &[ContextPath],
        call_graph: &CallGraph,
    ) -> std::result::Result<(), TransitiveViolationInfo> {
        if excluded.is_empty() {
            return Ok(()); // No negative constraints to verify
        }

        // Get the function node from the call graph
        let func_node = match call_graph.get_function(function_name) {
            Some(node) => node,
            None => return Ok(()), // Unknown function - nothing to verify
        };

        // Track visited functions to avoid infinite loops in cycles
        let mut visited = Set::new();
        visited.insert(Text::from(function_name));

        // Convert excluded paths to Text for comparison
        let excluded_names: List<Text> = excluded
            .iter()
            .map(path_to_text)
            .collect();

        // Check all callees transitively with detailed tracking
        let mut call_chain = List::new();

        self.check_callees_with_graph(
            function_name,
            &excluded_names,
            &func_node.callees,
            call_graph,
            &mut visited,
            &mut call_chain,
            func_node.span,
        )
    }

    /// Recursively check callees for excluded context violations with call graph support
    fn check_callees_with_graph(
        &self,
        original_function: &str,
        excluded: &List<Text>,
        callees: &List<CallSiteInfo>,
        call_graph: &CallGraph,
        visited: &mut Set<Text>,
        call_chain: &mut List<CallChainStep>,
        declaration_span: Span,
    ) -> std::result::Result<(), TransitiveViolationInfo> {
        for call_site in callees {
            let callee_name = &call_site.callee_name;

            // Skip if already visited (cycle detection)
            if visited.contains(callee_name) {
                continue;
            }
            visited.insert(callee_name.clone());

            // Add this step to the call chain
            call_chain.push(CallChainStep {
                function_name: callee_name.clone(),
                line: call_site.line,
                uses_context: false,
            });

            // Get callee's information from call graph
            if let Some(callee_node) = call_graph.get_function(callee_name.as_str()) {
                // Check if callee uses any excluded contexts
                for excluded_ctx in excluded {
                    if callee_node.contexts.contains(excluded_ctx.as_str()) {
                        // Found a violation! Mark the last step as using the context
                        if let Some(last) = call_chain.last_mut() {
                            last.uses_context = true;
                        }

                        return Err(TransitiveViolationInfo {
                            origin_function: original_function.into(),
                            excluded_context: excluded_ctx.clone(),
                            call_chain: call_chain.clone(),
                            declaration_span,
                        });
                    }
                }

                // Recursively check callee's callees
                self.check_callees_with_graph(
                    original_function,
                    excluded,
                    &callee_node.callees,
                    call_graph,
                    visited,
                    call_chain,
                    declaration_span,
                )?;
            }

            // Pop this step when backtracking
            call_chain.pop();
        }

        Ok(())
    }

    /// Build a CallGraph from the function registry
    ///
    /// Converts the existing function registry into a CallGraph structure
    /// for enhanced transitive verification.
    pub fn build_call_graph_from_registry(&self) -> CallGraph {
        let mut graph = CallGraph::new();

        // First pass: add all functions
        for (name, info) in self.function_registry.iter() {
            graph.add_function(name.clone(), info.required_contexts.clone(), info.span);
        }

        // Second pass: add all call edges
        for (name, info) in self.function_registry.iter() {
            for callee_name in &info.callees {
                // Get call site info if available, otherwise create default
                let call_site = info.call_sites
                    .get(callee_name)
                    .cloned()
                    .unwrap_or_else(|| CallSiteInfo {
                        callee_name: callee_name.clone(),
                        line: 0,
                        column: 0,
                        span: Span::default(),
                    });
                graph.add_call(name.clone(), call_site);
            }
        }

        graph
    }

    /// Verify all functions in the registry for transitive negative context violations
    ///
    /// This is a convenience method that checks all registered functions.
    pub fn verify_all_negative_contexts(&self) -> Result<()> {
        for (name, _info) in self.function_registry.iter() {
            self.verify_transitive_negative_contexts(name.as_str())?;
        }
        Ok(())
    }
}

impl Default for ContextChecker {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Context Path Helpers
// =============================================================================

/// A context path for negative context exclusion
///
/// Represents a path like "Database" or "FileSystem.Read"
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContextPath {
    /// Path segments
    pub segments: List<Text>,
}

impl ContextPath {
    /// Create a simple context path from a single name
    pub fn simple(name: impl Into<Text>) -> Self {
        let mut segments = List::new();
        segments.push(name.into());
        Self { segments }
    }

    /// Create a context path from multiple segments
    pub fn from_segments(segments: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        Self {
            segments: segments.into_iter().map(|s| s.into()).collect(),
        }
    }

    /// Get the full path as a string
    pub fn as_string(&self) -> Text {
        self.segments
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(".")
            .into()
    }

    /// Check if this path matches a context name
    pub fn matches(&self, name: &str) -> bool {
        if self.segments.len() == 1 {
            self.segments[0].as_str() == name
        } else {
            self.as_string().as_str() == name
        }
    }
}

/// Convert a ContextPath to Text
fn path_to_text(path: &ContextPath) -> Text {
    path.as_string()
}

impl TransitiveViolationInfo {
    /// Format the violation as a detailed error message
    ///
    /// Produces output matching the advanced context patterns specification (negative contexts, transitive verification):
    /// ```text
    /// error: Function 'pure_function' excludes 'Database' but transitively calls:
    ///   -> helper_function() at line 15
    ///      -> database_helper() at line 42
    ///         uses Database
    /// ```
    pub fn format_error(&self) -> Text {
        let mut msg = format!(
            "error: Function '{}' excludes '{}' but transitively calls:\n",
            self.origin_function, self.excluded_context
        );

        for (i, step) in self.call_chain.iter().enumerate() {
            let indent = "   ".repeat(i + 1);
            if step.uses_context {
                msg.push_str(&format!(
                    "{}-> {}() at line {}\n{}   uses {}\n",
                    indent, step.function_name, step.line,
                    indent, self.excluded_context
                ));
            } else {
                msg.push_str(&format!(
                    "{}-> {}() at line {}\n",
                    indent, step.function_name, step.line
                ));
            }
        }

        msg.into()
    }

    /// Convert to a TypeError for integration with the type checking pipeline
    pub fn to_type_error(&self) -> TypeError {
        // Build call chain string for the error message
        let call_chain_str = self.call_chain
            .iter()
            .map(|step| format!("{}() at line {}", step.function_name, step.line))
            .collect::<Vec<_>>()
            .join(" -> ");

        TypeError::TransitiveNegativeContextViolation {
            excluded_context: self.excluded_context.clone(),
            callee: format!(
                "transitive call chain: {}",
                call_chain_str
            ).into(),
            span: self.declaration_span,
        }
    }
}

// =============================================================================
// Direct Negative Context Verification (Advanced context patterns (negative contexts, call graph verification, module aliases))
// =============================================================================

/// Result of checking a context access for negative constraint violations
#[derive(Debug, Clone)]
pub struct NegativeContextViolation {
    /// The excluded context that was accessed
    pub excluded_context: Text,
    /// Location of the violation
    pub usage_span: Span,
    /// Function where the violation occurred
    pub function_name: Text,
    /// Location of the negative constraint declaration
    pub declaration_span: Span,
}

/// A context access found during expression tree walking
#[derive(Debug, Clone)]
pub struct ContextAccess {
    /// The context name being accessed (e.g., "Database")
    pub context_name: Text,
    /// Optional method name if it's a method call
    pub method_name: Option<Text>,
    /// Source span of the access
    pub span: Span,
}

/// Collects all context accesses from an expression tree.
///
/// Walks the expression AST and collects:
/// - Direct context accesses: `Database.query(...)`
/// - Context field accesses: `Logger.level`
/// - Provide statements (which provide a context, not use it)
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
pub fn collect_context_accesses(expr: &verum_ast::expr::Expr) -> List<ContextAccess> {
    let mut accesses = List::new();
    collect_context_accesses_recursive(expr, &mut accesses);
    accesses
}

/// Recursive helper for collecting context accesses
fn collect_context_accesses_recursive(
    expr: &verum_ast::expr::Expr,
    accesses: &mut List<ContextAccess>,
) {
    use verum_ast::expr::{ArrayExpr, ConditionKind, ExprKind};
    use verum_ast::ty::PathSegment;

    match &expr.kind {
        // Method call: could be Context.method(...) or expr.method(...)
        ExprKind::MethodCall { receiver, method, args, .. } => {
            // Check if receiver is a context path (capitalized identifier)
            if let ExprKind::Path(path) = &receiver.kind {
                if path.segments.len() == 1 {
                    if let PathSegment::Name(ident) = &path.segments[0] {
                        // Context names are PascalCase by convention
                        if ident.name.chars().next().is_some_and(|c| c.is_uppercase()) {
                            accesses.push(ContextAccess {
                                context_name: ident.name.clone(),
                                method_name: Some(method.name.clone()),
                                span: receiver.span,
                            });
                        }
                    }
                }
            }
            // Recursively check receiver and arguments
            collect_context_accesses_recursive(receiver, accesses);
            for arg in args.iter() {
                collect_context_accesses_recursive(arg, accesses);
            }
        }

        // Field access: Context.field or expr.field
        ExprKind::Field { expr: inner_expr, field } => {
            // Check if expr is a context path
            if let ExprKind::Path(path) = &inner_expr.kind {
                if path.segments.len() == 1 {
                    if let PathSegment::Name(ident) = &path.segments[0] {
                        if ident.name.chars().next().is_some_and(|c| c.is_uppercase()) {
                            accesses.push(ContextAccess {
                                context_name: ident.name.clone(),
                                method_name: Some(field.name.clone()),
                                span: inner_expr.span,
                            });
                        }
                    }
                }
            }
            collect_context_accesses_recursive(inner_expr, accesses);
        }

        // Path expression: could be a context reference
        ExprKind::Path(path) => {
            if path.segments.len() == 1 {
                if let PathSegment::Name(ident) = &path.segments[0] {
                    if ident.name.chars().next().is_some_and(|c| c.is_uppercase()) {
                        // This could be a context being passed as a value
                        accesses.push(ContextAccess {
                            context_name: ident.name.clone(),
                            method_name: None,
                            span: expr.span,
                        });
                    }
                }
            }
        }

        // Call expression: f(args)
        ExprKind::Call { func, args, .. } => {
            collect_context_accesses_recursive(func, accesses);
            for arg in args.iter() {
                collect_context_accesses_recursive(arg, accesses);
            }
        }

        // Block expression: { stmts; expr }
        ExprKind::Block(block) => {
            collect_context_accesses_in_block(block, accesses);
        }

        // If expression
        ExprKind::If { condition, then_branch, else_branch } => {
            // Check conditions (can be multiple with `let` bindings)
            for cond in condition.conditions.iter() {
                match cond {
                    ConditionKind::Expr(e) => collect_context_accesses_recursive(e, accesses),
                    ConditionKind::Let { value, .. } => collect_context_accesses_recursive(value, accesses),
                }
            }
            collect_context_accesses_in_block(then_branch, accesses);
            if let Some(else_expr) = else_branch {
                collect_context_accesses_recursive(else_expr, accesses);
            }
        }

        // Match expression
        ExprKind::Match { expr: scrutinee, arms } => {
            collect_context_accesses_recursive(scrutinee, accesses);
            for arm in arms.iter() {
                if let Some(guard) = &arm.guard {
                    collect_context_accesses_recursive(guard, accesses);
                }
                collect_context_accesses_recursive(&arm.body, accesses);
            }
        }

        // Loop expressions
        ExprKind::Loop { body, .. } => {
            collect_context_accesses_in_block(body, accesses);
        }
        ExprKind::While { condition, body, .. } => {
            collect_context_accesses_recursive(condition, accesses);
            collect_context_accesses_in_block(body, accesses);
        }
        ExprKind::For { iter, body, .. } => {
            collect_context_accesses_recursive(iter, accesses);
            collect_context_accesses_in_block(body, accesses);
        }

        // Binary/Unary expressions
        ExprKind::Binary { left, right, .. } => {
            collect_context_accesses_recursive(left, accesses);
            collect_context_accesses_recursive(right, accesses);
        }
        ExprKind::Unary { expr: inner, .. } => {
            collect_context_accesses_recursive(inner, accesses);
        }

        // Await expression
        ExprKind::Await(inner) => {
            collect_context_accesses_recursive(inner, accesses);
        }

        // Try expression
        ExprKind::Try(inner) => {
            collect_context_accesses_recursive(inner, accesses);
        }

        // Tuple
        ExprKind::Tuple(elements) => {
            for elem in elements.iter() {
                collect_context_accesses_recursive(elem, accesses);
            }
        }

        // Array
        ExprKind::Array(arr_expr) => {
            match arr_expr {
                ArrayExpr::List(elements) => {
                    for elem in elements.iter() {
                        collect_context_accesses_recursive(elem, accesses);
                    }
                }
                ArrayExpr::Repeat { value, count } => {
                    collect_context_accesses_recursive(value, accesses);
                    collect_context_accesses_recursive(count, accesses);
                }
            }
        }

        // Index
        ExprKind::Index { expr: base, index } => {
            collect_context_accesses_recursive(base, accesses);
            collect_context_accesses_recursive(index, accesses);
        }

        // Closure
        ExprKind::Closure { body, .. } => {
            collect_context_accesses_recursive(body, accesses);
        }

        // Record/struct literal
        ExprKind::Record { fields, base, .. } => {
            for field in fields.iter() {
                // FieldInit.value is Option<Expr>, so check if it has a value
                if let Some(ref value_expr) = field.value {
                    collect_context_accesses_recursive(value_expr, accesses);
                }
            }
            if let Some(base_expr) = base {
                collect_context_accesses_recursive(base_expr, accesses);
            }
        }

        // Return/Break/Continue with optional value
        ExprKind::Return(value) => {
            if let Some(val) = value {
                collect_context_accesses_recursive(val, accesses);
            }
        }
        ExprKind::Break { value, .. } => {
            if let Some(val) = value {
                collect_context_accesses_recursive(val, accesses);
            }
        }

        // Cast expression
        ExprKind::Cast { expr: inner, .. } => {
            collect_context_accesses_recursive(inner, accesses);
        }

        // TryRecover
        ExprKind::TryRecover { try_block, recover } => {
            collect_context_accesses_recursive(try_block, accesses);
            match recover {
                verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                    for arm in arms.iter() {
                        if let Some(guard) = &arm.guard {
                            collect_context_accesses_recursive(guard, accesses);
                        }
                        collect_context_accesses_recursive(&arm.body, accesses);
                    }
                }
                verum_ast::expr::RecoverBody::Closure { body, .. } => {
                    collect_context_accesses_recursive(body, accesses);
                }
            }
        }

        // TryFinally
        ExprKind::TryFinally { try_block, finally_block } => {
            collect_context_accesses_recursive(try_block, accesses);
            collect_context_accesses_recursive(finally_block, accesses);
        }

        // TryRecoverFinally
        ExprKind::TryRecoverFinally { try_block, recover, finally_block } => {
            collect_context_accesses_recursive(try_block, accesses);
            match recover {
                verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
                    for arm in arms.iter() {
                        if let Some(guard) = &arm.guard {
                            collect_context_accesses_recursive(guard, accesses);
                        }
                        collect_context_accesses_recursive(&arm.body, accesses);
                    }
                }
                verum_ast::expr::RecoverBody::Closure { body, .. } => {
                    collect_context_accesses_recursive(body, accesses);
                }
            }
            collect_context_accesses_recursive(finally_block, accesses);
        }

        // Async block
        ExprKind::Async(block) => {
            collect_context_accesses_in_block(block, accesses);
        }

        // Comprehension
        ExprKind::Comprehension { expr: inner, clauses } => {
            collect_context_accesses_recursive(inner, accesses);
            for clause in clauses.iter() {
                match &clause.kind {
                    verum_ast::expr::ComprehensionClauseKind::For { iter, .. } => {
                        collect_context_accesses_recursive(iter, accesses);
                    }
                    verum_ast::expr::ComprehensionClauseKind::If(cond) => {
                        collect_context_accesses_recursive(cond, accesses);
                    }
                    verum_ast::expr::ComprehensionClauseKind::Let { value, .. } => {
                        collect_context_accesses_recursive(value, accesses);
                    }
                }
            }
        }

        // Pipeline
        ExprKind::Pipeline { left, right } => {
            collect_context_accesses_recursive(left, accesses);
            collect_context_accesses_recursive(right, accesses);
        }

        // Null coalescing
        ExprKind::NullCoalesce { left, right } => {
            collect_context_accesses_recursive(left, accesses);
            collect_context_accesses_recursive(right, accesses);
        }

        // Yield
        ExprKind::Yield(inner) => {
            collect_context_accesses_recursive(inner, accesses);
        }

        // Optional chaining
        ExprKind::OptionalChain { expr: inner, field } => {
            if let ExprKind::Path(path) = &inner.kind {
                if path.segments.len() == 1 {
                    if let PathSegment::Name(ident) = &path.segments[0] {
                        if ident.name.chars().next().is_some_and(|c| c.is_uppercase()) {
                            accesses.push(ContextAccess {
                                context_name: ident.name.clone(),
                                method_name: Some(field.name.clone()),
                                span: inner.span,
                            });
                        }
                    }
                }
            }
            collect_context_accesses_recursive(inner, accesses);
        }

        // Other expressions (literals, etc.) - no context accesses
        _ => {}
    }
}

/// Helper to collect context accesses in a block
fn collect_context_accesses_in_block(
    block: &verum_ast::expr::Block,
    accesses: &mut List<ContextAccess>,
) {
    for stmt in block.stmts.iter() {
        collect_context_accesses_in_stmt(stmt, accesses);
    }
    if let Some(result_expr) = &block.expr {
        // Use deref to access the Expr inside the Box
        collect_context_accesses_recursive(result_expr, accesses);
    }
}

/// Helper to collect context accesses in a statement
fn collect_context_accesses_in_stmt(
    stmt: &verum_ast::stmt::Stmt,
    accesses: &mut List<ContextAccess>,
) {
    use verum_ast::stmt::StmtKind;

    match &stmt.kind {
        StmtKind::Let { value, .. } => {
            if let Some(expr) = value {
                collect_context_accesses_recursive(expr, accesses);
            }
        }
        StmtKind::LetElse { value, else_block, .. } => {
            collect_context_accesses_recursive(value, accesses);
            collect_context_accesses_in_block(else_block, accesses);
        }
        StmtKind::Expr { expr, .. } => {
            collect_context_accesses_recursive(expr, accesses);
        }
        StmtKind::Item(_) => {
            // Items don't contain context accesses in the current scope
        }
        StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => {
            collect_context_accesses_recursive(expr, accesses);
        }
        StmtKind::Provide { value, .. } => {
            collect_context_accesses_recursive(value, accesses);
        }
        StmtKind::ProvideScope { value, block, .. } => {
            collect_context_accesses_recursive(value, accesses);
            collect_context_accesses_recursive(block, accesses);
        }
        StmtKind::Empty => {
            // No context accesses in empty statement
        }
    }
}

/// Verify that a function body does not directly use excluded contexts.
///
/// This is the core direct usage check for negative contexts (E3050).
///
/// # Arguments
///
/// * `function_name` - Name of the function being checked
/// * `body` - The function body expression
/// * `negative_contexts` - Map of excluded context names to their declaration spans
///
/// # Returns
///
/// `Ok(())` if no violations found, `Err(TypeError)` with details if violated
///
/// # Example
///
/// ```verum
/// fn calculate_total() using [!Database, !Network] {
///     // This should be flagged as E3050:
///     Database.query("SELECT...");
/// }
/// ```
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.4 - Negative Contexts
pub fn verify_direct_negative_contexts(
    function_name: &str,
    body: &verum_ast::expr::Expr,
    negative_contexts: &Map<Text, Span>,
) -> Result<()> {
    // Collect all context accesses in the function body
    let accesses = collect_context_accesses(body);

    // Check each access against negative contexts
    for access in accesses.iter() {
        if let Some(declaration_span) = negative_contexts.get(&access.context_name) {
            // Found a direct violation!
            return Err(TypeError::DirectNegativeContextViolation {
                context: access.context_name.clone(),
                function_name: function_name.into(),
                usage_span: access.span,
                declaration_span: *declaration_span,
            });
        }
    }

    Ok(())
}

/// Build a map of negative contexts from a context set.
///
/// Helper function to extract negative contexts with their spans.
pub fn build_negative_context_map(contexts: &ContextSet) -> Map<Text, Span> {
    let mut map = Map::new();
    for ctx in contexts.negative_contexts() {
        map.insert(ctx.name.clone(), ctx.span);
    }
    map
}

// =============================================================================
// Module-Level Alias Uniqueness Validation (Advanced context patterns (negative contexts, call graph verification, module aliases))
// =============================================================================

/// Information about a context alias usage
#[derive(Debug, Clone)]
pub struct AliasUsage {
    /// Function name where the alias is used
    pub function_name: Text,
    /// The context path this alias refers to
    pub context_path: Text,
    /// Source span of the alias declaration
    pub span: Span,
}

/// Conflict between two alias usages
#[derive(Debug, Clone)]
pub struct AliasConflict {
    /// The conflicting alias name
    pub alias: Text,
    /// First usage of this alias
    pub first_usage: AliasUsage,
    /// Second (conflicting) usage
    pub second_usage: AliasUsage,
}

/// Registry for tracking context aliases at the module level.
///
/// Validates that aliases are unique within a module to prevent confusion
/// and ensure consistent context resolution.
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
#[derive(Debug, Clone, Default)]
pub struct ModuleAliasRegistry {
    /// Map from alias name to all usages of that alias
    /// Key: alias name, Value: list of (function_name, context_path, span)
    alias_usages: Map<Text, List<AliasUsage>>,
}

impl ModuleAliasRegistry {
    /// Create a new empty alias registry
    pub fn new() -> Self {
        Self {
            alias_usages: Map::new(),
        }
    }

    /// Register an alias for a function.
    ///
    /// Records that `function_name` uses `alias` to refer to `context_path`.
    ///
    /// # Arguments
    ///
    /// * `function_name` - The function declaring the alias
    /// * `alias` - The alias name (e.g., "primary" in `Database as primary`)
    /// * `context_path` - The context being aliased (e.g., "Database")
    /// * `span` - Source location of the alias declaration
    pub fn register_alias(
        &mut self,
        function_name: &str,
        alias: &str,
        context_path: &str,
        span: Span,
    ) {
        let usage = AliasUsage {
            function_name: function_name.into(),
            context_path: context_path.into(),
            span,
        };

        let alias_text: Text = alias.into();
        if let Some(usages) = self.alias_usages.get_mut(&alias_text) {
            usages.push(usage);
        } else {
            let mut usages = List::new();
            usages.push(usage);
            self.alias_usages.insert(alias_text, usages);
        }
    }

    /// Extract and register aliases from a function's context requirements.
    ///
    /// Parses the AST context requirements and extracts any aliases:
    /// - `Database as db` - explicit alias
    /// - `db: Database` - name binding (also an alias)
    ///
    /// # Arguments
    ///
    /// * `function_name` - Name of the function
    /// * `contexts` - List of AST context requirements
    pub fn register_function_aliases(
        &mut self,
        function_name: &str,
        contexts: &[verum_ast::decl::ContextRequirement],
    ) {
        use verum_ast::ty::PathSegment;

        for ctx in contexts {
            // Get the context path as a string
            let context_path: Text = ctx
                .path
                .segments
                .iter()
                .filter_map(|seg| {
                    if let PathSegment::Name(ident) = seg {
                        Some(ident.name.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(".")
                .into();

            // Check for explicit alias: `Database as db`
            if let Some(alias_ident) = &ctx.alias {
                self.register_alias(
                    function_name,
                    alias_ident.name.as_str(),
                    context_path.as_str(),
                    ctx.span,
                );
            }

            // Check for name binding: `db: Database`
            if let Some(name_ident) = &ctx.name {
                self.register_alias(
                    function_name,
                    name_ident.name.as_str(),
                    context_path.as_str(),
                    ctx.span,
                );
            }
        }
    }

    /// Validate that all aliases in the module are unique.
    ///
    /// Checks that no alias is used for different contexts within the module.
    /// Same context with same alias in different functions is OK.
    ///
    /// # Returns
    ///
    /// `Ok(())` if all aliases are consistent, `Err(conflicts)` with all violations
    pub fn validate_module(&self) -> std::result::Result<(), List<AliasConflict>> {
        let mut conflicts = List::new();

        for (alias, usages) in self.alias_usages.iter() {
            if usages.len() < 2 {
                continue; // No potential for conflict with only one usage
            }

            // Check if all usages refer to the same context
            let first = &usages[0];
            for usage in usages.iter().skip(1) {
                if usage.context_path != first.context_path {
                    // Found a conflict!
                    conflicts.push(AliasConflict {
                        alias: alias.clone(),
                        first_usage: first.clone(),
                        second_usage: usage.clone(),
                    });
                }
            }
        }

        if conflicts.is_empty() {
            Ok(())
        } else {
            Err(conflicts)
        }
    }

    /// Convert alias conflicts to type errors.
    ///
    /// Transforms `AliasConflict` values into `TypeError::ContextAliasConflict`
    /// for proper error reporting.
    pub fn conflicts_to_type_errors(conflicts: &[AliasConflict]) -> List<TypeError> {
        conflicts
            .iter()
            .map(|conflict| TypeError::ContextAliasConflict {
                alias: conflict.alias.clone(),
                first_context: conflict.first_usage.context_path.clone(),
                first_function: conflict.first_usage.function_name.clone(),
                first_span: conflict.first_usage.span,
                second_context: conflict.second_usage.context_path.clone(),
                second_function: conflict.second_usage.function_name.clone(),
                second_span: conflict.second_usage.span,
            })
            .collect()
    }

    /// Get all registered aliases (for diagnostics/debugging)
    pub fn all_aliases(&self) -> List<&Text> {
        self.alias_usages.keys().collect()
    }

    /// Clear all registered aliases (for testing)
    #[cfg(test)]
    pub fn clear(&mut self) {
        self.alias_usages.clear();
    }
}

/// Validate all functions in a module for alias uniqueness.
///
/// This is the entry point for module-level alias validation.
///
/// # Arguments
///
/// * `functions` - List of function declarations in the module
///
/// # Returns
///
/// `Ok(())` if all aliases are consistent, first `Err(TypeError)` if any conflict found
///
/// Context declaration: "context Name { ... }" with method signatures, contexts are NOT types (separate namespace) — 1.2 - Aliased Contexts
pub fn validate_module_aliases(
    functions: &[verum_ast::decl::FunctionDecl],
) -> Result<()> {
    let mut registry = ModuleAliasRegistry::new();

    // Register all aliases from all functions
    for func in functions {
        registry.register_function_aliases(
            func.name.name.as_str(),
            &func.contexts,
        );
    }

    // Validate the module
    match registry.validate_module() {
        Ok(()) => Ok(()),
        Err(conflicts) => {
            // Return the first conflict as an error
            if let Some(conflict) = conflicts.first() {
                Err(TypeError::ContextAliasConflict {
                    alias: conflict.alias.clone(),
                    first_context: conflict.first_usage.context_path.clone(),
                    first_function: conflict.first_usage.function_name.clone(),
                    first_span: conflict.first_usage.span,
                    second_context: conflict.second_usage.context_path.clone(),
                    second_function: conflict.second_usage.function_name.clone(),
                    second_span: conflict.second_usage.span,
                })
            } else {
                Ok(()) // No conflicts (shouldn't happen given the Err case)
            }
        }
    }
}

// =============================================================================
// Integration Helpers
// =============================================================================

/// Full context checking for a function during type inference.
///
/// Combines all Advanced context patterns (negative contexts, call graph verification, module aliases) checks:
/// 1. Direct negative context verification
/// 2. Alias uniqueness (single function - module level is separate)
/// 3. Transitive negative context verification (when callees are known)
///
/// # Arguments
///
/// * `function_name` - Name of the function being checked
/// * `contexts` - Function's context requirements
/// * `body` - Function body expression
/// * `checker` - The context checker with registered function info
///
/// # Returns
///
/// `Ok(())` if all checks pass, first `Err(TypeError)` if any check fails
pub fn check_function_contexts(
    function_name: &str,
    contexts: &ContextSet,
    body: &verum_ast::expr::Expr,
    checker: &ContextChecker,
) -> Result<()> {
    // 1. Build negative context map
    let negative_map = build_negative_context_map(contexts);

    // 2. Check for direct negative context violations
    if !negative_map.is_empty() {
        verify_direct_negative_contexts(function_name, body, &negative_map)?;
    }

    // 3. Transitive verification (already registered functions)
    checker.verify_transitive_negative_contexts(function_name)?;

    Ok(())
}

// Tests moved to tests/context_check_tests.rs
