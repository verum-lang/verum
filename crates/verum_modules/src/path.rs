//! Module path handling and resolution.
//!
//! This module defines the structure and manipulation of module paths in Verum.
//! Module paths follow a hierarchical structure (e.g., `std.collections.List`).
//!
//! Module paths follow hierarchical structure (e.g., `std.collections.List`).
//! Absolute paths start from crate root (`crate.*`), relative paths use
//! `self`, `super`, or direct names. Segments are dot-separated identifiers.

use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};

/// Unique identifier for a module.
///
/// Module IDs are allocated sequentially and remain stable throughout
/// a compilation session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ModuleId(u32);

impl ModuleId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Get the inner u32 value (for internal use)
    pub fn get(&self) -> u32 {
        self.0
    }

    /// The root module ID (cog root)
    pub const ROOT: ModuleId = ModuleId(0);
}

impl std::fmt::Display for ModuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ModuleId({})", self.0)
    }
}

/// A module path representing the hierarchical location of a module.
///
/// Examples:
/// - `std` → ["std"]
/// - `std.collections` → ["std", "collections"]
/// - `cog.parser.ast` → ["cog", "parser", "ast"]
///
/// # Specification
///
/// Module paths follow these rules:
/// - Absolute paths start from cog root (`cog.*`) or external cog
/// - Relative paths use `self`, `super`, or direct names
/// - Path segments are identifiers
///
/// Absolute paths start from cog root (`cog.*`) or external cog.
/// Relative paths use `self`, `super`, or direct names.
/// Path segments are identifiers separated by dots.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModulePath {
    segments: List<Text>,
}

impl ModulePath {
    /// Create a new module path from segments.
    pub fn new(segments: List<Text>) -> Self {
        Self { segments }
    }

    /// Create a module path from a string (dot-separated).
    ///
    /// Example: `ModulePath::from_str("std.collections.List")`
    pub fn from_str(path: &str) -> Self {
        let segments = path.split('.').map(Text::from).collect::<List<_>>();
        Self {
            segments: List::from_iter(segments),
        }
    }

    /// Create a root path (cog root).
    pub fn root() -> Self {
        Self {
            segments: List::new(),
        }
    }

    /// Create a single-segment path.
    pub fn single(name: impl Into<Text>) -> Self {
        Self {
            segments: vec![name.into()].into(),
        }
    }

    /// Get the segments of this path.
    pub fn segments(&self) -> &List<Text> {
        &self.segments
    }

    /// Get the last segment (the name).
    pub fn name(&self) -> Maybe<&Text> {
        match self.segments.last() {
            Some(v) => Maybe::Some(v),
            None => Maybe::None,
        }
    }

    /// Get the parent path (all but the last segment).
    pub fn parent(&self) -> Option<ModulePath> {
        if self.segments.len() <= 1 {
            None
        } else {
            let mut parent_segments = self.segments.clone();
            parent_segments.pop();
            Some(ModulePath::new(parent_segments))
        }
    }

    /// Append a segment to this path.
    pub fn push(&mut self, segment: impl Into<Text>) {
        self.segments.push(segment.into());
    }

    /// Create a new path by appending a segment.
    pub fn join(&self, segment: impl Into<Text>) -> ModulePath {
        let mut new_path = self.clone();
        new_path.push(segment);
        new_path
    }

    /// Check if this is a root path.
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// Check if this is an absolute path (starts with `crate` or external crate name).
    pub fn is_absolute(&self) -> bool {
        self.segments
            .first()
            .map(|s| s.as_str() == "cog" || !s.as_str().starts_with("self"))
            .unwrap_or(false)
    }

    /// Check if this is a relative path (starts with `self` or `super`).
    pub fn is_relative(&self) -> bool {
        self.segments
            .first()
            .map(|s| s.as_str() == "self" || s.as_str() == "super")
            .unwrap_or(false)
    }

