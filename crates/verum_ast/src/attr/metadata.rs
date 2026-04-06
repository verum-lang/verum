//! Attribute metadata definitions for the Verum AST.
//!
//! This module defines [`AttributeMetadata`], which provides complete
//! compile-time information about an attribute: its name, valid targets,
//! argument specification, documentation, and more.
//!
//! # Overview
//!
//! Every registered attribute in Verum has associated metadata:
//!
//! ```rust
//! use verum_ast::attr::{AttributeMetadata, AttributeTarget, ArgSpec, ArgType, AttributeCategory};
//!
//! let inline_meta = AttributeMetadata::new("inline")
//!     .targets(AttributeTarget::Function)
//!     .args(ArgSpec::Optional(ArgType::Ident))
//!     .category(AttributeCategory::Optimization)
//!     .doc("Control function inlining behavior")
//!     .conflicts_with(["cold"])
//!     .build();
//! ```
//!
//! # Design
//!
//! Metadata is created using the builder pattern for ergonomic construction.
//! Once built, metadata is immutable and can be shared across threads.
//!
//! # Attribute Registry
//!
//! Every attribute in Verum is registered with metadata specifying valid targets
//! (function, type, field, etc.), argument specs, category, stability, conflicts,
//! and requirements. Built-in attributes include @inline, @cold, @hot, @derive,
//! @repr, @align, @cfg, @verify, @deprecated, @test, @export, and many more.
//! User-defined attributes are registered via @tagged_literal or procedural macros.

use serde::{Deserialize, Serialize};
use std::any::TypeId;
use verum_common::{List, Maybe, Text};

use super::args::ArgSpec;
use super::target::AttributeTarget;

/// Complete metadata for a registered attribute.
///
/// Contains all information needed to:
/// - Validate attribute usage at compile time
/// - Generate documentation
/// - Provide IDE support (completion, hover)
/// - Convert to typed attribute structs
#[derive(Debug, Clone)]
pub struct AttributeMetadata {
    /// Attribute name (without `@` prefix)
    pub name: Text,

    /// Valid targets for this attribute
    pub targets: AttributeTarget,

    /// Argument specification
    pub args: ArgSpec,

    /// Category for organization
    pub category: AttributeCategory,

    /// Documentation string (markdown supported)
    pub doc: Text,

    /// Extended documentation with examples
    pub doc_extended: Maybe<Text>,

    /// Whether this attribute can appear multiple times on the same item
    pub repeatable: bool,

    /// Attributes that conflict with this one
    pub conflicts_with: List<Text>,

    /// Attributes that must also be present
    pub requires: List<Text>,

    /// Deprecation notice (if deprecated)
    pub deprecated: Maybe<DeprecationNotice>,

    /// Stability level
    pub stability: Stability,

    /// Minimum language version required
    pub since: Maybe<Text>,

    /// Type ID of the typed attribute struct (for conversion)
    ///
    /// This is `None` for attributes without a typed representation.
    typed_attr_id: Maybe<TypeIdWrapper>,

    /// Whether this is a built-in compiler attribute
    pub builtin: bool,

    /// Feature flag required to use this attribute
    pub feature_gate: Maybe<Text>,
}

/// Wrapper for TypeId to make it serializable (for debugging only).
///
/// Note: TypeId serialization is not stable across compilations,
/// so this should only be used for debugging, not persistence.
#[derive(Debug, Clone)]
struct TypeIdWrapper(TypeId);

impl Serialize for TypeIdWrapper {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a debug string (not stable, for debugging only)
        serializer.serialize_str(&format!("{:?}", self.0))
    }
}

impl<'de> Deserialize<'de> for TypeIdWrapper {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Cannot deserialize TypeId - return a placeholder
        // This is fine because we only use TypeId at runtime, not after serialization
        Err(serde::de::Error::custom(
            "TypeId cannot be deserialized; use typed_attr methods at runtime only",
        ))
    }
}

