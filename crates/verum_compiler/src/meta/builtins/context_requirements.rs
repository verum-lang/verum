//! Context Requirements for Meta Builtins
//!
//! This module defines the unified context model for meta-system builtins.
//! All meta functions are categorized into tiers based on their context requirements.
//!
//! ## Tier Model
//!
//! ### Tier 0: Core Primitives (Always Available)
//!
//! Pure functions that work on values without accessing any external state:
//! - Arithmetic: `abs`, `min`, `max`
//! - Type conversions: `int_to_text`, `text_to_int`
//! - Collection operations: `list_len`, `list_get`, `list_push`, `list_concat`, etc.
//! - Text operations: `text_len`, `text_concat`, `text_split`, etc.
//! - Quote/Unquote: `quote`, `unquote`, `stringify`
//! - Identity operations: `concat_idents`, `format_ident`
//!
//! ### Tier 1: Capability-Gated Functions (Require Context)
//!
//! Functions that access compiler state, build configuration, or have side effects:
//!
//! | Context | Functions | Purpose |
//! |---------|-----------|---------|
//! | MetaTypes | `type_name`, `fields_of`, `variants_of`, `is_struct`, `implements`, `size_of`, `align_of` | Type registry access |
//! | MetaRuntime | `target_os`, `target_arch`, `env`, `has_feature` | Build/platform info |
//! | CompileDiag | `compile_error`, `compile_warning` | Compiler diagnostics |
//! | BuildAssets | `load_text`, `include_bytes` | File system access |
//!
//! ## Design Rationale
//!
//! The previous design had a "double standard" where:
//! - Context system required explicit `using [...]` declarations
//! - Builtins were implicitly available to all meta functions
//!
//! This unified model ensures:
//! 1. **Consistency**: All capabilities require explicit declaration
//! 2. **Security**: Can sandbox meta functions with restricted reflection
//! 3. **Testability**: Can mock contexts for unit testing
//! 4. **Composability**: Build higher-level contexts from lower-level ones
//!
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use std::collections::HashSet;
use std::fmt;

use verum_common::{Map, Text};

use super::BuiltinMetaFn;

/// Context required for a builtin function
///
/// Each builtin is categorized into one of these contexts based on
/// what external state it accesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequiredContext {
    /// Tier 0: Always available - pure computation
    ///
    /// These functions operate only on their input values without
    /// accessing any external state. Examples:
    /// - `abs(-5)` → `5`
    /// - `text_concat("a", "b")` → `"ab"`
    /// - `quote(expr)` → AST node
    None,

    /// Tier 1: Type reflection access
    ///
    /// Functions that access the type registry to introspect types.
    /// Required for: `type_name`, `fields_of`, `variants_of`, `is_struct`,
    /// `is_enum`, `implements`, `size_of`, `align_of`, `stride_of`, etc.
    MetaTypes,

    /// Tier 1: Build/platform information access
    ///
    /// Functions that access compile-time build configuration.
    /// Required for: `target_os`, `target_arch`, `env`, `has_feature`,
    /// `compiler_version`, `is_debug`, `opt_level`, etc.
    MetaRuntime,

    /// Tier 1: Compiler diagnostics interaction
    ///
    /// Functions that emit compile-time diagnostics.
    /// Required for: `compile_error`, `compile_warning`, `compile_note`
    CompileDiag,

    /// Tier 1: File system access for build assets
    ///
    /// Functions that read files at compile time.
    /// Required for: `load_text`, `include_bytes`, `include_str`
    BuildAssets,

    /// Tier 1: Source map context for generated code tracking
    ///
    /// Functions that manage source map scopes and span mappings.
    /// Required for: `source_map_enter_generated`, `source_map_exit_generated`,
    /// `source_map_current_scope`, `source_map_scope_path`, `source_map_map_span`,
    /// `source_map_get_source_span`, `source_map_synthetic_span`, `source_map_get_mappings`
    SourceMap,

    /// Tier 1: Project information context
    ///
    /// Functions that access project metadata from Verum.toml.
    /// Required for: `project_package_name`, `project_package_version`,
    /// `project_dependencies`, `project_target_os`, `project_is_debug`, etc.
    ProjectInfo,

    /// Tier 1: Meta benchmarking context
    ///
    /// Functions that measure compile-time performance of meta functions.
    /// Required for: `bench_start`, `bench_now_ns`, `bench_report`,
    /// `bench_memory_usage`, `bench_count`, `bench_all_results`, etc.
    MetaBench,

    /// Tier 1: Stage information context
    ///
    /// Functions that query and manage N-level staged metaprogramming.
    /// Required for: `stage_current`, `stage_max`, `stage_is_runtime`,
    /// `stage_is_compile_time`, `stage_quote_target`, `stage_unique_ident`,
    /// `stage_function_stage`, `stage_functions_at`, `stage_generation_chain`,
    /// `stage_trace_marker`, etc.
    StageInfo,
}

