//! # Band-wrapper expansion for registry intrinsics (T0103 LEG-2b)
//!
//! Cross-module band `Call`s recorded against intrinsic fn-forms
//! (`core.base.primitives.eq`, `core.intrinsics.memory.ptr_offset`, …)
//! can reach module assembly with NO body under the recorded name:
//! the defining module's template body was never materialized into
//! this compile, and the re-export spelling (`core.base.primitives.eq`
//! for `core.intrinsics.arithmetic.eq`) defeats ranked suffix
//! resolution. Tier-0 fails those calls loud (`FunctionNotFound`);
//! Tier-1 AOT degraded them to silent const-zero stubs.
//!
//! This module turns a REGISTRY intrinsic into a self-contained
//! wrapper body — params in `r0..rN`, result in `r(N)`, `Ret` — so
//! `VbcModule::synthesize_intrinsic_band_wrappers` can materialize a
//! callee for such a band name once, at module assembly, for BOTH
//! tiers. The registry stays the one strategy authority
//! (docs/architecture/intrinsic-dispatch-contract.md §1).
//!
//! ## Fidelity contract
//!
//! Every arm here MIRRORS the corresponding call-site expansion in
//! `codegen/expressions.rs` (`emit_intrinsic_instructions` and its
//! per-strategy helpers) instruction-for-instruction — that expansion
//! is what the same intrinsic compiles to at an in-module call site,
//! and what the archived template bodies already contain. A strategy
//! whose call-site expansion needs codegen-context knowledge
//! (compile-time constants, tensor shapes, call-site pointer strides
//! other than the default) is NOT synthesized — `expand_intrinsic_wrapper`
//! returns `None` and the band name stays loudly unresolved.
//! `tests` below pin representative sequences byte-for-byte; if the
//! call-site emitter changes shape, change it here in the same commit.

use super::registry::{CodegenStrategy, InlineSequenceId, Intrinsic};
use crate::instruction::{
    ArithSubOpcode, AtomicRmwOp, BinaryFloatOp, BinaryIntOp, BitwiseOp, CompareOp, FloatToIntMode,
    Instruction, Opcode, Reg, UnaryFloatOp, UnaryIntOp,
};

/// A synthesized wrapper body: straight-line instructions ending in
/// `Ret`, with params in `r0..param_count` and `register_count`
/// covering every register the body touches.
#[derive(Debug)]
pub struct WrapperBody {
    /// The wrapper's instructions (branch-free, ends with `Ret`).
    pub instructions: Vec<Instruction>,
    /// Total registers used (params + dest + temps).
    pub register_count: u16,
}

/// Bump-allocating instruction sink. `free` is intentionally absent:
/// wrapper bodies are tiny and a strictly-growing register file keeps
/// the expansion trivially deterministic.
struct Emitter {
    out: Vec<Instruction>,
    next_reg: u16,
}

impl Emitter {
    fn emit(&mut self, instr: Instruction) {
        self.out.push(instr);
    }
    fn alloc_temp(&mut self) -> Reg {
        let r = Reg(self.next_reg);
        self.next_reg += 1;
        r
    }
}

/// Register byte encoding for extended-opcode operand vectors —
/// mirrors `VbcCodegen::write_reg` (short `< 128`, long `0x80 |` hi).
fn write_reg(operands: &mut Vec<u8>, reg: u16) {
    if reg < 128 {
        operands.push(reg as u8);
    } else {
        operands.push(0x80 | ((reg >> 8) as u8));
        operands.push((reg & 0xFF) as u8);
    }
}

fn extended_operands(dest: Reg, args: &[Reg]) -> Vec<u8> {
    let mut operands = Vec::with_capacity(2 + args.len() * 2);
    write_reg(&mut operands, dest.0);
    for a in args {
        write_reg(&mut operands, a.0);
    }
    operands
}

