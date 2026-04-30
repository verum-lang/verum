//! Meta Context - Compile-time execution environment
//!
//! This module provides the MetaContext struct which maintains variable bindings,
//! type definitions, and protocol implementations during compile-time execution.
//!
//! ## Responsibility
//!
//! MetaContext is the central state container for meta-programming:
//! - Variable bindings (name -> MetaValue)
//! - Type definitions registry
//! - Protocol implementations registry
//! - Context subsystems (RuntimeInfo, BuildAssets, MacroState, etc.)
//! - Enabled contexts for builtin function access control
//!
//! ## Context Model
//!
//! The meta-system uses a unified context model where builtins are categorized into tiers:
//! - **Tier 0**: Always available (pure computation)
//! - **Tier 1**: Require explicit `using [...]` declaration (MetaTypes, MetaRuntime, CompileDiag, BuildAssets)
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_ast::{MetaValue, Span};
use verum_common::{List, Map, Maybe, Text};

use super::builtins::{EnabledContexts, RequiredContext};
use super::sandbox::AllowlistRegistry;
use super::{
    BenchResult, BuildAssetsInfo, CodeSearchTypeInfo, FunctionInfo, MacroStateInfo,
    MethodResolution, MethodSource, ModuleInfo, ProjectInfoData, ProtocolImplementation,
    RuntimeInfo, StageRecord, TypeDefinition, UsageInfo,
};

/// Type alias for backward compatibility.
/// MetaValue is the canonical type for compile-time values with AST support.
pub type ConstValue = MetaValue;

/// Execution context for meta functions
///
/// Maintains variable bindings and provides operations on constant values
/// during compile-time execution.
#[derive(Debug, Clone)]
pub struct MetaContext {
    /// Variable bindings (name -> value)
    pub(crate) bindings: Map<Text, ConstValue>,
    /// Type definitions for field/variant lookup
    /// Maps type name (e.g., "Point", "Color") to its definition
    pub(crate) type_definitions: Map<Text, TypeDefinition>,
    /// Protocol implementations registry
    /// Maps (type_name, protocol_name) -> implementation details
    pub(crate) protocol_implementations: Map<(Text, Text), ProtocolImplementation>,

    // ======== Subsystem Contexts ========
    /// Runtime information for MetaRuntime context
    pub runtime_info: RuntimeInfo,
    /// Build assets information for BuildAssets context
    pub build_assets: BuildAssetsInfo,
    /// Macro state information for MacroState context
    pub macro_state: MacroStateInfo,

    // ======== AstAccess Context Fields ========
    /// Span of the current macro invocation (call site)
    pub call_site_span: Span,
    /// Span of the macro definition (def site)
    pub def_site_span: Span,
    /// Source code map: file_id -> source text
    pub source_map: Map<u32, Text>,
    /// Source file path for parsing
    pub source_file: Option<Text>,
    /// Input token stream to the current macro
    pub macro_input: Option<Text>,
    /// Attribute arguments if invoked as attribute macro
    pub attr_args: Option<Text>,

    // ======== CompileDiag Context Fields ========
    /// Accumulated diagnostics
    pub diagnostics: List<verum_diagnostics::Diagnostic>,
    /// Count of errors emitted
    pub error_count: usize,
    /// Count of warnings emitted
    pub warning_count: usize,

    // ======== StageInfo Context Fields ========
    /// Current execution stage (0 = runtime, 1+ = compile-time)
    pub current_stage: u32,
    /// Maximum stage level in the compilation
    pub max_stage: u32,
    /// Current quote nesting depth
    pub quote_depth: u32,
    /// Function name -> stage level mapping
    pub function_stages: Map<Text, u32>,
    /// Whether staged metaprogramming is enabled
    pub staged_enabled: bool,
    /// Stage configuration key-value pairs
    pub stage_config: Map<Text, Text>,
    /// Iteration limit for meta function execution
    pub iteration_limit: u64,
    /// Recursion limit for meta function execution
    pub recursion_limit: u64,
    /// Current recursion depth (for detecting infinite recursion)
    pub current_recursion_depth: u64,
    /// Memory limit for meta function execution (bytes)
    pub memory_limit: u64,
    /// Timeout for meta function execution (milliseconds)
    pub timeout_ms: u64,
    /// Chain of generation records (for tracking code provenance)
    pub generation_chain: List<StageRecord>,
    /// Trace markers for debugging staged execution
    pub trace_markers: List<ConstValue>,
    /// Whether tracing is enabled for macro debugging
    pub trace_enabled: bool,
    /// Trace output buffer (accumulated trace messages)
    pub trace_output: List<Text>,
    /// Trace indent level for nested calls
    pub trace_indent: usize,
    /// Original source span that generated current code
    pub generation_origin: Option<Span>,
    /// Function name that generated current code
    pub generation_source_function: Option<Text>,
    /// Counter for generating unique identifiers
    pub unique_counter: u64,

