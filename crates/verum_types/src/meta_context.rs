//! Meta Context Validation for Compile-Time Functions
//!
//! This module implements capability-based meta context validation for `meta fn`.
//! Meta functions can only use compiler-provided meta contexts, not runtime contexts.
//!
//! # Meta Contexts
//!
//! The compiler provides 14 meta contexts (Spec: core/meta/contexts.vr):
//!
//! - **BuildAssets**: File loading during compilation
//! - **TypeInfo**: Type introspection (fields_of, variants_of, etc.)
//! - **AstAccess**: AST manipulation and parsing
//! - **CompileDiag**: Emit compiler warnings/errors
//! - **MetaRuntime**: Crate/module identity and runtime introspection
//! - **MacroState**: Macro expansion state
//! - **StageInfo**: N-level staged metaprogramming support
//! - **Hygiene**: Unified hygiene context for macro identifier management
//! - **CodeSearch**: Code search for advanced codebase analysis
//! - **ProjectInfo**: Project metadata and target platform info
//! - **SourceMap**: Source location mapping for generated code
//! - **Schema**: Generated code validation schemas
//! - **DepGraph**: Dependency graph analysis
//! - **MetaBench**: Meta function benchmarking
//!
//! # Context Groups
//!
//! Convenience groups bundle related contexts (Spec: core/meta/contexts.vr lines 2847-3102):
//!
//! - **MetaCore**: [TypeInfo, AstAccess, CompileDiag, Hygiene] (most macros)
//! - **MetaFull**: All 14 contexts
//! - **MetaTypes**: [TypeInfo] only
//! - **MetaSafe**: [TypeInfo, AstAccess, CompileDiag] (no I/O, no state)
//! - **MetaNoIO**: [TypeInfo, AstAccess, CompileDiag, MetaRuntime, MacroState, StageInfo, ProjectInfo, Hygiene]
//! - **MetaDerive**: [TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene]
//! - **MetaAttr**: [BuildAssets, TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene]
//! - **MetaStaged**: [StageInfo, TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene] (multi-stage)
//! - **MetaAnalysis**: [CodeSearch, TypeInfo, AstAccess, CompileDiag]
//! - **MetaProject**: [ProjectInfo, TypeInfo, AstAccess, CompileDiag]
//! - **MetaSourced**: [SourceMap, TypeInfo, AstAccess, CompileDiag]
//! - **MetaValidated**: [Schema, TypeInfo, AstAccess, CompileDiag]
//! - **MetaDeps**: [DepGraph, ProjectInfo, CompileDiag]
//! - **MetaProfiled**: [MetaBench, TypeInfo, AstAccess, CompileDiag]
//! - **MetaTooling**: [CodeSearch, ProjectInfo, SourceMap, Schema, DepGraph, MetaBench, TypeInfo, AstAccess, CompileDiag, MacroState]
//!
//! # Example
//!
//! ```verum
//! // Valid: uses meta context
//! meta fn field_count<T>() -> Int using TypeInfo {
//!     TypeInfo.fields_of::<T>().len()
//! }
//!
//! // Invalid: uses runtime context
//! meta fn bad() using Database {  // E502: Database is not a meta context
//!     ...
//! }
//! ```
//!
//! # Architecture
//!
//! The validation happens in three places (defense-in-depth):
//! 1. **Type System** (this module): Validates context requirements at type check time
//! 2. **Meta Linter**: Additional static analysis in lint passes
//! 3. **Sandbox Runtime**: Runtime enforcement during meta evaluation
//!
//! Meta audit: validation that meta functions are pure and do not access runtime state

use verum_ast::span::Span;
use verum_common::{List, Set, Text};

