//! Attribute argument specifications for the Verum AST.
//!
//! This module defines the type system for attribute arguments, enabling
//! compile-time validation of attribute syntax.
//!
//! # Overview
//!
//! Verum attributes can have various argument forms:
//!
//! ```verum
//! @cold                                    // No arguments
//! @inline(always)                          // Single positional argument
//! @align(16)                               // Single integer argument
//! @serialize(rename = "user_id")           // Named argument
//! @validate(min = 1, max = 100)            // Multiple named arguments
//! @derive(Clone, Serialize, Debug)         // Variadic arguments
//! @deprecated(since = "2.0", use = "new")  // Multiple named strings
//! ```
//!
//! # Design
//!
//! - [`ArgSpec`]: Specification for what arguments an attribute accepts
//! - [`ArgType`]: Type of a single argument value
//! - [`NamedArgSpec`]: Specification for a named argument
//!
//! # Argument Validation
//!
//! Attribute argument specs define what arguments an attribute accepts: none (@cold),
//! optional (@inline or @inline(always)), required (@align(16)), named keyword-style
//! (@serialize(rename = "x")), variadic (@derive(Clone, Serialize)), or mixed.
//! The compiler validates arguments against the registered spec at parse time.

use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};

use crate::expr::Expr;
use crate::span::Span;

/// Specification for attribute arguments.
///
/// Defines what arguments an attribute accepts and their structure.
///
/// # Examples
///
/// ```rust
/// use verum_ast::attr::{ArgSpec, ArgType, NamedArgSpec};
///
/// // @cold - no arguments
/// let no_args = ArgSpec::None;
///
/// // @inline or @inline(always)
/// let optional = ArgSpec::Optional(ArgType::Ident);
///
/// // @align(16) - required integer
/// let required = ArgSpec::Required(ArgType::Int);
///
/// // @derive(Clone, Serialize) - variadic identifiers
/// let variadic = ArgSpec::Variadic(ArgType::Ident);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum ArgSpec {
    /// No arguments allowed: `@cold`, `@hot`, `@packed`
    #[default]
    None,

    /// Optional single argument: `@inline` or `@inline(always)`
    Optional(ArgType),

    /// Required single argument: `@align(16)`, `@pattern("regex")`
    Required(ArgType),

    /// Named arguments (keyword style): `@serialize(rename = "x", skip = true)`
    ///
    /// Named arguments can be required or optional with defaults.
    Named(List<NamedArgSpec>),

    /// Variadic positional arguments: `@derive(Clone, Serialize, Debug)`
    ///
    /// All arguments must be of the same type.
    Variadic(ArgType),

    /// Mixed: optional first positional, then named: `@verify(static, timeout = 5s)`
    Mixed {
        /// Optional first positional argument
        positional: Maybe<ArgType>,
        /// Named arguments
        named: List<NamedArgSpec>,
    },

    /// Either a single positional OR named arguments: `@inline(always)` or `@inline(mode = always)`
    Either {
        positional: ArgType,
        named: List<NamedArgSpec>,
    },

    /// Custom validation with a reason string (for complex cases)
    Custom {
        /// Description for error messages
        description: Text,
    },
}

impl ArgSpec {
    /// Create a spec for no arguments.
    #[must_use]
    pub const fn none() -> Self {
        Self::None
    }

    /// Create a spec for an optional single argument.
    #[must_use]
    pub const fn optional(ty: ArgType) -> Self {
        Self::Optional(ty)
    }

    /// Create a spec for a required single argument.
    #[must_use]
    pub const fn required(ty: ArgType) -> Self {
        Self::Required(ty)
    }

    /// Create a spec for variadic arguments.
    #[must_use]
    pub const fn variadic(ty: ArgType) -> Self {
        Self::Variadic(ty)
    }

    /// Create a spec for named arguments.
    #[must_use]
    pub fn named(args: impl IntoIterator<Item = NamedArgSpec>) -> Self {
        Self::Named(args.into_iter().collect())
    }

