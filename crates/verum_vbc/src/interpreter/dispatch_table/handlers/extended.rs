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
        Some(ExtendedSubOpcode::ProcessExit) => {
            // Format: `[0x1F][0x10][reg:u16]`. Read the register holding
            // the exit code, flush stdio (Rust's stdout is line-buffered
            // by default — an immediate `process::exit` could otherwise
            // drop the tail of a partial line that `print(...)` emitted
            // just before this call), and terminate the process.
            //
            // First-class divergent instruction: dispatch never returns,
            // so the dispatch loop's `Ok(DispatchResult::Continue)` arm
            // is structurally unreachable.
            let code_reg = super::bytecode_io::read_reg(state)?;
            let code = state.get_reg(code_reg).as_integer_compatible() as i32;
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            std::process::exit(code);
        }
        None => Err(InterpreterError::NotImplemented {
            feature: "Extended sub-opcode",
            opcode: Some(Opcode::Extended),
        }),
    }
}
