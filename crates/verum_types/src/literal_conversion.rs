//! Protocol-Based Literal Conversions
//!
//! Type aliases and newtype definitions via "type X is T" syntax
//!
//! Verum's literal system enables types to declare how they construct from
//! literal values through compile-time protocols. This provides:
//! - Type-directed literal interpretation
//! - Domain-specific literals (units, tagged strings)
//! - Compile-time validation with zero runtime overhead
//!
//! # Core Literal Protocols
//!
//! Types implement literal conversion protocols to define how literal values
//! construct instances:
//!
//! ```verum
//! // Integer literal conversion
//! type FromIntegerLiteral is protocol {
//!     fn from_integer_literal(value: i64) -> Self
//!         where const_eval;
//! }
//!
//! // Floating-point literal conversion
//! type FromFloatLiteral is protocol {
//!     fn from_float_literal(value: f64) -> Self
//!         where const_eval;
//! }
//!
//! // Text literal conversion
//! type FromTextLiteral is protocol {
//!     fn from_text_literal(value: &str) -> Self
//!         where const_eval;
//! }
//! ```
//!
//! # Examples
//!
//! ```verum
//! // Custom numeric type with validation
//! type Percentage is { value: f64 where value >= 0.0 && value <= 100.0 };
//!
//! implement FromFloatLiteral for Percentage {
//!     fn from_float_literal(value: f64) -> Self {
//!         if value < 0.0 || value > 100.0 {
//!             compile_error!("Percentage must be between 0 and 100");
//!         }
//!         Percentage { value }
//!     }
//! }
//!
//! let p: Percentage = 50.0;  // ✓ OK
//! let q: Percentage = 150.0; // ✗ Compile error
//! ```

use crate::TypeError;
use crate::const_eval::ConstEvaluator;
use verum_common::ConstValue;
use crate::ty::Type;
use verum_ast::literal::Literal;
use verum_ast::span::Span;
#[allow(unused_imports)]
use verum_common::Text;

/// Standard literal conversion protocols
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralProtocol {
    /// FromIntegerLiteral protocol
    FromInteger,
    /// FromFloatLiteral protocol
    FromFloat,
    /// FromTextLiteral protocol
    FromText,
    /// FromBoolLiteral protocol
    FromBool,
    /// FromCharLiteral protocol
    FromChar,
}

impl LiteralProtocol {
    /// Get the protocol name
    pub fn name(&self) -> &'static str {
        match self {
            LiteralProtocol::FromInteger => "FromIntegerLiteral",
            LiteralProtocol::FromFloat => "FromFloatLiteral",
            LiteralProtocol::FromText => "FromTextLiteral",
            LiteralProtocol::FromBool => "FromBoolLiteral",
            LiteralProtocol::FromChar => "FromCharLiteral",
        }
    }

    /// Get the method name for this protocol
    pub fn method_name(&self) -> &'static str {
        match self {
            LiteralProtocol::FromInteger => "from_integer_literal",
            LiteralProtocol::FromFloat => "from_float_literal",
            LiteralProtocol::FromText => "from_text_literal",
            LiteralProtocol::FromBool => "from_bool_literal",
            LiteralProtocol::FromChar => "from_char_literal",
        }
    }

    /// Determine which protocol is needed for a literal
    pub fn for_literal(lit: &Literal) -> Option<Self> {
        use verum_ast::literal::LiteralKind;
        match &lit.kind {
            LiteralKind::Int(_) => Some(LiteralProtocol::FromInteger),
            LiteralKind::Float(_) => Some(LiteralProtocol::FromFloat),
            LiteralKind::Text(_) => Some(LiteralProtocol::FromText),
            LiteralKind::Bool(_) => Some(LiteralProtocol::FromBool),
            LiteralKind::Char(_) => Some(LiteralProtocol::FromChar),
            _ => None,
        }
    }
}