impl RequiredContext {
    /// Get the context name as it appears in `using [...]` clause
    pub fn context_name(&self) -> &'static str {
        match self {
            RequiredContext::None => "MetaCore",  // Implicit, always available
            RequiredContext::MetaTypes => "MetaTypes",
            RequiredContext::MetaRuntime => "MetaRuntime",
            RequiredContext::CompileDiag => "CompileDiag",
            RequiredContext::BuildAssets => "BuildAssets",
            RequiredContext::SourceMap => "SourceMap",
            RequiredContext::ProjectInfo => "ProjectInfo",
            RequiredContext::MetaBench => "MetaBench",
            RequiredContext::StageInfo => "StageInfo",
        }
    }

    /// Get the tier level (0 = always available, 1 = requires context)
    pub fn tier(&self) -> u8 {
        match self {
            RequiredContext::None => 0,
            _ => 1,
        }
    }

    /// Check if this context requires explicit declaration
    pub fn requires_declaration(&self) -> bool {
        !matches!(self, RequiredContext::None)
    }
}

impl fmt::Display for RequiredContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.context_name())
    }
}

/// Information about a builtin function including its context requirements
#[derive(Clone)]
pub struct BuiltinInfo {
    /// The builtin function implementation
    pub function: BuiltinMetaFn,

    /// The context required to call this function
    pub required_context: RequiredContext,

    /// Brief description for documentation
    pub description: &'static str,

    /// Function signature for documentation (e.g., "(T) -> Text")
    pub signature: &'static str,
}

impl BuiltinInfo {
    /// Create a new Tier 0 (always available) builtin
    pub const fn tier0(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::None,
            description,
            signature,
        }
    }

    /// Create a new Tier 1 builtin requiring MetaTypes context
    pub const fn meta_types(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::MetaTypes,
            description,
            signature,
        }
    }

    /// Create a new Tier 1 builtin requiring MetaRuntime context
    pub const fn meta_runtime(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::MetaRuntime,
            description,
            signature,
        }
    }

    /// Create a new Tier 1 builtin requiring CompileDiag context
    pub const fn compile_diag(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::CompileDiag,
            description,
            signature,
        }
    }

    /// Create a new Tier 1 builtin requiring BuildAssets context
    pub const fn build_assets(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::BuildAssets,
            description,
            signature,
        }
    }

    /// Create a new Tier 1 builtin requiring SourceMap context
    pub const fn source_map(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::SourceMap,
            description,
            signature,
        }
    }

    /// Create a new Tier 1 builtin requiring ProjectInfo context
    pub const fn project_info(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::ProjectInfo,
            description,
            signature,
        }
    }

    /// Create a new Tier 1 builtin requiring MetaBench context
    pub const fn meta_bench(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::MetaBench,
            description,
            signature,
        }
    }

    /// Create a new Tier 1 builtin requiring StageInfo context
    pub const fn stage_info(
        function: BuiltinMetaFn,
        description: &'static str,
        signature: &'static str,
    ) -> Self {
        Self {
            function,
            required_context: RequiredContext::StageInfo,
            description,
            signature,
        }
    }
}

