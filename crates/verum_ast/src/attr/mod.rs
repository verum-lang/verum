//! Attribute system for the Verum AST.
//!
//! This module provides comprehensive support for Verum's attribute system,
//! including typed attributes, target specifications, and metadata.
//!
//! # Overview
//!
//! Verum attributes follow the `@attribute` syntax and can be applied to
//! various syntactic elements:
//!
//! ```verum
//! @profile(systems)                         // Module attribute
//! module kernel {
//!     @derive(Clone, Serialize)             // Type attribute
//!     type Config is {
//!         @serialize(rename = "configId")   // Field attribute
//!         @validate(min_length = 1)
//!         id: Text,
//!
//!         @deprecated(since = "2.0")
//!         legacy_field: Int,
//!     };
//!
//!     @inline(always)                        // Function attribute
//!     @verify(static)
//!     fn process(
//!         @unused _ctx: &Context,            // Parameter attribute
//!     ) -> Result<Config> {
//!         match result {
//!             @cold Err(e) => handle(e),     // Match arm attribute
//!             Ok(config) => config,
//!         }
//!     }
//! }
//! ```
//!
//! # Module Structure
//!
//! - [`target`]: `AttributeTarget` bitflags for valid attribute positions
//! - [`args`]: Argument specifications (`ArgSpec`, `ArgType`)
//! - [`metadata`]: Complete attribute metadata with builder pattern
//! - [`typed`]: All typed attribute structs (`InlineAttr`, `ColdAttr`, etc.)
//!
//! # Design Principles
//!
//! 1. **Type Safety**: Typed attribute structs for compile-time safety
//! 2. **Extensibility**: Generic `Attribute` for unknown/custom attributes
//! 3. **Validation**: `ArgSpec` enables argument validation
//! 4. **Documentation**: `AttributeMetadata` captures full attribute info
//!
//! # Example Usage
//!
//! ## Creating Typed Attributes
//!
//! ```rust
//! use verum_ast::attr::{InlineAttr, InlineMode, ColdAttr};
//! use verum_ast::span::Span;
//!
//! let inline = InlineAttr::new(InlineMode::Always, Span::default());
//! let cold = ColdAttr::new(Span::default());
//! ```
//!
//! ## Defining Attribute Metadata
//!
//! ```rust
//! use verum_ast::attr::{
//!     AttributeMetadata, AttributeTarget, AttributeCategory,
//!     ArgSpec, ArgType,
//! };
//!
//! let meta = AttributeMetadata::new("inline")
//!     .targets(AttributeTarget::Function)
//!     .args(ArgSpec::Optional(ArgType::Ident))
//!     .category(AttributeCategory::Optimization)
//!     .doc("Control function inlining behavior")
//!     .conflicts_with(["cold"])
//!     .build();
//! ```
//!
//! ## Checking Attribute Targets
//!
//! ```rust
//! use verum_ast::attr::AttributeTarget;
//!
//! let targets = AttributeTarget::Function | AttributeTarget::Type;
//! assert!(targets.contains(AttributeTarget::Function));
//! assert!(!targets.contains(AttributeTarget::Field));
//! ```
//!
//! # Attribute Positions and Registry
//!
//! Verum supports attributes on functions, types, modules, impl blocks, constants,
//! statics, contexts, protocols (item-level), fields, variants (member-level),
//! parameters, match arms, loops, expressions, and statements (code-level).
//! Each attribute is registered with valid targets, argument specs, and metadata.

// =============================================================================
// SUBMODULES
// =============================================================================

pub mod args;
pub mod conversion;
pub mod metadata;
pub mod target;
mod typed;

// =============================================================================
// RE-EXPORTS
// =============================================================================

// Target system
pub use target::AttributeTarget;

// Argument specifications
pub use args::{
    ArgSpec, ArgType, ArgValidationError, ArgValidationErrorKind, ArgValidationResult,
    ArgValidationWarning, ArgValidationWarningKind, NamedArgSpec,
};

// Metadata system
pub use metadata::{
    AttributeCategory, AttributeMetadata, AttributeMetadataBuilder, DeprecationNotice, Stability,
};