/// Expand a registry intrinsic into a wrapper body, or `None` when the
/// strategy is not synthesizable without call-site context.
pub fn expand_intrinsic_wrapper(intr: &Intrinsic) -> Option<WrapperBody> {
    let n = intr.param_count as u16;
    let args: Vec<Reg> = (0..n).map(Reg).collect();
    let dest = Reg(n);
    let mut e = Emitter {
        out: Vec::new(),
        next_reg: n + 1,
    };

    let emitted = match &intr.strategy {
        CodegenStrategy::DirectOpcode(op) => expand_direct(&mut e, *op, &args, dest),
        CodegenStrategy::OpcodeWithMode(op, mode) => {
            expand_with_mode(&mut e, *op, *mode, &args, dest)
        }
        CodegenStrategy::OpcodeWithSize(op, size) => {
            expand_with_size(&mut e, *op, *size, &args, dest)
        }
        CodegenStrategy::ArithExtendedOpcode(sub) => {
            if args.is_empty() {
                false
            } else {
                e.emit(Instruction::ArithExtended {
                    sub_op: *sub as u8,
                    operands: extended_operands(dest, &args),
                });
                true
            }
        }
        CodegenStrategy::MathExtendedOpcode(sub) => {
            e.emit(Instruction::MathExtended {
                sub_op: *sub as u8,
                operands: extended_operands(dest, &args),
            });
            true
        }
        CodegenStrategy::InlineSequence(seq) => {
            expand_sequence(&mut e, *seq, &args, dest, 8)
        }
        CodegenStrategy::InlineSequenceWithWidth(seq, width) => {
            expand_sequence(&mut e, *seq, &args, dest, *width)
        }
        // Compile-time constants, tensor/GPU trees, wrapping/saturating
        // width-typed strategies and everything else need call-site or
        // type context this synthesis point does not have.
        _ => false,
    };
    if !emitted {
        return None;
    }

    e.emit(Instruction::Ret { value: dest });
    Some(WrapperBody {
        register_count: e.next_reg,
        instructions: e.out,
    })
}