impl AttributeMetadata {
    /// Start building a new attribute metadata.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_ast::attr::AttributeMetadata;
    ///
    /// let meta = AttributeMetadata::new("inline")
    ///     .doc("Control function inlining")
    ///     .build();
    /// ```
    #[must_use]
    pub fn new(name: impl Into<Text>) -> AttributeMetadataBuilder {
        AttributeMetadataBuilder::new(name)
    }

    /// Check if this attribute is valid for the given target.
    #[must_use]
    pub fn is_valid_for(&self, target: AttributeTarget) -> bool {
        self.targets.contains(target)
    }

    /// Check if this attribute conflicts with any in the given list.
    ///
    /// Returns the first conflicting attribute name, if any.
    #[must_use]
    pub fn conflicts_with_any(&self, attrs: &[&Text]) -> Maybe<Text> {
        for attr_name in attrs {
            if self.conflicts_with.iter().any(|c| c == *attr_name) {
                return Maybe::Some((*attr_name).clone());
            }
        }
        Maybe::None
    }

    /// Check if this attribute requires any missing attributes.
    ///
    /// Returns a list of missing required attributes.
    #[must_use]
    pub fn missing_requirements(&self, attrs: &[&Text]) -> List<Text> {
        self.requires
            .iter()
            .filter(|req| !attrs.iter().any(|a| a.as_str() == req.as_str()))
            .cloned()
            .collect()
    }

    /// Check if this attribute is deprecated.
    #[must_use]
    pub fn is_deprecated(&self) -> bool {
        self.deprecated.is_some()
    }

    /// Check if this attribute requires a feature flag.
    #[must_use]
    pub fn requires_feature(&self) -> bool {
        self.feature_gate.is_some()
    }

    /// Get the typed attribute TypeId, if available.
    #[must_use]
    pub fn typed_attr_type_id(&self) -> Option<TypeId> {
        self.typed_attr_id.as_ref().map(|w| w.0)
    }

    /// Check if this attribute has a typed representation.
    #[must_use]
    pub fn has_typed_repr(&self) -> bool {
        self.typed_attr_id.is_some()
    }
}

/// Category for attribute organization.
///
/// Categories help organize attributes in documentation and IDE support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[derive(Default)]
pub enum AttributeCategory {
    /// Optimization hints: `@inline`, `@cold`, `@hot`, `@vectorize`
    Optimization,

    /// Serialization control: `@serialize`, `@skip_serialize`, `@rename`
    Serialization,

    /// Validation rules: `@validate`, `@range`, `@pattern`
    Validation,

    /// Documentation: `@doc`, `@deprecated`, `@experimental`
    Documentation,

    /// Safety annotations: `@verify`, `@trusted`, `@unsafe_fn`
    Safety,

    /// Module-level control: `@profile`, `@feature`
    ModuleControl,

    /// Language core: `@std`, `@derive`, `@specialize`
    LanguageCore,

    /// Memory layout: `@repr`, `@align`, `@packed`
    Layout,

    /// Concurrency: `@lock_level`, `@deadlock_detection`
    Concurrency,

    /// Meta-system: `@tagged_literal`, `@const_eval`
    MetaSystem,

    /// Testing: `@test`, `@bench`, `@ignore`
    Testing,

    /// FFI and interop: `@export`, `@import`, `@calling_convention`
    FFI,

    /// Platform/Conditional compilation: `@cfg`, `@target_os`, `@target_arch`
    /// Controls conditional compilation based on target platform, features, etc.
    Platform,

    /// Custom/user-defined attributes
    #[default]
    Custom,
}

