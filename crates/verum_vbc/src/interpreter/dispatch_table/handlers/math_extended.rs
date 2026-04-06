//! Math extended opcode handler for VBC interpreter dispatch.

use crate::instruction::{MathSubOpcode, Opcode};
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

/// MathExtended (0x29) - Transcendental and special math functions.
///
/// Zero-cost dispatch via Rust match. Each sub-opcode maps 1:1 to:
/// - Interpreter: Native Rust method (f64::sin, f64::cos, etc.)
/// - AOT/LLVM: LLVM intrinsic (llvm.sin.f64, llvm.sqrt.f64, etc.)
/// - MLIR: math dialect ops (math.sin, math.sqrt, etc.)
///
/// Sub-opcode ranges:
/// - 0x00-0x0F: Trigonometric F64 (sin, cos, tan, asin, acos, atan, atan2)
/// - 0x10-0x17: Trigonometric F32
/// - 0x18-0x1F: Hyperbolic F64 (sinh, cosh, tanh, asinh, acosh, atanh)
/// - 0x20-0x27: Hyperbolic F32
/// - 0x28-0x2F: Exponential/Log F64 (exp, exp2, expm1, log, log2, log10, log1p, pow)
/// - 0x30-0x37: Exponential/Log F32
/// - 0x38-0x3F: Root/Power F64 (sqrt, cbrt, hypot)
/// - 0x40-0x47: Root/Power F32
/// - 0x48-0x4F: Rounding F64 (floor, ceil, round, trunc)
/// - 0x50-0x57: Rounding F32
/// - 0x58-0x5F: Special F64 (abs, copysign, fma, fmod, remainder, fdim, minnum, maxnum)
/// - 0x60-0x67: Special F32
/// - 0x68-0x6F: Classification F64 (is_nan, is_inf, is_finite)
/// - 0x70-0x77: Classification F32
pub(in super::super) fn handle_math_extended(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    let sub_op = MathSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        // ================================================================
        // Trigonometric F64 (0x00-0x07)
        // ================================================================
        Some(MathSubOpcode::SinF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.sin()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CosF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.cos()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::TanF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.tan()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AsinF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.asin()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AcosF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.acos()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AtanF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.atan()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Atan2F64) => {
            let dst = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y = state.get_reg(y_reg).as_f64();
            let x = state.get_reg(x_reg).as_f64();
            state.set_reg(dst, Value::from_f64(y.atan2(x)));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Trigonometric F32 (0x10-0x17)
        // ================================================================
        Some(MathSubOpcode::SinF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.sin() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CosF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.cos() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::TanF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.tan() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AsinF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.asin() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AcosF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.acos() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AtanF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.atan() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Atan2F32) => {
            let dst = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y = state.get_reg(y_reg).as_f64() as f32;
            let x = state.get_reg(x_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(y.atan2(x) as f64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Hyperbolic F64 (0x18-0x1F)
        // ================================================================
        Some(MathSubOpcode::SinhF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.sinh()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CoshF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.cosh()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::TanhF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.tanh()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AsinhF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.asinh()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AcoshF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.acosh()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AtanhF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.atanh()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Hyperbolic F32 (0x20-0x27)
        // ================================================================
        Some(MathSubOpcode::SinhF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.sinh() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CoshF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.cosh() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::TanhF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.tanh() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AsinhF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.asinh() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AcoshF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.acosh() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::AtanhF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.atanh() as f64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Exponential/Logarithmic F64 (0x28-0x2F)
        // ================================================================
        Some(MathSubOpcode::ExpF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.exp()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Exp2F64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.exp2()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Expm1F64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.exp_m1()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::LogF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.ln()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Log2F64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.log2()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Log10F64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.log10()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Log1pF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.ln_1p()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::PowF64) => {
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let exp_reg = read_reg(state)?;
            let base = state.get_reg(base_reg).as_f64();
            let exp = state.get_reg(exp_reg).as_f64();
            state.set_reg(dst, Value::from_f64(base.powf(exp)));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::PowiF64) => {
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let exp_reg = read_reg(state)?;
            let base = state.get_reg(base_reg).as_f64();
            let exp = state.get_reg(exp_reg).as_i64() as i32;
            state.set_reg(dst, Value::from_f64(base.powi(exp)));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Exponential/Logarithmic F32 (0x30-0x37)
        // ================================================================
        Some(MathSubOpcode::ExpF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.exp() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Exp2F32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.exp2() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Expm1F32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.exp_m1() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::LogF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.ln() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Log2F32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.log2() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Log10F32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.log10() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::Log1pF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.ln_1p() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::PowF32) => {
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let exp_reg = read_reg(state)?;
            let base = state.get_reg(base_reg).as_f64() as f32;
            let exp = state.get_reg(exp_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(base.powf(exp) as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::PowiF32) => {
            let dst = read_reg(state)?;
            let base_reg = read_reg(state)?;
            let exp_reg = read_reg(state)?;
            let base = state.get_reg(base_reg).as_f64() as f32;
            let exp = state.get_reg(exp_reg).as_i64() as i32;
            state.set_reg(dst, Value::from_f64(base.powi(exp) as f64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Root/Power F64 (0x38-0x3F)
        // ================================================================
        Some(MathSubOpcode::SqrtF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.sqrt()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CbrtF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.cbrt()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::HypotF64) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            state.set_reg(dst, Value::from_f64(x.hypot(y)));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Root/Power F32 (0x40-0x47)
        // ================================================================
        Some(MathSubOpcode::SqrtF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.sqrt() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CbrtF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.cbrt() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::HypotF32) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64() as f32;
            let y = state.get_reg(y_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.hypot(y) as f64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Rounding F64 (0x48-0x4F)
        // ================================================================
        Some(MathSubOpcode::FloorF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.floor()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CeilF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.ceil()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::RoundF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.round()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::TruncF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.trunc()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Rounding F32 (0x50-0x57)
        // ================================================================
        Some(MathSubOpcode::FloorF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.floor() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CeilF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.ceil() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::RoundF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.round() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::TruncF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.trunc() as f64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Special F64 (0x58-0x5F)
        // ================================================================
        Some(MathSubOpcode::AbsF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_f64(x.abs()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CopysignF64) => {
            let dst = read_reg(state)?;
            let mag_reg = read_reg(state)?;
            let sign_reg = read_reg(state)?;
            let mag = state.get_reg(mag_reg).as_f64();
            let sign = state.get_reg(sign_reg).as_f64();
            state.set_reg(dst, Value::from_f64(mag.copysign(sign)));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::FmaF64) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let c_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64();
            let b = state.get_reg(b_reg).as_f64();
            let c = state.get_reg(c_reg).as_f64();
            state.set_reg(dst, Value::from_f64(a.mul_add(b, c)));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::FmodF64) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            // Rust rem_euclid doesn't match fmod, use manual calculation
            state.set_reg(dst, Value::from_f64(x - (x / y).trunc() * y));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::RemainderF64) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            // IEEE 754 remainder: x - n*y where n = round(x/y)
            let n = (x / y).round();
            state.set_reg(dst, Value::from_f64(x - n * y));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::FdimF64) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            state.set_reg(dst, Value::from_f64(if x > y { x - y } else { 0.0 }));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::MinnumF64) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            // minnum: returns the minimum, NaN-propagating
            state.set_reg(dst, Value::from_f64(x.min(y)));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::MaxnumF64) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64();
            let y = state.get_reg(y_reg).as_f64();
            // maxnum: returns the maximum, NaN-propagating
            state.set_reg(dst, Value::from_f64(x.max(y)));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Special F32 (0x60-0x67)
        // ================================================================
        Some(MathSubOpcode::AbsF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.abs() as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::CopysignF32) => {
            let dst = read_reg(state)?;
            let mag_reg = read_reg(state)?;
            let sign_reg = read_reg(state)?;
            let mag = state.get_reg(mag_reg).as_f64() as f32;
            let sign = state.get_reg(sign_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(mag.copysign(sign) as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::FmaF32) => {
            let dst = read_reg(state)?;
            let a_reg = read_reg(state)?;
            let b_reg = read_reg(state)?;
            let c_reg = read_reg(state)?;
            let a = state.get_reg(a_reg).as_f64() as f32;
            let b = state.get_reg(b_reg).as_f64() as f32;
            let c = state.get_reg(c_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(a.mul_add(b, c) as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::FmodF32) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64() as f32;
            let y = state.get_reg(y_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64((x - (x / y).trunc() * y) as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::RemainderF32) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64() as f32;
            let y = state.get_reg(y_reg).as_f64() as f32;
            let n = (x / y).round();
            state.set_reg(dst, Value::from_f64((x - n * y) as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::FdimF32) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64() as f32;
            let y = state.get_reg(y_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(if x > y { (x - y) as f64 } else { 0.0 }));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::MinnumF32) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64() as f32;
            let y = state.get_reg(y_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.min(y) as f64));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::MaxnumF32) => {
            let dst = read_reg(state)?;
            let x_reg = read_reg(state)?;
            let y_reg = read_reg(state)?;
            let x = state.get_reg(x_reg).as_f64() as f32;
            let y = state.get_reg(y_reg).as_f64() as f32;
            state.set_reg(dst, Value::from_f64(x.max(y) as f64));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Classification F64 (0x68-0x6F)
        // ================================================================
        Some(MathSubOpcode::IsNanF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_bool(x.is_nan()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::IsInfF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_bool(x.is_infinite()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::IsFiniteF64) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64();
            state.set_reg(dst, Value::from_bool(x.is_finite()));
            Ok(DispatchResult::Continue)
        }

        // ================================================================
        // Classification F32 (0x70-0x77)
        // ================================================================
        Some(MathSubOpcode::IsNanF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_bool(x.is_nan()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::IsInfF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_bool(x.is_infinite()));
            Ok(DispatchResult::Continue)
        }
        Some(MathSubOpcode::IsFiniteF32) => {
            let dst = read_reg(state)?;
            let src = read_reg(state)?;
            let x = state.get_reg(src).as_f64() as f32;
            state.set_reg(dst, Value::from_bool(x.is_finite()));
            Ok(DispatchResult::Continue)
        }

        // Unimplemented sub-opcodes
        None => {
            Err(InterpreterError::NotImplemented {
                feature: "math_extended sub-opcode",
                opcode: Some(Opcode::MathExtended),
            })
        }
    }
}
