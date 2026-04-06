//! Arithmetic Intrinsics (Tier 0 - Always Available)
//!
//! Pure arithmetic functions that operate only on input values without
//! accessing any external state. These are always available in meta expressions.
//!
//! ## Functions
//!
//! | Function | Signature | Description |
//! |----------|-----------|-------------|
//! | `abs(x)` | `(Numeric) -> Numeric` | Absolute value |
//! | `min(a, b)` | `(T, T) -> T` | Minimum of two values |
//! | `max(a, b)` | `(T, T) -> T` | Maximum of two values |
//! | `int_to_text(x)` | `(Int) -> Text` | Convert integer to text |
//! | `text_to_int(s)` | `(Text) -> Int` | Parse text as integer |
//! | `bitwise_and(a, b)` | `(Int, Int) -> Int` | Bitwise AND |
//! | `bitwise_or(a, b)` | `(Int, Int) -> Int` | Bitwise OR |
//! | `bitwise_xor(a, b)` | `(Int, Int) -> Int` | Bitwise XOR |
//! | `bitwise_not(x)` | `(Int) -> Int` | Bitwise NOT |
//! | `shift_left(x, n)` | `(Int, Int) -> Int` | Left shift |
//! | `shift_right(x, n)` | `(Int, Int) -> Int` | Right shift |
//! | `clamp(x, min, max)` | `(T, T, T) -> T` | Clamp value to range |
//! | `pow(base, exp)` | `(Int, Int) -> Int` | Integer power |
//!
//! ## Context Requirements
//!
//! **Tier 0**: No context required - these are pure computation functions.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta context unification: all compile-time features desugar to meta-system
//! operations, providing one coherent model with convenient syntax sugar.

use verum_common::{List, Text};

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};
use super::{ConstValue, MetaContext, MetaError};

/// Register arithmetic builtins with context requirements
///
/// All arithmetic functions are Tier 0 (always available) since they
/// perform pure computation without accessing external state.
pub fn register_builtins(map: &mut BuiltinRegistry) {
    map.insert(
        Text::from("abs"),
        BuiltinInfo::tier0(
            meta_abs,
            "Absolute value of a number",
            "(Numeric) -> Numeric",
        ),
    );
    map.insert(
        Text::from("min"),
        BuiltinInfo::tier0(
            meta_min,
            "Minimum of two comparable values",
            "(T, T) -> T where T: Ord",
        ),
    );
    map.insert(
        Text::from("max"),
        BuiltinInfo::tier0(
            meta_max,
            "Maximum of two comparable values",
            "(T, T) -> T where T: Ord",
        ),
    );
    map.insert(
        Text::from("int_to_text"),
        BuiltinInfo::tier0(
            meta_int_to_text,
            "Convert integer to text representation",
            "(Int) -> Text",
        ),
    );
    map.insert(
        Text::from("text_to_int"),
        BuiltinInfo::tier0(
            meta_text_to_int,
            "Parse text as integer",
            "(Text) -> Int",
        ),
    );

    // Bitwise operations (P1 feature - audit requirement)
    map.insert(
        Text::from("bitwise_and"),
        BuiltinInfo::tier0(
            meta_bitwise_and,
            "Bitwise AND of two integers",
            "(Int, Int) -> Int",
        ),
    );
    map.insert(
        Text::from("bitwise_or"),
        BuiltinInfo::tier0(
            meta_bitwise_or,
            "Bitwise OR of two integers",
            "(Int, Int) -> Int",
        ),
    );
    map.insert(
        Text::from("bitwise_xor"),
        BuiltinInfo::tier0(
            meta_bitwise_xor,
            "Bitwise XOR of two integers",
            "(Int, Int) -> Int",
        ),
    );
    map.insert(
        Text::from("bitwise_not"),
        BuiltinInfo::tier0(
            meta_bitwise_not,
            "Bitwise NOT of an integer",
            "(Int) -> Int",
        ),
    );
    map.insert(
        Text::from("shift_left"),
        BuiltinInfo::tier0(
            meta_shift_left,
            "Left shift by n bits",
            "(Int, Int) -> Int",
        ),
    );
    map.insert(
        Text::from("shift_right"),
        BuiltinInfo::tier0(
            meta_shift_right,
            "Right shift by n bits",
            "(Int, Int) -> Int",
        ),
    );
    map.insert(
        Text::from("clamp"),
        BuiltinInfo::tier0(
            meta_clamp,
            "Clamp value to range [min, max]",
            "(T, T, T) -> T where T: Ord",
        ),
    );
    map.insert(
        Text::from("pow"),
        BuiltinInfo::tier0(
            meta_pow,
            "Power (base^exp), works with Int, UInt, or Float base",
            "(T, Int) -> T where T: Numeric",
        ),
    );
}

