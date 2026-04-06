//! Meta Registry - Global registry for meta functions and macros
//!
//! This module implements the cross-file resolution system for meta functions
//! and macro definitions during Pass 1 of the multi-pass compilation pipeline.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//!
//! # Lock Poisoning Recovery
//!
//! This module uses mutexes that may become poisoned if a thread panics
//! while holding a lock. Recovery strategy:
//! - **meta_functions, macros, dependencies**: Compile-time data → recover with warning
//!   These registries hold AST data collected during compilation. If poisoned,
//!   it means a panic during registration (e.g., during AST traversal). We recover
//!   because the data is being built incrementally and partial data is acceptable
//!   (compilation will fail anyway if data is incomplete).
//!
//! See helper function for recovery rationale.

use std::sync::{Arc, Mutex, MutexGuard};
use verum_ast::{Span, context::ContextRequirement, decl::FunctionDecl, expr::Expr, ty::Type};
use verum_common::{List, Map, Maybe, Set, Text};

/// Acquires lock on meta functions map, recovering from poisoned state if necessary.
///
/// # Safety
/// If poisoned, the Map may have partial/duplicate entries from a failed registration.
/// We recover because:
/// 1. Compilation is incremental - partial data is acceptable
/// 2. Type errors will catch inconsistencies later
/// 3. We must allow compilation to continue to report all errors
fn lock_meta_functions_with_recovery(
    mutex: &Mutex<Map<(Text, Text), MetaFunction>>,
) -> MutexGuard<'_, Map<(Text, Text), MetaFunction>> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                "Meta functions registry mutex poisoned (panic during registration). \
                 Recovering - registry may be incomplete."
            );
            poisoned.into_inner()
        }
    }
}

/// Acquires lock on macros map, recovering from poisoned state if necessary.
fn lock_macros_with_recovery(
    mutex: &Mutex<Map<(Text, Text), MacroDefinition>>,
) -> MutexGuard<'_, Map<(Text, Text), MacroDefinition>> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                "Macros registry mutex poisoned (panic during registration). \
                 Recovering - registry may be incomplete."
            );
            poisoned.into_inner()
        }
    }
}

/// Acquires lock on dependencies map, recovering from poisoned state if necessary.
fn lock_dependencies_with_recovery(
    mutex: &Mutex<Map<Text, List<Text>>>,
) -> MutexGuard<'_, Map<Text, List<Text>>> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                "Dependencies graph mutex poisoned (panic during registration). \
                 Recovering - dependency graph may be incomplete."
            );
            poisoned.into_inner()
        }
    }
}

/// Acquires lock on extern functions set, recovering from poisoned state if necessary.
fn lock_extern_functions_with_recovery(
    mutex: &Mutex<Set<(Text, Text)>>,
) -> MutexGuard<'_, Set<(Text, Text)>> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(
                "Extern functions registry mutex poisoned (panic during registration). \
                 Recovering - registry may be incomplete."
            );
            poisoned.into_inner()
        }
    }
}

/// Global registry for meta functions and macros (cross-file resolution)
///
/// Thread-safe registry that maintains all meta functions and macros
/// discovered during Pass 1 of compilation.
#[derive(Debug, Clone)]
pub struct MetaRegistry {
    /// Map: (module_path, function_name) -> meta function
    meta_functions: Arc<Mutex<Map<(Text, Text), MetaFunction>>>,

    /// Map: (module_path, macro_name) -> macro definition
    macros: Arc<Mutex<Map<(Text, Text), MacroDefinition>>>,

    /// Dependency graph for ordering
    dependencies: Arc<Mutex<Map<Text, List<Text>>>>,

    /// Set of extern (FFI) function names: (module_path, function_name)
    /// Used to detect and block FFI calls in meta functions
    extern_functions: Arc<Mutex<Set<(Text, Text)>>>,
}

/// A meta function that executes at compile-time
#[derive(Debug, Clone)]
pub struct MetaFunction {
    /// Function name
    pub name: Text,

    /// Module path where defined
    pub module: Text,

    /// Function parameters
    pub params: List<MetaParam>,

    /// Return type
    pub return_type: Type,

    /// Function body (AST)
    pub body: Expr,

    /// Context requirements from `using [...]` clause
    /// These contexts are enabled when evaluating this function
    pub contexts: List<ContextRequirement>,

    /// Whether this is an async meta function
    pub is_async: bool,

    /// Whether this macro is @transparent (disables hygiene)
    pub is_transparent: bool,

