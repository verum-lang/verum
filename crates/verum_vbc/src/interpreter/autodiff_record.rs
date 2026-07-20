//! Forward tape recording for the interpreter's reverse-mode autodiff.
//!
//! This is the single place where forward arithmetic reaches the gradient
//! tape. Arithmetic handlers call [`record_binop`] / [`record_unop`]
//! unconditionally; both open with one test of `state.grad_recording` and
//! return immediately when no gradient scope is active. That test is the only
//! cost the ordinary interpreter path pays — a single, perfectly predicted
//! branch on a `bool` that already shares a cache line with the hot
//! interpreter state. Every byte of real recording work lives in `#[cold]`
//! out-of-line functions, so the common path spends no registers and no
//! instruction-cache footprint on autodiff.
//!
//! Keeping the logic here (rather than copying it into each handler) means a
//! new differentiable opcode is wired by adding one call plus, if the VJP rule
//! needs a saved activation, one arm to the tables in
//! [`saved_for_unop`] / [`saved_for_binop`].

use super::autodiff::{SavedValue, TapeOp, TensorId};
use super::state::InterpreterState;
use super::tensor::{DType, TensorHandle};
use crate::instruction::Reg;

/// Scalars enter the tape as rank-0 tensors so they reuse the one reverse
/// engine and its VJP rules rather than needing a parallel scalar engine.
fn scalar_node_value(v: f64) -> Option<TensorHandle> {
    TensorHandle::full(&[], DType::F64, v)
}

/// Absolute slot identity for a register in the current frame.
///
/// The register file is windowed, so `reg` alone is ambiguous across frames;
/// `reg_base + reg` is what uniquely names a slot at a point in time.
#[inline]
fn slot_of(state: &InterpreterState, reg: Reg) -> u32 {
    state.reg_base() + reg.0 as u32
}

/// Resolves the tape node a register currently holds, creating a leaf node if
/// the register is not tracked.
///
/// A mapping is trusted only while the slot still holds the exact bits
/// recorded with it. Any untracked instruction that overwrote the slot changes
/// those bits, so the stale entry is ignored and the value enters the tape as
/// a fresh leaf — the same treatment a literal constant gets.
fn node_for(state: &mut InterpreterState, reg: Reg, value: f64) -> Option<TensorId> {
    let slot = slot_of(state, reg);
    let bits = value.to_bits();

    if let Some((node, stamp)) = state.grad_reg_nodes.get(&slot) {
        if *stamp == bits {
            return Some(*node);
        }
    }

    let node = state.grad_tape.track_tensor(scalar_node_value(value)?)?;
    state.grad_reg_nodes.insert(slot, (node, bits));
    Some(node)
}

/// Binds a register to the tape node its value was just produced by.
fn bind(state: &mut InterpreterState, reg: Reg, node: TensorId, value: f64) {
    let slot = slot_of(state, reg);
    state.grad_reg_nodes.insert(slot, (node, value.to_bits()));
}

/// Activations a binary rule needs saved, per `compute_vjp`.
fn saved_for_binop(op: TapeOp, av: f64, bv: f64, result: f64) -> Option<Vec<SavedValue>> {
    let scalars: &[f64] = match op {
        // dx = dout * y, dy = dout * x
        TapeOp::Mul => &[av, bv],
        // dx = dout / y, dy = -dout * x / y^2
        TapeOp::Div => &[av, bv],
        // needs x, y and x^y
        TapeOp::Pow => &[av, bv, result],
        // Add/Sub route the cotangent through unchanged.
        _ => &[],
    };
    let mut saved = Vec::with_capacity(scalars.len());
    for s in scalars {
        saved.push(SavedValue::Tensor(scalar_node_value(*s)?));
    }
    Some(saved)
}

/// The single activation a unary rule needs saved, per `compute_vjp`.
///
/// The rules differ in what they want: some consume the output they just
/// produced (`exp`, `tanh`), some the input (`log`, `abs`), and the
/// trigonometric pair wants the *other* function of the input.
fn saved_for_unop(op: TapeOp, xv: f64, result: f64) -> Option<Option<SavedValue>> {
    let scalar = match op {
        TapeOp::Neg => None,
        TapeOp::Exp | TapeOp::Sqrt | TapeOp::Tanh | TapeOp::Sigmoid => Some(result),
        TapeOp::Log | TapeOp::Abs | TapeOp::Atanh => Some(xv),
        TapeOp::Sin => Some(xv.cos()),
        TapeOp::Cos => Some(xv.sin()),
        TapeOp::Sinh => Some(xv.cosh()),
        TapeOp::Cosh => Some(xv.sinh()),
        // Anything reaching here is not on the recorded-op list.
        _ => return None,
    };
    match scalar {
        None => Some(None),
        Some(s) => Some(Some(SavedValue::Tensor(scalar_node_value(s)?))),
    }
}

