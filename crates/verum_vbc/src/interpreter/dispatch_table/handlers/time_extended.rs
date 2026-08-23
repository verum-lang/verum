//! `Opcode::TimeExtended` (0xF0) — the time-clock family's honest home
//! (T0852).  These arms lived in `ffi_extended.rs` as
//! `SystemSubOpcode::Time*` squatters; the operations have nothing to
//! do with FFI.
//!
//! Wire: standard extended envelope `[0xF0][sub_op][len][operands]`
//! via `dispatch_enveloped`.

use super::super::DispatchResult;
use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::bytecode_io::read_reg;
use super::envelope::dispatch_enveloped;
use super::method_dispatch::{monotonic_nanos_shared, realtime_nanos_shared};
use crate::instruction::{Opcode, TimeSubOpcode};
use crate::interpreter::error::InterpreterError;
use crate::value::Value;

pub(in super::super) fn handle_time_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    dispatch_enveloped(state, time_extended_body)
}

fn time_extended_body(
    state: &mut InterpreterState,
    sub_op_byte: u8,
) -> InterpreterResult<DispatchResult> {
    match TimeSubOpcode::from_byte(sub_op_byte) {
        Some(TimeSubOpcode::MonotonicNanos) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(monotonic_nanos_shared()));
            Ok(DispatchResult::Continue)
        }
        Some(TimeSubOpcode::RealtimeNanos) => {
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(realtime_nanos_shared()));
            Ok(DispatchResult::Continue)
        }
        Some(TimeSubOpcode::MonotonicRawNanos) => {
            // Same as MonotonicNanos for the interpreter (no NTP
            // distinction at Tier 0).
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(monotonic_nanos_shared()));
            Ok(DispatchResult::Continue)
        }
        Some(TimeSubOpcode::SleepNanos) => {
            let nanos_reg = read_reg(state)?;
            // BOXED-INT-OPERAND-1: the duration can arrive boxed
            // (`ns as UInt64`) — the canonical maybe-boxed reader, or
            // the sleep silently no-ops on box bits.
            let nanos = state.get_reg(nanos_reg).as_integer_compatible();
            if nanos > 0 {
                std::thread::sleep(std::time::Duration::from_nanos(nanos as u64));
            }
            Ok(DispatchResult::Continue)
        }
        Some(TimeSubOpcode::ThreadCpuNanos) | Some(TimeSubOpcode::ProcessCpuNanos) => {
            // Tier-0 approximates cpu-time clocks with the monotonic
            // clock (single-threaded interpreter).
            let dst = read_reg(state)?;
            state.set_reg(dst, Value::from_i64(monotonic_nanos_shared()));
            Ok(DispatchResult::Continue)
        }
        Some(TimeSubOpcode::SleepMillis) => {
            let ms_reg = read_reg(state)?;
            let ms = state.get_reg(ms_reg).as_integer_compatible();
            if ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            }
            Ok(DispatchResult::Continue)
        }
        None => Err(InterpreterError::NotImplemented {
            feature: "time_extended sub-opcode",
            opcode: Some(Opcode::TimeExtended),
        }),
    }
}
