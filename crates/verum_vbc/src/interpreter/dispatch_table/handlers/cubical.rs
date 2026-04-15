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
use super::super::{call_closure_sync};
use super::method_dispatch::call_function_sync;

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

/// Try to call a value as a function with a single argument.
///
/// Handles both `FuncRef` (named function) and closure pointer (heap object
/// with `[ObjectHeader][func_id:u32][capture_count:u32][...]` layout).
///
/// Returns `Ok(Some(result))` if the call succeeded, `Ok(None)` if the value
/// is not callable (so the caller can fall back to an identity result).
fn try_call_value(
    state: &mut InterpreterState,
    callee: Value,
    arg: Value,
) -> InterpreterResult<Option<Value>> {
    if callee.is_func_ref() {
        let func_id = callee.as_func_id();
        let result = call_function_sync(state, func_id, &[arg])?;
        Ok(Some(result))
    } else if callee.is_ptr() && !callee.is_nil() {
        // Assume closure object: [ObjectHeader][func_id:u32][capture_count:u32][captures...]
        let result = call_closure_sync(state, callee, &[arg])?;
        Ok(Some(result))
    } else {
        Ok(None)
    }
}

/// Read a Value field from a heap object by index.
///
/// Object layout:  [ObjectHeader (24 bytes)] [Value fields...]
///
/// # Codegen contract
///
/// The field indices must match the codegen output exactly:
///   - Equiv struct: field 0 = forward, field 1 = inverse
///   - Copattern body: fields in declaration order of coinductive destructors
///   - Ua wrapper: field 0 = the wrapped equivalence
///
/// If the VBC codegen changes field layout, this handler must be updated.
///
/// Returns `Value::unit()` for non-pointer objects or out-of-bounds access
/// (graceful degradation).
fn read_object_field(obj: Value, field_idx: usize) -> Value {
    use super::super::super::heap::{OBJECT_HEADER_SIZE, ObjectHeader};
    if !obj.is_ptr() || obj.is_nil() {
        return Value::unit();
    }
    let ptr = obj.as_ptr::<u8>();
    // Bounds check: read the ObjectHeader to get data size and verify
    // the field index is within bounds.
    // SAFETY: `ptr` is a live heap object starting with ObjectHeader.
    let header = unsafe { &*(ptr as *const ObjectHeader) };
    let field_offset = field_idx * std::mem::size_of::<Value>();
    let field_end = field_offset + std::mem::size_of::<Value>();
    if field_end > header.size as usize {
        // Out of bounds — return unit rather than reading garbage.
        return Value::unit();
    }
    // SAFETY: `field_offset` is bounds-checked against `header.size` above.
    // `ptr` is a live, aligned heap object. The field at the computed
    // offset is an initialized Value (set at object construction time).
    unsafe {
        let field_ptr =
            ptr.add(OBJECT_HEADER_SIZE + field_offset)
                as *const Value;
        std::ptr::read(field_ptr)
    }
}

// ============================================================================
// Path Construction (0x00-0x0F)
// ============================================================================

/// `refl(x)` = λi. x — constant path at x.
///
/// After proof erasure, `refl` carries its base point as its runtime value.
/// `refl(x) @ r = x` for any interval `r`, so returning `x` is semantically
/// exact for all downstream consumers.
fn handle_path_refl(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let value = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `λ(i:I). body` — path lambda.
///
/// At runtime a path lambda is its underlying closure / function reference.
/// The value is passed through unchanged so that `PathApp` can later invoke it.
fn handle_path_lambda(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let func = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, func);
    Ok(DispatchResult::Continue)
}

