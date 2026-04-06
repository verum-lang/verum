//! Phase 2: Meta Registry & AST Registration
//!
//! Registers all meta handlers BEFORE any macro expansion.
//!
//! ## Multi-Pass Architecture
//!
//! This phase implements **Pass 1** of the three-pass compilation:
//! - **Pass 1: Parse + Register** (this phase)
//! - Pass 2: Expand Macros (MacroExpansionPhase)
//! - Pass 3: Semantic Analysis (type checking)
//!
//! ## Responsibilities
//!
//! 1. Register @tagged_literal handlers - e.g., `d#"2025-11-05"`
//! 2. Register @derive macros - e.g., `@derive(Serialize)`
//! 3. Register @differentiable functions - for autodiff
//! 4. Register @interpolation_handler - e.g., `sql"SELECT..."`
//! 5. Register meta fn declarations
//! 6. Build complete MetaRegistry (order-independent cross-file resolution)
//!
//! ## Output
//!
//! - Complete MetaRegistry with all handlers
//! - Used by Phase 3 (Macro Expansion)
//!
//! ## Cross-File Resolution
//!
//! The MetaRegistry enables macros defined in one file to be used in any other
//! file, regardless of parsing order. This eliminates the "chicken-and-egg"
//! problem where a macro might be used before its definition is seen.
//!
//! Phase 2: Meta Registry and AST Registration. Registers @tagged_literal handlers,
//! @derive macros, @differentiable functions, @verify annotations, and
//! @interpolation_handler definitions. Order-independent (cross-file resolution).
//! Multi-pass: Pass 1 registers all handlers in MetaRegistry, Pass 2 expands.

use anyhow::Result;
use std::time::Instant;
use verum_ast::expr::ExprKind;
use verum_ast::literal::StringLit;
use verum_ast::{ItemKind, LiteralKind, Module, decl::FunctionDecl};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_common::{List, Text};

use crate::literal_registry::{LiteralRegistry, TaggedLiteralHandler};
use crate::meta::{MacroKind, MetaRegistry};

use super::{CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput};

/// Meta registry phase - Pass 1 of multi-pass compilation
///
/// This phase scans all source files and registers meta definitions:
/// - @tagged_literal handlers → LiteralRegistry
/// - @interpolation_handler handlers → InterpolationRegistry (within MetaRegistry)
/// - @derive macros → MetaRegistry
/// - meta fn declarations → MetaRegistry
///
/// After this phase completes, the MetaRegistry has complete knowledge of all
/// available macros and can be used by Pass 2 (MacroExpansionPhase) for expansion.
pub struct MetaRegistryPhase {
    /// Registry for meta functions and macros (cross-file resolution)
    registry: MetaRegistry,
    /// Registry for tagged literal handlers (compile-time literal parsing)
    literal_registry: LiteralRegistry,
    /// Current module path being processed (for scoped registration)
    current_module: Text,
    /// Statistics for this phase
    stats: RegistrationStats,
}

/// Statistics for meta registration phase
#[derive(Debug, Clone, Default)]
pub struct RegistrationStats {
    /// Number of meta functions registered
    pub meta_functions: usize,
    /// Number of @tagged_literal handlers registered
    pub tagged_literal_handlers: usize,
    /// Number of @interpolation_handler handlers registered
    pub interpolation_handlers: usize,
    /// Number of @derive macros registered
    pub derive_macros: usize,
    /// Number of @differentiable functions registered
    pub differentiable_functions: usize,
}

impl MetaRegistryPhase {
    pub fn new() -> Self {
        let literal_registry = LiteralRegistry::new();
        // Register all built-in handlers (d#"...", rx#"...", etc.)
        literal_registry.register_builtin_handlers();

        Self {
            registry: MetaRegistry::new(),
            literal_registry,
            current_module: Text::from(""),
            stats: RegistrationStats::default(),
        }
    }

