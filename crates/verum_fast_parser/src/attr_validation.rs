//! Attribute validation integration for the parser.
//!
//! This module provides infrastructure for validating attributes during parsing.
//! It is designed to work without creating circular dependencies by using a
//! trait-based abstraction that can be implemented by higher-level crates.
//!
//! # Design
//!
//! The validation system uses a trait-based approach:
//!
//! 1. [`AttributeValidatorTrait`] - The trait that validators must implement
//! 2. [`AttributeValidator`] - A concrete validator that can be configured
//! 3. [`ValidationConfig`] - Configuration for validation behavior
//!
//! By default, the parser uses a permissive validator that allows all attributes.
//! Higher-level crates (like verum_compiler) can provide a strict validator that
//! uses the attribute registry from verum_types.
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_parser::{RecursiveParser, attr_validation::ValidationConfig};
//!
//! // Create parser with validation enabled
//! let mut parser = RecursiveParser::with_attr_validation(&tokens, file_id);
//!
//! // Or enable it later
//! let mut parser = RecursiveParser::new(&tokens, file_id);
//! parser.enable_attr_validation();
//!
//! // Parse and get warnings
//! let module = parser.parse_module()?;
//! let warnings = parser.take_attr_warnings();
//! ```

use verum_ast::Span;
use verum_ast::attr::{Attribute, AttributeTarget};
use verum_common::{List, Text};

/// Configuration for attribute validation.
#[derive(Debug, Clone)]
pub struct ValidationConfig {
    /// Whether to validate attributes at all
    pub enabled: bool,

    /// Whether to allow unknown attributes (with warning)
    pub allow_unknown: bool,

    /// Whether to emit warnings for unknown attributes
    pub warn_unknown: bool,

    /// Whether to emit warnings for deprecated attributes
    pub warn_deprecated: bool,

    /// Whether to emit warnings for unstable features
    pub warn_unstable: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_unknown: true,
            warn_unknown: true,
            warn_deprecated: true,
            warn_unstable: true,
        }
    }
}

impl ValidationConfig {
    /// Create a configuration that disables all validation.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Create a strict configuration that reports all warnings.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            enabled: true,
            allow_unknown: false,
            warn_unknown: true,
            warn_deprecated: true,
            warn_unstable: true,
        }
    }

    /// Create a lenient configuration that only reports critical issues.
    #[must_use]
    pub fn lenient() -> Self {
        Self {
            enabled: true,
            allow_unknown: true,
            warn_unknown: false,
            warn_deprecated: false,
            warn_unstable: false,
        }
    }
}

/// A warning produced during attribute validation.
#[derive(Debug, Clone)]
pub struct AttributeValidationWarning {
    /// The warning message
    pub message: Text,
    /// The span where the issue was found
    pub span: Span,
    /// An optional hint for fixing the issue
    pub hint: Option<Text>,
    /// Error code for reference
    pub code: Text,
}

impl AttributeValidationWarning {
    /// Create a new warning.
    #[must_use]
    pub fn new(message: impl Into<Text>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            hint: None,
            code: Text::from("W0400"),
        }
    }

    /// Add a hint to the warning.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<Text>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Set the error code.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<Text>) -> Self {
        self.code = code.into();
        self
    }
}

impl std::fmt::Display for AttributeValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, " (hint: {})", hint)?;
        }
        Ok(())
    }
}

/// Trait for attribute validation.
///
/// This trait allows external crates to provide custom validation logic
/// without creating circular dependencies. The parser uses this trait
/// to validate attributes during parsing.
pub trait AttributeValidatorTrait: Send + Sync {
    /// Validate attributes for a specific target.
    ///
    /// Returns a list of validation warnings. Validation should not fail
    /// hard - instead, issues should be returned as warnings for backward
    /// compatibility.
    fn validate(
        &self,
        attrs: &[Attribute],
        target: AttributeTarget,
    ) -> List<AttributeValidationWarning>;

    /// Check if validation is enabled.
    fn is_enabled(&self) -> bool;
}

/// Default attribute validator for the parser.
///
/// This validator can be configured to either:
/// - Allow all attributes (default, for backward compatibility)
/// - Validate against a basic built-in set of known attributes
///
/// For full validation with the attribute registry, use the validator
/// from verum_types (via the compiler).
#[derive(Debug, Clone)]
pub struct AttributeValidator {
    config: ValidationConfig,
}

