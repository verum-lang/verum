//! The single enforcement authority for extended-carrier stream advance.
//!
//! The length prefix determines which bytes the handler may read and where
//! execution resumes. Surplus bytes inside a valid envelope are skipped.
//! Reading an operand past the declared length fails, including when the
//! following instruction happens to begin with a valid register byte.
//! Each call frame owns its limit so nested interpretation has its own bounds.
//!
//! # Control transfer
//!
//! No extended arm performs a tail control transfer: none returns
//! `DispatchResult::Return`/`Yield`/`FinalReturn`, so repositioning is always
//! correct. The one arm that runs nested interpretation — the GPU
//! kernel-launch simulation, which drives `dispatch_loop_table_with_entry_depth`
//! per simulated thread — returns synchronously with its frames popped, and
//! *relies* on this reposition to restore pc afterwards. Should a future arm
//! ever need to leave pc where it put it, it must be given an explicit typed
//! escape here rather than a bare `return`.

use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::{read_operand, read_u8};

/// The body of one extended family: the `match sub_op { … }` and nothing else.
///
/// Receives the already-decoded sub-op byte and reads its own operands from
/// the stream. It must NOT read the sub-op or the length varint (the envelope
/// owns those) and must NOT reposition pc (the envelope owns that too).
pub(in super::super) type ExtendedFamilyBody =
    fn(&mut InterpreterState, u8) -> InterpreterResult<DispatchResult>;

/// Decode the envelope, run the family body, and reposition pc from the
/// declared operand length — on every path, including `Err` and any early
/// `return` taken inside an arm.
///
/// The `Err` path is deliberately repositioned too: a diagnostic that reports
/// pc should name the instruction that failed, not the misaligned byte after
/// it.
#[inline]
pub(in super::super) fn dispatch_enveloped(
    state: &mut InterpreterState,
    body: ExtendedFamilyBody,
) -> InterpreterResult<DispatchResult> {
    let sub_op = read_u8(state)?;
    let range = read_operand(state, crate::encoding::decode_payload_range)?;
    let depth = state.call_stack.depth() - 1; // read_operand verified the current frame
    let frame = state.call_stack.current_mut().unwrap();
    let previous_end = std::mem::replace(&mut frame.operand_end, range.end as u32);
    frame.pc = range.start as u32;

    let result = body(state, sub_op);

    // Nested execution may have left its failing frame above the caller.
    let frame =
        state
            .call_stack
            .at_mut(depth)
            .ok_or_else(|| InterpreterError::InvalidBytecode {
                pc: range.start,
                message: "extended instruction removed its caller frame".into(),
            })?;
    frame.operand_end = previous_end;
    frame.pc = range.end as u32;
    result
}