// Typed attributes - all from the typed module
pub use typed::{
    // Bitfield attributes (low-level bit manipulation)
    BitfieldAttr,
    BitOffsetAttr,
    BitsAttr,
    EndianAttr,
    // MMIO/Register attributes (memory-mapped hardware access)
    AccessMode,
    RegisterBlockAttr,
    RegisterOffsetAttr,
    // Optimization attributes
    AccessPattern,
    AccessPatternAttr,
    AlignAttr,
    AssumeAttr,
    // Core attributes
    Attribute,
    BlackBoxAttr,
    ColdAttr,
    ConstEvalAttr,
    ConstEvalMode,
    CpuDispatchAttr,
    // Multi-version dispatch for SIMD
    MultiversionAttr,
    MultiversionVariant,
    // Interrupt handling attributes
    InterruptAttr,
    // Concurrency attributes
    DeadlockDetectionAttr,
    DifferentiableAttr,
    FeatureAttr,
    FrameworkAttr,
    HotAttr,
    InlineAttr,
    InlineMode,
    IvdepAttr,
    Likelihood,
    LikelihoodAttr,
    LockLevelAttr,
    LtoAttr,
    LtoMode,
    NoAliasAttr,
    OptimizationAttr,
    OptimizationHints,
    OptimizationLevel,
    OptimizeAttr,
    OptimizeBarrierAttr,
    ParallelAttr,
    PerformanceContract,
    PgoAttr,
    PrefetchAccess,
    PrefetchAttr,
    Profile,
    ProfileAttr,
    ReduceAttr,
    ReductionOp,
    Repr,
    ReprAttr,
    SpecializeAttr,
    StdAttr,
    SymbolVisibility,
    TaggedLiteralAttr,
    TargetCpuAttr,
    TargetFeatureAttr,
    // Context system attributes: context transforms (e.g., .transactional(), .scoped())
    TransformAttr,
    // Dependency injection attributes: @injectable(Scope) for static DI, @inject for field injection
    InjectableAttr,
    InjectAttr,
    InjectionScope,
    UnrollAttr,
    UnrollMode,
    UsedAttr,
    VectorizeAttr,
    VectorizeMode,
    VerificationMode,
    VerifyAttr,
    VisibilityAttr,
    // Linker control attributes (Phase 6)
    AliasAttr,
    WeakAttr,
    LinkageAttr,
    LinkageKind,
    InitPriorityAttr,
    SectionAttr,
    ExportAttr,
    // Additional linker attributes
    NakedAttr,
    LinkNameAttr,
    NoReturnAttr,
    NoMangleAttr,
    // LLVM-only execution attribute
    LlvmOnlyAttr,
    // Formal verification attributes
    GhostAttr,
    RequiresAttr,
    EnsuresAttr,
    InvariantAttr,
    DecreasesAttr,
};

// Conversion helpers and additional attribute types
pub use conversion::{
    // Additional attribute wrappers for variant attributes
    AssumeAlignedAttr,
    AssumeNoAliasAttr,
    AssumeNoOverflowAttr,
    BranchProbabilityAttr,
    ConstFoldAttr,
    ConstPropAttr,
    ConstantTimeAttr,
    FrequencyAttr,
    MaxMemoryAttr,
    MaxTimeAttr,
    NoLtoAttr,
    NoUnrollAttr,
    NoVectorizeAttr,
    SimdAttr,
    UnlikelyAttr,
    // Helper functions for extracting values from Expr
    extract_bool,
    extract_float,
    extract_ident,
    extract_ident_list,
    extract_int,
    extract_named_arg,
    extract_string,
    extract_string_list,
    extract_u32,
    extract_u64,
};

// =============================================================================
// HELPER TRAITS
// =============================================================================

use verum_common::{List, Maybe, Text};

/// Trait for converting a generic `Attribute` to a typed attribute struct.
///
/// This trait enables type-safe extraction of attribute data from the AST.
///
/// # Examples
///
/// ```rust
/// use verum_ast::attr::{Attribute, InlineAttr, FromAttribute};
///
/// fn process_inline(attr: &Attribute) {
///     if let Ok(inline) = InlineAttr::from_attribute(attr) {
///         println!("Inline mode: {:?}", inline.mode);
///     }
/// }
/// ```
pub trait FromAttribute: Sized {
    /// The attribute name (without `@` prefix)
    const NAME: &'static str;

    /// Convert from a generic `Attribute` to this typed struct.
    ///
    /// # Errors
    ///
    /// Returns an error if the attribute name doesn't match or
    /// the arguments are invalid.
    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError>;
}

/// Error during attribute conversion.
#[derive(Debug, Clone)]
pub struct AttributeConversionError {
    /// Error message
    pub message: Text,
    /// Expected attribute name
    pub expected: Text,
    /// Actual attribute name
    pub actual: Text,
    /// Source location
    pub span: crate::span::Span,
}

