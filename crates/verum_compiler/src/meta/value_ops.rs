//! MetaValue Operations Extension Trait
//!
//! This module provides arithmetic, comparison, and logical operations for
//! `MetaValue` that return `Result<MetaValue, SandboxError>`.
//!
//! Since `MetaValue` is defined in `verum_ast` and `SandboxError` is defined
//! in `verum_compiler`, we cannot add these methods directly to `MetaValue`.
//! Instead, we use an extension trait.
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_ast::MetaValue;
//! use verum_compiler::meta_value_ops::MetaValueOps;
//!
//! let a = MetaValue::int(10);
//! let b = MetaValue::int(5);
//! let result = a.add(b)?; // MetaValue::int(15)
//! ```

use verum_ast::MetaValue;
use verum_common::Text;

use super::sandbox::SandboxError;

/// Extension trait providing arithmetic and comparison operations for MetaValue.
///
/// These operations return `Result<MetaValue, SandboxError>` to properly
/// handle type mismatches and division by zero.
pub trait MetaValueOps: Sized {
    /// Add two values.
    fn add(self, other: Self) -> Result<Self, SandboxError>;

    /// Subtract two values.
    fn sub(self, other: Self) -> Result<Self, SandboxError>;

    /// Multiply two values.
    fn mul(self, other: Self) -> Result<Self, SandboxError>;

    /// Divide two values.
    fn div(self, other: Self) -> Result<Self, SandboxError>;

    /// Modulo operation.
    fn modulo(self, other: Self) -> Result<Self, SandboxError>;

    /// Less than comparison.
    fn lt(self, other: Self) -> Result<Self, SandboxError>;

    /// Less than or equal comparison.
    fn le(self, other: Self) -> Result<Self, SandboxError>;

    /// Greater than comparison.
    fn gt(self, other: Self) -> Result<Self, SandboxError>;

    /// Greater than or equal comparison.
    fn ge(self, other: Self) -> Result<Self, SandboxError>;

    /// Logical AND.
    fn and(self, other: Self) -> Result<Self, SandboxError>;

    /// Logical OR.
    fn or(self, other: Self) -> Result<Self, SandboxError>;

    /// Logical NOT.
    fn not(self) -> Result<Self, SandboxError>;

    /// Negation.
    fn neg(self) -> Result<Self, SandboxError>;

    /// Equality comparison returning MetaValue.
    fn value_eq(&self, other: &Self) -> bool;
}