/// Literal conversion checker
pub struct LiteralConverter {
    /// Const evaluator for compile-time conversion
    evaluator: ConstEvaluator,
}

impl LiteralConverter {
    /// Create a new literal converter
    pub fn new() -> Self {
        Self {
            evaluator: ConstEvaluator::new(),
        }
    }

    /// Convert a literal to a target type using protocol-based conversion
    ///
    /// This performs compile-time evaluation of the conversion function.
    pub fn convert_literal(
        &mut self,
        lit: &Literal,
        target_type: &Type,
        span: Span,
    ) -> Result<ConstValue, TypeError> {
        // Determine which protocol we need
        let protocol = LiteralProtocol::for_literal(lit)
            .ok_or_else(|| TypeError::Other("No literal protocol for this literal type".into()))?;

        // Check if target type implements the protocol
        // For now, we'll do basic type-directed conversion
        // In a full implementation, this would check protocol implementations

        use verum_ast::literal::LiteralKind;
        match (&lit.kind, target_type) {
            // Direct conversions for built-in types
            (LiteralKind::Int(int_lit), Type::Int) => Ok(ConstValue::Int(int_lit.value)),
            (LiteralKind::Bool(b), Type::Bool) => Ok(ConstValue::Bool(*b)),

            // Float, Text, and Char conversions
            (LiteralKind::Float(float_lit), Type::Float) => Ok(ConstValue::Float(float_lit.value)),

            (LiteralKind::Text(string_lit), Type::Text) => {
                Ok(ConstValue::Text(Text::from(string_lit.as_str())))
            }

            (LiteralKind::Char(c), Type::Char) => Ok(ConstValue::Char(*c)),

            // For named types, we would look up the protocol implementation
            // and call from_X_literal() at compile time
            (_, Type::Named { path, .. }) => {
                // This is where we would:
                // 1. Look up the protocol implementation for the type
                // 2. Call the from_X_literal method
                // 3. Evaluate it at compile time
                // For now, return an error
                Err(TypeError::Other(
                    format!(
                        "Literal conversion for type {} not yet implemented\n  \
                     at: {}\n  \
                     help: implement {} for the target type\n  \
                     help: ensure the conversion function is marked with const_eval",
                        path,
                        span.start,
                        protocol.name()
                    )
                    .into(),
                ))
            }

            // Refinement types: convert base type, then validate refinement
            (_, Type::Refined { base, predicate: _ }) => {
                let base_value = self.convert_literal(lit, base, span)?;

                // Validate refinement predicate at compile time
                // This would use the const evaluator to check the predicate
                // For now, just convert the base
                Ok(base_value)
            }

            _ => Err(TypeError::Mismatch {
                expected: "compatible literal type".into(),
                actual: "incompatible literal".into(),
                span,
            }),
        }
    }

    /// Check if a type implements a literal conversion protocol
    pub fn implements_literal_protocol(&self, ty: &Type, protocol: LiteralProtocol) -> bool {
        // In a full implementation, this would check the protocol registry
        // For built-in types, we can return true directly
        matches!(
            (ty, protocol),
            (Type::Int, LiteralProtocol::FromInteger)
                | (Type::Float, LiteralProtocol::FromFloat)
                | (Type::Text, LiteralProtocol::FromText)
                | (Type::Bool, LiteralProtocol::FromBool)
                | (Type::Char, LiteralProtocol::FromChar)
        )
    }

    /// Validate that a literal conversion function is const_eval
    ///
    /// All literal conversion functions must be evaluable at compile time.
    pub fn validate_const_eval(&self, _function_name: &str, _span: Span) -> Result<(), TypeError> {
        // In a full implementation, this would check that the function
        // is marked with const_eval and can actually be evaluated at compile time
        Ok(())
    }
}

impl Default for LiteralConverter {
    fn default() -> Self {
        Self::new()
    }
}

// Tests moved to tests/literal_conversion_tests.rs per project testing guidelines.
