//! Error types for the module system.
//!
//! Comprehensive error handling for module loading, resolution, and validation.
//! Includes smart suggestions for typos and similar names using Levenshtein distance.
//!
//! Covers module not found, item not found, ambiguous imports, circular
//! dependencies, visibility violations, profile incompatibilities, and more.
//! Includes smart suggestions for typos and similar names using Levenshtein distance.

use crate::path::{ModuleId, ModulePath};
use crate::suggestions::{find_similar_items, find_similar_modules, format_module_suggestions, format_suggestions};
use std::fmt;
use verum_ast::{Path, Span};
use verum_common::{List, Text};

/// Suggestion for breaking a circular dependency.
#[derive(Debug, Clone, PartialEq)]
pub struct CycleBreakSuggestion {
    /// Type of suggestion
    pub kind: CycleBreakKind,
    /// Module(s) involved in this suggestion
    pub modules: List<ModulePath>,
    /// Human-readable description
    pub description: Text,
    /// Estimated complexity (1-5, lower is easier)
    pub complexity: u8,
}

impl CycleBreakSuggestion {
    /// Create a new cycle break suggestion.
    pub fn new(kind: CycleBreakKind, modules: List<ModulePath>, description: impl Into<Text>) -> Self {
        let complexity = kind.default_complexity();
        Self {
            kind,
            modules,
            description: description.into(),
            complexity,
        }
    }

    /// Create a suggestion with custom complexity.
    pub fn with_complexity(mut self, complexity: u8) -> Self {
        self.complexity = complexity.clamp(1, 5);
        self
    }
}

impl fmt::Display for CycleBreakSuggestion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.kind, self.description)
    }
}

/// Kind of cycle-breaking suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleBreakKind {
    /// Extract shared types/protocols into a common module
    ExtractInterface,
    /// Use lazy initialization or late binding
    LazyInit,
    /// Replace direct dependency with protocol/interface
    InvertDependency,
    /// Merge tightly coupled modules
    MergeModules,
    /// Move specific items to break the cycle
    MoveItems,
    /// Convert to runtime dependency (context system)
    RuntimeDependency,
}

impl CycleBreakKind {
    /// Get default complexity for this suggestion kind.
    pub fn default_complexity(&self) -> u8 {
        match self {
            CycleBreakKind::MoveItems => 1,
            CycleBreakKind::LazyInit => 2,
            CycleBreakKind::ExtractInterface => 3,
            CycleBreakKind::InvertDependency => 3,
            CycleBreakKind::MergeModules => 2,
            CycleBreakKind::RuntimeDependency => 4,
        }
    }
}

impl fmt::Display for CycleBreakKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CycleBreakKind::ExtractInterface => write!(f, "Extract Interface"),
            CycleBreakKind::LazyInit => write!(f, "Lazy Init"),
            CycleBreakKind::InvertDependency => write!(f, "Invert Dependency"),
            CycleBreakKind::MergeModules => write!(f, "Merge Modules"),
            CycleBreakKind::MoveItems => write!(f, "Move Items"),
            CycleBreakKind::RuntimeDependency => write!(f, "Runtime Dependency"),
        }
    }
}

/// Result type for module operations.
pub type ModuleResult<T> = Result<T, ModuleError>;

