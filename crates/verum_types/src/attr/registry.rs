//! Attribute Registry for Verum.
//!
//! This module provides a centralized registry for all known attributes,
//! enabling compile-time validation, IDE support, and documentation generation.
//!
//! # Overview
//!
//! The [`AttributeRegistry`] is the single source of truth for attribute metadata.
//! It provides:
//!
//! - Validation of attribute names and targets
//! - Argument specification checking
//! - Conflict and requirement detection
//! - IDE completion support
//! - Documentation generation
//!
//! # Usage
//!
//! ```rust
//! use verum_types::attr::{registry, AttributeRegistry};
//! use verum_ast::attr::{Attribute, AttributeTarget};
//!
//! // Get read access to global registry
//! let reg = registry();
//!
//! // Check if an attribute exists
//! assert!(reg.exists("inline"));
//! assert!(reg.exists("cold"));
//!
//! // Validate an attribute
//! let attr = Attribute::simple("inline".into(), Default::default());
//! let result = reg.validate(&attr, AttributeTarget::Function);
//! assert!(result.is_ok());
//! ```
//!
//! # Thread Safety
//!
//! The registry is thread-safe and can be accessed from multiple threads
//! concurrently. Use [`registry()`] for read access and [`registry_mut()`]
//! for write access (rare, typically only during initialization).
//!
//! # Specification
//!
//! Attribute registry: validation rules for @derive, @verify, @cfg, @repr and other compile-time attributes

use std::sync::RwLock;

use once_cell::sync::Lazy;
use verum_ast::attr::{
    ArgSpec, ArgValidationError, Attribute, AttributeCategory, AttributeMetadata, AttributeTarget,
};
use verum_common::{List, Map, Maybe, Text};

use super::error::AttributeError;

/// Global attribute registry instance.
///
/// Initialized lazily on first access with all standard Verum attributes.
pub static REGISTRY: Lazy<RwLock<AttributeRegistry>> = Lazy::new(|| {
    let mut registry = AttributeRegistry::new();
    super::standard::register_standard_attributes(&mut registry);
    RwLock::new(registry)
});

/// Get read access to the global attribute registry.
///
/// This is the primary way to access the registry for validation.
///
/// # Panics
///
/// Panics if the registry lock is poisoned (should never happen in practice).
#[must_use]
pub fn registry() -> std::sync::RwLockReadGuard<'static, AttributeRegistry> {
    REGISTRY.read().expect("attribute registry lock poisoned")
}

/// Get write access to the global attribute registry.
///
/// This is rarely needed - typically only during initialization or
/// when registering custom attributes.
///
/// # Panics
///
/// Panics if the registry lock is poisoned.
#[must_use]
pub fn registry_mut() -> std::sync::RwLockWriteGuard<'static, AttributeRegistry> {
    REGISTRY.write().expect("attribute registry lock poisoned")
}

/// Centralized registry for all known attributes.
///
/// The registry maintains metadata for all registered attributes and provides
/// validation, lookup, and iteration capabilities.
#[derive(Debug)]
pub struct AttributeRegistry {
    /// Attributes indexed by name
    attrs: Map<Text, AttributeMetadata>,

    /// Attributes indexed by category for quick lookup
    by_category: Map<AttributeCategory, List<Text>>,

    /// Whether to allow unknown attributes (for gradual adoption)
    allow_unknown: bool,

    /// Whether to emit warnings for unknown attributes (when allowed)
    warn_unknown: bool,
}