impl fmt::Debug for BuiltinInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuiltinInfo")
            .field("required_context", &self.required_context)
            .field("description", &self.description)
            .field("signature", &self.signature)
            .finish()
    }
}

/// Set of enabled contexts for a meta function execution
///
/// This tracks which contexts are available based on the function's
/// `using [...]` declaration.
#[derive(Debug, Clone, Default)]
pub struct EnabledContexts {
    contexts: HashSet<RequiredContext>,
}

impl EnabledContexts {
    /// Create a new empty context set (only Tier 0 builtins available)
    pub fn new() -> Self {
        Self {
            contexts: HashSet::new(),
        }
    }

    /// Create a context set with all contexts enabled (for backward compatibility during migration)
    pub fn all() -> Self {
        let mut contexts = HashSet::new();
        contexts.insert(RequiredContext::None);
        contexts.insert(RequiredContext::MetaTypes);
        contexts.insert(RequiredContext::MetaRuntime);
        contexts.insert(RequiredContext::CompileDiag);
        contexts.insert(RequiredContext::BuildAssets);
        contexts.insert(RequiredContext::SourceMap);
        contexts.insert(RequiredContext::ProjectInfo);
        contexts.insert(RequiredContext::MetaBench);
        contexts.insert(RequiredContext::StageInfo);
        Self { contexts }
    }

    /// Enable a specific context
    pub fn enable(&mut self, context: RequiredContext) {
        self.contexts.insert(context);
    }

    /// Enable multiple contexts
    pub fn enable_all(&mut self, contexts: impl IntoIterator<Item = RequiredContext>) {
        self.contexts.extend(contexts);
    }

    /// Check if a context is enabled
    pub fn is_enabled(&self, context: RequiredContext) -> bool {
        // Tier 0 is always enabled
        context == RequiredContext::None || self.contexts.contains(&context)
    }

    /// Check if a builtin can be called with current contexts
    pub fn can_call(&self, builtin: &BuiltinInfo) -> bool {
        self.is_enabled(builtin.required_context)
    }

    /// Get all enabled contexts
    pub fn enabled(&self) -> impl Iterator<Item = RequiredContext> + '_ {
        self.contexts.iter().copied()
    }

    /// Parse contexts from `using [...]` clause identifiers
    ///
    /// This is a convenience method that ignores unknown contexts for backward
    /// compatibility. For better error handling, use `parse_using_clause` instead.
    pub fn from_using_clause(names: &[Text]) -> Self {
        Self::parse_using_clause(names).enabled_contexts
    }

    /// Parse contexts from `using [...]` clause with full error reporting
    ///
    /// Unlike `from_using_clause`, this method:
    /// - Reports warnings for possible typos of standard context names
    /// - Tracks user-defined contexts separately
    /// - Detects duplicate context declarations
    ///
    /// # Example
    ///
    /// ```ignore
    /// let result = EnabledContexts::parse_using_clause(&[
    ///     Text::from("MetaTypes"),
    ///     Text::from("metatype"),  // Typo - will generate warning
    ///     Text::from("MyCustomContext"),  // User-defined - will be tracked
    /// ]);
    /// assert!(result.enabled_contexts.is_enabled(RequiredContext::MetaTypes));
    /// assert_eq!(result.warnings.len(), 1);  // Warning for "metatype"
    /// assert!(result.user_contexts.contains(&Text::from("MyCustomContext")));
    /// ```
    pub fn parse_using_clause(names: &[Text]) -> ParsedUsingClause {
        let mut result = ParsedUsingClause::default();
        let mut seen_contexts: HashSet<Text> = HashSet::new();

        for name in names {
            // Check for duplicates first
            if seen_contexts.contains(name) {
                result.duplicates.push(DuplicateContextError { name: name.clone() });
                continue;  // Don't process duplicate again
            }
            seen_contexts.insert(name.clone());

            match name.as_str() {
                "MetaTypes" => result.enabled_contexts.enable(RequiredContext::MetaTypes),
                "MetaRuntime" => result.enabled_contexts.enable(RequiredContext::MetaRuntime),
                "CompileDiag" => result.enabled_contexts.enable(RequiredContext::CompileDiag),
                "BuildAssets" => result.enabled_contexts.enable(RequiredContext::BuildAssets),
                "SourceMap" => result.enabled_contexts.enable(RequiredContext::SourceMap),
                "ProjectInfo" => result.enabled_contexts.enable(RequiredContext::ProjectInfo),
                "MetaBench" => result.enabled_contexts.enable(RequiredContext::MetaBench),
                "StageInfo" => result.enabled_contexts.enable(RequiredContext::StageInfo),
                // Bundles
                "MetaReflection" => {
                    result.enabled_contexts.enable(RequiredContext::MetaTypes);
                    result.enabled_contexts.enable(RequiredContext::MetaRuntime);
                    result.enabled_contexts.enable(RequiredContext::CompileDiag);
                }
                _ => {
                    // Check if this looks like a typo of a standard context
                    if let Some(suggestion) = suggest_context_name(name.as_str()) {
                        result.warnings.push(UnknownContextError::PossibleTypo {
                            provided: name.clone(),
                            suggestion,
                        });
                    } else {
                        // User-defined context - track it separately
                        result.user_contexts.push(name.clone());
                    }
                }
            }
        }

        result
    }
}

