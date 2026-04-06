//! Module system warnings.
//!
//! Provides warnings for non-fatal issues like prelude shadowing,
//! unused imports, and other patterns that may indicate bugs.
//!
//! Detects patterns like prelude shadowing (local definition shadows a
//! std.prelude item), unused imports, glob import conflicts, deprecated
//! items, self-shadowing, and module name collisions (file + inline).

use crate::path::{ModuleId, ModulePath};
use crate::resolver::NameKind;
use verum_ast::Span;
use verum_common::{List, Text};

/// A warning severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningSeverity {
    /// Informational - not necessarily a problem
    Info,
    /// Warning - may indicate a bug
    Warning,
    /// Deprecated - code pattern is discouraged
    Deprecated,
}

impl std::fmt::Display for WarningSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WarningSeverity::Info => write!(f, "info"),
            WarningSeverity::Warning => write!(f, "warning"),
            WarningSeverity::Deprecated => write!(f, "deprecated"),
        }
    }
}

/// A module system warning.
#[derive(Debug, Clone)]
pub struct ModuleWarning {
    /// Warning kind
    pub kind: WarningKind,
    /// Severity level
    pub severity: WarningSeverity,
    /// Module where the warning occurred
    pub module_id: ModuleId,
    /// Source span (if available)
    pub span: Option<Span>,
    /// Additional context
    pub context: Option<Text>,
}

impl ModuleWarning {
    /// Create a new warning.
    pub fn new(kind: WarningKind, module_id: ModuleId) -> Self {
        let severity = kind.default_severity();
        Self {
            kind,
            severity,
            module_id,
            span: None,
            context: None,
        }
    }

    /// Add source span to the warning.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Add context to the warning.
    pub fn with_context(mut self, context: impl Into<Text>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Get a user-friendly message for this warning.
    pub fn message(&self) -> String {
        match &self.kind {
            WarningKind::PreludeShadowing {
                name,
                local_kind,
                prelude_kind,
            } => {
                format!(
                    "{} `{}` shadows prelude {} of the same name",
                    local_kind, name, prelude_kind
                )
            }
            WarningKind::UnusedImport { name, module_path } => {
                format!("unused import `{}` from `{}`", name, module_path)
            }
            WarningKind::GlobImportShadowing {
                name,
                first_module,
                second_module,
            } => {
                format!(
                    "`{}` is imported from both `{}` and `{}` via glob imports",
                    name, first_module, second_module
                )
            }
            WarningKind::DeprecatedItem { name, reason } => match reason {
                Some(reason) => format!("`{}` is deprecated: {}", name, reason),
                None => format!("`{}` is deprecated", name),
            },
            WarningKind::SelfShadowing { name, outer_kind } => {
                format!(
                    "definition of `{}` shadows {} in outer scope",
                    name, outer_kind
                )
            }
            WarningKind::ModuleNameCollision {
                name,
                file_path,
                inline_span: _,
            } => {
                format!(
                    "module `{}` is defined both in `{}` and inline",
                    name, file_path
                )
            }
        }
    }

    /// Get a help message for this warning.
    pub fn help(&self) -> Option<String> {
        match &self.kind {
            WarningKind::PreludeShadowing { name, .. } => {
                Some(format!(
                    "consider renaming `{}` or using `std.prelude.{}` to access the prelude item",
                    name, name
                ))
            }
            WarningKind::UnusedImport { .. } => Some("remove this import".to_string()),
            WarningKind::GlobImportShadowing { name, .. } => {
                Some(format!(
                    "use explicit import `import {{ {} }}` to resolve ambiguity",
                    name
                ))
            }
            WarningKind::DeprecatedItem { .. } => None,
            WarningKind::SelfShadowing { name, .. } => {
                Some(format!("consider renaming `{}` to avoid confusion", name))
            }
            WarningKind::ModuleNameCollision { name, .. } => {
                Some(format!(
                    "remove either the file module or inline module for `{}`",
                    name
                ))
            }
        }
    }
}

impl std::fmt::Display for ModuleWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.severity, self.message())?;
        if let Some(help) = self.help() {
            write!(f, "\n  help: {}", help)?;
        }
        Ok(())
    }
}

/// Kinds of module warnings.
#[derive(Debug, Clone)]
pub enum WarningKind {
    /// A local definition shadows a prelude item.
    PreludeShadowing {
        /// The name being shadowed
        name: Text,
        /// Kind of the local item
        local_kind: NameKind,
        /// Kind of the prelude item
        prelude_kind: NameKind,
    },

    /// An import is not used anywhere in the module.
    UnusedImport {
        /// The imported name
        name: Text,
        /// The module it was imported from
        module_path: ModulePath,
    },

    /// Multiple glob imports provide the same name.
    GlobImportShadowing {
        /// The name with multiple definitions
        name: Text,
        /// First module providing the name
        first_module: ModulePath,
        /// Second module providing the name
        second_module: ModulePath,
    },