/// Absolute value
fn meta_abs(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Int(i) => Ok(ConstValue::Int(i.abs())),
        ConstValue::Float(f) => Ok(ConstValue::Float(f.abs())),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int or Float"),
            found: args[0].type_name(),
        }),
    }
}

/// Minimum of two values
fn meta_min(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(*a.min(b))),
        (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(*a.min(b))),
        (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Float(a.min(*b))),
        (ConstValue::Int(a), ConstValue::UInt(b)) => {
            if *a < 0 {
                Ok(ConstValue::Int(*a))
            } else {
                let a_u = *a as u128;
                if a_u < *b {
                    Ok(ConstValue::Int(*a))
                } else {
                    Ok(ConstValue::UInt(*b))
                }
            }
        }
        (ConstValue::UInt(a), ConstValue::Int(b)) => {
            if *b < 0 {
                Ok(ConstValue::Int(*b))
            } else {
                let b_u = *b as u128;
                if *a < b_u {
                    Ok(ConstValue::UInt(*a))
                } else {
                    Ok(ConstValue::Int(*b))
                }
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("numeric types"),
            found: Text::from(format!("({}, {})", args[0].type_name(), args[1].type_name())),
        }),
    }
}

/// Maximum of two values
fn meta_max(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(*a.max(b))),
        (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(*a.max(b))),
        (ConstValue::Float(a), ConstValue::Float(b)) => Ok(ConstValue::Float(a.max(*b))),
        (ConstValue::Int(a), ConstValue::UInt(b)) => {
            if *a < 0 {
                Ok(ConstValue::UInt(*b))
            } else {
                let a_u = *a as u128;
                if a_u > *b {
                    Ok(ConstValue::Int(*a))
                } else {
                    Ok(ConstValue::UInt(*b))
                }
            }
        }
        (ConstValue::UInt(a), ConstValue::Int(b)) => {
            if *b < 0 {
                Ok(ConstValue::UInt(*a))
            } else {
                let b_u = *b as u128;
                if *a > b_u {
                    Ok(ConstValue::UInt(*a))
                } else {
                    Ok(ConstValue::Int(*b))
                }
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("numeric types"),
            found: Text::from(format!("({}, {})", args[0].type_name(), args[1].type_name())),
        }),
    }
}

/// Convert integer to text
fn meta_int_to_text(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Int(i) => Ok(ConstValue::Text(Text::from(i.to_string()))),
        ConstValue::UInt(u) => Ok(ConstValue::Text(Text::from(u.to_string()))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int or UInt"),
            found: args[0].type_name(),
        }),
    }
}

/// Parse text as integer
fn meta_text_to_int(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Text(t) => {
            let s = t.trim();
            if let Ok(i) = s.parse::<i128>() {
                Ok(ConstValue::Int(i))
            } else if let Ok(u) = s.parse::<u128>() {
                Ok(ConstValue::UInt(u))
            } else {
                Err(MetaError::BuiltinEvalError {
                    function: Text::from("parse_int"),
                    message: Text::from(format!("Cannot parse '{}' as integer", s)),
                })
            }
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Text"),
            found: args[0].type_name(),
        }),
    }
}

// ============================================================================
// Bitwise Operations (P1 feature: bit_and, bit_or, bit_xor, bit_not, shl, shr)
// ============================================================================

