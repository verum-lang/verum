//! Cubical type theory instruction handlers for the VBC interpreter.
//!
//! These handlers implement the runtime semantics of cubical type theory
//! operations. At runtime:
//! - Path values are represented as their computational content
//!   (refl → the point, transport → the transported value, etc.)
//! - Interval points i0, i1 are represented as integers (0 and 1)
//! - Most cubical operations are erased by proof_erasure.rs before
//!   reaching the interpreter
//!
//! The handlers support the computational residue that survives proof
//! erasure (e.g., `transport(refl, x) → x`, `transport(ua(e), x) → e.forward(x)`).
//! Complex cubical terms are normalized at the type-checking level by
//! `verum_types::cubical` and do not need full runtime interpretation.

use crate::instruction::{CubicalSubOpcode, Opcode};
use crate::value::Value;
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

// ============================================================================
// CubicalExtended Handler (0xDE)
// ============================================================================

/// Handler for `CubicalExtended` opcode (0xDE).
///
/// Reads the sub-opcode byte and dispatches to the appropriate cubical
/// operation handler.
pub(in super::super) fn handle_cubical_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    let sub_op = CubicalSubOpcode::from_byte(sub_op_byte);

    match sub_op {
        Some(CubicalSubOpcode::PathRefl) => handle_path_refl(state),
        Some(CubicalSubOpcode::PathLambda) => handle_path_lambda(state),
        Some(CubicalSubOpcode::PathApp) => handle_path_app(state),
        Some(CubicalSubOpcode::PathSym) => handle_path_sym(state),
        Some(CubicalSubOpcode::PathTrans) => handle_path_trans(state),
        Some(CubicalSubOpcode::PathAp) => handle_path_ap(state),
        Some(CubicalSubOpcode::Transport) => handle_transport(state),
        Some(CubicalSubOpcode::Hcomp) => handle_hcomp(state),
        Some(CubicalSubOpcode::IntervalI0) => handle_interval_i0(state),
        Some(CubicalSubOpcode::IntervalI1) => handle_interval_i1(state),
        Some(CubicalSubOpcode::IntervalMeet) => handle_interval_meet(state),
        Some(CubicalSubOpcode::IntervalJoin) => handle_interval_join(state),
        Some(CubicalSubOpcode::IntervalRev) => handle_interval_rev(state),
        Some(CubicalSubOpcode::Ua) => handle_ua(state),
        Some(CubicalSubOpcode::UaInv) => handle_ua_inv(state),
        Some(CubicalSubOpcode::EquivFwd) => handle_equiv_fwd(state),
        Some(CubicalSubOpcode::EquivBwd) => handle_equiv_bwd(state),
        None => Err(InterpreterError::NotImplemented {
            feature: "cubical_extended sub-opcode",
            opcode: Some(Opcode::CubicalExtended),
        }),
    }
}

/// Helper: read dst register and arg_count, then consume arg regs.
/// Returns (dst, list of arg values).
fn read_cubical_args(state: &mut InterpreterState) -> InterpreterResult<(crate::instruction::Reg, Vec<Value>)> {
    let dst = read_reg(state)?;
    let arg_count = read_u8(state)?;
    let mut args = Vec::with_capacity(arg_count as usize);
    for _ in 0..arg_count {
        let reg = read_reg(state)?;
        args.push(state.get_reg(reg));
    }
    Ok((dst, args))
}

// ============================================================================
// Path Construction (0x00-0x0F)
// ============================================================================

/// `refl(x)` = λi. x — constant path at x.
fn handle_path_refl(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let value = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// λ(i:I). body — path lambda.
fn handle_path_lambda(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let func = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, func);
    Ok(DispatchResult::Continue)
}

/// `p @ r` — path application. For refl-like paths, returns the path value.
fn handle_path_app(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let path = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, path);
    Ok(DispatchResult::Continue)
}

/// `sym(p)` = λi. p @ (1-i) — path symmetry.
fn handle_path_sym(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let path = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, path);
    Ok(DispatchResult::Continue)
}

/// `trans(p, q)` — path transitivity.
fn handle_path_trans(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let q = if args.len() >= 2 { args[1] } else if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, q);
    Ok(DispatchResult::Continue)
}

/// `ap(f, p)` = λi. f(p @ i) — action on paths.
fn handle_path_ap(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let path = if args.len() >= 2 { args[1] } else if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, path);
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Transport and Composition (0x10-0x1F)
// ============================================================================

/// `transport(type_path, value)` — transport along a type path.
/// After cubical normalization, surviving transports are identity.
fn handle_transport(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let value = if args.len() >= 2 { args[1] } else if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `hcomp(face, walls, base)` — homogeneous composition.
/// After normalization, hcomp reduces to its base.
fn handle_hcomp(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let base = if args.len() >= 3 { args[2] } else if !args.is_empty() { args[args.len() - 1] } else { Value::unit() };
    state.set_reg(dst, base);
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Interval Operations (0x20-0x2F)
// ============================================================================

fn handle_interval_i0(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, _args) = read_cubical_args(state)?;
    state.set_reg(dst, Value::from_i64(0));
    Ok(DispatchResult::Continue)
}

fn handle_interval_i1(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, _args) = read_cubical_args(state)?;
    state.set_reg(dst, Value::from_i64(1));
    Ok(DispatchResult::Continue)
}

fn handle_interval_meet(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    if args.len() >= 2 {
        let a = args[0].as_integer_compatible();
        let b = args[1].as_integer_compatible();
        state.set_reg(dst, Value::from_i64(a.min(b)));
    } else {
        state.set_reg(dst, Value::from_i64(0));
    }
    Ok(DispatchResult::Continue)
}

fn handle_interval_join(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    if args.len() >= 2 {
        let a = args[0].as_integer_compatible();
        let b = args[1].as_integer_compatible();
        state.set_reg(dst, Value::from_i64(a.max(b)));
    } else {
        state.set_reg(dst, Value::from_i64(0));
    }
    Ok(DispatchResult::Continue)
}

fn handle_interval_rev(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    if !args.is_empty() {
        let i = args[0].as_integer_compatible();
        state.set_reg(dst, Value::from_i64(1 - i));
    } else {
        state.set_reg(dst, Value::from_i64(1));
    }
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Univalence (0x30-0x3F)
// ============================================================================

fn handle_ua(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let equiv = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, equiv);
    Ok(DispatchResult::Continue)
}

fn handle_ua_inv(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let path = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, path);
    Ok(DispatchResult::Continue)
}

fn handle_equiv_fwd(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let value = if args.len() >= 2 { args[1] } else if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

fn handle_equiv_bwd(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let value = if args.len() >= 2 { args[1] } else if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}