/// Errors that can occur during module operations.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleError {
    /// Module not found at expected location
    ModuleNotFound {
        path: ModulePath,
        searched_paths: List<std::path::PathBuf>,
        /// Suggested similar module paths
        suggestions: List<ModulePath>,
        span: Option<Span>,
    },

    /// Item not found in module
    ItemNotFound {
        item_name: Text,
        module_path: ModulePath,
        available_items: List<Text>,
        /// Suggested similar item names
        suggestions: List<Text>,
        span: Option<Span>,
    },

    /// Ambiguous import (multiple candidates)
    AmbiguousImport {
        name: Text,
        candidates: List<ModulePath>,
        span: Option<Span>,
    },

    /// Circular module dependency detected
    CircularDependency {
        /// Module IDs in the cycle (for internal use)
        cycle: List<ModuleId>,
        /// Module paths in the cycle (for display)
        cycle_paths: List<ModulePath>,
        /// Suggestions for breaking the cycle
        suggestions: List<CycleBreakSuggestion>,
        span: Option<Span>,
    },

    /// Private item accessed from wrong context
    PrivateAccess {
        item_name: Text,
        item_module: ModulePath,
        accessing_module: ModulePath,
        span: Option<Span>,
    },

    /// Visibility violation
    VisibilityViolation {
        item_name: Text,
        required_visibility: Text,
        actual_visibility: Text,
        span: Option<Span>,
    },

    /// Module already loaded with different contents
    ConflictingModule {
        path: ModulePath,
        existing_id: ModuleId,
        span: Option<Span>,
    },

    /// Two filesystem rules both produce a file for the same module path.
    /// Concrete trigger: a project has BOTH `src/foo.vr` (Rule 2 — file
    /// form) AND `src/foo/mod.vr` (Rule 4 — directory form); the loader's
    /// candidate-search returns both, but the module system can only
    /// admit one. Without this diagnostic the loader picks the
    /// first-found candidate and silently drops every declaration in
    /// the loser — the user sees `unbound variable` errors at use-sites
    /// that look like the module wasn't loaded at all.
    ///
    /// Inline-module-block collisions (a file with `module foo { ... }`
    /// inside it AND a sibling file `src/foo.vr` for the same path)
    /// surface through the same variant; the message lists every
    /// existing source so the user knows which to delete.
    PathCollision {
        path: ModulePath,
        winning_path: std::path::PathBuf,
        losing_paths: List<std::path::PathBuf>,
        span: Option<Span>,
    },

    /// Invalid module path
    InvalidPath {
        path: Text,
        reason: Text,
        span: Option<Span>,
    },

    /// Failed to load module file
    IoError {
        path: std::path::PathBuf,
        error: Text,
        span: Option<Span>,
    },

    /// Parse error while loading module
    ParseError {
        path: ModulePath,
        error: Text,
        span: Option<Span>,
    },

    /// Invalid re-export
    InvalidReexport {
        item_name: Text,
        reason: Text,
        span: Option<Span>,
    },

    /// Orphan implementation (violates coherence rules)
    OrphanImpl {
        protocol: Path,
        for_type: Path,
        reason: Text,
        span: Option<Span>,
    },

    /// Profile incompatibility
    ProfileIncompatible {
        module_path: ModulePath,
        required_profile: Text,
        current_profile: Text,
        span: Option<Span>,
    },

    /// Generic error with message
    Other { message: Text, span: Option<Span> },
}

impl ModuleError {
    /// Get the span associated with this error, if any.
    pub fn span(&self) -> Option<Span> {
        match self {
            ModuleError::ModuleNotFound { span, .. } => *span,
            ModuleError::ItemNotFound { span, .. } => *span,
            ModuleError::AmbiguousImport { span, .. } => *span,
            ModuleError::CircularDependency { span, .. } => *span,
            ModuleError::PrivateAccess { span, .. } => *span,
            ModuleError::VisibilityViolation { span, .. } => *span,
            ModuleError::ConflictingModule { span, .. } => *span,
            ModuleError::PathCollision { span, .. } => *span,
            ModuleError::InvalidPath { span, .. } => *span,
            ModuleError::IoError { span, .. } => *span,
            ModuleError::ParseError { span, .. } => *span,
            ModuleError::InvalidReexport { span, .. } => *span,
            ModuleError::OrphanImpl { span, .. } => *span,
            ModuleError::ProfileIncompatible { span, .. } => *span,
            ModuleError::Other { span, .. } => *span,
        }
    }

    /// Create a module not found error
    pub fn module_not_found(path: ModulePath, searched_paths: List<std::path::PathBuf>) -> Self {
        ModuleError::ModuleNotFound {
            path,
            searched_paths,
            suggestions: List::new(),
            span: None,
        }
    }

