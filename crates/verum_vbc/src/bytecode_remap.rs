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

#[derive(Debug)]
pub(crate) enum NamedOperandError {
    Missing(u32),
    Malformed,
}

/// Resolve only string operands. Function and type slots have independent namespaces.
pub(crate) fn rewrite_named_operands(
    instruction: &mut Instruction,
    mut lookup: impl FnMut(u32) -> Option<u32>,
) -> Result<(), NamedOperandError> {
    use crate::encoding::{decode_reg, decode_varint, encode_varint};

    let mut map = |id: u32| lookup(id).ok_or(NamedOperandError::Missing(id));
    match instruction {
        Instruction::CallM { method_id: id, .. }
        | Instruction::GetFieldNamed { name: id, .. }
        | Instruction::SetFieldNamed { name: id, .. }
        | Instruction::Panic { message_id: id }
        | Instruction::Assert { message_id: id, .. }
        | Instruction::CtxGet { ctx_type: id, .. }
        | Instruction::CtxProvide { ctx_type: id, .. } => *id = map(*id)?,
        Instruction::CtxCheckNegative { ctx_type, func_name } => {
            *ctx_type = map(*ctx_type)?;
            *func_name = map(*func_name)?;
        }
        // A nonzero comparison hint carries the encoded receiver-name offset plus one.
        Instruction::CmpG { protocol_id, .. } if *protocol_id != 0 => {
            *protocol_id = map(*protocol_id - 1)?.checked_add(1).ok_or(NamedOperandError::Malformed)?;
        }
        Instruction::CbgrExtended { sub_op, operands }
            if *sub_op == crate::instruction::CbgrSubOpcode::RefFieldNamed as u8 =>
        {
            let mut offset = 0;
            for _ in 0..2 {
                decode_reg(operands, &mut offset).map_err(|_| NamedOperandError::Malformed)?;
            }
            let start = offset;
            let name = decode_varint(operands, &mut offset).map_err(|_| NamedOperandError::Malformed)?;
            let name = u32::try_from(name).map_err(|_| NamedOperandError::Malformed)?;
            if offset != operands.len() { return Err(NamedOperandError::Malformed); }
            let mapped = map(name)?;
            operands.truncate(start);
            encode_varint(u64::from(mapped), operands);
        }
        _ => {}
    }
    Ok(())
}

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
    /// Map a string-pool id into the destination module (identity by default).
    fn map_string(&self, src: StringId) -> StringId {
        src
    }
    /// Map a function id into the destination module (identity by default).
    fn map_function(&self, src: FunctionId) -> FunctionId {
        src
    }
    /// Map a type id into the destination module (identity by default).
    fn map_type_id(&self, src: TypeId) -> TypeId {
        src
    }
    /// Map a constant-pool id into the destination module (identity by default).
    fn map_const(&self, src: ConstId) -> ConstId {
        src
    }
    /// Map a protocol id into the destination module (identity by default).
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
        Instruction::BindMethodReceiver { type_ref }
        | Instruction::DropTyped { type_ref, .. }
        | Instruction::TypeResource { type_ref, .. }
        | Instruction::TypeLayout { type_ref, .. }
        | Instruction::LoadT { type_ref, .. } => {
            rewrite_type_ref_ids(type_ref, remap);
        }
        Instruction::SetCallWitness { type_args } => {
            for ty in type_args { rewrite_type_ref_ids(ty, remap); }
        }
        Instruction::CallG { func_id, type_args, .. } => {
            *func_id = remap.map_function(FunctionId(*func_id)).0;
            for ty in type_args { rewrite_type_ref_ids(ty, remap); }
        }
        Instruction::NewClosure { func_id, capture_types, lexical_types, .. } => {
            for ty in lexical_types { rewrite_type_ref_ids(ty, remap); }
            *func_id = remap.map_function(FunctionId(*func_id)).0;
            if let Some(types) = capture_types {
                for ty in types.iter_mut().flatten() { rewrite_type_ref_ids(ty, remap); }
            }
        }
        // --- Constant pool index ---
        Instruction::LoadK { const_id, .. } => {
            *const_id = remap.map_const(ConstId(*const_id)).0;
        }
        Instruction::MetaQuote { bytes_const_id, .. } => {
            *bytes_const_id = remap.map_const(ConstId(*bytes_const_id)).0;
        }
        // --- Function table index ---
        Instruction::Call { func_id, .. }
        | Instruction::TailCall { func_id, .. }
        | Instruction::Spawn { func_id, .. }
        | Instruction::GenCreate { func_id, .. } => {
            *func_id = remap.map_function(FunctionId(*func_id)).0;
        }
        // `CallM.method_id` is a STRING id (interned method name —
        // used by the interpreter's `dispatch_user_method` to resolve
        // the receiver-type-qualified function name at dispatch time).
        // It is NOT a function-id and MUST NOT be passed through
        // `func_id_remap` — doing so silently corrupts the StringId
        // into a stale FunctionId that the runtime then interprets
        // as a StringId pointing to arbitrary strings in the module's
        // string table (observed at runtime as bogus method names
        // like `addr_len`, `XACK unexpected:`, `)\n`, …).
        //
        // The string-id remap pass that runs BEFORE this function
        // (`remap_archive_string_operands` in `codegen/mod.rs`)
        // already converts the archive's `method_id` to a codegen-
        // local StringId; this pass should leave it alone.
        Instruction::CallM { .. } => {}
        // FIELD-ACCESS-BYNAME-1: `name` is a STRING id (field name) —
        // exactly the CallM.method_id lifecycle: rebased by
        // `remap_archive_string_operands` / build_module, NOT by this
        // pass. Explicit no-op arm so a future id-space here is a
        // conscious decision, not catch-all drift.
        Instruction::GetFieldNamed { .. } | Instruction::SetFieldNamed { .. } => {}
        // --- Type table index ---
        Instruction::New { type_id, .. }
        | Instruction::NewG { type_id, .. }
        | Instruction::MetaReflect { type_id, .. }
        | Instruction::MakeVariantTyped { type_id, .. } => {
            *type_id = remap.map_type_id(TypeId(*type_id)).0;
        }
        // Spec/Guard carry a TYPE-table index too (T0447: Spec is the
        // interior-pointer pointee fact channel — an unremapped id
        // resolves to the WRONG descriptor in the merged module and
        // the AOT consumer silently derives no layout marks).
        Instruction::Spec { expected_type, .. }
        | Instruction::Guard { expected_type, .. } => {
            *expected_type = remap.map_type_id(TypeId(*expected_type)).0;
        }
        Instruction::MakePi { return_type_id, .. } => {
            *return_type_id = remap.map_type_id(TypeId(*return_type_id)).0;
        }
        // --- Protocol table index ---
        Instruction::BinaryG { protocol_id, .. } => {
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

/// Rebase every nested nominal TypeId without changing generic parameter IDs.
pub fn rewrite_type_ref_ids(ty: &mut crate::types::TypeRef, remap: &dyn IdRemap) {
    use crate::types::TypeRef;
    match ty {
        TypeRef::Concrete(id) => *id = remap.map_type_id(*id),
        TypeRef::RecordFieldProjection { owner, .. } => *owner = remap.map_type_id(*owner),
        TypeRef::Instantiated { base, args } => {
            *base = remap.map_type_id(*base);
            for arg in args { rewrite_type_ref_ids(arg, remap); }
        }
        TypeRef::Function { params, return_type, .. }
        | TypeRef::Rank2Function { params, return_type, .. } => {
            for param in params { rewrite_type_ref_ids(param, remap); }
            rewrite_type_ref_ids(return_type, remap);
        }
        TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => rewrite_type_ref_ids(inner, remap),
        TypeRef::Tuple(elements) => { for element in elements { rewrite_type_ref_ids(element, remap); } }
        TypeRef::Array { element, .. } => rewrite_type_ref_ids(element, remap),
        TypeRef::AssociatedProjection { base, qualifier, args, .. } => {
            rewrite_type_ref_ids(base, remap);
            if let Some(application) = qualifier {
                application.protocol = remap.map_protocol(application.protocol);
                for arg in &mut application.args { rewrite_type_ref_ids(arg, remap); }
            }
            for arg in args { rewrite_type_ref_ids(arg, remap); }
        }
        TypeRef::Generic(_) | TypeRef::ConstValue(_) => {}
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
    fn negative_context_ir_preserves_both_declaration_names() {
        let mut instr = Instruction::CtxCheckNegative { ctx_type: 2, func_name: 5 };
        rewrite_named_operands(&mut instr, |id| match id {
            2 => Some(22),
            5 => Some(55),
            _ => None,
        }).unwrap();
        assert!(matches!(instr, Instruction::CtxCheckNegative { ctx_type: 22, func_name: 55 }));
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

    /// **Drift-pin**: `CallM.method_id` is a STRING id (interned
    /// method name), NOT a function id.  The function-id remap
    /// pass MUST leave it untouched — string-id remap is the
    /// separate `remap_archive_string_operands` pre-pass in
    /// `codegen/mod.rs`.  Without this invariant, the second pass
    /// silently corrupts every cross-module `CallM` by treating its
    /// `method_id` as a function id, producing runtime panics like
    /// `method 'addr_len' not found on Int` and
    /// `method 'XACK unexpected:' not found on Int` (the corrupted
    /// id points to arbitrary strings in the module's string table).
    #[test]
    fn function_remap_leaves_callm_method_id_untouched() {
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
            Instruction::CallM { method_id, .. } => assert_eq!(
                method_id, 7,
                "CallM.method_id is a StringId; func-id remap must leave it alone"
            ),
            _ => unreachable!(),
        }
    }
}