    // ======== CodeSearch Context Fields ========
    /// Type registry for code search (name -> type info)
    pub type_registry: Map<Text, CodeSearchTypeInfo>,
    /// Function usage index (function name -> usages)
    pub usage_index: Map<Text, List<UsageInfo>>,
    /// Type usage index (type name -> usages)
    pub type_usage_index: Map<Text, List<UsageInfo>>,
    /// Constant usage index (const name -> usages)
    pub const_usage_index: Map<Text, List<UsageInfo>>,
    /// String literals in the code (literal text -> span)
    pub string_literals: List<(Text, Span)>,
    /// Module registry (module path -> module info)
    pub module_registry: Map<Text, ModuleInfo>,

    // ======== ProjectInfo Context Fields ========
    /// Project metadata
    pub project_info: ProjectInfoData,

    // ======== SourceMap Context Fields ========
    /// Stack of generated code scopes
    pub source_map_scope_stack: List<Text>,
    /// Span mappings (generated span -> generator function)
    pub span_mappings: List<(verum_common::span::LineColSpan, Text)>,
    /// Generated to source span mapping
    pub generated_to_source_map: Map<Text, verum_common::span::LineColSpan>,
    /// Line directives (file, line)
    pub line_directives: List<(Text, u32)>,
    /// Counter for synthetic span IDs
    pub next_synthetic_span_id: u64,

    // ======== MetaBench Context Fields ========
    /// Benchmark results (name -> list of results)
    pub bench_results: Map<Text, List<BenchResult>>,
    /// Memory usage reports (name -> bytes)
    pub memory_reports: Map<Text, u64>,
    /// Counters (name -> count)
    pub counters: Map<Text, u64>,
    /// Current memory usage (bytes)
    pub memory_used: u64,
    /// Peak memory usage (bytes)
    pub peak_memory: u64,

    // ======== Builtin Context Access Control ========
    /// Enabled contexts for builtin function access
    ///
    /// This tracks which contexts are available based on the meta function's
    /// `using [...]` declaration. Tier 0 builtins are always available,
    /// while Tier 1 builtins require explicit context declaration.
    ///
    /// Example:
    /// ```verum
    /// meta fn example() using [MetaTypes, CompileDiag] {
    ///     // Can call type_name(), compile_warning()
    /// }
    /// ```
    pub enabled_contexts: EnabledContexts,

    // ======== User Meta Function Support ========
    /// Registry for looking up user-defined meta functions
    ///
    /// When set, allows MetaExpr::Call to fall back to user-defined
    /// meta functions after checking builtins.
    pub registry: Option<std::sync::Arc<super::MetaRegistry>>,

    /// Current module path for resolving user meta function calls
    ///
    /// This is used to resolve unqualified function names to their
    /// full module path when looking up user-defined meta functions.
    pub current_module: Text,

    // ======== Sandbox/Security ========
    /// Allowlist registry for checking forbidden operations
    ///
    /// This tracks which operations are allowed or forbidden in meta functions.
    /// Forbidden operations include file I/O, network, process spawning, etc.
    pub allowlist: AllowlistRegistry,

    // ======== Hygiene Mode ========
    /// Whether the current meta function is marked @transparent
    ///
    /// When true, hygienic renaming is disabled and bare identifiers
    /// in quote blocks will capture from the expansion site. This enables
    /// intentional capture but also allows accidental capture (M402).
    pub is_transparent: bool,