    /// Create a module not found error with available modules for suggestions
    pub fn module_not_found_with_available(
        path: ModulePath,
        searched_paths: List<std::path::PathBuf>,
        available_modules: impl IntoIterator<Item = ModulePath>,
    ) -> Self {
        let suggestions = find_similar_modules(&path, available_modules);
        ModuleError::ModuleNotFound {
            path,
            searched_paths,
            suggestions,
            span: None,
        }
    }

    /// Create an item not found error
    pub fn item_not_found(
        item_name: impl Into<Text>,
        module_path: ModulePath,
        available_items: List<Text>,
    ) -> Self {
        let name: Text = item_name.into();
        let suggestions = find_similar_items(name.as_str(), &available_items);
        ModuleError::ItemNotFound {
            item_name: name,
            module_path,
            available_items,
            suggestions,
            span: None,
        }
    }

    /// Create an item not found error without computing suggestions
    pub fn item_not_found_simple(
        item_name: impl Into<Text>,
        module_path: ModulePath,
    ) -> Self {
        ModuleError::ItemNotFound {
            item_name: item_name.into(),
            module_path,
            available_items: List::new(),
            suggestions: List::new(),
            span: None,
        }
    }

    /// Create an ambiguous import error
    pub fn ambiguous_import(name: impl Into<Text>, candidates: List<ModulePath>) -> Self {
        ModuleError::AmbiguousImport {
            name: name.into(),
            candidates,
            span: None,
        }
    }

    /// Create a circular dependency error (basic, without paths or suggestions)
    pub fn circular_dependency(cycle: List<ModuleId>) -> Self {
        ModuleError::CircularDependency {
            cycle,
            cycle_paths: List::new(),
            suggestions: List::new(),
            span: None,
        }
    }

    /// Create a circular dependency error with full information.
    pub fn circular_dependency_with_paths(
        cycle: List<ModuleId>,
        cycle_paths: List<ModulePath>,
    ) -> Self {
        let suggestions = generate_cycle_break_suggestions(&cycle_paths);
        ModuleError::CircularDependency {
            cycle,
            cycle_paths,
            suggestions,
            span: None,
        }
    }

    /// Create a circular dependency error with custom suggestions.
    pub fn circular_dependency_with_suggestions(
        cycle: List<ModuleId>,
        cycle_paths: List<ModulePath>,
        suggestions: List<CycleBreakSuggestion>,
    ) -> Self {
        ModuleError::CircularDependency {
            cycle,
            cycle_paths,
            suggestions,
            span: None,
        }
    }

    /// Create a private access error
    pub fn private_access(
        item_name: impl Into<Text>,
        item_module: ModulePath,
        accessing_module: ModulePath,
    ) -> Self {
        ModuleError::PrivateAccess {
            item_name: item_name.into(),
            item_module,
            accessing_module,
            span: None,
        }
    }

    /// Add span information to this error
    pub fn with_span(mut self, span: Span) -> Self {
        match &mut self {
            ModuleError::ModuleNotFound { span: s, .. } => *s = Some(span),
            ModuleError::ItemNotFound { span: s, .. } => *s = Some(span),
            ModuleError::AmbiguousImport { span: s, .. } => *s = Some(span),
            ModuleError::CircularDependency { span: s, .. } => *s = Some(span),
            ModuleError::PrivateAccess { span: s, .. } => *s = Some(span),
            ModuleError::VisibilityViolation { span: s, .. } => *s = Some(span),
            ModuleError::ConflictingModule { span: s, .. } => *s = Some(span),
            ModuleError::PathCollision { span: s, .. } => *s = Some(span),
            ModuleError::InvalidPath { span: s, .. } => *s = Some(span),
            ModuleError::IoError { span: s, .. } => *s = Some(span),
            ModuleError::ParseError { span: s, .. } => *s = Some(span),
            ModuleError::InvalidReexport { span: s, .. } => *s = Some(span),
            ModuleError::OrphanImpl { span: s, .. } => *s = Some(span),
            ModuleError::ProfileIncompatible { span: s, .. } => *s = Some(span),
            ModuleError::Other { span: s, .. } => *s = Some(span),
        }
        self
    }
}