    /// Resolve a relative path from a base path.
    ///
    /// # Example
    /// ```
    /// use verum_modules::ModulePath;
    ///
    /// let base = ModulePath::from_str("cog.parser.ast");
    /// let relative = ModulePath::from_str("super.lexer");
    /// let resolved = base.resolve(&relative).unwrap();
    /// assert_eq!(resolved.to_string(), "cog.parser.lexer");
    /// ```
    pub fn resolve(&self, relative: &ModulePath) -> Option<ModulePath> {
        if !relative.is_relative() {
            return Some(relative.clone());
        }

        let mut result = self.clone();

        for segment in relative.segments.iter() {
            match segment.as_str() {
                "self" => {
                    // Stay in current module
                }
                "super" => {
                    // Go to parent module
                    if result.segments.is_empty() {
                        return None; // Can't go above root
                    }
                    result.segments.pop();
                }
                name => {
                    // Append child module
                    result.segments.push(Text::from(name));
                }
            }
        }

        Some(result)
    }

    /// Resolve an import path string relative to a current module path.
    ///
    /// This is the centralized import resolution logic. It handles:
    /// - `self.foo` -> sibling module (parent.foo) or child module for mod.vr
    /// - `super.foo` -> parent's sibling module
    /// - Absolute paths -> returned as-is
    ///
    /// # Arguments
    /// * `import_path` - The import path string (e.g., "self.foo", "super.bar", "std.io")
    /// * `current` - The current module's path
    ///
    /// # Returns
    /// * `Ok(ModulePath)` - The resolved absolute module path
    /// * `Err(ModuleError)` - If the path cannot be resolved (e.g., super from root)
    ///
    /// # Example
    /// ```
    /// use verum_modules::ModulePath;
    ///
    /// let current = ModulePath::from_str("handlers.search");
    /// let resolved = ModulePath::resolve_import("self.utils", &current).unwrap();
    /// assert_eq!(resolved.to_string(), "handlers.utils");
    ///
    /// let resolved = ModulePath::resolve_import("super.contexts", &current).unwrap();
    /// assert_eq!(resolved.to_string(), "contexts");
    /// ```
    pub fn resolve_import(
        import_path: &str,
        current: &ModulePath,
    ) -> Result<ModulePath, crate::error::ModuleError> {
        use crate::error::ModuleError;

        if import_path.starts_with("self.") {
            // self.foo.bar means:
            // - For regular modules (handlers.search): handlers.foo.bar (sibling)
            // - For mod.vr modules (contexts): contexts.foo.bar (child)
            //
            // The key insight: mod.vr files represent the "parent" directory, so
            // their children are direct children. Regular files are at the same
            // level as their siblings.
            //
            // We detect mod.vr modules by checking if the current module has a parent.
            // If it does, use parent.rest (sibling). If not, use current.rest (child).
            let rest = import_path.strip_prefix("self.").unwrap_or("");
            if current.is_root() {
                Ok(ModulePath::from_str(rest))
            } else if let Some(parent) = current.parent() {
                // Regular module like handlers.search: self.foo -> handlers.foo
                let mut result = parent;
                for segment in rest.split('.') {
                    if !segment.is_empty() {
                        result.push(segment);
                    }
                }
                Ok(result)
            } else {
                // Top-level mod.vr module like contexts: self.database -> contexts.database
                let mut result = current.clone();
                for segment in rest.split('.') {
                    if !segment.is_empty() {
                        result.push(segment);
                    }
                }
                Ok(result)
            }
        } else if import_path.starts_with("super.") {
            // super.foo.bar means:
            // - From handlers.search: super.contexts -> contexts (sibling of handlers, i.e., src/contexts)
            //
            // super goes up one level from the current module's parent.
            let rest = import_path.strip_prefix("super.").unwrap_or("");
            if let Some(parent) = current.parent() {
                // From handlers.search: parent is handlers
                if let Some(grandparent) = parent.parent() {
                    // grandparent.rest
                    let mut result = grandparent;
                    for segment in rest.split('.') {
                        if !segment.is_empty() {
                            result.push(segment);
                        }
                    }
                    Ok(result)
                } else {
                    // parent is top-level, so super.X is just X at root level
                    Ok(ModulePath::from_str(rest))
                }
            } else {
                // Current is already top-level, super is invalid
                Err(ModuleError::InvalidPath {
                    path: Text::from(import_path),
                    reason: Text::from("cannot use 'super' from root module"),
                    span: None,
                })
            }
        } else {
            // Absolute path, return as-is
            Ok(ModulePath::from_str(import_path))
        }
    }