    /// Check if this spec allows no arguments.
    #[must_use]
    pub fn allows_empty(&self) -> bool {
        match self {
            Self::None => true,
            Self::Optional(_) => true,
            Self::Required(_) => false,
            Self::Named(args) => args.iter().all(|a| !a.required),
            Self::Variadic(_) => true, // Empty list is valid
            Self::Mixed { positional, named } => {
                positional.is_none() && named.iter().all(|a| !a.required)
            }
            Self::Either { .. } => true, // Either form is optional
            Self::Custom { .. } => true, // Assume custom allows empty
        }
    }

    /// Get a description of expected arguments for error messages.
    #[must_use]
    pub fn description(&self) -> Text {
        match self {
            Self::None => Text::from("no arguments"),
            Self::Optional(ty) => Text::from(format!("optional {} argument", ty.description())),
            Self::Required(ty) => Text::from(format!("required {} argument", ty.description())),
            Self::Named(args) => {
                let names: Vec<_> = args.iter().map(|a| a.name.as_str()).collect();
                if names.len() <= 3 {
                    Text::from(format!("named arguments: {}", names.join(", ")))
                } else {
                    Text::from(format!(
                        "named arguments: {}, ... ({} total)",
                        names[..3].join(", "),
                        names.len()
                    ))
                }
            }
            Self::Variadic(ty) => {
                Text::from(format!("zero or more {} arguments", ty.description()))
            }
            Self::Mixed {
                positional,
                named: _,
            } => {
                let pos_desc = positional
                    .as_ref()
                    .map(|t| t.description())
                    .unwrap_or("none");
                Text::from(format!(
                    "optional {} positional, then named arguments",
                    pos_desc
                ))
            }
            Self::Either { positional, named } => {
                let names: Vec<_> = named.iter().map(|a| a.name.as_str()).collect();
                Text::from(format!(
                    "either {} or named ({})",
                    positional.description(),
                    names.join(", ")
                ))
            }
            Self::Custom { description } => description.clone(),
        }
    }
}


/// Type of a single attribute argument value.
///
/// Used to validate argument values at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArgType {
    /// Identifier: `Clone`, `always`, `never`, `C`
    Ident,

    /// String literal: `"user_id"`, `"^[a-z]+$"`
    String,

    /// Integer literal: `16`, `32`, `1024`
    Int,

    /// Unsigned integer: must be non-negative
    UInt,

    /// Float literal: `0.95`, `1.5`
    Float,

    /// Boolean: `true`, `false`
    Bool,

    /// Any expression (for complex cases)
    Expr,

    /// Path expression: `std::io::FileSystem`, `Clone`
    Path,

    /// Type expression: `Int`, `List<T>`
    Type,

    /// Duration literal: `5s`, `100ms`, `1m`
    Duration,

    /// Size literal: `1KB`, `16MB`
    Size,
}