/// Records a binary float operation when a gradient scope is active.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_binop(
    state: &mut InterpreterState,
    op: TapeOp,
    dst: Reg,
    a: Reg,
    b: Reg,
    av: f64,
    bv: f64,
    result: f64,
) {
    if !state.grad_recording {
        return;
    }
    record_binop_cold(state, op, dst, a, b, av, bv, result);
}

#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn record_binop_cold(
    state: &mut InterpreterState,
    op: TapeOp,
    dst: Reg,
    a: Reg,
    b: Reg,
    av: f64,
    bv: f64,
    result: f64,
) {
    let Some(lhs) = node_for(state, a, av) else {
        return;
    };
    let Some(rhs) = node_for(state, b, bv) else {
        return;
    };
    let Some(out_value) = scalar_node_value(result) else {
        return;
    };
    let Some(out) = state.grad_tape.track_tensor(out_value) else {
        return;
    };
    let Some(saved) = saved_for_binop(op, av, bv, result) else {
        return;
    };

    if state
        .grad_tape
        .record_op(op, &[lhs, rhs], out, saved)
        .is_some()
    {
        bind(state, dst, out, result);
    }
}

/// Records a unary float operation when a gradient scope is active.
#[inline(always)]
pub(crate) fn record_unop(
    state: &mut InterpreterState,
    op: TapeOp,
    dst: Reg,
    src: Reg,
    xv: f64,
    result: f64,
) {
    if !state.grad_recording {
        return;
    }
    record_unop_cold(state, op, dst, src, xv, result);
}

#[cold]
#[inline(never)]
fn record_unop_cold(
    state: &mut InterpreterState,
    op: TapeOp,
    dst: Reg,
    src: Reg,
    xv: f64,
    result: f64,
) {
    let Some(input) = node_for(state, src, xv) else {
        return;
    };
    let Some(out_value) = scalar_node_value(result) else {
        return;
    };
    let Some(out) = state.grad_tape.track_tensor(out_value) else {
        return;
    };
    let Some(saved_slot) = saved_for_unop(op, xv, result) else {
        return;
    };
    let saved = match saved_slot {
        Some(s) => vec![s],
        None => Vec::new(),
    };

    if state.grad_tape.record_op(op, &[input], out, saved).is_some() {
        bind(state, dst, out, result);
    }
}

/// Notes that a float operation with no VJP rule ran inside a gradient scope.
///
/// Such an operation breaks the chain: its result would otherwise re-enter the
/// tape as a fresh leaf and contribute a silent zero to the gradient. Recording
/// the name lets the pullback fail loudly instead of returning a plausible
/// wrong number.
#[inline(always)]
pub(crate) fn note_unsupported(state: &mut InterpreterState, what: &'static str) {
    if !state.grad_recording {
        return;
    }
    note_unsupported_cold(state, what);
}

#[cold]
#[inline(never)]
fn note_unsupported_cold(state: &mut InterpreterState, what: &'static str) {
    if state.grad_unsupported.is_none() {
        state.grad_unsupported = Some(what);
    }
}

// ============================================================================
// Frame boundaries
// ============================================================================
//
// A call copies argument values into fresh callee slots and a return copies the
// result back into a caller slot. Both cross into slots the callee/caller never
// bound, so without carrying the binding across, a differentiated function's
// own parameter would look untracked and re-enter the tape as a leaf — which
// silently disconnects the gradient from `wrt` and yields exactly the 0.0 this
// wiring exists to eliminate.

/// Carries a tape binding from a caller argument register into the callee slot
/// it was copied to.
#[inline(always)]
pub(crate) fn propagate_arg(
    state: &mut InterpreterState,
    caller_base: u32,
    src: Reg,
    callee_base: u32,
    dst: Reg,
) {
    if !state.grad_recording {
        return;
    }
    propagate_arg_cold(state, caller_base, src, callee_base, dst);
}

#[cold]
#[inline(never)]
fn propagate_arg_cold(
    state: &mut InterpreterState,
    caller_base: u32,
    src: Reg,
    callee_base: u32,
    dst: Reg,
) {
    let from = caller_base + src.0 as u32;
    if let Some(entry) = state.grad_reg_nodes.get(&from).copied() {
        state.grad_reg_nodes.insert(callee_base + dst.0 as u32, entry);
    }
}

/// Carries a tape binding along a register-to-register move within one frame.
///
/// A move copies the value but not the binding, so without this the
/// destination would look untracked and the gradient chain would end there.
#[inline(always)]
pub(crate) fn propagate_move(state: &mut InterpreterState, src: Reg, dst: Reg) {
    if !state.grad_recording {
        return;
    }
    propagate_move_cold(state, src, dst);
}

#[cold]
#[inline(never)]
fn propagate_move_cold(state: &mut InterpreterState, src: Reg, dst: Reg) {
    let from = slot_of(state, src);
    let to = slot_of(state, dst);
    match state.grad_reg_nodes.get(&from).copied() {
        Some(entry) => {
            state.grad_reg_nodes.insert(to, entry);
        }
        // The source is untracked, so the destination must not keep an older
        // binding of its own.
        None => {
            state.grad_reg_nodes.remove(&to);
        }
    }
}