/// Mirror of `emit_intrinsic_direct_opcode` for the opcodes that can
/// appear as cross-module intrinsic fn-forms. Unknown opcodes return
/// `false` (no wrapper) rather than the call-site `LoadNil` fallback:
/// a silent nil-returning wrapper would recreate exactly the
/// const-zero class this synthesis exists to remove.
fn expand_direct(e: &mut Emitter, op: Opcode, args: &[Reg], dest: Reg) -> bool {
    let bin_i = |e: &mut Emitter, biop: BinaryIntOp| {
        e.emit(Instruction::BinaryI {
            op: biop,
            dst: dest,
            a: args[0],
            b: args[1],
        });
    };
    let bin_f = |e: &mut Emitter, bfop: BinaryFloatOp| {
        e.emit(Instruction::BinaryF {
            op: bfop,
            dst: dest,
            a: args[0],
            b: args[1],
        });
    };
    let bitwise = |e: &mut Emitter, bop: BitwiseOp| {
        e.emit(Instruction::Bitwise {
            op: bop,
            dst: dest,
            a: args[0],
            b: args[1],
        });
    };
    let cmp_i = |e: &mut Emitter, cop: CompareOp| {
        e.emit(Instruction::CmpI {
            op: cop,
            dst: dest,
            a: args[0],
            b: args[1],
        });
    };

    match op {
        Opcode::AddI if args.len() >= 2 => bin_i(e, BinaryIntOp::Add),
        Opcode::SubI if args.len() >= 2 => bin_i(e, BinaryIntOp::Sub),
        Opcode::MulI if args.len() >= 2 => bin_i(e, BinaryIntOp::Mul),
        Opcode::DivI if args.len() >= 2 => bin_i(e, BinaryIntOp::Div),
        Opcode::ModI if args.len() >= 2 => bin_i(e, BinaryIntOp::Mod),
        Opcode::AddF if args.len() >= 2 => bin_f(e, BinaryFloatOp::Add),
        Opcode::SubF if args.len() >= 2 => bin_f(e, BinaryFloatOp::Sub),
        Opcode::MulF if args.len() >= 2 => bin_f(e, BinaryFloatOp::Mul),
        Opcode::DivF if args.len() >= 2 => bin_f(e, BinaryFloatOp::Div),
        Opcode::Band if args.len() >= 2 => bitwise(e, BitwiseOp::And),
        Opcode::Bor if args.len() >= 2 => bitwise(e, BitwiseOp::Or),
        Opcode::Bxor if args.len() >= 2 => bitwise(e, BitwiseOp::Xor),
        Opcode::Bnot if !args.is_empty() => {
            e.emit(Instruction::Bitwise {
                op: BitwiseOp::Not,
                dst: dest,
                a: args[0],
                b: args[0],
            });
        }
        Opcode::Shl if args.len() >= 2 => bitwise(e, BitwiseOp::Shl),
        Opcode::Shr if args.len() >= 2 => bitwise(e, BitwiseOp::Shr),
        Opcode::Ushr if args.len() >= 2 => bitwise(e, BitwiseOp::Ushr),
        Opcode::EqI if args.len() >= 2 => cmp_i(e, CompareOp::Eq),
        Opcode::NeI if args.len() >= 2 => cmp_i(e, CompareOp::Ne),
        Opcode::LtI if args.len() >= 2 => cmp_i(e, CompareOp::Lt),
        Opcode::LeI if args.len() >= 2 => cmp_i(e, CompareOp::Le),
        Opcode::GtI if args.len() >= 2 => cmp_i(e, CompareOp::Gt),
        Opcode::GeI if args.len() >= 2 => cmp_i(e, CompareOp::Ge),
        Opcode::NegI if !args.is_empty() => {
            e.emit(Instruction::UnaryI {
                op: UnaryIntOp::Neg,
                dst: dest,
                src: args[0],
            });
        }
        Opcode::NegF if !args.is_empty() => {
            e.emit(Instruction::UnaryF {
                op: UnaryFloatOp::Neg,
                dst: dest,
                src: args[0],
            });
        }
        Opcode::AbsF if !args.is_empty() => {
            e.emit(Instruction::UnaryF {
                op: UnaryFloatOp::Abs,
                dst: dest,
                src: args[0],
            });
        }
        Opcode::AtomicLoad if args.len() >= 2 => {
            e.emit(Instruction::AtomicLoad {
                dst: dest,
                ptr: args[0],
                ordering: 2,
                size: 8,
            });
        }
        Opcode::AtomicStore if args.len() >= 2 => {
            e.emit(Instruction::AtomicStore {
                ptr: args[0],
                val: args[1],
                ordering: 2,
                size: 8,
            });
            e.emit(Instruction::LoadNil { dst: dest });
        }
        Opcode::AtomicCas if args.len() >= 3 => {
            e.emit(Instruction::AtomicCas {
                dst: dest,
                ptr: args[0],
                expected: args[1],
                desired: args[2],
                ordering: 4,
                size: 8,
            });
        }
        Opcode::AtomicFence => {
            e.emit(Instruction::AtomicFence { ordering: 2 });
            e.emit(Instruction::LoadNil { dst: dest });
        }
        Opcode::Deref if !args.is_empty() => {
            e.emit(Instruction::Deref {
                dst: dest,
                ref_reg: args[0],
            });
        }
        Opcode::DerefMut if args.len() >= 2 => {
            e.emit(Instruction::DerefMut {
                ref_reg: args[0],
                value: args[1],
            });
            e.emit(Instruction::LoadNil { dst: dest });
        }
        Opcode::Unreachable => {
            e.emit(Instruction::Unreachable);
            e.emit(Instruction::LoadNil { dst: dest });
        }
        Opcode::LoadI => {
            e.emit(Instruction::LoadI {
                dst: dest,
                value: 0,
            });
        }
        Opcode::CvtIF => {
            let src = if !args.is_empty() { args[0] } else { dest };
            e.emit(Instruction::CvtIF { dst: dest, src });
        }
        Opcode::CvtFI => {
            let src = if !args.is_empty() { args[0] } else { dest };
            e.emit(Instruction::CvtFI {
                mode: FloatToIntMode::Trunc,
                dst: dest,
                src,
            });
        }
        _ => return false,
    }
    true
}