impl ArgType {
    /// Get a human-readable description of this type.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Ident => "identifier",
            Self::String => "string",
            Self::Int => "integer",
            Self::UInt => "unsigned integer",
            Self::Float => "number",
            Self::Bool => "boolean",
            Self::Expr => "expression",
            Self::Path => "path",
            Self::Type => "type",
            Self::Duration => "duration",
            Self::Size => "size",
        }
    }

    /// Check if an expression matches this expected type.
    ///
    /// This performs a shallow check based on expression structure,
    /// not full type inference.
    #[must_use]
    pub fn matches(&self, expr: &Expr) -> bool {
        use crate::expr::ExprKind;
        use crate::literal::LiteralKind;

        match self {
            Self::Ident => matches!(&expr.kind, ExprKind::Path(p) if p.segments.len() == 1),
            Self::String => matches!(
                &expr.kind,
                ExprKind::Literal(lit) if matches!(lit.kind, LiteralKind::Text(_))
            ),
            Self::Int => matches!(
                &expr.kind,
                ExprKind::Literal(lit) if matches!(lit.kind, LiteralKind::Int(_))
            ),
            Self::UInt => {
                if let ExprKind::Literal(lit) = &expr.kind {
                    if let LiteralKind::Int(int_lit) = &lit.kind {
                        return int_lit.value >= 0;
                    }
                }
                false
            }
            Self::Float => matches!(
                &expr.kind,
                ExprKind::Literal(lit) if matches!(lit.kind, LiteralKind::Float(_) | LiteralKind::Int(_))
            ),
            Self::Bool => matches!(
                &expr.kind,
                ExprKind::Literal(lit) if matches!(lit.kind, LiteralKind::Bool(_))
            ),
            Self::Path => matches!(&expr.kind, ExprKind::Path(_)),
            Self::Type => {
                // Types can be paths or call expressions (for generics like List<T>)
                matches!(&expr.kind, ExprKind::Path(_) | ExprKind::Call { .. })
            }
            Self::Expr => true, // Any expression matches
            Self::Duration | Self::Size => {
                // These are special literals, check for suffix
                // For now, accept any expression and validate later
                true
            }
        }
    }

    /// Get example values for documentation.
    #[must_use]
    pub const fn examples(&self) -> &'static [&'static str] {
        match self {
            Self::Ident => &["always", "never", "C", "Clone"],
            Self::String => &["\"name\"", "\"^[a-z]+$\""],
            Self::Int => &["16", "32", "-1"],
            Self::UInt => &["16", "32", "1024"],
            Self::Float => &["0.95", "1.5", "0.0"],
            Self::Bool => &["true", "false"],
            Self::Expr => &["x + 1", "data.len()"],
            Self::Path => &["Clone", "std::io::Read"],
            Self::Type => &["Int", "List<T>"],
            Self::Duration => &["5s", "100ms", "1m"],
            Self::Size => &["1KB", "16MB"],
        }
    }
}

impl std::fmt::Display for ArgType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Specification for a single named argument.
///
/// Used in [`ArgSpec::Named`] to define keyword-style arguments.
///
/// # Examples
///
/// ```rust
/// use verum_ast::attr::{NamedArgSpec, ArgType};
///
/// // @serialize(rename = "user_id")
/// let rename_arg = NamedArgSpec::optional("rename", ArgType::String);
///
/// // @validate(min = 1)  -- required
/// let min_arg = NamedArgSpec::required("min", ArgType::Int);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedArgSpec {
    /// Argument name (the key in `key = value`)
    pub name: Text,

    /// Expected value type
    pub ty: ArgType,

    /// Whether this argument is required
    pub required: bool,

    /// Default value expression (for optional arguments)
    ///
    /// If `None` and `required` is `false`, the argument is truly optional.
    pub default: Maybe<Text>,

    /// Short description for documentation
    pub doc: Maybe<Text>,

    /// Aliases for this argument name
    pub aliases: List<Text>,
}

impl NamedArgSpec {
    /// Create a required named argument specification.
    #[must_use]
    pub fn required(name: impl Into<Text>, ty: ArgType) -> Self {
        Self {
            name: name.into(),
            ty,
            required: true,
            default: Maybe::None,
            doc: Maybe::None,
            aliases: List::new(),
        }
    }

    /// Create an optional named argument specification.
    #[must_use]
    pub fn optional(name: impl Into<Text>, ty: ArgType) -> Self {
        Self {
            name: name.into(),
            ty,
            required: false,
            default: Maybe::None,
            doc: Maybe::None,
            aliases: List::new(),
        }
    }

    /// Create an optional named argument with a default value.
    #[must_use]
    pub fn with_default(name: impl Into<Text>, ty: ArgType, default: impl Into<Text>) -> Self {
        Self {
            name: name.into(),
            ty,
            required: false,
            default: Maybe::Some(default.into()),
            doc: Maybe::None,
            aliases: List::new(),
        }
    }

    /// Add documentation to this argument.
    #[must_use]
    pub fn doc(mut self, doc: impl Into<Text>) -> Self {
        self.doc = Maybe::Some(doc.into());
        self
    }