    // ======== Sandbox Gates ========
    /// `[meta] reflection = false` — hard-disable the reflection
    /// surface (`RequiredContext::MetaTypes`, `CompileDiag`)
    /// regardless of any function-level `using [...]` declaration.
    ///
    /// When `true`, `get_builtin` rejects calls to reflection-tagged
    /// builtins with `MetaError::MissingContext` even if the function
    /// declared `using [MetaTypes]` — the user-supplied capability
    /// is overridden by the global gate. This is the security
    /// stance: a language-level sandbox that wants to forbid
    /// reflection cannot be circumvented by individual function
    /// declarations.
    ///
    /// Default `false` (reflection allowed). Wired through
    /// `MacroExpansionPhase::with_reflection_enabled(false)`.
    pub reflection_disabled: bool,
}

impl Default for MetaContext {
    fn default() -> Self {
        Self::new()
    }
}

impl MetaContext {
    /// Create a new empty meta context
    pub fn new() -> Self {
        Self {
            bindings: Map::new(),
            type_definitions: Map::new(),
            protocol_implementations: Map::new(),
            runtime_info: RuntimeInfo::default(),
            build_assets: BuildAssetsInfo::default(),
            macro_state: MacroStateInfo::default(),
            call_site_span: Span::dummy(),
            def_site_span: Span::dummy(),
            source_map: Map::new(),
            diagnostics: List::new(),
            error_count: 0,
            warning_count: 0,
            current_stage: 1, // Default to compile-time
            max_stage: 1,
            quote_depth: 0,
            function_stages: Map::new(),
            staged_enabled: true,
            stage_config: Map::new(),
            iteration_limit: 1_000_000,
            recursion_limit: 50,  // Lower limit to prevent stack overflow in debug mode
            current_recursion_depth: 0,
            memory_limit: 100 * 1024 * 1024, // 100 MB
            timeout_ms: 30_000,              // 30 seconds
            generation_chain: List::new(),
            trace_markers: List::new(),
            trace_enabled: false,
            trace_output: List::new(),
            trace_indent: 0,
            generation_origin: None,
            generation_source_function: None,
            unique_counter: 0,
            type_registry: Map::new(),
            usage_index: Map::new(),
            type_usage_index: Map::new(),
            const_usage_index: Map::new(),
            string_literals: List::new(),
            module_registry: Map::new(),
            project_info: ProjectInfoData::default(),
            source_map_scope_stack: List::new(),
            span_mappings: List::new(),
            generated_to_source_map: Map::new(),
            line_directives: List::new(),
            next_synthetic_span_id: 0,
            source_file: None,
            macro_input: None,
            attr_args: None,
            bench_results: Map::new(),
            memory_reports: Map::new(),
            counters: Map::new(),
            memory_used: 0,
            peak_memory: 0,
            enabled_contexts: EnabledContexts::new(),
            registry: None,
            current_module: Text::from(""),
            allowlist: AllowlistRegistry::new(),
            is_transparent: false,
            reflection_disabled: false,
        }
    }

    /// Create a context with all contexts enabled
    ///
    /// This is useful for testing or for backward compatibility during migration.
    /// In production code, prefer explicit context declaration.
    pub fn with_all_contexts() -> Self {
        let mut ctx = Self::new();
        ctx.enabled_contexts = EnabledContexts::all();
        ctx
    }

    /// Create a context with specific contexts enabled
    ///
    /// This is typically used when executing a meta function with a
    /// `using [...]` clause.
    pub fn with_contexts(contexts: &[RequiredContext]) -> Self {
        let mut ctx = Self::new();
        for context in contexts {
            ctx.enabled_contexts.enable(*context);
        }
        ctx
    }

    /// Create a context from a `using [...]` clause
    ///
    /// Parses context names from the using clause and enables them.
    pub fn with_using_clause(names: &[Text]) -> Self {
        let mut ctx = Self::new();
        ctx.enabled_contexts = EnabledContexts::from_using_clause(names);
        ctx
    }

