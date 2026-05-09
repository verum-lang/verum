//! Cross-module ID remap helpers for bytecode body copy.
//!
//! Two consumers reuse this surface:
//!
//!   * [`crate::linker::VbcLinker`] — archive-wide merge that remaps
//!     every input module's source IDs into the linker output's
//!     contiguous ID space.
//!   * `verum_compiler::archive_ctx_loader` — lazy per-module body
//!     copy that pulls only the wanted stdlib functions out of the
//!     embedded archive and injects them into the user codegen's
//!     namespace.
//!
//! Both clients implement [`IdRemap`] over their own remap tables and
//! pass the trait object to [`rewrite_instruction_ids`] which updates
//! every id-bearing operand in a decoded [`Instruction`]. The
//! per-instruction logic lives ONLY here so a new id-bearing
//! `Instruction` variant added to `instruction.rs` is covered for
//! both clients in one place.

use crate::instruction::Instruction;
use crate::module::{ConstId, FunctionId};
use crate::types::{ProtocolId, StringId, TypeId};

/// Cross-module ID remap surface. Each method takes a source-module
/// ID and returns the destination-module ID it should be rewritten
/// to. The default implementation is identity, so an implementor only
/// needs to override the maps it actually carries.
///
/// **Invariant**: `rewrite_instruction_ids` calls these methods on
/// every id operand of a decoded instruction. Identity-fallback for
/// unmapped ids is the contract — it lets cross-archive references
/// (kernel intrinsics dispatched via the global function table,
/// stdlib types declared in a sibling cog) resolve at runtime instead
/// of failing at link time.
pub trait IdRemap {
    fn map_string(&self, src: StringId) -> StringId {
        src
    }
    fn map_function(&self, src: FunctionId) -> FunctionId {
        src
    }
    fn map_type_id(&self, src: TypeId) -> TypeId {
        src
    }
    fn map_const(&self, src: ConstId) -> ConstId {
        src
    }
    fn map_protocol(&self, src: ProtocolId) -> ProtocolId {
        src
    }
}

/// In-place rewrite of every id-bearing operand on `instr`. No-op on
/// id-free variants. Mirror of the `rewrite_instruction_ids` private
/// helper that lived in `linker.rs` before this module was extracted —
/// linker still uses it via the [`IdRemap`] trait.
///
/// **Maintenance contract**: every new id-bearing instruction variant
/// added to `instruction.rs` MUST be added here. Missing a variant
/// surfaces as a runtime `FunctionNotFound` / `TypeNotFound` panic
/// when user code transitively calls into the unrewritten body. The
/// archive-driven body-copy path (which is hot) propagates the same
/// invariant — drift HERE breaks BOTH stdlib body merge AND linker
/// archive merge in lockstep.
pub fn rewrite_instruction_ids(instr: &mut Instruction, remap: &dyn IdRemap) {
    match instr {
        // --- Constant pool index ---
        Instruction::LoadK { const_id, .. } => {
            *const_id = remap.map_const(ConstId(*const_id)).0;
        }
        Instruction::MetaQuote { bytes_const_id, .. } => {
            *bytes_const_id = remap.map_const(ConstId(*bytes_const_id)).0;
        }
        // --- Function table index ---
        Instruction::Call { func_id, .. }
        | Instruction::CallG { func_id, .. }
        | Instruction::TailCall { func_id, .. }
        | Instruction::NewClosure { func_id, .. }
        | Instruction::Spawn { func_id, .. }
        | Instruction::GenCreate { func_id, .. } => {
            *func_id = remap.map_function(FunctionId(*func_id)).0;
        }
        Instruction::CallM { method_id, .. } => {
            // Method table is a slice of the function table; method_id
            // is a flat function-id under the hood.
            *method_id = remap.map_function(FunctionId(*method_id)).0;
        }
        // --- Type table index ---
        Instruction::New { type_id, .. }
        | Instruction::NewG { type_id, .. }
        | Instruction::MetaReflect { type_id, .. }
        | Instruction::MakeVariantTyped { type_id, .. } => {
            *type_id = remap.map_type_id(TypeId(*type_id)).0;
        }
        Instruction::MakePi { return_type_id, .. } => {
            *return_type_id = remap.map_type_id(TypeId(*return_type_id)).0;
        }
        // --- Protocol table index ---
        Instruction::BinaryG { protocol_id, .. }
        | Instruction::CmpG { protocol_id, .. } => {
            *protocol_id = remap.map_protocol(ProtocolId(*protocol_id)).0;
        }
        // --- FfiExtended::CreateCallback embedded func_id ---
        // Format: dst:reg (variable-length), fn_id:u32, signature_idx:u32
        // Register encoding: 1 byte if < 0x80, 2 bytes otherwise.
        Instruction::FfiExtended { sub_op, operands } if *sub_op == 0x50 => {
            let fn_id_offset = if !operands.is_empty() && operands[0] & 0x80 != 0 {
                2
            } else {
                1
            };
            if operands.len() >= fn_id_offset + 4 {
                let old_fn_id = u32::from_le_bytes([
                    operands[fn_id_offset],
                    operands[fn_id_offset + 1],
                    operands[fn_id_offset + 2],
                    operands[fn_id_offset + 3],
                ]);
                let new_fn_id = remap.map_function(FunctionId(old_fn_id)).0;
                if new_fn_id != old_fn_id {
                    operands[fn_id_offset..fn_id_offset + 4]
                        .copy_from_slice(&new_fn_id.to_le_bytes());
                }
            }
        }
        // Everything else has no id operand.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::Reg;

    struct IdentityRemap;
    impl IdRemap for IdentityRemap {}

    struct FunctionShift {
        delta: u32,
    }
    impl IdRemap for FunctionShift {
        fn map_function(&self, src: FunctionId) -> FunctionId {
            FunctionId(src.0 + self.delta)
        }
    }

    #[test]
    fn identity_leaves_call_unchanged() {
        let mut instr = Instruction::Call {
            dst: Reg(0),
            func_id: 42,
            args: crate::instruction::RegRange {
                start: Reg(0),
                count: 0,
            },
        };
        rewrite_instruction_ids(&mut instr, &IdentityRemap);
        match instr {
            Instruction::Call { func_id, .. } => assert_eq!(func_id, 42),
            _ => unreachable!(),
        }
    }

    #[test]
    fn function_remap_shifts_call_id() {
        let mut instr = Instruction::Call {
            dst: Reg(0),
            func_id: 7,
            args: crate::instruction::RegRange {
                start: Reg(0),
                count: 0,
            },
        };
        rewrite_instruction_ids(&mut instr, &FunctionShift { delta: 100 });
        match instr {
            Instruction::Call { func_id, .. } => assert_eq!(func_id, 107),
            _ => unreachable!(),
        }
    }

    #[test]
    fn function_remap_covers_callm() {
        let mut instr = Instruction::CallM {
            dst: Reg(0),
            receiver: Reg(0),
            method_id: 7,
            args: crate::instruction::RegRange {
                start: Reg(0),
                count: 0,
            },
        };
        rewrite_instruction_ids(&mut instr, &FunctionShift { delta: 100 });
        match instr {
            Instruction::CallM { method_id, .. } => assert_eq!(method_id, 107),
            _ => unreachable!(),
        }
    }
}