    /// An item is marked as deprecated.
    DeprecatedItem {
        /// The deprecated item name
        name: Text,
        /// Deprecation reason (if provided)
        reason: Option<Text>,
    },

    /// A definition shadows another definition in an outer scope.
    SelfShadowing {
        /// The name being shadowed
        name: Text,
        /// Kind of the outer item
        outer_kind: NameKind,
    },

    /// A module is defined both as a file and inline.
    ModuleNameCollision {
        /// The module name
        name: Text,
        /// Path to the file module
        file_path: Text,
        /// Span of the inline module
        inline_span: Option<Span>,
    },
}

impl WarningKind {
    /// Get the default severity for this warning kind.
    pub fn default_severity(&self) -> WarningSeverity {
        match self {
            WarningKind::PreludeShadowing { .. } => WarningSeverity::Warning,
            WarningKind::UnusedImport { .. } => WarningSeverity::Warning,
            WarningKind::GlobImportShadowing { .. } => WarningSeverity::Warning,
            WarningKind::DeprecatedItem { .. } => WarningSeverity::Deprecated,
            WarningKind::SelfShadowing { .. } => WarningSeverity::Warning,
            WarningKind::ModuleNameCollision { .. } => WarningSeverity::Warning,
        }
    }

    /// Get a short code for this warning kind.
    pub fn code(&self) -> &'static str {
        match self {
            WarningKind::PreludeShadowing { .. } => "W001",
            WarningKind::UnusedImport { .. } => "W002",
            WarningKind::GlobImportShadowing { .. } => "W003",
            WarningKind::DeprecatedItem { .. } => "W004",
            WarningKind::SelfShadowing { .. } => "W005",
            WarningKind::ModuleNameCollision { .. } => "W006",
        }
    }
}

/// Warning collector for gathering warnings during compilation.
#[derive(Debug, Default)]
pub struct WarningCollector {
    /// Collected warnings
    warnings: List<ModuleWarning>,
    /// Whether to suppress warnings by code
    suppressed: std::collections::HashSet<&'static str>,
}

impl WarningCollector {
    /// Create a new warning collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a warning.
    pub fn add(&mut self, warning: ModuleWarning) {
        // Check if this warning type is suppressed
        if !self.suppressed.contains(warning.kind.code()) {
            self.warnings.push(warning);
        }
    }

    /// Suppress warnings of a specific code.
    pub fn suppress(&mut self, code: &'static str) {
        self.suppressed.insert(code);
    }

    /// Unsuppress warnings of a specific code.
    pub fn unsuppress(&mut self, code: &'static str) {
        self.suppressed.remove(code);
    }

    /// Get all collected warnings.
    pub fn warnings(&self) -> &List<ModuleWarning> {
        &self.warnings
    }

    /// Drain all warnings.
    pub fn drain(&mut self) -> List<ModuleWarning> {
        std::mem::take(&mut self.warnings)
    }

    /// Check if any warnings were collected.
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Get count of warnings by severity.
    pub fn count_by_severity(&self, severity: WarningSeverity) -> usize {
        self.warnings.iter().filter(|w| w.severity == severity).count()
    }

    /// Get warnings for a specific module.
    pub fn warnings_for_module(&self, module_id: ModuleId) -> impl Iterator<Item = &ModuleWarning> {
        self.warnings.iter().filter(move |w| w.module_id == module_id)
    }

    /// Clear all warnings.
    pub fn clear(&mut self) {
        self.warnings.clear();
    }
}

/// Prelude shadowing checker.
///
/// Checks if definitions shadow prelude items and generates warnings.
#[derive(Debug)]
pub struct PreludeShadowingChecker {
    /// Known prelude items (name -> kind)
    prelude_items: std::collections::HashMap<Text, NameKind>,
}

impl PreludeShadowingChecker {
    /// Create a new prelude shadowing checker.
    pub fn new() -> Self {
        Self {
            prelude_items: std::collections::HashMap::new(),
        }
    }

    /// Register a prelude item.
    pub fn register_prelude_item(&mut self, name: impl Into<Text>, kind: NameKind) {
        self.prelude_items.insert(name.into(), kind);
    }

    /// Register standard prelude items.
    ///
    /// This registers the common items from std.prelude.
    pub fn register_standard_prelude(&mut self) {
        // Types
        for name in [
            "Int", "Float", "Bool", "Text", "Char", "Unit",
            "List", "Map", "Set", "Maybe", "Result",
            "Option", "Some", "None", "Ok", "Err",
            "Heap", "Shared", "Weak",
        ] {
            self.prelude_items.insert(Text::from(name), NameKind::Type);
        }

        // Functions
        for name in [
            "print", "println", "panic", "assert", "assert_eq",
            "unreachable", "todo", "unimplemented",
        ] {
            self.prelude_items.insert(Text::from(name), NameKind::Function);
        }

        // Protocols
        for name in [
            "Iterator", "Clone", "Copy", "Debug", "Display",
            "Eq", "Ord", "Hash", "Default", "From", "Into",
        ] {
            self.prelude_items.insert(Text::from(name), NameKind::Protocol);
        }
    }