impl Default for AttributeValidator {
    fn default() -> Self {
        Self::new(ValidationConfig::default())
    }
}

impl AttributeValidator {
    /// Create a new validator with the given configuration.
    #[must_use]
    pub fn new(config: ValidationConfig) -> Self {
        Self { config }
    }

    /// Check if validation is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Read mirror of `ValidationConfig.allow_unknown`. Surfaced
    /// for orchestrators that compose the validator with their
    /// own attribute-extension registry — they consult this to
    /// decide whether to register additional names before
    /// validation runs.
    #[must_use]
    pub fn allow_unknown(&self) -> bool {
        self.config.allow_unknown
    }

    /// Read mirror of `ValidationConfig.warn_unknown`. Mirrors
    /// the verbosity stance for diagnostics that re-summarise
    /// validator output.
    #[must_use]
    pub fn warn_unknown(&self) -> bool {
        self.config.warn_unknown
    }

    /// Read mirror of `ValidationConfig.warn_deprecated`.
    /// Surfaced for the deprecated-attribute extension hook
    /// that lives in downstream orchestrators (the validator
    /// itself doesn't currently maintain a deprecated-attribute
    /// list — the hook is the read surface that lets a future
    /// extension consult the configured stance).
    #[must_use]
    pub fn warn_deprecated(&self) -> bool {
        self.config.warn_deprecated
    }

    /// Read mirror of `ValidationConfig.warn_unstable`. Same
    /// extension-hook contract as `warn_deprecated`.
    #[must_use]
    pub fn warn_unstable(&self) -> bool {
        self.config.warn_unstable
    }