    /// Mount the resource limits and enabled-contexts surface from a
    /// `SecurityContext` onto this `MetaContext`.
    ///
    /// `SecurityContext` is the user-facing API for capping meta
    /// execution (recursion depth, iteration count, memory ceiling,
    /// timeout). Until this method existed, `SecurityContext::set_limits`
    /// landed on a free-standing `ResourceLimits` value that nothing in
    /// the evaluator or sandbox ever consulted — a silent no-op exposed
    /// only by a `tracing::debug!` warning. This method closes that gap
    /// by copying the four limits + the enabled-contexts mask onto the
    /// fields the evaluator actually checks:
    ///
    ///   * `recursion_limit`  → `MetaContext::execute_user_meta_fn`
    ///                          gate at `evaluator.rs:2237`
    ///   * `iteration_limit`  → loop counters in the evaluator
    ///   * `memory_limit`     → sandbox `ResourceLimiter` (mirrored)
    ///   * `timeout_ms`       → sandbox deadline
    ///   * `enabled_contexts` → required-context dispatch in builtins
    ///
    /// Embedders that build a meta context from a security policy
    /// should now write:
    ///
    /// ```ignore
    /// let mut sec = SecurityContext::new();
    /// sec.set_recursion_limit(256);
    /// let mut meta = MetaContext::new();
    /// meta.apply_security_context(&sec);
    /// ```
    ///
    /// Or use the `from_security_context` builder for one-shot construction.
    pub fn apply_security_context(
        &mut self,
        sec: &crate::meta::contexts::SecurityContext,
    ) {
        let limits = sec.limits();
        self.recursion_limit = limits.recursion_limit;
        self.iteration_limit = limits.iteration_limit;
        self.memory_limit = limits.memory_limit;
        self.timeout_ms = limits.timeout_ms;
        self.enabled_contexts = sec.enabled_contexts().clone();
    }

    /// Build a fully-configured `MetaContext` directly from a
    /// `SecurityContext`. Equivalent to `MetaContext::new()` followed
    /// by `apply_security_context(sec)`.
    pub fn from_security_context(
        sec: &crate::meta::contexts::SecurityContext,
    ) -> Self {
        let mut ctx = Self::new();
        ctx.apply_security_context(sec);
        ctx
    }

    // ======== Builder Methods ========

    /// Create context with runtime info
    #[inline]
    pub fn with_runtime_info(runtime_info: RuntimeInfo) -> Self {
        let mut ctx = Self::new();
        ctx.runtime_info = runtime_info;
        ctx
    }

    /// Create context with build assets
    #[inline]
    pub fn with_build_assets(build_assets: BuildAssetsInfo) -> Self {
        let mut ctx = Self::new();
        ctx.build_assets = build_assets;
        ctx
    }

    /// Create context with macro state
    #[inline]
    pub fn with_macro_state(macro_state: MacroStateInfo) -> Self {
        let mut ctx = Self::new();
        ctx.macro_state = macro_state;
        ctx
    }

    /// Set the meta registry for user function lookup
    ///
    /// When set, allows `MetaExpr::Call` to resolve user-defined meta functions
    /// after checking builtins.
    #[inline]
    pub fn set_registry(&mut self, registry: std::sync::Arc<super::MetaRegistry>) {
        self.registry = Some(registry);
    }

    /// Set the current module path for user function resolution
    ///
    /// This determines which module's user functions are checked when
    /// resolving unqualified function calls.
    #[inline]
    pub fn set_current_module(&mut self, module: Text) {
        self.current_module = module;
    }