    /// Check if a name shadows a prelude item.
    ///
    /// Returns a warning if shadowing is detected.
    pub fn check_shadowing(
        &self,
        name: &Text,
        local_kind: NameKind,
        module_id: ModuleId,
        span: Option<Span>,
    ) -> Option<ModuleWarning> {
        if let Some(&prelude_kind) = self.prelude_items.get(name) {
            let mut warning = ModuleWarning::new(
                WarningKind::PreludeShadowing {
                    name: name.clone(),
                    local_kind,
                    prelude_kind,
                },
                module_id,
            );
            if let Some(span) = span {
                warning = warning.with_span(span);
            }
            Some(warning)
        } else {
            None
        }
    }

    /// Check if a name is a prelude item.
    pub fn is_prelude_item(&self, name: &str) -> bool {
        self.prelude_items.contains_key(&Text::from(name))
    }

    /// Get the kind of a prelude item.
    pub fn prelude_item_kind(&self, name: &str) -> Option<NameKind> {
        self.prelude_items.get(&Text::from(name)).copied()
    }
}

impl Default for PreludeShadowingChecker {
    fn default() -> Self {
        let mut checker = Self::new();
        checker.register_standard_prelude();
        checker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prelude_shadowing_detection() {
        let checker = PreludeShadowingChecker::default();

        // "List" is a prelude type
        let warning = checker.check_shadowing(
            &Text::from("List"),
            NameKind::Type,
            ModuleId::new(1),
            None,
        );
        assert!(warning.is_some());
        let w = warning.unwrap();
        assert!(matches!(w.kind, WarningKind::PreludeShadowing { .. }));

        // "MyCustomType" is not in prelude
        let warning = checker.check_shadowing(
            &Text::from("MyCustomType"),
            NameKind::Type,
            ModuleId::new(1),
            None,
        );
        assert!(warning.is_none());
    }

    #[test]
    fn test_prelude_shadowing_message() {
        let checker = PreludeShadowingChecker::default();

        let warning = checker.check_shadowing(
            &Text::from("print"),
            NameKind::Function,
            ModuleId::new(1),
            None,
        )
        .unwrap();

        let message = warning.message();
        assert!(message.contains("print"));
        assert!(message.contains("shadows"));
        assert!(message.contains("prelude"));
    }

    #[test]
    fn test_warning_severity() {
        let warning = ModuleWarning::new(
            WarningKind::PreludeShadowing {
                name: Text::from("List"),
                local_kind: NameKind::Type,
                prelude_kind: NameKind::Type,
            },
            ModuleId::new(1),
        );

        assert_eq!(warning.severity, WarningSeverity::Warning);
    }

    #[test]
    fn test_warning_collector() {
        let mut collector = WarningCollector::new();

        collector.add(ModuleWarning::new(
            WarningKind::PreludeShadowing {
                name: Text::from("List"),
                local_kind: NameKind::Type,
                prelude_kind: NameKind::Type,
            },
            ModuleId::new(1),
        ));

        assert!(collector.has_warnings());
        assert_eq!(collector.warnings().len(), 1);
    }

    #[test]
    fn test_warning_suppression() {
        let mut collector = WarningCollector::new();
        collector.suppress("W001"); // Suppress prelude shadowing

        collector.add(ModuleWarning::new(
            WarningKind::PreludeShadowing {
                name: Text::from("List"),
                local_kind: NameKind::Type,
                prelude_kind: NameKind::Type,
            },
            ModuleId::new(1),
        ));

        // Warning should be suppressed
        assert!(!collector.has_warnings());
    }

    #[test]
    fn test_warning_codes() {
        assert_eq!(
            WarningKind::PreludeShadowing {
                name: Text::from("x"),
                local_kind: NameKind::Type,
                prelude_kind: NameKind::Type,
            }
            .code(),
            "W001"
        );
        assert_eq!(
            WarningKind::UnusedImport {
                name: Text::from("x"),
                module_path: ModulePath::from_str("a"),
            }
            .code(),
            "W002"
        );
    }

    #[test]
    fn test_glob_import_shadowing_warning() {
        let warning = ModuleWarning::new(
            WarningKind::GlobImportShadowing {
                name: Text::from("HashMap"),
                first_module: ModulePath::from_str("std.collections"),
                second_module: ModulePath::from_str("my.collections"),
            },
            ModuleId::new(1),
        );

        let message = warning.message();
        assert!(message.contains("HashMap"));
        assert!(message.contains("std.collections"));
        assert!(message.contains("my.collections"));
    }
}