/// Compiler-provided meta contexts.
///
/// These are the only contexts that can be used in `meta fn`.
/// Each maps to a compiler intrinsic implementation.
///
/// Spec: core/meta/contexts.vr
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetaContext {
    /// Build-time asset loading (files, directories)
    BuildAssets,
    /// Type introspection (fields_of, variants_of, etc.)
    TypeInfo,
    /// AST manipulation and parsing
    AstAccess,
    /// Emit compiler diagnostics
    CompileDiag,
    /// Runtime configuration and limits
    MetaRuntime,
    /// Macro expansion state
    MacroState,
    /// N-level staged metaprogramming support
    StageInfo,
    /// Unified hygiene context for macro identifier management
    Hygiene,
    /// Code search for advanced codebase analysis
    CodeSearch,
    /// Project metadata access
    ProjectInfo,
    /// Source location mapping for generated code
    SourceMap,
    /// Generated code validation schemas
    Schema,
    /// Dependency graph analysis
    DepGraph,
    /// Meta function benchmarking
    MetaBench,
}

impl MetaContext {
    /// Get all meta context names.
    pub fn all() -> &'static [&'static str] {
        &[
            "BuildAssets",
            "TypeInfo",
            "AstAccess",
            "CompileDiag",
            "MetaRuntime",
            "MacroState",
            "StageInfo",
            "Hygiene",
            "CodeSearch",
            "ProjectInfo",
            "SourceMap",
            "Schema",
            "DepGraph",
            "MetaBench",
        ]
    }

    /// Parse a context name into a MetaContext if it's a valid meta context.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "BuildAssets" => Some(MetaContext::BuildAssets),
            "TypeInfo" => Some(MetaContext::TypeInfo),
            "AstAccess" => Some(MetaContext::AstAccess),
            "CompileDiag" => Some(MetaContext::CompileDiag),
            "MetaRuntime" => Some(MetaContext::MetaRuntime),
            "MacroState" => Some(MetaContext::MacroState),
            "StageInfo" => Some(MetaContext::StageInfo),
            "Hygiene" => Some(MetaContext::Hygiene),
            "CodeSearch" => Some(MetaContext::CodeSearch),
            "ProjectInfo" => Some(MetaContext::ProjectInfo),
            "SourceMap" => Some(MetaContext::SourceMap),
            "Schema" => Some(MetaContext::Schema),
            "DepGraph" => Some(MetaContext::DepGraph),
            "MetaBench" => Some(MetaContext::MetaBench),
            _ => None,
        }
    }

    /// Get the context name as a string.
    pub fn name(&self) -> &'static str {
        match self {
            MetaContext::BuildAssets => "BuildAssets",
            MetaContext::TypeInfo => "TypeInfo",
            MetaContext::AstAccess => "AstAccess",
            MetaContext::CompileDiag => "CompileDiag",
            MetaContext::MetaRuntime => "MetaRuntime",
            MetaContext::MacroState => "MacroState",
            MetaContext::StageInfo => "StageInfo",
            MetaContext::Hygiene => "Hygiene",
            MetaContext::CodeSearch => "CodeSearch",
            MetaContext::ProjectInfo => "ProjectInfo",
            MetaContext::SourceMap => "SourceMap",
            MetaContext::Schema => "Schema",
            MetaContext::DepGraph => "DepGraph",
            MetaContext::MetaBench => "MetaBench",
        }
    }
}

/// Predefined context groups for convenience.
///
/// Groups bundle related meta contexts for common use cases.
///
/// Spec: core/meta/contexts.vr lines 2847-3102
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetaContextGroup {
    /// [TypeInfo, AstAccess, CompileDiag, Hygiene] - Most common for macros
    MetaCore,
    /// All contexts
    MetaFull,
    /// [TypeInfo] only
    MetaTypes,
    /// [TypeInfo, AstAccess, CompileDiag] - Safe subset (no I/O, no state)
    MetaSafe,
    /// [TypeInfo, AstAccess, CompileDiag, MetaRuntime, MacroState, StageInfo, ProjectInfo, Hygiene] - No file I/O
    MetaNoIO,
    /// [TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene] - For derive macros
    MetaDerive,
    /// [BuildAssets, TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene] - For attribute macros
    MetaAttr,
    /// [StageInfo, TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene] - For multi-stage compilation
    MetaStaged,
    /// [CodeSearch, TypeInfo, AstAccess, CompileDiag] - For code analysis
    MetaAnalysis,
    /// [ProjectInfo, TypeInfo, AstAccess, CompileDiag] - For project-aware macros
    MetaProject,
    /// [SourceMap, TypeInfo, AstAccess, CompileDiag] - For source mapping
    MetaSourced,
    /// [Schema, TypeInfo, AstAccess, CompileDiag] - For validated code generation
    MetaValidated,
    /// [DepGraph, ProjectInfo, CompileDiag] - For dependency analysis
    MetaDeps,
    /// [MetaBench, TypeInfo, AstAccess, CompileDiag] - For performance profiling
    MetaProfiled,
    /// [CodeSearch, ProjectInfo, SourceMap, Schema, DepGraph, MetaBench, TypeInfo, AstAccess, CompileDiag, MacroState] - Full tooling
    MetaTooling,
}