/// Bitwise AND of two integers
fn meta_bitwise_and(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a & b)),
        (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a & b)),
        (ConstValue::Int(a), ConstValue::UInt(b)) => {
            // Promote to i128 for mixed operations
            Ok(ConstValue::Int(*a & (*b as i128)))
        }
        (ConstValue::UInt(a), ConstValue::Int(b)) => {
            Ok(ConstValue::Int((*a as i128) & *b))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int or UInt"),
            found: Text::from(format!("({}, {})", args[0].type_name(), args[1].type_name())),
        }),
    }
}

/// Bitwise OR of two integers
fn meta_bitwise_or(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a | b)),
        (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a | b)),
        (ConstValue::Int(a), ConstValue::UInt(b)) => {
            Ok(ConstValue::Int(*a | (*b as i128)))
        }
        (ConstValue::UInt(a), ConstValue::Int(b)) => {
            Ok(ConstValue::Int((*a as i128) | *b))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int or UInt"),
            found: Text::from(format!("({}, {})", args[0].type_name(), args[1].type_name())),
        }),
    }
}

/// Bitwise XOR of two integers
fn meta_bitwise_xor(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    match (&args[0], &args[1]) {
        (ConstValue::Int(a), ConstValue::Int(b)) => Ok(ConstValue::Int(a ^ b)),
        (ConstValue::UInt(a), ConstValue::UInt(b)) => Ok(ConstValue::UInt(a ^ b)),
        (ConstValue::Int(a), ConstValue::UInt(b)) => {
            Ok(ConstValue::Int(*a ^ (*b as i128)))
        }
        (ConstValue::UInt(a), ConstValue::Int(b)) => {
            Ok(ConstValue::Int((*a as i128) ^ *b))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int or UInt"),
            found: Text::from(format!("({}, {})", args[0].type_name(), args[1].type_name())),
        }),
    }
}

/// Bitwise NOT of an integer
fn meta_bitwise_not(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 1 {
        return Err(MetaError::ArityMismatch { expected: 1, got: args.len() });
    }

    match &args[0] {
        ConstValue::Int(i) => Ok(ConstValue::Int(!i)),
        ConstValue::UInt(u) => Ok(ConstValue::UInt(!u)),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int or UInt"),
            found: args[0].type_name(),
        }),
    }
}

/// Left shift by n bits
fn meta_shift_left(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    let shift_amount = match &args[1] {
        ConstValue::Int(n) if *n >= 0 && *n < 128 => *n as u32,
        ConstValue::UInt(n) if *n < 128 => *n as u32,
        _ => return Err(MetaError::ConstOverflow {
            operation: Text::from("shift_left"),
            value: Text::from("Shift amount must be 0..127"),
        }),
    };

    match &args[0] {
        ConstValue::Int(i) => Ok(ConstValue::Int(i.wrapping_shl(shift_amount))),
        ConstValue::UInt(u) => Ok(ConstValue::UInt(u.wrapping_shl(shift_amount))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int or UInt"),
            found: args[0].type_name(),
        }),
    }
}

/// Right shift by n bits
fn meta_shift_right(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    let shift_amount = match &args[1] {
        ConstValue::Int(n) if *n >= 0 && *n < 128 => *n as u32,
        ConstValue::UInt(n) if *n < 128 => *n as u32,
        _ => return Err(MetaError::ConstOverflow {
            operation: Text::from("shift_right"),
            value: Text::from("Shift amount must be 0..127"),
        }),
    };

    match &args[0] {
        ConstValue::Int(i) => Ok(ConstValue::Int(i.wrapping_shr(shift_amount))),
        ConstValue::UInt(u) => Ok(ConstValue::UInt(u.wrapping_shr(shift_amount))),
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("Int or UInt"),
            found: args[0].type_name(),
        }),
    }
}