    /// Builder: Set the meta registry for user function lookup
    #[inline]
    pub fn with_registry(mut self, registry: std::sync::Arc<super::MetaRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Builder: Set the current module path
    #[inline]
    pub fn with_current_module(mut self, module: Text) -> Self {
        self.current_module = module;
        self
    }

    /// Builder: Replace the entire project info block.
    ///
    /// The pipeline driver typically constructs a `ProjectInfoData`
    /// with captured `git_revision` + `build_time_unix_ms`
    /// (`with_captured_version_stamp`) and attaches it via this
    /// method so the `@version_stamp` family of meta builtins
    /// (#20 / P7) returns real data instead of the deterministic
    /// fallback (empty SHA + zero timestamp).
    #[inline]
    pub fn with_project_info(mut self, project_info: ProjectInfoData) -> Self {
        self.project_info = project_info;
        self
    }

    /// Builder: Capture git revision + build-time stamp from the
    /// surrounding environment in-place (#20 / P7). Calls
    /// [`ProjectInfoData::capture_version_stamp_in_place`] on the
    /// existing `project_info`. Idempotent — safe to call before
    /// or after [`Self::with_project_info`]; both fields are
    /// `Option`-typed and overwritten unconditionally.
    #[inline]
    pub fn with_captured_version_stamp(mut self) -> Self {
        self.project_info.capture_version_stamp_in_place();
        self
    }

    // ======== Subsystem Setters ========

    /// Set runtime info
    #[inline]
    pub fn set_runtime_info(&mut self, runtime_info: RuntimeInfo) {
        self.runtime_info = runtime_info;
    }

    /// Set build assets
    #[inline]
    pub fn set_build_assets(&mut self, build_assets: BuildAssetsInfo) {
        self.build_assets = build_assets;
    }

    /// Set macro state
    #[inline]
    pub fn set_macro_state(&mut self, macro_state: MacroStateInfo) {
        self.macro_state = macro_state;
    }

    /// Enter a macro execution scope
    #[inline]
    pub fn enter_macro(&mut self, name: Text) {
        self.macro_state.enter_macro(name);
    }

    /// Exit the current macro execution scope
    #[inline]
    pub fn exit_macro(&mut self) {
        self.macro_state.exit_macro();
    }

    // ======== Sandbox Operations ========

    /// Check if a function is a forbidden I/O operation
    ///
    /// This checks the allowlist registry to determine if the function
    /// is a filesystem, network, process, or other forbidden operation.
    #[inline]
    pub fn is_forbidden_function(&self, name: &Text) -> bool {
        self.allowlist.is_forbidden_io_function(name)
    }

    /// Get the category of a forbidden function for error reporting
    pub fn get_forbidden_category(&self, name: &Text) -> Option<&'static str> {
        if self.allowlist.is_filesystem_function(name) {
            Some("filesystem")
        } else if self.allowlist.is_network_function(name) {
            Some("network")
        } else if self.allowlist.is_process_function(name) {
            Some("process")
        } else if self.allowlist.is_time_function(name) {
            Some("time")
        } else if self.allowlist.is_env_function(name) {
            Some("environment")
        } else if self.allowlist.is_random_function(name) {
            Some("random")
        } else if self.allowlist.is_unsafe_function(name) {
            Some("unsafe")
        } else if self.allowlist.is_ffi_function(name) {
            Some("ffi")
        } else {
            None
        }
    }

    // ======== Binding Operations ========

    /// Bind a variable to a value
    #[inline]
    pub fn bind(&mut self, name: Text, value: ConstValue) {
        self.bindings.insert(name, value);
    }

    /// Get a variable's value
    #[inline]
    pub fn get(&self, name: &Text) -> Option<ConstValue> {
        self.bindings.get(name).cloned()
    }

    /// Check if a variable is bound
    #[inline]
    pub fn has(&self, name: &Text) -> bool {
        self.bindings.contains_key(name)
    }

    /// Remove a binding
    #[inline]
    pub fn unbind(&mut self, name: &Text) -> Maybe<ConstValue> {
        self.bindings.remove(name).into()
    }

    /// Clear all bindings
    #[inline]
    pub fn clear(&mut self) {
        self.bindings.clear();
    }

    /// Get all binding names
    pub fn binding_names(&self) -> List<Text> {
        self.bindings.keys().cloned().collect()
    }

    // ======== Type Definition Operations ========

    /// Register a struct type
    pub fn register_struct(&mut self, name: Text, fields: List<(Text, verum_ast::ty::Type)>) {
        self.type_definitions
            .insert(name.clone(), TypeDefinition::simple_struct(name, fields));
    }

    /// Register an enum type
    pub fn register_enum(&mut self, name: Text, variants: List<(Text, verum_ast::ty::Type)>) {
        self.type_definitions
            .insert(name.clone(), TypeDefinition::simple_enum(name, variants));
    }

    /// Register a protocol type
    pub fn register_protocol(&mut self, name: Text, methods: List<Text>) {
        self.type_definitions
            .insert(name.clone(), TypeDefinition::simple_protocol(name, methods));
    }

    /// Register a type definition with full metadata
    pub fn register_type_definition(&mut self, type_def: TypeDefinition) {
        self.type_definitions
            .insert(type_def.name().clone(), type_def);
    }

    /// Get a type definition
    #[inline]
    pub fn get_type_definition(&self, name: &Text) -> Option<&TypeDefinition> {
        self.type_definitions.get(name)
    }