    /// Register meta handlers from all modules
    pub fn register_handlers(&mut self, modules: &[Module]) -> Result<(), List<Diagnostic>> {
        let start = Instant::now();
        let mut errors = List::new();

        for module in modules {
            if let Err(mut module_errors) = self.register_module_handlers(module) {
                errors.append(&mut module_errors);
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        // Check for circular dependencies
        if let Err(e) = self.registry.check_circular_dependencies() {
            let diag = DiagnosticBuilder::new(Severity::Error)
                .message(format!("Circular dependency detected: {}", e))
                .build();
            return Err(List::from(vec![diag]));
        }

        let duration = start.elapsed();
        tracing::info!(
            "Registered {} meta functions and {} macros in {:.2}ms",
            self.registry.all_meta_functions().len(),
            self.registry.all_macros().len(),
            duration.as_millis()
        );

        Ok(())
    }

    /// Register handlers from a single module
    fn register_module_handlers(&mut self, module: &Module) -> Result<(), List<Diagnostic>> {
        let mut errors = List::new();

        for item in &module.items {
            match &item.kind {
                // Register meta functions
                ItemKind::Function(func) if func.is_meta => {
                    if let Err(e) = self.register_meta_function(func) {
                        errors.push(e);
                    }
                }

                // Register @derive macros
                ItemKind::Function(func) if self.has_derive_attr(func) => {
                    if let Err(e) = self.register_derive_macro(func) {
                        errors.push(e);
                    }
                }

                // Register @tagged_literal handlers
                ItemKind::Function(func) if self.has_tagged_literal_attr(func) => {
                    if let Err(e) = self.register_tagged_literal_handler(func) {
                        errors.push(e);
                    }
                }

                // Register @differentiable functions
                ItemKind::Function(func) if self.has_differentiable_attr(func) => {
                    if let Err(e) = self.register_differentiable_function(func) {
                        errors.push(e);
                    }
                }

                // Register @interpolation_handler
                ItemKind::Function(func) if self.has_interpolation_handler_attr(func) => {
                    if let Err(e) = self.register_interpolation_handler(func) {
                        errors.push(e);
                    }
                }

                // Register extern (FFI) functions for sandbox detection
                ItemKind::Function(func) if func.extern_abi.is_some() => {
                    self.register_extern_function(func);
                }

                _ => {}
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Register a meta function during Pass 1
    ///
    /// Meta functions are compile-time functions that can:
    /// - Compute constants
    /// - Generate code via quote!
    /// - Perform type-level computations
    ///
    /// # Example
    /// ```verum
    /// meta fn fibonacci(n: Int) -> Int {
    ///     if n <= 1 { n } else { fibonacci(n-1) + fibonacci(n-2) }
    /// }
    /// ```
    fn register_meta_function(&mut self, func: &FunctionDecl) -> Result<(), Diagnostic> {
        tracing::debug!(
            "Registering meta function: {} (async: {})",
            func.name.name,
            func.is_async
        );

        // Register in MetaRegistry
        if let Err(e) = self
            .registry
            .register_meta_function(&self.current_module, func)
        {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!("Failed to register meta function: {}", e))
                .build());
        }

        self.stats.meta_functions += 1;
        Ok(())
    }

    /// Register an extern (FFI) function during Pass 1
    ///
    /// Extern functions are tracked so that meta function evaluation can
    /// detect and block attempts to call FFI functions at compile time.
    ///
    /// # Example
    /// ```verum
    /// extern "C" fn external_function() -> Int;
    /// ```
    fn register_extern_function(&mut self, func: &FunctionDecl) {
        tracing::debug!(
            "Registering extern function: {} (abi: {:?})",
            func.name.name,
            func.extern_abi
        );

        // Register in MetaRegistry for sandbox detection
        self.registry
            .register_extern_function(&self.current_module, &Text::from(func.name.name.as_str()));
    }

    /// Register a @derive macro during Pass 1
    ///
    /// Derive macros automatically generate protocol implementations for types.
    ///
    /// # Example
    /// ```verum
    /// @derive_macro("Builder")
    /// meta fn derive_builder(input: MacroInput) -> Result<TokenStream, MacroError> {
    ///     // Generate builder pattern implementation
    /// }
    /// ```
    fn register_derive_macro(&mut self, func: &FunctionDecl) -> Result<(), Diagnostic> {
        // Extract derive name from @derive_macro("Name") attribute
        let derive_name = self.extract_string_arg_from_attr(func, "derive_macro")?;

        tracing::debug!(
            "Registering derive macro '{}' from function '{}'",
            derive_name,
            func.name.name
        );

        // Register in MetaRegistry
        if let Err(e) = self.registry.register_macro(
            &self.current_module,
            derive_name.clone(),
            MacroKind::Derive,
            Text::from(func.name.name.as_str()),
            func.span,
        ) {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!("Failed to register derive macro: {}", e))
                .build());
        }

        self.stats.derive_macros += 1;
        Ok(())
    }

    /// Register a @tagged_literal handler during Pass 1
    ///
    /// Tagged literal handlers parse and validate domain-specific literals at compile-time.
    ///
    /// # Example
    /// ```verum
    /// @tagged_literal("d")
    /// meta fn date_literal(s: &str) -> Date {
    ///     Date.parse(s).expect("Invalid date")
    /// }
    ///
    /// // Usage: let birthday = d#"1990-01-01"
    /// ```
    ///
    /// # Spec
    /// Tagged literal desugaring: tag#"content" → const_eval of registered handler.
    fn register_tagged_literal_handler(&mut self, func: &FunctionDecl) -> Result<(), Diagnostic> {
        // Extract tag from @tagged_literal("tag") attribute
        let tag = self.extract_string_arg_from_attr(func, "tagged_literal")?;

        tracing::info!(
            "Registering tagged literal handler: tag='{}', handler='{}'",
            tag,
            func.name.name
        );

        // Validate handler signature: meta fn handler(s: &str) -> Type
        self.validate_tagged_literal_signature(func, &tag)?;

        // Create handler registration
        let handler = TaggedLiteralHandler {
            tag: tag.as_str().to_string().into(),
            handler_fn: format!("{}::{}", self.current_module.as_str(), func.name.name).into(),
            compile_time: true, // All @tagged_literal handlers are compile-time
            runtime: false,     // Runtime validation is separate
        };

        // Register in LiteralRegistry
        if let Err(e) = self.literal_registry.register_handler(handler) {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "Failed to register tagged literal handler '{}': {}",
                    tag, e
                ))
                .build());
        }

        // Also register as a macro in MetaRegistry for cross-file resolution
        if let Err(e) = self.registry.register_macro(
            &self.current_module,
            Text::from(format!("tagged_literal_{}", tag)),
            MacroKind::Attribute,
            Text::from(func.name.name.as_str()),
            func.span,
        ) {
            tracing::warn!("Could not register tagged literal in MetaRegistry: {}", e);
        }

        self.stats.tagged_literal_handlers += 1;
        Ok(())
    }

    /// Register a @differentiable function during Pass 1
    ///
    /// Differentiable functions support automatic differentiation for tensor operations.
    ///
    /// # Example
    /// ```verum
    /// @differentiable(wrt = "x, y")
    /// fn loss(x: Tensor, y: Tensor) -> Tensor {
    ///     (x - y).pow(2).mean()
    /// }
    /// ```
    fn register_differentiable_function(&mut self, func: &FunctionDecl) -> Result<(), Diagnostic> {
        tracing::debug!("Registering differentiable function: {}", func.name.name);

        // Extract wrt parameters from @differentiable(wrt = "param1, param2")
        let _wrt_params = self.extract_differentiable_params(func);

        // Register for autodiff processing in later phase
        self.stats.differentiable_functions += 1;
        Ok(())
    }

    /// Register an @interpolation_handler during Pass 1
    ///
    /// Interpolation handlers process text interpolation syntax safely.
    ///
    /// # Example
    /// ```verum
    /// @interpolation_handler("sql")
    /// @safe  // Verified to prevent SQL injection
    /// meta fn sql_interpolate(template: &str, args: &[Expr]) -> TokenStream {
    ///     quote! {
    ///         SqlQuery.with_params(#template, list![#(#args.to_sql_param()),*])
    ///     }
    /// }
    ///
    /// // Usage: let query = sql"SELECT * WHERE id = {user_id}"
    /// ```
    ///
    /// # Spec
    /// Interpolation handler: prefix"template {expr}" → safe parameterized output.
    fn register_interpolation_handler(&mut self, func: &FunctionDecl) -> Result<(), Diagnostic> {
        // Extract handler name from @interpolation_handler("name") attribute
        let handler_name = self.extract_string_arg_from_attr(func, "interpolation_handler")?;

        tracing::info!(
            "Registering interpolation handler: name='{}', handler='{}'",
            handler_name,
            func.name.name
        );

        // Validate handler signature: meta fn handler(template: &str, args: &[Expr]) -> TokenStream
        self.validate_interpolation_handler_signature(func, &handler_name)?;

        // Check for @safe/@unsafe annotations
        let _is_safe = self.has_safe_attr(func);
        let is_unsafe = self.has_unsafe_attr(func);

        if is_unsafe {
            tracing::warn!(
                "Interpolation handler '{}' is marked @unsafe - usage will generate warnings",
                handler_name
            );
        }

        // Register as macro in MetaRegistry
        if let Err(e) = self.registry.register_macro(
            &self.current_module,
            Text::from(format!("interpolation_{}", handler_name)),
            MacroKind::Attribute,
            Text::from(func.name.name.as_str()),
            func.span,
        ) {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "Failed to register interpolation handler '{}': {}",
                    handler_name, e
                ))
                .build());
        }

        self.stats.interpolation_handlers += 1;
        Ok(())
    }

    // ========== Helper Methods ==========

    /// Extract a string argument from an attribute like @attr("value")
    ///
    /// Returns the string value or an error if the attribute doesn't have
    /// exactly one string argument.
    fn extract_string_arg_from_attr(
        &self,
        func: &FunctionDecl,
        attr_name: &str,
    ) -> Result<Text, Diagnostic> {
        // Find the attribute
        let attr = func
            .attributes
            .iter()
            .find(|a| a.name.as_str() == attr_name)
            .ok_or_else(|| {
                DiagnosticBuilder::new(Severity::Error)
                    .message(format!("Missing @{} attribute", attr_name))
                    .build()
            })?;

        // Extract arguments
        match &attr.args {
            Some(args) if args.len() == 1 => {
                // First arg should be a string literal
                if let Some(first_arg) = args.first() {
                    if let ExprKind::Literal(lit) = &first_arg.kind {
                        if let LiteralKind::Text(s) = &lit.kind {
                            return Ok(self.extract_string_content(s).into());
                        }
                    }
                }
                Err(DiagnosticBuilder::new(Severity::Error)
                    .message(format!(
                        "@{} attribute requires a string argument: @{}(\"value\")",
                        attr_name, attr_name
                    ))
                    .build())
            }
            Some(_) => Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "@{} attribute requires exactly one string argument",
                    attr_name
                ))
                .build()),
            None => Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "@{} attribute requires a string argument: @{}(\"value\")",
                    attr_name, attr_name
                ))
                .build()),
        }
    }

    /// Extract string content from StringLit
    fn extract_string_content(&self, s: &StringLit) -> String {
        match s {
            StringLit::Regular(text)
            | StringLit::MultiLine(text) => text.to_string(),
        }
    }

    /// Validate the signature of a @tagged_literal handler
    ///
    /// Must be: meta fn name(s: &str) -> ReturnType
    fn validate_tagged_literal_signature(
        &self,
        func: &FunctionDecl,
        tag: &Text,
    ) -> Result<(), Diagnostic> {
        // Must be a meta function
        if !func.is_meta {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "@tagged_literal(\"{}\") handler must be a meta function (use 'meta fn')",
                    tag
                ))
                .build());
        }

        // Must have exactly one parameter (the string to parse)
        if func.params.len() != 1 {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "@tagged_literal(\"{}\") handler must have exactly one parameter (the string to parse)",
                    tag
                ))
                .help("Signature should be: meta fn handler(s: &str) -> Type".to_string())
                .build());
        }

        // Must have a return type
        if func.return_type.is_none() {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "@tagged_literal(\"{}\") handler must have a return type",
                    tag
                ))
                .help("Signature should be: meta fn handler(s: &str) -> Type".to_string())
                .build());
        }

        Ok(())
    }

    /// Validate the signature of an @interpolation_handler
    ///
    /// Must be: meta fn name(template: &str, args: &[Expr]) -> TokenStream
    fn validate_interpolation_handler_signature(
        &self,
        func: &FunctionDecl,
        handler_name: &Text,
    ) -> Result<(), Diagnostic> {
        // Must be a meta function
        if !func.is_meta {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "@interpolation_handler(\"{}\") handler must be a meta function (use 'meta fn')",
                    handler_name
                ))
                .build());
        }

        // Should have two parameters: template and args
        if func.params.len() != 2 {
            tracing::warn!(
                "@interpolation_handler(\"{}\") handler should have two parameters: \
                (template: &str, args: &[Expr])",
                handler_name
            );
        }

        Ok(())
    }

    /// Extract @differentiable(wrt = "params") parameters
    fn extract_differentiable_params(&self, func: &FunctionDecl) -> List<Text> {
        // Find @differentiable attribute
        let attr = func
            .attributes
            .iter()
            .find(|a| a.name.as_str() == "differentiable");

        if let Some(attr) = attr {
            // Parse wrt = "x, y, z" format
            // For now, just return empty list - full implementation requires named arg parsing
            if let Some(args) = &attr.args {
                tracing::debug!("Differentiable attribute has {} args", args.len());
            }
        }

        List::new()
    }

    /// Check if function has @safe annotation
    fn has_safe_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "safe")
    }

    /// Check if function has @unsafe annotation
    fn has_unsafe_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "unsafe")
    }

    /// Check if function has @derive_macro attribute (defines a derive macro)
    fn has_derive_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "derive_macro")
    }

    /// Check if function has @tagged_literal attribute
    fn has_tagged_literal_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "tagged_literal")
    }

    /// Check if function has @differentiable attribute
    fn has_differentiable_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "differentiable")
    }

    /// Check if function has @interpolation_handler attribute
    fn has_interpolation_handler_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "interpolation_handler")
    }

    /// Check if function has @attribute_macro attribute (defines a custom attribute)
    #[allow(dead_code)] // Reserved for custom attribute macro support
    fn has_attribute_macro_attr(&self, func: &FunctionDecl) -> bool {
        func.attributes
            .iter()
            .any(|attr| attr.name.as_str() == "attribute_macro")
    }

    // ========== Public Accessors ==========

    /// Get the MetaRegistry (for use by later phases)
    pub fn get_registry(&self) -> &MetaRegistry {
        &self.registry
    }

    /// Get the LiteralRegistry (for use by later phases)
    pub fn get_literal_registry(&self) -> &LiteralRegistry {
        &self.literal_registry
    }

    /// Get registration statistics
    pub fn stats(&self) -> &RegistrationStats {
        &self.stats
    }

    /// Take ownership of registries (for transfer to MacroExpansionPhase)
    pub fn into_registries(self) -> (MetaRegistry, LiteralRegistry) {
        (self.registry, self.literal_registry)
    }
}