impl MetaContextGroup {
    /// Get all context group names.
    pub fn all() -> &'static [&'static str] {
        &[
            "MetaCore",
            "MetaFull",
            "MetaTypes",
            "MetaSafe",
            "MetaNoIO",
            "MetaDerive",
            "MetaAttr",
            "MetaStaged",
            "MetaAnalysis",
            "MetaProject",
            "MetaSourced",
            "MetaValidated",
            "MetaDeps",
            "MetaProfiled",
            "MetaTooling",
        ]
    }

    /// Parse a context group name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "MetaCore" => Some(MetaContextGroup::MetaCore),
            "MetaFull" => Some(MetaContextGroup::MetaFull),
            "MetaTypes" => Some(MetaContextGroup::MetaTypes),
            "MetaSafe" => Some(MetaContextGroup::MetaSafe),
            "MetaNoIO" => Some(MetaContextGroup::MetaNoIO),
            "MetaDerive" => Some(MetaContextGroup::MetaDerive),
            "MetaAttr" => Some(MetaContextGroup::MetaAttr),
            "MetaStaged" => Some(MetaContextGroup::MetaStaged),
            "MetaAnalysis" => Some(MetaContextGroup::MetaAnalysis),
            "MetaProject" => Some(MetaContextGroup::MetaProject),
            "MetaSourced" => Some(MetaContextGroup::MetaSourced),
            "MetaValidated" => Some(MetaContextGroup::MetaValidated),
            "MetaDeps" => Some(MetaContextGroup::MetaDeps),
            "MetaProfiled" => Some(MetaContextGroup::MetaProfiled),
            "MetaTooling" => Some(MetaContextGroup::MetaTooling),
            _ => None,
        }
    }

    /// Expand a context group to its constituent meta contexts.
    pub fn expand(&self) -> List<MetaContext> {
        match self {
            // MetaCore: [TypeInfo, AstAccess, CompileDiag, Hygiene]
            MetaContextGroup::MetaCore => List::from_iter([
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
                MetaContext::Hygiene,
            ]),
            // MetaFull: All 14 contexts
            MetaContextGroup::MetaFull => List::from_iter([
                MetaContext::BuildAssets,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
                MetaContext::MetaRuntime,
                MetaContext::MacroState,
                MetaContext::StageInfo,
                MetaContext::CodeSearch,
                MetaContext::ProjectInfo,
                MetaContext::SourceMap,
                MetaContext::Schema,
                MetaContext::DepGraph,
                MetaContext::MetaBench,
                MetaContext::Hygiene,
            ]),
            // MetaTypes: [TypeInfo] only
            MetaContextGroup::MetaTypes => List::from_iter([MetaContext::TypeInfo]),
            // MetaSafe: [TypeInfo, AstAccess, CompileDiag] - no I/O, no state
            MetaContextGroup::MetaSafe => List::from_iter([
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
            ]),
            // MetaNoIO: All except BuildAssets that don't require file I/O
            MetaContextGroup::MetaNoIO => List::from_iter([
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
                MetaContext::MetaRuntime,
                MetaContext::MacroState,
                MetaContext::StageInfo,
                MetaContext::ProjectInfo,
                MetaContext::Hygiene,
            ]),
            // MetaDerive: [TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene]
            MetaContextGroup::MetaDerive => List::from_iter([
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
                MetaContext::MacroState,
                MetaContext::Hygiene,
            ]),
            // MetaAttr: [BuildAssets, TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene]
            MetaContextGroup::MetaAttr => List::from_iter([
                MetaContext::BuildAssets,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
                MetaContext::MacroState,
                MetaContext::Hygiene,
            ]),
            // MetaStaged: [StageInfo, TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene]
            MetaContextGroup::MetaStaged => List::from_iter([
                MetaContext::StageInfo,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
                MetaContext::MacroState,
                MetaContext::Hygiene,
            ]),
            // MetaAnalysis: [CodeSearch, TypeInfo, AstAccess, CompileDiag]
            MetaContextGroup::MetaAnalysis => List::from_iter([
                MetaContext::CodeSearch,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
            ]),
            // MetaProject: [ProjectInfo, TypeInfo, AstAccess, CompileDiag]
            MetaContextGroup::MetaProject => List::from_iter([
                MetaContext::ProjectInfo,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
            ]),
            // MetaSourced: [SourceMap, TypeInfo, AstAccess, CompileDiag]
            MetaContextGroup::MetaSourced => List::from_iter([
                MetaContext::SourceMap,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
            ]),
            // MetaValidated: [Schema, TypeInfo, AstAccess, CompileDiag]
            MetaContextGroup::MetaValidated => List::from_iter([
                MetaContext::Schema,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
            ]),
            // MetaDeps: [DepGraph, ProjectInfo, CompileDiag]
            MetaContextGroup::MetaDeps => List::from_iter([
                MetaContext::DepGraph,
                MetaContext::ProjectInfo,
                MetaContext::CompileDiag,
            ]),
            // MetaProfiled: [MetaBench, TypeInfo, AstAccess, CompileDiag]
            MetaContextGroup::MetaProfiled => List::from_iter([
                MetaContext::MetaBench,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
            ]),
            // MetaTooling: Full tooling support
            MetaContextGroup::MetaTooling => List::from_iter([
                MetaContext::CodeSearch,
                MetaContext::ProjectInfo,
                MetaContext::SourceMap,
                MetaContext::Schema,
                MetaContext::DepGraph,
                MetaContext::MetaBench,
                MetaContext::TypeInfo,
                MetaContext::AstAccess,
                MetaContext::CompileDiag,
                MetaContext::MacroState,
            ]),
        }
    }
}