impl AttributeCategory {
    /// Get a human-readable name for this category.
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Optimization => "Optimization",
            Self::Serialization => "Serialization",
            Self::Validation => "Validation",
            Self::Documentation => "Documentation",
            Self::Safety => "Safety",
            Self::ModuleControl => "Module Control",
            Self::LanguageCore => "Language Core",
            Self::Layout => "Memory Layout",
            Self::Concurrency => "Concurrency",
            Self::MetaSystem => "Meta-System",
            Self::Testing => "Testing",
            Self::FFI => "FFI",
            Self::Platform => "Platform",
            Self::Custom => "Custom",
        }
    }

    /// Get a short description of this category.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Optimization => "Hints for compiler optimizations",
            Self::Serialization => "Control serialization and deserialization",
            Self::Validation => "Data validation constraints",
            Self::Documentation => "Documentation and deprecation",
            Self::Safety => "Safety annotations and verification",
            Self::ModuleControl => "Module-level configuration",
            Self::LanguageCore => "Core language features",
            Self::Layout => "Memory layout control",
            Self::Concurrency => "Concurrency and synchronization",
            Self::MetaSystem => "Compile-time metaprogramming",
            Self::Testing => "Test and benchmark annotations",
            Self::FFI => "Foreign function interface",
            Self::Platform => "Conditional compilation and target platform",
            Self::Custom => "User-defined attributes",
        }
    }
}


impl std::fmt::Display for AttributeCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Deprecation notice for an attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeprecationNotice {
    /// Version when deprecation started
    pub since: Text,

    /// Suggested replacement
    pub replacement: Maybe<Text>,

    /// Reason for deprecation
    pub reason: Maybe<Text>,

    /// Version when it will be removed
    pub removal: Maybe<Text>,
}

impl DeprecationNotice {
    /// Create a simple deprecation notice.
    #[must_use]
    pub fn new(since: impl Into<Text>) -> Self {
        Self {
            since: since.into(),
            replacement: Maybe::None,
            reason: Maybe::None,
            removal: Maybe::None,
        }
    }

    /// Create a deprecation notice with a replacement suggestion.
    #[must_use]
    pub fn with_replacement(since: impl Into<Text>, replacement: impl Into<Text>) -> Self {
        Self {
            since: since.into(),
            replacement: Maybe::Some(replacement.into()),
            reason: Maybe::None,
            removal: Maybe::None,
        }
    }

    /// Add a reason for deprecation.
    #[must_use]
    pub fn reason(mut self, reason: impl Into<Text>) -> Self {
        self.reason = Maybe::Some(reason.into());
        self
    }

    /// Add planned removal version.
    #[must_use]
    pub fn removal(mut self, version: impl Into<Text>) -> Self {
        self.removal = Maybe::Some(version.into());
        self
    }

    /// Format as a warning message.
    #[must_use]
    pub fn message(&self, attr_name: &str) -> Text {
        let mut msg = format!("`@{}` is deprecated since {}", attr_name, self.since);

        if let Maybe::Some(ref replacement) = self.replacement {
            msg.push_str(&format!("; use `@{}` instead", replacement));
        }

        if let Maybe::Some(ref reason) = self.reason {
            msg.push_str(&format!(": {}", reason));
        }

        if let Maybe::Some(ref removal) = self.removal {
            msg.push_str(&format!(" (will be removed in {})", removal));
        }

        Text::from(msg)
    }
}

/// Stability level for an attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Stability {
    /// Stable API, will not change
    #[default]
    Stable,

    /// May change in future versions
    Unstable,

    /// Internal compiler use only
    Internal,

    /// Experimental, may be removed
    Experimental,
}

impl Stability {
    /// Get a human-readable name.
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Unstable => "unstable",
            Self::Internal => "internal",
            Self::Experimental => "experimental",
        }
    }

    /// Check if this stability level requires a feature flag.
    #[must_use]
    pub const fn requires_feature(&self) -> bool {
        matches!(self, Self::Unstable | Self::Experimental)
    }

    /// Check if this is usable in user code.
    #[must_use]
    pub const fn is_public(&self) -> bool {
        matches!(self, Self::Stable | Self::Unstable | Self::Experimental)
    }
}