/// Registry of all builtin functions with their context requirements
pub type BuiltinRegistry = Map<Text, BuiltinInfo>;

/// Error returned when a builtin is called without required context
#[derive(Debug, Clone)]
pub struct MissingContextError {
    /// Name of the builtin function that was called
    pub function_name: Text,
    /// The context that was required
    pub required_context: RequiredContext,
    /// Currently enabled contexts
    pub enabled_contexts: Vec<RequiredContext>,
}

impl fmt::Display for MissingContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "builtin `{}` requires `using [{}]` context",
            self.function_name,
            self.required_context.context_name()
        )
    }
}

impl std::error::Error for MissingContextError {}

/// Error returned when parsing unknown context names
#[derive(Debug, Clone)]
pub enum UnknownContextError {
    /// Unknown context name that looks like a typo of a standard context
    PossibleTypo {
        /// The context name that was provided
        provided: Text,
        /// The suggested correct context name
        suggestion: Text,
    },
}

impl fmt::Display for UnknownContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnknownContextError::PossibleTypo { provided, suggestion } => {
                write!(
                    f,
                    "unknown context `{}`, did you mean `{}`?",
                    provided, suggestion
                )
            }
        }
    }
}

impl std::error::Error for UnknownContextError {}

/// Standard context names that are recognized
const STANDARD_CONTEXT_NAMES: &[&str] = &[
    "MetaTypes",
    "MetaRuntime",
    "CompileDiag",
    "BuildAssets",
    "SourceMap",
    "ProjectInfo",
    "MetaBench",
    "StageInfo",
    "MetaReflection",  // Bundle
    "MetaCore",        // Implicit tier 0
];