impl MetaValueOps for MetaValue {
    fn add(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => Ok(MetaValue::Int(a + b)),
            (MetaValue::UInt(a), MetaValue::UInt(b)) => Ok(MetaValue::UInt(a + b)),
            (MetaValue::Float(a), MetaValue::Float(b)) => Ok(MetaValue::Float(a + b)),
            (MetaValue::Text(a), MetaValue::Text(b)) => {
                let mut result = a.clone();
                result.push_str(b.as_str());
                Ok(MetaValue::Text(result))
            }
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("add"),
                reason: Text::from("Type mismatch in addition"),
            }),
        }
    }

    fn sub(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => Ok(MetaValue::Int(a - b)),
            (MetaValue::UInt(a), MetaValue::UInt(b)) => Ok(MetaValue::UInt(a - b)),
            (MetaValue::Float(a), MetaValue::Float(b)) => Ok(MetaValue::Float(a - b)),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("sub"),
                reason: Text::from("Type mismatch in subtraction"),
            }),
        }
    }

    fn mul(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => Ok(MetaValue::Int(a * b)),
            (MetaValue::UInt(a), MetaValue::UInt(b)) => Ok(MetaValue::UInt(a * b)),
            (MetaValue::Float(a), MetaValue::Float(b)) => Ok(MetaValue::Float(a * b)),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("mul"),
                reason: Text::from("Type mismatch in multiplication"),
            }),
        }
    }

    fn div(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => {
                if b == 0 {
                    Err(SandboxError::UnsafeOperation {
                        operation: Text::from("div"),
                        reason: Text::from("Division by zero"),
                    })
                } else {
                    Ok(MetaValue::Int(a / b))
                }
            }
            (MetaValue::UInt(a), MetaValue::UInt(b)) => {
                if b == 0 {
                    Err(SandboxError::UnsafeOperation {
                        operation: Text::from("div"),
                        reason: Text::from("Division by zero"),
                    })
                } else {
                    Ok(MetaValue::UInt(a / b))
                }
            }
            (MetaValue::Float(a), MetaValue::Float(b)) => Ok(MetaValue::Float(a / b)),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("div"),
                reason: Text::from("Type mismatch in division"),
            }),
        }
    }

    fn modulo(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => {
                if b == 0 {
                    Err(SandboxError::UnsafeOperation {
                        operation: Text::from("mod"),
                        reason: Text::from("Modulo by zero"),
                    })
                } else {
                    Ok(MetaValue::Int(a % b))
                }
            }
            (MetaValue::UInt(a), MetaValue::UInt(b)) => {
                if b == 0 {
                    Err(SandboxError::UnsafeOperation {
                        operation: Text::from("mod"),
                        reason: Text::from("Modulo by zero"),
                    })
                } else {
                    Ok(MetaValue::UInt(a % b))
                }
            }
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("mod"),
                reason: Text::from("Type mismatch in modulo"),
            }),
        }
    }

    fn lt(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => Ok(MetaValue::Bool(a < b)),
            (MetaValue::UInt(a), MetaValue::UInt(b)) => Ok(MetaValue::Bool(a < b)),
            (MetaValue::Float(a), MetaValue::Float(b)) => Ok(MetaValue::Bool(a < b)),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("lt"),
                reason: Text::from("Type mismatch in comparison"),
            }),
        }
    }

    fn le(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => Ok(MetaValue::Bool(a <= b)),
            (MetaValue::UInt(a), MetaValue::UInt(b)) => Ok(MetaValue::Bool(a <= b)),
            (MetaValue::Float(a), MetaValue::Float(b)) => Ok(MetaValue::Bool(a <= b)),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("le"),
                reason: Text::from("Type mismatch in comparison"),
            }),
        }
    }

    fn gt(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => Ok(MetaValue::Bool(a > b)),
            (MetaValue::UInt(a), MetaValue::UInt(b)) => Ok(MetaValue::Bool(a > b)),
            (MetaValue::Float(a), MetaValue::Float(b)) => Ok(MetaValue::Bool(a > b)),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("gt"),
                reason: Text::from("Type mismatch in comparison"),
            }),
        }
    }

    fn ge(self, other: Self) -> Result<Self, SandboxError> {
        match (self, other) {
            (MetaValue::Int(a), MetaValue::Int(b)) => Ok(MetaValue::Bool(a >= b)),
            (MetaValue::UInt(a), MetaValue::UInt(b)) => Ok(MetaValue::Bool(a >= b)),
            (MetaValue::Float(a), MetaValue::Float(b)) => Ok(MetaValue::Bool(a >= b)),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("ge"),
                reason: Text::from("Type mismatch in comparison"),
            }),
        }
    }

    fn and(self, other: Self) -> Result<Self, SandboxError> {
        Ok(MetaValue::Bool(self.as_bool() && other.as_bool()))
    }

    fn or(self, other: Self) -> Result<Self, SandboxError> {
        Ok(MetaValue::Bool(self.as_bool() || other.as_bool()))
    }

    fn not(self) -> Result<Self, SandboxError> {
        Ok(MetaValue::Bool(!self.as_bool()))
    }

    fn neg(self) -> Result<Self, SandboxError> {
        match self {
            MetaValue::Int(i) => Ok(MetaValue::Int(-i)),
            MetaValue::Float(f) => Ok(MetaValue::Float(-f)),
            _ => Err(SandboxError::UnsafeOperation {
                operation: Text::from("neg"),
                reason: Text::from("Type mismatch in negation"),
            }),
        }
    }

    fn value_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (MetaValue::Unit, MetaValue::Unit) => true,
            (MetaValue::Bool(a), MetaValue::Bool(b)) => a == b,
            (MetaValue::Int(a), MetaValue::Int(b)) => a == b,
            (MetaValue::UInt(a), MetaValue::UInt(b)) => a == b,
            (MetaValue::Float(a), MetaValue::Float(b)) => (a - b).abs() < f64::EPSILON,
            (MetaValue::Char(a), MetaValue::Char(b)) => a == b,
            (MetaValue::Text(a), MetaValue::Text(b)) => a == b,
            (MetaValue::Array(a), MetaValue::Array(b)) => a == b,
            (MetaValue::Tuple(a), MetaValue::Tuple(b)) => a == b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        let a = MetaValue::int(10);
        let b = MetaValue::int(5);
        assert_eq!(a.add(b).unwrap(), MetaValue::int(15));
    }

    #[test]
    fn test_sub() {
        let a = MetaValue::int(10);
        let b = MetaValue::int(3);
        assert_eq!(a.sub(b).unwrap(), MetaValue::int(7));
    }

    #[test]
    fn test_mul() {
        let a = MetaValue::int(10);
        let b = MetaValue::int(3);
        assert_eq!(a.mul(b).unwrap(), MetaValue::int(30));
    }

    #[test]
    fn test_div() {
        let a = MetaValue::int(10);
        let b = MetaValue::int(3);
        assert_eq!(a.div(b).unwrap(), MetaValue::int(3));
    }

    #[test]
    fn test_div_by_zero() {
        let a = MetaValue::int(10);
        let b = MetaValue::int(0);
        assert!(a.div(b).is_err());
    }

    #[test]
    fn test_comparisons() {
        let a = MetaValue::int(10);
        let b = MetaValue::int(5);
        assert_eq!(a.clone().gt(b.clone()).unwrap(), MetaValue::bool(true));
        assert_eq!(a.clone().lt(b.clone()).unwrap(), MetaValue::bool(false));
        assert_eq!(a.clone().ge(b.clone()).unwrap(), MetaValue::bool(true));
        assert_eq!(b.clone().le(a.clone()).unwrap(), MetaValue::bool(true));
    }

    #[test]
    fn test_logical() {
        let t = MetaValue::bool(true);
        let f = MetaValue::bool(false);
        assert_eq!(t.clone().and(f.clone()).unwrap(), MetaValue::bool(false));
        assert_eq!(t.clone().or(f.clone()).unwrap(), MetaValue::bool(true));
        assert_eq!(f.clone().not().unwrap(), MetaValue::bool(true));
    }

    #[test]
    fn test_neg() {
        let a = MetaValue::int(10);
        assert_eq!(a.neg().unwrap(), MetaValue::int(-10));
    }

    #[test]
    fn test_text_concat() {
        let a = MetaValue::text("hello");
        let b = MetaValue::text(" world");
        assert_eq!(a.add(b).unwrap(), MetaValue::text("hello world"));
    }
}