    /// Get struct fields
    pub fn get_struct_fields(&self, name: &Text) -> Option<&List<(Text, verum_ast::ty::Type)>> {
        match self.type_definitions.get(name) {
            Some(TypeDefinition::Struct { fields, .. }) => Some(fields),
            _ => None,
        }
    }

    /// Get enum variants
    pub fn get_enum_variants(&self, name: &Text) -> Option<&List<(Text, verum_ast::ty::Type)>> {
        match self.type_definitions.get(name) {
            Some(TypeDefinition::Enum { variants, .. }) => Some(variants),
            _ => None,
        }
    }

    /// Get type functions (methods defined in impl blocks)
    pub fn get_type_functions(&self, type_name: &Text) -> List<FunctionInfo> {
        // Look up implementations for this type
        let mut functions = List::new();
        for ((ty, _protocol), impl_def) in &self.protocol_implementations {
            if ty == type_name {
                for method in &impl_def.implemented_methods {
                    functions.push(FunctionInfo::new(method.clone(), Text::from("()")));
                }
            }
        }
        functions
    }

    /// Resolve a method on a type
    pub fn resolve_method(&self, type_name: &Text, method_name: &Text) -> Option<MethodResolution> {
        // First check inherent methods
        for ((ty, protocol), impl_def) in &self.protocol_implementations {
            if ty == type_name {
                for method in &impl_def.implemented_methods {
                    if method == method_name {
                        return Some(MethodResolution {
                            function: FunctionInfo::new(method.clone(), Text::from("()")),
                            source: MethodSource::Inherent,
                            providing_protocol: if protocol.is_empty() {
                                Maybe::None
                            } else {
                                Maybe::Some(protocol.clone())
                            },
                            is_default_impl: false,
                        });
                    }
                }
            }
        }
        None
    }

    /// Get type attributes
    ///
    /// Returns a list of attribute names for the given type.
    pub fn get_type_attributes(&self, type_name: &Text) -> List<Text> {
        if let Some(type_def) = self.type_definitions.get(type_name) {
            type_def
                .attributes()
                .iter()
                .map(|a| a.name.clone())
                .collect()
        } else {
            List::new()
        }
    }

    /// Check if type has attribute
    pub fn type_has_attribute(&self, type_name: &Text, attr_name: &Text) -> bool {
        if let Some(type_def) = self.type_definitions.get(type_name) {
            type_def.has_attribute(attr_name.as_str())
        } else {
            false
        }
    }

    /// Get type attribute value
    ///
    /// Returns the value associated with an attribute, if any.
    pub fn get_type_attribute(&self, type_name: &Text, attr_name: &Text) -> Option<ConstValue> {
        if let Some(type_def) = self.type_definitions.get(type_name) {
            type_def
                .get_attribute(attr_name.as_str())
                .and_then(|attr| attr.value.as_ref().cloned())
        } else {
            None
        }
    }

    /// Get type documentation
    ///
    /// Returns the documentation string for the given type.
    pub fn get_type_doc(&self, type_name: &Text) -> Maybe<Text> {
        if let Some(type_def) = self.type_definitions.get(type_name) {
            type_def.doc().cloned()
        } else {
            Maybe::None
        }
    }

    /// Get associated types
    ///
    /// Returns the associated types defined on the given type.
    pub fn get_associated_types(&self, type_name: &Text) -> List<(Text, verum_ast::ty::Type)> {
        if let Some(type_def) = self.type_definitions.get(type_name) {
            type_def.associated_types()
        } else {
            List::new()
        }
    }

    /// Get super types (protocols this type implements)
    ///
    /// Returns the list of protocols/traits this type implements or extends.
    pub fn get_super_types(&self, type_name: &Text) -> List<Text> {
        if let Some(type_def) = self.type_definitions.get(type_name) {
            type_def.super_types()
        } else {
            List::new()
        }
    }

    /// Get methods defined on a type
    ///
    /// Returns the method signatures for the given type.
    pub fn get_type_methods(&self, type_name: &Text) -> List<Text> {
        if let Some(type_def) = self.type_definitions.get(type_name) {
            type_def
                .get_methods()
                .iter()
                .map(|m| m.name.clone())
                .collect()
        } else {
            List::new()
        }
    }

    // ======== Protocol Implementation Operations ========

