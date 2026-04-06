//! Float arithmetic handlers for VBC interpreter dispatch.

use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

// ============================================================================
// Handler Implementations - Float Arithmetic
// ============================================================================

pub(in super::super) fn handle_addf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() + state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_f64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_subf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() - state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_f64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_mulf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() * state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_f64(result));
    Ok(DispatchResult::Continue)
}

pub(in super::super) fn handle_divf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() / state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_f64(result));
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - Unary Float Operations
// ============================================================================

/// Unary float operations with sub-opcode dispatch.
///
/// Format: `[0x25] [sub_op:u8] [dst:reg] [src:reg]`
///
/// Sub-opcodes (basic, aligned with UnaryFloatOp enum):
/// - 0: Neg - Negate
/// - 1: Abs - Absolute value
/// - 2: Sqrt - Square root
/// - 3: Exp - Natural exponential (e^x)
/// - 4: Log - Natural logarithm (ln)
/// - 5: Sin - Sine
/// - 6: Cos - Cosine
/// - 7: Tan - Tangent
/// - 8: Floor - Floor (round down)
/// - 9: Ceil - Ceiling (round up)
/// - 10: Round - Round to nearest
///
/// Extended sub-opcodes (transcendental functions):
/// - 11: Asin - Inverse sine (arcsin)
/// - 12: Acos - Inverse cosine (arccos)
/// - 13: Atan - Inverse tangent (arctan)
/// - 14: Sinh - Hyperbolic sine
/// - 15: Cosh - Hyperbolic cosine
/// - 16: Tanh - Hyperbolic tangent
/// - 17: Asinh - Inverse hyperbolic sine
/// - 18: Acosh - Inverse hyperbolic cosine
/// - 19: Atanh - Inverse hyperbolic tangent
/// - 20: Log10 - Base-10 logarithm
/// - 21: Log2 - Base-2 logarithm
/// - 22: Exp2 - Base-2 exponential (2^x)
/// - 23: Cbrt - Cube root
/// - 24: Expm1 - exp(x) - 1 (accurate for small x)
/// - 25: Ln1p - ln(1 + x) (accurate for small x)
/// - 26: Signum - Sign function (-1, 0, or 1)
/// - 27: Trunc - Truncate toward zero
/// - 28: Fract - Fractional part
/// - 29: Recip - Reciprocal (1/x)
pub(in super::super) fn handle_negf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    // Read sub-opcode byte for the specific unary float operation
    let sub_op = read_u8(state)?;
    let dst = read_reg(state)?;
    let src = read_reg(state)?;

    let x = state.get_reg(src).as_f64();

    let result = match sub_op {
        // Basic operations (0-10)
        0 => -x,                    // Neg
        1 => x.abs(),               // Abs
        2 => x.sqrt(),              // Sqrt
        3 => x.exp(),               // Exp (e^x)
        4 => x.ln(),                // Log (natural log)
        5 => x.sin(),               // Sin
        6 => x.cos(),               // Cos
        7 => x.tan(),               // Tan
        8 => x.floor(),             // Floor
        9 => x.ceil(),              // Ceil
        10 => x.round(),            // Round

        // Inverse trigonometric (11-13)
        11 => x.asin(),             // Asin
        12 => x.acos(),             // Acos
        13 => x.atan(),             // Atan

        // Hyperbolic (14-16)
        14 => x.sinh(),             // Sinh
        15 => x.cosh(),             // Cosh
        16 => x.tanh(),             // Tanh

        // Inverse hyperbolic (17-19)
        17 => x.asinh(),            // Asinh
        18 => x.acosh(),            // Acosh
        19 => x.atanh(),            // Atanh

        // Logarithms and exponentials (20-22)
        20 => x.log10(),            // Log10
        21 => x.log2(),             // Log2
        22 => x.exp2(),             // Exp2 (2^x)

        // Special functions (23-29)
        23 => x.cbrt(),             // Cbrt (cube root)
        24 => x.exp_m1(),           // Expm1 (exp(x) - 1)
        25 => x.ln_1p(),            // Ln1p (ln(1 + x))
        26 => x.signum(),           // Signum
        27 => x.trunc(),            // Trunc
        28 => x.fract(),            // Fract
        29 => x.recip(),            // Recip (1/x)

        _ => {
            return Err(InterpreterError::InvalidSubOpcode {
                opcode: 0x25,
                sub_opcode: sub_op,
            });
        }
    };

    state.set_reg(dst, Value::from_f64(result));
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Handler Implementations - More Float Arithmetic (0x28-0x2F)
// ============================================================================

/// Float power: `dst = a ** b`
pub(in super::super) fn handle_powf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let base = read_reg(state)?;
    let exp = read_reg(state)?;
    let result = state.get_reg(base).as_f64().powf(state.get_reg(exp).as_f64());
    state.set_reg(dst, Value::from_f64(result));
    Ok(DispatchResult::Continue)
}

/// Float modulo: `dst = a % b`
pub(in super::super) fn handle_modf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let a = read_reg(state)?;
    let b = read_reg(state)?;
    let result = state.get_reg(a).as_f64() % state.get_reg(b).as_f64();
    state.set_reg(dst, Value::from_f64(result));
    Ok(DispatchResult::Continue)
}

/// Float absolute value: `dst = |src|`
pub(in super::super) fn handle_absf(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let dst = read_reg(state)?;
    let src = read_reg(state)?;
    let result = state.get_reg(src).as_f64().abs();
    state.set_reg(dst, Value::from_f64(result));
    Ok(DispatchResult::Continue)
}