/// Rewrites bindings for a tail call, which shuffles arguments down to the
/// start of the *same* frame.
///
/// The whole set is snapshotted before anything is written, so an earlier
/// argument cannot clobber a later one's binding mid-shuffle. Slots whose
/// source was unbound are cleared rather than left stale.
#[inline(always)]
pub(crate) fn propagate_tail_args(
    state: &mut InterpreterState,
    base: u32,
    start: Reg,
    count: u16,
) {
    if !state.grad_recording {
        return;
    }
    propagate_tail_args_cold(state, base, start, count);
}

#[cold]
#[inline(never)]
fn propagate_tail_args_cold(state: &mut InterpreterState, base: u32, start: Reg, count: u16) {
    let snapshot: Vec<Option<(TensorId, u64)>> = (0..count)
        .map(|i| {
            state
                .grad_reg_nodes
                .get(&(base + start.0 as u32 + i as u32))
                .copied()
        })
        .collect();

    for (i, entry) in snapshot.into_iter().enumerate() {
        let slot = base + i as u32;
        match entry {
            Some(e) => {
                state.grad_reg_nodes.insert(slot, e);
            }
            None => {
                state.grad_reg_nodes.remove(&slot);
            }
        }
    }
}

/// Captures the binding of a returned value before its frame is popped.
#[inline(always)]
pub(crate) fn capture_return(state: &mut InterpreterState, src: Reg) {
    if !state.grad_recording {
        return;
    }
    capture_return_cold(state, src);
}

#[cold]
#[inline(never)]
fn capture_return_cold(state: &mut InterpreterState, src: Reg) {
    let slot = slot_of(state, src);
    state.grad_pending_return = state.grad_reg_nodes.get(&slot).copied();
}

/// Re-attaches a captured return binding in the caller's frame.
///
/// The stamp is re-checked against the value actually written, so a binding
/// left over from an unrelated return can never be applied.
#[inline(always)]
pub(crate) fn restore_return(
    state: &mut InterpreterState,
    caller_base: u32,
    return_reg: Reg,
    value_bits: u64,
) {
    if !state.grad_recording {
        return;
    }
    restore_return_cold(state, caller_base, return_reg, value_bits);
}

#[cold]
#[inline(never)]
fn restore_return_cold(
    state: &mut InterpreterState,
    caller_base: u32,
    return_reg: Reg,
    value_bits: u64,
) {
    if let Some((node, stamp)) = state.grad_pending_return.take() {
        if stamp == value_bits {
            state
                .grad_reg_nodes
                .insert(caller_base + return_reg.0 as u32, (node, stamp));
        }
    }
}

/// Opens a recording scope, seeding one leaf node per `wrt` register.
///
/// Called by `GradBegin`. Keeping this here rather than in the handler keeps
/// every register-to-node decision in one module.
pub(crate) fn begin_recording(state: &mut InterpreterState, wrt_regs: &[Reg]) {
    state.grad_reg_nodes.clear();
    state.grad_unsupported = None;

    let mut wrt = Vec::with_capacity(wrt_regs.len());
    for reg in wrt_regs {
        let value = state.get_reg(*reg).as_f64();
        let Some(tensor) = scalar_node_value(value) else {
            continue;
        };
        let Some(node) = state.grad_tape.track_tensor(tensor) else {
            continue;
        };
        bind(state, *reg, node, value);
        wrt.push(node);
    }
    state.grad_tape.set_wrt(wrt);
    state.grad_recording = true;
}

/// Closes the recording scope and retains the tape for a later pullback.
///
/// Returns the handle the pullback passes back to reach this tape. Called by
/// `GradEnd`.
pub(crate) fn finish_recording(state: &mut InterpreterState, output_reg: Reg) -> Option<u32> {
    state.grad_recording = false;

    let value = state.get_reg(output_reg).as_f64();
    let slot = slot_of(state, output_reg);
    let output = state
        .grad_reg_nodes
        .get(&slot)
        .filter(|(_, stamp)| *stamp == value.to_bits())
        .map(|(node, _)| *node);

    state.grad_reg_nodes.clear();
    state.grad_tape.finish_scope(output)
}

/// Maps a float unary sub-opcode (see `handlers::float_arith`) to its tape
/// operation, or `None` when no VJP rule covers it.
pub(crate) fn unary_float_tape_op(sub_op: u8) -> Option<TapeOp> {
    Some(match sub_op {
        0 => TapeOp::Neg,
        1 => TapeOp::Abs,
        2 => TapeOp::Sqrt,
        3 => TapeOp::Exp,
        4 => TapeOp::Log,
        5 => TapeOp::Sin,
        6 => TapeOp::Cos,
        14 => TapeOp::Sinh,
        15 => TapeOp::Cosh,
        16 => TapeOp::Tanh,
        19 => TapeOp::Atanh,
        _ => return None,
    })
}
