//! Generic Extended (`Opcode::Extended` = `0x1F`) opcode handler.
//!
//! Implements #167 Part A — the general-purpose extension-byte scheme.
//! The dispatcher reads a single sub-op byte, then routes to the
//! sub-op handler.  Sub-op `0x00` is reserved as a forward-compat
//! anchor; encoders must never emit it, decoders accept-and-skip it.
//!
//! Future #167 Part B work (and any later first-class instruction
//! that doesn't fit an existing extension namespace) wires its
//! handler here.

use crate::instruction::{ExtendedSubOpcode, Opcode};
use super::super::super::error::{InterpreterError, InterpreterResult};
use super::super::super::state::InterpreterState;
use super::super::DispatchResult;
use super::bytecode_io::*;

/// Dispatcher for `Opcode::Extended` (0x1F) — #167 Part A.
///
/// Format: `[0x1F] [sub_op:u8] [operands...]`.  The sub-op byte
/// selects the extended-instruction kind from a 256-entry secondary
/// space.  An unknown sub-op surfaces `InterpreterError::NotImplemented`
/// with `opcode = Some(Opcode::Extended)` so the caller can identify
/// the extension family.
pub(in super::super) fn handle_extended(
    state: &mut InterpreterState,
) -> InterpreterResult<DispatchResult> {
    let sub_op_byte = read_u8(state)?;
    match ExtendedSubOpcode::from_byte(sub_op_byte) {
        Some(ExtendedSubOpcode::Reserved) => {
            // Forward-compat anchor.  Encoders MUST NOT emit this
            // sub-op; decoders accept it as a no-op so a future
            // extension that lands here can roll out without breaking
            // older interpreters.
            Ok(DispatchResult::Continue)
        }
        None => Err(InterpreterError::NotImplemented {
            feature: "Extended sub-opcode",
            opcode: Some(Opcode::Extended),
        }),
    }
}