/// Mirror of `emit_intrinsic_opcode_with_mode`.
fn expand_with_mode(e: &mut Emitter, op: Opcode, mode: u8, args: &[Reg], dest: Reg) -> bool {
    match op {
        Opcode::AtomicLoad if !args.is_empty() => {
            e.emit(Instruction::AtomicLoad {
                dst: dest,
                ptr: args[0],
                ordering: mode,
                size: 8,
            });
            true
        }
        Opcode::AtomicStore if args.len() >= 2 => {
            e.emit(Instruction::AtomicStore {
                ptr: args[0],
                val: args[1],
                ordering: mode,
                size: 8,
            });
            e.emit(Instruction::LoadNil { dst: dest });
            true
        }
        Opcode::AtomicFence => {
            e.emit(Instruction::AtomicFence { ordering: mode });
            e.emit(Instruction::LoadNil { dst: dest });
            true
        }
        _ => expand_direct(e, op, args, dest),
    }
}

/// Mirror of `emit_intrinsic_opcode_with_size`.
fn expand_with_size(e: &mut Emitter, op: Opcode, size: u8, args: &[Reg], dest: Reg) -> bool {
    match op {
        Opcode::AtomicLoad if !args.is_empty() => {
            e.emit(Instruction::AtomicLoad {
                dst: dest,
                ptr: args[0],
                ordering: 1,
                size,
            });
            true
        }
        Opcode::AtomicStore if args.len() >= 2 => {
            e.emit(Instruction::AtomicStore {
                ptr: args[0],
                val: args[1],
                ordering: 2,
                size,
            });
            e.emit(Instruction::LoadNil { dst: dest });
            true
        }
        Opcode::AtomicCas if args.len() >= 3 => {
            e.emit(Instruction::AtomicCas {
                dst: dest,
                ptr: args[0],
                expected: args[1],
                desired: args[2],
                ordering: 4,
                size,
            });
            true
        }
        _ => expand_direct(e, op, args, dest),
    }
}

/// Mirror of `emit_arith_extended_unary` / `_binary` / `_ternary`.
fn arith_extended(e: &mut Emitter, sub: ArithSubOpcode, dest: Reg, srcs: &[Reg]) {
    e.emit(Instruction::ArithExtended {
        sub_op: sub.to_byte(),
        operands: extended_operands(dest, srcs),
    });
}