    /// Register a protocol implementation
    pub fn register_protocol_implementation(
        &mut self,
        type_name: Text,
        protocol_name: Text,
        methods: List<Text>,
    ) {
        self.protocol_implementations.insert(
            (type_name.clone(), protocol_name.clone()),
            ProtocolImplementation {
                implementing_type: type_name,
                protocol_name,
                implemented_methods: methods,
            },
        );
    }

    /// Get protocols implemented by a type
    pub fn get_implemented_protocols(&self, type_name: &Text) -> List<Text> {
        self.protocol_implementations
            .keys()
            .filter(|(ty, _)| ty == type_name)
            .map(|(_, proto)| proto.clone())
            .collect()
    }

    /// Get types that implement a protocol
    pub fn get_implementors(&self, protocol_name: &Text) -> List<Text> {
        self.protocol_implementations
            .keys()
            .filter(|(_, proto)| proto == protocol_name)
            .map(|(ty, _)| ty.clone())
            .collect()
    }

    /// Get protocol implementation details
    pub fn get_protocol_implementation(
        &self,
        type_name: &Text,
        protocol_name: &Text,
    ) -> Option<&ProtocolImplementation> {
        self.protocol_implementations
            .get(&(type_name.clone(), protocol_name.clone()))
    }

    /// Check if type implements protocol
    pub fn type_implements_protocol(&self, type_name: &Text, protocol_name: &Text) -> bool {
        self.protocol_implementations
            .contains_key(&(type_name.clone(), protocol_name.clone()))
    }

    // ======== Clear Operations ========

    /// Clear all type definitions
    #[inline]
    pub fn clear_type_definitions(&mut self) {
        self.type_definitions.clear();
    }

    /// Clear all protocol implementations
    #[inline]
    pub fn clear_protocol_implementations(&mut self) {
        self.protocol_implementations.clear();
    }

    /// Clear all state
    pub fn clear_all(&mut self) {
        self.bindings.clear();
        self.type_definitions.clear();
        self.protocol_implementations.clear();
    }

    // ======== Unique ID Generation ========

    /// Generate a unique identifier
    #[inline]
    pub fn gen_unique_id(&mut self) -> u64 {
        let id = self.unique_counter;
        self.unique_counter += 1;
        id
    }

    /// Generate a unique identifier with prefix
    pub fn gen_unique_ident(&mut self, prefix: &str) -> Text {
        let id = self.gen_unique_id();
        Text::from(format!("{}_{}", prefix, id))
    }

    // ======== Trace Operations (@meta_trace) ========

    /// Enable tracing for macro debugging
    #[inline]
    pub fn trace_on(&mut self) {
        self.trace_enabled = true;
    }

    /// Disable tracing for macro debugging
    #[inline]
    pub fn trace_off(&mut self) {
        self.trace_enabled = false;
    }

    /// Check if tracing is enabled
    #[inline]
    pub fn is_tracing(&self) -> bool {
        self.trace_enabled
    }

    /// Log a trace message with current indentation
    pub fn trace_log(&mut self, message: Text) {
        if self.trace_enabled {
            let indent = "  ".repeat(self.trace_indent);
            self.trace_output
                .push(Text::from(format!("{}{}", indent, message)));
        }
    }

    /// Log entry into a function/macro
    pub fn trace_enter(&mut self, name: &Text) {
        if self.trace_enabled {
            self.trace_log(Text::from(format!("-> {}", name)));
            self.trace_indent += 1;
        }
    }

    /// Log exit from a function/macro with result
    pub fn trace_exit(&mut self, name: &Text, result: &ConstValue) {
        if self.trace_enabled {
            if self.trace_indent > 0 {
                self.trace_indent -= 1;
            }
            self.trace_log(Text::from(format!("<- {} = {:?}", name, result)));
        }
    }

    /// Log exit from a function/macro with error
    pub fn trace_error(&mut self, name: &Text, error: &str) {
        if self.trace_enabled {
            if self.trace_indent > 0 {
                self.trace_indent -= 1;
            }
            self.trace_log(Text::from(format!("<- {} ERROR: {}", name, error)));
        }
    }

    /// Get the accumulated trace output
    pub fn get_trace_output(&self) -> List<Text> {
        self.trace_output.clone()
    }