impl fmt::Display for ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModuleError::ModuleNotFound {
                path,
                searched_paths,
                suggestions,
                ..
            } => {
                write!(f, "Module not found: {}", path)?;
                if !suggestions.is_empty() {
                    write!(f, "{}", format_module_suggestions(suggestions))?;
                }
                if !searched_paths.is_empty() {
                    write!(f, "\nSearched paths:")?;
                    for p in searched_paths.iter() {
                        write!(f, "\n  - {}", p.display())?;
                    }
                }
                Ok(())
            }
            ModuleError::ItemNotFound {
                item_name,
                module_path,
                suggestions,
                ..
            } => {
                write!(
                    f,
                    "Item '{}' not found in module '{}'",
                    item_name, module_path
                )?;
                if !suggestions.is_empty() {
                    write!(f, "{}", format_suggestions(suggestions))?;
                }
                Ok(())
            }
            ModuleError::AmbiguousImport {
                name, candidates, ..
            } => {
                write!(f, "Ambiguous import: '{}'", name)?;
                write!(f, "\nFound in multiple modules:")?;
                for candidate in candidates {
                    write!(f, "\n  - {}", candidate)?;
                }
                Ok(())
            }
            ModuleError::CircularDependency {
                cycle,
                cycle_paths,
                suggestions,
                ..
            } => {
                write!(f, "Circular module dependency detected:")?;

                // Prefer showing paths if available, fall back to IDs
                if !cycle_paths.is_empty() {
                    for (i, path) in cycle_paths.iter().enumerate() {
                        if i == 0 {
                            write!(f, "\n  {}", path)?;
                        } else {
                            write!(f, "\n  → {}", path)?;
                        }
                    }
                    // Close the cycle
                    if cycle_paths.len() > 1 {
                        write!(f, "\n  → {}", cycle_paths[0])?;
                    }
                } else {
                    for (i, id) in cycle.iter().enumerate() {
                        if i == 0 {
                            write!(f, "\n  {}", id)?;
                        } else {
                            write!(f, "\n  → {}", id)?;
                        }
                    }
                }

                // Show suggestions if available
                if !suggestions.is_empty() {
                    write!(f, "\n\nSuggestions to break the cycle:")?;
                    for (i, suggestion) in suggestions.iter().enumerate().take(3) {
                        write!(f, "\n  {}. {}", i + 1, suggestion)?;
                    }
                    if suggestions.len() > 3 {
                        write!(f, "\n  ... and {} more", suggestions.len() - 3)?;
                    }
                }

                Ok(())
            }
            ModuleError::PrivateAccess {
                item_name,
                item_module,
                accessing_module,
                ..
            } => {
                write!(
                    f,
                    "Cannot access private item '{}' from module '{}' in module '{}'",
                    item_name, item_module, accessing_module
                )
            }
            ModuleError::VisibilityViolation {
                item_name,
                required_visibility,
                actual_visibility,
                ..
            } => {
                write!(
                    f,
                    "Visibility violation for '{}': requires '{}' but has '{}'",
                    item_name, required_visibility, actual_visibility
                )
            }
            ModuleError::ConflictingModule {
                path, existing_id, ..
            } => {
                write!(
                    f,
                    "Module '{}' conflicts with existing module {}",
                    path, existing_id
                )
            }
            ModuleError::PathCollision {
                path, winning_path, losing_paths, ..
            } => {
                write!(
                    f,
                    "module path collision: '{}' resolves to multiple files on disk:\n  using:    {}",
                    path, winning_path.display(),
                )?;
                for p in losing_paths.iter() {
                    write!(f, "\n  ignoring: {}", p.display())?;
                }
                write!(
                    f,
                    "\n  hint: pick exactly one of the file form (`<name>.vr`) \
                     or the directory form (`<name>/mod.vr`); having both makes \
                     declarations in the loser invisible at use sites and is \
                     classified as `E_MODULE_PATH_COLLISION` per VUVA §15.5",
                )
            }
            ModuleError::InvalidPath { path, reason, .. } => {
                write!(f, "Invalid module path '{}': {}", path, reason)
            }
            ModuleError::IoError { path, error, .. } => {
                write!(f, "Failed to load '{}': {}", path.display(), error)
            }
            ModuleError::ParseError { path, error, .. } => {
                write!(f, "Failed to parse module '{}': {}", path, error)
            }
            ModuleError::InvalidReexport {
                item_name, reason, ..
            } => {
                write!(f, "Invalid re-export of '{}': {}", item_name, reason)
            }
            ModuleError::OrphanImpl {
                protocol,
                for_type,
                reason,
                ..
            } => {
                write!(
                    f,
                    "Orphan implementation: implement {:?} for {:?}\n{}",
                    protocol, for_type, reason
                )
            }
            ModuleError::ProfileIncompatible {
                module_path,
                required_profile,
                current_profile,
                ..
            } => {
                write!(
                    f,
                    "Profile incompatibility: module '{}' requires '{}' but current profile is '{}'",
                    module_path, required_profile, current_profile
                )
            }
            ModuleError::Other { message, .. } => write!(f, "{}", message),
        }
    }
}