impl AttributeRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            attrs: Map::new(),
            by_category: Map::new(),
            allow_unknown: false,
            warn_unknown: true,
        }
    }

    /// Register a new attribute.
    ///
    /// # Errors
    ///
    /// Returns an error if an attribute with the same name is already registered.
    pub fn register(&mut self, meta: AttributeMetadata) -> Result<(), RegistryError> {
        if self.attrs.contains_key(&meta.name) {
            return Err(RegistryError::AlreadyRegistered(meta.name.clone()));
        }

        // Index by category
        self.by_category
            .entry(meta.category)
            .or_default()
            .push(meta.name.clone());

        self.attrs.insert(meta.name.clone(), meta);
        Ok(())
    }

    /// Register an attribute, replacing any existing registration.
    ///
    /// Use this for testing or overriding built-in attributes.
    pub fn register_or_replace(&mut self, meta: AttributeMetadata) {
        // Remove from old category if exists
        if let Some(old) = self.attrs.get(&meta.name) {
            if let Some(names) = self.by_category.get_mut(&old.category) {
                names.retain(|n| n != &meta.name);
            }
        }

        // Index by new category
        self.by_category
            .entry(meta.category)
            .or_default()
            .push(meta.name.clone());

        self.attrs.insert(meta.name.clone(), meta);
    }

    /// Look up attribute metadata by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&AttributeMetadata> {
        self.attrs.get(&Text::from(name))
    }

    /// Check if an attribute with the given name exists.
    #[must_use]
    pub fn exists(&self, name: &str) -> bool {
        self.attrs.contains_key(&Text::from(name))
    }

    /// Get all attributes in a specific category.
    #[must_use]
    pub fn by_category(&self, category: AttributeCategory) -> List<&AttributeMetadata> {
        self.by_category
            .get(&category)
            .map(|names| names.iter().filter_map(|n| self.attrs.get(n)).collect())
            .unwrap_or_default()
    }

    /// Get all registered attribute names.
    #[must_use]
    pub fn all_names(&self) -> List<&Text> {
        self.attrs.keys().collect()
    }

    /// Get the total number of registered attributes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.attrs.len()
    }

    /// Check if the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty()
    }

    /// Validate a single attribute.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The attribute is unknown and `allow_unknown` is false
    /// - The attribute is not valid for the given target
    /// - The attribute arguments are invalid
    pub fn validate(
        &self,
        attr: &Attribute,
        target: AttributeTarget,
    ) -> Result<ValidationResult, AttributeError> {
        match self.get(attr.name.as_str()) {
            Some(meta) => {
                let mut result = ValidationResult::ok();

                // Check target validity
                if !meta.is_valid_for(target) {
                    return Err(AttributeError::InvalidTarget {
                        attr: attr.name.clone(),
                        target,
                        valid_targets: meta.targets,
                        span: attr.span,
                    });
                }

                // Validate arguments
                if let Err(e) = self.validate_args(&meta.args, &attr.args, attr.span) {
                    return Err(AttributeError::InvalidArgs {
                        attr: attr.name.clone(),
                        error: e,
                        span: attr.span,
                    });
                }

                // Check deprecation (warning, not error)
                if let Maybe::Some(notice) = &meta.deprecated {
                    result.warnings.push(ValidationWarning::Deprecated {
                        attr: attr.name.clone(),
                        notice: notice.clone(),
                        span: attr.span,
                    });
                }

                // Check stability
                if meta.stability.requires_feature() {
                    if let Maybe::Some(feature) = &meta.feature_gate {
                        result.warnings.push(ValidationWarning::UnstableFeature {
                            attr: attr.name.clone(),
                            feature: feature.clone(),
                            span: attr.span,
                        });
                    }
                }

                Ok(result)
            }
            None if self.allow_unknown => {
                let mut result = ValidationResult::ok();
                if self.warn_unknown {
                    result.warnings.push(ValidationWarning::Unknown {
                        attr: attr.name.clone(),
                        suggestions: self.suggest_similar(&attr.name),
                        span: attr.span,
                    });
                }
                Ok(result)
            }
            None => Err(AttributeError::Unknown {
                attr: attr.name.clone(),
                span: attr.span,
                suggestions: self.suggest_similar(&attr.name),
            }),
        }
    }

    /// Validate a collection of attributes on the same item.
    ///
    /// This checks for:
    /// - Individual attribute validity
    /// - Duplicate non-repeatable attributes
    /// - Conflicting attributes
    /// - Missing required attributes
    ///
    /// # Errors
    ///
    /// Returns a list of all validation errors found.
    pub fn validate_collection(
        &self,
        attrs: &[Attribute],
        target: AttributeTarget,
    ) -> Result<ValidationResult, List<AttributeError>> {
        let mut errors = List::new();
        let mut result = ValidationResult::ok();

        // Validate each attribute individually
        for attr in attrs {
            match self.validate(attr, target) {
                Ok(r) => result.warnings.extend(r.warnings),
                Err(e) => errors.push(e),
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        // Check for duplicates (non-repeatable)
        let mut seen: Map<&Text, &Attribute> = Map::new();
        for attr in attrs {
            if let Some(meta) = self.get(attr.name.as_str()) {
                if !meta.repeatable {
                    if let Some(first) = seen.get(&&attr.name) {
                        errors.push(AttributeError::Duplicate {
                            attr: attr.name.clone(),
                            first_span: first.span,
                            second_span: attr.span,
                        });
                    }
                }
                seen.insert(&attr.name, attr);
            }
        }

        // Check for conflicts
        let attr_names: List<&Text> = attrs.iter().map(|a| &a.name).collect();
        for attr in attrs {
            if let Some(meta) = self.get(attr.name.as_str()) {
                if let Maybe::Some(conflict) =
                    meta.conflicts_with_any(&attr_names)
                {
                    errors.push(AttributeError::Conflict {
                        attr1: attr.name.clone(),
                        attr2: conflict,
                        span: attr.span,
                    });
                }
            }
        }

        // Check for missing requirements
        for attr in attrs {
            if let Some(meta) = self.get(attr.name.as_str()) {
                let missing =
                    meta.missing_requirements(&attr_names);
                for req in missing {
                    errors.push(AttributeError::MissingRequirement {
                        attr: attr.name.clone(),
                        requires: req,
                        span: attr.span,
                    });
                }
            }
        }

        if errors.is_empty() {
            Ok(result)
        } else {
            Err(errors)
        }
    }

    /// Set whether to allow unknown attributes.
    ///
    /// When enabled, unknown attributes will produce warnings instead of errors.
    pub fn set_allow_unknown(&mut self, allow: bool) {
        self.allow_unknown = allow;
    }

    /// Set whether to warn about unknown attributes.
    ///
    /// Only applies when `allow_unknown` is true.
    pub fn set_warn_unknown(&mut self, warn: bool) {
        self.warn_unknown = warn;
    }

    /// Suggest similar attribute names for typo correction.
    fn suggest_similar(&self, name: &str) -> List<Text> {
        self.attrs
            .keys()
            .filter(|k| levenshtein_distance(name, k.as_str()) <= 2)
            .take(3)
            .cloned()
            .collect()
    }

    /// Validate attribute arguments against a specification.
    fn validate_args(
        &self,
        spec: &ArgSpec,
        args: &Maybe<List<verum_ast::expr::Expr>>,
        span: verum_ast::span::Span,
    ) -> Result<(), ArgValidationError> {
        match spec {
            ArgSpec::None => {
                if args.is_some() {
                    return Err(ArgValidationError::unexpected_args(span));
                }
            }
            ArgSpec::Required(_ty) => match args {
                Maybe::None => {
                    return Err(ArgValidationError {
                        kind: verum_ast::attr::ArgValidationErrorKind::TooFewArgs {
                            min: 1,
                            got: 0,
                        },
                        span,
                        context: Maybe::None,
                    });
                }
                Maybe::Some(list) if list.is_empty() => {
                    return Err(ArgValidationError {
                        kind: verum_ast::attr::ArgValidationErrorKind::TooFewArgs {
                            min: 1,
                            got: 0,
                        },
                        span,
                        context: Maybe::None,
                    });
                }
                _ => {}
            },
            ArgSpec::Optional(_ty) => {
                // Optional can have 0 or 1 argument
                if let Maybe::Some(list) = args {
                    if list.len() > 1 {
                        return Err(ArgValidationError {
                            kind: verum_ast::attr::ArgValidationErrorKind::TooManyArgs {
                                max: 1,
                                got: list.len(),
                            },
                            span,
                            context: Maybe::None,
                        });
                    }
                }
            }
            ArgSpec::Variadic(_ty) => {
                // Variadic accepts any number
            }
            ArgSpec::Named(specs) => {
                if let Maybe::Some(list) = args {
                    // Parse named arguments from key=value expressions (BinOp::Assign)
                    // e.g., @deprecated(since = "2.0", use = "new_fn")
                    let mut found_names: Vec<Text> = Vec::new();

                    for expr in list.iter() {
                        match &expr.kind {
                            verum_ast::expr::ExprKind::Binary {
                                op: verum_ast::expr::BinOp::Assign,
                                left,
                                ..
                            } => {
                                // Extract the argument name from the left side
                                if let verum_ast::expr::ExprKind::Path(path) = &left.kind {
                                    if let Some(ident) = path.as_ident() {
                                        let arg_name = ident.name.clone();
                                        // Check for duplicate arguments
                                        if found_names.iter().any(|n| n.as_str() == arg_name.as_str()) {
                                            return Err(ArgValidationError {
                                                kind: verum_ast::attr::ArgValidationErrorKind::DuplicateArg {
                                                    name: arg_name,
                                                },
                                                span,
                                                context: Maybe::None,
                                            });
                                        }
                                        // Check that the argument name is recognized
                                        if !specs.iter().any(|s| s.matches_name(arg_name.as_str())) {
                                            let suggestions: List<Text> = specs.iter()
                                                .map(|s| s.name.clone())
                                                .collect();
                                            return Err(ArgValidationError::unknown_arg(
                                                arg_name,
                                                suggestions,
                                                span,
                                            ));
                                        }
                                        found_names.push(arg_name);
                                    }
                                }
                            }
                            _ => {
                                // Non key=value expression in a Named arg spec is invalid
                                return Err(ArgValidationError {
                                    kind: verum_ast::attr::ArgValidationErrorKind::InvalidValue {
                                        message: Text::from("expected named argument (key = value)"),
                                    },
                                    span,
                                    context: Maybe::None,
                                });
                            }
                        }
                    }

                    // Check that all required named args were provided
                    for spec in specs {
                        if spec.required {
                            if !found_names.iter().any(|n| spec.matches_name(n.as_str())) {
                                return Err(ArgValidationError::missing_required(
                                    spec.name.clone(),
                                    span,
                                ));
                            }
                        }
                    }
                } else {
                    // No arguments provided - check if any are required
                    for spec in specs {
                        if spec.required {
                            return Err(ArgValidationError::missing_required(
                                spec.name.clone(),
                                span,
                            ));
                        }
                    }
                }
            }
            ArgSpec::Mixed {
                positional: _,
                named: _,
            } => {
                // Mixed validation - complex, simplified here
            }
            ArgSpec::Either {
                positional: _,
                named: _,
            } => {
                // Either validation - complex, simplified here
            }
            ArgSpec::Custom { description: _ } => {
                // Custom validation is handled elsewhere
            }
        }
        Ok(())
    }

    /// Iterate over all registered attributes.
    pub fn iter(&self) -> impl Iterator<Item = (&Text, &AttributeMetadata)> {
        self.attrs.iter()
    }

    /// Get all categories that have registered attributes.
    #[must_use]
    pub fn categories(&self) -> List<AttributeCategory> {
        self.by_category.keys().copied().collect()
    }
}

impl Default for AttributeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Error during registry operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RegistryError {
    /// Attribute with this name is already registered
    #[error("attribute `@{0}` is already registered")]
    AlreadyRegistered(Text),
}

/// Result of successful validation.
#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    /// Warnings encountered during validation
    pub warnings: List<ValidationWarning>,
}