    /// Add an alias for this argument name.
    #[must_use]
    pub fn alias(mut self, alias: impl Into<Text>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    /// Check if a name matches this argument (including aliases).
    #[must_use]
    pub fn matches_name(&self, name: &str) -> bool {
        self.name.as_str() == name || self.aliases.iter().any(|a| a.as_str() == name)
    }
}

/// Result of validating attribute arguments against a specification.
#[derive(Debug, Clone)]
pub struct ArgValidationResult {
    /// Whether validation succeeded
    pub valid: bool,

    /// Validation errors (empty if valid)
    pub errors: List<ArgValidationError>,

    /// Warnings (non-fatal issues)
    pub warnings: List<ArgValidationWarning>,
}

impl ArgValidationResult {
    /// Create a successful validation result.
    #[must_use]
    pub fn ok() -> Self {
        Self {
            valid: true,
            errors: List::new(),
            warnings: List::new(),
        }
    }

    /// Create a failed validation result with a single error.
    #[must_use]
    pub fn error(error: ArgValidationError) -> Self {
        Self {
            valid: false,
            errors: vec![error].into(),
            warnings: List::new(),
        }
    }

    /// Create a failed validation result with multiple errors.
    #[must_use]
    pub fn errors(errors: impl IntoIterator<Item = ArgValidationError>) -> Self {
        let errors: List<_> = errors.into_iter().collect();
        Self {
            valid: errors.is_empty(),
            errors,
            warnings: List::new(),
        }
    }

    /// Add a warning to this result.
    #[must_use]
    pub fn with_warning(mut self, warning: ArgValidationWarning) -> Self {
        self.warnings.push(warning);
        self
    }

    /// Check if validation passed (no errors).
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.valid
    }

    /// Check if there are any warnings.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// An error in attribute argument validation.
#[derive(Debug, Clone)]
pub struct ArgValidationError {
    /// Error kind
    pub kind: ArgValidationErrorKind,

    /// Source location of the error
    pub span: Span,

    /// Additional context
    pub context: Maybe<Text>,
}

/// Kinds of argument validation errors.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgValidationErrorKind {
    /// Arguments provided when none expected
    UnexpectedArgs,

    /// Required argument missing
    MissingRequired { name: Text },

    /// Wrong argument type
    TypeMismatch { expected: ArgType, got: Text },

    /// Unknown named argument
    UnknownArg { name: Text, suggestions: List<Text> },

    /// Duplicate argument
    DuplicateArg { name: Text },

    /// Too few arguments
    TooFewArgs { min: usize, got: usize },

    /// Too many arguments
    TooManyArgs { max: usize, got: usize },

    /// Invalid value
    InvalidValue { message: Text },
}

impl ArgValidationError {
    /// Create an "unexpected arguments" error.
    #[must_use]
    pub fn unexpected_args(span: Span) -> Self {
        Self {
            kind: ArgValidationErrorKind::UnexpectedArgs,
            span,
            context: Maybe::None,
        }
    }

    /// Create a "missing required" error.
    #[must_use]
    pub fn missing_required(name: impl Into<Text>, span: Span) -> Self {
        Self {
            kind: ArgValidationErrorKind::MissingRequired { name: name.into() },
            span,
            context: Maybe::None,
        }
    }

    /// Create a "type mismatch" error.
    #[must_use]
    pub fn type_mismatch(expected: ArgType, got: impl Into<Text>, span: Span) -> Self {
        Self {
            kind: ArgValidationErrorKind::TypeMismatch {
                expected,
                got: got.into(),
            },
            span,
            context: Maybe::None,
        }
    }

    /// Create an "unknown argument" error.
    #[must_use]
    pub fn unknown_arg(name: impl Into<Text>, suggestions: List<Text>, span: Span) -> Self {
        Self {
            kind: ArgValidationErrorKind::UnknownArg {
                name: name.into(),
                suggestions,
            },
            span,
            context: Maybe::None,
        }
    }

