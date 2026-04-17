//! Instruction bytecode encoding and decoding.
//!
//! This module provides functions to encode [`Instruction`] values to binary bytecode
//! and decode them back. The encoding is designed for:
//!
//! - **Compactness**: Common operations use fewer bytes
//! - **Determinism**: Same instruction always produces same bytes
//! - **Roundtrip safety**: `decode(encode(instr)) == instr`
//!
//! # Encoding Format
//!
//! Each instruction is encoded as:
//! ```text
//! [opcode:u8] [operands...]
//! ```
//!
//! ## Operand Encoding
//!
//! | Type | Encoding |
//! |------|----------|
//! | `Reg` | 1-2 bytes (r0-127 = 1 byte, r128+ = 2 bytes) |
//! | `RegRange` | Reg + u8 count |
//! | `i8` | 1 byte fixed |
//! | `i32` | Signed VarInt (ZigZag) |
//! | `i64` | Signed VarInt (ZigZag) |
//! | `u32` | VarInt |
//! | `f64` | 8 bytes little-endian |
//! | `bool` | 1 byte (0/1) |
//! | `Vec<T>` | VarInt length + elements |
//! | `Option<T>` | 1 byte flag + optional value |
//! | `Enum` | 1 byte discriminant |
//!
//! # Examples
//!
//! ```ignore
//! use verum_vbc::bytecode::{encode_instruction, decode_instruction};
//! use verum_vbc::instruction::{Instruction, Reg};
//!
//! // Encode an instruction
//! let instr = Instruction::Mov { dst: Reg(0), src: Reg(1) };
//! let mut bytes = Vec::new();
//! encode_instruction(&instr, &mut bytes);
//!
//! // Decode it back
//! let mut offset = 0;
//! let decoded = decode_instruction(&bytes, &mut offset).unwrap();
//! assert_eq!(decoded, instr);
//! ```

use crate::encoding::{
    decode_f32, decode_f64, decode_reg, decode_reg_range, decode_signed_varint, decode_u8,
    decode_varint, encode_f64, encode_reg, encode_reg_range, encode_signed_varint, encode_varint,
};
use crate::error::{VbcError, VbcResult};
use crate::instruction::{
    BinaryFloatOp, BinaryGenericOp, BinaryIntOp, BitwiseOp, CmpSubOpcode, CompareOp,
    FloatToIntMode, GpuSubOpcode, GradMode, Instruction, Opcode, Reg, TensorBinaryOp,
    TensorCumulativeOp, TensorDType, TensorExtSubOpcode, TensorPoolOp, TensorReduceOp,
    TensorSubOpcode, TensorUnaryOp, UnaryFloatOp, UnaryIntOp,
};
#[cfg(test)]
use crate::instruction::RegRange;
use crate::types::{CbgrTier, ContextRef, Mutability, TypeId, TypeParamId, TypeRef};

// ============================================================================
// Instruction Encoding
// ============================================================================