/// Clamp value to range [min, max]
fn meta_clamp(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 3 {
        return Err(MetaError::ArityMismatch { expected: 3, got: args.len() });
    }

    match (&args[0], &args[1], &args[2]) {
        (ConstValue::Int(x), ConstValue::Int(min), ConstValue::Int(max)) => {
            Ok(ConstValue::Int(*x.max(min).min(max)))
        }
        (ConstValue::UInt(x), ConstValue::UInt(min), ConstValue::UInt(max)) => {
            Ok(ConstValue::UInt(*x.max(min).min(max)))
        }
        (ConstValue::Float(x), ConstValue::Float(min), ConstValue::Float(max)) => {
            Ok(ConstValue::Float(x.max(*min).min(*max)))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("matching numeric types"),
            found: Text::from(format!(
                "({}, {}, {})",
                args[0].type_name(),
                args[1].type_name(),
                args[2].type_name()
            )),
        }),
    }
}

/// Integer power (base^exp)
fn meta_pow(_ctx: &mut MetaContext, args: List<ConstValue>) -> Result<ConstValue, MetaError> {
    if args.len() != 2 {
        return Err(MetaError::ArityMismatch { expected: 2, got: args.len() });
    }

    // Handle Float exponent: pow(Float, Float) -> Float using powf
    if let ConstValue::Float(exp_f) = &args[1] {
        return match &args[0] {
            ConstValue::Float(base) => Ok(ConstValue::Float(base.powf(*exp_f))),
            ConstValue::Int(base) => Ok(ConstValue::Float((*base as f64).powf(*exp_f))),
            _ => Err(MetaError::TypeMismatch {
                expected: Text::from("numeric type"),
                found: args[0].type_name(),
            }),
        };
    }

    let exp = match &args[1] {
        ConstValue::Int(n) if *n >= 0 => *n as u32,
        ConstValue::UInt(n) if *n <= u32::MAX as u128 => *n as u32,
        ConstValue::Int(_) => return Err(MetaError::ConstOverflow {
            operation: Text::from("pow"),
            value: Text::from("Negative exponent not supported"),
        }),
        _ => return Err(MetaError::TypeMismatch {
            expected: Text::from("non-negative Int or Float"),
            found: args[1].type_name(),
        }),
    };

    match &args[0] {
        ConstValue::Int(base) => {
            if exp > 127 {
                return Err(MetaError::ConstOverflow {
                    operation: Text::from("pow"),
                    value: Text::from("Exponent too large"),
                });
            }
            Ok(ConstValue::Int(base.wrapping_pow(exp)))
        }
        ConstValue::UInt(base) => {
            if exp > 127 {
                return Err(MetaError::ConstOverflow {
                    operation: Text::from("pow"),
                    value: Text::from("Exponent too large"),
                });
            }
            Ok(ConstValue::UInt(base.wrapping_pow(exp)))
        }
        ConstValue::Float(base) => {
            Ok(ConstValue::Float(base.powi(exp as i32)))
        }
        _ => Err(MetaError::TypeMismatch {
            expected: Text::from("numeric type"),
            found: args[0].type_name(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abs_positive() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(42)]);
        let result = meta_abs(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(42));
    }

    #[test]
    fn test_abs_negative() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(-42)]);
        let result = meta_abs(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(42));
    }

    #[test]
    fn test_abs_float() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Float(-2.5)]);
        let result = meta_abs(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Float(2.5));
    }

    #[test]
    fn test_min_int() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(10), ConstValue::Int(5)]);
        let result = meta_min(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(5));
    }

    #[test]
    fn test_max_int() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(10), ConstValue::Int(5)]);
        let result = meta_max(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(10));
    }

    #[test]
    fn test_min_float() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Float(2.5), ConstValue::Float(1.5)]);
        let result = meta_min(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Float(1.5));
    }

    #[test]
    fn test_int_to_text() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Int(42)]);
        let result = meta_int_to_text(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Text(Text::from("42")));
    }

    #[test]
    fn test_text_to_int() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("42"))]);
        let result = meta_text_to_int(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(42));
    }

    #[test]
    fn test_text_to_int_negative() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("-123"))]);
        let result = meta_text_to_int(&mut ctx, args).unwrap();
        assert_eq!(result, ConstValue::Int(-123));
    }

    #[test]
    fn test_text_to_int_invalid() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![ConstValue::Text(Text::from("not a number"))]);
        let result = meta_text_to_int(&mut ctx, args);
        assert!(result.is_err());
    }
}