impl std::error::Error for ModuleError {}

impl From<std::io::Error> for ModuleError {
    fn from(err: std::io::Error) -> Self {
        ModuleError::Other {
            message: Text::from(err.to_string()),
            span: None,
        }
    }
}

/// Generate suggestions for breaking a circular dependency.
///
/// Analyzes the module paths in the cycle and generates intelligent suggestions
/// based on common patterns like:
/// - Shared parent modules (extract interface)
/// - Module name patterns (e.g., "model" + "service" suggests interface extraction)
/// - Small cycles (merge suggestion for 2-module cycles)
///
/// Suggestions are sorted by complexity (easiest first).
pub fn generate_cycle_break_suggestions(cycle_paths: &List<ModulePath>) -> List<CycleBreakSuggestion> {
    let mut suggestions = List::new();

    if cycle_paths.is_empty() {
        return suggestions;
    }

    // For 2-module cycles, suggest merging as an option
    if cycle_paths.len() == 2 {
        let desc = format!(
            "Consider merging '{}' and '{}' into a single module if they're tightly coupled",
            cycle_paths[0], cycle_paths[1]
        );
        suggestions.push(CycleBreakSuggestion::new(
            CycleBreakKind::MergeModules,
            cycle_paths.clone(),
            desc,
        ));
    }

    // Analyze for common parent - suggests interface extraction
    let common_prefix = find_common_module_prefix(cycle_paths);
    if !common_prefix.is_empty() {
        let interface_module = format!("{}.shared", common_prefix);
        let desc = format!(
            "Extract shared types/protocols into '{}' to break the cycle",
            interface_module
        );
        suggestions.push(CycleBreakSuggestion::new(
            CycleBreakKind::ExtractInterface,
            cycle_paths.clone(),
            desc,
        ));
    }

    // Always suggest dependency inversion as an option
    if cycle_paths.len() >= 2 {
        let desc = format!(
            "Define a protocol in '{}' that '{}' implements, then depend on the protocol instead",
            cycle_paths[0], cycle_paths[1]
        );
        suggestions.push(CycleBreakSuggestion::new(
            CycleBreakKind::InvertDependency,
            List::from(vec![cycle_paths[0].clone(), cycle_paths[1].clone()]),
            desc,
        ));
    }

    // Suggest lazy initialization for runtime dependency resolution
    let desc = "Use lazy initialization or the context system for runtime dependency injection";
    suggestions.push(CycleBreakSuggestion::new(
        CycleBreakKind::RuntimeDependency,
        cycle_paths.clone(),
        desc,
    ));

    // Suggest moving items
    if cycle_paths.len() >= 2 {
        let desc = format!(
            "Identify specific items causing the dependency from '{}' to '{}' and move them",
            cycle_paths[0], cycle_paths[1]
        );
        suggestions.push(CycleBreakSuggestion::new(
            CycleBreakKind::MoveItems,
            List::from(vec![cycle_paths[0].clone(), cycle_paths[1].clone()]),
            desc,
        ));
    }

    // Sort by complexity (easiest first)
    let mut sorted: Vec<_> = suggestions.iter().cloned().collect();
    sorted.sort_by_key(|s| s.complexity);

    List::from(sorted)
}