/// Mirror of the synthesizable arms of `emit_intrinsic_inline_sequence`.
fn expand_sequence(
    e: &mut Emitter,
    seq: InlineSequenceId,
    args: &[Reg],
    dest: Reg,
    byte_width: u8,
) -> bool {
    match seq {
        InlineSequenceId::Memcpy | InlineSequenceId::Memmove if args.len() >= 3 => {
            let sub_op = if matches!(seq, InlineSequenceId::Memcpy) {
                0x43 // CMemcpy
            } else {
                0x45 // CMemmove
            };
            let mut operands = Vec::<u8>::new();
            write_reg(&mut operands, args[0].0);
            write_reg(&mut operands, args[1].0);
            write_reg(&mut operands, args[2].0);
            e.emit(Instruction::FfiExtended { sub_op, operands });
            e.emit(Instruction::Mov {
                dst: dest,
                src: args[0],
            });
        }
        InlineSequenceId::Memset if args.len() >= 3 => {
            let mut operands = Vec::<u8>::new();
            write_reg(&mut operands, args[0].0);
            write_reg(&mut operands, args[1].0);
            write_reg(&mut operands, args[2].0);
            e.emit(Instruction::FfiExtended {
                sub_op: 0x44, // CMemset
                operands,
            });
            e.emit(Instruction::Mov {
                dst: dest,
                src: args[0],
            });
        }
        InlineSequenceId::Memcmp if args.len() >= 3 => {
            let mut operands = Vec::<u8>::new();
            write_reg(&mut operands, dest.0);
            write_reg(&mut operands, args[0].0);
            write_reg(&mut operands, args[1].0);
            write_reg(&mut operands, args[2].0);
            e.emit(Instruction::FfiExtended {
                sub_op: 0x46, // CMemcmp
                operands,
            });
        }
        // The wrapper synthesis point has no call-site pointee type, so
        // it takes the default 8-byte Value stride — the same stride the
        // archived template body was compiled with (`ptr_offset<T>`'s
        // own signature resolves to the default). The `&unsafe Byte`
        // stride-1 refinement is call-site-only by design.
        InlineSequenceId::PtrOffset | InlineSequenceId::PtrSubSeq if args.len() >= 2 => {
            let sub_op = if matches!(seq, InlineSequenceId::PtrOffset) {
                0x63 // PtrAdd (element-scaled ×8)
            } else {
                0x64 // PtrSub (element-scaled ×8)
            };
            let mut operands = Vec::<u8>::new();
            write_reg(&mut operands, dest.0);
            write_reg(&mut operands, args[0].0);
            write_reg(&mut operands, args[1].0);
            e.emit(Instruction::FfiExtended { sub_op, operands });
        }
        InlineSequenceId::CheckedAdd
        | InlineSequenceId::CheckedSub
        | InlineSequenceId::CheckedMul
        | InlineSequenceId::CheckedDiv
            if args.len() >= 2 =>
        {
            let sub_op = match seq {
                InlineSequenceId::CheckedAdd => ArithSubOpcode::CheckedAddI as u8,
                InlineSequenceId::CheckedSub => ArithSubOpcode::CheckedSubI as u8,
                InlineSequenceId::CheckedMul => ArithSubOpcode::CheckedMulI as u8,
                _ => ArithSubOpcode::CheckedDivI as u8,
            };
            let mut operands = Vec::<u8>::new();
            write_reg(&mut operands, dest.0);
            write_reg(&mut operands, args[0].0);
            write_reg(&mut operands, args[1].0);
            e.emit(Instruction::ArithExtended { sub_op, operands });
        }
        InlineSequenceId::OverflowingAdd
        | InlineSequenceId::OverflowingSub
        | InlineSequenceId::OverflowingMul
            if args.len() >= 2 =>
        {
            let sub_op = match seq {
                InlineSequenceId::OverflowingAdd => ArithSubOpcode::OverflowingAddI as u8,
                InlineSequenceId::OverflowingSub => ArithSubOpcode::OverflowingSubI as u8,
                _ => ArithSubOpcode::OverflowingMulI as u8,
            };
            let mut operands = Vec::<u8>::new();
            write_reg(&mut operands, dest.0);
            write_reg(&mut operands, args[0].0);
            write_reg(&mut operands, args[1].0);
            e.emit(Instruction::ArithExtended { sub_op, operands });
        }
        InlineSequenceId::AtomicFetchAdd
        | InlineSequenceId::AtomicFetchSub
        | InlineSequenceId::AtomicFetchAnd
        | InlineSequenceId::AtomicFetchOr
        | InlineSequenceId::AtomicFetchXor
            if args.len() >= 2 =>
        {
            // Mirrors the codegen arm: ONE indivisible RMW opcode, not
            // a load / modify / single-shot-CAS sequence that drops
            // updates when threads interleave.
            let op = match seq {
                InlineSequenceId::AtomicFetchAdd => AtomicRmwOp::Add,
                InlineSequenceId::AtomicFetchSub => AtomicRmwOp::Sub,
                InlineSequenceId::AtomicFetchAnd => AtomicRmwOp::And,
                InlineSequenceId::AtomicFetchOr => AtomicRmwOp::Or,
                _ => AtomicRmwOp::Xor,
            };
            e.emit(op.encode(dest, args[0], args[1], byte_width));
        }
        InlineSequenceId::AtomicExchange if args.len() >= 2 => {
            // The swap form of the same indivisible RMW opcode. Kept in
            // lockstep with the codegen inline path and the fetch arm
            // above so the wrapper-synthesis and call-site emitters can
            // never disagree on how an atomic exchange lowers.
            e.emit(AtomicRmwOp::Xchg.encode(dest, args[0], args[1], byte_width));
        }
        InlineSequenceId::Clz => arith_extended(e, ArithSubOpcode::Clz, dest, args),
        InlineSequenceId::Ctz => arith_extended(e, ArithSubOpcode::Ctz, dest, args),
        InlineSequenceId::ClzU32 => {
            let clz_tmp = e.alloc_temp();
            arith_extended(e, ArithSubOpcode::Clz, clz_tmp, args);
            let thirty_two = e.alloc_temp();
            e.emit(Instruction::LoadI {
                dst: thirty_two,
                value: 32,
            });
            e.emit(Instruction::BinaryI {
                op: BinaryIntOp::Sub,
                dst: dest,
                a: clz_tmp,
                b: thirty_two,
            });
        }
        InlineSequenceId::CtzU32 if !args.is_empty() => {
            let guard = e.alloc_temp();
            e.emit(Instruction::LoadI {
                dst: guard,
                value: 1i64 << 32,
            });
            let masked = e.alloc_temp();
            e.emit(Instruction::Bitwise {
                op: BitwiseOp::Or,
                dst: masked,
                a: args[0],
                b: guard,
            });
            arith_extended(e, ArithSubOpcode::Ctz, dest, &[masked]);
        }
        InlineSequenceId::Ilog2 => {
            let clz_tmp = e.alloc_temp();
            arith_extended(e, ArithSubOpcode::Clz, clz_tmp, args);
            let sixty_three = e.alloc_temp();
            e.emit(Instruction::LoadI {
                dst: sixty_three,
                value: 63,
            });
            e.emit(Instruction::BinaryI {
                op: BinaryIntOp::Sub,
                dst: dest,
                a: sixty_three,
                b: clz_tmp,
            });
        }
        InlineSequenceId::Popcnt => arith_extended(e, ArithSubOpcode::Popcnt, dest, args),
        InlineSequenceId::Bswap => arith_extended(e, ArithSubOpcode::Bswap, dest, args),
        InlineSequenceId::Bitreverse => {
            arith_extended(e, ArithSubOpcode::BitReverse, dest, args)
        }
        InlineSequenceId::ByteSwapBits => {
            let rev_tmp = e.alloc_temp();
            arith_extended(e, ArithSubOpcode::BitReverse, rev_tmp, args);
            arith_extended(e, ArithSubOpcode::Bswap, dest, &[rev_tmp]);
        }
        InlineSequenceId::NullPtr => {
            e.emit(Instruction::LoadI {
                dst: dest,
                value: 0,
            });
        }
        InlineSequenceId::PtrIsNull if !args.is_empty() => {
            let zero = e.alloc_temp();
            e.emit(Instruction::LoadI {
                dst: zero,
                value: 0,
            });
            e.emit(Instruction::CmpI {
                op: CompareOp::Eq,
                dst: dest,
                a: args[0],
                b: zero,
            });
        }
        InlineSequenceId::RotateLeft => {
            arith_extended(e, ArithSubOpcode::RotateLeft, dest, args)
        }
        InlineSequenceId::RotateRight => {
            arith_extended(e, ArithSubOpcode::RotateRight, dest, args)
        }
        InlineSequenceId::Fshl if args.len() >= 3 => {
            arith_extended(e, ArithSubOpcode::FunnelShiftLeft, dest, args)
        }
        InlineSequenceId::Fshr if args.len() >= 3 => {
            arith_extended(e, ArithSubOpcode::FunnelShiftRight, dest, args)
        }
        InlineSequenceId::F32ToBits
        | InlineSequenceId::F32FromBits
        | InlineSequenceId::F64ToBits
        | InlineSequenceId::F64FromBits => {
            let sub = match seq {
                InlineSequenceId::F32ToBits => ArithSubOpcode::F32ToBits,
                InlineSequenceId::F32FromBits => ArithSubOpcode::F32FromBits,
                InlineSequenceId::F64ToBits => ArithSubOpcode::F64ToBits,
                _ => ArithSubOpcode::F64FromBits,
            };
            let src = if !args.is_empty() { args[0] } else { dest };
            e.emit(Instruction::ArithExtended {
                sub_op: sub as u8,
                operands: extended_operands(dest, &[src]),
            });
        }
        InlineSequenceId::ToLeBytes | InlineSequenceId::ToBeBytes => {
            let width = byte_width as usize;
            let src = if !args.is_empty() { args[0] } else { dest };
            e.emit(Instruction::NewList {
                dst: dest,
                capacity_hint: 0,
            });
            let shift_reg = e.alloc_temp();
            let byte_reg = e.alloc_temp();
            let mask_reg = e.alloc_temp();
            e.emit(Instruction::LoadI {
                dst: mask_reg,
                value: 0xFF,
            });
            let idx: Vec<usize> = if matches!(seq, InlineSequenceId::ToLeBytes) {
                (0..width).collect()
            } else {
                (0..width).rev().collect()
            };
            for i in idx {
                if i == 0 {
                    e.emit(Instruction::Mov {
                        dst: shift_reg,
                        src,
                    });
                } else {
                    let shift_amt = e.alloc_temp();
                    e.emit(Instruction::LoadI {
                        dst: shift_amt,
                        value: (i * 8) as i64,
                    });
                    e.emit(Instruction::Bitwise {
                        op: BitwiseOp::Shr,
                        dst: shift_reg,
                        a: src,
                        b: shift_amt,
                    });
                }
                e.emit(Instruction::Bitwise {
                    op: BitwiseOp::And,
                    dst: byte_reg,
                    a: shift_reg,
                    b: mask_reg,
                });
                e.emit(Instruction::ListPush {
                    list: dest,
                    val: byte_reg,
                });
            }
        }
        InlineSequenceId::FromLeBytes | InlineSequenceId::FromBeBytes => {
            let width = byte_width as usize;
            let src = if !args.is_empty() { args[0] } else { dest };
            e.emit(Instruction::LoadI {
                dst: dest,
                value: 0,
            });
            let byte_reg = e.alloc_temp();
            let shifted_reg = e.alloc_temp();
            let idx_reg = e.alloc_temp();
            let shift_amt = e.alloc_temp();
            for i in 0..width {
                e.emit(Instruction::LoadI {
                    dst: idx_reg,
                    value: i as i64,
                });
                e.emit(Instruction::GetE {
                    dst: byte_reg,
                    arr: src,
                    idx: idx_reg,
                });
                let shift_bits = if matches!(seq, InlineSequenceId::FromLeBytes) {
                    i * 8
                } else {
                    (width - 1 - i) * 8
                };
                if shift_bits == 0 {
                    e.emit(Instruction::Bitwise {
                        op: BitwiseOp::Or,
                        dst: dest,
                        a: dest,
                        b: byte_reg,
                    });
                } else {
                    e.emit(Instruction::LoadI {
                        dst: shift_amt,
                        value: shift_bits as i64,
                    });
                    e.emit(Instruction::Bitwise {
                        op: BitwiseOp::Shl,
                        dst: shifted_reg,
                        a: byte_reg,
                        b: shift_amt,
                    });
                    e.emit(Instruction::Bitwise {
                        op: BitwiseOp::Or,
                        dst: dest,
                        a: dest,
                        b: shifted_reg,
                    });
                }
            }
        }
        InlineSequenceId::DropInPlace => {
            if !args.is_empty() {
                e.emit(Instruction::ChkRef { ref_reg: args[0] });
            }
            e.emit(Instruction::LoadNil { dst: dest });
        }
        InlineSequenceId::MakeSlice if args.len() >= 2 => {
            let mut operands = Vec::with_capacity(7);
            write_reg(&mut operands, dest.0);
            write_reg(&mut operands, args[0].0);
            write_reg(&mut operands, args[1].0);
            operands.push(byte_width);
            e.emit(Instruction::CbgrExtended {
                sub_op: crate::instruction::CbgrSubOpcode::RefSliceRaw as u8,
                operands,
            });
        }
        InlineSequenceId::Uninit => {
            e.emit(Instruction::LoadUnit { dst: dest });
        }
        InlineSequenceId::CbgrDealloc => {
            let mut operands = Vec::<u8>::new();
            write_reg(&mut operands, dest.0);
            for a in args {
                write_reg(&mut operands, a.0);
            }
            e.emit(Instruction::FfiExtended {
                sub_op: 0xA2, // CbgrDealloc
                operands,
            });
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intrinsics::lookup_intrinsic;

    fn expand_by_name(name: &str) -> WrapperBody {
        let info = lookup_intrinsic(name).unwrap_or_else(|| panic!("no registry entry: {name}"));
        expand_intrinsic_wrapper(info.intrinsic)
            .unwrap_or_else(|| panic!("not synthesizable: {name}"))
    }

    /// `eq` (DirectOpcode(EqI)): CmpI Eq r2 <- r0, r1; Ret r2 — the
    /// wrapper twin of the `Opcode::EqI` arm in
    /// `emit_intrinsic_direct_opcode`.
    #[test]
    fn eq_wrapper_is_cmpi_ret() {
        let body = expand_by_name("eq");
        assert_eq!(
            body.instructions,
            vec![
                Instruction::CmpI {
                    op: CompareOp::Eq,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ]
        );
        assert_eq!(body.register_count, 3);
    }

    /// `wrapping_add` (DirectOpcode(AddI)) — i64 wraparound semantics
    /// come from BinaryI::Add exactly as the template body does.
    #[test]
    fn wrapping_add_wrapper_is_binaryi_ret() {
        let body = expand_by_name("wrapping_add");
        assert_eq!(
            body.instructions,
            vec![
                Instruction::BinaryI {
                    op: BinaryIntOp::Add,
                    dst: Reg(2),
                    a: Reg(0),
                    b: Reg(1),
                },
                Instruction::Ret { value: Reg(2) },
            ]
        );
    }

    /// `clz` (InlineSequence(Clz)): ArithExtended sub=Clz operands
    /// [dst=r1, src=r0]; Ret r1.
    #[test]
    fn clz_wrapper_is_arith_extended() {
        let body = expand_by_name("clz");
        assert_eq!(
            body.instructions,
            vec![
                Instruction::ArithExtended {
                    sub_op: ArithSubOpcode::Clz.to_byte(),
                    operands: vec![1, 0],
                },
                Instruction::Ret { value: Reg(1) },
            ]
        );
    }

    /// `saturating_add` (ArithExtendedOpcode): one ArithExtended with
    /// [dst, a, b] operands.
    #[test]
    fn saturating_add_wrapper_shape() {
        let body = expand_by_name("saturating_add");
        assert_eq!(
            body.instructions,
            vec![
                Instruction::ArithExtended {
                    sub_op: ArithSubOpcode::SaturatingAdd as u8,
                    operands: vec![2, 0, 1],
                },
                Instruction::Ret { value: Reg(2) },
            ]
        );
    }

    /// `to_le_bytes` (InlineSequenceWithWidth(ToLeBytes, 8)): NewList +
    /// per-byte shift/mask/push, exactly the call-site loop shape.
    #[test]
    fn to_le_bytes_wrapper_shape() {
        let body = expand_by_name("to_le_bytes");
        // NewList + LoadI(mask) + 8×(shift setup + And + Push) + Ret.
        assert!(matches!(
            body.instructions.first(),
            Some(Instruction::NewList { dst: Reg(1), .. })
        ));
        assert!(matches!(
            body.instructions.last(),
            Some(Instruction::Ret { value: Reg(1) })
        ));
        let pushes = body
            .instructions
            .iter()
            .filter(|i| matches!(i, Instruction::ListPush { .. }))
            .count();
        assert_eq!(pushes, 8);
    }

    /// `ptr_offset` keeps the default 8-byte Value stride (FfiExtended
    /// PtrAdd 0x63) — the call-site stride-1 byte-buffer refinement is
    /// deliberately out of wrapper scope.
    #[test]
    fn ptr_offset_wrapper_is_ptradd() {
        let body = expand_by_name("ptr_offset");
        assert_eq!(
            body.instructions,
            vec![
                Instruction::FfiExtended {
                    sub_op: 0x63,
                    operands: vec![2, 0, 1],
                },
                Instruction::Ret { value: Reg(2) },
            ]
        );
    }

    /// Family-B runtime-raw declarations (`__async_read_raw` …) have no
    /// registry entry — the synthesis must refuse them so they stay
    /// loudly unresolved rather than silently nil.
    #[test]
    fn runtime_raw_names_are_not_synthesizable() {
        assert!(lookup_intrinsic("__async_read_raw").is_none());
    }
}