/// Check if a name looks like a typo of a standard context name
fn suggest_context_name(name: &str) -> Option<Text> {
    let name_lower = name.to_lowercase();

    // Check for common typo patterns
    for &standard in STANDARD_CONTEXT_NAMES {
        let standard_lower = standard.to_lowercase();

        // Exact case-insensitive match
        if name_lower == standard_lower {
            return Some(Text::from(standard));
        }

        // Check for close matches (Levenshtein-like heuristics)
        // - Missing or extra characters at the end
        // - Common typos
        if name_lower.starts_with(&standard_lower[..standard_lower.len().saturating_sub(2)]) ||
           standard_lower.starts_with(&name_lower[..name_lower.len().saturating_sub(2)]) {
            return Some(Text::from(standard));
        }

        // Check for specific common typos
        if ((name_lower == "metatype" || name_lower == "metatypes") && standard == "MetaTypes") ||
           (name_lower == "metaruntime" && standard == "MetaRuntime") ||
           (name_lower == "runtime" && standard == "MetaRuntime") ||
           (name_lower == "compilediag" && standard == "CompileDiag") ||
           (name_lower == "diagnostic" && standard == "CompileDiag") ||
           (name_lower == "diagnostics" && standard == "CompileDiag") ||
           (name_lower == "buildasset" && standard == "BuildAssets") ||
           (name_lower == "buildassets" && standard == "BuildAssets") ||
           (name_lower == "assets" && standard == "BuildAssets") ||
           (name_lower == "reflection" && standard == "MetaReflection") ||
           (name_lower == "metareflection" && standard == "MetaReflection") ||
           (name_lower == "sourcemap" && standard == "SourceMap") ||
           (name_lower == "source_map" && standard == "SourceMap") ||
           (name_lower == "projectinfo" && standard == "ProjectInfo") ||
           (name_lower == "project_info" && standard == "ProjectInfo") ||
           (name_lower == "metabench" && standard == "MetaBench") ||
           (name_lower == "meta_bench" && standard == "MetaBench") ||
           (name_lower == "bench" && standard == "MetaBench") ||
           (name_lower == "stageinfo" && standard == "StageInfo") ||
           (name_lower == "stage_info" && standard == "StageInfo") ||
           (name_lower == "stage" && standard == "StageInfo") {
            return Some(Text::from(standard));
        }
    }

    None
}

/// Error for duplicate context declarations
#[derive(Debug, Clone)]
pub struct DuplicateContextError {
    /// The context name that was duplicated
    pub name: Text,
}

impl fmt::Display for DuplicateContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "duplicate context `{}` in using clause", self.name)
    }
}

impl std::error::Error for DuplicateContextError {}

/// Result of parsing a using clause
#[derive(Debug, Clone, Default)]
pub struct ParsedUsingClause {
    /// Standard contexts that were recognized
    pub enabled_contexts: EnabledContexts,
    /// User-defined contexts (not standard but allowed)
    pub user_contexts: Vec<Text>,
    /// Warnings for possible typos
    pub warnings: Vec<UnknownContextError>,
    /// Errors for duplicate context declarations
    pub duplicates: Vec<DuplicateContextError>,
}

impl ParsedUsingClause {
    /// Check if there are any errors (duplicates)
    pub fn has_errors(&self) -> bool {
        !self.duplicates.is_empty()
    }

    /// Check if there are any warnings
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_required_context_tier() {
        assert_eq!(RequiredContext::None.tier(), 0);
        assert_eq!(RequiredContext::MetaTypes.tier(), 1);
        assert_eq!(RequiredContext::MetaRuntime.tier(), 1);
        assert_eq!(RequiredContext::CompileDiag.tier(), 1);
        assert_eq!(RequiredContext::BuildAssets.tier(), 1);
    }

    #[test]
    fn test_required_context_names() {
        assert_eq!(RequiredContext::None.context_name(), "MetaCore");
        assert_eq!(RequiredContext::MetaTypes.context_name(), "MetaTypes");
        assert_eq!(RequiredContext::MetaRuntime.context_name(), "MetaRuntime");
        assert_eq!(RequiredContext::CompileDiag.context_name(), "CompileDiag");
        assert_eq!(RequiredContext::BuildAssets.context_name(), "BuildAssets");
    }

    #[test]
    fn test_enabled_contexts_tier0_always_available() {
        let contexts = EnabledContexts::new();
        assert!(contexts.is_enabled(RequiredContext::None));
        assert!(!contexts.is_enabled(RequiredContext::MetaTypes));
    }

    #[test]
    fn test_enabled_contexts_explicit_enable() {
        let mut contexts = EnabledContexts::new();
        contexts.enable(RequiredContext::MetaTypes);
        assert!(contexts.is_enabled(RequiredContext::MetaTypes));
        assert!(!contexts.is_enabled(RequiredContext::MetaRuntime));
    }

