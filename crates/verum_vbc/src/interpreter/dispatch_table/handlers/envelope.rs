//! The single enforcement authority for extended-carrier stream advance.
//!
//! Every extended carrier — `FfiExtended`, `MathExtended`, `CbgrExtended`,
//! `ArithExtended`, `CharExtended`, `TextExtended`, `LogExtended`,
//! `MemExtended`, `SimdExtended`, `GpuExtended`, `TensorExtended` — is
//! encoded by `encode_instruction` as
//!
//! ```text
//! [opcode] [sub_op:u8] [varint operand_byte_count] [operand bytes]
//! ```
//!
//! The length varint is written from `operands.len()`, so it is correct BY
//! CONSTRUCTION: it always describes exactly the bytes the emitter packed,
//! whatever the per-sub-op arity happens to be.
//!
//! That makes the envelope — not the arm's read sequence — the authority on
//! where the next instruction begins. [`dispatch_enveloped`] is the ONE place
//! that acts on it. A family body reads whatever operands it wants; when it
//! returns, pc is repositioned to `operands_start + operand_byte_count`
//! unconditionally. An arm that reads too few or too many registers can still
//! compute a wrong VALUE, but it can no longer desync the instruction stream.
//!
//! # Why the authority lives outside the family body
//!
//! The previous convention placed a `set_pc` at the tail of each family
//! handler, inside the same function as the `match`. That is bypassable: any
//! `return` inside an arm — a fast path, a graceful-degradation path, a guard
//! — jumps straight past the tail and desyncs the stream. The defect class
//! recurred for exactly this reason (T0177 gpu_memset SIGSEGV, T0193, task #8
//! MemExtended NullPointer, and the `Realloc` fast path of T0429 that
//! re-opened the crack inside the very handler written to close it).
//!
//! Hoisting the reposition into the CALLER makes the failure structurally
//! impossible rather than merely discouraged: an early `return` in an arm
//! returns into [`dispatch_enveloped`], which then repositions. Early returns
//! are harmless by construction, so no arm needs converting and no future arm
//! can reintroduce the bug.
//!
//! # Why a new family cannot forget the envelope
//!
//! [`ExtendedFamilyBody`] takes the sub-op byte as a parameter, which makes it
//! type-incompatible with the dispatch table's
//! `fn(&mut InterpreterState) -> InterpreterResult<DispatchResult>`. A family
//! body therefore CANNOT be registered in `DISPATCH_TABLE` directly — it only
//! type-checks when routed through [`dispatch_enveloped`]. The guarantee is
//! enforced by rustc, not by convention.
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

use super::super::super::error::InterpreterResult;
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::{read_u8, read_varint};

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
    let operand_byte_count = read_varint(state)?;
    let operands_start = state.pc();

    let result = body(state, sub_op);

    state.set_pc(operands_start.wrapping_add(operand_byte_count as u32));
    result
}