    /// Validate attributes for a function declaration.
    #[must_use]
    pub fn validate_function_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Function)
    }

    /// Validate attributes for a type declaration.
    #[must_use]
    pub fn validate_type_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Type)
    }

    /// Validate attributes for a field declaration.
    #[must_use]
    pub fn validate_field_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Field)
    }

    /// Validate attributes for a match arm.
    #[must_use]
    pub fn validate_match_arm_attrs(
        &self,
        attrs: &[Attribute],
    ) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::MatchArm)
    }

    /// Validate attributes for a function parameter.
    #[must_use]
    pub fn validate_param_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Param)
    }

    /// Validate attributes for an impl block.
    #[must_use]
    pub fn validate_impl_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Impl)
    }

    /// Validate attributes for a module.
    #[must_use]
    pub fn validate_module_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Module)
    }

    /// Validate attributes for a protocol (trait).
    #[must_use]
    pub fn validate_protocol_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Protocol)
    }

    /// Validate attributes for a variant.
    #[must_use]
    pub fn validate_variant_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Variant)
    }

    /// Validate attributes for a statement.
    #[must_use]
    pub fn validate_stmt_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Stmt)
    }

    /// Validate attributes for an expression.
    #[must_use]
    pub fn validate_expr_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Expr)
    }

    /// Validate attributes for a loop.
    #[must_use]
    pub fn validate_loop_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Loop)
    }

    /// Validate attributes for a static declaration.
    #[must_use]
    pub fn validate_static_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Static)
    }

    /// Validate attributes for a const declaration.
    #[must_use]
    pub fn validate_const_attrs(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, AttributeTarget::Const)
    }

    /// Core validation logic for any target.
    ///
    /// This method provides basic validation against known attribute targets.
    /// For full validation, use the registry-based validator from verum_types.
    pub fn validate_attrs(
        &self,
        attrs: &[Attribute],
        target: AttributeTarget,
    ) -> List<AttributeValidationWarning> {
        if !self.config.enabled || attrs.is_empty() {
            return List::new();
        }

        let mut warnings = List::new();

        // Basic validation: check for obviously wrong attribute targets
        // This is a simplified version - full validation is done by verum_types
        for attr in attrs {
            if let Some(warning) = self.validate_single_attr(attr, target) {
                warnings.push(warning);
            }
        }

        // Check for conflicting attributes
        warnings.extend(self.check_conflicts(attrs));

        // Check for duplicates (non-repeatable attributes)
        warnings.extend(self.check_duplicates(attrs));

        warnings
    }

    /// Validate a single attribute against its target.
    fn validate_single_attr(
        &self,
        attr: &Attribute,
        target: AttributeTarget,
    ) -> Option<AttributeValidationWarning> {
        let name = attr.name.as_str();

        // Quick check for known attribute-target combinations
        // This is not exhaustive - full validation is done by verum_types registry
        let valid = match name {
            // Function-only attributes
            // NOTE: `pure`, `async`, `unsafe`, `meta` are MODIFIERS (keywords), not attributes
            // Correct: `pure fn foo()`, NOT `@pure fn foo()`
            "inline"
            | "cold"
            | "hot"
            | "test"
            | "bench"
            | "export"
            | "calling_convention"
            | "no_mangle"
            | "const_eval"
            | "tagged_literal"
            | "differentiable"
            | "intrinsic" // @intrinsic("name") for intrinsic function declarations
            | "extern"    // @extern("C") for FFI function declarations
            => target.contains(AttributeTarget::Function),

            // Type-only attributes (per grammar/verum.ebnf)
            // @derive(...) - derive_attribute in grammar
            // @repr(...) - common attribute for type layout
            // @sealed - prevents external implementations
            "derive" | "repr" | "sealed" => target.contains(AttributeTarget::Type),

            // Field-only attributes
            "skip_serialize" | "skip_deserialize" | "flatten" => {
                target.contains(AttributeTarget::Field)
            }

            // Bitfield attributes for hardware register layouts and protocol headers
            // @bits(N) - specifies bit width for a field in a bitfield type
            // @offset(N) - specifies explicit bit offset for a field
            "bits" | "offset" => target.contains(AttributeTarget::Field),

            // @bitfield - marks a type as a bitfield (layout calculated from @bits fields)
            // @endian(big|little|native) - specifies byte order for multi-byte bitfields
            "bitfield" => target.contains(AttributeTarget::Type),
            "endian" => {
                target.contains(AttributeTarget::Type) || target.contains(AttributeTarget::Field)
            }

            // Impl-only attributes (per grammar/verum.ebnf)
            // @specialize - specialize_attribute in grammar
            // NOTE: `unsafe` before `implement` is a MODIFIER, not attribute
            // Correct: `unsafe implement Foo`, NOT `@unsafe implement Foo`
            "specialize" => target.contains(AttributeTarget::Impl),

            // Protocol-only attributes
            "marker" | "auto" => target.contains(AttributeTarget::Protocol),

            // Module-only attributes
            "profile" | "feature" | "no_implicit_prelude" | "link" => {
                target.contains(AttributeTarget::Module)
            }

            // Loop-only attributes
            "unroll" | "no_unroll" | "ivdep" | "parallel" => target.contains(AttributeTarget::Loop),

            // Static-only attributes
            "thread_local" | "used" => {
                target.contains(AttributeTarget::Static) || target.contains(AttributeTarget::Const)
            }

            // Broad applicability attributes (can be used on almost any target)
            // @cfg is for conditional compilation and can appear on functions, types, impls, statements, etc.
            "doc" | "deprecated" | "experimental" | "todo" | "allow" | "warn" | "deny" | "cfg" => {
                true
            }

            // Universe polymorphism attribute
            // @universe_poly marks a fn/type declaration as universe-polymorphic.
            // It signals that the declaration uses universe level parameters (introduced
            // via `universe u` or `u: Level` in generic param lists).
            // Spec: verum-ext.md §2.1 - Universe Polymorphism
            "universe_poly" => {
                target.contains(AttributeTarget::Function) || target.contains(AttributeTarget::Type)
            }

            // Multi-target attributes
            "verify" | "must_use" => {
                target.contains(AttributeTarget::Function) || target.contains(AttributeTarget::Type)
            }
            "vectorize" | "simd" | "no_vectorize" => {
                target.contains(AttributeTarget::Loop) || target.contains(AttributeTarget::Function)
            }
            "optimize" | "target_cpu" => {
                target.contains(AttributeTarget::Function)
                    || target.contains(AttributeTarget::Module)
            }
            "align" => {
                target.contains(AttributeTarget::Type) || target.contains(AttributeTarget::Field)
            }
            "validate" | "custom" => {
                target.contains(AttributeTarget::Type) || target.contains(AttributeTarget::Field)
            }
            "serialize" | "deserialize" => {
                target.contains(AttributeTarget::Type)
                    || target.contains(AttributeTarget::Field)
                    || target.contains(AttributeTarget::Variant)
            }
            "rename" | "default" => {
                target.contains(AttributeTarget::Field) || target.contains(AttributeTarget::Variant)
            }
            "likely" | "unlikely" => {
                target.contains(AttributeTarget::Expr) || target.contains(AttributeTarget::MatchArm)
            }
            "unreachable" => {
                target.contains(AttributeTarget::MatchArm) || target.contains(AttributeTarget::Expr)
            }
            "no_alias" => {
                target.contains(AttributeTarget::Loop)
                    || target.contains(AttributeTarget::Function)
                    || target.contains(AttributeTarget::Param)
            }
            "unused" => {
                target.contains(AttributeTarget::Param)
                    || target.contains(AttributeTarget::Field)
                    || target.contains(AttributeTarget::Item)
            }
            "prefetch" | "assume" => {
                target.contains(AttributeTarget::Stmt) || target.contains(AttributeTarget::Expr)
            }
            "deadlock_detection" => {
                target.contains(AttributeTarget::Function)
                    || target.contains(AttributeTarget::Module)
            }
            "packed" => target.contains(AttributeTarget::Type),
            "std" => target.contains(AttributeTarget::Function),
            "ignore" | "should_panic" => target.contains(AttributeTarget::Function),
            "range" | "length" | "pattern" => target.contains(AttributeTarget::Field),
            "target_feature" => target.contains(AttributeTarget::Function),
            "black_box" => target.contains(AttributeTarget::Expr),
            "optimize_barrier" => target.contains(AttributeTarget::Stmt),

            // Unknown attributes — behaviour gated on
            // `allow_unknown` and `warn_unknown`:
            //
            //   * `allow_unknown == true`  + `warn_unknown` → W0400 (warn, accept).
            //   * `allow_unknown == true`  + !warn_unknown  → silently accept.
            //   * `allow_unknown == false`                  → W0402 (reject as
            //     "unknown attribute, no extension installed"); the
            //     attribute fails validation regardless of the warn
            //     gate so strict mode (`disabled()` /
            //     `strict()` constructors) actually rejects.
            //
            // Before this wire-up `allow_unknown` was inert — every
            // unknown attribute fell through to the warn-or-pass
            // path regardless of the configured stance.
            _ => {
                if !self.config.allow_unknown {
                    return Some(
                        AttributeValidationWarning::new(
                            format!(
                                "unknown attribute `@{}` rejected by strict mode \
                                 (`allow_unknown = false`)",
                                name
                            ),
                            attr.span,
                        )
                        .with_code("W0402"),
                    );
                }
                if self.config.warn_unknown {
                    return Some(
                        AttributeValidationWarning::new(
                            format!("unknown attribute `@{}`", name),
                            attr.span,
                        )
                        .with_code("W0400"),
                    );
                }
                true
            }
        };

        if !valid {
            Some(
                AttributeValidationWarning::new(
                    format!("`@{}` is not valid on {}", name, target.display_name()),
                    attr.span,
                )
                .with_code("W0401"),
            )
        } else {
            None
        }
    }

    /// Check for conflicting attributes.
    fn check_conflicts(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        let mut warnings = List::new();

        // Define conflicting pairs
        let conflicts = [
            ("cold", "hot"),
            ("cold", "inline"),
            ("likely", "unlikely"),
            ("vectorize", "no_vectorize"),
            ("unroll", "no_unroll"),
        ];

        let attr_names: Vec<&str> = attrs.iter().map(|a| a.name.as_str()).collect();

        for (a, b) in conflicts {
            if attr_names.contains(&a) && attr_names.contains(&b) {
                // Find the span of the second attribute
                if let Some(attr) = attrs.iter().find(|x| x.name.as_str() == b) {
                    warnings.push(
                        AttributeValidationWarning::new(
                            format!("`@{}` conflicts with `@{}`", b, a),
                            attr.span,
                        )
                        .with_code("W0402")
                        .with_hint("remove one of the conflicting attributes"),
                    );
                }
            }
        }

        warnings
    }

    /// Check for duplicate non-repeatable attributes.
    fn check_duplicates(&self, attrs: &[Attribute]) -> List<AttributeValidationWarning> {
        let mut warnings = List::new();

        // Attributes that should not be repeated
        let non_repeatable = [
            "inline",
            "cold",
            "hot",
            "repr",
            "derive",
            "verify",
            "specialize",
            "align",
            "packed",
            "optimize",
            "test",
            "bench",
            "ignore",
            "profile",
            "feature",
        ];

        let mut seen: std::collections::HashMap<&str, &Attribute> =
            std::collections::HashMap::new();

        for attr in attrs {
            let name = attr.name.as_str();
            if non_repeatable.contains(&name) {
                if let Some(first) = seen.get(name) {
                    warnings.push(
                        AttributeValidationWarning::new(
                            format!("`@{}` can only appear once", name),
                            attr.span,
                        )
                        .with_code("W0403")
                        .with_hint(format!("first occurrence at line {}", first.span.start)),
                    );
                } else {
                    seen.insert(name, attr);
                }
            }
        }

        warnings
    }
}