    /// Get the depth (number of segments).
    pub fn depth(&self) -> usize {
        self.segments.len()
    }

    /// Check if this path is a prefix of another path.
    pub fn is_prefix_of(&self, other: &ModulePath) -> bool {
        if self.segments.len() > other.segments.len() {
            return false;
        }

        self.segments
            .iter()
            .zip(other.segments.iter())
            .all(|(a, b)| a == b)
    }

    /// Check if this path is a descendant of another path.
    pub fn is_descendant_of(&self, ancestor: &ModulePath) -> bool {
        ancestor.is_prefix_of(self) && self.segments.len() > ancestor.segments.len()
    }
}

impl std::fmt::Display for ModulePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.segments.is_empty() {
            write!(f, "<root>")
        } else {
            write!(
                f,
                "{}",
                self.segments
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<List<_>>()
                    .join(".")
            )
        }
    }
}

impl From<&str> for ModulePath {
    fn from(s: &str) -> Self {
        Self::from_str(s)
    }
}

impl From<Text> for ModulePath {
    fn from(s: Text) -> Self {
        Self::from_str(s.as_str())
    }
}

use crate::error::ModuleError;

/// Resolve an import path relative to the current module.
///
/// # Arguments
/// * `import_path` - Raw import path (e.g., "super.super.domain.Package")
/// * `current_module` - Full path of the importing module (e.g., "services.package_service")
///
/// # Returns
/// Fully resolved module path
///
/// # Examples
/// ```
/// // From services.package_service:
/// // super -> services (parent)
/// // super.super.domain -> domain (sibling of services)
/// // crate.domain -> domain (absolute)
/// ```
pub fn resolve_import(
    import_path: &str,
    current_module: &ModulePath,
) -> Result<ModulePath, ModuleError> {
    let segments: Vec<&str> = import_path.split('.').collect();

    // Determine if the import is relative (starts with self/super/cog) or absolute.
    // Absolute imports resolve from the project root, not relative to the current module.
    let first_is_relative = segments.first().is_some_and(|s| {
        *s == "self" || *s == "super" || *s == "cog"
    });
    let mut result_segments = if first_is_relative {
        current_module.segments.clone()
    } else {
        // Absolute import: resolve from project root
        verum_common::List::new()
    };

    for (i, segment) in segments.iter().enumerate() {
        match *segment {
            "self" => {
                // self only valid as first segment
                // self.foo means sibling access: pop to parent, then navigate
                // Example: from domain.package, self.version -> domain.version
                if i != 0 {
                    return Err(ModuleError::InvalidPath {
                        path: Text::from(import_path),
                        reason: Text::from("'self' can only appear at the start of path"),
                        span: None,
                    });
                }
                // Pop to parent for sibling access (if not at root)
                // This aligns with ModulePath::resolve_import semantics
                if !result_segments.is_empty() {
                    result_segments.pop();
                }
            }
            "super" => {
                if result_segments.is_empty() {
                    return Err(ModuleError::InvalidPath {
                        path: Text::from(import_path),
                        reason: Text::from("cannot use 'super' from root module"),
                        span: None,
                    });
                }
                result_segments.pop();
            }
            "cog" | "crate" => {
                // Reset to root - only valid as first segment
                if i != 0 {
                    return Err(ModuleError::InvalidPath {
                        path: Text::from(import_path),
                        reason: Text::from("'cog' can only appear at the start of path"),
                        span: None,
                    });
                }
                result_segments.clear();
            }
            name => {
                result_segments.push(Text::from(name));
            }
        }
    }

    Ok(ModulePath::new(result_segments))
}