impl Default for MetaRegistryPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for MetaRegistryPhase {
    fn name(&self) -> &str {
        "Phase 2: Meta Registry & AST Registration"
    }

    fn description(&self) -> &str {
        "Register all meta handlers before macro expansion (Pass 1 of multi-pass compilation)"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        let modules = match &input.data {
            PhaseData::AstModules(modules) => modules,
            _ => {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message("Invalid input for meta registry phase".to_string())
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // Build registry
        let mut phase = MetaRegistryPhase::new();
        phase.register_handlers(modules)?;

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);

        // Add comprehensive statistics
        metrics.add_custom_metric("meta_functions", phase.stats.meta_functions.to_string());
        metrics.add_custom_metric(
            "tagged_literal_handlers",
            phase.stats.tagged_literal_handlers.to_string(),
        );
        metrics.add_custom_metric(
            "interpolation_handlers",
            phase.stats.interpolation_handlers.to_string(),
        );
        metrics.add_custom_metric("derive_macros", phase.stats.derive_macros.to_string());
        metrics.add_custom_metric(
            "differentiable_functions",
            phase.stats.differentiable_functions.to_string(),
        );
        metrics.add_custom_metric(
            "total_registry_items",
            phase.registry.all_meta_functions().len().to_string(),
        );
        metrics.add_custom_metric(
            "total_macros",
            phase.registry.all_macros().len().to_string(),
        );

        tracing::info!(
            "Meta registry phase complete in {:.2}ms: {} meta functions, \
             {} tagged literal handlers, {} interpolation handlers, {} derive macros",
            duration.as_secs_f64() * 1000.0,
            phase.stats.meta_functions,
            phase.stats.tagged_literal_handlers,
            phase.stats.interpolation_handlers,
            phase.stats.derive_macros,
        );

        Ok(PhaseOutput {
            data: input.data, // Pass through AST
            warnings: List::new(),
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        // Registry must be built sequentially to avoid race conditions
        false
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::FileId;
    use verum_ast::span::Span;
    use verum_common::List;

    fn create_empty_module() -> Module {
        Module {
            items: List::new(),
            attributes: List::new(),
            file_id: FileId::new(0),
            span: Span::default(),
        }
    }

    #[test]
    fn test_phase_creation() {
        let phase = MetaRegistryPhase::new();
        assert_eq!(phase.stats.meta_functions, 0);
        assert_eq!(phase.stats.tagged_literal_handlers, 0);
    }

    #[test]
    fn test_empty_module_registration() {
        let mut phase = MetaRegistryPhase::new();
        let module = create_empty_module();

        let result = phase.register_handlers(&[module]);
        assert!(result.is_ok());

        // Empty module should register no handlers
        assert_eq!(phase.stats.meta_functions, 0);
        assert_eq!(phase.stats.tagged_literal_handlers, 0);
        assert_eq!(phase.stats.interpolation_handlers, 0);
        assert_eq!(phase.stats.derive_macros, 0);
    }

    #[test]
    fn test_builtin_handlers_registered() {
        let phase = MetaRegistryPhase::new();

        // Built-in literal handlers should be registered
        let registry = phase.get_literal_registry();

        // Check for built-in tags
        assert!(registry.get_handler(&"d".into()).is_some());
        assert!(registry.get_handler(&"rx".into()).is_some());
        assert!(registry.get_handler(&"json".into()).is_some());
        assert!(registry.get_handler(&"url".into()).is_some());
    }

    #[test]
    fn test_stats_tracking() {
        let phase = MetaRegistryPhase::new();
        let stats = phase.stats();

        assert_eq!(stats.meta_functions, 0);
        assert_eq!(stats.tagged_literal_handlers, 0);
        assert_eq!(stats.interpolation_handlers, 0);
        assert_eq!(stats.derive_macros, 0);
        assert_eq!(stats.differentiable_functions, 0);
    }
}