/// `p @ r` — path application.
///
/// Computes the value of path `p` at interval point `r`.
///
/// - If `p` is a `FuncRef` → call `p(r)` directly.
/// - If `p` is a closure pointer → call the closure with `r`.
/// - Otherwise (`p` is a constant / refl) → `p` is already the point, return it.
fn handle_path_app(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let path = if !args.is_empty() { args[0] } else { Value::unit() };
    let interval = if args.len() >= 2 { args[1] } else { Value::from_i64(1) };

    let result = match try_call_value(state, path, interval)? {
        Some(v) => v,
        // Refl / constant path: the value IS the point at every interval.
        None => path,
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

/// `sym(p)` = λi. p @ (1-i) — path symmetry.
///
/// After proof erasure the computational content of `sym(p)` is the same as
/// `p` (both endpoints are equal modulo the path direction). We preserve the
/// path value so that any downstream `PathApp` receives the correct callable.
fn handle_path_sym(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let path = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, path);
    Ok(DispatchResult::Continue)
}

/// `trans(p, q)` — path transitivity: concatenate path `p` (from a to b)
/// with path `q` (from b to c) to get a path from a to c.
///
/// After proof erasure the destination endpoint of the composed path is the
/// destination endpoint of `q`.  When the caller later evaluates the composed
/// path at `i1` they get `q @ i1`, which equals the endpoint `c`.  We
/// therefore carry `q` forward as the runtime representative of the composed
/// path — the best approximation achievable without a live interval value.
fn handle_path_trans(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let q = if args.len() >= 2 { args[1] } else if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, q);
    Ok(DispatchResult::Continue)
}

/// `ap(f, p)` = λi. f(p @ i) — functorial action on paths.
///
/// Applies function `f` pointwise along path `p`.  At runtime:
/// - If `p` is callable (FuncRef or closure), compose `f ∘ p` by calling
///   `p` at `i1` (the canonical endpoint) and then applying `f`.
/// - Otherwise treat `p` as a constant and apply `f` directly.
fn handle_path_ap(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let f    = if !args.is_empty() { args[0] } else { Value::unit() };
    let path = if args.len() >= 2  { args[1] } else { Value::unit() };

    // Evaluate path at i1 to obtain the endpoint value.
    let endpoint = match try_call_value(state, path, Value::from_i64(1))? {
        Some(v) => v,
        None => path, // constant path
    };

    // Apply f to the endpoint.
    let result = match try_call_value(state, f, endpoint)? {
        Some(v) => v,
        None => endpoint, // f not callable; return endpoint as best effort
    };
    state.set_reg(dst, result);
    Ok(DispatchResult::Continue)
}

// ============================================================================
// Transport and Composition (0x10-0x1F)
// ============================================================================

/// `transport(type_path, value)` — transport a value along a type path.
///
/// This is the computational heart of cubical transport.  The surviving
/// computational cases after proof erasure are:
///
/// 1. **`ua`-transport**: `type_path` evaluates to an equivalence struct
///    `{ forward, inverse, ... }`.  We extract `forward` (field 0) and apply
///    it to `value`.
/// 2. **Trivial / identity transport**: `type_path` is a constant (e.g. the
///    refl path on a type), so the type does not change and `value` is
///    returned unchanged.
///
/// `type_path` encoding at runtime:
/// - A plain equivalence struct (ptr): produced by `ua(equiv)` which passes
///   the equiv through — field 0 is the forward function.
/// - A `FuncRef` or closure representing `λi. A` where `A` is constant:
///   evaluating at i1 gives back the same type, so transport is identity.
fn handle_transport(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let type_path = if !args.is_empty() { args[0] } else { Value::unit() };
    let value     = if args.len() >= 2  { args[1] } else if !args.is_empty() { args[0] } else { Value::unit() };

    // Case 1: type_path is a heap object (equiv struct from ua).
    // Equiv layout: [forward: fn(A)->B, inverse: fn(B)->A, ...]
    // Field 0 is the forward function.
    if type_path.is_ptr() && !type_path.is_nil() {
        let forward_fn = read_object_field(type_path, 0);
        if let Some(result) = try_call_value(state, forward_fn, value)? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }
        // forward_fn is not callable — fall through to identity.
    }

    // Case 2: type_path is a function / closure representing a constant type
    // path (λi.A).  Evaluating at i1 gives A; since A = A the transport is
    // the identity.  We return value unchanged.
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `hcomp(face, walls, base)` — homogeneous composition.
///
/// `hcomp` fills a box whose open face is given by `base` and whose walls are
/// provided by `walls : I → I → A`.  The filler at `i1` (the lid) is:
///
///   `hcomp(face, walls, base) = walls(face)(i1)`
///
/// Runtime handling:
/// - If `walls` is callable (FuncRef or closure) → call `walls(face)` to get
///   the wall function, then call that at `i1`.
/// - Otherwise → return `base` (the open face is the best approximation).
fn handle_hcomp(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let face  = if !args.is_empty() { args[0] } else { Value::from_i64(1) };
    let walls = if args.len() >= 2  { args[1] } else { Value::unit() };
    let base  = if args.len() >= 3  { args[2] } else if !args.is_empty() { args[args.len() - 1] } else { Value::unit() };

    // Try walls(face) to obtain the wall at the given face.
    if let Some(wall_fn) = try_call_value(state, walls, face)? {
        // Try wall_fn(i1) to obtain the lid value.
        if let Some(lid) = try_call_value(state, wall_fn, Value::from_i64(1))? {
            state.set_reg(dst, lid);
            return Ok(DispatchResult::Continue);
        }
        // wall_fn not callable: use it directly as the lid value.
        state.set_reg(dst, wall_fn);
        return Ok(DispatchResult::Continue);
    }

    // walls not callable: composition is trivial, return base.
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

/// `ua(equiv)` — univalence axiom: turn an equivalence into a path of types.
///
/// At runtime, the equiv struct is the computational content.  We pass it
/// through so that `transport` can later extract `equiv.forward`.
fn handle_ua(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let equiv = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, equiv);
    Ok(DispatchResult::Continue)
}