/// Result of validating a context requirement for a meta function.
#[derive(Debug, Clone)]
pub enum MetaContextValidation {
    /// The context requirement is valid for meta functions.
    Valid(List<MetaContext>),
    /// The context requirement contains invalid (runtime) contexts.
    Invalid {
        /// Invalid context names that are not meta contexts.
        invalid_contexts: List<Text>,
        /// Valid meta contexts found in the requirement.
        valid_contexts: List<MetaContext>,
    },
}

/// Validator for meta function context requirements.
///
/// Ensures that `meta fn` only uses compiler-provided meta contexts.
pub struct MetaContextValidator {
    /// Set of valid meta context names.
    valid_names: Set<Text>,
    /// Set of valid meta context group names.
    valid_groups: Set<Text>,
}

impl Default for MetaContextValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl MetaContextValidator {
    /// Create a new meta context validator.
    pub fn new() -> Self {
        let mut valid_names = Set::new();
        for name in MetaContext::all() {
            valid_names.insert(Text::from(*name));
        }

        let mut valid_groups = Set::new();
        for name in MetaContextGroup::all() {
            valid_groups.insert(Text::from(*name));
        }

        MetaContextValidator {
            valid_names,
            valid_groups,
        }
    }

    /// Check if a name is a valid meta context or meta context group.
    pub fn is_valid_meta_context(&self, name: &str) -> bool {
        self.valid_names.contains(&Text::from(name)) || self.valid_groups.contains(&Text::from(name))
    }

    /// Check if a name is a meta context group (not a single context).
    pub fn is_meta_context_group(&self, name: &str) -> bool {
        self.valid_groups.contains(&Text::from(name))
    }