/// Find the longest common module path prefix.
fn find_common_module_prefix(paths: &List<ModulePath>) -> String {
    if paths.is_empty() {
        return String::new();
    }

    let first_segments: Vec<_> = paths[0].segments().iter().cloned().collect();

    let mut common_len = first_segments.len();

    for path in paths.iter().skip(1) {
        let segments: Vec<_> = path.segments().iter().cloned().collect();
        let mut match_len = 0;

        for (i, seg) in first_segments.iter().enumerate().take(common_len) {
            if i < segments.len() && segments[i] == *seg {
                match_len += 1;
            } else {
                break;
            }
        }

        common_len = match_len;
    }

    if common_len == 0 {
        String::new()
    } else {
        first_segments[..common_len].join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cycle_break_suggestion_creation() {
        let modules = List::from(vec![
            ModulePath::from_str("a.b"),
            ModulePath::from_str("a.c"),
        ]);

        let suggestion = CycleBreakSuggestion::new(
            CycleBreakKind::ExtractInterface,
            modules,
            "Extract shared interface",
        );

        assert_eq!(suggestion.kind, CycleBreakKind::ExtractInterface);
        assert_eq!(suggestion.complexity, 3); // Default for ExtractInterface
    }

    #[test]
    fn test_cycle_break_suggestion_complexity() {
        assert_eq!(CycleBreakKind::MoveItems.default_complexity(), 1);
        assert_eq!(CycleBreakKind::LazyInit.default_complexity(), 2);
        assert_eq!(CycleBreakKind::ExtractInterface.default_complexity(), 3);
        assert_eq!(CycleBreakKind::RuntimeDependency.default_complexity(), 4);
    }

    #[test]
    fn test_generate_cycle_break_suggestions() {
        let paths = List::from(vec![
            ModulePath::from_str("app.models.user"),
            ModulePath::from_str("app.services.auth"),
        ]);

        let suggestions = generate_cycle_break_suggestions(&paths);

        assert!(!suggestions.is_empty());
        // Should have merge, extract, invert, runtime, and move suggestions
        assert!(suggestions.len() >= 4);

        // First suggestion should be easiest (lowest complexity)
        let complexities: Vec<_> = suggestions.iter().map(|s| s.complexity).collect();
        let mut sorted_complexities = complexities.clone();
        sorted_complexities.sort();
        assert_eq!(complexities, sorted_complexities);
    }

    #[test]
    fn test_find_common_module_prefix() {
        let paths = List::from(vec![
            ModulePath::from_str("app.models.user"),
            ModulePath::from_str("app.models.post"),
        ]);

        let prefix = find_common_module_prefix(&paths);
        assert_eq!(prefix, "app.models");
    }

    #[test]
    fn test_find_common_module_prefix_no_common() {
        let paths = List::from(vec![
            ModulePath::from_str("foo.bar"),
            ModulePath::from_str("baz.qux"),
        ]);

        let prefix = find_common_module_prefix(&paths);
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_circular_dependency_error_with_paths() {
        let cycle = List::from(vec![ModuleId::new(1), ModuleId::new(2)]);
        let paths = List::from(vec![
            ModulePath::from_str("a.b"),
            ModulePath::from_str("a.c"),
        ]);

        let err = ModuleError::circular_dependency_with_paths(cycle, paths);

        match err {
            ModuleError::CircularDependency { suggestions, .. } => {
                assert!(!suggestions.is_empty());
            }
            _ => panic!("Expected CircularDependency"),
        }
    }
}