/// `ua_inv(path)` — inverse of `ua`: extract equivalence from a path of types.
///
/// The path (as produced by `ua`) carries the equiv as its runtime value.
/// We pass it through unchanged.
fn handle_ua_inv(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let path = if !args.is_empty() { args[0] } else { Value::unit() };
    state.set_reg(dst, path);
    Ok(DispatchResult::Continue)
}

/// `equiv_fwd(equiv, value)` — apply the forward direction of an equivalence.
///
/// Equiv struct layout: [forward: fn(A)->B, inverse: fn(B)->A, ...]
///
/// 1. Read field 0 from the equiv object to get the forward function.
/// 2. Call it with `value`.
/// 3. If field 0 is not callable, return `value` unchanged (identity fallback).
fn handle_equiv_fwd(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let equiv = if !args.is_empty() { args[0] } else { Value::unit() };
    let value = if args.len() >= 2  { args[1] } else { Value::unit() };

    if equiv.is_ptr() && !equiv.is_nil() {
        let forward_fn = read_object_field(equiv, 0);
        if let Some(result) = try_call_value(state, forward_fn, value)? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }
    }

    // Fallback: identity (equiv not a recognisable struct, or forward not callable).
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}

/// `equiv_bwd(equiv, value)` — apply the inverse direction of an equivalence.
///
/// Equiv struct layout: [forward: fn(A)->B, inverse: fn(B)->A, ...]
///
/// 1. Read field 1 from the equiv object to get the inverse function.
/// 2. Call it with `value`.
/// 3. If field 1 is not callable, return `value` unchanged (identity fallback).
fn handle_equiv_bwd(state: &mut InterpreterState) -> InterpreterResult<DispatchResult> {
    let (dst, args) = read_cubical_args(state)?;
    let equiv = if !args.is_empty() { args[0] } else { Value::unit() };
    let value = if args.len() >= 2  { args[1] } else { Value::unit() };

    if equiv.is_ptr() && !equiv.is_nil() {
        let inverse_fn = read_object_field(equiv, 1);
        if let Some(result) = try_call_value(state, inverse_fn, value)? {
            state.set_reg(dst, result);
            return Ok(DispatchResult::Continue);
        }
    }

    // Fallback: identity.
    state.set_reg(dst, value);
    Ok(DispatchResult::Continue)
}