/// Encodes an instruction to bytecode.
///
/// Appends the encoded bytes to `output` and returns the number of bytes written.
pub fn encode_instruction(instr: &Instruction, output: &mut Vec<u8>) -> usize {
    let start_len = output.len();

    match instr {
        // ====================================================================
        // Data Movement
        // ====================================================================
        Instruction::Mov { dst, src } => {
            output.push(Opcode::Mov.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::LoadK { dst, const_id } => {
            output.push(Opcode::LoadK.to_byte());
            encode_reg(*dst, output);
            encode_varint(*const_id as u64, output);
        }

        Instruction::LoadI { dst, value } => {
            output.push(Opcode::LoadI.to_byte());
            encode_reg(*dst, output);
            encode_signed_varint(*value, output);
        }

        Instruction::LoadF { dst, value } => {
            output.push(Opcode::LoadF.to_byte());
            encode_reg(*dst, output);
            encode_f64(*value, output);
        }

        Instruction::LoadTrue { dst } => {
            output.push(Opcode::LoadTrue.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::LoadFalse { dst } => {
            output.push(Opcode::LoadFalse.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::LoadUnit { dst } => {
            output.push(Opcode::LoadUnit.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::LoadT { dst, type_ref } => {
            output.push(Opcode::LoadT.to_byte());
            encode_reg(*dst, output);
            encode_type_ref(type_ref, output);
        }

        Instruction::LoadSmallI { dst, value } => {
            output.push(Opcode::LoadSmallI.to_byte());
            encode_reg(*dst, output);
            output.push(*value as u8);
        }

        // ====================================================================
        // Type Conversions
        // ====================================================================
        Instruction::CvtIF { dst, src } => {
            output.push(Opcode::CvtIF.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::CvtFI { mode, dst, src } => {
            output.push(Opcode::CvtFI.to_byte());
            output.push(mode.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::CvtIC { dst, src } => {
            output.push(Opcode::CvtIC.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::CvtCI { dst, src } => {
            output.push(Opcode::CvtCI.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::CvtBI { dst, src } => {
            output.push(Opcode::CvtBI.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::CvtToI { dst, src } => {
            output.push(Opcode::CvtToI.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::CvtToF { dst, src } => {
            output.push(Opcode::CvtToF.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        // ====================================================================
        // Arithmetic
        // ====================================================================
        Instruction::BinaryI { op, dst, a, b } => {
            output.push(binary_int_op_to_opcode(*op).to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::BinaryF { op, dst, a, b } => {
            output.push(binary_float_op_to_opcode(*op).to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::BinaryG {
            op,
            dst,
            a,
            b,
            protocol_id,
        } => {
            output.push(binary_generic_op_to_opcode(*op).to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
            encode_varint(*protocol_id as u64, output);
        }

        Instruction::UnaryI { op, dst, src } => {
            output.push(unary_int_op_to_opcode(*op).to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::UnaryF { op, dst, src } => {
            // Unary float ops share opcode with a sub-opcode byte
            output.push(Opcode::NegF.to_byte()); // Base opcode for unary float
            output.push(*op as u8);
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::Not { dst, src } => {
            output.push(Opcode::Not.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::Bitwise { op, dst, a, b } => {
            output.push(bitwise_op_to_opcode(*op).to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        // ====================================================================
        // Comparison
        // ====================================================================
        Instruction::CmpI { op, dst, a, b } => {
            output.push(cmp_int_op_to_opcode(*op).to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::CmpF { op, dst, a, b } => {
            output.push(cmp_float_op_to_opcode(*op).to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::CmpG {
            eq,
            dst,
            a,
            b,
            protocol_id,
        } => {
            output.push(if *eq {
                Opcode::EqG.to_byte()
            } else {
                Opcode::CmpG.to_byte()
            });
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
            encode_varint(*protocol_id as u64, output);
        }

        Instruction::CmpU { sub_op, dst, a, b } => {
            output.push(Opcode::CmpExtended.to_byte());
            output.push(sub_op.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        // ====================================================================
        // Control Flow
        // ====================================================================
        Instruction::Jmp { offset } => {
            output.push(Opcode::Jmp.to_byte());
            encode_signed_varint(*offset as i64, output);
        }

        Instruction::JmpIf { cond, offset } => {
            output.push(Opcode::JmpIf.to_byte());
            encode_reg(*cond, output);
            encode_signed_varint(*offset as i64, output);
        }

        Instruction::JmpNot { cond, offset } => {
            output.push(Opcode::JmpNot.to_byte());
            encode_reg(*cond, output);
            encode_signed_varint(*offset as i64, output);
        }

        Instruction::JmpCmp { op, a, b, offset } => {
            output.push(jmp_cmp_op_to_opcode(*op).to_byte());
            encode_reg(*a, output);
            encode_reg(*b, output);
            encode_signed_varint(*offset as i64, output);
        }

        Instruction::Ret { value } => {
            output.push(Opcode::Ret.to_byte());
            encode_reg(*value, output);
        }

        Instruction::RetV => {
            output.push(Opcode::RetV.to_byte());
        }

        Instruction::Call { dst, func_id, args } => {
            output.push(Opcode::Call.to_byte());
            encode_reg(*dst, output);
            encode_varint(*func_id as u64, output);
            encode_reg_range(*args, output);
        }

        Instruction::TailCall { func_id, args } => {
            output.push(Opcode::TailCall.to_byte());
            encode_varint(*func_id as u64, output);
            encode_reg_range(*args, output);
        }

        Instruction::CallM {
            dst,
            receiver,
            method_id,
            args,
        } => {
            output.push(Opcode::CallM.to_byte());
            encode_reg(*dst, output);
            encode_reg(*receiver, output);
            encode_varint(*method_id as u64, output);
            encode_reg_range(*args, output);
        }

        // ====================================================================
        // Memory
        // ====================================================================
        Instruction::New { dst, type_id, field_count } => {
            output.push(Opcode::New.to_byte());
            encode_reg(*dst, output);
            encode_varint(*type_id as u64, output);
            encode_varint(*field_count as u64, output);
        }

        Instruction::NewG {
            dst,
            type_id,
            type_args,
        } => {
            output.push(Opcode::NewG.to_byte());
            encode_reg(*dst, output);
            encode_varint(*type_id as u64, output);
            encode_reg_vec(type_args, output);
        }

        Instruction::GetF { dst, obj, field_idx } => {
            output.push(Opcode::GetF.to_byte());
            encode_reg(*dst, output);
            encode_reg(*obj, output);
            encode_varint(*field_idx as u64, output);
        }

        Instruction::SetF {
            obj,
            field_idx,
            value,
        } => {
            output.push(Opcode::SetF.to_byte());
            encode_reg(*obj, output);
            encode_varint(*field_idx as u64, output);
            encode_reg(*value, output);
        }

        Instruction::GetE { dst, arr, idx } => {
            output.push(Opcode::GetE.to_byte());
            encode_reg(*dst, output);
            encode_reg(*arr, output);
            encode_reg(*idx, output);
        }

        Instruction::SetE { arr, idx, value } => {
            output.push(Opcode::SetE.to_byte());
            encode_reg(*arr, output);
            encode_reg(*idx, output);
            encode_reg(*value, output);
        }

        Instruction::Len { dst, arr, type_hint } => {
            output.push(Opcode::Len.to_byte());
            encode_reg(*dst, output);
            encode_reg(*arr, output);
            output.push(*type_hint);
        }

        // ====================================================================
        // CBGR
        // ====================================================================
        Instruction::Ref { dst, src } => {
            output.push(Opcode::Ref.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::RefMut { dst, src } => {
            output.push(Opcode::RefMut.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::Deref { dst, ref_reg } => {
            output.push(Opcode::Deref.to_byte());
            encode_reg(*dst, output);
            encode_reg(*ref_reg, output);
        }

        Instruction::DerefMut { ref_reg, value } => {
            output.push(Opcode::DerefMut.to_byte());
            encode_reg(*ref_reg, output);
            encode_reg(*value, output);
        }

        Instruction::ChkRef { ref_reg } => {
            output.push(Opcode::ChkRef.to_byte());
            encode_reg(*ref_reg, output);
        }

        Instruction::DropRef { src } => {
            output.push(Opcode::DropRef.to_byte());
            encode_reg(*src, output);
        }

        Instruction::RefChecked { dst, src } => {
            output.push(Opcode::RefChecked.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::RefUnsafe { dst, src } => {
            output.push(Opcode::RefUnsafe.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        // ====================================================================
        // Generic
        // ====================================================================
        Instruction::CallG {
            dst,
            func_id,
            type_args,
            args,
        } => {
            output.push(Opcode::CallG.to_byte());
            encode_reg(*dst, output);
            encode_varint(*func_id as u64, output);
            encode_reg_vec(type_args, output);
            encode_reg_range(*args, output);
        }

        Instruction::CallV {
            dst,
            receiver,
            vtable_slot,
            args,
        } => {
            output.push(Opcode::CallV.to_byte());
            encode_reg(*dst, output);
            encode_reg(*receiver, output);
            encode_varint(*vtable_slot as u64, output);
            encode_reg_range(*args, output);
        }

        Instruction::CallC { dst, cache_id, args } => {
            output.push(Opcode::CallC.to_byte());
            encode_reg(*dst, output);
            encode_varint(*cache_id as u64, output);
            encode_reg_range(*args, output);
        }

        // ====================================================================
        // Pattern Matching
        // ====================================================================
        Instruction::IsVar { dst, value, tag } => {
            output.push(Opcode::IsVar.to_byte());
            encode_reg(*dst, output);
            encode_reg(*value, output);
            encode_varint(*tag as u64, output);
        }

        Instruction::AsVar { dst, value, tag } => {
            output.push(Opcode::AsVar.to_byte());
            encode_reg(*dst, output);
            encode_reg(*value, output);
            encode_varint(*tag as u64, output);
        }

        Instruction::Unpack {
            dst_start,
            tuple,
            count,
        } => {
            output.push(Opcode::Unpack.to_byte());
            encode_reg(*dst_start, output);
            encode_reg(*tuple, output);
            output.push(*count);
        }

        Instruction::Pack {
            dst,
            src_start,
            count,
        } => {
            output.push(Opcode::Pack.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src_start, output);
            output.push(*count);
        }

        Instruction::Switch {
            value,
            default_offset,
            cases,
        } => {
            output.push(Opcode::Switch.to_byte());
            encode_reg(*value, output);
            encode_signed_varint(*default_offset as i64, output);
            encode_varint(cases.len() as u64, output);
            for (case_val, offset) in cases {
                encode_varint(*case_val as u64, output);
                encode_signed_varint(*offset as i64, output);
            }
        }

        // ====================================================================
        // Generator Operations: encoding for GenCreate/GenNext/GenHasNext/Yield opcodes.
        // These implement the iterator protocol for fn* generator functions.
        // ====================================================================

        Instruction::GenCreate { dst, func_id, args } => {
            output.push(Opcode::GenCreate.to_byte());
            encode_reg(*dst, output);
            encode_varint(*func_id as u64, output);
            encode_reg_range(*args, output);
        }

        Instruction::GenNext { dst, generator } => {
            output.push(Opcode::GenNext.to_byte());
            encode_reg(*dst, output);
            encode_reg(*generator, output);
        }

        Instruction::GenHasNext { dst, generator } => {
            output.push(Opcode::GenHasNext.to_byte());
            encode_reg(*dst, output);
            encode_reg(*generator, output);
        }

        // ====================================================================
        // Async
        // ====================================================================
        Instruction::Spawn { dst, func_id, args } => {
            output.push(Opcode::Spawn.to_byte());
            encode_reg(*dst, output);
            encode_varint(*func_id as u64, output);
            encode_reg_range(*args, output);
        }

        Instruction::Await { dst, task } => {
            output.push(Opcode::Await.to_byte());
            encode_reg(*dst, output);
            encode_reg(*task, output);
        }

        Instruction::Yield { value } => {
            output.push(Opcode::Yield.to_byte());
            encode_reg(*value, output);
        }

        Instruction::Select {
            dst,
            futures,
            handlers,
        } => {
            output.push(Opcode::Select.to_byte());
            encode_reg(*dst, output);
            encode_reg_vec(futures, output);
            encode_varint(handlers.len() as u64, output);
            for h in handlers {
                encode_signed_varint(*h as i64, output);
            }
        }

        // ====================================================================
        // Autodiff
        // ====================================================================
        Instruction::GradBegin { scope_id, mode, wrt } => {
            output.push(Opcode::GradBegin.to_byte());
            encode_varint(*scope_id as u64, output);
            output.push(*mode as u8);
            encode_reg_vec(wrt, output);
        }

        Instruction::GradEnd {
            scope_id,
            output: out_reg,
            grad_out,
            grad_regs,
        } => {
            output.push(Opcode::GradEnd.to_byte());
            encode_varint(*scope_id as u64, output);
            encode_reg(*out_reg, output);
            encode_reg(*grad_out, output);
            encode_reg_vec(grad_regs, output);
        }

        Instruction::GradCheckpoint { id, tensors } => {
            output.push(Opcode::GradCheckpoint.to_byte());
            encode_varint(*id as u64, output);
            encode_reg_vec(tensors, output);
        }

        Instruction::GradAccumulate { dst, src } => {
            output.push(Opcode::GradAccumulate.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::GradStop { dst, src } => {
            output.push(Opcode::GradStop.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        // ====================================================================
        // Context
        // ====================================================================
        Instruction::CtxGet { dst, ctx_type } => {
            output.push(Opcode::CtxGet.to_byte());
            encode_reg(*dst, output);
            encode_varint(*ctx_type as u64, output);
        }

        Instruction::CtxProvide {
            ctx_type,
            value,
            body_offset,
        } => {
            output.push(Opcode::CtxProvide.to_byte());
            encode_varint(*ctx_type as u64, output);
            encode_reg(*value, output);
            encode_signed_varint(*body_offset as i64, output);
        }

        // ====================================================================
        // Debug/Verify
        // ====================================================================
        Instruction::Spec { reg, expected_type } => {
            output.push(Opcode::Spec.to_byte());
            encode_reg(*reg, output);
            encode_varint(*expected_type as u64, output);
        }

        Instruction::Guard {
            reg,
            expected_type,
            deopt_offset,
        } => {
            output.push(Opcode::Guard.to_byte());
            encode_reg(*reg, output);
            encode_varint(*expected_type as u64, output);
            encode_signed_varint(*deopt_offset as i64, output);
        }

        Instruction::Assert { cond, message_id } => {
            output.push(Opcode::Assert.to_byte());
            encode_reg(*cond, output);
            encode_varint(*message_id as u64, output);
        }

        Instruction::Panic { message_id } => {
            output.push(Opcode::Panic.to_byte());
            encode_varint(*message_id as u64, output);
        }

        Instruction::Unreachable => {
            output.push(Opcode::Unreachable.to_byte());
        }

        // ====================================================================
        // Tensor Operations
        // ====================================================================
        Instruction::TensorNew { dst, dtype, dims } => {
            output.push(Opcode::TensorNew.to_byte());
            encode_reg(*dst, output);
            output.push(*dtype as u8);
            encode_reg_vec(dims, output);
        }

        Instruction::TensorBinop { op, dst, a, b } => {
            output.push(Opcode::TensorBinop.to_byte());
            output.push(*op as u8);
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::TensorUnop { op, dst, src } => {
            output.push(Opcode::TensorUnop.to_byte());
            output.push(*op as u8);
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::TensorMatmul { dst, a, b } => {
            output.push(Opcode::TensorMatmul.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::TensorReduce {
            op,
            dst,
            src,
            axes,
            keepdim,
        } => {
            output.push(Opcode::TensorReduce.to_byte());
            output.push(*op as u8);
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_varint(axes.len() as u64, output);
            output.extend_from_slice(axes);
            output.push(if *keepdim { 1 } else { 0 });
        }

        Instruction::TensorFlashAttention {
            dst,
            q,
            k,
            v,
            mask,
            scale,
            causal,
        } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::FlashAttention.to_byte());
            encode_reg(*dst, output);
            encode_reg(*q, output);
            encode_reg(*k, output);
            encode_reg(*v, output);
            encode_optional_reg(*mask, output);
            encode_reg(*scale, output);
            output.push(if *causal { 1 } else { 0 });
        }

        Instruction::TensorContiguousView { dst, src } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::ContiguousView.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::RandomU64 { dst } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::RandomU64.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::RandomFloat { dst, low, high } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::RandomFloat.to_byte());
            encode_reg(*dst, output);
            encode_reg(*low, output);
            encode_reg(*high, output);
        }

        Instruction::GlobalAllocator { dst } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::GlobalAllocator.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::MemNewId { dst } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::MemNewId.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::MemAllocTensor { dst, shape, dtype } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::MemAllocTensor.to_byte());
            encode_reg(*dst, output);
            encode_reg(*shape, output);
            output.push(*dtype);
        }

        // ====================================================================
        // GPU - Fast Path (single-byte opcodes for common operations)
        // ====================================================================
        Instruction::GpuLaunch {
            kernel_id,
            grid,
            block,
            shared_mem,
            stream,
            args,
        } => {
            // Use GpuExtended + Launch sub-opcode
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::Launch.to_byte());
            encode_varint(*kernel_id as u64, output);
            for r in grid {
                encode_reg(*r, output);
            }
            for r in block {
                encode_reg(*r, output);
            }
            encode_reg(*shared_mem, output);
            encode_reg(*stream, output);
            encode_reg_vec(args, output);
        }

        Instruction::GpuSync { stream } => {
            output.push(Opcode::GpuSync.to_byte());
            encode_reg(*stream, output);
        }

        Instruction::GpuMemcpy { dst, src, direction } => {
            output.push(Opcode::GpuMemcpy.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(*direction);
        }

        Instruction::GpuAlloc { dst, size, device } => {
            output.push(Opcode::GpuAlloc.to_byte());
            encode_reg(*dst, output);
            encode_reg(*size, output);
            encode_reg(*device, output);
        }

        // ====================================================================
        // GPU Extended - Streams
        // ====================================================================
        Instruction::GpuStreamCreate { dst } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::StreamCreate.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::GpuStreamCreateWithPriority { dst, priority } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::StreamCreateWithPriority.to_byte());
            encode_reg(*dst, output);
            encode_reg(*priority, output);
        }

        Instruction::GpuStreamCreateNonBlocking { dst } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::StreamCreateNonBlocking.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::GpuStreamDestroy { stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::StreamDestroy.to_byte());
            encode_reg(*stream, output);
        }

        Instruction::GpuStreamQuery { dst, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::StreamQuery.to_byte());
            encode_reg(*dst, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuStreamWaitEvent { stream, event } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::StreamWaitEvent.to_byte());
            encode_reg(*stream, output);
            encode_reg(*event, output);
        }

        Instruction::GpuStreamGetPriority { dst, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::StreamGetPriority.to_byte());
            encode_reg(*dst, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuStreamAddCallback { stream, callback_id, user_data } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::StreamAddCallback.to_byte());
            encode_reg(*stream, output);
            encode_varint(*callback_id as u64, output);
            encode_reg(*user_data, output);
        }

        // ====================================================================
        // GPU Extended - Events
        // ====================================================================
        Instruction::GpuEventCreate { dst } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EventCreate.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::GpuEventCreateWithFlags { dst, flags } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EventCreateWithFlags.to_byte());
            encode_reg(*dst, output);
            output.push(*flags);
        }

        Instruction::GpuEventDestroy { event } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EventDestroy.to_byte());
            encode_reg(*event, output);
        }

        Instruction::GpuEventRecord { event, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EventRecord.to_byte());
            encode_reg(*event, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuEventRecordWithFlags { event, stream, flags } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EventRecordWithFlags.to_byte());
            encode_reg(*event, output);
            encode_reg(*stream, output);
            output.push(*flags);
        }

        Instruction::GpuEventSynchronize { event } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EventSynchronize.to_byte());
            encode_reg(*event, output);
        }

        Instruction::GpuEventQuery { dst, event } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EventQuery.to_byte());
            encode_reg(*dst, output);
            encode_reg(*event, output);
        }

        Instruction::GpuEventElapsed { dst, start_event, end_event } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EventElapsed.to_byte());
            encode_reg(*dst, output);
            encode_reg(*start_event, output);
            encode_reg(*end_event, output);
        }

        // ====================================================================
        // GPU Extended - Device Management
        // ====================================================================
        Instruction::GpuGetDevice { dst } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GetDevice.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::GpuSetDevice { device } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::SetDevice.to_byte());
            encode_reg(*device, output);
        }

        Instruction::GpuGetDeviceCount { dst } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GetDeviceCount.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::GpuGetDeviceProperty { dst, device, property_id } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GetDeviceProperty.to_byte());
            encode_reg(*dst, output);
            encode_reg(*device, output);
            output.push(*property_id);
        }

        Instruction::GpuGetMemoryInfo { free, total, device } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GetMemoryInfo.to_byte());
            encode_reg(*free, output);
            encode_reg(*total, output);
            encode_reg(*device, output);
        }

        Instruction::GpuCanAccessPeer { dst, device, peer_device } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::CanAccessPeer.to_byte());
            encode_reg(*dst, output);
            encode_reg(*device, output);
            encode_reg(*peer_device, output);
        }

        Instruction::GpuEnablePeerAccess { device, peer_device } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::EnablePeerAccess.to_byte());
            encode_reg(*device, output);
            encode_reg(*peer_device, output);
        }

        Instruction::GpuDisablePeerAccess { device, peer_device } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::DisablePeerAccess.to_byte());
            encode_reg(*device, output);
            encode_reg(*peer_device, output);
        }

        Instruction::GpuDeviceReset { device } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::DeviceReset.to_byte());
            encode_reg(*device, output);
        }

        Instruction::GpuSetDeviceFlags { flags } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::SetDeviceFlags.to_byte());
            output.push(*flags);
        }

        // ====================================================================
        // GPU Extended - Memory Operations
        // ====================================================================
        Instruction::GpuMemcpyAsync { dst, src, size, direction, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemcpyAsync.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*size, output);
            output.push(*direction);
            encode_reg(*stream, output);
        }

        Instruction::GpuFree { ptr } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::Free.to_byte());
            encode_reg(*ptr, output);
        }

        Instruction::GpuPinMemory { ptr, size } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::PinMemory.to_byte());
            encode_reg(*ptr, output);
            encode_reg(*size, output);
        }

        Instruction::GpuUnpinMemory { ptr } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::UnpinMemory.to_byte());
            encode_reg(*ptr, output);
        }

        Instruction::GpuPrefetch { ptr, size, device, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::Prefetch.to_byte());
            encode_reg(*ptr, output);
            encode_reg(*size, output);
            encode_reg(*device, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuMemset { ptr, value, size } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::Memset.to_byte());
            encode_reg(*ptr, output);
            output.push(*value);
            encode_reg(*size, output);
        }

        Instruction::GpuMemsetAsync { ptr, value, size, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemsetAsync.to_byte());
            encode_reg(*ptr, output);
            output.push(*value);
            encode_reg(*size, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuMemcpy2D { dst, dst_pitch, src, src_pitch, width, height, direction } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::Memcpy2D.to_byte());
            encode_reg(*dst, output);
            encode_reg(*dst_pitch, output);
            encode_reg(*src, output);
            encode_reg(*src_pitch, output);
            encode_reg(*width, output);
            encode_reg(*height, output);
            output.push(*direction);
        }

        Instruction::GpuMemcpy2DAsync { dst, dst_pitch, src, src_pitch, width, height, direction, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::Memcpy2DAsync.to_byte());
            encode_reg(*dst, output);
            encode_reg(*dst_pitch, output);
            encode_reg(*src, output);
            encode_reg(*src_pitch, output);
            encode_reg(*width, output);
            encode_reg(*height, output);
            output.push(*direction);
            encode_reg(*stream, output);
        }

        Instruction::GpuMemcpyH2D { dst, src, size } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemcpyH2D.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*size, output);
        }

        Instruction::GpuMemcpyD2H { dst, src, size } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemcpyD2H.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*size, output);
        }

        Instruction::GpuMemcpyD2D { dst, src, size } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemcpyD2D.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*size, output);
        }

        Instruction::GpuMemcpyAsyncH2D { dst, src, size, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemcpyAsyncH2D.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*size, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuMemcpyAsyncD2H { dst, src, size, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemcpyAsyncD2H.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*size, output);
            encode_reg(*stream, output);
        }

        // ====================================================================
        // GPU Extended - Unified Memory
        // ====================================================================
        Instruction::GpuMallocManaged { dst, size, attach_flags } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MallocManaged.to_byte());
            encode_reg(*dst, output);
            encode_reg(*size, output);
            output.push(*attach_flags);
        }

        Instruction::GpuMemAdvise { ptr, size, advice, device } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemAdvise.to_byte());
            encode_reg(*ptr, output);
            encode_reg(*size, output);
            output.push(*advice);
            encode_reg(*device, output);
        }

        Instruction::GpuPrefetchAsync { ptr, size, device, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::PrefetchAsync.to_byte());
            encode_reg(*ptr, output);
            encode_reg(*size, output);
            encode_reg(*device, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuMemGetAttribute { dst, ptr, attribute } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::MemGetAttribute.to_byte());
            encode_reg(*dst, output);
            encode_reg(*ptr, output);
            output.push(*attribute);
        }

        // ====================================================================
        // GPU Extended - CUDA Graphs / Metal ICB
        // ====================================================================
        Instruction::GpuGraphCreate { dst } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GraphCreate.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::GpuGraphBeginCapture { stream, mode } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GraphBeginCapture.to_byte());
            encode_reg(*stream, output);
            output.push(*mode);
        }

        Instruction::GpuGraphEndCapture { dst, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GraphEndCapture.to_byte());
            encode_reg(*dst, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuGraphInstantiate { dst, graph } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GraphInstantiate.to_byte());
            encode_reg(*dst, output);
            encode_reg(*graph, output);
        }

        Instruction::GpuGraphLaunch { graph_exec, stream } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GraphLaunch.to_byte());
            encode_reg(*graph_exec, output);
            encode_reg(*stream, output);
        }

        Instruction::GpuGraphDestroy { graph } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GraphDestroy.to_byte());
            encode_reg(*graph, output);
        }

        Instruction::GpuGraphExecDestroy { graph_exec } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GraphExecDestroy.to_byte());
            encode_reg(*graph_exec, output);
        }

        Instruction::GpuGraphExecUpdate { graph_exec, graph } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::GraphExecUpdate.to_byte());
            encode_reg(*graph_exec, output);
            encode_reg(*graph, output);
        }

        // ====================================================================
        // GPU Extended - Profiling
        // ====================================================================
        Instruction::GpuProfileRangeStart { name_id } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::ProfileRangeStart.to_byte());
            encode_varint(*name_id as u64, output);
        }

        Instruction::GpuProfileRangeEnd => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::ProfileRangeEnd.to_byte());
        }

        Instruction::GpuProfileMarkerPush { name_id } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::ProfileMarkerPush.to_byte());
            encode_varint(*name_id as u64, output);
        }

        Instruction::GpuProfileMarkerPop => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::ProfileMarkerPop.to_byte());
        }

        // ====================================================================
        // GPU Extended - Device Enumeration
        // ====================================================================
        Instruction::GpuEnumerateDevices { dst, backend } => {
            output.push(Opcode::GpuExtended.to_byte());
            let sub_op = match backend {
                0 => GpuSubOpcode::EnumerateCuda,
                1 => GpuSubOpcode::EnumerateMetal,
                2 => GpuSubOpcode::EnumerateRocm,
                3 => GpuSubOpcode::EnumerateVulkan,
                _ => GpuSubOpcode::EnumerateCuda, // Default to CUDA
            };
            output.push(sub_op.to_byte());
            encode_reg(*dst, output);
        }

        // ====================================================================
        // GPU Extended - Advanced Kernel Execution
        // ====================================================================
        Instruction::GpuLaunchCooperative {
            kernel_id,
            grid,
            block,
            shared_mem,
            stream,
            args,
        } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::LaunchCooperative.to_byte());
            encode_varint(*kernel_id as u64, output);
            for r in grid {
                encode_reg(*r, output);
            }
            for r in block {
                encode_reg(*r, output);
            }
            encode_reg(*shared_mem, output);
            encode_reg(*stream, output);
            encode_reg_vec(args, output);
        }

        Instruction::GpuLaunchMultiDevice {
            kernel_id,
            devices,
            grid,
            block,
            shared_mem,
            args,
        } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::LaunchMultiDevice.to_byte());
            encode_varint(*kernel_id as u64, output);
            encode_reg(*devices, output);
            for r in grid {
                encode_reg(*r, output);
            }
            for r in block {
                encode_reg(*r, output);
            }
            encode_reg(*shared_mem, output);
            encode_reg_vec(args, output);
        }

        Instruction::GpuDeviceSync => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(GpuSubOpcode::SyncDevice.to_byte());
        }

        // ====================================================================
        // Additional Tensor Operations
        // ====================================================================
        Instruction::TensorFull { dst, value, shape, dtype } => {
            output.push(Opcode::TensorFull.to_byte());
            encode_reg(*dst, output);
            encode_reg(*value, output);
            encode_reg_vec(shape, output);
            output.push(*dtype as u8);
        }

        Instruction::TensorFromSlice { dst, data, shape, dtype } => {
            output.push(Opcode::TensorFromSlice.to_byte());
            encode_reg(*dst, output);
            encode_reg(*data, output);
            encode_reg_vec(shape, output);
            output.push(*dtype as u8);
        }

        Instruction::TensorArange { dst, start, end, step, dtype } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Arange.to_byte());
            encode_reg(*dst, output);
            encode_reg(*start, output);
            encode_reg(*end, output);
            encode_reg(*step, output);
            output.push(*dtype as u8);
        }

        Instruction::TensorLinspace { dst, start, end, num, dtype } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Linspace.to_byte());
            encode_reg(*dst, output);
            encode_reg(*start, output);
            encode_reg(*end, output);
            encode_reg(*num, output);
            output.push(*dtype as u8);
        }

        Instruction::TensorRand { dst, shape, dtype } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Rand.to_byte());
            encode_reg(*dst, output);
            encode_reg_vec(shape, output);
            output.push(*dtype as u8);
        }

        Instruction::TensorClone { dst, src } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Clone.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::TensorIdentity { dst, size, dtype } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Identity.to_byte());
            encode_reg(*dst, output);
            encode_reg(*size, output);
            output.push(*dtype as u8);
        }

        Instruction::TensorReshape { dst, src, shape } => {
            output.push(Opcode::TensorReshape.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg_vec(shape, output);
        }

        Instruction::TensorTranspose { dst, src, perm } => {
            output.push(Opcode::TensorTranspose.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_varint(perm.len() as u64, output);
            output.extend_from_slice(perm);
        }

        Instruction::TensorSlice { dst, src, starts, ends } => {
            output.push(Opcode::TensorSlice.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg_vec(starts, output);
            encode_reg_vec(ends, output);
        }

        Instruction::TensorIndex { dst, src, indices, axis } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Index.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*indices, output);
            output.push(*axis);
        }

        Instruction::TensorConcat { dst, tensors, axis } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Concat.to_byte());
            encode_reg(*dst, output);
            encode_reg_vec(tensors, output);
            output.push(*axis);
        }

        Instruction::TensorStack { dst, tensors, axis } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Stack.to_byte());
            encode_reg(*dst, output);
            encode_reg_vec(tensors, output);
            output.push(*axis);
        }

        Instruction::TensorBroadcast { dst, src, shape } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::BroadcastToShape.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg_vec(shape, output);
        }

        Instruction::TensorSqueeze { dst, src, axes } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Squeeze.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_varint(axes.len() as u64, output);
            output.extend_from_slice(axes);
        }

        Instruction::TensorCmp { op, dst, a, b } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Cmp.to_byte());
            output.push(*op as u8);
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::TensorWhere { dst, cond, x, y } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Where.to_byte());
            encode_reg(*dst, output);
            encode_reg(*cond, output);
            encode_reg(*x, output);
            encode_reg(*y, output);
        }

        Instruction::TensorClamp { dst, src, min, max } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Clamp.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*min, output);
            encode_reg(*max, output);
        }

        Instruction::TensorCast { dst, src, dtype } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Cast.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(*dtype as u8);
        }

        Instruction::TensorMaskedFill { dst, src, mask, value } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::MaskedFill.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*mask, output);
            encode_reg(*value, output);
        }

        Instruction::TensorLerp { dst, a, b, t } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Lerp.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
            encode_reg(*t, output);
        }

        Instruction::TensorDot { dst, a, b, axes_a, axes_b } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Dot.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
            encode_varint(axes_a.len() as u64, output);
            output.extend_from_slice(axes_a);
            encode_varint(axes_b.len() as u64, output);
            output.extend_from_slice(axes_b);
        }

        Instruction::TensorConv { dst, input, kernel, bias, stride, padding, dilation, groups } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Conv.to_byte());
            encode_reg(*dst, output);
            encode_reg(*input, output);
            encode_reg(*kernel, output);
            encode_optional_reg(*bias, output);
            encode_varint(stride.len() as u64, output);
            output.extend_from_slice(stride);
            encode_varint(padding.len() as u64, output);
            output.extend_from_slice(padding);
            encode_varint(dilation.len() as u64, output);
            output.extend_from_slice(dilation);
            output.push(*groups);
        }

        Instruction::TensorBatchMatmul { dst, a, b } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::BatchMatmul.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::TensorEinsum { dst, inputs, equation_id } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Einsum.to_byte());
            encode_reg(*dst, output);
            encode_reg_vec(inputs, output);
            encode_varint(*equation_id as u64, output);
        }

        Instruction::TensorOuter { dst, a, b } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Outer.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::TensorTriSolve { dst, a, b, upper } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::TriSolve.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
            output.push(if *upper { 1 } else { 0 });
        }

        Instruction::TensorCholesky { dst, src, upper } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Cholesky.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(if *upper { 1 } else { 0 });
        }

        Instruction::TensorArgmax { dst, src, axis, keepdim } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Argmax.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(*axis as u8);
            output.push(if *keepdim { 1 } else { 0 });
        }

        Instruction::TensorTopk { values, indices, src, k, axis, largest } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Topk.to_byte());
            encode_reg(*values, output);
            encode_reg(*indices, output);
            encode_reg(*src, output);
            encode_reg(*k, output);
            output.push(*axis as u8);
            output.push(if *largest { 1 } else { 0 });
        }

        Instruction::TensorCumulative { op, dst, src, axis } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Cumulative.to_byte());
            output.push(*op as u8);
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(*axis as u8);
        }

        Instruction::TensorSoftmax { dst, src, axis } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Softmax.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(*axis as u8);
        }

        Instruction::TensorLayerNorm { dst, input, gamma, beta, normalized_shape, eps } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::LayerNorm.to_byte());
            encode_reg(*dst, output);
            encode_reg(*input, output);
            encode_optional_reg(*gamma, output);
            encode_optional_reg(*beta, output);
            encode_varint(*normalized_shape as u64, output);
            output.extend_from_slice(&eps.to_le_bytes());
        }

        Instruction::TensorBatchNorm { dst, input, gamma, beta, running_mean, running_var, training, momentum, eps } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::BatchNorm.to_byte());
            encode_reg(*dst, output);
            encode_reg(*input, output);
            encode_optional_reg(*gamma, output);
            encode_optional_reg(*beta, output);
            encode_optional_reg(*running_mean, output);
            encode_optional_reg(*running_var, output);
            output.push(if *training { 1 } else { 0 });
            output.extend_from_slice(&momentum.to_le_bytes());
            output.extend_from_slice(&eps.to_le_bytes());
        }

        Instruction::TensorRmsNorm { dst, input, gamma, eps } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::RmsNorm.to_byte());
            encode_reg(*dst, output);
            encode_reg(*input, output);
            encode_optional_reg(*gamma, output);
            output.extend_from_slice(&eps.to_le_bytes());
        }

        Instruction::TensorFft { dst, src, dim, inverse } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::Fft.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(*dim as u8);
            output.push(if *inverse { 1 } else { 0 });
        }

        Instruction::TensorScatter { dst, src, index, values, axis, mode } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorExtSubOpcode::Scatter.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*index, output);
            encode_reg(*values, output);
            output.push(*axis as u8);
            output.push(*mode);
        }

        Instruction::TensorPool { op, dst, src, kernel_size, stride, padding } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Pool.to_byte());
            output.push(*op as u8);
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_varint(kernel_size.len() as u64, output);
            output.extend_from_slice(kernel_size);
            encode_varint(stride.len() as u64, output);
            output.extend_from_slice(stride);
            encode_varint(padding.len() as u64, output);
            output.extend_from_slice(padding);
        }

        // TensorExtended operations
        Instruction::TensorArgmin { dst, src, axis, keepdim } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Argmin.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(*axis as u8);
            output.push(if *keepdim { 1 } else { 0 });
        }

        Instruction::TensorSolve { dst, a, b } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Solve.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::TensorGather { dst, src, index, axis } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Gather.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_reg(*index, output);
            output.push(*axis as u8);
        }

        Instruction::TensorPermute { dst, src, axes } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Permute.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            encode_varint(axes.len() as u64, output);
            output.extend_from_slice(axes);
        }

        Instruction::TensorQR { q, r, src, mode } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::QR.to_byte());
            encode_reg(*q, output);
            encode_reg(*r, output);
            encode_reg(*src, output);
            output.push(*mode);
        }

        Instruction::TensorSVD { u, s, vh, src, full_matrices, compute_uv } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::SVD.to_byte());
            encode_reg(*u, output);
            encode_reg(*s, output);
            encode_reg(*vh, output);
            encode_reg(*src, output);
            output.push(if *full_matrices { 1 } else { 0 });
            output.push(if *compute_uv { 1 } else { 0 });
        }

        Instruction::TensorLU { p, l, u, src } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::LU.to_byte());
            encode_reg(*p, output);
            encode_reg(*l, output);
            encode_reg(*u, output);
            encode_reg(*src, output);
        }

        Instruction::TensorEig { eigenvalues, eigenvectors, src, compute_v } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Eig.to_byte());
            encode_reg(*eigenvalues, output);
            encode_reg(*eigenvectors, output);
            encode_reg(*src, output);
            output.push(if *compute_v { 1 } else { 0 });
        }

        Instruction::TensorEigSymmetric { eigenvalues, eigenvectors, src, upper } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::EigSymmetric.to_byte());
            encode_reg(*eigenvalues, output);
            encode_reg(*eigenvectors, output);
            encode_reg(*src, output);
            output.push(if *upper { 1 } else { 0 });
        }

        Instruction::TensorLstsq { x, residuals, rank, s, a, b, rcond } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Lstsq.to_byte());
            encode_reg(*x, output);
            encode_reg(*residuals, output);
            encode_reg(*rank, output);
            encode_reg(*s, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
            encode_f64(*rcond, output);
        }

        Instruction::TensorDet { dst, src } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Det.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::TensorTrace { dst, src } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Trace.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::TensorNorm { dst, src, ord } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(TensorSubOpcode::Norm.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
            output.push(*ord as u8);
        }

        // ====================================================================
        // Collections
        // ====================================================================
        Instruction::NewList { dst } => {
            output.push(Opcode::NewList.to_byte());
            encode_reg(*dst, output);
        }
        Instruction::ListPush { list, val } => {
            output.push(Opcode::ListPush.to_byte());
            encode_reg(*list, output);
            encode_reg(*val, output);
        }
        Instruction::ListPop { dst, list } => {
            output.push(Opcode::ListPop.to_byte());
            encode_reg(*dst, output);
            encode_reg(*list, output);
        }
        Instruction::NewMap { dst } => {
            output.push(Opcode::NewMap.to_byte());
            encode_reg(*dst, output);
            // Emit capacity_hint=0 so the handler's read_varint is satisfied.
            // MakeMap emits a real capacity; NewMap uses default.
            encode_varint(0, output);
        }
        Instruction::MapGet { dst, map, key } => {
            output.push(Opcode::MapGet.to_byte());
            encode_reg(*dst, output);
            encode_reg(*map, output);
            encode_reg(*key, output);
        }
        Instruction::MapSet { map, key, val } => {
            output.push(Opcode::MapSet.to_byte());
            encode_reg(*map, output);
            encode_reg(*key, output);
            encode_reg(*val, output);
        }
        Instruction::MapContains { dst, map, key } => {
            output.push(Opcode::MapContains.to_byte());
            encode_reg(*dst, output);
            encode_reg(*map, output);
            encode_reg(*key, output);
        }
        Instruction::IterNew { dst, iterable } => {
            output.push(Opcode::IterNew.to_byte());
            encode_reg(*dst, output);
            encode_reg(*iterable, output);
        }
        Instruction::IterNext { dst, has_next, iter } => {
            output.push(Opcode::IterNext.to_byte());
            encode_reg(*dst, output);
            encode_reg(*has_next, output);
            encode_reg(*iter, output);
        }
        Instruction::Clone { dst, src } => {
            output.push(Opcode::Clone.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        // ====================================================================
        // Closures
        // ====================================================================
        Instruction::CallClosure { dst, closure, args } => {
            output.push(Opcode::CallClosure.to_byte());
            encode_reg(*dst, output);
            encode_reg(*closure, output);
            encode_reg(args.start, output);
            output.push(args.count);
        }
        Instruction::NewClosure { dst, func_id, captures } => {
            output.push(Opcode::NewClosure.to_byte());
            encode_reg(*dst, output);
            encode_varint(*func_id as u64, output);
            encode_reg_vec(captures, output);
        }

        // ====================================================================
        // Context
        // ====================================================================
        Instruction::CtxEnd => {
            output.push(Opcode::CtxEnd.to_byte());
        }

        Instruction::CtxCheckNegative { ctx_type, func_name: _ } => {
            // Negative context enforcement: emit CtxGet to a scratch register.
            // If the context IS present (non-nil), the program should panic.
            // In the bytecode interpreter path, negative contexts are enforced
            // at compile time by context_validation.rs. At the bytecode level,
            // we emit a no-op. The AOT path handles this via LLVM IR (see
            // verum_codegen::llvm::instruction.rs).
            //
            // We still emit a CtxGet so the bytecode remains well-formed,
            // using register 0 as a throwaway destination.
            output.push(Opcode::CtxGet.to_byte());
            encode_reg(Reg(0), output);
            encode_varint(*ctx_type as u64, output);
        }

        // ====================================================================
        // Debug
        // ====================================================================
        Instruction::DebugPrint { value } => {
            output.push(Opcode::DebugPrint.to_byte());
            encode_reg(*value, output);
        }

        // ====================================================================
        // Control Flow
        // ====================================================================
        Instruction::Nop => {
            output.push(Opcode::Nop.to_byte());
        }

        // ====================================================================
        // Set Operations
        // ====================================================================
        Instruction::NewSet { dst } => {
            output.push(Opcode::NewSet.to_byte());
            encode_reg(*dst, output);
            // Emit capacity_hint=0 so the handler's read_varint is satisfied.
            // MakeSet emits a real capacity; NewSet uses default.
            encode_varint(0, output);
        }

        Instruction::SetInsert { set, elem } => {
            output.push(Opcode::SetInsert.to_byte());
            encode_reg(*set, output);
            encode_reg(*elem, output);
        }

        Instruction::SetContains { dst, set, elem } => {
            output.push(Opcode::SetContains.to_byte());
            encode_reg(*dst, output);
            encode_reg(*set, output);
            encode_reg(*elem, output);
        }

        Instruction::SetRemove { set, elem } => {
            output.push(Opcode::SetRemove.to_byte());
            encode_reg(*set, output);
            encode_reg(*elem, output);
        }

        // ====================================================================
        // String Operations
        // ====================================================================
        Instruction::ToString { dst, src } => {
            output.push(Opcode::ToString.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::Concat { dst, a, b } => {
            output.push(Opcode::Concat.to_byte());
            encode_reg(*dst, output);
            encode_reg(*a, output);
            encode_reg(*b, output);
        }

        Instruction::CharToStr { dst, src } => {
            output.push(Opcode::CharToStr.to_byte());
            encode_reg(*dst, output);
            encode_reg(*src, output);
        }

        Instruction::NewRange { dst, start, end, inclusive } => {
            output.push(Opcode::NewRange.to_byte());
            encode_reg(*dst, output);
            encode_reg(*start, output);
            encode_reg(*end, output);
            output.push(if *inclusive { 1 } else { 0 });
        }

        // ====================================================================
        // Stack Operations
        // ====================================================================
        Instruction::Push { src } => {
            output.push(Opcode::Push.to_byte());
            encode_reg(*src, output);
        }

        Instruction::Pop { dst } => {
            output.push(Opcode::Pop.to_byte());
            encode_reg(*dst, output);
        }

        // ====================================================================
        // Indirect Calls
        // ====================================================================
        Instruction::CallR { dst, func, argc } => {
            output.push(Opcode::CallR.to_byte());
            encode_reg(*dst, output);
            encode_reg(*func, output);
            output.push(*argc);
        }

        // ====================================================================
        // Nil Value
        // ====================================================================
        Instruction::LoadNil { dst } => {
            output.push(Opcode::LoadNil.to_byte());
            encode_reg(*dst, output);
        }

        // ====================================================================
        // Exception Handling
        // ====================================================================
        Instruction::Throw { error } => {
            output.push(Opcode::Throw.to_byte());
            encode_reg(*error, output);
        }

        Instruction::TryBegin { handler_offset } => {
            output.push(Opcode::TryBegin.to_byte());
            encode_signed_varint(*handler_offset as i64, output);
        }

        Instruction::TryEnd => {
            output.push(Opcode::TryEnd.to_byte());
        }

        Instruction::GetException { dst } => {
            output.push(Opcode::GetException.to_byte());
            encode_reg(*dst, output);
        }

        // ====================================================================
        // Future/Async Operations (extended)
        // ====================================================================
        Instruction::FutureReady { dst, future } => {
            output.push(Opcode::FutureReady.to_byte());
            encode_reg(*dst, output);
            encode_reg(*future, output);
        }

        Instruction::FutureGet { dst, future } => {
            output.push(Opcode::FutureGet.to_byte());
            encode_reg(*dst, output);
            encode_reg(*future, output);
        }

        Instruction::AsyncNext { dst, iter } => {
            output.push(Opcode::AsyncNext.to_byte());
            encode_reg(*dst, output);
            encode_reg(*iter, output);
        }

        // ====================================================================
        // Context System (extended)
        // ====================================================================
        Instruction::PushContext { name, handler } => {
            output.push(Opcode::PushContext.to_byte());
            encode_varint(*name as u64, output);
            encode_reg(*handler, output);
        }

        Instruction::PopContext { name } => {
            output.push(Opcode::PopContext.to_byte());
            encode_varint(*name as u64, output);
        }

        Instruction::Attenuate { dst, context, capabilities } => {
            output.push(Opcode::Attenuate.to_byte());
            encode_reg(*dst, output);
            encode_reg(*context, output);
            encode_varint(*capabilities as u64, output);
        }

        // ====================================================================
        // Collection Operations (extended)
        // ====================================================================
        Instruction::Iter { dst, iterable } => {
            output.push(Opcode::IterNew.to_byte());
            encode_reg(*dst, output);
            encode_reg(*iterable, output);
        }

        Instruction::GetTag { dst, variant } => {
            output.push(Opcode::GetTag.to_byte());
            encode_reg(*dst, output);
            encode_reg(*variant, output);
        }

        Instruction::MakeVariant { dst, tag, field_count } => {
            output.push(Opcode::MakeVariant.to_byte());
            encode_reg(*dst, output);
            encode_varint(*tag as u64, output);
            encode_varint(*field_count as u64, output);
        }

        Instruction::SetVariantData { variant, field, value } => {
            output.push(Opcode::SetVariantData.to_byte());
            encode_reg(*variant, output);
            encode_varint(*field as u64, output);
            encode_reg(*value, output);
        }

        Instruction::GetVariantData { dst, variant, field } => {
            output.push(Opcode::GetVariantData.to_byte());
            encode_reg(*dst, output);
            encode_reg(*variant, output);
            encode_varint(*field as u64, output);
        }

        Instruction::GetVariantDataRef { dst, variant, field } => {
            output.push(Opcode::GetVariantDataRef.to_byte());
            encode_reg(*dst, output);
            encode_reg(*variant, output);
            encode_varint(*field as u64, output);
        }

        Instruction::MakeList { dst, len } => {
            output.push(Opcode::NewList.to_byte());
            encode_reg(*dst, output);
            encode_varint(*len as u64, output);
        }

        Instruction::MakeMap { dst, capacity } => {
            output.push(Opcode::NewMap.to_byte());
            encode_reg(*dst, output);
            encode_varint(*capacity as u64, output);
        }

        Instruction::MakeSet { dst, capacity } => {
            output.push(Opcode::NewSet.to_byte());
            encode_reg(*dst, output);
            encode_varint(*capacity as u64, output);
        }

        Instruction::MapInsert { map, key, value } => {
            output.push(Opcode::MapSet.to_byte());
            encode_reg(*map, output);
            encode_reg(*key, output);
            encode_reg(*value, output);
        }

        Instruction::MakeTensor { dst, shape_len, total_size, data } => {
            output.push(Opcode::TensorNew.to_byte());
            encode_reg(*dst, output);
            encode_varint(*shape_len as u64, output);
            encode_varint(*total_size as u64, output);
            encode_reg(*data, output);
        }

        // ====================================================================
        // Structured Concurrency (Nursery)
        // Uses dedicated opcodes (0xCA-0xCF) for core operations
        // and NurseryConfig (0xCE) with extended config_type for Enter/Exit/Set
        // ====================================================================
        Instruction::NurseryInit { dst } => {
            // NurseryInit: 0xCA dst
            output.push(Opcode::NurseryInit.to_byte());
            encode_reg(*dst, output);
        }

        Instruction::NurserySetTimeout { nursery, timeout } => {
            // NurseryConfig: 0xCE nursery config_type=0 value
            output.push(Opcode::NurseryConfig.to_byte());
            encode_reg(*nursery, output);
            encode_varint(0, output); // config_type: timeout
            encode_reg(*timeout, output);
        }

        Instruction::NurserySetMaxTasks { nursery, max_tasks } => {
            // NurseryConfig: 0xCE nursery config_type=1 value
            output.push(Opcode::NurseryConfig.to_byte());
            encode_reg(*nursery, output);
            encode_varint(1, output); // config_type: max_tasks
            encode_reg(*max_tasks, output);
        }

        Instruction::NurserySetErrorBehavior { nursery, behavior } => {
            // NurseryConfig: 0xCE nursery config_type=2 value
            output.push(Opcode::NurseryConfig.to_byte());
            encode_reg(*nursery, output);
            encode_varint(2, output); // config_type: error_behavior
            encode_reg(*behavior, output);
        }

        Instruction::NurseryEnter { nursery } => {
            // NurseryConfig: 0xCE nursery config_type=3 (enter scope)
            // Value register not used - set to same as nursery
            output.push(Opcode::NurseryConfig.to_byte());
            encode_reg(*nursery, output);
            encode_varint(3, output); // config_type: enter
            encode_reg(*nursery, output); // dummy value reg
        }

        Instruction::NurseryExit { nursery } => {
            // NurseryConfig: 0xCE nursery config_type=4 (exit scope)
            // Value register not used - set to same as nursery
            output.push(Opcode::NurseryConfig.to_byte());
            encode_reg(*nursery, output);
            encode_varint(4, output); // config_type: exit
            encode_reg(*nursery, output); // dummy value reg
        }

        Instruction::NurserySpawn { dst, nursery, task } => {
            // NurserySpawn: 0xCB dst nursery func_id
            output.push(Opcode::NurserySpawn.to_byte());
            encode_reg(*dst, output);
            encode_reg(*nursery, output);
            encode_reg(*task, output);
        }

        Instruction::NurseryAwaitAll { nursery, success } => {
            // NurseryAwait: 0xCC success nursery
            output.push(Opcode::NurseryAwait.to_byte());
            encode_reg(*success, output);
            encode_reg(*nursery, output);
        }

        Instruction::NurseryGetError { nursery, dst } => {
            // NurseryError: 0xCF dst nursery
            output.push(Opcode::NurseryError.to_byte());
            encode_reg(*dst, output);
            encode_reg(*nursery, output);
        }

        Instruction::NurseryCancel { nursery } => {
            // NurseryCancel: 0xCD nursery
            output.push(Opcode::NurseryCancel.to_byte());
            encode_reg(*nursery, output);
        }

        // ====================================================================
        // V-LLSI System Operations
        // ====================================================================
        Instruction::SyscallLinux { dst, num, a1, a2, a3, a4, a5, a6 } => {
            output.push(Opcode::SyscallLinux.to_byte());
            encode_reg(*dst, output);
            encode_reg(*num, output);
            encode_reg(*a1, output);
            encode_reg(*a2, output);
            encode_reg(*a3, output);
            encode_reg(*a4, output);
            encode_reg(*a5, output);
            encode_reg(*a6, output);
        }

        Instruction::Mmap { dst, addr, len, prot, flags, fd, offset } => {
            output.push(Opcode::Mmap.to_byte());
            encode_reg(*dst, output);
            encode_reg(*addr, output);
            encode_reg(*len, output);
            encode_reg(*prot, output);
            encode_reg(*flags, output);
            encode_reg(*fd, output);
            encode_reg(*offset, output);
        }

        Instruction::Munmap { dst, addr, len } => {
            output.push(Opcode::Munmap.to_byte());
            encode_reg(*dst, output);
            encode_reg(*addr, output);
            encode_reg(*len, output);
        }

        Instruction::AtomicLoad { dst, ptr, ordering, size } => {
            output.push(Opcode::AtomicLoad.to_byte());
            encode_reg(*dst, output);
            encode_reg(*ptr, output);
            output.push(*ordering);
            output.push(*size);
        }

        Instruction::AtomicStore { ptr, val, ordering, size } => {
            output.push(Opcode::AtomicStore.to_byte());
            encode_reg(*ptr, output);
            encode_reg(*val, output);
            output.push(*ordering);
            output.push(*size);
        }

        Instruction::AtomicCas { dst, ptr, expected, desired, ordering, size } => {
            output.push(Opcode::AtomicCas.to_byte());
            encode_reg(*dst, output);
            encode_reg(*ptr, output);
            encode_reg(*expected, output);
            encode_reg(*desired, output);
            output.push(*ordering);
            output.push(*size);
        }

        Instruction::AtomicFence { ordering } => {
            output.push(Opcode::AtomicFence.to_byte());
            output.push(*ordering);
        }

        Instruction::IoSubmit { dst, engine, ops } => {
            output.push(Opcode::IoSubmit.to_byte());
            encode_reg(*dst, output);
            encode_reg(*engine, output);
            encode_reg(*ops, output);
        }

        Instruction::IoPoll { dst, engine, timeout } => {
            output.push(Opcode::IoPoll.to_byte());
            encode_reg(*dst, output);
            encode_reg(*engine, output);
            encode_reg(*timeout, output);
        }

        Instruction::TlsGet { dst, slot } => {
            output.push(Opcode::TlsGet.to_byte());
            encode_reg(*dst, output);
            encode_reg(*slot, output);
        }

        Instruction::TlsSet { slot, val } => {
            output.push(Opcode::TlsSet.to_byte());
            encode_reg(*slot, output);
            encode_reg(*val, output);
        }

        // ====================================================================
        // Arithmetic Extended
        // ====================================================================
        Instruction::ArithExtended { sub_op, operands } => {
            output.push(Opcode::ArithExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // Tensor Extended
        // ====================================================================
        Instruction::TensorExtended { sub_op, operands } => {
            output.push(Opcode::TensorExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // Math Extended
        // ====================================================================
        Instruction::MathExtended { sub_op, operands } => {
            output.push(Opcode::MathExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // SIMD Extended
        // ====================================================================
        Instruction::SimdExtended { sub_op, operands } => {
            output.push(Opcode::SimdExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // Char Extended
        // ====================================================================
        Instruction::CharExtended { sub_op, operands } => {
            output.push(Opcode::CharExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // CBGR Extended
        // ====================================================================
        Instruction::CbgrExtended { sub_op, operands } => {
            output.push(Opcode::CbgrExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // Text Extended
        // ====================================================================
        Instruction::TextExtended { sub_op, operands } => {
            output.push(Opcode::TextExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // GPU Extended
        // ====================================================================
        Instruction::GpuExtended { sub_op, operands } => {
            output.push(Opcode::GpuExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // FFI Extended
        // ====================================================================
        Instruction::FfiExtended { sub_op, operands } => {
            output.push(Opcode::FfiExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // Memory Extended
        // ====================================================================
        Instruction::MemExtended { sub_op, operands } => {
            output.push(Opcode::MemExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // Logging Extended
        // ====================================================================
        Instruction::LogExtended { sub_op, operands } => {
            output.push(Opcode::LogExtended.to_byte());
            output.push(*sub_op);
            output.extend_from_slice(operands);
        }

        // ====================================================================
        // Meta-Programming
        // ====================================================================
        Instruction::MetaQuote { dst, bytes_const_id } => {
            output.push(Opcode::MetaQuote.to_byte());
            encode_reg(*dst, output);
            encode_varint(*bytes_const_id as u64, output);
        }

        Instruction::MetaSplice { src } => {
            output.push(Opcode::MetaSplice.to_byte());
            encode_reg(*src, output);
        }

        Instruction::MetaEval { dst, expr } => {
            output.push(Opcode::MetaEval.to_byte());
            encode_reg(*dst, output);
            encode_reg(*expr, output);
        }

        Instruction::MetaReflect { dst, type_id } => {
            output.push(Opcode::MetaReflect.to_byte());
            encode_reg(*dst, output);
            encode_varint(*type_id as u64, output);
        }

        // ====================================================================
        // Optimization hint instructions (metadata-only, encode as Nop)
        // ====================================================================
        Instruction::LoopHint { .. }
        | Instruction::BranchHint { .. }
        | Instruction::PrefetchHint { .. }
        | Instruction::OptBarrier { .. } => {
            output.push(Opcode::Nop.to_byte());
        }

        // ====================================================================
        // Raw (passthrough)
        // ====================================================================
        Instruction::Raw { opcode, data } => {
            output.push(opcode.to_byte());
            output.extend_from_slice(data);
        }

        // ====================================================================
        // Cubical type theory (CubicalExtended = 0xDE)
        // ====================================================================
        Instruction::CubicalExtended { sub_op, dst, args } => {
            output.push(Opcode::CubicalExtended.to_byte());
            output.push(*sub_op);
            output.push(dst.0 as u8);
            output.push(args.len() as u8);
            for arg in args {
                output.push(arg.0 as u8);
            }
        }
    }

    output.len() - start_len
}

/// Encodes a vector of registers.
#[inline]
fn encode_reg_vec(regs: &[Reg], output: &mut Vec<u8>) {
    encode_varint(regs.len() as u64, output);
    for r in regs {
        encode_reg(*r, output);
    }
}

/// Encodes an optional register.
#[inline]
fn encode_optional_reg(reg: Option<Reg>, output: &mut Vec<u8>) {
    match reg {
        Some(r) => {
            output.push(1);
            encode_reg(r, output);
        }
        None => {
            output.push(0);
        }
    }
}

/// Encodes a TypeRef.
///
/// Encoding format matches the actual TypeRef enum variants:
/// - 0x00: Concrete(TypeId)
/// - 0x01: Generic(TypeParamId)
/// - 0x02: Instantiated { base, args }
/// - 0x03: Function { params, return_type, contexts }
/// - 0x04: Reference { inner, mutability, tier }
/// - 0x05: Tuple(elems)
/// - 0x06: Array { element, length }
/// - 0x07: Slice(inner)
/// - 0x08: Rank2Function { type_param_count, params, return_type, contexts }
#[inline]
fn encode_type_ref(type_ref: &TypeRef, output: &mut Vec<u8>) {
    match type_ref {
        TypeRef::Concrete(type_id) => {
            output.push(0x00);
            encode_varint(type_id.0 as u64, output);
        }
        TypeRef::Generic(type_param_id) => {
            output.push(0x01);
            encode_varint(type_param_id.0 as u64, output);
        }
        TypeRef::Instantiated { base, args } => {
            output.push(0x02);
            encode_varint(base.0 as u64, output);
            encode_varint(args.len() as u64, output);
            for arg in args {
                encode_type_ref(arg, output);
            }
        }
        TypeRef::Function {
            params,
            return_type,
            contexts,
        } => {
            output.push(0x03);
            encode_varint(params.len() as u64, output);
            for p in params {
                encode_type_ref(p, output);
            }
            encode_type_ref(return_type, output);
            encode_varint(contexts.len() as u64, output);
            for ctx in contexts {
                encode_varint(ctx.0 as u64, output);
            }
        }
        TypeRef::Reference {
            inner,
            mutability,
            tier,
        } => {
            output.push(0x04);
            encode_type_ref(inner, output);
            output.push(*mutability as u8);
            output.push(*tier as u8);
        }
        TypeRef::Tuple(elems) => {
            output.push(0x05);
            encode_varint(elems.len() as u64, output);
            for e in elems {
                encode_type_ref(e, output);
            }
        }
        TypeRef::Array { element, length } => {
            output.push(0x06);
            encode_type_ref(element, output);
            encode_varint(*length, output);
        }
        TypeRef::Slice(inner) => {
            output.push(0x07);
            encode_type_ref(inner, output);
        }
        TypeRef::Rank2Function {
            type_param_count,
            params,
            return_type,
            contexts,
        } => {
            output.push(0x08);
            encode_varint(*type_param_count as u64, output);
            encode_varint(params.len() as u64, output);
            for p in params {
                encode_type_ref(p, output);
            }
            encode_type_ref(return_type, output);
            encode_varint(contexts.len() as u64, output);
            for ctx in contexts {
                encode_varint(ctx.0 as u64, output);
            }
        }
    }
}

// ============================================================================
// Instruction Decoding
// ============================================================================

/// Decodes an instruction from bytecode.
///
/// Updates `offset` to point past the decoded instruction.
pub fn decode_instruction(data: &[u8], offset: &mut usize) -> VbcResult<Instruction> {
    let opcode_byte = decode_u8(data, offset)?;
    let opcode = Opcode::from_byte(opcode_byte);

    match opcode {
        // ====================================================================
        // Data Movement
        // ====================================================================
        Opcode::Mov => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::Mov { dst, src })
        }

        Opcode::LoadK => {
            let dst = decode_reg(data, offset)?;
            let const_id = decode_varint(data, offset)? as u32;
            Ok(Instruction::LoadK { dst, const_id })
        }

        Opcode::LoadI => {
            let dst = decode_reg(data, offset)?;
            let value = decode_signed_varint(data, offset)?;
            Ok(Instruction::LoadI { dst, value })
        }

        Opcode::LoadF => {
            let dst = decode_reg(data, offset)?;
            let value = decode_f64(data, offset)?;
            Ok(Instruction::LoadF { dst, value })
        }

        Opcode::LoadTrue => {
            let dst = decode_reg(data, offset)?;
            Ok(Instruction::LoadTrue { dst })
        }

        Opcode::LoadFalse => {
            let dst = decode_reg(data, offset)?;
            Ok(Instruction::LoadFalse { dst })
        }

        Opcode::LoadUnit => {
            let dst = decode_reg(data, offset)?;
            Ok(Instruction::LoadUnit { dst })
        }

        Opcode::LoadT => {
            let dst = decode_reg(data, offset)?;
            let type_ref = decode_type_ref(data, offset)?;
            Ok(Instruction::LoadT { dst, type_ref })
        }

        Opcode::LoadSmallI => {
            let dst = decode_reg(data, offset)?;
            let value = decode_u8(data, offset)? as i8;
            Ok(Instruction::LoadSmallI { dst, value })
        }

        Opcode::LoadNil => {
            let dst = decode_reg(data, offset)?;
            Ok(Instruction::LoadNil { dst })
        }

        // ====================================================================
        // Type Conversions
        // ====================================================================
        Opcode::CvtIF => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::CvtIF { dst, src })
        }

        Opcode::CvtFI => {
            let mode_byte = decode_u8(data, offset)?;
            let mode = FloatToIntMode::from_byte(mode_byte).unwrap_or(FloatToIntMode::Trunc);
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::CvtFI { mode, dst, src })
        }

        Opcode::CvtIC => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::CvtIC { dst, src })
        }

        Opcode::CvtCI => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::CvtCI { dst, src })
        }

        Opcode::CvtBI => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::CvtBI { dst, src })
        }

        Opcode::CvtToI => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::CvtToI { dst, src })
        }

        Opcode::CvtToF => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::CvtToF { dst, src })
        }

        // ====================================================================
        // Integer Arithmetic
        // ====================================================================
        Opcode::AddI => decode_binary_i(data, offset, BinaryIntOp::Add),
        Opcode::SubI => decode_binary_i(data, offset, BinaryIntOp::Sub),
        Opcode::MulI => decode_binary_i(data, offset, BinaryIntOp::Mul),
        Opcode::DivI => decode_binary_i(data, offset, BinaryIntOp::Div),
        Opcode::ModI => decode_binary_i(data, offset, BinaryIntOp::Mod),
        Opcode::PowI => decode_binary_i(data, offset, BinaryIntOp::Pow),

        // ====================================================================
        // Float Arithmetic
        // ====================================================================
        Opcode::AddF => decode_binary_f(data, offset, BinaryFloatOp::Add),
        Opcode::SubF => decode_binary_f(data, offset, BinaryFloatOp::Sub),
        Opcode::MulF => decode_binary_f(data, offset, BinaryFloatOp::Mul),
        Opcode::DivF => decode_binary_f(data, offset, BinaryFloatOp::Div),
        Opcode::PowF => decode_binary_f(data, offset, BinaryFloatOp::Pow),
        Opcode::ModF => decode_binary_f(data, offset, BinaryFloatOp::Mod),

        // ====================================================================
        // Generic Arithmetic
        // ====================================================================
        Opcode::AddG => decode_binary_g(data, offset, BinaryGenericOp::Add),
        Opcode::SubG => decode_binary_g(data, offset, BinaryGenericOp::Sub),
        Opcode::MulG => decode_binary_g(data, offset, BinaryGenericOp::Mul),
        Opcode::DivG => decode_binary_g(data, offset, BinaryGenericOp::Div),

        // ====================================================================
        // Unary Operations
        // ====================================================================
        Opcode::NegI => decode_unary_i(data, offset, UnaryIntOp::Neg),
        Opcode::NegF => {
            // Check if next byte is sub-opcode for UnaryF
            let sub_op = decode_u8(data, offset)?;
            let op = UnaryFloatOp::try_from(sub_op).unwrap_or(UnaryFloatOp::Neg);
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::UnaryF { op, dst, src })
        }
        Opcode::AbsI => decode_unary_i(data, offset, UnaryIntOp::Abs),
        Opcode::AbsF => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::UnaryF {
                op: UnaryFloatOp::Abs,
                dst,
                src,
            })
        }
        Opcode::Inc => decode_unary_i(data, offset, UnaryIntOp::Inc),
        Opcode::Dec => decode_unary_i(data, offset, UnaryIntOp::Dec),
        Opcode::Not => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::Not { dst, src })
        }

        // ====================================================================
        // Bitwise Operations
        // ====================================================================
        Opcode::Band => decode_bitwise(data, offset, BitwiseOp::And),
        Opcode::Bor => decode_bitwise(data, offset, BitwiseOp::Or),
        Opcode::Bxor => decode_bitwise(data, offset, BitwiseOp::Xor),
        Opcode::Bnot => decode_bitwise(data, offset, BitwiseOp::Not),
        Opcode::Shl => decode_bitwise(data, offset, BitwiseOp::Shl),
        Opcode::Shr => decode_bitwise(data, offset, BitwiseOp::Shr),
        Opcode::Ushr => decode_bitwise(data, offset, BitwiseOp::Ushr),

        // ====================================================================
        // Integer Comparison
        // ====================================================================
        Opcode::EqI => decode_cmp_i(data, offset, CompareOp::Eq),
        Opcode::NeI => decode_cmp_i(data, offset, CompareOp::Ne),
        Opcode::LtI => decode_cmp_i(data, offset, CompareOp::Lt),
        Opcode::LeI => decode_cmp_i(data, offset, CompareOp::Le),
        Opcode::GtI => decode_cmp_i(data, offset, CompareOp::Gt),
        Opcode::GeI => decode_cmp_i(data, offset, CompareOp::Ge),

        // ====================================================================
        // Float Comparison
        // ====================================================================
        Opcode::EqF => decode_cmp_f(data, offset, CompareOp::Eq),
        Opcode::NeF => decode_cmp_f(data, offset, CompareOp::Ne),
        Opcode::LtF => decode_cmp_f(data, offset, CompareOp::Lt),
        Opcode::LeF => decode_cmp_f(data, offset, CompareOp::Le),
        Opcode::GtF => decode_cmp_f(data, offset, CompareOp::Gt),
        Opcode::GeF => decode_cmp_f(data, offset, CompareOp::Ge),

        // ====================================================================
        // Generic Comparison
        // ====================================================================
        Opcode::EqG => {
            let dst = decode_reg(data, offset)?;
            let a = decode_reg(data, offset)?;
            let b = decode_reg(data, offset)?;
            let protocol_id = decode_varint(data, offset)? as u32;
            Ok(Instruction::CmpG {
                eq: true,
                dst,
                a,
                b,
                protocol_id,
            })
        }
        Opcode::CmpG => {
            let dst = decode_reg(data, offset)?;
            let a = decode_reg(data, offset)?;
            let b = decode_reg(data, offset)?;
            let protocol_id = decode_varint(data, offset)? as u32;
            Ok(Instruction::CmpG {
                eq: false,
                dst,
                a,
                b,
                protocol_id,
            })
        }
        Opcode::EqRef => {
            // Reference equality - decode as CmpI with Eq op
            let dst = decode_reg(data, offset)?;
            let a = decode_reg(data, offset)?;
            let b = decode_reg(data, offset)?;
            Ok(Instruction::CmpI {
                op: CompareOp::Eq,
                dst,
                a,
                b,
            })
        }

        Opcode::CmpExtended => {
            let sub_byte = decode_u8(data, offset)?;
            let sub_op = CmpSubOpcode::from_byte(sub_byte).ok_or({
                VbcError::InvalidOpcode(sub_byte)
            })?;
            let dst = decode_reg(data, offset)?;
            let a = decode_reg(data, offset)?;
            let b = decode_reg(data, offset)?;
            Ok(Instruction::CmpU { sub_op, dst, a, b })
        }

        // ====================================================================
        // Logic Operations
        // ====================================================================
        Opcode::And | Opcode::Or | Opcode::Xor => {
            // These are boolean logic, treat as bitwise for now
            let op = match opcode {
                Opcode::And => BitwiseOp::And,
                Opcode::Or => BitwiseOp::Or,
                Opcode::Xor => BitwiseOp::Xor,
                _ => unreachable!(),
            };
            decode_bitwise(data, offset, op)
        }

        // ====================================================================
        // Control Flow
        // ====================================================================
        Opcode::Jmp => {
            let offset_val = decode_signed_varint(data, offset)? as i32;
            Ok(Instruction::Jmp {
                offset: offset_val,
            })
        }

        Opcode::JmpIf => {
            let cond = decode_reg(data, offset)?;
            let offset_val = decode_signed_varint(data, offset)? as i32;
            Ok(Instruction::JmpIf {
                cond,
                offset: offset_val,
            })
        }

        Opcode::JmpNot => {
            let cond = decode_reg(data, offset)?;
            let offset_val = decode_signed_varint(data, offset)? as i32;
            Ok(Instruction::JmpNot {
                cond,
                offset: offset_val,
            })
        }

        Opcode::JmpEq => decode_jmp_cmp(data, offset, CompareOp::Eq),
        Opcode::JmpNe => decode_jmp_cmp(data, offset, CompareOp::Ne),
        Opcode::JmpLt => decode_jmp_cmp(data, offset, CompareOp::Lt),
        Opcode::JmpLe => decode_jmp_cmp(data, offset, CompareOp::Le),
        Opcode::JmpGt => decode_jmp_cmp(data, offset, CompareOp::Gt),
        Opcode::JmpGe => decode_jmp_cmp(data, offset, CompareOp::Ge),

        Opcode::Ret => {
            let value = decode_reg(data, offset)?;
            Ok(Instruction::Ret { value })
        }

        Opcode::RetV => Ok(Instruction::RetV),

        Opcode::Call => {
            let dst = decode_reg(data, offset)?;
            let func_id = decode_varint(data, offset)? as u32;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::Call { dst, func_id, args })
        }

        Opcode::TailCall => {
            let func_id = decode_varint(data, offset)? as u32;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::TailCall { func_id, args })
        }

        Opcode::CallM => {
            let dst = decode_reg(data, offset)?;
            let receiver = decode_reg(data, offset)?;
            let method_id = decode_varint(data, offset)? as u32;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::CallM {
                dst,
                receiver,
                method_id,
                args,
            })
        }

        Opcode::CallClosure => {
            let dst = decode_reg(data, offset)?;
            let closure = decode_reg(data, offset)?;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::CallClosure { dst, closure, args })
        }

        Opcode::NewClosure => {
            let dst = decode_reg(data, offset)?;
            let func_id = decode_varint(data, offset)? as u32;
            let captures = decode_reg_vec(data, offset)?;
            Ok(Instruction::NewClosure { dst, func_id, captures })
        }

        // ====================================================================
        // Memory Operations
        // ====================================================================
        Opcode::New => {
            let dst = decode_reg(data, offset)?;
            let type_id = decode_varint(data, offset)? as u32;
            let field_count = decode_varint(data, offset)? as u32;
            Ok(Instruction::New { dst, type_id, field_count })
        }

        Opcode::NewG => {
            let dst = decode_reg(data, offset)?;
            let type_id = decode_varint(data, offset)? as u32;
            let type_args = decode_reg_vec(data, offset)?;
            Ok(Instruction::NewG {
                dst,
                type_id,
                type_args,
            })
        }

        Opcode::GetF => {
            let dst = decode_reg(data, offset)?;
            let obj = decode_reg(data, offset)?;
            let field_idx = decode_varint(data, offset)? as u32;
            Ok(Instruction::GetF { dst, obj, field_idx })
        }

        Opcode::SetF => {
            let obj = decode_reg(data, offset)?;
            let field_idx = decode_varint(data, offset)? as u32;
            let value = decode_reg(data, offset)?;
            Ok(Instruction::SetF {
                obj,
                field_idx,
                value,
            })
        }

        Opcode::GetE => {
            let dst = decode_reg(data, offset)?;
            let arr = decode_reg(data, offset)?;
            let idx = decode_reg(data, offset)?;
            Ok(Instruction::GetE { dst, arr, idx })
        }

        Opcode::SetE => {
            let arr = decode_reg(data, offset)?;
            let idx = decode_reg(data, offset)?;
            let value = decode_reg(data, offset)?;
            Ok(Instruction::SetE { arr, idx, value })
        }

        Opcode::Len => {
            let dst = decode_reg(data, offset)?;
            let arr = decode_reg(data, offset)?;
            let type_hint = if *offset < data.len() {
                let b = data[*offset];
                *offset += 1;
                b
            } else {
                0 // backward compatibility
            };
            Ok(Instruction::Len { dst, arr, type_hint })
        }

        Opcode::NewArray => {
            // NewArray not in Instruction enum, use Raw
            Ok(Instruction::Raw {
                opcode,
                data: vec![],
            })
        }

        Opcode::NewList => {
            let dst = decode_reg(data, offset)?;
            Ok(Instruction::NewList { dst })
        }

        Opcode::ListPush => {
            let list = decode_reg(data, offset)?;
            let val = decode_reg(data, offset)?;
            Ok(Instruction::ListPush { list, val })
        }

        Opcode::ListPop => {
            let dst = decode_reg(data, offset)?;
            let list = decode_reg(data, offset)?;
            Ok(Instruction::ListPop { dst, list })
        }

        Opcode::NewMap => {
            let dst = decode_reg(data, offset)?;
            Ok(Instruction::NewMap { dst })
        }

        Opcode::MapGet => {
            let dst = decode_reg(data, offset)?;
            let map = decode_reg(data, offset)?;
            let key = decode_reg(data, offset)?;
            Ok(Instruction::MapGet { dst, map, key })
        }

        Opcode::MapSet => {
            let map = decode_reg(data, offset)?;
            let key = decode_reg(data, offset)?;
            let val = decode_reg(data, offset)?;
            Ok(Instruction::MapSet { map, key, val })
        }

        Opcode::MapContains => {
            let dst = decode_reg(data, offset)?;
            let map = decode_reg(data, offset)?;
            let key = decode_reg(data, offset)?;
            Ok(Instruction::MapContains { dst, map, key })
        }

        // ====================================================================
        // CBGR Operations
        // ====================================================================
        Opcode::Ref => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::Ref { dst, src })
        }

        Opcode::RefMut => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::RefMut { dst, src })
        }

        Opcode::Deref => {
            let dst = decode_reg(data, offset)?;
            let ref_reg = decode_reg(data, offset)?;
            Ok(Instruction::Deref { dst, ref_reg })
        }

        Opcode::DerefMut => {
            let ref_reg = decode_reg(data, offset)?;
            let value = decode_reg(data, offset)?;
            Ok(Instruction::DerefMut { ref_reg, value })
        }

        Opcode::ChkRef => {
            let ref_reg = decode_reg(data, offset)?;
            Ok(Instruction::ChkRef { ref_reg })
        }

        Opcode::RefChecked => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::RefChecked { dst, src })
        }

        Opcode::RefUnsafe => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::RefUnsafe { dst, src })
        }

        Opcode::DropRef => {
            let src = decode_reg(data, offset)?;
            Ok(Instruction::DropRef { src })
        }

        Opcode::Clone => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::Clone { dst, src })
        }

        // ====================================================================
        // Generic Operations
        // ====================================================================
        Opcode::CallG => {
            let dst = decode_reg(data, offset)?;
            let func_id = decode_varint(data, offset)? as u32;
            let type_args = decode_reg_vec(data, offset)?;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::CallG {
                dst,
                func_id,
                type_args,
                args,
            })
        }

        Opcode::CallV => {
            let dst = decode_reg(data, offset)?;
            let receiver = decode_reg(data, offset)?;
            let vtable_slot = decode_varint(data, offset)? as u32;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::CallV {
                dst,
                receiver,
                vtable_slot,
                args,
            })
        }

        Opcode::CallC => {
            let dst = decode_reg(data, offset)?;
            let cache_id = decode_varint(data, offset)? as u32;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::CallC { dst, cache_id, args })
        }

        Opcode::SizeOfG | Opcode::AlignOfG | Opcode::Instantiate => {
            // Use Raw for now
            Ok(Instruction::Raw {
                opcode,
                data: vec![],
            })
        }

        // ====================================================================
        // Pattern Matching
        // ====================================================================
        Opcode::IsVar => {
            let dst = decode_reg(data, offset)?;
            let value = decode_reg(data, offset)?;
            let tag = decode_varint(data, offset)? as u32;
            Ok(Instruction::IsVar { dst, value, tag })
        }

        Opcode::AsVar => {
            let dst = decode_reg(data, offset)?;
            let value = decode_reg(data, offset)?;
            let tag = decode_varint(data, offset)? as u32;
            Ok(Instruction::AsVar { dst, value, tag })
        }

        Opcode::Unpack => {
            let dst_start = decode_reg(data, offset)?;
            let tuple = decode_reg(data, offset)?;
            let count = decode_u8(data, offset)?;
            Ok(Instruction::Unpack {
                dst_start,
                tuple,
                count,
            })
        }

        Opcode::Pack => {
            let dst = decode_reg(data, offset)?;
            let src_start = decode_reg(data, offset)?;
            let count = decode_u8(data, offset)?;
            Ok(Instruction::Pack {
                dst,
                src_start,
                count,
            })
        }

        Opcode::Switch => {
            let value = decode_reg(data, offset)?;
            let default_offset = decode_signed_varint(data, offset)? as i32;
            let case_count = decode_varint(data, offset)? as usize;
            let mut cases = Vec::with_capacity(case_count);
            for _ in 0..case_count {
                let case_val = decode_varint(data, offset)? as u32;
                let case_offset = decode_signed_varint(data, offset)? as i32;
                cases.push((case_val, case_offset));
            }
            Ok(Instruction::Switch {
                value,
                default_offset,
                cases,
            })
        }

        Opcode::MatchGuard => {
            Ok(Instruction::Raw {
                opcode,
                data: vec![],
            })
        }

        // ====================================================================
        // Generator Operations: decoding for GenCreate/GenNext/GenHasNext/Yield opcodes.
        // ====================================================================

        Opcode::GenCreate => {
            let dst = decode_reg(data, offset)?;
            let func_id = decode_varint(data, offset)? as u32;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::GenCreate { dst, func_id, args })
        }

        Opcode::GenNext => {
            let dst = decode_reg(data, offset)?;
            let generator = decode_reg(data, offset)?;
            Ok(Instruction::GenNext { dst, generator })
        }

        Opcode::GenHasNext => {
            let dst = decode_reg(data, offset)?;
            let generator = decode_reg(data, offset)?;
            Ok(Instruction::GenHasNext { dst, generator })
        }

        // ====================================================================
        // Async Operations
        // ====================================================================
        Opcode::Spawn => {
            let dst = decode_reg(data, offset)?;
            let func_id = decode_varint(data, offset)? as u32;
            let args = decode_reg_range(data, offset)?;
            Ok(Instruction::Spawn { dst, func_id, args })
        }

        Opcode::Await => {
            let dst = decode_reg(data, offset)?;
            let task = decode_reg(data, offset)?;
            Ok(Instruction::Await { dst, task })
        }

        Opcode::Yield => {
            let value = decode_reg(data, offset)?;
            Ok(Instruction::Yield { value })
        }

        Opcode::Select => {
            let dst = decode_reg(data, offset)?;
            let futures = decode_reg_vec(data, offset)?;
            let handler_count = decode_varint(data, offset)? as usize;
            let mut handlers = Vec::with_capacity(handler_count);
            for _ in 0..handler_count {
                handlers.push(decode_signed_varint(data, offset)? as i32);
            }
            Ok(Instruction::Select {
                dst,
                futures,
                handlers,
            })
        }

        Opcode::Join => {
            // Map to Raw for now
            Ok(Instruction::Raw {
                opcode,
                data: vec![],
            })
        }

        // ====================================================================
        // Autodiff Operations
        // ====================================================================
        Opcode::GradBegin => {
            let scope_id = decode_varint(data, offset)? as u32;
            let mode_byte = decode_u8(data, offset)?;
            let mode = match mode_byte {
                0 => GradMode::Reverse,
                1 => GradMode::Forward,
                _ => GradMode::Auto,
            };
            let wrt = decode_reg_vec(data, offset)?;
            Ok(Instruction::GradBegin { scope_id, mode, wrt })
        }

        Opcode::GradEnd => {
            let scope_id = decode_varint(data, offset)? as u32;
            let output = decode_reg(data, offset)?;
            let grad_out = decode_reg(data, offset)?;
            let grad_regs = decode_reg_vec(data, offset)?;
            Ok(Instruction::GradEnd {
                scope_id,
                output,
                grad_out,
                grad_regs,
            })
        }

        Opcode::GradCheckpoint => {
            let id = decode_varint(data, offset)? as u32;
            let tensors = decode_reg_vec(data, offset)?;
            Ok(Instruction::GradCheckpoint { id, tensors })
        }

        Opcode::GradAccumulate => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::GradAccumulate { dst, src })
        }

        Opcode::GradStop => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::GradStop { dst, src })
        }

        // ====================================================================
        // Context Operations
        // ====================================================================
        Opcode::CtxGet => {
            let dst = decode_reg(data, offset)?;
            let ctx_type = decode_varint(data, offset)? as u32;
            Ok(Instruction::CtxGet { dst, ctx_type })
        }

        Opcode::CtxProvide => {
            let ctx_type = decode_varint(data, offset)? as u32;
            let value = decode_reg(data, offset)?;
            let body_offset = decode_signed_varint(data, offset)? as i32;
            Ok(Instruction::CtxProvide {
                ctx_type,
                value,
                body_offset,
            })
        }

        Opcode::CtxEnd => Ok(Instruction::CtxEnd),

        // ====================================================================
        // Debug/Verify Operations
        // ====================================================================
        Opcode::Spec => {
            let reg = decode_reg(data, offset)?;
            let expected_type = decode_varint(data, offset)? as u32;
            Ok(Instruction::Spec { reg, expected_type })
        }

        Opcode::Guard => {
            let reg = decode_reg(data, offset)?;
            let expected_type = decode_varint(data, offset)? as u32;
            let deopt_offset = decode_signed_varint(data, offset)? as i32;
            Ok(Instruction::Guard {
                reg,
                expected_type,
                deopt_offset,
            })
        }

        Opcode::Assert => {
            let cond = decode_reg(data, offset)?;
            let message_id = decode_varint(data, offset)? as u32;
            Ok(Instruction::Assert { cond, message_id })
        }

        Opcode::Panic => {
            let message_id = decode_varint(data, offset)? as u32;
            Ok(Instruction::Panic { message_id })
        }

        Opcode::Unreachable => Ok(Instruction::Unreachable),

        Opcode::Requires | Opcode::Ensures | Opcode::Invariant => {
            Ok(Instruction::Raw {
                opcode,
                data: vec![],
            })
        }

        // ====================================================================
        // Tensor Operations
        // ====================================================================
        Opcode::TensorNew => {
            let dst = decode_reg(data, offset)?;
            let dtype_byte = decode_u8(data, offset)?;
            let dtype = TensorDType::try_from(dtype_byte).unwrap_or(TensorDType::F64);
            let dims = decode_reg_vec(data, offset)?;
            Ok(Instruction::TensorNew { dst, dtype, dims })
        }

        Opcode::TensorBinop => {
            let op_byte = decode_u8(data, offset)?;
            let op = TensorBinaryOp::try_from(op_byte).unwrap_or(TensorBinaryOp::Add);
            let dst = decode_reg(data, offset)?;
            let a = decode_reg(data, offset)?;
            let b = decode_reg(data, offset)?;
            Ok(Instruction::TensorBinop { op, dst, a, b })
        }

        Opcode::TensorUnop => {
            let op_byte = decode_u8(data, offset)?;
            let op = TensorUnaryOp::try_from(op_byte).unwrap_or(TensorUnaryOp::Neg);
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            Ok(Instruction::TensorUnop { op, dst, src })
        }

        Opcode::TensorMatmul => {
            let dst = decode_reg(data, offset)?;
            let a = decode_reg(data, offset)?;
            let b = decode_reg(data, offset)?;
            Ok(Instruction::TensorMatmul { dst, a, b })
        }

        Opcode::TensorReduce => {
            let op_byte = decode_u8(data, offset)?;
            let op = TensorReduceOp::try_from(op_byte).unwrap_or(TensorReduceOp::Sum);
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            let axes_len = decode_varint(data, offset)? as usize;
            let mut axes = Vec::with_capacity(axes_len);
            for _ in 0..axes_len {
                axes.push(decode_u8(data, offset)?);
            }
            let keepdim = decode_u8(data, offset)? != 0;
            Ok(Instruction::TensorReduce {
                op,
                dst,
                src,
                axes,
                keepdim,
            })
        }

        // Additional Tensor Operations
        Opcode::TensorFull => {
            let dst = decode_reg(data, offset)?;
            let value = decode_reg(data, offset)?;
            let shape = decode_reg_vec(data, offset)?;
            let dtype_byte = decode_u8(data, offset)?;
            let dtype = TensorDType::try_from(dtype_byte).unwrap_or(TensorDType::F32);
            Ok(Instruction::TensorFull { dst, value, shape, dtype })
        }

        Opcode::TensorFromSlice => {
            let dst = decode_reg(data, offset)?;
            let data_reg = decode_reg(data, offset)?;
            let shape = decode_reg_vec(data, offset)?;
            let dtype_byte = decode_u8(data, offset)?;
            let dtype = TensorDType::try_from(dtype_byte).unwrap_or(TensorDType::F32);
            Ok(Instruction::TensorFromSlice { dst, data: data_reg, shape, dtype })
        }

        Opcode::TensorReshape => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            let shape = decode_reg_vec(data, offset)?;
            Ok(Instruction::TensorReshape { dst, src, shape })
        }

        Opcode::TensorTranspose => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            let perm_len = decode_varint(data, offset)? as usize;
            let mut perm = Vec::with_capacity(perm_len);
            for _ in 0..perm_len {
                perm.push(decode_u8(data, offset)?);
            }
            Ok(Instruction::TensorTranspose { dst, src, perm })
        }

        Opcode::TensorSlice => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            let starts = decode_reg_vec(data, offset)?;
            let ends = decode_reg_vec(data, offset)?;
            Ok(Instruction::TensorSlice { dst, src, starts, ends })
        }

        Opcode::TensorExtended => {
            let sub_opcode_byte = decode_u8(data, offset)?;
            let sub_opcode = TensorSubOpcode::from_byte(sub_opcode_byte);

            match sub_opcode {
                Some(TensorSubOpcode::Pool) => {
                    // Pool sub-opcode: decode pool-specific operands
                    let op_byte = decode_u8(data, offset)?;
                    let op = TensorPoolOp::from_byte(op_byte);
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let kernel_size_len = decode_varint(data, offset)? as usize;
                    let mut kernel_size = Vec::with_capacity(kernel_size_len);
                    for _ in 0..kernel_size_len {
                        kernel_size.push(decode_u8(data, offset)?);
                    }
                    let stride_len = decode_varint(data, offset)? as usize;
                    let mut stride = Vec::with_capacity(stride_len);
                    for _ in 0..stride_len {
                        stride.push(decode_u8(data, offset)?);
                    }
                    let padding_len = decode_varint(data, offset)? as usize;
                    let mut padding = Vec::with_capacity(padding_len);
                    for _ in 0..padding_len {
                        padding.push(decode_u8(data, offset)?);
                    }
                    Ok(Instruction::TensorPool { op, dst, src, kernel_size, stride, padding })
                }
                // Register-based tensor ops — decode as generic TensorExtended
                // The interpreter reads operands directly from the byte stream.
                Some(TensorSubOpcode::NewFromArgs)
                | Some(TensorSubOpcode::FillFromArgs)
                | Some(TensorSubOpcode::FromSliceArgs)
                | Some(TensorSubOpcode::BinopFromArgs)
                | Some(TensorSubOpcode::UnopFromArgs)
                | Some(TensorSubOpcode::MatmulFromArgs)
                | Some(TensorSubOpcode::ReduceFromArgs)
                | Some(TensorSubOpcode::ReshapeFromArgs)
                | Some(TensorSubOpcode::TransposeFromArgs)
                | Some(TensorSubOpcode::SliceFromArgs)
                | Some(TensorSubOpcode::GetElementFromArgs)
                | Some(TensorSubOpcode::SetElementFromArgs) => {
                    // Read remaining bytes as raw operands
                    let _remaining_start = *offset;
                    // Estimate operand length by reading registers until end
                    // For roundtrip safety, read 2-4 registers (max operand count)
                    let mut operands = Vec::new();
                    // The operands were encoded by emit_intrinsic_tensor_extended which puts
                    // [dst:1-2b] [arg0:1-2b] [arg1:1-2b] ... — variable length.
                    // For decode, we need to reconstruct the raw bytes.
                    // Since these sub-opcodes have known parameter counts, read accordingly.
                    let param_count = match sub_opcode.unwrap() {
                        TensorSubOpcode::NewFromArgs => 3,  // dst, shape, dtype
                        TensorSubOpcode::FillFromArgs => 4, // dst, shape, value, dtype
                        TensorSubOpcode::FromSliceArgs => 4, // dst, data, shape, dtype
                        TensorSubOpcode::BinopFromArgs => 4, // dst, a, b, op
                        TensorSubOpcode::UnopFromArgs => 3,  // dst, src, op
                        TensorSubOpcode::MatmulFromArgs => 3, // dst, a, b
                        TensorSubOpcode::ReduceFromArgs => 4, // dst, src, op, axis
                        TensorSubOpcode::ReshapeFromArgs => 3, // dst, src, shape
                        TensorSubOpcode::TransposeFromArgs => 2, // dst, src
                        TensorSubOpcode::SliceFromArgs => 3, // dst, src, ranges
                        TensorSubOpcode::GetElementFromArgs => 3, // dst, src, index
                        TensorSubOpcode::SetElementFromArgs => 4, // dst, src, index, value
                        _ => 0,
                    };
                    for _ in 0..param_count {
                        let b = decode_u8(data, offset)?;
                        if b & 0x80 != 0 {
                            let lo = decode_u8(data, offset)?;
                            operands.push(b);
                            operands.push(lo);
                        } else {
                            operands.push(b);
                        }
                    }
                    Ok(Instruction::TensorExtended { sub_op: sub_opcode_byte, operands })
                }
                Some(TensorSubOpcode::Argmin) => {
                    // Argmin: dst, src, axis, keepdim
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let axis = decode_u8(data, offset)? as i8;
                    let keepdim = decode_u8(data, offset)? != 0;
                    Ok(Instruction::TensorArgmin { dst, src, axis, keepdim })
                }
                Some(TensorSubOpcode::Solve) => {
                    // Solve: dst, a, b
                    let dst = decode_reg(data, offset)?;
                    let a = decode_reg(data, offset)?;
                    let b = decode_reg(data, offset)?;
                    Ok(Instruction::TensorSolve { dst, a, b })
                }
                Some(TensorSubOpcode::Gather) => {
                    // Gather: dst, src, index, axis
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let index = decode_reg(data, offset)?;
                    let axis = decode_u8(data, offset)? as i8;
                    Ok(Instruction::TensorGather { dst, src, index, axis })
                }
                Some(TensorSubOpcode::Permute) => {
                    // Permute: dst, src, axes
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let axes_len = decode_varint(data, offset)? as usize;
                    let mut axes = Vec::with_capacity(axes_len);
                    for _ in 0..axes_len {
                        axes.push(decode_u8(data, offset)?);
                    }
                    Ok(Instruction::TensorPermute { dst, src, axes })
                }
                Some(TensorSubOpcode::QR) => {
                    // QR: q, r, src, mode
                    let q = decode_reg(data, offset)?;
                    let r = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let mode = decode_u8(data, offset)?;
                    Ok(Instruction::TensorQR { q, r, src, mode })
                }
                Some(TensorSubOpcode::SVD) => {
                    // SVD: u, s, vh, src, full_matrices, compute_uv
                    let u = decode_reg(data, offset)?;
                    let s = decode_reg(data, offset)?;
                    let vh = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let full_matrices = decode_u8(data, offset)? != 0;
                    let compute_uv = decode_u8(data, offset)? != 0;
                    Ok(Instruction::TensorSVD { u, s, vh, src, full_matrices, compute_uv })
                }
                Some(TensorSubOpcode::LU) => {
                    // LU: p, l, u, src
                    let p = decode_reg(data, offset)?;
                    let l = decode_reg(data, offset)?;
                    let u = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    Ok(Instruction::TensorLU { p, l, u, src })
                }
                Some(TensorSubOpcode::Eig) => {
                    // Eig: eigenvalues, eigenvectors, src, compute_v
                    let eigenvalues = decode_reg(data, offset)?;
                    let eigenvectors = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let compute_v = decode_u8(data, offset)? != 0;
                    Ok(Instruction::TensorEig { eigenvalues, eigenvectors, src, compute_v })
                }
                Some(TensorSubOpcode::EigSymmetric) => {
                    // EigSymmetric: eigenvalues, eigenvectors, src, upper
                    let eigenvalues = decode_reg(data, offset)?;
                    let eigenvectors = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let upper = decode_u8(data, offset)? != 0;
                    Ok(Instruction::TensorEigSymmetric { eigenvalues, eigenvectors, src, upper })
                }
                Some(TensorSubOpcode::Lstsq) => {
                    // Lstsq: x, residuals, rank, s, a, b, rcond
                    let x = decode_reg(data, offset)?;
                    let residuals = decode_reg(data, offset)?;
                    let rank_reg = decode_reg(data, offset)?;
                    let s = decode_reg(data, offset)?;
                    let a = decode_reg(data, offset)?;
                    let b = decode_reg(data, offset)?;
                    let rcond = decode_f64(data, offset)?;
                    Ok(Instruction::TensorLstsq { x, residuals, rank: rank_reg, s, a, b, rcond })
                }
                Some(TensorSubOpcode::Det) => {
                    // Det: dst, src
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    Ok(Instruction::TensorDet { dst, src })
                }
                Some(TensorSubOpcode::Trace) => {
                    // Trace: dst, src
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    Ok(Instruction::TensorTrace { dst, src })
                }
                Some(TensorSubOpcode::Norm) => {
                    // Norm: dst, src, ord
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let ord = decode_u8(data, offset)? as i8;
                    Ok(Instruction::TensorNorm { dst, src, ord })
                }

                // Tensor creation operations
                Some(TensorSubOpcode::Arange) => {
                    let dst = decode_reg(data, offset)?;
                    let start = decode_reg(data, offset)?;
                    let end = decode_reg(data, offset)?;
                    let step = decode_reg(data, offset)?;
                    let dtype = TensorDType::from_byte(decode_u8(data, offset)?);
                    Ok(Instruction::TensorArange { dst, start, end, step, dtype })
                }
                Some(TensorSubOpcode::Linspace) => {
                    let dst = decode_reg(data, offset)?;
                    let start = decode_reg(data, offset)?;
                    let end = decode_reg(data, offset)?;
                    let num = decode_reg(data, offset)?;
                    let dtype = TensorDType::from_byte(decode_u8(data, offset)?);
                    Ok(Instruction::TensorLinspace { dst, start, end, num, dtype })
                }
                Some(TensorSubOpcode::Rand) => {
                    let dst = decode_reg(data, offset)?;
                    let shape = decode_reg_vec(data, offset)?;
                    let dtype = TensorDType::from_byte(decode_u8(data, offset)?);
                    Ok(Instruction::TensorRand { dst, shape, dtype })
                }
                Some(TensorSubOpcode::Clone) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    Ok(Instruction::TensorClone { dst, src })
                }
                Some(TensorSubOpcode::Identity) => {
                    let dst = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let dtype = TensorDType::from_byte(decode_u8(data, offset)?);
                    Ok(Instruction::TensorIdentity { dst, size, dtype })
                }

                // Tensor manipulation operations
                Some(TensorSubOpcode::Index) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let indices = decode_reg(data, offset)?;
                    let axis = decode_u8(data, offset)?;
                    Ok(Instruction::TensorIndex { dst, src, indices, axis })
                }
                Some(TensorSubOpcode::Concat) => {
                    let dst = decode_reg(data, offset)?;
                    let tensors = decode_reg_vec(data, offset)?;
                    let axis = decode_u8(data, offset)?;
                    Ok(Instruction::TensorConcat { dst, tensors, axis })
                }
                Some(TensorSubOpcode::Stack) => {
                    let dst = decode_reg(data, offset)?;
                    let tensors = decode_reg_vec(data, offset)?;
                    let axis = decode_u8(data, offset)?;
                    Ok(Instruction::TensorStack { dst, tensors, axis })
                }
                Some(TensorSubOpcode::BroadcastToShape) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let shape = decode_reg_vec(data, offset)?;
                    Ok(Instruction::TensorBroadcast { dst, src, shape })
                }
                Some(TensorSubOpcode::Squeeze) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let axes_len = decode_varint(data, offset)? as usize;
                    let mut axes = Vec::with_capacity(axes_len);
                    for _ in 0..axes_len {
                        axes.push(decode_u8(data, offset)?);
                    }
                    Ok(Instruction::TensorSqueeze { dst, src, axes })
                }

                // Element-wise operations
                Some(TensorSubOpcode::Cmp) => {
                    let op = CompareOp::from_byte(decode_u8(data, offset)?);
                    let dst = decode_reg(data, offset)?;
                    let a = decode_reg(data, offset)?;
                    let b = decode_reg(data, offset)?;
                    Ok(Instruction::TensorCmp { op, dst, a, b })
                }
                Some(TensorSubOpcode::Where) => {
                    let dst = decode_reg(data, offset)?;
                    let cond = decode_reg(data, offset)?;
                    let x = decode_reg(data, offset)?;
                    let y = decode_reg(data, offset)?;
                    Ok(Instruction::TensorWhere { dst, cond, x, y })
                }
                Some(TensorSubOpcode::Clamp) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let min = decode_reg(data, offset)?;
                    let max = decode_reg(data, offset)?;
                    Ok(Instruction::TensorClamp { dst, src, min, max })
                }
                Some(TensorSubOpcode::Cast) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let dtype = TensorDType::from_byte(decode_u8(data, offset)?);
                    Ok(Instruction::TensorCast { dst, src, dtype })
                }
                Some(TensorSubOpcode::MaskedFill) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let mask = decode_reg(data, offset)?;
                    let value = decode_reg(data, offset)?;
                    Ok(Instruction::TensorMaskedFill { dst, src, mask, value })
                }
                Some(TensorSubOpcode::Lerp) => {
                    let dst = decode_reg(data, offset)?;
                    let a = decode_reg(data, offset)?;
                    let b = decode_reg(data, offset)?;
                    let t = decode_reg(data, offset)?;
                    Ok(Instruction::TensorLerp { dst, a, b, t })
                }

                // Computation operations
                Some(TensorSubOpcode::Dot) => {
                    let dst = decode_reg(data, offset)?;
                    let a = decode_reg(data, offset)?;
                    let b = decode_reg(data, offset)?;
                    let axes_a_len = decode_varint(data, offset)? as usize;
                    let mut axes_a = Vec::with_capacity(axes_a_len);
                    for _ in 0..axes_a_len {
                        axes_a.push(decode_u8(data, offset)?);
                    }
                    let axes_b_len = decode_varint(data, offset)? as usize;
                    let mut axes_b = Vec::with_capacity(axes_b_len);
                    for _ in 0..axes_b_len {
                        axes_b.push(decode_u8(data, offset)?);
                    }
                    Ok(Instruction::TensorDot { dst, a, b, axes_a, axes_b })
                }
                Some(TensorSubOpcode::Conv) => {
                    let dst = decode_reg(data, offset)?;
                    let input = decode_reg(data, offset)?;
                    let kernel = decode_reg(data, offset)?;
                    let bias = decode_optional_reg(data, offset)?;
                    let stride_len = decode_varint(data, offset)? as usize;
                    let mut stride = Vec::with_capacity(stride_len);
                    for _ in 0..stride_len {
                        stride.push(decode_u8(data, offset)?);
                    }
                    let padding_len = decode_varint(data, offset)? as usize;
                    let mut padding = Vec::with_capacity(padding_len);
                    for _ in 0..padding_len {
                        padding.push(decode_u8(data, offset)?);
                    }
                    let dilation_len = decode_varint(data, offset)? as usize;
                    let mut dilation = Vec::with_capacity(dilation_len);
                    for _ in 0..dilation_len {
                        dilation.push(decode_u8(data, offset)?);
                    }
                    let groups = decode_varint(data, offset)? as u8;
                    Ok(Instruction::TensorConv { dst, input, kernel, bias, stride, padding, dilation, groups })
                }
                Some(TensorSubOpcode::BatchMatmul) => {
                    let dst = decode_reg(data, offset)?;
                    let a = decode_reg(data, offset)?;
                    let b = decode_reg(data, offset)?;
                    Ok(Instruction::TensorBatchMatmul { dst, a, b })
                }
                Some(TensorSubOpcode::Einsum) => {
                    let dst = decode_reg(data, offset)?;
                    let inputs = decode_reg_vec(data, offset)?;
                    let equation_id = decode_varint(data, offset)? as u32;
                    Ok(Instruction::TensorEinsum { dst, inputs, equation_id })
                }
                Some(TensorSubOpcode::Outer) => {
                    let dst = decode_reg(data, offset)?;
                    let a = decode_reg(data, offset)?;
                    let b = decode_reg(data, offset)?;
                    Ok(Instruction::TensorOuter { dst, a, b })
                }

                // Linear algebra operations (moved from old opcodes)
                Some(TensorSubOpcode::TriSolve) => {
                    let dst = decode_reg(data, offset)?;
                    let a = decode_reg(data, offset)?;
                    let b = decode_reg(data, offset)?;
                    let upper = decode_u8(data, offset)? != 0;
                    Ok(Instruction::TensorTriSolve { dst, a, b, upper })
                }
                Some(TensorSubOpcode::Cholesky) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let upper = decode_u8(data, offset)? != 0;
                    Ok(Instruction::TensorCholesky { dst, src, upper })
                }

                // Statistics operations (moved from old opcodes)
                Some(TensorSubOpcode::Argmax) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let axis = decode_u8(data, offset)? as i8;
                    let keepdim = decode_u8(data, offset)? != 0;
                    Ok(Instruction::TensorArgmax { dst, src, axis, keepdim })
                }
                Some(TensorSubOpcode::Topk) => {
                    let values = decode_reg(data, offset)?;
                    let indices = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let k = decode_reg(data, offset)?;
                    let axis = decode_u8(data, offset)? as i8;
                    let largest = decode_u8(data, offset)? != 0;
                    Ok(Instruction::TensorTopk { values, indices, src, k, axis, largest })
                }
                Some(TensorSubOpcode::Cumulative) => {
                    let op_byte = decode_u8(data, offset)?;
                    let op = TensorCumulativeOp::from_byte(op_byte);
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let axis = decode_u8(data, offset)? as i8;
                    Ok(Instruction::TensorCumulative { op, dst, src, axis })
                }

                // Neural network operations (moved from old opcodes)
                Some(TensorSubOpcode::Softmax) => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let axis = decode_u8(data, offset)? as i8;
                    Ok(Instruction::TensorSoftmax { dst, src, axis })
                }
                Some(TensorSubOpcode::LayerNorm) => {
                    let dst = decode_reg(data, offset)?;
                    let input = decode_reg(data, offset)?;
                    let gamma = decode_optional_reg(data, offset)?;
                    let beta = decode_optional_reg(data, offset)?;
                    let normalized_shape = decode_varint(data, offset)? as u32;
                    let eps = decode_f32(data, offset)?;
                    Ok(Instruction::TensorLayerNorm { dst, input, gamma, beta, normalized_shape, eps })
                }
                Some(TensorSubOpcode::BatchNorm) => {
                    let dst = decode_reg(data, offset)?;
                    let input = decode_reg(data, offset)?;
                    let gamma = decode_optional_reg(data, offset)?;
                    let beta = decode_optional_reg(data, offset)?;
                    let running_mean = decode_optional_reg(data, offset)?;
                    let running_var = decode_optional_reg(data, offset)?;
                    let training = decode_u8(data, offset)? != 0;
                    let momentum = decode_f32(data, offset)?;
                    let eps = decode_f32(data, offset)?;
                    Ok(Instruction::TensorBatchNorm { dst, input, gamma, beta, running_mean, running_var, training, momentum, eps })
                }

                // Try TensorExtSubOpcode for overflow operations
                None => {
                    let ext_sub_opcode = TensorExtSubOpcode::from_byte(sub_opcode_byte);
                    match ext_sub_opcode {
                        Some(TensorExtSubOpcode::RmsNorm) => {
                            let dst = decode_reg(data, offset)?;
                            let input = decode_reg(data, offset)?;
                            let gamma = decode_optional_reg(data, offset)?;
                            let eps = decode_f32(data, offset)?;
                            Ok(Instruction::TensorRmsNorm { dst, input, gamma, eps })
                        }
                        Some(TensorExtSubOpcode::Fft) => {
                            let dst = decode_reg(data, offset)?;
                            let src = decode_reg(data, offset)?;
                            let dim = decode_u8(data, offset)? as i8;
                            let inverse = decode_u8(data, offset)? != 0;
                            Ok(Instruction::TensorFft { dst, src, dim, inverse })
                        }
                        Some(TensorExtSubOpcode::Scatter) => {
                            let dst = decode_reg(data, offset)?;
                            let src = decode_reg(data, offset)?;
                            let index = decode_reg(data, offset)?;
                            let values = decode_reg(data, offset)?;
                            let axis = decode_u8(data, offset)? as i8;
                            let mode = decode_u8(data, offset)?;
                            Ok(Instruction::TensorScatter { dst, src, index, values, axis, mode })
                        }
                        Some(TensorExtSubOpcode::FlashAttention) => {
                            // FlashAttention: dst, q, k, v, mask?, scale, causal
                            let dst = decode_reg(data, offset)?;
                            let q = decode_reg(data, offset)?;
                            let k = decode_reg(data, offset)?;
                            let v = decode_reg(data, offset)?;
                            let mask = decode_optional_reg(data, offset)?;
                            let scale = decode_reg(data, offset)?;
                            let causal = decode_u8(data, offset)? != 0;
                            Ok(Instruction::TensorFlashAttention { dst, q, k, v, mask, scale, causal })
                        }
                        Some(TensorExtSubOpcode::ContiguousView) => {
                            let dst = decode_reg(data, offset)?;
                            let src = decode_reg(data, offset)?;
                            Ok(Instruction::TensorContiguousView { dst, src })
                        }
                        Some(TensorExtSubOpcode::RandomU64) => {
                            let dst = decode_reg(data, offset)?;
                            Ok(Instruction::RandomU64 { dst })
                        }
                        Some(TensorExtSubOpcode::RandomFloat) => {
                            let dst = decode_reg(data, offset)?;
                            let low = decode_reg(data, offset)?;
                            let high = decode_reg(data, offset)?;
                            Ok(Instruction::RandomFloat { dst, low, high })
                        }
                        Some(TensorExtSubOpcode::GlobalAllocator) => {
                            let dst = decode_reg(data, offset)?;
                            Ok(Instruction::GlobalAllocator { dst })
                        }
                        Some(TensorExtSubOpcode::MemNewId) => {
                            let dst = decode_reg(data, offset)?;
                            Ok(Instruction::MemNewId { dst })
                        }
                        Some(TensorExtSubOpcode::MemAllocTensor) => {
                            let dst = decode_reg(data, offset)?;
                            let shape = decode_reg(data, offset)?;
                            let dtype = decode_u8(data, offset)?;
                            Ok(Instruction::MemAllocTensor { dst, shape, dtype })
                        }
                        None => Err(VbcError::InvalidOpcode(sub_opcode_byte)),
                    }
                }
                _ => Err(VbcError::InvalidOpcode(sub_opcode_byte)),
            }
        }

        // ====================================================================
        // GPU Operations - Fast Path
        // ====================================================================
        Opcode::GpuSync => {
            let stream = decode_reg(data, offset)?;
            Ok(Instruction::GpuSync { stream })
        }

        Opcode::GpuMemcpy => {
            let dst = decode_reg(data, offset)?;
            let src = decode_reg(data, offset)?;
            let direction = decode_u8(data, offset)?;
            Ok(Instruction::GpuMemcpy { dst, src, direction })
        }

        Opcode::GpuAlloc => {
            let dst = decode_reg(data, offset)?;
            let size = decode_reg(data, offset)?;
            let device = decode_reg(data, offset)?;
            Ok(Instruction::GpuAlloc { dst, size, device })
        }

        // ====================================================================
        // GPU Extended Operations
        // ====================================================================
        Opcode::GpuExtended => {
            let sub_op_byte = decode_u8(data, offset)?;
            let sub_op = GpuSubOpcode::from_byte(sub_op_byte)
                .ok_or(VbcError::InvalidOpcode(sub_op_byte))?;

            match sub_op {
                GpuSubOpcode::Launch => {
                    let kernel_id = decode_varint(data, offset)? as u32;
                    let mut grid = [Reg(0); 3];
                    let mut block = [Reg(0); 3];
                    for item in &mut grid {
                        *item = decode_reg(data, offset)?;
                    }
                    for item in &mut block {
                        *item = decode_reg(data, offset)?;
                    }
                    let shared_mem = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    let args = decode_reg_vec(data, offset)?;
                    Ok(Instruction::GpuLaunch { kernel_id, grid, block, shared_mem, stream, args })
                }

                GpuSubOpcode::LaunchCooperative => {
                    let kernel_id = decode_varint(data, offset)? as u32;
                    let mut grid = [Reg(0); 3];
                    let mut block = [Reg(0); 3];
                    for item in &mut grid {
                        *item = decode_reg(data, offset)?;
                    }
                    for item in &mut block {
                        *item = decode_reg(data, offset)?;
                    }
                    let shared_mem = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    let args = decode_reg_vec(data, offset)?;
                    Ok(Instruction::GpuLaunchCooperative { kernel_id, grid, block, shared_mem, stream, args })
                }

                GpuSubOpcode::LaunchMultiDevice => {
                    let kernel_id = decode_varint(data, offset)? as u32;
                    let devices = decode_reg(data, offset)?;
                    let mut grid = [Reg(0); 3];
                    let mut block = [Reg(0); 3];
                    for item in &mut grid {
                        *item = decode_reg(data, offset)?;
                    }
                    for item in &mut block {
                        *item = decode_reg(data, offset)?;
                    }
                    let shared_mem = decode_reg(data, offset)?;
                    let args = decode_reg_vec(data, offset)?;
                    Ok(Instruction::GpuLaunchMultiDevice { kernel_id, devices, grid, block, shared_mem, args })
                }

                GpuSubOpcode::SyncStream => {
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuSync { stream })
                }

                GpuSubOpcode::SyncDevice => {
                    Ok(Instruction::GpuDeviceSync)
                }

                GpuSubOpcode::SyncEvent => {
                    let event = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEventSynchronize { event })
                }

                GpuSubOpcode::QueryStream => {
                    let dst = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamQuery { dst, stream })
                }

                GpuSubOpcode::Memcpy => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let direction = decode_u8(data, offset)?;
                    Ok(Instruction::GpuMemcpy { dst, src, direction })
                }

                GpuSubOpcode::MemcpyAsync => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let direction = decode_u8(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemcpyAsync { dst, src, size, direction, stream })
                }

                GpuSubOpcode::Alloc => {
                    let dst = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let device = decode_reg(data, offset)?;
                    Ok(Instruction::GpuAlloc { dst, size, device })
                }

                GpuSubOpcode::Free => {
                    let ptr = decode_reg(data, offset)?;
                    Ok(Instruction::GpuFree { ptr })
                }

                GpuSubOpcode::PinMemory => {
                    let ptr = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    Ok(Instruction::GpuPinMemory { ptr, size })
                }

                GpuSubOpcode::UnpinMemory => {
                    let ptr = decode_reg(data, offset)?;
                    Ok(Instruction::GpuUnpinMemory { ptr })
                }

                GpuSubOpcode::Prefetch => {
                    let ptr = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let device = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuPrefetch { ptr, size, device, stream })
                }

                GpuSubOpcode::Memset => {
                    let ptr = decode_reg(data, offset)?;
                    let value = decode_u8(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemset { ptr, value, size })
                }

                GpuSubOpcode::MemsetAsync => {
                    let ptr = decode_reg(data, offset)?;
                    let value = decode_u8(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemsetAsync { ptr, value, size, stream })
                }

                GpuSubOpcode::Memcpy2D => {
                    let dst = decode_reg(data, offset)?;
                    let dst_pitch = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let src_pitch = decode_reg(data, offset)?;
                    let width = decode_reg(data, offset)?;
                    let height = decode_reg(data, offset)?;
                    let direction = decode_u8(data, offset)?;
                    Ok(Instruction::GpuMemcpy2D { dst, dst_pitch, src, src_pitch, width, height, direction })
                }

                GpuSubOpcode::Memcpy2DAsync => {
                    let dst = decode_reg(data, offset)?;
                    let dst_pitch = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let src_pitch = decode_reg(data, offset)?;
                    let width = decode_reg(data, offset)?;
                    let height = decode_reg(data, offset)?;
                    let direction = decode_u8(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemcpy2DAsync { dst, dst_pitch, src, src_pitch, width, height, direction, stream })
                }

                GpuSubOpcode::MemcpyH2D => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemcpyH2D { dst, src, size })
                }

                GpuSubOpcode::MemcpyD2H => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemcpyD2H { dst, src, size })
                }

                GpuSubOpcode::MemcpyD2D => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemcpyD2D { dst, src, size })
                }

                GpuSubOpcode::MemcpyAsyncH2D => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemcpyAsyncH2D { dst, src, size, stream })
                }

                GpuSubOpcode::MemcpyAsyncD2H => {
                    let dst = decode_reg(data, offset)?;
                    let src = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemcpyAsyncD2H { dst, src, size, stream })
                }

                GpuSubOpcode::StreamCreate => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamCreate { dst })
                }

                GpuSubOpcode::StreamDestroy => {
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamDestroy { stream })
                }

                GpuSubOpcode::StreamQuery => {
                    let dst = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamQuery { dst, stream })
                }

                GpuSubOpcode::StreamWaitEvent => {
                    let stream = decode_reg(data, offset)?;
                    let event = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamWaitEvent { stream, event })
                }

                GpuSubOpcode::StreamCreateWithPriority => {
                    let dst = decode_reg(data, offset)?;
                    let priority = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamCreateWithPriority { dst, priority })
                }

                GpuSubOpcode::StreamGetPriority => {
                    let dst = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamGetPriority { dst, stream })
                }

                GpuSubOpcode::StreamCreateNonBlocking => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamCreateNonBlocking { dst })
                }

                GpuSubOpcode::StreamAddCallback => {
                    let stream = decode_reg(data, offset)?;
                    let callback_id = decode_varint(data, offset)? as u32;
                    let user_data = decode_reg(data, offset)?;
                    Ok(Instruction::GpuStreamAddCallback { stream, callback_id, user_data })
                }

                GpuSubOpcode::EventCreate => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEventCreate { dst })
                }

                GpuSubOpcode::EventDestroy => {
                    let event = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEventDestroy { event })
                }

                GpuSubOpcode::EventRecord => {
                    let event = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEventRecord { event, stream })
                }

                GpuSubOpcode::EventSynchronize => {
                    let event = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEventSynchronize { event })
                }

                GpuSubOpcode::EventQuery => {
                    let dst = decode_reg(data, offset)?;
                    let event = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEventQuery { dst, event })
                }

                GpuSubOpcode::EventElapsed => {
                    let dst = decode_reg(data, offset)?;
                    let start_event = decode_reg(data, offset)?;
                    let end_event = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEventElapsed { dst, start_event, end_event })
                }

                GpuSubOpcode::EventCreateWithFlags => {
                    let dst = decode_reg(data, offset)?;
                    let flags = decode_u8(data, offset)?;
                    Ok(Instruction::GpuEventCreateWithFlags { dst, flags })
                }

                GpuSubOpcode::EventRecordWithFlags => {
                    let event = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    let flags = decode_u8(data, offset)?;
                    Ok(Instruction::GpuEventRecordWithFlags { event, stream, flags })
                }

                GpuSubOpcode::GetDevice => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGetDevice { dst })
                }

                GpuSubOpcode::SetDevice => {
                    let device = decode_reg(data, offset)?;
                    Ok(Instruction::GpuSetDevice { device })
                }

                GpuSubOpcode::GetDeviceCount => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGetDeviceCount { dst })
                }

                GpuSubOpcode::GetDeviceProperty => {
                    let dst = decode_reg(data, offset)?;
                    let device = decode_reg(data, offset)?;
                    let property_id = decode_u8(data, offset)?;
                    Ok(Instruction::GpuGetDeviceProperty { dst, device, property_id })
                }

                GpuSubOpcode::GetMemoryInfo => {
                    let free = decode_reg(data, offset)?;
                    let total = decode_reg(data, offset)?;
                    let device = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGetMemoryInfo { free, total, device })
                }

                GpuSubOpcode::CanAccessPeer => {
                    let dst = decode_reg(data, offset)?;
                    let device = decode_reg(data, offset)?;
                    let peer_device = decode_reg(data, offset)?;
                    Ok(Instruction::GpuCanAccessPeer { dst, device, peer_device })
                }

                GpuSubOpcode::EnablePeerAccess => {
                    let device = decode_reg(data, offset)?;
                    let peer_device = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEnablePeerAccess { device, peer_device })
                }

                GpuSubOpcode::DisablePeerAccess => {
                    let device = decode_reg(data, offset)?;
                    let peer_device = decode_reg(data, offset)?;
                    Ok(Instruction::GpuDisablePeerAccess { device, peer_device })
                }

                GpuSubOpcode::DeviceReset => {
                    let device = decode_reg(data, offset)?;
                    Ok(Instruction::GpuDeviceReset { device })
                }

                GpuSubOpcode::SetDeviceFlags => {
                    let flags = decode_u8(data, offset)?;
                    Ok(Instruction::GpuSetDeviceFlags { flags })
                }

                GpuSubOpcode::MallocManaged => {
                    let dst = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let attach_flags = decode_u8(data, offset)?;
                    Ok(Instruction::GpuMallocManaged { dst, size, attach_flags })
                }

                GpuSubOpcode::MemAdvise => {
                    let ptr = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let advice = decode_u8(data, offset)?;
                    let device = decode_reg(data, offset)?;
                    Ok(Instruction::GpuMemAdvise { ptr, size, advice, device })
                }

                GpuSubOpcode::PrefetchAsync => {
                    let ptr = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    let device = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuPrefetchAsync { ptr, size, device, stream })
                }

                GpuSubOpcode::MemGetAttribute => {
                    let dst = decode_reg(data, offset)?;
                    let ptr = decode_reg(data, offset)?;
                    let attribute = decode_u8(data, offset)?;
                    Ok(Instruction::GpuMemGetAttribute { dst, ptr, attribute })
                }

                GpuSubOpcode::GraphCreate => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGraphCreate { dst })
                }

                GpuSubOpcode::GraphBeginCapture => {
                    let stream = decode_reg(data, offset)?;
                    let mode = decode_u8(data, offset)?;
                    Ok(Instruction::GpuGraphBeginCapture { stream, mode })
                }

                GpuSubOpcode::GraphEndCapture => {
                    let dst = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGraphEndCapture { dst, stream })
                }

                GpuSubOpcode::GraphInstantiate => {
                    let dst = decode_reg(data, offset)?;
                    let graph = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGraphInstantiate { dst, graph })
                }

                GpuSubOpcode::GraphLaunch => {
                    let graph_exec = decode_reg(data, offset)?;
                    let stream = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGraphLaunch { graph_exec, stream })
                }

                GpuSubOpcode::GraphDestroy => {
                    let graph = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGraphDestroy { graph })
                }

                GpuSubOpcode::GraphExecDestroy => {
                    let graph_exec = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGraphExecDestroy { graph_exec })
                }

                GpuSubOpcode::GraphExecUpdate => {
                    let graph_exec = decode_reg(data, offset)?;
                    let graph = decode_reg(data, offset)?;
                    Ok(Instruction::GpuGraphExecUpdate { graph_exec, graph })
                }

                GpuSubOpcode::ProfileRangeStart => {
                    let name_id = decode_varint(data, offset)? as u32;
                    Ok(Instruction::GpuProfileRangeStart { name_id })
                }

                GpuSubOpcode::ProfileRangeEnd => {
                    Ok(Instruction::GpuProfileRangeEnd)
                }

                GpuSubOpcode::ProfileMarkerPush => {
                    let name_id = decode_varint(data, offset)? as u32;
                    Ok(Instruction::GpuProfileMarkerPush { name_id })
                }

                GpuSubOpcode::ProfileMarkerPop => {
                    Ok(Instruction::GpuProfileMarkerPop)
                }

                // Device Enumeration
                GpuSubOpcode::EnumerateCuda => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEnumerateDevices { dst, backend: 0 }) // 0 = CUDA
                }

                GpuSubOpcode::EnumerateMetal => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEnumerateDevices { dst, backend: 1 }) // 1 = Metal
                }

                GpuSubOpcode::EnumerateRocm => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEnumerateDevices { dst, backend: 2 }) // 2 = ROCm
                }

                GpuSubOpcode::EnumerateVulkan => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::GpuEnumerateDevices { dst, backend: 3 }) // 3 = Vulkan
                }

                // Thread Intrinsics (0xA0-0xAF) - all have format: dst:reg
                GpuSubOpcode::ThreadIdX
                | GpuSubOpcode::ThreadIdY
                | GpuSubOpcode::ThreadIdZ
                | GpuSubOpcode::BlockIdX
                | GpuSubOpcode::BlockIdY
                | GpuSubOpcode::BlockIdZ
                | GpuSubOpcode::BlockDimX
                | GpuSubOpcode::BlockDimY
                | GpuSubOpcode::BlockDimZ
                | GpuSubOpcode::GridDimX
                | GpuSubOpcode::GridDimY
                | GpuSubOpcode::GridDimZ
                | GpuSubOpcode::WarpSize
                | GpuSubOpcode::LinearThreadId => {
                    let dst = decode_reg(data, offset)?;
                    Ok(Instruction::Raw {
                        opcode: Opcode::GpuExtended,
                        data: vec![sub_op_byte, dst.0 as u8],
                    })
                }

                GpuSubOpcode::SyncThreads => {
                    Ok(Instruction::Raw {
                        opcode: Opcode::GpuExtended,
                        data: vec![sub_op_byte],
                    })
                }

                GpuSubOpcode::SyncWarp => {
                    let mask = decode_reg(data, offset)?;
                    Ok(Instruction::Raw {
                        opcode: Opcode::GpuExtended,
                        data: vec![sub_op_byte, mask.0 as u8],
                    })
                }

                // Shared Memory Operations (0xB0-0xBF)
                GpuSubOpcode::SharedMemAlloc => {
                    let dst = decode_reg(data, offset)?;
                    let size = decode_reg(data, offset)?;
                    Ok(Instruction::Raw {
                        opcode: Opcode::GpuExtended,
                        data: vec![sub_op_byte, dst.0 as u8, size.0 as u8],
                    })
                }

                GpuSubOpcode::SharedMemLoadI64
                | GpuSubOpcode::SharedMemLoadF64
                | GpuSubOpcode::SharedMemLoadU32 => {
                    let dst = decode_reg(data, offset)?;
                    let off = decode_reg(data, offset)?;
                    Ok(Instruction::Raw {
                        opcode: Opcode::GpuExtended,
                        data: vec![sub_op_byte, dst.0 as u8, off.0 as u8],
                    })
                }

                GpuSubOpcode::SharedMemStoreI64
                | GpuSubOpcode::SharedMemStoreF64
                | GpuSubOpcode::SharedMemStoreU32 => {
                    let off = decode_reg(data, offset)?;
                    let val = decode_reg(data, offset)?;
                    Ok(Instruction::Raw {
                        opcode: Opcode::GpuExtended,
                        data: vec![sub_op_byte, off.0 as u8, val.0 as u8],
                    })
                }

                GpuSubOpcode::SharedMemAtomicAddI64
                | GpuSubOpcode::SharedMemAtomicAddF64
                | GpuSubOpcode::SharedMemAtomicMaxI64
                | GpuSubOpcode::SharedMemAtomicMinI64 => {
                    let dst = decode_reg(data, offset)?;
                    let off = decode_reg(data, offset)?;
                    let val = decode_reg(data, offset)?;
                    Ok(Instruction::Raw {
                        opcode: Opcode::GpuExtended,
                        data: vec![sub_op_byte, dst.0 as u8, off.0 as u8, val.0 as u8],
                    })
                }

                GpuSubOpcode::SharedMemAtomicCasI64 => {
                    let dst = decode_reg(data, offset)?;
                    let off = decode_reg(data, offset)?;
                    let expected = decode_reg(data, offset)?;
                    let desired = decode_reg(data, offset)?;
                    Ok(Instruction::Raw {
                        opcode: Opcode::GpuExtended,
                        data: vec![sub_op_byte, dst.0 as u8, off.0 as u8, expected.0 as u8, desired.0 as u8],
                    })
                }
            }
        }

        // ====================================================================
        // Meta Operations
        // ====================================================================
        Opcode::MetaEval
        | Opcode::MetaQuote
        | Opcode::MetaSplice
        | Opcode::MetaReflect => Ok(Instruction::Raw {
            opcode,
            data: vec![],
        }),

        // ====================================================================
        // Memory Extended Operations
        // ====================================================================
        Opcode::MemExtended => {
            let sub_op = decode_u8(data, offset)?;
            Ok(Instruction::MemExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // Arithmetic Extended Operations
        // ====================================================================
        Opcode::ArithExtended => {
            let sub_op = decode_u8(data, offset)?;
            // Decode based on arithmetic sub-opcode
            Ok(Instruction::ArithExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // FFI Extended Operations
        // ====================================================================
        Opcode::FfiExtended => {
            let sub_op = decode_u8(data, offset)?;
            // Decode based on FFI sub-opcode
            Ok(Instruction::FfiExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // Math Extended Operations (0x29)
        // ====================================================================
        Opcode::MathExtended => {
            let sub_op = decode_u8(data, offset)?;
            // Math sub-opcodes use uniform dst, src format
            // Operands decoded inline by interpreter for ~2ns dispatch
            Ok(Instruction::MathExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // SIMD Extended Operations (0x2A)
        // ====================================================================
        Opcode::SimdExtended => {
            let sub_op = decode_u8(data, offset)?;
            // SIMD sub-opcodes for platform-agnostic vector operations
            // Operands decoded inline by interpreter
            Ok(Instruction::SimdExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // Char Extended Operations (0x2B)
        // ====================================================================
        Opcode::CharExtended => {
            let sub_op = decode_u8(data, offset)?;
            // Character classification and conversion operations
            // Operands decoded inline by interpreter
            Ok(Instruction::CharExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // CBGR Extended Operations (0x78)
        // ====================================================================
        Opcode::CbgrExtended => {
            let sub_op = decode_u8(data, offset)?;
            // CBGR (Capability-Based Generational References) operations
            // Operands decoded inline by interpreter for zero-cost dispatch
            Ok(Instruction::CbgrExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // Text Extended Operations (0x79)
        // ====================================================================
        Opcode::TextExtended => {
            let sub_op = decode_u8(data, offset)?;
            // Text parsing and conversion operations
            // Operands decoded inline by interpreter for zero-cost dispatch
            Ok(Instruction::TextExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // Logging Extended Operations (0xBE)
        // ====================================================================
        Opcode::LogExtended => {
            let sub_op = decode_u8(data, offset)?;
            // Structured logging operations
            // Operands decoded inline by interpreter
            Ok(Instruction::LogExtended {
                sub_op,
                operands: vec![], // Operands decoded by interpreter
            })
        }

        // ====================================================================
        // Iterator Operations
        // ====================================================================
        Opcode::IterNew => {
            let dst = decode_reg(data, offset)?;
            let iterable = decode_reg(data, offset)?;
            Ok(Instruction::IterNew { dst, iterable })
        }

        Opcode::IterNext => {
            let dst = decode_reg(data, offset)?;
            let has_next = decode_reg(data, offset)?;
            let iter = decode_reg(data, offset)?;
            Ok(Instruction::IterNext { dst, has_next, iter })
        }

        // ====================================================================
        // Atomic Operations
        // ====================================================================
        Opcode::AtomicLoad => {
            let dst = decode_reg(data, offset)?;
            let ptr = decode_reg(data, offset)?;
            let ordering = decode_u8(data, offset)?;
            let size = decode_u8(data, offset)?;
            Ok(Instruction::AtomicLoad { dst, ptr, ordering, size })
        }

        Opcode::AtomicStore => {
            let ptr = decode_reg(data, offset)?;
            let val = decode_reg(data, offset)?;
            let ordering = decode_u8(data, offset)?;
            let size = decode_u8(data, offset)?;
            Ok(Instruction::AtomicStore { ptr, val, ordering, size })
        }

        Opcode::AtomicCas => {
            let dst = decode_reg(data, offset)?;
            let ptr = decode_reg(data, offset)?;
            let expected = decode_reg(data, offset)?;
            let desired = decode_reg(data, offset)?;
            let ordering = decode_u8(data, offset)?;
            let size = decode_u8(data, offset)?;
            Ok(Instruction::AtomicCas { dst, ptr, expected, desired, ordering, size })
        }

        Opcode::AtomicFence => {
            let ordering = decode_u8(data, offset)?;
            Ok(Instruction::AtomicFence { ordering })
        }

        // ====================================================================
        // Miscellaneous Operations
        // ====================================================================
        Opcode::Nop => Ok(Instruction::Nop),

        Opcode::DebugPrint => {
            let value = decode_reg(data, offset)?;
            Ok(Instruction::DebugPrint { value })
        }

        // ====================================================================
        // Reserved Opcodes
        // ====================================================================
        _ => Ok(Instruction::Raw {
            opcode,
            data: vec![],
        }),
    }
}

// ============================================================================
// Decoding Helpers
// ============================================================================

/// Decodes a vector of registers.
#[inline]
fn decode_reg_vec(data: &[u8], offset: &mut usize) -> VbcResult<Vec<Reg>> {
    let count = decode_varint(data, offset)? as usize;
    let mut regs = Vec::with_capacity(count);
    for _ in 0..count {
        regs.push(decode_reg(data, offset)?);
    }
    Ok(regs)
}

/// Decodes an optional register.
#[inline]
fn decode_optional_reg(data: &[u8], offset: &mut usize) -> VbcResult<Option<Reg>> {
    let flag = decode_u8(data, offset)?;
    if flag == 0 {
        Ok(None)
    } else {
        Ok(Some(decode_reg(data, offset)?))
    }
}

/// Decodes a TypeRef.
///
/// Matches the encoding in encode_type_ref:
/// - 0x00: Concrete(TypeId)
/// - 0x01: Generic(TypeParamId)
/// - 0x02: Instantiated { base, args }
/// - 0x03: Function { params, return_type, contexts }
/// - 0x04: Reference { inner, mutability, tier }
/// - 0x05: Tuple(elems)
/// - 0x06: Array { element, length }
/// - 0x07: Slice(inner)
fn decode_type_ref(data: &[u8], offset: &mut usize) -> VbcResult<TypeRef> {
    use smallvec::SmallVec;

    let discriminant = decode_u8(data, offset)?;

    match discriminant {
        0x00 => {
            // Concrete(TypeId)
            let id = decode_varint(data, offset)? as u32;
            Ok(TypeRef::Concrete(TypeId(id)))
        }
        0x01 => {
            // Generic(TypeParamId)
            let id = decode_varint(data, offset)? as u16;
            Ok(TypeRef::Generic(TypeParamId(id)))
        }
        0x02 => {
            // Instantiated { base, args }
            let base_id = decode_varint(data, offset)? as u32;
            let arg_count = decode_varint(data, offset)? as usize;
            let mut args = Vec::with_capacity(arg_count);
            for _ in 0..arg_count {
                args.push(decode_type_ref(data, offset)?);
            }
            Ok(TypeRef::Instantiated {
                base: TypeId(base_id),
                args,
            })
        }
        0x03 => {
            // Function { params, return_type, contexts }
            let param_count = decode_varint(data, offset)? as usize;
            let mut params = Vec::with_capacity(param_count);
            for _ in 0..param_count {
                params.push(decode_type_ref(data, offset)?);
            }
            let return_type = Box::new(decode_type_ref(data, offset)?);
            let ctx_count = decode_varint(data, offset)? as usize;
            let mut contexts = SmallVec::with_capacity(ctx_count);
            for _ in 0..ctx_count {
                contexts.push(ContextRef(decode_varint(data, offset)? as u32));
            }
            Ok(TypeRef::Function {
                params,
                return_type,
                contexts,
            })
        }
        0x04 => {
            // Reference { inner, mutability, tier }
            let inner = Box::new(decode_type_ref(data, offset)?);
            let mutability_byte = decode_u8(data, offset)?;
            let tier_byte = decode_u8(data, offset)?;
            let mutability = match mutability_byte {
                0 => Mutability::Immutable,
                _ => Mutability::Mutable,
            };
            let tier = match tier_byte {
                0 => CbgrTier::Tier0,
                1 => CbgrTier::Tier1,
                _ => CbgrTier::Tier2,
            };
            Ok(TypeRef::Reference {
                inner,
                mutability,
                tier,
            })
        }
        0x05 => {
            // Tuple(elems)
            let elem_count = decode_varint(data, offset)? as usize;
            let mut elems = Vec::with_capacity(elem_count);
            for _ in 0..elem_count {
                elems.push(decode_type_ref(data, offset)?);
            }
            Ok(TypeRef::Tuple(elems))
        }
        0x06 => {
            // Array { element, length }
            let element = Box::new(decode_type_ref(data, offset)?);
            let length = decode_varint(data, offset)?;
            Ok(TypeRef::Array { element, length })
        }
        0x07 => {
            // Slice(inner)
            let inner = Box::new(decode_type_ref(data, offset)?);
            Ok(TypeRef::Slice(inner))
        }
        _ => Err(VbcError::InvalidTypeRef {
            offset: *offset as u32 - 1,
            discriminant,
        }),
    }
}

/// Decodes a binary integer instruction.
#[inline]
fn decode_binary_i(data: &[u8], offset: &mut usize, op: BinaryIntOp) -> VbcResult<Instruction> {
    let dst = decode_reg(data, offset)?;
    let a = decode_reg(data, offset)?;
    let b = decode_reg(data, offset)?;
    Ok(Instruction::BinaryI { op, dst, a, b })
}

/// Decodes a binary float instruction.
#[inline]
fn decode_binary_f(data: &[u8], offset: &mut usize, op: BinaryFloatOp) -> VbcResult<Instruction> {
    let dst = decode_reg(data, offset)?;
    let a = decode_reg(data, offset)?;
    let b = decode_reg(data, offset)?;
    Ok(Instruction::BinaryF { op, dst, a, b })
}

/// Decodes a binary generic instruction.
#[inline]
fn decode_binary_g(data: &[u8], offset: &mut usize, op: BinaryGenericOp) -> VbcResult<Instruction> {
    let dst = decode_reg(data, offset)?;
    let a = decode_reg(data, offset)?;
    let b = decode_reg(data, offset)?;
    let protocol_id = decode_varint(data, offset)? as u32;
    Ok(Instruction::BinaryG {
        op,
        dst,
        a,
        b,
        protocol_id,
    })
}

/// Decodes a unary integer instruction.
#[inline]
fn decode_unary_i(data: &[u8], offset: &mut usize, op: UnaryIntOp) -> VbcResult<Instruction> {
    let dst = decode_reg(data, offset)?;
    let src = decode_reg(data, offset)?;
    Ok(Instruction::UnaryI { op, dst, src })
}

/// Decodes a bitwise instruction.
#[inline]
fn decode_bitwise(data: &[u8], offset: &mut usize, op: BitwiseOp) -> VbcResult<Instruction> {
    let dst = decode_reg(data, offset)?;
    let a = decode_reg(data, offset)?;
    let b = decode_reg(data, offset)?;
    Ok(Instruction::Bitwise { op, dst, a, b })
}

/// Decodes an integer comparison instruction.
#[inline]
fn decode_cmp_i(data: &[u8], offset: &mut usize, op: CompareOp) -> VbcResult<Instruction> {
    let dst = decode_reg(data, offset)?;
    let a = decode_reg(data, offset)?;
    let b = decode_reg(data, offset)?;
    Ok(Instruction::CmpI { op, dst, a, b })
}

/// Decodes a float comparison instruction.
#[inline]
fn decode_cmp_f(data: &[u8], offset: &mut usize, op: CompareOp) -> VbcResult<Instruction> {
    let dst = decode_reg(data, offset)?;
    let a = decode_reg(data, offset)?;
    let b = decode_reg(data, offset)?;
    Ok(Instruction::CmpF { op, dst, a, b })
}

/// Decodes a fused compare-and-jump instruction.
#[inline]
fn decode_jmp_cmp(data: &[u8], offset: &mut usize, op: CompareOp) -> VbcResult<Instruction> {
    let a = decode_reg(data, offset)?;
    let b = decode_reg(data, offset)?;
    let offset_val = decode_signed_varint(data, offset)? as i32;
    Ok(Instruction::JmpCmp {
        op,
        a,
        b,
        offset: offset_val,
    })
}

// ============================================================================
// Opcode Mapping Helpers
// ============================================================================

/// Maps BinaryIntOp to Opcode.
fn binary_int_op_to_opcode(op: BinaryIntOp) -> Opcode {
    match op {
        BinaryIntOp::Add => Opcode::AddI,
        BinaryIntOp::Sub => Opcode::SubI,
        BinaryIntOp::Mul => Opcode::MulI,
        BinaryIntOp::Div => Opcode::DivI,
        BinaryIntOp::Mod => Opcode::ModI,
        BinaryIntOp::Pow => Opcode::PowI,
    }
}

/// Maps BinaryFloatOp to Opcode.
fn binary_float_op_to_opcode(op: BinaryFloatOp) -> Opcode {
    match op {
        BinaryFloatOp::Add => Opcode::AddF,
        BinaryFloatOp::Sub => Opcode::SubF,
        BinaryFloatOp::Mul => Opcode::MulF,
        BinaryFloatOp::Div => Opcode::DivF,
        BinaryFloatOp::Pow => Opcode::PowF,
        BinaryFloatOp::Mod => Opcode::ModF,
    }
}

/// Maps BinaryGenericOp to Opcode.
fn binary_generic_op_to_opcode(op: BinaryGenericOp) -> Opcode {
    match op {
        BinaryGenericOp::Add => Opcode::AddG,
        BinaryGenericOp::Sub => Opcode::SubG,
        BinaryGenericOp::Mul => Opcode::MulG,
        BinaryGenericOp::Div => Opcode::DivG,
    }
}

/// Maps UnaryIntOp to Opcode.
fn unary_int_op_to_opcode(op: UnaryIntOp) -> Opcode {
    match op {
        UnaryIntOp::Neg => Opcode::NegI,
        UnaryIntOp::Abs => Opcode::AbsI,
        UnaryIntOp::Inc => Opcode::Inc,
        UnaryIntOp::Dec => Opcode::Dec,
    }
}

/// Maps BitwiseOp to Opcode.
fn bitwise_op_to_opcode(op: BitwiseOp) -> Opcode {
    match op {
        BitwiseOp::And => Opcode::Band,
        BitwiseOp::Or => Opcode::Bor,
        BitwiseOp::Xor => Opcode::Bxor,
        BitwiseOp::Not => Opcode::Bnot,
        BitwiseOp::Shl => Opcode::Shl,
        BitwiseOp::Shr => Opcode::Shr,
        BitwiseOp::Ushr => Opcode::Ushr,
    }
}

/// Maps CompareOp for integer comparison to Opcode.
fn cmp_int_op_to_opcode(op: CompareOp) -> Opcode {
    match op {
        CompareOp::Eq => Opcode::EqI,
        CompareOp::Ne => Opcode::NeI,
        CompareOp::Lt => Opcode::LtI,
        CompareOp::Le => Opcode::LeI,
        CompareOp::Gt => Opcode::GtI,
        CompareOp::Ge => Opcode::GeI,
    }
}

/// Maps CompareOp for float comparison to Opcode.
fn cmp_float_op_to_opcode(op: CompareOp) -> Opcode {
    match op {
        CompareOp::Eq => Opcode::EqF,
        CompareOp::Ne => Opcode::NeF,
        CompareOp::Lt => Opcode::LtF,
        CompareOp::Le => Opcode::LeF,
        CompareOp::Gt => Opcode::GtF,
        CompareOp::Ge => Opcode::GeF,
    }
}

/// Maps CompareOp for fused compare-and-jump to Opcode.
fn jmp_cmp_op_to_opcode(op: CompareOp) -> Opcode {
    match op {
        CompareOp::Eq => Opcode::JmpEq,
        CompareOp::Ne => Opcode::JmpNe,
        CompareOp::Lt => Opcode::JmpLt,
        CompareOp::Le => Opcode::JmpLe,
        CompareOp::Gt => Opcode::JmpGt,
        CompareOp::Ge => Opcode::JmpGe,
    }
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Encodes a sequence of instructions.
pub fn encode_instructions(instructions: &[Instruction], output: &mut Vec<u8>) -> usize {
    let start_len = output.len();
    for instr in instructions {
        encode_instruction(instr, output);
    }
    output.len() - start_len
}

/// Decodes a sequence of instructions from bytecode.
pub fn decode_instructions(data: &[u8]) -> VbcResult<Vec<Instruction>> {
    let mut instructions = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        let instr = decode_instruction(data, &mut offset)?;
        instructions.push(instr);
    }

    Ok(instructions)
}

/// Calculates the encoded size of an instruction without encoding it.
pub fn instruction_size(instr: &Instruction) -> usize {
    // Encode to a temporary buffer and return length
    let mut buf = Vec::with_capacity(32);
    encode_instruction(instr, &mut buf)
}

/// Converts instruction-level jump offsets to byte-level offsets.
///
/// The codegen produces instructions with jump offsets in terms of instruction
/// indices, but the interpreter expects byte offsets. This function converts
/// all jump offsets to byte offsets.
///
/// This uses an iterative approach because changing jump offsets can change
/// instruction sizes (varints), which in turn affects other offsets.
pub fn fixup_jump_offsets(instructions: &mut [Instruction]) {
    let trace = std::env::var("VBC_FIXUP_TRACE").is_ok();

    // Store original instruction-level target indices for all jumps
    // This allows us to recalculate byte offsets after sizes change
    let mut jump_targets: Vec<Option<usize>> = vec![None; instructions.len()];

    for (idx, instr) in instructions.iter().enumerate() {
        let target_idx = match instr {
            Instruction::Jmp { offset } => Some((idx as i32 + *offset) as usize),
            Instruction::JmpIf { offset, .. } | Instruction::JmpNot { offset, .. } => {
                Some((idx as i32 + *offset) as usize)
            }
            Instruction::JmpCmp { offset, .. } => Some((idx as i32 + *offset) as usize),
            Instruction::CtxProvide { body_offset, .. } => {
                Some((idx as i32 + *body_offset) as usize)
            }
            Instruction::TryBegin { handler_offset } => {
                Some((idx as i32 + *handler_offset) as usize)
            }
            _ => None,
        };
        jump_targets[idx] = target_idx;
    }

    // Iterate until sizes stabilize (max 10 iterations to prevent infinite loops)
    for iteration in 0..10 {
        // Calculate byte offsets based on current instruction sizes
        let mut byte_offsets = Vec::with_capacity(instructions.len() + 1);
        let mut instr_sizes = Vec::with_capacity(instructions.len());
        let mut current_offset = 0usize;

        for instr in instructions.iter() {
            byte_offsets.push(current_offset);
            let size = instruction_size(instr);
            instr_sizes.push(size);
            current_offset += size;
        }
        byte_offsets.push(current_offset); // End position

        if trace && iteration == 0 {
            eprintln!("FIXUP: {} instructions, initial {} bytes", instructions.len(), current_offset);
        }

        // Track if any offset changed
        let mut changed = false;

        // Update jump offsets to byte offsets
        for (idx, instr) in instructions.iter_mut().enumerate() {
            let instr_end_byte = byte_offsets[idx] + instr_sizes[idx];

            if let Some(target_idx) = jump_targets[idx]
                && target_idx < byte_offsets.len() {
                    let target_byte = byte_offsets[target_idx];
                    let new_offset = target_byte as i32 - instr_end_byte as i32;

                    match instr {
                        Instruction::Jmp { offset }
                            if *offset != new_offset => {
                                if trace {
                                    eprintln!("FIXUP[{}] Jmp[{}]: target_idx={} -> byte_offset={} (was {}, instr_end={})",
                                        iteration, idx, target_idx, new_offset, *offset, instr_end_byte);
                                }
                                *offset = new_offset;
                                changed = true;
                            }
                        Instruction::JmpIf { offset, .. } | Instruction::JmpNot { offset, .. }
                            if *offset != new_offset => {
                                if trace {
                                    eprintln!("FIXUP[{}] JmpIf/Not[{}]: target_idx={} -> byte_offset={} (was {}, instr_end={})",
                                        iteration, idx, target_idx, new_offset, *offset, instr_end_byte);
                                }
                                *offset = new_offset;
                                changed = true;
                            }
                        Instruction::JmpCmp { offset, .. }
                            if *offset != new_offset => {
                                *offset = new_offset;
                                changed = true;
                            }
                        Instruction::CtxProvide { body_offset, .. }
                            if *body_offset != new_offset => {
                                *body_offset = new_offset;
                                changed = true;
                            }
                        Instruction::TryBegin { handler_offset }
                            if *handler_offset != new_offset => {
                                if trace {
                                    eprintln!("FIXUP[{}] TryBegin[{}]: target_idx={} -> byte_offset={} (was {}, instr_end={})",
                                        iteration, idx, target_idx, new_offset, *handler_offset, instr_end_byte);
                                }
                                *handler_offset = new_offset;
                                changed = true;
                            }
                        _ => {}
                    }
                }
        }

        // If no offsets changed, we've converged
        if !changed {
            if trace {
                eprintln!("FIXUP: converged after {} iteration(s), final {} bytes",
                    iteration + 1, current_offset);
            }
            break;
        }
    }
}

/// Encodes instructions with proper jump offset fixup.
///
/// This is the preferred way to encode instructions as it handles
/// the conversion from instruction-level to byte-level jump offsets.
pub fn encode_instructions_with_fixup(instructions: &[Instruction], output: &mut Vec<u8>) -> usize {
    let mut fixed = instructions.to_vec();
    fixup_jump_offsets(&mut fixed);
    encode_instructions(&fixed, output)
}

// ============================================================================
// TryFrom implementations for enum conversions
// ============================================================================

impl TryFrom<u8> for UnaryFloatOp {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(UnaryFloatOp::Neg),
            1 => Ok(UnaryFloatOp::Abs),
            2 => Ok(UnaryFloatOp::Sqrt),
            3 => Ok(UnaryFloatOp::Exp),
            4 => Ok(UnaryFloatOp::Log),
            5 => Ok(UnaryFloatOp::Sin),
            6 => Ok(UnaryFloatOp::Cos),
            7 => Ok(UnaryFloatOp::Tan),
            8 => Ok(UnaryFloatOp::Floor),
            9 => Ok(UnaryFloatOp::Ceil),
            10 => Ok(UnaryFloatOp::Round),
            _ => Err(()),
        }
    }
}

impl TryFrom<u8> for TensorDType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(TensorDType::F64),
            0x01 => Ok(TensorDType::F32),
            0x02 => Ok(TensorDType::F16),
            0x03 => Ok(TensorDType::BF16),
            0x04 => Ok(TensorDType::I64),
            0x05 => Ok(TensorDType::I32),
            0x06 => Ok(TensorDType::I16),
            0x07 => Ok(TensorDType::I8),
            0x08 => Ok(TensorDType::U64),
            0x09 => Ok(TensorDType::U32),
            0x0A => Ok(TensorDType::U16),
            0x0B => Ok(TensorDType::U8),
            0x0C => Ok(TensorDType::Bool),
            0x0D => Ok(TensorDType::Complex64),
            0x0E => Ok(TensorDType::Complex128),
            _ => Err(()),
        }
    }
}

impl TryFrom<u8> for TensorBinaryOp {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(TensorBinaryOp::Add),
            0x01 => Ok(TensorBinaryOp::Sub),
            0x02 => Ok(TensorBinaryOp::Mul),
            0x03 => Ok(TensorBinaryOp::Div),
            0x04 => Ok(TensorBinaryOp::Pow),
            0x05 => Ok(TensorBinaryOp::Mod),
            0x06 => Ok(TensorBinaryOp::Min),
            0x07 => Ok(TensorBinaryOp::Max),
            _ => Err(()),
        }
    }
}

impl TryFrom<u8> for TensorUnaryOp {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(TensorUnaryOp::Neg),
            0x01 => Ok(TensorUnaryOp::Abs),
            0x02 => Ok(TensorUnaryOp::Sqrt),
            0x03 => Ok(TensorUnaryOp::Exp),
            0x04 => Ok(TensorUnaryOp::Log),
            0x05 => Ok(TensorUnaryOp::Sin),
            0x06 => Ok(TensorUnaryOp::Cos),
            0x07 => Ok(TensorUnaryOp::Tan),
            0x08 => Ok(TensorUnaryOp::Tanh),
            0x09 => Ok(TensorUnaryOp::Sigmoid),
            0x0A => Ok(TensorUnaryOp::Relu),
            0x0B => Ok(TensorUnaryOp::Gelu),
            0x0C => Ok(TensorUnaryOp::Silu),
            0x0D => Ok(TensorUnaryOp::Floor),
            0x0E => Ok(TensorUnaryOp::Ceil),
            0x0F => Ok(TensorUnaryOp::Round),
            0x10 => Ok(TensorUnaryOp::Sign),
            0x11 => Ok(TensorUnaryOp::Rsqrt),
            0x12 => Ok(TensorUnaryOp::Erf),
            0x13 => Ok(TensorUnaryOp::Log2),
            0x14 => Ok(TensorUnaryOp::Softplus),
            0x15 => Ok(TensorUnaryOp::Mish),
            _ => Err(()),
        }
    }
}

impl TryFrom<u8> for TensorReduceOp {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(TensorReduceOp::Sum),
            0x01 => Ok(TensorReduceOp::Prod),
            0x02 => Ok(TensorReduceOp::Max),
            0x03 => Ok(TensorReduceOp::Min),
            0x04 => Ok(TensorReduceOp::Mean),
            0x05 => Ok(TensorReduceOp::Var),
            0x06 => Ok(TensorReduceOp::Std),
            0x07 => Ok(TensorReduceOp::Norm),
            0x08 => Ok(TensorReduceOp::LogSumExp),
            0x09 => Ok(TensorReduceOp::All),
            0x0A => Ok(TensorReduceOp::Any),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Roundtrip Tests
    // ========================================================================

    /// Test helper for roundtrip verification.
    fn test_roundtrip(instr: &Instruction) {
        let mut encoded = Vec::new();
        encode_instruction(instr, &mut encoded);

        let mut offset = 0;
        let decoded = decode_instruction(&encoded, &mut offset).expect("Failed to decode");

        assert_eq!(
            offset,
            encoded.len(),
            "Didn't consume all bytes for {:?}",
            instr
        );
        assert_eq!(
            &decoded, instr,
            "Roundtrip failed for {:?}\nEncoded: {:?}\nDecoded: {:?}",
            instr, encoded, decoded
        );
    }

    #[test]
    fn test_mov_roundtrip() {
        test_roundtrip(&Instruction::Mov {
            dst: Reg(0),
            src: Reg(1),
        });
        test_roundtrip(&Instruction::Mov {
            dst: Reg(127),
            src: Reg(128),
        });
        test_roundtrip(&Instruction::Mov {
            dst: Reg(16383),
            src: Reg(0),
        });
    }

    #[test]
    fn test_load_k_roundtrip() {
        test_roundtrip(&Instruction::LoadK {
            dst: Reg(0),
            const_id: 0,
        });
        test_roundtrip(&Instruction::LoadK {
            dst: Reg(5),
            const_id: 12345,
        });
        test_roundtrip(&Instruction::LoadK {
            dst: Reg(100),
            const_id: u32::MAX,
        });
    }

    #[test]
    fn test_load_i_roundtrip() {
        test_roundtrip(&Instruction::LoadI {
            dst: Reg(0),
            value: 0,
        });
        test_roundtrip(&Instruction::LoadI {
            dst: Reg(0),
            value: 42,
        });
        test_roundtrip(&Instruction::LoadI {
            dst: Reg(0),
            value: -42,
        });
        test_roundtrip(&Instruction::LoadI {
            dst: Reg(0),
            value: i64::MAX,
        });
        test_roundtrip(&Instruction::LoadI {
            dst: Reg(0),
            value: i64::MIN,
        });
    }

    #[test]
    fn test_load_f_roundtrip() {
        test_roundtrip(&Instruction::LoadF {
            dst: Reg(0),
            value: 0.0,
        });
        test_roundtrip(&Instruction::LoadF {
            dst: Reg(0),
            value: 3.14159265358979,
        });
        test_roundtrip(&Instruction::LoadF {
            dst: Reg(0),
            value: -1e100,
        });
        test_roundtrip(&Instruction::LoadF {
            dst: Reg(0),
            value: f64::INFINITY,
        });
    }

    #[test]
    fn test_load_bool_roundtrip() {
        test_roundtrip(&Instruction::LoadTrue { dst: Reg(0) });
        test_roundtrip(&Instruction::LoadFalse { dst: Reg(5) });
    }

    #[test]
    fn test_load_unit_roundtrip() {
        test_roundtrip(&Instruction::LoadUnit { dst: Reg(10) });
    }

    #[test]
    fn test_load_small_i_roundtrip() {
        test_roundtrip(&Instruction::LoadSmallI {
            dst: Reg(0),
            value: 0,
        });
        test_roundtrip(&Instruction::LoadSmallI {
            dst: Reg(0),
            value: 63,
        });
        test_roundtrip(&Instruction::LoadSmallI {
            dst: Reg(0),
            value: -64,
        });
    }

    #[test]
    fn test_binary_i_roundtrip() {
        for op in [
            BinaryIntOp::Add,
            BinaryIntOp::Sub,
            BinaryIntOp::Mul,
            BinaryIntOp::Div,
            BinaryIntOp::Mod,
            BinaryIntOp::Pow,
        ] {
            test_roundtrip(&Instruction::BinaryI {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_binary_f_roundtrip() {
        for op in [
            BinaryFloatOp::Add,
            BinaryFloatOp::Sub,
            BinaryFloatOp::Mul,
            BinaryFloatOp::Div,
            BinaryFloatOp::Pow,
            BinaryFloatOp::Mod,
        ] {
            test_roundtrip(&Instruction::BinaryF {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_binary_g_roundtrip() {
        // Test all generic binary operations with various protocol IDs
        for op in [
            BinaryGenericOp::Add,
            BinaryGenericOp::Sub,
            BinaryGenericOp::Mul,
            BinaryGenericOp::Div,
        ] {
            // Basic test with protocol ID 0
            test_roundtrip(&Instruction::BinaryG {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
                protocol_id: 0,
            });
            // Test with non-zero protocol ID
            test_roundtrip(&Instruction::BinaryG {
                op,
                dst: Reg(5),
                a: Reg(10),
                b: Reg(15),
                protocol_id: 42,
            });
            // Test with large register numbers
            test_roundtrip(&Instruction::BinaryG {
                op,
                dst: Reg(200),
                a: Reg(300),
                b: Reg(400),
                protocol_id: 1000,
            });
        }
    }

    #[test]
    fn test_binary_g_edge_cases() {
        // Test with maximum register values
        for op in [
            BinaryGenericOp::Add,
            BinaryGenericOp::Sub,
            BinaryGenericOp::Mul,
            BinaryGenericOp::Div,
        ] {
            test_roundtrip(&Instruction::BinaryG {
                op,
                dst: Reg(Reg::MAX),
                a: Reg(Reg::MAX - 1),
                b: Reg(Reg::MAX - 2),
                protocol_id: u32::MAX,
            });
        }
    }

    #[test]
    fn test_cmp_g_roundtrip() {
        // Test EqG (eq = true)
        test_roundtrip(&Instruction::CmpG {
            eq: true,
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
            protocol_id: 0,
        });
        test_roundtrip(&Instruction::CmpG {
            eq: true,
            dst: Reg(10),
            a: Reg(20),
            b: Reg(30),
            protocol_id: 100,
        });

        // Test CmpG (eq = false, for Ord protocol)
        test_roundtrip(&Instruction::CmpG {
            eq: false,
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
            protocol_id: 0,
        });
        test_roundtrip(&Instruction::CmpG {
            eq: false,
            dst: Reg(5),
            a: Reg(6),
            b: Reg(7),
            protocol_id: 50,
        });
    }

    #[test]
    fn test_cmp_g_edge_cases() {
        // Test with maximum values
        test_roundtrip(&Instruction::CmpG {
            eq: true,
            dst: Reg(Reg::MAX),
            a: Reg(Reg::MAX - 1),
            b: Reg(Reg::MAX - 2),
            protocol_id: u32::MAX,
        });
        test_roundtrip(&Instruction::CmpG {
            eq: false,
            dst: Reg(Reg::MAX),
            a: Reg(Reg::MAX - 1),
            b: Reg(Reg::MAX - 2),
            protocol_id: u32::MAX,
        });
    }

    #[test]
    fn test_unary_i_roundtrip() {
        for op in [
            UnaryIntOp::Neg,
            UnaryIntOp::Abs,
            UnaryIntOp::Inc,
            UnaryIntOp::Dec,
        ] {
            test_roundtrip(&Instruction::UnaryI {
                op,
                dst: Reg(0),
                src: Reg(1),
            });
        }
    }

    #[test]
    fn test_not_roundtrip() {
        test_roundtrip(&Instruction::Not {
            dst: Reg(0),
            src: Reg(1),
        });
    }

    #[test]
    fn test_bitwise_roundtrip() {
        for op in [
            BitwiseOp::And,
            BitwiseOp::Or,
            BitwiseOp::Xor,
            BitwiseOp::Not,
            BitwiseOp::Shl,
            BitwiseOp::Shr,
            BitwiseOp::Ushr,
        ] {
            test_roundtrip(&Instruction::Bitwise {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_cmp_i_roundtrip() {
        for op in [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            test_roundtrip(&Instruction::CmpI {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_cmp_f_roundtrip() {
        for op in [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            test_roundtrip(&Instruction::CmpF {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_jmp_roundtrip() {
        test_roundtrip(&Instruction::Jmp { offset: 0 });
        test_roundtrip(&Instruction::Jmp { offset: 100 });
        test_roundtrip(&Instruction::Jmp { offset: -100 });
        test_roundtrip(&Instruction::Jmp { offset: i32::MAX });
        test_roundtrip(&Instruction::Jmp { offset: i32::MIN });
    }

    #[test]
    fn test_jmp_if_roundtrip() {
        test_roundtrip(&Instruction::JmpIf {
            cond: Reg(0),
            offset: 10,
        });
        test_roundtrip(&Instruction::JmpNot {
            cond: Reg(5),
            offset: -20,
        });
    }

    #[test]
    fn test_jmp_cmp_roundtrip() {
        for op in [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            test_roundtrip(&Instruction::JmpCmp {
                op,
                a: Reg(1),
                b: Reg(2),
                offset: 50,
            });
        }
    }

    #[test]
    fn test_ret_roundtrip() {
        test_roundtrip(&Instruction::Ret { value: Reg(0) });
        test_roundtrip(&Instruction::RetV);
    }

    #[test]
    fn test_call_roundtrip() {
        test_roundtrip(&Instruction::Call {
            dst: Reg(0),
            func_id: 0,
            args: RegRange::new(Reg(1), 0),
        });
        test_roundtrip(&Instruction::Call {
            dst: Reg(10),
            func_id: 12345,
            args: RegRange::new(Reg(0), 5),
        });
    }

    #[test]
    fn test_tail_call_roundtrip() {
        test_roundtrip(&Instruction::TailCall {
            func_id: 42,
            args: RegRange::new(Reg(0), 3),
        });
    }

    #[test]
    fn test_new_roundtrip() {
        test_roundtrip(&Instruction::New {
            dst: Reg(0),
            type_id: 0,
            field_count: 2,
        });
        test_roundtrip(&Instruction::New {
            dst: Reg(5),
            type_id: 9999,
            field_count: 4,
        });
    }

    #[test]
    fn test_new_g_roundtrip() {
        test_roundtrip(&Instruction::NewG {
            dst: Reg(0),
            type_id: 42,
            type_args: vec![],
        });
        test_roundtrip(&Instruction::NewG {
            dst: Reg(0),
            type_id: 42,
            type_args: vec![Reg(1), Reg(2), Reg(3)],
        });
    }

    #[test]
    fn test_get_set_f_roundtrip() {
        test_roundtrip(&Instruction::GetF {
            dst: Reg(0),
            obj: Reg(1),
            field_idx: 0,
        });
        test_roundtrip(&Instruction::SetF {
            obj: Reg(1),
            field_idx: 5,
            value: Reg(2),
        });
    }

    #[test]
    fn test_get_set_e_roundtrip() {
        test_roundtrip(&Instruction::GetE {
            dst: Reg(0),
            arr: Reg(1),
            idx: Reg(2),
        });
        test_roundtrip(&Instruction::SetE {
            arr: Reg(1),
            idx: Reg(2),
            value: Reg(3),
        });
    }

    #[test]
    fn test_len_roundtrip() {
        test_roundtrip(&Instruction::Len {
            dst: Reg(0),
            arr: Reg(1),
            type_hint: 0,
        });
    }

    #[test]
    fn test_ref_deref_roundtrip() {
        test_roundtrip(&Instruction::Ref {
            dst: Reg(0),
            src: Reg(1),
        });
        test_roundtrip(&Instruction::RefMut {
            dst: Reg(0),
            src: Reg(1),
        });
        test_roundtrip(&Instruction::Deref {
            dst: Reg(0),
            ref_reg: Reg(1),
        });
        test_roundtrip(&Instruction::ChkRef { ref_reg: Reg(0) });
        test_roundtrip(&Instruction::RefChecked {
            dst: Reg(0),
            src: Reg(1),
        });
        test_roundtrip(&Instruction::RefUnsafe {
            dst: Reg(0),
            src: Reg(1),
        });
    }

    #[test]
    fn test_call_g_roundtrip() {
        test_roundtrip(&Instruction::CallG {
            dst: Reg(0),
            func_id: 100,
            type_args: vec![Reg(1), Reg(2)],
            args: RegRange::new(Reg(3), 2),
        });
    }

    #[test]
    fn test_call_v_roundtrip() {
        test_roundtrip(&Instruction::CallV {
            dst: Reg(0),
            receiver: Reg(1),
            vtable_slot: 5,
            args: RegRange::new(Reg(2), 3),
        });
    }

    #[test]
    fn test_is_as_var_roundtrip() {
        test_roundtrip(&Instruction::IsVar {
            dst: Reg(0),
            value: Reg(1),
            tag: 0,
        });
        test_roundtrip(&Instruction::AsVar {
            dst: Reg(0),
            value: Reg(1),
            tag: 5,
        });
    }

    #[test]
    fn test_pack_unpack_roundtrip() {
        test_roundtrip(&Instruction::Pack {
            dst: Reg(0),
            src_start: Reg(1),
            count: 3,
        });
        test_roundtrip(&Instruction::Unpack {
            dst_start: Reg(0),
            tuple: Reg(5),
            count: 4,
        });
    }

    #[test]
    fn test_switch_roundtrip() {
        test_roundtrip(&Instruction::Switch {
            value: Reg(0),
            default_offset: 100,
            cases: vec![],
        });
        test_roundtrip(&Instruction::Switch {
            value: Reg(0),
            default_offset: -50,
            cases: vec![(0, 10), (1, 20), (5, 30)],
        });
    }

    #[test]
    fn test_spawn_await_roundtrip() {
        test_roundtrip(&Instruction::Spawn {
            dst: Reg(0),
            func_id: 42,
            args: RegRange::new(Reg(1), 2),
        });
        test_roundtrip(&Instruction::Await {
            dst: Reg(0),
            task: Reg(1),
        });
        test_roundtrip(&Instruction::Yield { value: Reg(0) });
    }

    #[test]
    fn test_select_roundtrip() {
        test_roundtrip(&Instruction::Select {
            dst: Reg(0),
            futures: vec![Reg(1), Reg(2), Reg(3)],
            handlers: vec![10, 20, 30],
        });
    }

    #[test]
    fn test_grad_roundtrip() {
        test_roundtrip(&Instruction::GradBegin {
            scope_id: 1,
            mode: GradMode::Reverse,
            wrt: vec![Reg(0), Reg(1)],
        });
        test_roundtrip(&Instruction::GradEnd {
            scope_id: 1,
            output: Reg(0),
            grad_out: Reg(1),
            grad_regs: vec![Reg(2), Reg(3)],
        });
        test_roundtrip(&Instruction::GradCheckpoint {
            id: 5,
            tensors: vec![Reg(0), Reg(1)],
        });
        test_roundtrip(&Instruction::GradAccumulate {
            dst: Reg(0),
            src: Reg(1),
        });
        test_roundtrip(&Instruction::GradStop {
            dst: Reg(0),
            src: Reg(1),
        });
    }

    #[test]
    fn test_ctx_roundtrip() {
        test_roundtrip(&Instruction::CtxGet {
            dst: Reg(0),
            ctx_type: 42,
        });
        test_roundtrip(&Instruction::CtxProvide {
            ctx_type: 42,
            value: Reg(1),
            body_offset: 100,
        });
    }

    #[test]
    fn test_debug_roundtrip() {
        test_roundtrip(&Instruction::Spec {
            reg: Reg(0),
            expected_type: 5,
        });
        test_roundtrip(&Instruction::Guard {
            reg: Reg(0),
            expected_type: 5,
            deopt_offset: 50,
        });
        test_roundtrip(&Instruction::Assert {
            cond: Reg(0),
            message_id: 10,
        });
        test_roundtrip(&Instruction::Panic { message_id: 20 });
        test_roundtrip(&Instruction::Unreachable);
    }

    #[test]
    fn test_tensor_new_roundtrip() {
        test_roundtrip(&Instruction::TensorNew {
            dst: Reg(0),
            dtype: TensorDType::F64,
            dims: vec![],
        });
        test_roundtrip(&Instruction::TensorNew {
            dst: Reg(0),
            dtype: TensorDType::F32,
            dims: vec![Reg(1), Reg(2)],
        });
    }

    #[test]
    fn test_tensor_binop_roundtrip() {
        for op in [
            TensorBinaryOp::Add,
            TensorBinaryOp::Sub,
            TensorBinaryOp::Mul,
            TensorBinaryOp::Div,
        ] {
            test_roundtrip(&Instruction::TensorBinop {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_tensor_unop_roundtrip() {
        test_roundtrip(&Instruction::TensorUnop {
            op: TensorUnaryOp::Neg,
            dst: Reg(0),
            src: Reg(1),
        });
        test_roundtrip(&Instruction::TensorUnop {
            op: TensorUnaryOp::Relu,
            dst: Reg(0),
            src: Reg(1),
        });
    }

    #[test]
    fn test_tensor_matmul_roundtrip() {
        test_roundtrip(&Instruction::TensorMatmul {
            dst: Reg(0),
            a: Reg(1),
            b: Reg(2),
        });
    }

    #[test]
    fn test_tensor_reduce_roundtrip() {
        test_roundtrip(&Instruction::TensorReduce {
            op: TensorReduceOp::Sum,
            dst: Reg(0),
            src: Reg(1),
            axes: vec![0, 1],
            keepdim: false,
        });
        test_roundtrip(&Instruction::TensorReduce {
            op: TensorReduceOp::Mean,
            dst: Reg(0),
            src: Reg(1),
            axes: vec![],
            keepdim: true,
        });
    }

    #[test]
    fn test_tensor_flash_attention_roundtrip() {
        test_roundtrip(&Instruction::TensorFlashAttention {
            dst: Reg(0),
            q: Reg(1),
            k: Reg(2),
            v: Reg(3),
            mask: None,
            scale: Reg(4),
            causal: false,
        });
        test_roundtrip(&Instruction::TensorFlashAttention {
            dst: Reg(0),
            q: Reg(1),
            k: Reg(2),
            v: Reg(3),
            mask: Some(Reg(5)),
            scale: Reg(4),
            causal: true,
        });
    }

    #[test]
    fn test_gpu_launch_roundtrip() {
        test_roundtrip(&Instruction::GpuLaunch {
            kernel_id: 42,
            grid: [Reg(0), Reg(1), Reg(2)],
            block: [Reg(3), Reg(4), Reg(5)],
            shared_mem: Reg(6),
            stream: Reg(7),
            args: vec![Reg(8), Reg(9)],
        });
    }

    #[test]
    fn test_gpu_sync_roundtrip() {
        test_roundtrip(&Instruction::GpuSync { stream: Reg(0) });
    }

    // ========================================================================
    // Multi-instruction Sequence Tests
    // ========================================================================

    #[test]
    fn test_encode_decode_sequence() {
        let instructions = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 10,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 20,
            },
            Instruction::BinaryI {
                op: BinaryIntOp::Add,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Ret { value: Reg(2) },
        ];

        let mut encoded = Vec::new();
        encode_instructions(&instructions, &mut encoded);

        let decoded = decode_instructions(&encoded).expect("Failed to decode sequence");
        assert_eq!(decoded.len(), instructions.len());

        for (original, decoded) in instructions.iter().zip(decoded.iter()) {
            assert_eq!(original, decoded);
        }
    }

    #[test]
    fn test_fibonacci_sequence() {
        // A simplified Fibonacci-like computation bytecode
        let instructions = vec![
            // r0 = n (input)
            // r1 = 0 (fib_prev)
            Instruction::LoadI {
                dst: Reg(1),
                value: 0,
            },
            // r2 = 1 (fib_curr)
            Instruction::LoadI {
                dst: Reg(2),
                value: 1,
            },
            // r3 = 0 (counter)
            Instruction::LoadI {
                dst: Reg(3),
                value: 0,
            },
            // loop: if r3 >= r0 goto end
            Instruction::JmpCmp {
                op: CompareOp::Ge,
                a: Reg(3),
                b: Reg(0),
                offset: 6, // Skip to return
            },
            // r4 = r1 + r2
            Instruction::BinaryI {
                op: BinaryIntOp::Add,
                dst: Reg(4),
                a: Reg(1),
                b: Reg(2),
            },
            // r1 = r2
            Instruction::Mov {
                dst: Reg(1),
                src: Reg(2),
            },
            // r2 = r4
            Instruction::Mov {
                dst: Reg(2),
                src: Reg(4),
            },
            // r3 = r3 + 1
            Instruction::UnaryI {
                op: UnaryIntOp::Inc,
                dst: Reg(3),
                src: Reg(3),
            },
            // goto loop
            Instruction::Jmp { offset: -5 },
            // return r2
            Instruction::Ret { value: Reg(2) },
        ];

        let mut encoded = Vec::new();
        encode_instructions(&instructions, &mut encoded);

        let decoded = decode_instructions(&encoded).expect("Failed to decode Fibonacci");
        assert_eq!(decoded.len(), instructions.len());

        for (i, (original, dec)) in instructions.iter().zip(decoded.iter()).enumerate() {
            assert_eq!(original, dec, "Mismatch at instruction {}", i);
        }
    }

    // ========================================================================
    // TypeRef Encoding Tests
    // ========================================================================

    #[test]
    fn test_type_ref_concrete_roundtrip() {
        let type_ref = TypeRef::Concrete(TypeId(5));
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_type_ref_generic_param_roundtrip() {
        let type_ref = TypeRef::Generic(TypeParamId(42));
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_type_ref_instantiated_roundtrip() {
        let type_ref = TypeRef::Instantiated {
            base: TypeId(10),
            args: vec![TypeRef::Concrete(TypeId(1)), TypeRef::Concrete(TypeId(2))],
        };
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_type_ref_reference_roundtrip() {
        let type_ref = TypeRef::Reference {
            inner: Box::new(TypeRef::Concrete(TypeId(5))),
            mutability: Mutability::Mutable,
            tier: CbgrTier::Tier1,
        };
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_type_ref_function_roundtrip() {
        use smallvec::smallvec;
        let type_ref = TypeRef::Function {
            params: vec![TypeRef::Concrete(TypeId(1)), TypeRef::Concrete(TypeId(2))],
            return_type: Box::new(TypeRef::Concrete(TypeId(3))),
            contexts: smallvec![ContextRef(1), ContextRef(2)],
        };
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_type_ref_tuple_roundtrip() {
        let type_ref = TypeRef::Tuple(vec![
            TypeRef::Concrete(TypeId(1)),
            TypeRef::Concrete(TypeId(2)),
            TypeRef::Generic(TypeParamId(0)),
        ]);
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_type_ref_array_roundtrip() {
        let type_ref = TypeRef::Array {
            element: Box::new(TypeRef::Concrete(TypeId(1))),
            length: 10,
        };
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_type_ref_slice_roundtrip() {
        let type_ref = TypeRef::Slice(Box::new(TypeRef::Concrete(TypeId(5))));
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    // ========================================================================
    // Size Calculation Tests
    // ========================================================================

    #[test]
    fn test_instruction_size() {
        // Simple instructions should be compact
        let mov = Instruction::Mov {
            dst: Reg(0),
            src: Reg(1),
        };
        assert_eq!(instruction_size(&mov), 3); // opcode + 2 regs

        let load_true = Instruction::LoadTrue { dst: Reg(0) };
        assert_eq!(instruction_size(&load_true), 2); // opcode + reg

        let ret_v = Instruction::RetV;
        assert_eq!(instruction_size(&ret_v), 1); // just opcode
    }

    #[test]
    fn test_large_register_encoding() {
        // Large register numbers should still work
        let mov = Instruction::Mov {
            dst: Reg(16383),
            src: Reg(16383),
        };
        let size = instruction_size(&mov);
        assert_eq!(size, 5); // opcode + 2 * 2-byte regs

        test_roundtrip(&mov);
    }

    // ========================================================================
    // Edge Cases
    // ========================================================================

    #[test]
    fn test_empty_vec_instructions() {
        test_roundtrip(&Instruction::NewG {
            dst: Reg(0),
            type_id: 1,
            type_args: vec![],
        });

        test_roundtrip(&Instruction::Select {
            dst: Reg(0),
            futures: vec![],
            handlers: vec![],
        });
    }

    #[test]
    fn test_max_varint_values() {
        test_roundtrip(&Instruction::LoadK {
            dst: Reg(0),
            const_id: u32::MAX,
        });

        test_roundtrip(&Instruction::Jmp {
            offset: i32::MAX,
        });

        test_roundtrip(&Instruction::Jmp {
            offset: i32::MIN,
        });
    }

    // ========================================================================
    // Comprehensive Edge Case Tests
    // ========================================================================

    #[test]
    fn test_all_register_boundaries() {
        // Test register encoding boundaries
        let boundaries = [0, 1, 126, 127, 128, 129, 255, 256, 16382, 16383];
        for &r in &boundaries {
            let instr = Instruction::Mov {
                dst: Reg(r),
                src: Reg(0),
            };
            test_roundtrip(&instr);
        }
    }

    #[test]
    fn test_all_small_immediates() {
        // Test all valid small integer values (-64..63)
        for v in -64i8..64i8 {
            let instr = Instruction::LoadSmallI {
                dst: Reg(0),
                value: v,
            };
            test_roundtrip(&instr);
        }
    }

    #[test]
    fn test_varint_boundary_values() {
        // VarInt boundaries: 1-byte, 2-byte, 3-byte, etc.
        let boundaries: &[i64] = &[
            0,
            1,
            63,
            64,
            127,
            128,
            255,
            256,
            16383,
            16384,
            2097151,
            2097152,
            i64::MAX / 2,
            i64::MAX,
            -1,
            -64,
            -65,
            -128,
            -129,
            -16384,
            -16385,
            i64::MIN / 2,
            i64::MIN,
        ];
        for &v in boundaries {
            let instr = Instruction::LoadI {
                dst: Reg(0),
                value: v,
            };
            test_roundtrip(&instr);
        }
    }

    #[test]
    fn test_special_float_values() {
        let special_values = [
            0.0,
            -0.0,
            f64::MIN,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            1.0e-308,  // Very small denormalized
            1.0e308,   // Very large
            std::f64::consts::PI,
            std::f64::consts::E,
        ];
        for &v in &special_values {
            let instr = Instruction::LoadF {
                dst: Reg(0),
                value: v,
            };
            let mut encoded = Vec::new();
            encode_instruction(&instr, &mut encoded);
            let mut offset = 0;
            let decoded = decode_instruction(&encoded, &mut offset).unwrap();
            // For NaN, check that it's still NaN (NaN != NaN)
            if let (
                Instruction::LoadF { value: v1, .. },
                Instruction::LoadF { value: v2, .. },
            ) = (&instr, &decoded)
            {
                if v1.is_nan() {
                    assert!(v2.is_nan(), "NaN should decode to NaN");
                } else {
                    assert_eq!(v1.to_bits(), v2.to_bits(), "Float bits should match");
                }
            }
        }
    }

    #[test]
    fn test_deeply_nested_type_refs() {
        // Test deeply nested TypeRef structures
        let mut type_ref = TypeRef::Concrete(TypeId(0));
        for i in 0..10 {
            type_ref = TypeRef::Slice(Box::new(type_ref.clone()));
            type_ref = TypeRef::Array {
                element: Box::new(type_ref),
                length: i as u64,
            };
        }
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_complex_function_type() {
        use smallvec::smallvec;
        // Complex function type with many parameters
        let type_ref = TypeRef::Function {
            params: (0..20)
                .map(|i| TypeRef::Concrete(TypeId(i)))
                .collect(),
            return_type: Box::new(TypeRef::Tuple(
                (0..5).map(|i| TypeRef::Concrete(TypeId(i))).collect(),
            )),
            contexts: smallvec![ContextRef(1), ContextRef(2), ContextRef(3)],
        };
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_large_switch_table() {
        // Large switch statement with many cases
        let cases: Vec<(u32, i32)> = (0..256).map(|i| (i, (i as i32) * 10)).collect();
        let instr = Instruction::Switch {
            value: Reg(0),
            default_offset: -1000,
            cases,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_many_type_args() {
        // Generic with many type arguments
        let instr = Instruction::NewG {
            dst: Reg(0),
            type_id: 100,
            type_args: (0..50).map(Reg).collect(),
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_long_instruction_sequence() {
        // Test encoding/decoding a long sequence
        let mut instructions: Vec<Instruction> = Vec::new();
        for i in 0..1000 {
            instructions.push(Instruction::LoadI {
                dst: Reg((i % 128) as u16),
                value: i as i64,
            });
            instructions.push(Instruction::BinaryI {
                op: BinaryIntOp::Add,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }

        let mut encoded = Vec::new();
        encode_instructions(&instructions, &mut encoded);
        let decoded = decode_instructions(&encoded).expect("Failed to decode sequence");

        assert_eq!(decoded.len(), instructions.len());
        for (i, (orig, dec)) in instructions.iter().zip(decoded.iter()).enumerate() {
            assert_eq!(orig, dec, "Mismatch at instruction {}", i);
        }
    }

    // ========================================================================
    // Security Tests - Malformed Input Handling
    // ========================================================================

    #[test]
    fn test_truncated_input() {
        // Test handling of truncated bytecode
        let instr = Instruction::LoadI {
            dst: Reg(0),
            value: 12345,
        };
        let mut encoded = Vec::new();
        encode_instruction(&instr, &mut encoded);

        // Try decoding with truncated data
        for truncate_at in 0..encoded.len() {
            let truncated = &encoded[..truncate_at];
            let mut offset = 0;
            let result = decode_instruction(truncated, &mut offset);
            // Should either succeed (if we have enough) or return an error
            if truncate_at < encoded.len() {
                // Truncated input should fail gracefully
                assert!(
                    result.is_err() || offset <= truncate_at,
                    "Should handle truncated input at {} bytes",
                    truncate_at
                );
            }
        }
    }

    #[test]
    fn test_incomplete_instruction_handling() {
        // Test that incomplete instructions return errors, not panics.
        // After opcode reorganization, all 256 opcodes map to valid instructions,
        // but most require operand bytes. Providing only the opcode should give Err,
        // not panic.
        let sample_opcodes = [
            0x00, // Mov - needs 2 register bytes
            0x10, // AddI - needs 3 register bytes
            0x20, // AddF - needs 3 register bytes
            0x40, // EqI - needs 3 register bytes
            0x50, // Jmp - needs jump offset
            0xF0, // TensorNew - needs register + shape info
        ];

        for &opcode in &sample_opcodes {
            // This should return Err (incomplete data), not panic
            let result = std::panic::catch_unwind(|| {
                let mut off = 0;
                decode_instruction(&[opcode], &mut off)
            });
            assert!(
                result.is_ok(),
                "Opcode {:#x} decoding panicked instead of returning error",
                opcode
            );
        }
    }

    #[test]
    fn test_malformed_varint() {
        // Test handling of invalid VarInt sequences
        let malformed_cases = [
            vec![0x80], // Incomplete VarInt (continuation bit set but no more bytes)
            vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80], // Too long
        ];

        for malformed in &malformed_cases {
            // Try to decode as LoadK which expects a VarInt
            let mut data = vec![0x01]; // LoadK opcode
            data.push(0x00); // register
            data.extend_from_slice(malformed);

            let mut offset = 0;
            let result = decode_instruction(&data, &mut offset);
            // Should fail gracefully, not panic
            assert!(
                result.is_err() || offset <= data.len(),
                "Malformed VarInt should be handled gracefully"
            );
        }
    }

    #[test]
    fn test_invalid_type_ref_discriminant() {
        // Test handling of invalid TypeRef discriminants
        let invalid_discriminants: &[u8] = &[0x08, 0x09, 0x0A, 0x0B, 0xFF];

        for &disc in invalid_discriminants {
            // LoadT opcode + reg + invalid type ref discriminant
            let data = vec![0x07, 0x00, disc];
            let mut offset = 0;
            let result = decode_instruction(&data, &mut offset);
            // Should return an error, not panic
            assert!(
                result.is_err(),
                "Invalid TypeRef discriminant {:#x} should return error",
                disc
            );
        }
    }

    #[test]
    fn test_overflow_protection_in_vec_decode() {
        // Test protection against memory exhaustion from large vec lengths
        // Encode a vec with a huge claimed length but no actual data
        let mut data = vec![Opcode::Select.to_byte()]; // Select instruction
        data.push(0x00); // dst register

        // First vec (futures) - claim huge length
        // Use max valid VarInt for length
        for _ in 0..5 {
            data.push(0x80 | 0x7F);
        }
        data.push(0x7F); // Final byte without continuation

        let mut offset = 0;
        let result = decode_instruction(&data, &mut offset);
        // Should fail because there's not enough data, not allocate huge vec
        assert!(
            result.is_err(),
            "Huge vec length should fail gracefully without OOM"
        );
    }

    #[test]
    fn test_zero_length_bytecode() {
        let data: &[u8] = &[];
        let mut offset = 0;
        let result = decode_instruction(data, &mut offset);
        assert!(result.is_err(), "Empty input should return error");
    }

    #[test]
    fn test_instruction_after_instruction() {
        // Ensure decode properly advances offset and doesn't read extra
        let instrs = vec![
            Instruction::RetV,
            Instruction::LoadTrue { dst: Reg(0) },
            Instruction::RetV,
        ];

        let mut encoded = Vec::new();
        for i in &instrs {
            encode_instruction(i, &mut encoded);
        }

        let mut offset = 0;
        for expected in &instrs {
            let decoded = decode_instruction(&encoded, &mut offset).unwrap();
            assert_eq!(
                &decoded, expected,
                "Should decode correct instruction in sequence"
            );
        }
        assert_eq!(
            offset,
            encoded.len(),
            "Should consume all bytes exactly"
        );
    }

    // ========================================================================
    // Determinism Tests
    // ========================================================================

    #[test]
    fn test_encoding_determinism() {
        // Same instruction should always encode to same bytes
        let instr = Instruction::CallG {
            dst: Reg(10),
            func_id: 12345,
            type_args: vec![Reg(1), Reg(2), Reg(3)],
            args: RegRange::new(Reg(5), 4),
        };

        let mut enc1 = Vec::new();
        let mut enc2 = Vec::new();
        let mut enc3 = Vec::new();

        encode_instruction(&instr, &mut enc1);
        encode_instruction(&instr, &mut enc2);
        encode_instruction(&instr, &mut enc3);

        assert_eq!(enc1, enc2, "Encoding should be deterministic (1 vs 2)");
        assert_eq!(enc2, enc3, "Encoding should be deterministic (2 vs 3)");
    }

    #[test]
    fn test_all_binary_int_ops() {
        for op in [
            BinaryIntOp::Add,
            BinaryIntOp::Sub,
            BinaryIntOp::Mul,
            BinaryIntOp::Div,
            BinaryIntOp::Mod,
            BinaryIntOp::Pow,
        ] {
            for dst in [Reg(0), Reg(127), Reg(128), Reg(16383)] {
                for a in [Reg(0), Reg(50), Reg(255)] {
                    let instr = Instruction::BinaryI {
                        op,
                        dst,
                        a,
                        b: Reg(0),
                    };
                    test_roundtrip(&instr);
                }
            }
        }
    }

    #[test]
    fn test_all_compare_ops() {
        for op in [
            CompareOp::Eq,
            CompareOp::Ne,
            CompareOp::Lt,
            CompareOp::Le,
            CompareOp::Gt,
            CompareOp::Ge,
        ] {
            // Integer comparison
            test_roundtrip(&Instruction::CmpI {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
            // Float comparison
            test_roundtrip(&Instruction::CmpF {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
            // Jump comparison
            test_roundtrip(&Instruction::JmpCmp {
                op,
                a: Reg(0),
                b: Reg(1),
                offset: 100,
            });
        }
    }

    #[test]
    fn test_all_bitwise_ops() {
        for op in [
            BitwiseOp::And,
            BitwiseOp::Or,
            BitwiseOp::Xor,
            BitwiseOp::Not,
            BitwiseOp::Shl,
            BitwiseOp::Shr,
            BitwiseOp::Ushr,
        ] {
            test_roundtrip(&Instruction::Bitwise {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_all_tensor_dtypes() {
        let dtypes = [
            TensorDType::F64,
            TensorDType::F32,
            TensorDType::F16,
            TensorDType::BF16,
            TensorDType::I64,
            TensorDType::I32,
            TensorDType::I16,
            TensorDType::I8,
            TensorDType::U64,
            TensorDType::U32,
            TensorDType::U16,
            TensorDType::U8,
            TensorDType::Bool,
            TensorDType::Complex64,
            TensorDType::Complex128,
        ];
        for dtype in dtypes {
            test_roundtrip(&Instruction::TensorNew {
                dst: Reg(0),
                dtype,
                dims: vec![Reg(1), Reg(2)],
            });
        }
    }

    #[test]
    fn test_all_tensor_binary_ops() {
        let ops = [
            TensorBinaryOp::Add,
            TensorBinaryOp::Sub,
            TensorBinaryOp::Mul,
            TensorBinaryOp::Div,
            TensorBinaryOp::Pow,
            TensorBinaryOp::Mod,
            TensorBinaryOp::Min,
            TensorBinaryOp::Max,
        ];
        for op in ops {
            test_roundtrip(&Instruction::TensorBinop {
                op,
                dst: Reg(0),
                a: Reg(1),
                b: Reg(2),
            });
        }
    }

    #[test]
    fn test_all_tensor_unary_ops() {
        let ops = [
            TensorUnaryOp::Neg,
            TensorUnaryOp::Abs,
            TensorUnaryOp::Sqrt,
            TensorUnaryOp::Exp,
            TensorUnaryOp::Log,
            TensorUnaryOp::Sin,
            TensorUnaryOp::Cos,
            TensorUnaryOp::Tan,
            TensorUnaryOp::Tanh,
            TensorUnaryOp::Sigmoid,
            TensorUnaryOp::Relu,
            TensorUnaryOp::Gelu,
            TensorUnaryOp::Silu,
            TensorUnaryOp::Floor,
            TensorUnaryOp::Ceil,
            TensorUnaryOp::Round,
            TensorUnaryOp::Sign,
            TensorUnaryOp::Rsqrt,
            TensorUnaryOp::Erf,
            TensorUnaryOp::Log2,
            TensorUnaryOp::Softplus,
            TensorUnaryOp::Mish,
        ];
        for op in ops {
            test_roundtrip(&Instruction::TensorUnop {
                op,
                dst: Reg(0),
                src: Reg(1),
            });
        }
    }

    #[test]
    fn test_all_tensor_reduce_ops() {
        let ops = [
            TensorReduceOp::Sum,
            TensorReduceOp::Prod,
            TensorReduceOp::Max,
            TensorReduceOp::Min,
            TensorReduceOp::Mean,
            TensorReduceOp::Var,
            TensorReduceOp::Std,
            TensorReduceOp::Norm,
            TensorReduceOp::LogSumExp,
            TensorReduceOp::All,
            TensorReduceOp::Any,
        ];
        for op in ops {
            test_roundtrip(&Instruction::TensorReduce {
                op,
                dst: Reg(0),
                src: Reg(1),
                axes: vec![0, 1, 2],
                keepdim: true,
            });
        }
    }

    #[test]
    fn test_all_grad_modes() {
        for mode in [GradMode::Reverse, GradMode::Forward, GradMode::Auto] {
            test_roundtrip(&Instruction::GradBegin {
                scope_id: 1,
                mode,
                wrt: vec![Reg(0), Reg(1)],
            });
        }
    }

    #[test]
    fn test_cbgr_tiers_and_mutability() {
        for mutability in [Mutability::Immutable, Mutability::Mutable] {
            for tier in [CbgrTier::Tier0, CbgrTier::Tier1, CbgrTier::Tier2] {
                let type_ref = TypeRef::Reference {
                    inner: Box::new(TypeRef::Concrete(TypeId(0))),
                    mutability,
                    tier,
                };
                let instr = Instruction::LoadT {
                    dst: Reg(0),
                    type_ref,
                };
                test_roundtrip(&instr);
            }
        }
    }

    // ========================================================================
    // Stress Tests
    // ========================================================================

    #[test]
    fn test_many_nested_instantiated_types() {
        // Create deeply nested instantiated types: List<List<List<...>>>
        let mut type_ref = TypeRef::Concrete(TypeId(1));
        for i in 0..15 {
            type_ref = TypeRef::Instantiated {
                base: TypeId(i + 100),
                args: vec![type_ref],
            };
        }
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    #[test]
    fn test_wide_tuple_type() {
        // Tuple with many elements
        let type_ref = TypeRef::Tuple(
            (0..50)
                .map(|i| TypeRef::Concrete(TypeId(i)))
                .collect(),
        );
        let instr = Instruction::LoadT {
            dst: Reg(0),
            type_ref,
        };
        test_roundtrip(&instr);
    }

    // ========================================================================
    // Closure Tests
    // ========================================================================

    #[test]
    fn test_new_closure_roundtrip() {
        // Empty captures
        test_roundtrip(&Instruction::NewClosure {
            dst: Reg(0),
            func_id: 42,
            captures: vec![],
        });

        // Single capture
        test_roundtrip(&Instruction::NewClosure {
            dst: Reg(0),
            func_id: 100,
            captures: vec![Reg(1)],
        });

        // Multiple captures
        test_roundtrip(&Instruction::NewClosure {
            dst: Reg(0),
            func_id: 12345,
            captures: vec![Reg(1), Reg(2), Reg(3), Reg(4), Reg(5)],
        });

        // Large func_id and many captures
        test_roundtrip(&Instruction::NewClosure {
            dst: Reg(255),
            func_id: u32::MAX / 2,
            captures: (0..20).map(Reg).collect(),
        });
    }

    #[test]
    fn test_call_closure_roundtrip() {
        // No arguments
        test_roundtrip(&Instruction::CallClosure {
            dst: Reg(0),
            closure: Reg(1),
            args: RegRange::new(Reg(0), 0),
        });

        // Single argument
        test_roundtrip(&Instruction::CallClosure {
            dst: Reg(0),
            closure: Reg(5),
            args: RegRange::new(Reg(10), 1),
        });

        // Multiple arguments
        test_roundtrip(&Instruction::CallClosure {
            dst: Reg(0),
            closure: Reg(1),
            args: RegRange::new(Reg(2), 5),
        });
    }

    #[test]
    fn test_closure_sequence() {
        // Simulate creating a closure that captures a variable and calling it
        let instructions = vec![
            // Load captured value
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            // Create closure with captured value
            Instruction::NewClosure {
                dst: Reg(1),
                func_id: 0,
                captures: vec![Reg(0)],
            },
            // Load argument for closure call
            Instruction::LoadI {
                dst: Reg(2),
                value: 10,
            },
            // Call the closure
            Instruction::CallClosure {
                dst: Reg(3),
                closure: Reg(1),
                args: RegRange::new(Reg(2), 1),
            },
            // Return result
            Instruction::Ret { value: Reg(3) },
        ];

        let mut encoded = Vec::new();
        encode_instructions(&instructions, &mut encoded);

        let decoded = decode_instructions(&encoded).expect("Failed to decode closure sequence");
        assert_eq!(decoded.len(), instructions.len());

        for (original, decoded) in instructions.iter().zip(decoded.iter()) {
            assert_eq!(original, decoded);
        }
    }

    #[test]
    fn test_nested_closure_captures() {
        // Test creating multiple closures with different captures
        let instructions = vec![
            // Load base values
            Instruction::LoadI {
                dst: Reg(0),
                value: 1,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 2,
            },
            Instruction::LoadI {
                dst: Reg(2),
                value: 3,
            },
            // Outer closure captures r0
            Instruction::NewClosure {
                dst: Reg(10),
                func_id: 100,
                captures: vec![Reg(0)],
            },
            // Inner closure captures r0, r1
            Instruction::NewClosure {
                dst: Reg(11),
                func_id: 101,
                captures: vec![Reg(0), Reg(1)],
            },
            // Combined closure captures all three
            Instruction::NewClosure {
                dst: Reg(12),
                func_id: 102,
                captures: vec![Reg(0), Reg(1), Reg(2)],
            },
        ];

        let mut encoded = Vec::new();
        encode_instructions(&instructions, &mut encoded);

        let decoded = decode_instructions(&encoded).expect("Failed to decode nested closures");
        assert_eq!(decoded.len(), instructions.len());

        for (original, decoded) in instructions.iter().zip(decoded.iter()) {
            assert_eq!(original, decoded);
        }
    }
}