    /// Stage level (1 = meta fn, 2 = meta(2) fn, etc.)
    /// Used by the evaluator to set the correct execution stage context.
    pub stage_level: u32,

    /// Source span
    pub span: Span,
}

/// A meta function parameter
#[derive(Debug, Clone)]
pub struct MetaParam {
    /// Parameter name
    pub name: Text,

    /// Parameter type
    pub ty: Type,

    /// Whether this is a meta parameter (compile-time value)
    pub is_meta: bool,
}

/// A macro definition
#[derive(Debug, Clone)]
pub struct MacroDefinition {
    /// Macro name
    pub name: Text,

    /// Macro kind (derive, attribute, procedural)
    pub kind: MacroKind,

    /// Function name that performs the expansion
    pub expander: Text,

    /// Module where defined
    pub module: Text,

    /// Source span
    pub span: Span,
}

/// The kind of macro
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroKind {
    /// Derive macro like #[derive(Debug)]
    Derive,

    /// Attribute macro like #[tagged_literal(...)]
    Attribute,

    /// Procedural macro (custom)
    Procedural,
}

/// Error type for meta registry operations
#[derive(Debug, Clone)]
pub enum MetaError {
    /// Duplicate meta function definition
    DuplicateMetaFunction {
        name: Text,
        module: Text,
        span: Span,
        existing_span: Span,
    },

    /// Duplicate macro definition
    DuplicateMacro {
        name: Text,
        module: Text,
        span: Span,
        existing_span: Span,
    },

    /// Meta function not found
    MetaFunctionNotFound { name: Text, module: Text },

    /// Macro not found
    MacroNotFound { name: Text, module: Text },

    /// Circular dependency detected
    CircularDependency { modules: List<Text> },
}