impl AttributeValidatorTrait for AttributeValidator {
    fn validate(
        &self,
        attrs: &[Attribute],
        target: AttributeTarget,
    ) -> List<AttributeValidationWarning> {
        self.validate_attrs(attrs, target)
    }

    fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

/// Convenience function to validate attributes with default configuration.
///
/// This is useful for quick validation without creating a validator instance.
#[must_use]
pub fn validate_parsed_attributes(
    attrs: &[Attribute],
    target: AttributeTarget,
) -> List<AttributeValidationWarning> {
    AttributeValidator::default().validate_attrs(attrs, target)
}

/// Convenience function to validate function attributes.
#[must_use]
pub fn validate_function_attributes(attrs: &[Attribute]) -> List<AttributeValidationWarning> {
    AttributeValidator::default().validate_function_attrs(attrs)
}

/// Convenience function to validate type attributes.
#[must_use]
pub fn validate_type_attributes(attrs: &[Attribute]) -> List<AttributeValidationWarning> {
    AttributeValidator::default().validate_type_attrs(attrs)
}

/// Convenience function to validate field attributes.
#[must_use]
pub fn validate_field_attributes(attrs: &[Attribute]) -> List<AttributeValidationWarning> {
    AttributeValidator::default().validate_field_attrs(attrs)
}

/// Convenience function to validate match arm attributes.
#[must_use]
pub fn validate_match_arm_attributes(attrs: &[Attribute]) -> List<AttributeValidationWarning> {
    AttributeValidator::default().validate_match_arm_attrs(attrs)
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_attr(name: &str, span: Span) -> Attribute {
        Attribute::simple(Text::from(name), span)
    }

    #[test]
    fn test_validation_disabled() {
        let config = ValidationConfig::disabled();
        let validator = AttributeValidator::new(config);

        let attrs = vec![make_attr("unknown_attr", Span::default())];
        let warnings = validator.validate_function_attrs(&attrs);

        assert!(
            warnings.is_empty(),
            "disabled validator should not produce warnings"
        );
    }

    #[test]
    fn test_validate_known_attribute() {
        let config = ValidationConfig::default();
        let validator = AttributeValidator::new(config);

        let attrs = vec![make_attr("inline", Span::default())];
        let warnings = validator.validate_function_attrs(&attrs);

        // @inline on function is valid, should produce no warnings
        assert!(
            warnings.is_empty(),
            "valid attribute should not produce warnings, got: {:?}",
            warnings
                .iter()
                .map(|w| w.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_validate_unknown_attribute() {
        let mut config = ValidationConfig::default();
        config.allow_unknown = true;
        config.warn_unknown = true;

        let validator = AttributeValidator::new(config);

        let attrs = vec![make_attr("totally_unknown_attribute_xyz", Span::default())];
        let warnings = validator.validate_function_attrs(&attrs);

        assert_eq!(
            warnings.len(),
            1,
            "should produce one warning for unknown attribute"
        );
        assert!(
            warnings[0].message.as_str().contains("unknown"),
            "warning message should mention 'unknown'"
        );
    }

    #[test]
    fn test_validate_wrong_target() {
        let config = ValidationConfig::strict();
        let validator = AttributeValidator::new(config);

        // @inline is only valid on functions, not fields
        let attrs = vec![make_attr("inline", Span::default())];
        let warnings = validator.validate_field_attrs(&attrs);

        assert!(
            !warnings.is_empty(),
            "wrong target should produce a warning"
        );
        assert!(
            warnings[0].message.as_str().contains("not valid"),
            "warning should mention invalid target"
        );
    }

    #[test]
    fn test_validate_conflicting_attributes() {
        let config = ValidationConfig::strict();
        let validator = AttributeValidator::new(config);

        // @cold and @hot conflict
        let attrs = vec![
            make_attr("cold", Span::default()),
            make_attr("hot", Span::default()),
        ];
        let warnings = validator.validate_function_attrs(&attrs);

        assert!(
            !warnings.is_empty(),
            "conflicting attributes should produce warnings"
        );
        let has_conflict_warning = warnings
            .iter()
            .any(|w| w.message.as_str().contains("conflicts"));
        assert!(has_conflict_warning, "should have a conflict warning");
    }

    #[test]
    fn test_validate_duplicate_attributes() {
        let config = ValidationConfig::strict();
        let validator = AttributeValidator::new(config);

        let attrs = vec![
            make_attr("inline", Span::default()),
            make_attr("inline", Span::default()),
        ];
        let warnings = validator.validate_function_attrs(&attrs);

        assert!(
            !warnings.is_empty(),
            "duplicate attributes should produce warnings"
        );
        let has_duplicate_warning = warnings
            .iter()
            .any(|w| w.message.as_str().contains("only appear once"));
        assert!(has_duplicate_warning, "should have a duplicate warning");
    }

    #[test]
    fn test_lenient_config() {
        let config = ValidationConfig::lenient();
        let validator = AttributeValidator::new(config);

        // Unknown attribute with lenient config should not warn
        let attrs = vec![make_attr("custom_user_attribute", Span::default())];
        let warnings = validator.validate_function_attrs(&attrs);

        assert!(
            warnings.is_empty(),
            "lenient config should not warn about unknown attributes"
        );
    }

    #[test]
    fn allow_unknown_false_rejects_unknown_attribute_with_w0402() {
        // Pin: `allow_unknown = false` rejects unknown attributes
        // outright (W0402), not just warns. Before the wire-up
        // the field was inert — every unknown attribute fell
        // through to the warn-or-pass path regardless of the
        // configured stance.
        let mut config = ValidationConfig::default();
        config.allow_unknown = false;
        // Set warn_unknown false too so we can be sure the
        // returned warning came from the strict-mode rejection,
        // not the warn path.
        config.warn_unknown = false;
        let validator = AttributeValidator::new(config);

        let attrs = vec![make_attr("totally_unknown_attribute_xyz", Span::default())];
        let warnings = validator.validate_function_attrs(&attrs);

        assert_eq!(
            warnings.len(),
            1,
            "strict mode must produce exactly one rejection"
        );
        assert_eq!(
            warnings[0].code.as_str(),
            "W0402",
            "rejection must use the W0402 code: {:?}",
            warnings[0]
        );
        assert!(
            warnings[0]
                .message
                .as_str()
                .contains("strict mode"),
            "rejection message must name the strict-mode gate: {}",
            warnings[0].message
        );
    }

    #[test]
    fn config_accessors_mirror_constructed_values() {
        // Pin: every accessor on AttributeValidator returns the
        // configured value. Before the wire-up, four config
        // fields (allow_unknown, warn_unknown, warn_deprecated,
        // warn_unstable) had no public read surface — external
        // orchestrators that wanted to drive the validator with
        // a custom extension registry couldn't observe its
        // configured stance.
        for &allow in &[true, false] {
            for &warn_u in &[true, false] {
                for &warn_d in &[true, false] {
                    for &warn_uns in &[true, false] {
                        let config = ValidationConfig {
                            enabled: true,
                            allow_unknown: allow,
                            warn_unknown: warn_u,
                            warn_deprecated: warn_d,
                            warn_unstable: warn_uns,
                        };
                        let v = AttributeValidator::new(config);
                        assert_eq!(v.allow_unknown(), allow);
                        assert_eq!(v.warn_unknown(), warn_u);
                        assert_eq!(v.warn_deprecated(), warn_d);
                        assert_eq!(v.warn_unstable(), warn_uns);
                    }
                }
            }
        }
    }

    #[test]
    fn test_validate_parsed_attributes_function() {
        let attrs = vec![make_attr("test", Span::default())];
        let warnings = validate_function_attributes(&attrs);

        // @test on function is valid
        assert!(
            warnings.is_empty(),
            "@test on function should be valid, got: {:?}",
            warnings
                .iter()
                .map(|w| w.message.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_warning_display() {
        let warning = AttributeValidationWarning::new("test warning", Span::default())
            .with_code("W0100")
            .with_hint("fix it");

        let display = format!("{}", warning);
        assert!(display.contains("W0100"));
        assert!(display.contains("test warning"));
        assert!(display.contains("fix it"));
    }
}