impl std::fmt::Display for Stability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Builder for [`AttributeMetadata`].
///
/// Use [`AttributeMetadata::new()`] to start building.
#[derive(Debug)]
pub struct AttributeMetadataBuilder {
    meta: AttributeMetadata,
}

impl AttributeMetadataBuilder {
    /// Create a new builder with the given attribute name.
    fn new(name: impl Into<Text>) -> Self {
        Self {
            meta: AttributeMetadata {
                name: name.into(),
                targets: AttributeTarget::empty(),
                args: ArgSpec::None,
                category: AttributeCategory::Custom,
                doc: Text::new(),
                doc_extended: Maybe::None,
                repeatable: false,
                conflicts_with: List::new(),
                requires: List::new(),
                deprecated: Maybe::None,
                stability: Stability::Stable,
                since: Maybe::None,
                typed_attr_id: Maybe::None,
                builtin: false,
                feature_gate: Maybe::None,
            },
        }
    }

    /// Set valid targets for this attribute.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use verum_ast::attr::{AttributeMetadata, AttributeTarget};
    ///
    /// let meta = AttributeMetadata::new("inline")
    ///     .targets(AttributeTarget::Function)
    ///     .build();
    /// ```
    #[must_use]
    pub fn targets(mut self, targets: AttributeTarget) -> Self {
        self.meta.targets = targets;
        self
    }

    /// Set argument specification.
    #[must_use]
    pub fn args(mut self, args: ArgSpec) -> Self {
        self.meta.args = args;
        self
    }

    /// Set category.
    #[must_use]
    pub fn category(mut self, category: AttributeCategory) -> Self {
        self.meta.category = category;
        self
    }