    /// Get a human-readable error message.
    #[must_use]
    pub fn message(&self) -> Text {
        match &self.kind {
            ArgValidationErrorKind::UnexpectedArgs => {
                Text::from("attribute does not accept arguments")
            }
            ArgValidationErrorKind::MissingRequired { name } => {
                Text::from(format!("missing required argument `{}`", name))
            }
            ArgValidationErrorKind::TypeMismatch { expected, got } => {
                Text::from(format!("expected {}, got {}", expected.description(), got))
            }
            ArgValidationErrorKind::UnknownArg { name, suggestions } => {
                if suggestions.is_empty() {
                    Text::from(format!("unknown argument `{}`", name))
                } else {
                    Text::from(format!(
                        "unknown argument `{}`; did you mean {}?",
                        name,
                        suggestions
                            .iter()
                            .map(|s| format!("`{}`", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            ArgValidationErrorKind::DuplicateArg { name } => {
                Text::from(format!("argument `{}` specified multiple times", name))
            }
            ArgValidationErrorKind::TooFewArgs { min, got } => {
                Text::from(format!("expected at least {} arguments, got {}", min, got))
            }
            ArgValidationErrorKind::TooManyArgs { max, got } => {
                Text::from(format!("expected at most {} arguments, got {}", max, got))
            }
            ArgValidationErrorKind::InvalidValue { message } => message.clone(),
        }
    }
}

/// A warning in attribute argument validation.
#[derive(Debug, Clone)]
pub struct ArgValidationWarning {
    /// Warning kind
    pub kind: ArgValidationWarningKind,

    /// Source location
    pub span: Span,
}

/// Kinds of argument validation warnings.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgValidationWarningKind {
    /// Deprecated argument name
    DeprecatedArg {
        name: Text,
        replacement: Maybe<Text>,
    },

    /// Redundant argument (default value specified explicitly)
    RedundantArg { name: Text },

    /// Argument order differs from convention
    NonConventionalOrder,
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arg_spec_none() {
        let spec = ArgSpec::None;
        assert!(spec.allows_empty());
        assert_eq!(spec.description().as_str(), "no arguments");
    }

    #[test]
    fn test_arg_spec_optional() {
        let spec = ArgSpec::Optional(ArgType::Ident);
        assert!(spec.allows_empty());
        assert!(spec.description().contains("optional"));
    }

    #[test]
    fn test_arg_spec_required() {
        let spec = ArgSpec::Required(ArgType::Int);
        assert!(!spec.allows_empty());
        assert!(spec.description().contains("required"));
    }

    #[test]
    fn test_named_arg_spec() {
        let spec = NamedArgSpec::required("rename", ArgType::String);
        assert!(spec.required);
        assert!(spec.matches_name("rename"));
        assert!(!spec.matches_name("other"));
    }

    #[test]
    fn test_named_arg_with_alias() {
        let spec = NamedArgSpec::optional("rename", ArgType::String).alias("name");
        assert!(spec.matches_name("rename"));
        assert!(spec.matches_name("name"));
        assert!(!spec.matches_name("other"));
    }

    #[test]
    fn test_arg_type_description() {
        assert_eq!(ArgType::Ident.description(), "identifier");
        assert_eq!(ArgType::String.description(), "string");
        assert_eq!(ArgType::Int.description(), "integer");
    }

    #[test]
    fn test_validation_result() {
        let ok = ArgValidationResult::ok();
        assert!(ok.is_ok());
        assert!(!ok.has_warnings());

        let err = ArgValidationResult::error(ArgValidationError::unexpected_args(Span::default()));
        assert!(!err.is_ok());
    }

    #[test]
    fn test_serialization() {
        let spec = ArgSpec::Named(vec![
            NamedArgSpec::required("min", ArgType::Int),
            NamedArgSpec::optional("max", ArgType::Int),
        ].into());

        let json = serde_json::to_string(&spec).unwrap();
        let restored: ArgSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, restored);
    }
}