    /// Clear the trace output buffer
    pub fn clear_trace_output(&mut self) {
        self.trace_output.clear();
        self.trace_indent = 0;
    }

    /// Dump trace output to a single string
    pub fn dump_trace(&self) -> Text {
        let lines: Vec<String> = self.trace_output.iter().map(|t| t.to_string()).collect();
        Text::from(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_context() {
        let ctx = MetaContext::new();
        assert!(ctx.bindings.is_empty());
        assert!(ctx.type_definitions.is_empty());
    }

    #[test]
    fn apply_security_context_propagates_all_four_limits() {
        // S6 invariant: SecurityContext::set_* must actually flow through
        // to the MetaContext fields the evaluator + sandbox check.
        // Before the fix, set_limits was a silent no-op exposed only via
        // a tracing::debug!.
        use crate::meta::contexts::SecurityContext;
        use crate::meta::contexts::ResourceLimits;

        let mut sec = SecurityContext::new();
        sec.set_limits(ResourceLimits {
            iteration_limit: 7_777,
            recursion_limit: 13,
            memory_limit:    42 * 1024 * 1024,
            timeout_ms:      9_999,
        });

        let mut meta = MetaContext::new();
        // Confirm defaults differ from the values we set, otherwise the
        // test could silently pass on a reverted plumbing.
        assert_ne!(meta.recursion_limit, 13);
        assert_ne!(meta.iteration_limit, 7_777);

        meta.apply_security_context(&sec);

        assert_eq!(meta.iteration_limit, 7_777);
        assert_eq!(meta.recursion_limit, 13);
        assert_eq!(meta.memory_limit, 42 * 1024 * 1024);
        assert_eq!(meta.timeout_ms, 9_999);
    }

    #[test]
    fn from_security_context_one_shot_construction() {
        use crate::meta::contexts::SecurityContext;
        use crate::meta::contexts::ResourceLimits;

        let mut sec = SecurityContext::new();
        sec.set_limits(ResourceLimits {
            iteration_limit: 100,
            recursion_limit: 5,
            memory_limit:    1_024,
            timeout_ms:      250,
        });

        let meta = MetaContext::from_security_context(&sec);

        assert_eq!(meta.recursion_limit, 5);
        assert_eq!(meta.iteration_limit, 100);
        assert_eq!(meta.memory_limit, 1_024);
        assert_eq!(meta.timeout_ms, 250);
    }

    #[test]
    fn test_bind_and_get() {
        let mut ctx = MetaContext::new();
        ctx.bind(Text::from("x"), ConstValue::Int(42));
        assert_eq!(ctx.get(&Text::from("x")), Some(ConstValue::Int(42)));
        assert!(ctx.has(&Text::from("x")));
        assert!(!ctx.has(&Text::from("y")));
    }

    #[test]
    fn test_unbind() {
        let mut ctx = MetaContext::new();
        ctx.bind(Text::from("x"), ConstValue::Int(42));
        let removed = ctx.unbind(&Text::from("x"));
        assert_eq!(removed, Maybe::Some(ConstValue::Int(42)));
        assert!(!ctx.has(&Text::from("x")));
    }

    #[test]
    fn test_register_struct() {
        let mut ctx = MetaContext::new();
        ctx.register_struct(
            Text::from("Point"),
            List::from(vec![
                (Text::from("x"), verum_ast::ty::Type::int(Span::dummy())),
                (Text::from("y"), verum_ast::ty::Type::int(Span::dummy())),
            ]),
        );
        assert!(ctx.get_type_definition(&Text::from("Point")).is_some());
        assert!(ctx.get_struct_fields(&Text::from("Point")).is_some());
    }

    #[test]
    fn test_protocol_implementation() {
        let mut ctx = MetaContext::new();
        ctx.register_protocol_implementation(
            Text::from("Point"),
            Text::from("Debug"),
            List::from(vec![Text::from("debug")]),
        );
        assert!(ctx.type_implements_protocol(&Text::from("Point"), &Text::from("Debug")));
        assert!(!ctx.type_implements_protocol(&Text::from("Point"), &Text::from("Clone")));
    }

    #[test]
    fn test_unique_id_generation() {
        let mut ctx = MetaContext::new();
        let id1 = ctx.gen_unique_id();
        let id2 = ctx.gen_unique_id();
        assert_ne!(id1, id2);
    }
}