    /// Set documentation string.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<Text>) -> Self {
        self.meta.doc = doc.into();
        self
    }

    /// Set extended documentation with examples.
    #[must_use]
    pub fn doc_extended(mut self, doc: impl Into<Text>) -> Self {
        self.meta.doc_extended = Maybe::Some(doc.into());
        self
    }

    /// Mark this attribute as repeatable.
    #[must_use]
    pub fn repeatable(mut self) -> Self {
        self.meta.repeatable = true;
        self
    }

    /// Set conflicting attributes.
    #[must_use]
    pub fn conflicts_with<I, S>(mut self, attrs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Text>,
    {
        self.meta.conflicts_with = attrs.into_iter().map(Into::into).collect();
        self
    }

    /// Set required attributes.
    #[must_use]
    pub fn requires<I, S>(mut self, attrs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Text>,
    {
        self.meta.requires = attrs.into_iter().map(Into::into).collect();
        self
    }

    /// Mark as deprecated.
    #[must_use]
    pub fn deprecated(mut self, notice: DeprecationNotice) -> Self {
        self.meta.deprecated = Maybe::Some(notice);
        self
    }

    /// Set stability level.
    #[must_use]
    pub fn stability(mut self, stability: Stability) -> Self {
        self.meta.stability = stability;
        self
    }

    /// Set minimum language version.
    #[must_use]
    pub fn since(mut self, version: impl Into<Text>) -> Self {
        self.meta.since = Maybe::Some(version.into());
        self
    }

    /// Associate with a typed attribute struct.
    ///
    /// This enables conversion from generic `Attribute` to the typed struct.
    #[must_use]
    pub fn typed_as<T: 'static>(mut self) -> Self {
        self.meta.typed_attr_id = Maybe::Some(TypeIdWrapper(TypeId::of::<T>()));
        self
    }

    /// Mark as a built-in compiler attribute.
    #[must_use]
    pub fn builtin(mut self) -> Self {
        self.meta.builtin = true;
        self
    }

    /// Set required feature flag.
    #[must_use]
    pub fn feature_gate(mut self, feature: impl Into<Text>) -> Self {
        self.meta.feature_gate = Maybe::Some(feature.into());
        self
    }

    /// Build the metadata.
    #[must_use]
    pub fn build(self) -> AttributeMetadata {
        self.meta
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attr::ArgType;

    #[test]
    fn test_basic_metadata() {
        let meta = AttributeMetadata::new("inline")
            .targets(AttributeTarget::Function)
            .args(ArgSpec::Optional(ArgType::Ident))
            .category(AttributeCategory::Optimization)
            .doc("Control function inlining")
            .build();

        assert_eq!(meta.name.as_str(), "inline");
        assert!(meta.is_valid_for(AttributeTarget::Function));
        assert!(!meta.is_valid_for(AttributeTarget::Field));
        assert_eq!(meta.category, AttributeCategory::Optimization);
    }

    #[test]
    fn test_conflicts() {
        let meta = AttributeMetadata::new("cold")
            .targets(AttributeTarget::Function)
            .conflicts_with(["hot", "inline"])
            .build();

        let hot = Text::from("hot");
        let inline = Text::from("inline");
        let other = Text::from("other");

        assert!(meta.conflicts_with_any(&[&hot]).is_some());
        assert!(meta.conflicts_with_any(&[&inline]).is_some());
        assert!(meta.conflicts_with_any(&[&other]).is_none());
    }

    #[test]
    fn test_requirements() {
        let meta = AttributeMetadata::new("specialize")
            .requires(["impl"])
            .build();

        let impl_attr = Text::from("impl");
        let other = Text::from("other");

        assert!(meta.missing_requirements(&[&impl_attr]).is_empty());
        assert!(!meta.missing_requirements(&[&other]).is_empty());
    }

    #[test]
    fn test_deprecation() {
        let notice = DeprecationNotice::with_replacement("2.0", "new_attr")
            .reason("superseded by better API")
            .removal("3.0");

        assert_eq!(notice.since.as_str(), "2.0");
        assert!(notice.replacement.is_some());
        assert!(notice.reason.is_some());
        assert!(notice.removal.is_some());

        let msg = notice.message("old_attr");
        assert!(msg.as_str().contains("deprecated"));
        assert!(msg.as_str().contains("new_attr"));
    }

    #[test]
    fn test_category() {
        assert_eq!(
            AttributeCategory::Optimization.display_name(),
            "Optimization"
        );
        assert!(!AttributeCategory::Optimization.description().is_empty());
    }

    #[test]
    fn test_stability() {
        assert!(!Stability::Stable.requires_feature());
        assert!(Stability::Unstable.requires_feature());
        assert!(Stability::Experimental.requires_feature());
        assert!(Stability::Stable.is_public());
        assert!(!Stability::Internal.is_public());
    }

    #[test]
    fn test_typed_attr() {
        #[derive(Debug)]
        struct TestAttr;

        let meta = AttributeMetadata::new("test")
            .typed_as::<TestAttr>()
            .build();

        assert!(meta.has_typed_repr());
        assert_eq!(meta.typed_attr_type_id(), Some(TypeId::of::<TestAttr>()));
    }

    #[test]
    fn test_builder_chain() {
        let meta = AttributeMetadata::new("complex")
            .targets(AttributeTarget::Function | AttributeTarget::Type)
            .args(ArgSpec::Required(ArgType::String))
            .category(AttributeCategory::Validation)
            .doc("Short doc")
            .doc_extended("Extended documentation with examples...")
            .conflicts_with(["other"])
            .requires(["base"])
            .stability(Stability::Unstable)
            .since("2.7")
            .feature_gate("extended_validation")
            .builtin()
            .build();

        assert!(meta.is_valid_for(AttributeTarget::Function));
        assert!(meta.is_valid_for(AttributeTarget::Type));
        assert!(!meta.is_valid_for(AttributeTarget::Field));
        assert!(meta.builtin);
        assert!(meta.requires_feature());
    }
}