impl AttributeConversionError {
    /// Create a "wrong name" error.
    #[must_use]
    pub fn wrong_name(expected: &str, actual: &str, span: crate::span::Span) -> Self {
        Self {
            message: Text::from(format!("expected @{} attribute, got @{}", expected, actual)),
            expected: Text::from(expected),
            actual: Text::from(actual),
            span,
        }
    }

    /// Create an "invalid arguments" error.
    #[must_use]
    pub fn invalid_args(message: impl Into<Text>, span: crate::span::Span) -> Self {
        Self {
            message: message.into(),
            expected: Text::new(),
            actual: Text::new(),
            span,
        }
    }
}

impl std::fmt::Display for AttributeConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AttributeConversionError {}

// =============================================================================
// COLLECTION HELPERS
// =============================================================================

/// Extension trait for working with attribute collections.
pub trait AttributeListExt {
    /// Find an attribute by name.
    fn find_by_name(&self, name: &str) -> Maybe<&Attribute>;

    /// Check if an attribute with the given name exists.
    fn has_attribute(&self, name: &str) -> bool;

    /// Get all attribute names.
    fn names(&self) -> List<&Text>;

    /// Find and convert a typed attribute.
    fn find_typed<T: FromAttribute>(&self) -> Maybe<Result<T, AttributeConversionError>>;

    /// Get all attributes with a specific name (for repeatable attrs).
    fn find_all_by_name(&self, name: &str) -> List<&Attribute>;
}

impl AttributeListExt for [Attribute] {
    fn find_by_name(&self, name: &str) -> Maybe<&Attribute> {
        self.iter().find(|a| a.name.as_str() == name)
    }

    fn has_attribute(&self, name: &str) -> bool {
        self.iter().any(|a| a.name.as_str() == name)
    }

    fn names(&self) -> List<&Text> {
        self.iter().map(|a| &a.name).collect()
    }

    fn find_typed<T: FromAttribute>(&self) -> Maybe<Result<T, AttributeConversionError>> {
        self.find_by_name(T::NAME)
            .map(|attr| T::from_attribute(attr))
    }

    fn find_all_by_name(&self, name: &str) -> List<&Attribute> {
        self.iter().filter(|a| a.name.as_str() == name).collect()
    }
}

impl AttributeListExt for List<Attribute> {
    fn find_by_name(&self, name: &str) -> Maybe<&Attribute> {
        self.iter().find(|a| a.name.as_str() == name)
    }

    fn has_attribute(&self, name: &str) -> bool {
        self.iter().any(|a| a.name.as_str() == name)
    }

    fn names(&self) -> List<&Text> {
        self.iter().map(|a| &a.name).collect()
    }

    fn find_typed<T: FromAttribute>(&self) -> Maybe<Result<T, AttributeConversionError>> {
        self.find_by_name(T::NAME)
            .map(|attr| T::from_attribute(attr))
    }

    fn find_all_by_name(&self, name: &str) -> List<&Attribute> {
        self.iter().filter(|a| a.name.as_str() == name).collect()
    }
}

// =============================================================================
// PRELUDE
// =============================================================================

/// Common imports for working with attributes.
///
/// ```rust
/// use verum_ast::attr::prelude::*;
/// ```
pub mod prelude {
    pub use super::{
        ArgSpec, ArgType, Attribute, AttributeCategory, AttributeConversionError, AttributeListExt,
        AttributeMetadata, AttributeTarget, FromAttribute, NamedArgSpec, Stability,
    };
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    #[test]
    fn test_attribute_list_ext() {
        let attrs = [Attribute::simple(Text::from("inline"), Span::default()),
            Attribute::simple(Text::from("cold"), Span::default()),
            Attribute::simple(Text::from("doc"), Span::default())];

        assert!(attrs.has_attribute("inline"));
        assert!(attrs.has_attribute("cold"));
        assert!(!attrs.has_attribute("hot"));

        assert!(attrs.find_by_name("inline").is_some());
        assert!(attrs.find_by_name("unknown").is_none());

        let names = attrs.names();
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn test_attribute_target_reexport() {
        // Verify that AttributeTarget is properly exported
        let target = AttributeTarget::Function | AttributeTarget::Type;
        assert!(target.contains(AttributeTarget::Function));
    }

    #[test]
    fn test_metadata_reexport() {
        // Verify that AttributeMetadata builder works
        let meta = AttributeMetadata::new("test")
            .targets(AttributeTarget::Function)
            .category(AttributeCategory::Optimization)
            .build();

        assert_eq!(meta.name.as_str(), "test");
    }
}