    /// Validate a list of context requirements for a meta function.
    ///
    /// # Arguments
    ///
    /// * `context_names` - List of context names from the `using` clause
    ///
    /// # Returns
    ///
    /// - `MetaContextValidation::Valid` if all contexts are valid meta contexts
    /// - `MetaContextValidation::Invalid` if any runtime contexts are used
    pub fn validate(&self, context_names: &[Text]) -> MetaContextValidation {
        let mut valid_contexts: List<MetaContext> = List::new();
        let mut invalid_contexts: List<Text> = List::new();

        for name in context_names {
            let name_str = name.as_str();

            // Check if it's a meta context group first
            if let Some(group) = MetaContextGroup::from_name(name_str) {
                // Expand the group and add all contexts
                for ctx in group.expand() {
                    if !valid_contexts.contains(&ctx) {
                        valid_contexts.push(ctx);
                    }
                }
            } else if let Some(ctx) = MetaContext::from_name(name_str) {
                // Single meta context
                if !valid_contexts.contains(&ctx) {
                    valid_contexts.push(ctx);
                }
            } else {
                // Not a meta context - it's a runtime context
                invalid_contexts.push(name.clone());
            }
        }

        if invalid_contexts.is_empty() {
            MetaContextValidation::Valid(valid_contexts)
        } else {
            MetaContextValidation::Invalid {
                invalid_contexts,
                valid_contexts,
            }
        }
    }

    /// Suggest similar meta contexts for a typo.
    pub fn suggest_similar(&self, name: &str) -> Option<&'static str> {
        // Simple Levenshtein-like matching for common typos
        let name_lower = name.to_lowercase();

        for meta_ctx in MetaContext::all() {
            let ctx_lower = meta_ctx.to_lowercase();
            if ctx_lower.contains(&name_lower) || name_lower.contains(&ctx_lower) {
                return Some(meta_ctx);
            }
        }

        for group in MetaContextGroup::all() {
            let group_lower = group.to_lowercase();
            if group_lower.contains(&name_lower) || name_lower.contains(&group_lower) {
                return Some(group);
            }
        }

        None
    }
}

/// Error type for invalid meta context usage.
#[derive(Debug, Clone)]
pub struct InvalidMetaContextError {
    /// The function name that has invalid context requirements.
    pub func_name: Text,
    /// Invalid context names.
    pub invalid_contexts: List<Text>,
    /// Span of the using clause.
    pub span: Span,
}

impl InvalidMetaContextError {
    /// Create a new error.
    pub fn new(func_name: Text, invalid_contexts: List<Text>, span: Span) -> Self {
        InvalidMetaContextError {
            func_name,
            invalid_contexts,
            span,
        }
    }