impl std::fmt::Display for MetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetaError::DuplicateMetaFunction { name, module, .. } => {
                write!(
                    f,
                    "Duplicate meta function '{}' in module '{}'",
                    name.as_str(),
                    module.as_str()
                )
            }
            MetaError::DuplicateMacro { name, module, .. } => {
                write!(
                    f,
                    "Duplicate macro '{}' in module '{}'",
                    name.as_str(),
                    module.as_str()
                )
            }
            MetaError::MetaFunctionNotFound { name, module } => {
                write!(
                    f,
                    "Meta function '{}' not found in module '{}'",
                    name.as_str(),
                    module.as_str()
                )
            }
            MetaError::MacroNotFound { name, module } => {
                write!(
                    f,
                    "Macro '{}' not found in module '{}'",
                    name.as_str(),
                    module.as_str()
                )
            }
            MetaError::CircularDependency { modules } => {
                write!(f, "Circular dependency detected: ")?;
                for (i, module) in modules.iter().enumerate() {
                    if i > 0 {
                        write!(f, " -> ")?;
                    }
                    write!(f, "{}", module.as_str())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for MetaError {}

impl MetaRegistry {
    /// Create a new empty meta registry
    pub fn new() -> Self {
        Self {
            meta_functions: Arc::new(Mutex::new(Map::new())),
            macros: Arc::new(Mutex::new(Map::new())),
            dependencies: Arc::new(Mutex::new(Map::new())),
            extern_functions: Arc::new(Mutex::new(Set::new())),
        }
    }

    /// Register an extern (FFI) function during Pass 1
    ///
    /// Extern functions are tracked so that meta function evaluation can
    /// detect and block attempts to call FFI functions at compile time.
    ///
    /// # Arguments
    /// - `module`: Module path where the function is defined
    /// - `name`: The function name
    pub fn register_extern_function(&mut self, module: &Text, name: &Text) {
        let key = (module.clone(), name.clone());
        let mut extern_fns = lock_extern_functions_with_recovery(&self.extern_functions);
        extern_fns.insert(key);
    }

    /// Check if a function is an extern (FFI) function
    ///
    /// Used to detect and block FFI calls in meta functions.
    ///
    /// # Arguments
    /// - `module`: Module path to search in
    /// - `name`: The function name to check
    ///
    /// # Returns
    /// `true` if the function is declared as extern in the given module
    pub fn is_extern_function(&self, module: &Text, name: &Text) -> bool {
        let key = (module.clone(), name.clone());
        let extern_fns = lock_extern_functions_with_recovery(&self.extern_functions);
        extern_fns.contains(&key)
    }

    /// Check if a function name is an extern function in any module
    ///
    /// This is a fallback check when the specific module is unknown.
    pub fn is_any_extern_function(&self, name: &Text) -> bool {
        let extern_fns = lock_extern_functions_with_recovery(&self.extern_functions);
        extern_fns.iter().any(|(_, fn_name)| fn_name == name)
    }

    /// Register a meta function during Pass 1
    ///
    /// # Arguments
    /// - `module`: Module path where the function is defined
    /// - `func`: The function declaration from the AST
    ///
    /// # Errors
    /// Returns `MetaError::DuplicateMetaFunction` if a meta function with the same
    /// name already exists in the same module.
    pub fn register_meta_function(
        &mut self,
        module: &Text,
        func: &FunctionDecl,
    ) -> std::result::Result<(), MetaError> {
        let key = (module.clone(), Text::from(func.name.as_str()));

        // Lock poisoning recovery: Use helper function
        let mut functions = lock_meta_functions_with_recovery(&self.meta_functions);

        // Check for duplicates
        if let Some(existing) = functions.get(&key) {
            return Err(MetaError::DuplicateMetaFunction {
                name: Text::from(func.name.as_str()),
                module: module.clone(),
                span: func.span,
                existing_span: existing.span,
            });
        }

        // Extract parameters
        let params = extract_params(func);

        // Create meta function
        // Convert FunctionBody to Expr
        let body = match &func.body {
            Some(body) => match body {
                verum_ast::decl::FunctionBody::Block(block) => {
                    // Convert Block to block expression
                    Expr::new(verum_ast::expr::ExprKind::Block(block.clone()), func.span)
                }
                verum_ast::decl::FunctionBody::Expr(expr) => expr.clone(),
            },
            None => {
                // Unit/empty expression
                Expr::new(verum_ast::expr::ExprKind::Tuple(List::new()), func.span)
            }
        };

        let meta_func = MetaFunction {
            name: Text::from(func.name.as_str()),
            module: module.clone(),
            params,
            return_type: func
                .return_type
                .clone()
                .unwrap_or_else(|| Type::unit(func.span)),
            body,
            contexts: func.contexts.clone(),
            is_async: func.is_async,
            is_transparent: func.is_transparent,
            stage_level: func.stage_level.max(1), // meta fn = stage 1 minimum
            span: func.span,
        };

        functions.insert(key, meta_func);
        Ok(())
    }

    /// Register a MetaFunction directly without going through FunctionDecl.
    ///
    /// This is used by the staged pipeline to import meta functions from an
    /// external registry with their proper stage-level routing. Unlike
    /// `register_meta_function`, this method takes a pre-constructed MetaFunction.
    ///
    /// # Arguments
    /// * `meta_func` - The pre-constructed MetaFunction to register
    ///
    /// # Errors
    /// Returns `MetaError::DuplicateMetaFunction` if a meta function with the same
    /// name already exists in the same module.
    ///
    /// # Example
    /// ```ignore
    /// let meta_fn = MetaFunction {
    ///     name: Text::from("derive_impl"),
    ///     module: Text::from("my_module"),
    ///     // ... other fields
    /// };
    /// registry.register_meta_fn_direct(meta_fn)?;
    /// ```
    pub fn register_meta_fn_direct(
        &mut self,
        meta_func: MetaFunction,
    ) -> std::result::Result<(), MetaError> {
        let key = (meta_func.module.clone(), meta_func.name.clone());

        // Lock poisoning recovery: Use helper function
        let mut functions = lock_meta_functions_with_recovery(&self.meta_functions);

        // Check for duplicates
        if let Some(existing) = functions.get(&key) {
            return Err(MetaError::DuplicateMetaFunction {
                name: meta_func.name.clone(),
                module: meta_func.module.clone(),
                span: meta_func.span,
                existing_span: existing.span,
            });
        }

        functions.insert(key, meta_func);
        Ok(())
    }

    /// Register a macro definition during Pass 1
    pub fn register_macro(
        &mut self,
        module: &Text,
        name: Text,
        kind: MacroKind,
        expander: Text,
        span: Span,
    ) -> std::result::Result<(), MetaError> {
        let key = (module.clone(), name.clone());

        // Lock poisoning recovery: Use helper function
        let mut macros = lock_macros_with_recovery(&self.macros);

        // Check for duplicates
        if let Some(existing) = macros.get(&key) {
            return Err(MetaError::DuplicateMacro {
                name: name.clone(),
                module: module.clone(),
                span,
                existing_span: existing.span,
            });
        }

        let macro_def = MacroDefinition {
            name,
            kind,
            expander,
            module: module.clone(),
            span,
        };

        macros.insert(key, macro_def);
        Ok(())
    }

    /// Add a dependency between modules
    pub fn add_dependency(&mut self, from: Text, to: Text) {
        // Lock poisoning recovery: Use helper function
        let mut deps = lock_dependencies_with_recovery(&self.dependencies);
        deps.entry(from).or_insert_with(List::new).push(to);
    }

    /// Resolve a meta function call during Pass 2
    ///
    /// Attempts to find the meta function, first in the local module,
    /// then in imported modules according to the dependency graph.
    pub fn resolve_meta_call(&self, module: &Text, name: &Text) -> Maybe<MetaFunction> {
        // Lock poisoning recovery: Use helper function
        let functions = lock_meta_functions_with_recovery(&self.meta_functions);

        // Try local module first
        let local_key = (module.clone(), name.clone());
        if let Some(func) = functions.get(&local_key) {
            return Maybe::Some(func.clone());
        }

        // Try imported modules (check dependencies)
        // Lock poisoning recovery: Use helper function
        let deps = lock_dependencies_with_recovery(&self.dependencies);
        if let Some(dep_list) = deps.get(module) {
            for dep in dep_list.iter() {
                let dep_key = (dep.clone(), name.clone());
                if let Some(func) = functions.get(&dep_key) {
                    return Maybe::Some(func.clone());
                }
            }
        }

        Maybe::None
    }

    /// Get a user-defined meta function by name (for direct lookup)
    ///
    /// This is useful for executing meta functions from the expansion phase.
    pub fn get_user_meta_fn(&self, module: &Text, name: &Text) -> Maybe<MetaFunction> {
        self.resolve_meta_call(module, name)
    }

    /// Resolve a macro definition
    pub fn resolve_macro(&self, module: &Text, name: &Text) -> Maybe<MacroDefinition> {
        // Lock poisoning recovery: Use helper function
        let macros = lock_macros_with_recovery(&self.macros);

        // Try local module first
        let local_key = (module.clone(), name.clone());
        if let Some(macro_def) = macros.get(&local_key) {
            return Maybe::Some(macro_def.clone());
        }

        // Try imported modules
        // Lock poisoning recovery: Use helper function
        let deps = lock_dependencies_with_recovery(&self.dependencies);
        if let Some(dep_list) = deps.get(module) {
            for dep in dep_list.iter() {
                let dep_key = (dep.clone(), name.clone());
                if let Some(macro_def) = macros.get(&dep_key) {
                    return Maybe::Some(macro_def.clone());
                }
            }
        }

        Maybe::None
    }

    /// Get all registered meta functions
    pub fn all_meta_functions(&self) -> List<MetaFunction> {
        // Lock poisoning recovery: Use helper function
        let functions = lock_meta_functions_with_recovery(&self.meta_functions);
        functions.values().cloned().collect()
    }

    /// Get all registered macros
    pub fn all_macros(&self) -> List<MacroDefinition> {
        // Lock poisoning recovery: Use helper function
        let macros = lock_macros_with_recovery(&self.macros);
        macros.values().cloned().collect()
    }

    /// Check for circular dependencies in the module graph
    pub fn check_circular_dependencies(&self) -> std::result::Result<(), MetaError> {
        // Lock poisoning recovery: Use helper function
        let deps = lock_dependencies_with_recovery(&self.dependencies);
        let mut visited = Map::new();
        let mut stack = List::new();

        for module in deps.keys() {
            if !visited.contains_key(module) {
                self.dfs_check(module, &deps, &mut visited, &mut stack)?;
            }
        }

        Ok(())
    }

    fn dfs_check(
        &self,
        module: &Text,
        deps: &Map<Text, List<Text>>,
        visited: &mut Map<Text, bool>,
        stack: &mut List<Text>,
    ) -> std::result::Result<(), MetaError> {
        visited.insert(module.clone(), true);
        stack.push(module.clone());

        if let Some(dep_list) = deps.get(module) {
            for dep in dep_list.iter() {
                if !visited.contains_key(dep) {
                    self.dfs_check(dep, deps, visited, stack)?;
                } else if stack.iter().any(|m| m == dep) {
                    // Circular dependency detected
                    let mut cycle = stack.clone();
                    cycle.push(dep.clone());
                    return Err(MetaError::CircularDependency { modules: cycle });
                }
            }
        }

        stack.pop();
        Ok(())
    }
}

impl Default for MetaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract parameters from a function declaration
fn extract_params(func: &FunctionDecl) -> List<MetaParam> {
    let mut params = List::new();

    for param in &func.params {
        use verum_ast::decl::FunctionParamKind;
        use verum_ast::pattern::PatternKind;

        match &param.kind {
            FunctionParamKind::Regular { pattern, ty, .. } => {
                // Extract name from pattern (usually an identifier pattern)
                let name = match &pattern.kind {
                    PatternKind::Ident { name, .. } => Text::from(name.as_str()),
                    _ => Text::from("_"), // Fallback for complex patterns
                };
                params.push(MetaParam {
                    name,
                    ty: ty.clone(),
                    is_meta: false,
                });
            }
            FunctionParamKind::SelfValue
            | FunctionParamKind::SelfValueMut
            | FunctionParamKind::SelfRef
            | FunctionParamKind::SelfRefMut
            | FunctionParamKind::SelfRefChecked
            | FunctionParamKind::SelfRefCheckedMut
            | FunctionParamKind::SelfRefUnsafe
            | FunctionParamKind::SelfRefUnsafeMut
            | FunctionParamKind::SelfOwn
            | FunctionParamKind::SelfOwnMut => {
                // Skip self params for meta functions
                continue;
            }
        }
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::ExprKind;

    fn make_test_meta_function(name: &str, module: &str) -> MetaFunction {
        MetaFunction {
            name: Text::from(name),
            module: Text::from(module),
            params: List::new(),
            return_type: Type::unit(Span::default()),
            body: Expr::new(ExprKind::Tuple(List::new()), Span::default()),
            contexts: List::new(),
            is_async: false,
            is_transparent: false,
            stage_level: 1,
            span: Span::default(),
        }
    }

    #[test]
    fn test_register_meta_fn_direct_basic() {
        let mut registry = MetaRegistry::new();
        let meta_fn = make_test_meta_function("test_fn", "test_module");

        // Should succeed on first registration
        let result = registry.register_meta_fn_direct(meta_fn);
        assert!(result.is_ok());

        // Should be resolvable
        let resolved = registry.resolve_meta_call(
            &Text::from("test_module"),
            &Text::from("test_fn"),
        );
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().name.as_str(), "test_fn");
    }

    #[test]
    fn test_register_meta_fn_direct_duplicate() {
        let mut registry = MetaRegistry::new();
        let meta_fn1 = make_test_meta_function("dup_fn", "dup_module");
        let meta_fn2 = make_test_meta_function("dup_fn", "dup_module");

        // First registration should succeed
        assert!(registry.register_meta_fn_direct(meta_fn1).is_ok());

        // Second registration with same name/module should fail
        let result = registry.register_meta_fn_direct(meta_fn2);
        assert!(result.is_err());

        match result {
            Err(MetaError::DuplicateMetaFunction { name, module, .. }) => {
                assert_eq!(name.as_str(), "dup_fn");
                assert_eq!(module.as_str(), "dup_module");
            }
            _ => panic!("Expected DuplicateMetaFunction error"),
        }
    }

    #[test]
    fn test_register_meta_fn_direct_different_modules() {
        let mut registry = MetaRegistry::new();
        let meta_fn1 = make_test_meta_function("same_name", "module_a");
        let meta_fn2 = make_test_meta_function("same_name", "module_b");

        // Both should succeed - same name but different modules
        assert!(registry.register_meta_fn_direct(meta_fn1).is_ok());
        assert!(registry.register_meta_fn_direct(meta_fn2).is_ok());

        // Both should be resolvable in their respective modules
        let resolved_a = registry.resolve_meta_call(
            &Text::from("module_a"),
            &Text::from("same_name"),
        );
        let resolved_b = registry.resolve_meta_call(
            &Text::from("module_b"),
            &Text::from("same_name"),
        );

        assert!(resolved_a.is_some());
        assert!(resolved_b.is_some());
        assert_eq!(resolved_a.unwrap().module.as_str(), "module_a");
        assert_eq!(resolved_b.unwrap().module.as_str(), "module_b");
    }

    #[test]
    fn test_all_meta_functions_includes_direct() {
        let mut registry = MetaRegistry::new();

        // Register multiple functions directly
        for i in 0..5 {
            let meta_fn = make_test_meta_function(
                &format!("fn_{}", i),
                "bulk_module",
            );
            registry.register_meta_fn_direct(meta_fn).unwrap();
        }

        let all_fns = registry.all_meta_functions();
        assert_eq!(all_fns.len(), 5);
    }
}