    #[test]
    fn test_enabled_contexts_from_using_clause() {
        let names = vec![Text::from("MetaTypes"), Text::from("CompileDiag")];
        let contexts = EnabledContexts::from_using_clause(&names);
        assert!(contexts.is_enabled(RequiredContext::MetaTypes));
        assert!(contexts.is_enabled(RequiredContext::CompileDiag));
        assert!(!contexts.is_enabled(RequiredContext::MetaRuntime));
    }

    #[test]
    fn test_enabled_contexts_bundle() {
        let names = vec![Text::from("MetaReflection")];
        let contexts = EnabledContexts::from_using_clause(&names);
        assert!(contexts.is_enabled(RequiredContext::MetaTypes));
        assert!(contexts.is_enabled(RequiredContext::MetaRuntime));
        assert!(contexts.is_enabled(RequiredContext::CompileDiag));
    }

    #[test]
    fn test_enabled_contexts_all() {
        let contexts = EnabledContexts::all();
        assert!(contexts.is_enabled(RequiredContext::None));
        assert!(contexts.is_enabled(RequiredContext::MetaTypes));
        assert!(contexts.is_enabled(RequiredContext::MetaRuntime));
        assert!(contexts.is_enabled(RequiredContext::CompileDiag));
        assert!(contexts.is_enabled(RequiredContext::BuildAssets));
    }

    #[test]
    fn test_parse_using_clause_typo_detection() {
        // Test that typos generate warnings
        let result = EnabledContexts::parse_using_clause(&[
            Text::from("metatype"),  // Typo of MetaTypes
        ]);
        assert_eq!(result.warnings.len(), 1);
        match &result.warnings[0] {
            UnknownContextError::PossibleTypo { provided, suggestion } => {
                assert_eq!(provided.as_str(), "metatype");
                assert_eq!(suggestion.as_str(), "MetaTypes");
            }
        }
    }

    #[test]
    fn test_parse_using_clause_user_defined() {
        // Test that user-defined contexts are tracked
        let result = EnabledContexts::parse_using_clause(&[
            Text::from("MetaTypes"),
            Text::from("MyCustomContext"),
        ]);
        assert!(result.enabled_contexts.is_enabled(RequiredContext::MetaTypes));
        assert!(result.user_contexts.contains(&Text::from("MyCustomContext")));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_parse_using_clause_case_sensitivity() {
        // Test that case-insensitive typos are detected
        let result = EnabledContexts::parse_using_clause(&[
            Text::from("METATYPES"),  // Wrong case
        ]);
        assert_eq!(result.warnings.len(), 1);
        match &result.warnings[0] {
            UnknownContextError::PossibleTypo { suggestion, .. } => {
                assert_eq!(suggestion.as_str(), "MetaTypes");
            }
        }
    }

    #[test]
    fn test_parse_using_clause_duplicate_detection() {
        // Test that duplicate contexts are detected
        let result = EnabledContexts::parse_using_clause(&[
            Text::from("MetaTypes"),
            Text::from("MetaTypes"),  // Duplicate
        ]);
        assert!(result.has_errors());
        assert_eq!(result.duplicates.len(), 1);
        assert_eq!(result.duplicates[0].name.as_str(), "MetaTypes");
        // Context should still be enabled (just once)
        assert!(result.enabled_contexts.is_enabled(RequiredContext::MetaTypes));
    }

    #[test]
    fn test_parse_using_clause_multiple_duplicates() {
        // Test detection of multiple different duplicates
        let result = EnabledContexts::parse_using_clause(&[
            Text::from("MetaTypes"),
            Text::from("CompileDiag"),
            Text::from("MetaTypes"),  // Duplicate
            Text::from("CompileDiag"), // Duplicate
        ]);
        assert_eq!(result.duplicates.len(), 2);
        // Both contexts should still be enabled
        assert!(result.enabled_contexts.is_enabled(RequiredContext::MetaTypes));
        assert!(result.enabled_contexts.is_enabled(RequiredContext::CompileDiag));
    }
}