    /// Format error message.
    pub fn message(&self) -> String {
        let invalid = self
            .invalid_contexts
            .iter()
            .map(|s| format!("`{}`", s))
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "meta function `{}` uses runtime context{} {} which {} not available at compile-time",
            self.func_name,
            if self.invalid_contexts.len() > 1 { "s" } else { "" },
            invalid,
            if self.invalid_contexts.len() > 1 { "are" } else { "is" }
        )
    }

    /// Get a hint for valid meta contexts.
    pub fn hint(&self) -> String {
        format!(
            "meta functions can only use compiler-provided contexts: {}",
            MetaContext::all().join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_meta_context_from_name() {
        assert_eq!(
            MetaContext::from_name("TypeInfo"),
            Some(MetaContext::TypeInfo)
        );
        assert_eq!(
            MetaContext::from_name("BuildAssets"),
            Some(MetaContext::BuildAssets)
        );
        assert_eq!(MetaContext::from_name("Database"), None);
        assert_eq!(MetaContext::from_name("Logger"), None);
    }

    #[test]
    fn test_meta_context_group_expand() {
        // MetaCore: [TypeInfo, AstAccess, CompileDiag, Hygiene]
        let core = MetaContextGroup::MetaCore.expand();
        assert_eq!(core.len(), 4);
        assert!(core.contains(&MetaContext::TypeInfo));
        assert!(core.contains(&MetaContext::AstAccess));
        assert!(core.contains(&MetaContext::CompileDiag));
        assert!(core.contains(&MetaContext::Hygiene));

        // MetaFull: All 14 contexts
        let full = MetaContextGroup::MetaFull.expand();
        assert_eq!(full.len(), 14);
        assert!(full.contains(&MetaContext::Hygiene));

        // MetaStaged: [StageInfo, TypeInfo, AstAccess, CompileDiag, MacroState, Hygiene]
        let staged = MetaContextGroup::MetaStaged.expand();
        assert_eq!(staged.len(), 6);
        assert!(staged.contains(&MetaContext::StageInfo));
        assert!(staged.contains(&MetaContext::TypeInfo));
        assert!(staged.contains(&MetaContext::AstAccess));
        assert!(staged.contains(&MetaContext::CompileDiag));
        assert!(staged.contains(&MetaContext::MacroState));
        assert!(staged.contains(&MetaContext::Hygiene));

        // MetaSafe: [TypeInfo, AstAccess, CompileDiag] - no Hygiene
        let safe = MetaContextGroup::MetaSafe.expand();
        assert_eq!(safe.len(), 3);
        assert!(safe.contains(&MetaContext::TypeInfo));
        assert!(safe.contains(&MetaContext::AstAccess));
        assert!(safe.contains(&MetaContext::CompileDiag));
        assert!(!safe.contains(&MetaContext::Hygiene));

        // MetaNoIO: [TypeInfo, AstAccess, CompileDiag, MetaRuntime, MacroState, StageInfo, ProjectInfo, Hygiene]
        let noio = MetaContextGroup::MetaNoIO.expand();
        assert_eq!(noio.len(), 8);
        assert!(!noio.contains(&MetaContext::BuildAssets));
    }

    #[test]
    fn test_validator_valid_contexts() {
        let validator = MetaContextValidator::new();

        // Single valid context
        let result = validator.validate(&[Text::from("TypeInfo")]);
        assert!(matches!(result, MetaContextValidation::Valid(_)));

        // Multiple valid contexts
        let result = validator.validate(&[
            Text::from("TypeInfo"),
            Text::from("AstAccess"),
            Text::from("CompileDiag"),
        ]);
        assert!(matches!(result, MetaContextValidation::Valid(_)));

        // Context group
        let result = validator.validate(&[Text::from("MetaCore")]);
        assert!(matches!(result, MetaContextValidation::Valid(ctxs) if ctxs.len() == 4));
    }

    #[test]
    fn test_validator_invalid_contexts() {
        let validator = MetaContextValidator::new();

        // Runtime context
        let result = validator.validate(&[Text::from("Database")]);
        assert!(matches!(
            result,
            MetaContextValidation::Invalid { invalid_contexts, .. }
            if invalid_contexts.len() == 1 && invalid_contexts[0].as_str() == "Database"
        ));

        // Mix of valid and invalid
        let result = validator.validate(&[
            Text::from("TypeInfo"),
            Text::from("Database"),
            Text::from("Logger"),
        ]);
        match result {
            MetaContextValidation::Invalid {
                invalid_contexts,
                valid_contexts,
            } => {
                assert_eq!(invalid_contexts.len(), 2);
                assert_eq!(valid_contexts.len(), 1);
                assert!(valid_contexts.contains(&MetaContext::TypeInfo));
            }
            _ => panic!("Expected Invalid result"),
        }
    }

    #[test]
    fn test_validator_is_valid() {
        let validator = MetaContextValidator::new();

        // Valid meta contexts
        assert!(validator.is_valid_meta_context("TypeInfo"));
        assert!(validator.is_valid_meta_context("BuildAssets"));
        assert!(validator.is_valid_meta_context("MetaCore"));
        assert!(validator.is_valid_meta_context("MetaFull"));

        // Invalid runtime contexts
        assert!(!validator.is_valid_meta_context("Database"));
        assert!(!validator.is_valid_meta_context("Logger"));
        assert!(!validator.is_valid_meta_context("FileSystem"));
    }

    #[test]
    fn test_suggest_similar() {
        let validator = MetaContextValidator::new();

        // Typo suggestions
        assert_eq!(validator.suggest_similar("typeinfo"), Some("TypeInfo"));
        assert_eq!(validator.suggest_similar("TypeInf"), Some("TypeInfo"));
        assert_eq!(validator.suggest_similar("Assets"), Some("BuildAssets"));
        assert_eq!(validator.suggest_similar("Ast"), Some("AstAccess"));

        // No suggestion for completely unrelated names
        assert_eq!(validator.suggest_similar("Database"), None);
    }
}