impl ValidationResult {
    /// Create an empty successful result.
    #[must_use]
    pub fn ok() -> Self {
        Self::default()
    }

    /// Check if there are any warnings.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// Warning from validation (non-fatal).
#[derive(Debug, Clone)]
pub enum ValidationWarning {
    /// Unknown attribute (when `allow_unknown` is true)
    Unknown {
        attr: Text,
        suggestions: List<Text>,
        span: verum_ast::span::Span,
    },

    /// Deprecated attribute
    Deprecated {
        attr: Text,
        notice: verum_ast::attr::DeprecationNotice,
        span: verum_ast::span::Span,
    },

    /// Unstable feature required
    UnstableFeature {
        attr: Text,
        feature: Text,
        span: verum_ast::span::Span,
    },
}

impl ValidationWarning {
    /// Get a human-readable message for this warning.
    #[must_use]
    pub fn message(&self) -> Text {
        match self {
            Self::Unknown {
                attr, suggestions, ..
            } => {
                if suggestions.is_empty() {
                    Text::from(format!("unknown attribute `@{}`", attr))
                } else {
                    Text::from(format!(
                        "unknown attribute `@{}`; did you mean {}?",
                        attr,
                        suggestions
                            .iter()
                            .map(|s| format!("`@{}`", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            Self::Deprecated { attr, notice, .. } => notice.message(attr.as_str()),
            Self::UnstableFeature { attr, feature, .. } => Text::from(format!(
                "`@{}` is unstable and requires feature `{}`",
                attr, feature
            )),
        }
    }
}

/// Calculate Levenshtein distance between two strings.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    if a_chars.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a_chars.len();
    }

    let mut matrix = vec![vec![0usize; b_chars.len() + 1]; a_chars.len() + 1];

    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b_chars.len() {
        matrix[0][j] = j;
    }

    for i in 1..=a_chars.len() {
        for j in 1..=b_chars.len() {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }

    matrix[a_chars.len()][b_chars.len()]
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::attr::{ArgType, AttributeMetadata as AstMeta};

    fn test_registry() -> AttributeRegistry {
        let mut reg = AttributeRegistry::new();

        reg.register(
            AstMeta::new("inline")
                .targets(AttributeTarget::Function)
                .args(ArgSpec::Optional(ArgType::Ident))
                .category(AttributeCategory::Optimization)
                .build(),
        )
        .unwrap();

        reg.register(
            AstMeta::new("cold")
                .targets(AttributeTarget::Function | AttributeTarget::MatchArm)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .conflicts_with(["hot"])
                .build(),
        )
        .unwrap();

        reg.register(
            AstMeta::new("hot")
                .targets(AttributeTarget::Function | AttributeTarget::MatchArm)
                .args(ArgSpec::None)
                .category(AttributeCategory::Optimization)
                .conflicts_with(["cold"])
                .build(),
        )
        .unwrap();

        reg
    }

    #[test]
    fn test_register() {
        let reg = test_registry();
        assert!(reg.exists("inline"));
        assert!(reg.exists("cold"));
        assert!(!reg.exists("unknown"));
    }

    #[test]
    fn test_duplicate_registration() {
        let mut reg = test_registry();
        let result = reg.register(
            AstMeta::new("inline")
                .targets(AttributeTarget::Type)
                .build(),
        );
        assert!(matches!(result, Err(RegistryError::AlreadyRegistered(_))));
    }

    #[test]
    fn test_validate_known() {
        let reg = test_registry();
        let attr = Attribute::simple(Text::from("inline"), Default::default());
        let result = reg.validate(&attr, AttributeTarget::Function);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_unknown() {
        let reg = test_registry();
        let attr = Attribute::simple(Text::from("unknown"), Default::default());
        let result = reg.validate(&attr, AttributeTarget::Function);
        assert!(matches!(result, Err(AttributeError::Unknown { .. })));
    }

    #[test]
    fn test_validate_invalid_target() {
        let reg = test_registry();
        let attr = Attribute::simple(Text::from("inline"), Default::default());
        let result = reg.validate(&attr, AttributeTarget::Field);
        assert!(matches!(result, Err(AttributeError::InvalidTarget { .. })));
    }

    #[test]
    fn test_by_category() {
        let reg = test_registry();
        let opt_attrs = reg.by_category(AttributeCategory::Optimization);
        assert_eq!(opt_attrs.len(), 3);
    }

    #[test]
    fn test_suggest_similar() {
        let reg = test_registry();
        let suggestions = reg.suggest_similar("inlie"); // typo for "inline"
        assert!(suggestions.iter().any(|s| s.as_str() == "inline"));
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein_distance("inline", "inline"), 0);
        assert_eq!(levenshtein_distance("inline", "inlie"), 1);
        assert_eq!(levenshtein_distance("cold", "hot"), 3); // c->h, o->o, l->t, d removed
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }
}
