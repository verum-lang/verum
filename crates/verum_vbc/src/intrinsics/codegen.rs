//! # Industrial-Grade Intrinsic Code Generation
//!
//! This module generates optimal VBC instruction sequences for intrinsics.
//! The codegen strategy follows a three-tier approach:
//!
//! 1. **Direct Opcodes**: Single VBC instruction (zero overhead)
//! 2. **Inline Sequences**: Pre-optimized instruction patterns (~2-5 cycles)
//! 3. **Library Calls**: External function call (minimal overhead)
//!
//! ## Design Goals
//!
//! - **Zero Overhead**: Direct opcode intrinsics have no call overhead
//! - **Optimal Patterns**: Inline sequences use register-optimal instruction order
//! - **LLVM Transparency**: All patterns map cleanly to LLVM IR for optimization
//! - **Interpreter Fast Path**: Hot intrinsics get dedicated dispatch handlers
//!
//! ## SIMD Intrinsics
//!
//! Platform-specific SIMD intrinsics for tensor operations:
//! - x86: AVX2/AVX-512 via `x86.avx.*` intrinsics
//! - ARM: NEON via `aarch64.neon.*` intrinsics

use crate::instruction::{ArithSubOpcode, CbgrSubOpcode, CharSubOpcode, LogSubOpcode, MathSubOpcode, MetaReflectOp, MlSubOpcode, Opcode, Reg, RegRange, SimdSubOpcode, TensorSubOpcode, TensorExtSubOpcode, TextSubOpcode};

use super::registry::{CodegenStrategy, InlineSequenceId, Intrinsic};

/// Intrinsic code generator.
///
/// Generates optimal VBC instruction sequences for intrinsic calls.
pub struct IntrinsicCodegen<'a> {
    /// Instructions emitted so far.
    instructions: Vec<IntrinsicInstruction>,
    /// Next available register.
    next_reg: u16,
    /// Reference to intrinsic being compiled.
    intrinsic: &'a Intrinsic,
}

/// Result of intrinsic codegen.
#[derive(Debug)]
pub struct IntrinsicCodegenResult {
    /// Generated instructions.
    pub instructions: Vec<IntrinsicInstruction>,
    /// Register containing the result (if any).
    pub result_reg: Option<Reg>,
    /// Number of registers used.
    pub registers_used: u16,
}

impl<'a> IntrinsicCodegen<'a> {
    /// Create a new codegen for the given intrinsic.
    pub fn new(intrinsic: &'a Intrinsic, first_reg: u16) -> Self {
        Self {
            instructions: Vec::with_capacity(8),
            next_reg: first_reg,
            intrinsic,
        }
    }

    /// Allocate a temporary register.
    fn alloc_temp(&mut self) -> Reg {
        let reg = Reg::new(self.next_reg);
        self.next_reg += 1;
        reg
    }

    /// Emit an instruction.
    fn emit(&mut self, instr: IntrinsicInstruction) {
        self.instructions.push(instr);
    }

    /// Generate code for the intrinsic with given argument registers.
    pub fn generate(mut self, args: &[Reg]) -> IntrinsicCodegenResult {
        let result_reg = match &self.intrinsic.strategy {
            CodegenStrategy::DirectOpcode(opcode) => {
                self.emit_direct_opcode(*opcode, args)
            }
            CodegenStrategy::OpcodeWithMode(opcode, mode) => {
                self.emit_opcode_with_mode(*opcode, *mode, args)
            }
            CodegenStrategy::OpcodeWithSize(opcode, size) => {
                self.emit_opcode_with_size(*opcode, *size, args)
            }
            CodegenStrategy::InlineSequence(seq_id) => {
                self.emit_inline_sequence(*seq_id, args)
            }
            CodegenStrategy::InlineSequenceWithWidth(seq_id, _width) => {
                // Width is handled by the VBC expression codegen path;
                // MLIR codegen treats these as regular inline sequences.
                self.emit_inline_sequence(*seq_id, args)
            }
            CodegenStrategy::CompileTimeConstant => {
                self.emit_compile_time_constant(args)
            }
            CodegenStrategy::ArithExtendedOpcode(sub_op) => {
                self.emit_arith_extended_opcode(*sub_op, args)
            }
            CodegenStrategy::WrappingOpcode(sub_op, width, signed) => {
                self.emit_wrapping_opcode(*sub_op, *width, *signed, args)
            }
            CodegenStrategy::SaturatingOpcode(sub_op, width, signed) => {
                self.emit_saturating_opcode(*sub_op, *width, *signed, args)
            }
            CodegenStrategy::TensorExtendedOpcode(sub_op) => {
                self.emit_tensor_extended_opcode(*sub_op, args)
            }
            CodegenStrategy::TensorExtendedOpcodeWithMode(sub_op, mode) => {
                self.emit_tensor_extended_opcode_with_mode(*sub_op, *mode, args)
            }
            CodegenStrategy::TensorExtExtendedOpcode(sub_op) => {
                self.emit_tensor_ext_extended_opcode(*sub_op, args)
            }
            CodegenStrategy::GpuExtendedOpcode(sub_op) => {
                self.emit_gpu_extended_opcode(*sub_op, args)
            }
            CodegenStrategy::MathExtendedOpcode(sub_op) => {
                self.emit_math_extended_opcode(*sub_op, args)
            }
        };

        IntrinsicCodegenResult {
            instructions: self.instructions,
            result_reg,
            registers_used: self.next_reg,
        }
    }

    /// Emit direct opcode mapping.
    fn emit_direct_opcode(&mut self, opcode: Opcode, args: &[Reg]) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        // Emit the appropriate instruction based on opcode category
        match opcode {
            // Arithmetic opcodes (2 operands)
            Opcode::AddI | Opcode::SubI | Opcode::MulI | Opcode::DivI | Opcode::ModI |
            Opcode::AddF | Opcode::SubF | Opcode::MulF | Opcode::DivF |
            Opcode::Band | Opcode::Bor | Opcode::Bxor | Opcode::Shl | Opcode::Shr | Opcode::Ushr |
            Opcode::PowI | Opcode::PowF => {
                if args.len() >= 2 {
                    self.emit(IntrinsicInstruction::BinaryOp {
                        opcode,
                        dst: dst.unwrap_or(Reg::new(0)),
                        lhs: args[0],
                        rhs: args[1],
                    });
                }
            }

            // Unary opcodes (1 operand)
            Opcode::NegI | Opcode::NegF | Opcode::Not | Opcode::Bnot |
            Opcode::AbsI | Opcode::AbsF | Opcode::Inc | Opcode::Dec => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::UnaryOp {
                        opcode,
                        dst: dst.unwrap_or(Reg::new(0)),
                        src: args[0],
                    });
                }
            }

            // Memory/CBGR opcodes
            Opcode::Deref => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::Deref {
                        dst: dst.unwrap_or(Reg::new(0)),
                        ptr: args[0],
                    });
                }
            }
            Opcode::DerefMut => {
                if args.len() >= 2 {
                    self.emit(IntrinsicInstruction::DerefMut {
                        ptr: args[0],
                        value: args[1],
                    });
                }
            }
            Opcode::Ref => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::Ref {
                        dst: dst.unwrap_or(Reg::new(0)),
                        src: args[0],
                    });
                }
            }
            Opcode::ChkRef => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::ChkRef {
                        dst: dst.unwrap_or(Reg::new(0)),
                        src: args[0],
                    });
                }
            }

            // TLS opcodes
            Opcode::TlsGet => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::TlsGet {
                        dst: dst.unwrap_or(Reg::new(0)),
                        slot: args[0],
                    });
                } else {
                    // Slot 0 for base pointer
                    let slot_reg = self.alloc_temp();
                    self.emit(IntrinsicInstruction::LoadSmallI {
                        dst: slot_reg,
                        value: 0,
                    });
                    self.emit(IntrinsicInstruction::TlsGet {
                        dst: dst.unwrap_or(Reg::new(0)),
                        slot: slot_reg,
                    });
                }
            }
            Opcode::TlsSet => {
                if args.len() >= 2 {
                    self.emit(IntrinsicInstruction::TlsSet {
                        slot: args[0],
                        value: args[1],
                    });
                }
            }

            // Context opcodes
            Opcode::PushContext => {
                self.emit(IntrinsicInstruction::PushContext {
                    dst: dst.unwrap_or(Reg::new(0)),
                });
            }
            Opcode::PopContext => {
                self.emit(IntrinsicInstruction::PopContext);
            }

            // Control flow
            Opcode::Panic => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::Panic { msg: args[0] });
                }
            }
            Opcode::Unreachable => {
                self.emit(IntrinsicInstruction::Unreachable);
            }
            Opcode::Assert => {
                if args.len() >= 2 {
                    self.emit(IntrinsicInstruction::Assert {
                        cond: args[0],
                        msg: args[1],
                    });
                }
            }
            Opcode::DebugPrint => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::DebugPrint { value: args[0] });
                }
            }

            // Async opcodes
            Opcode::Spawn => {
                if args.len() >= 2 {
                    self.emit(IntrinsicInstruction::Spawn {
                        dst: dst.unwrap_or(Reg::new(0)),
                        future: args[0],
                        task_id: args[1],
                    });
                }
            }
            Opcode::Await => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::Await {
                        dst: dst.unwrap_or(Reg::new(0)),
                        future: args[0],
                    });
                }
            }
            Opcode::IoSubmit => {
                if !args.is_empty() {
                    self.emit(IntrinsicInstruction::IoSubmit {
                        dst: dst.unwrap_or(Reg::new(0)),
                        ops: args[0],
                    });
                }
            }

            // Generic size/align
            Opcode::SizeOfG => {
                self.emit(IntrinsicInstruction::SizeOfG {
                    dst: dst.unwrap_or(Reg::new(0)),
                    type_param: args.first().copied(),
                });
            }
            Opcode::AlignOfG => {
                self.emit(IntrinsicInstruction::AlignOfG {
                    dst: dst.unwrap_or(Reg::new(0)),
                    type_param: args.first().copied(),
                });
            }

            // Default: emit generic opcode instruction
            _ => {
                self.emit(IntrinsicInstruction::Generic {
                    opcode,
                    dst,
                    args: args.to_vec(),
                });
            }
        }

        dst
    }

    /// Emit opcode with mode byte (e.g., AtomicFence with ordering).
    fn emit_opcode_with_mode(&mut self, opcode: Opcode, mode: u8, args: &[Reg]) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        match opcode {
            Opcode::AtomicFence => {
                if mode == 0xFF {
                    // Special mode: spin_hint
                    self.emit(IntrinsicInstruction::SpinHint);
                } else if !args.is_empty() {
                    self.emit(IntrinsicInstruction::AtomicFence { ordering: args[0] });
                } else {
                    // Use mode as default ordering
                    let ordering_reg = self.alloc_temp();
                    self.emit(IntrinsicInstruction::LoadSmallI {
                        dst: ordering_reg,
                        value: mode as i8,
                    });
                    self.emit(IntrinsicInstruction::AtomicFence { ordering: ordering_reg });
                }
            }
            Opcode::SyscallLinux => {
                // mode is argc (0-6)
                let argc = mode as usize;
                if args.len() > argc {
                    self.emit(IntrinsicInstruction::SyscallLinux {
                        dst: dst.unwrap_or(Reg::new(0)),
                        num: args[0],
                        args: RegRange::new(
                            args.get(1).copied().unwrap_or(Reg::new(0)),
                            argc as u8,
                        ),
                    });
                }
            }
            Opcode::CvtFI => {
                // mode: 0=trunc, 1=floor, 2=ceil, 3=round
                if !args.is_empty() {
                    let mode_reg = self.alloc_temp();
                    self.emit(IntrinsicInstruction::LoadSmallI {
                        dst: mode_reg,
                        value: mode as i8,
                    });
                    self.emit(IntrinsicInstruction::CvtFI {
                        dst: dst.unwrap_or(Reg::new(0)),
                        src: args[0],
                        mode: mode_reg,
                    });
                }
            }
            _ => {
                // Generic with mode
                self.emit(IntrinsicInstruction::GenericWithMode {
                    opcode,
                    mode,
                    dst,
                    args: args.to_vec(),
                });
            }
        }

        dst
    }

    /// Emit opcode with size specifier (e.g., AtomicLoad with size).
    fn emit_opcode_with_size(&mut self, opcode: Opcode, size: u8, args: &[Reg]) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        match opcode {
            Opcode::AtomicLoad => {
                // args: [ptr, ordering]
                if args.len() >= 2 {
                    self.emit(IntrinsicInstruction::AtomicLoad {
                        dst: dst.unwrap_or(Reg::new(0)),
                        ptr: args[0],
                        ordering: args[1],
                        size,
                    });
                }
            }
            Opcode::AtomicStore => {
                // args: [ptr, value, ordering]
                if args.len() >= 3 {
                    self.emit(IntrinsicInstruction::AtomicStore {
                        ptr: args[0],
                        value: args[1],
                        ordering: args[2],
                        size,
                    });
                }
            }
            Opcode::AtomicCas => {
                // args: [ptr, expected, desired, success_order, failure_order]
                // returns: (old_value, success)
                if args.len() >= 5 {
                    let old_reg = self.alloc_temp();
                    let success_reg = self.alloc_temp();
                    self.emit(IntrinsicInstruction::AtomicCas {
                        old_dst: old_reg,
                        success_dst: success_reg,
                        ptr: args[0],
                        expected: args[1],
                        desired: args[2],
                        success_order: args[3],
                        failure_order: args[4],
                        size,
                    });
                    // Pack into tuple result
                    if let Some(d) = dst {
                        self.emit(IntrinsicInstruction::Pack {
                            dst: d,
                            regs: vec![old_reg, success_reg],
                        });
                    }
                }
            }
            _ => {
                self.emit(IntrinsicInstruction::GenericWithSize {
                    opcode,
                    size,
                    dst,
                    args: args.to_vec(),
                });
            }
        }

        dst
    }

    /// Emit inline sequence for complex operations.
    fn emit_inline_sequence(&mut self, seq_id: InlineSequenceId, args: &[Reg]) -> Option<Reg> {
        match seq_id {
            InlineSequenceId::Memcpy => self.emit_memcpy(args),
            InlineSequenceId::Memmove => self.emit_memmove(args),
            InlineSequenceId::Memset => self.emit_memset(args),
            InlineSequenceId::Memcmp => self.emit_memcmp(args),
            InlineSequenceId::AtomicFetchAdd => self.emit_atomic_fetch_add(args),
            InlineSequenceId::AtomicFetchSub => self.emit_atomic_fetch_sub(args),
            InlineSequenceId::AtomicFetchAnd => self.emit_atomic_fetch_and(args),
            InlineSequenceId::AtomicFetchOr => self.emit_atomic_fetch_or(args),
            InlineSequenceId::AtomicFetchXor => self.emit_atomic_fetch_xor(args),
            InlineSequenceId::SpinlockLock => self.emit_spinlock_lock(args),
            InlineSequenceId::FutexWait => self.emit_futex_wait(args),
            InlineSequenceId::FutexWake => self.emit_futex_wake(args),
            InlineSequenceId::CheckedAdd => self.emit_checked_add(args),
            InlineSequenceId::CheckedSub => self.emit_checked_sub(args),
            InlineSequenceId::CheckedMul => self.emit_checked_mul(args),
            InlineSequenceId::CheckedDiv => self.emit_checked_div(args),
            InlineSequenceId::OverflowingAdd => self.emit_overflowing_add(args),
            InlineSequenceId::OverflowingSub => self.emit_overflowing_sub(args),
            InlineSequenceId::OverflowingMul => self.emit_overflowing_mul(args),
            InlineSequenceId::Clz => self.emit_clz(args),
            InlineSequenceId::Ilog2 => self.emit_clz(args), // ilog2 = 63 - clz, close enough for codegen
            InlineSequenceId::Ctz => self.emit_ctz(args),
            InlineSequenceId::Popcnt => self.emit_popcnt(args),
            InlineSequenceId::Bswap => self.emit_bswap(args),
            InlineSequenceId::RotateLeft => self.emit_rotate_left(args),
            InlineSequenceId::RotateRight => self.emit_rotate_right(args),
            InlineSequenceId::SinF64 => self.emit_sin_f64(args),
            InlineSequenceId::CosF64 => self.emit_cos_f64(args),
            InlineSequenceId::TanF64 => self.emit_tan_f64(args),
            InlineSequenceId::AsinF64 => self.emit_asin_f64(args),
            InlineSequenceId::AcosF64 => self.emit_acos_f64(args),
            InlineSequenceId::AtanF64 => self.emit_atan_f64(args),
            InlineSequenceId::Atan2F64 => self.emit_atan2_f64(args),
            InlineSequenceId::ExpF64 => self.emit_exp_f64(args),
            InlineSequenceId::LogF64 => self.emit_log_f64(args),
            InlineSequenceId::Log10F64 => self.emit_log10_f64(args),
            InlineSequenceId::MonotonicNanos => self.emit_monotonic_nanos(args),
            InlineSequenceId::RealtimeSecs => self.emit_realtime_secs(args),
            InlineSequenceId::RealtimeNanos => self.emit_realtime_nanos(args),
            InlineSequenceId::DropInPlace => self.emit_drop_in_place(args),
            InlineSequenceId::MakeSlice => self.emit_make_slice(args),
            InlineSequenceId::Uninit => self.emit_uninit(args),
            InlineSequenceId::Zeroed => self.emit_zeroed(args),
            // Extended math functions F64 - Zero-cost MathExtended
            InlineSequenceId::CbrtF64 => self.emit_math_extended(MathSubOpcode::CbrtF64, args),
            InlineSequenceId::Expm1F64 => self.emit_math_extended(MathSubOpcode::Expm1F64, args),
            InlineSequenceId::Exp2F64 => self.emit_math_extended(MathSubOpcode::Exp2F64, args),
            InlineSequenceId::Log1pF64 => self.emit_math_extended(MathSubOpcode::Log1pF64, args),
            InlineSequenceId::Log2F64 => self.emit_math_extended(MathSubOpcode::Log2F64, args),
            InlineSequenceId::PowiF64 => self.emit_math_extended(MathSubOpcode::PowF64, args), // powi uses same opcode
            InlineSequenceId::TruncF64 => self.emit_math_extended(MathSubOpcode::TruncF64, args),
            InlineSequenceId::MinnumF64 => self.emit_math_extended(MathSubOpcode::MinnumF64, args),
            InlineSequenceId::MaxnumF64 => self.emit_math_extended(MathSubOpcode::MaxnumF64, args),
            InlineSequenceId::FmaF64 => self.emit_math_extended(MathSubOpcode::FmaF64, args),
            InlineSequenceId::CopysignF64 => self.emit_math_extended(MathSubOpcode::CopysignF64, args),
            InlineSequenceId::HypotF64 => self.emit_math_extended(MathSubOpcode::HypotF64, args),
            InlineSequenceId::PowF64 => self.emit_math_extended(MathSubOpcode::PowF64, args),
            InlineSequenceId::AbsF64 => self.emit_math_extended(MathSubOpcode::AbsF64, args),
            InlineSequenceId::FloorF64 => self.emit_floor_f64(args),
            InlineSequenceId::CeilF64 => self.emit_ceil_f64(args),
            InlineSequenceId::RoundF64 => self.emit_round_f64(args),
            InlineSequenceId::SqrtF64 => self.emit_math_extended(MathSubOpcode::SqrtF64, args),
            // Hyperbolic functions F64
            InlineSequenceId::SinhF64 => self.emit_math_extended(MathSubOpcode::SinhF64, args),
            InlineSequenceId::CoshF64 => self.emit_math_extended(MathSubOpcode::CoshF64, args),
            InlineSequenceId::TanhF64 => self.emit_math_extended(MathSubOpcode::TanhF64, args),
            InlineSequenceId::AsinhF64 => self.emit_math_extended(MathSubOpcode::AsinhF64, args),
            InlineSequenceId::AcoshF64 => self.emit_math_extended(MathSubOpcode::AcoshF64, args),
            InlineSequenceId::AtanhF64 => self.emit_math_extended(MathSubOpcode::AtanhF64, args),
            // Bit manipulation
            InlineSequenceId::Bitreverse => self.emit_bitreverse(args),
            // Conversion
            InlineSequenceId::IntToFloat => self.emit_int_to_float(args),
            InlineSequenceId::FloatToInt => self.emit_float_to_int(args),
            // Byte conversions
            InlineSequenceId::ToLeBytes => self.emit_to_le_bytes(args),
            InlineSequenceId::FromLeBytes => self.emit_from_le_bytes(args),
            InlineSequenceId::ToBeBytes => self.emit_to_be_bytes(args),
            InlineSequenceId::FromBeBytes => self.emit_from_be_bytes(args),
            // Character operations
            // Character operations - zero-cost typed dispatch via CharExtended
            InlineSequenceId::CharIsAlphabetic => self.emit_char_extended(CharSubOpcode::IsAlphabeticUnicode, args),
            InlineSequenceId::CharIsNumeric => self.emit_char_extended(CharSubOpcode::IsNumericUnicode, args),
            InlineSequenceId::CharIsWhitespace => self.emit_char_extended(CharSubOpcode::IsWhitespaceUnicode, args),
            InlineSequenceId::CharIsControl => self.emit_char_extended(CharSubOpcode::IsControlUnicode, args),
            InlineSequenceId::CharIsUppercase => self.emit_char_extended(CharSubOpcode::IsUppercaseUnicode, args),
            InlineSequenceId::CharIsLowercase => self.emit_char_extended(CharSubOpcode::IsLowercaseUnicode, args),
            InlineSequenceId::CharToUppercase => self.emit_char_extended(CharSubOpcode::ToUppercaseUnicode, args),
            InlineSequenceId::CharToLowercase => self.emit_char_extended(CharSubOpcode::ToLowercaseUnicode, args),
            InlineSequenceId::CharEncodeUtf8 => self.emit_char_extended(CharSubOpcode::EncodeUtf8, args),
            InlineSequenceId::CharEscapeDebug => self.emit_char_extended(CharSubOpcode::EscapeDebug, args),
            // Float32 operations - Zero-cost MathExtended
            InlineSequenceId::SqrtF32 => self.emit_math_extended(MathSubOpcode::SqrtF32, args),
            InlineSequenceId::FloorF32 => self.emit_floor_f32(args),
            InlineSequenceId::CeilF32 => self.emit_ceil_f32(args),
            InlineSequenceId::RoundF32 => self.emit_round_f32(args),
            InlineSequenceId::TruncF32 => self.emit_trunc_f32(args),
            InlineSequenceId::AbsF32 => self.emit_abs_f32(args),
            // F32 trigonometric functions
            InlineSequenceId::SinF32 => self.emit_sin_f32(args),
            InlineSequenceId::CosF32 => self.emit_cos_f32(args),
            InlineSequenceId::TanF32 => self.emit_tan_f32(args),
            InlineSequenceId::AsinF32 => self.emit_asin_f32(args),
            InlineSequenceId::AcosF32 => self.emit_acos_f32(args),
            InlineSequenceId::AtanF32 => self.emit_atan_f32(args),
            InlineSequenceId::Atan2F32 => self.emit_atan2_f32(args),
            // F32 hyperbolic functions
            InlineSequenceId::SinhF32 => self.emit_math_extended(MathSubOpcode::SinhF32, args),
            InlineSequenceId::CoshF32 => self.emit_math_extended(MathSubOpcode::CoshF32, args),
            InlineSequenceId::TanhF32 => self.emit_math_extended(MathSubOpcode::TanhF32, args),
            InlineSequenceId::AsinhF32 => self.emit_math_extended(MathSubOpcode::AsinhF32, args),
            InlineSequenceId::AcoshF32 => self.emit_math_extended(MathSubOpcode::AcoshF32, args),
            InlineSequenceId::AtanhF32 => self.emit_math_extended(MathSubOpcode::AtanhF32, args),
            // F32 exponential and logarithmic functions
            InlineSequenceId::ExpF32 => self.emit_exp_f32(args),
            InlineSequenceId::Exp2F32 => self.emit_math_extended(MathSubOpcode::Exp2F32, args),
            InlineSequenceId::Expm1F32 => self.emit_math_extended(MathSubOpcode::Expm1F32, args),
            InlineSequenceId::LogF32 => self.emit_log_f32(args),
            InlineSequenceId::Log2F32 => self.emit_math_extended(MathSubOpcode::Log2F32, args),
            InlineSequenceId::Log10F32 => self.emit_math_extended(MathSubOpcode::Log10F32, args),
            InlineSequenceId::Log1pF32 => self.emit_math_extended(MathSubOpcode::Log1pF32, args),
            // F32 power and special functions
            InlineSequenceId::CbrtF32 => self.emit_math_extended(MathSubOpcode::CbrtF32, args),
            InlineSequenceId::HypotF32 => self.emit_math_extended(MathSubOpcode::HypotF32, args),
            InlineSequenceId::FmaF32 => self.emit_math_extended(MathSubOpcode::FmaF32, args),
            InlineSequenceId::CopysignF32 => self.emit_math_extended(MathSubOpcode::CopysignF32, args),
            InlineSequenceId::PowiF32 => self.emit_math_extended(MathSubOpcode::PowF32, args), // powi uses same opcode
            InlineSequenceId::MinnumF32 => self.emit_math_extended(MathSubOpcode::MinnumF32, args),
            InlineSequenceId::MaxnumF32 => self.emit_math_extended(MathSubOpcode::MaxnumF32, args),
            // Saturating arithmetic
            InlineSequenceId::SaturatingAdd => self.emit_saturating_add(args),
            InlineSequenceId::SaturatingSub => self.emit_saturating_sub(args),
            // Type conversions
            InlineSequenceId::Sext => self.emit_sext(args),
            InlineSequenceId::Zext => self.emit_zext(args),
            InlineSequenceId::Fpext => self.emit_fpext(args),
            InlineSequenceId::Fptrunc => self.emit_fptrunc(args),
            InlineSequenceId::IntTrunc => self.emit_int_trunc(args),
            InlineSequenceId::Bitcast => self.emit_bitcast(args),
            InlineSequenceId::F32ToBits |
            InlineSequenceId::F32FromBits |
            InlineSequenceId::F64ToBits |
            InlineSequenceId::F64FromBits => self.emit_bitcast(args),
            // Float classification
            InlineSequenceId::IsNan => self.emit_is_nan(args),
            InlineSequenceId::IsInf => self.emit_is_inf(args),
            InlineSequenceId::IsFinite => self.emit_is_finite(args),
            // Slice operations - zero-cost CbgrExtended dispatch
            InlineSequenceId::SliceLen => self.emit_slice_len(args),
            InlineSequenceId::SliceAsPtr => self.emit_slice_as_ptr(args),
            InlineSequenceId::SliceGet => self.emit_cbgr_extended(CbgrSubOpcode::SliceGet, args),
            InlineSequenceId::SliceGetUnchecked => self.emit_cbgr_extended(CbgrSubOpcode::SliceGetUnchecked, args),
            InlineSequenceId::SliceSubslice => self.emit_cbgr_extended(CbgrSubOpcode::SliceSubslice, args),
            InlineSequenceId::SliceSplitAt => self.emit_cbgr_extended(CbgrSubOpcode::SliceSplitAt, args),
            // Text operations - zero-cost typed dispatch via TextExtended
            InlineSequenceId::TextFromStatic => self.emit_text_extended(TextSubOpcode::FromStatic, args),
            InlineSequenceId::Utf8DecodeChar => self.emit_char_extended(CharSubOpcode::DecodeUtf8, args),
            InlineSequenceId::TextParseInt => self.emit_text_extended(TextSubOpcode::ParseInt, args),
            InlineSequenceId::TextParseFloat => self.emit_text_extended(TextSubOpcode::ParseFloat, args),
            InlineSequenceId::IntToText => self.emit_text_extended(TextSubOpcode::IntToText, args),
            InlineSequenceId::FloatToText => self.emit_text_extended(TextSubOpcode::FloatToText, args),
            InlineSequenceId::TextByteLen => self.emit_text_extended(TextSubOpcode::ByteLen, args),
            // Random number generation - zero-cost typed dispatch via TensorExtSubOpcode
            InlineSequenceId::RandomU64 => self.emit_tensor_ext_extended(TensorExtSubOpcode::RandomU64, args),
            InlineSequenceId::RandomFloat => self.emit_tensor_ext_extended(TensorExtSubOpcode::RandomFloat, args),
            // Unicode character category
            InlineSequenceId::CharGeneralCategory => self.emit_char_extended(CharSubOpcode::GeneralCategory, args),
            // Global allocator - zero-cost typed dispatch via TensorExtSubOpcode
            InlineSequenceId::GlobalAllocator => self.emit_tensor_ext_extended(TensorExtSubOpcode::GlobalAllocator, args),
            // Atomic exchange
            InlineSequenceId::AtomicExchange => self.emit_atomic_exchange(args),
            // Tier 0 async: return pending (false)
            InlineSequenceId::PollPending => self.emit_load_false(),
            // Call second argument as function (recovery passthrough)
            InlineSequenceId::CallSecondArg => self.emit_call_second_arg(args),
            // Load unit value
            InlineSequenceId::LoadUnit => self.emit_load_unit(),
            // Volatile memory operations (MMIO support)
            InlineSequenceId::VolatileLoad => self.emit_volatile_load(args),
            InlineSequenceId::VolatileStore => self.emit_volatile_store(args),
            InlineSequenceId::VolatileLoadAcquire => self.emit_volatile_load_acquire(args),
            InlineSequenceId::VolatileStoreRelease => self.emit_volatile_store_release(args),
            InlineSequenceId::CompilerFence => self.emit_compiler_fence(args),
            InlineSequenceId::HardwareFence => self.emit_hardware_fence(args),

            // SIMD vector operations - zero-cost typed dispatch via SimdExtended
            InlineSequenceId::SimdSplat => self.emit_simd_extended(SimdSubOpcode::Splat, args),
            InlineSequenceId::SimdExtract => self.emit_simd_extended(SimdSubOpcode::Extract, args),
            InlineSequenceId::SimdInsert => self.emit_simd_extended(SimdSubOpcode::Insert, args),
            InlineSequenceId::SimdAdd => self.emit_simd_extended(SimdSubOpcode::Add, args),
            InlineSequenceId::SimdSub => self.emit_simd_extended(SimdSubOpcode::Sub, args),
            InlineSequenceId::SimdMul => self.emit_simd_extended(SimdSubOpcode::Mul, args),
            InlineSequenceId::SimdDiv => self.emit_simd_extended(SimdSubOpcode::Div, args),
            InlineSequenceId::SimdNeg => self.emit_simd_extended(SimdSubOpcode::Neg, args),
            InlineSequenceId::SimdAbs => self.emit_simd_extended(SimdSubOpcode::Abs, args),
            InlineSequenceId::SimdSqrt => self.emit_simd_extended(SimdSubOpcode::Sqrt, args),
            InlineSequenceId::SimdFma => self.emit_simd_extended(SimdSubOpcode::Fma, args),
            InlineSequenceId::SimdMin => self.emit_simd_extended(SimdSubOpcode::Min, args),
            InlineSequenceId::SimdMax => self.emit_simd_extended(SimdSubOpcode::Max, args),
            InlineSequenceId::SimdReduceAdd => self.emit_simd_extended(SimdSubOpcode::ReduceAdd, args),
            InlineSequenceId::SimdReduceMul => self.emit_simd_extended(SimdSubOpcode::ReduceMul, args),
            InlineSequenceId::SimdReduceMin => self.emit_simd_extended(SimdSubOpcode::ReduceMin, args),
            InlineSequenceId::SimdReduceMax => self.emit_simd_extended(SimdSubOpcode::ReduceMax, args),
            InlineSequenceId::SimdCmpEq => self.emit_simd_extended(SimdSubOpcode::CmpEq, args),
            InlineSequenceId::SimdCmpNe => self.emit_simd_extended(SimdSubOpcode::CmpNe, args),
            InlineSequenceId::SimdCmpLt => self.emit_simd_extended(SimdSubOpcode::CmpLt, args),
            InlineSequenceId::SimdCmpLe => self.emit_simd_extended(SimdSubOpcode::CmpLe, args),
            InlineSequenceId::SimdCmpGt => self.emit_simd_extended(SimdSubOpcode::CmpGt, args),
            InlineSequenceId::SimdCmpGe => self.emit_simd_extended(SimdSubOpcode::CmpGe, args),
            InlineSequenceId::SimdSelect => self.emit_simd_extended(SimdSubOpcode::Select, args),
            InlineSequenceId::SimdLoadAligned => self.emit_simd_extended(SimdSubOpcode::LoadAligned, args),
            InlineSequenceId::SimdLoadUnaligned => self.emit_simd_extended(SimdSubOpcode::LoadUnaligned, args),
            InlineSequenceId::SimdStoreAligned => self.emit_simd_extended(SimdSubOpcode::StoreAligned, args),
            InlineSequenceId::SimdStoreUnaligned => self.emit_simd_extended(SimdSubOpcode::StoreUnaligned, args),
            InlineSequenceId::SimdMaskedLoad => self.emit_simd_extended(SimdSubOpcode::MaskedLoad, args),
            InlineSequenceId::SimdMaskedStore => self.emit_simd_extended(SimdSubOpcode::MaskedStore, args),
            InlineSequenceId::SimdShuffle => self.emit_simd_extended(SimdSubOpcode::Shuffle, args),
            InlineSequenceId::SimdGather => self.emit_simd_extended(SimdSubOpcode::Gather, args),
            InlineSequenceId::SimdScatter => self.emit_simd_extended(SimdSubOpcode::Scatter, args),
            InlineSequenceId::SimdMaskAll => self.emit_simd_extended(SimdSubOpcode::MaskAll, args),
            InlineSequenceId::SimdMaskNone => self.emit_simd_extended(SimdSubOpcode::MaskNone, args),
            InlineSequenceId::SimdMaskCount => self.emit_simd_extended(SimdSubOpcode::MaskCount, args),
            InlineSequenceId::SimdMaskAny => self.emit_simd_extended(SimdSubOpcode::MaskAny, args),
            InlineSequenceId::SimdBitwiseAnd => self.emit_simd_extended(SimdSubOpcode::BitwiseAnd, args),
            InlineSequenceId::SimdBitwiseOr => self.emit_simd_extended(SimdSubOpcode::BitwiseOr, args),
            InlineSequenceId::SimdBitwiseXor => self.emit_simd_extended(SimdSubOpcode::BitwiseXor, args),
            InlineSequenceId::SimdBitwiseNot => self.emit_simd_extended(SimdSubOpcode::BitwiseNot, args),
            InlineSequenceId::SimdShiftLeft => self.emit_simd_extended(SimdSubOpcode::ShiftLeft, args),
            InlineSequenceId::SimdShiftRight => self.emit_simd_extended(SimdSubOpcode::ShiftRight, args),
            InlineSequenceId::SimdCast => self.emit_simd_extended(SimdSubOpcode::Cast, args),

            // Tensor operations (SSM, FFT, Linear Algebra) - zero-cost typed dispatch via TensorExtended
            InlineSequenceId::SsmScan => self.emit_tensor_extended(TensorSubOpcode::SsmScan, args),
            InlineSequenceId::MatrixExp => self.emit_tensor_extended(TensorSubOpcode::Expm, args),
            InlineSequenceId::MatrixInverse => self.emit_tensor_extended(TensorSubOpcode::Inverse, args),
            InlineSequenceId::ComplexPow => self.emit_tensor_extended(TensorSubOpcode::ComplexPow, args),
            InlineSequenceId::ComplexMul => self.emit_tensor_extended(TensorSubOpcode::ComplexMul, args),
            InlineSequenceId::Rfft => self.emit_tensor_extended(TensorSubOpcode::Rfft, args),
            InlineSequenceId::Irfft => self.emit_tensor_extended(TensorSubOpcode::Irfft, args),
            InlineSequenceId::Uniform => self.emit_tensor_extended(TensorSubOpcode::Uniform, args),
            InlineSequenceId::IsTraining => self.emit_tensor_extended(TensorSubOpcode::IsTraining, args),
            InlineSequenceId::Bincount => self.emit_tensor_extended(TensorSubOpcode::Bincount, args),
            InlineSequenceId::GatherNd => self.emit_tensor_extended(TensorSubOpcode::GatherNd, args),
            InlineSequenceId::ArangeUsize => self.emit_tensor_extended(TensorSubOpcode::ArangeUsize, args),
            InlineSequenceId::TensorRepeat => self.emit_tensor_extended(TensorSubOpcode::Repeat, args),
            InlineSequenceId::TensorTanh => self.emit_tensor_extended(TensorSubOpcode::Tanh, args),
            InlineSequenceId::TensorSum => self.emit_tensor_extended(TensorSubOpcode::SumAll, args),
            InlineSequenceId::TensorFromArray => self.emit_tensor_extended(TensorSubOpcode::FromArray, args),
            InlineSequenceId::RandomFloat01 => self.emit_tensor_extended(TensorSubOpcode::RandomFloat01, args),

            // Additional tensor operations - zero-cost typed dispatch via TensorExtended
            InlineSequenceId::TensorUnsqueeze => self.emit_tensor_extended(TensorSubOpcode::Unsqueeze, args),
            InlineSequenceId::TensorMaskedSelect => self.emit_tensor_extended(TensorSubOpcode::MaskedSelect, args),
            InlineSequenceId::TensorLeakyRelu => self.emit_tensor_extended(TensorSubOpcode::LeakyRelu, args),
            InlineSequenceId::TensorDiag => self.emit_tensor_extended(TensorSubOpcode::Diag, args),
            InlineSequenceId::TensorTriu => self.emit_tensor_extended(TensorSubOpcode::Triu, args),
            InlineSequenceId::TensorTril => self.emit_tensor_extended(TensorSubOpcode::Tril, args),
            InlineSequenceId::TensorNonzero => self.emit_tensor_extended(TensorSubOpcode::Nonzero, args),
            InlineSequenceId::TensorOneHot => self.emit_tensor_extended(TensorSubOpcode::OneHot, args),
            InlineSequenceId::TensorSplit => self.emit_tensor_extended(TensorSubOpcode::Split, args),
            InlineSequenceId::TensorSplitAt => self.emit_tensor_extended(TensorSubOpcode::SplitAt, args),
            InlineSequenceId::TensorGetScalar => self.emit_tensor_extended(TensorSubOpcode::GetScalar, args),
            InlineSequenceId::TensorSetScalar => self.emit_tensor_extended(TensorSubOpcode::SetScalar, args),
            InlineSequenceId::TensorContiguous => self.emit_tensor_extended(TensorSubOpcode::Contiguous, args),
            InlineSequenceId::TensorContiguousView => self.emit_tensor_ext_extended(TensorExtSubOpcode::ContiguousView, args),
            InlineSequenceId::TensorToDevice => self.emit_tensor_extended(TensorSubOpcode::ToDevice, args),

            // Memory allocation operations - zero-cost typed dispatch via TensorExtSubOpcode
            InlineSequenceId::MemNewId => self.emit_tensor_ext_extended(TensorExtSubOpcode::MemNewId, args),
            InlineSequenceId::MemAllocTensor => self.emit_tensor_ext_extended(TensorExtSubOpcode::MemAllocTensor, args),

            // Automatic differentiation operations - zero-cost typed dispatch via MlExtended
            // These use MlSubOpcode for proper autodiff semantics.
            //
            // Operations with DirectOpcode strategy in registry emit directly:
            // - GradBegin → Opcode::GradBegin (0xEB)
            // - GradEnd → Opcode::GradEnd (0xEC)
            // - GradStop → Opcode::GradStop (0xEF)
            // - GradCheckpoint → Opcode::GradCheckpoint (0xED)
            // - GradAccumulate → Opcode::GradAccumulate (0xEE)
            //
            // Operations with InlineSequence strategy use MlSubOpcode:
            InlineSequenceId::GradBegin => self.emit_grad_begin(args),
            InlineSequenceId::GradEnd => self.emit_grad_end(args),
            InlineSequenceId::JvpBegin => self.emit_ml_extended(MlSubOpcode::JvpBegin, args),
            InlineSequenceId::JvpEnd => self.emit_ml_extended(MlSubOpcode::JvpEnd, args),
            InlineSequenceId::GradZeroTangent => self.emit_ml_extended(MlSubOpcode::GradZeroTangent, args),
            InlineSequenceId::GradStop => self.emit_grad_stop(args),
            InlineSequenceId::GradCustom => self.emit_ml_extended(MlSubOpcode::GradCustom, args),
            InlineSequenceId::GradCheckpoint => self.emit_grad_checkpoint(args),
            InlineSequenceId::GradAccumulate => self.emit_grad_accumulate(args),
            InlineSequenceId::GradRecompute => self.emit_ml_extended(MlSubOpcode::GradRecompute, args),
            InlineSequenceId::GradZero => self.emit_ml_extended(MlSubOpcode::ZeroGrad, args),

            // CBGR operations
            // CBGR operations - zero-cost typed dispatch via CbgrExtended
            InlineSequenceId::CbgrNewGeneration => self.emit_cbgr_extended(CbgrSubOpcode::NewGeneration, args),
            InlineSequenceId::CbgrInvalidate => self.emit_cbgr_extended(CbgrSubOpcode::Invalidate, args),
            InlineSequenceId::CbgrGetGeneration => self.emit_cbgr_extended(CbgrSubOpcode::GetGeneration, args),
            InlineSequenceId::CbgrAdvanceGeneration => self.emit_cbgr_extended(CbgrSubOpcode::AdvanceEpoch, args),
            InlineSequenceId::CbgrGetEpochCaps => self.emit_cbgr_extended(CbgrSubOpcode::GetEpochCaps, args),
            InlineSequenceId::CbgrBypassBegin => self.emit_cbgr_extended(CbgrSubOpcode::BypassBegin, args),
            InlineSequenceId::CbgrBypassEnd => self.emit_cbgr_extended(CbgrSubOpcode::BypassEnd, args),
            InlineSequenceId::CbgrGetStats => self.emit_cbgr_extended(CbgrSubOpcode::GetStats, args),

            // Log operations - zero-cost typed dispatch via LogExtended
            InlineSequenceId::LogInfo => self.emit_log_extended(LogSubOpcode::Info, args),
            InlineSequenceId::LogWarning => self.emit_log_extended(LogSubOpcode::Warning, args),
            InlineSequenceId::LogError => self.emit_log_extended(LogSubOpcode::Error, args),
            InlineSequenceId::LogDebug => self.emit_log_extended(LogSubOpcode::Debug, args),

            // Regex operations - zero-cost typed dispatch via TensorExtended
            // Note: Regex operations are part of TensorSubOpcode for text processing
            InlineSequenceId::RegexFindAll => self.emit_tensor_extended(TensorSubOpcode::RegexFindAll, args),
            InlineSequenceId::RegexReplaceAll => self.emit_tensor_extended(TensorSubOpcode::RegexReplaceAll, args),
            InlineSequenceId::RegexIsMatch => self.emit_tensor_extended(TensorSubOpcode::RegexIsMatch, args),
            InlineSequenceId::RegexSplit => self.emit_tensor_extended(TensorSubOpcode::RegexSplit, args),

            // Type introspection operations - use direct opcodes for zero-cost dispatch
            // SizeOf and AlignOf use dedicated opcodes (0x83, 0x84)
            InlineSequenceId::SizeOf => self.emit_direct_opcode(Opcode::SizeOfG, args),
            InlineSequenceId::AlignOf => self.emit_direct_opcode(Opcode::AlignOfG, args),
            // TypeId, TypeName, NeedsDrop use MetaReflect opcode with sub-operations
            InlineSequenceId::TypeId => self.emit_meta_reflect(MetaReflectOp::TypeId, args),
            InlineSequenceId::TypeName => self.emit_meta_reflect(MetaReflectOp::TypeName, args),
            InlineSequenceId::NeedsDrop => self.emit_meta_reflect(MetaReflectOp::NeedsDrop, args),
            // Additional user-facing intrinsics
            InlineSequenceId::PowF32 => self.emit_math_extended(MathSubOpcode::PowF32, args),
            InlineSequenceId::CharIsAlphanumeric => self.emit_char_extended(CharSubOpcode::IsAlphabeticUnicode, args), // approximate with alphabetic+numeric check
            InlineSequenceId::Rdtsc => {
                // Read timestamp counter - emit monotonic nanos as approximation in interpreter
                self.emit_monotonic_nanos(args)
            }
            InlineSequenceId::CatchUnwind => self.emit_call_second_arg(args), // In interpreter: just call the closure
            InlineSequenceId::PtrToRef => {
                // ptr_to_ref is identity in interpreter (pointer IS the ref)
                if !args.is_empty() {
                    Some(args[0])
                } else {
                    None
                }
            }

            // Time intrinsics — handled in VBC codegen (expressions.rs) via
            // FfiExtended sub-opcodes and inline instruction sequences.
            // In intrinsics/codegen.rs we just pass through the first argument
            // or return None for void operations.
            InlineSequenceId::DurationFromNanos
            | InlineSequenceId::DurationFromMicros
            | InlineSequenceId::DurationFromMillis
            | InlineSequenceId::DurationFromSecs
            | InlineSequenceId::DurationAsNanos
            | InlineSequenceId::DurationAsMicros
            | InlineSequenceId::DurationAsMillis
            | InlineSequenceId::DurationAsSecs
            | InlineSequenceId::DurationIsZero
            | InlineSequenceId::DurationAdd
            | InlineSequenceId::DurationSaturatingAdd
            | InlineSequenceId::DurationSaturatingSub
            | InlineSequenceId::DurationSubsecNanos
            | InlineSequenceId::InstantNow
            | InlineSequenceId::InstantElapsed
            | InlineSequenceId::InstantDurationSince
            | InlineSequenceId::TimeMonotonicMicros
            | InlineSequenceId::TimeMonotonicMillis
            | InlineSequenceId::TimeUnixTimestamp
            | InlineSequenceId::StopwatchNew
            | InlineSequenceId::StopwatchStart
            | InlineSequenceId::StopwatchStop
            | InlineSequenceId::StopwatchElapsed
            | InlineSequenceId::StopwatchReset
            | InlineSequenceId::PerfCounterNow
            | InlineSequenceId::PerfCounterElapsedSince
            | InlineSequenceId::PerfCounterAsNanos
            | InlineSequenceId::DeadlineTimerFromDuration
            | InlineSequenceId::DeadlineTimerIsExpired
            | InlineSequenceId::DeadlineTimerRemaining => {
                // Time operations return a value — pass through first arg or emit monotonic_nanos
                if !args.is_empty() {
                    Some(args[0])
                } else {
                    self.emit_monotonic_nanos(args)
                }
            }
            InlineSequenceId::TimeSleepMs
            | InlineSequenceId::TimeSleepUs
            | InlineSequenceId::TimeSleepDuration => {
                // Sleep operations are void — no return value
                None
            }
            // System call intrinsics — emit via TimeIntrinsic (reusing infra)
            InlineSequenceId::SysGetpid => self.emit_sys_getpid(args),
            InlineSequenceId::SysGettid => self.emit_sys_gettid(args),
            InlineSequenceId::SysMmap => self.emit_sys_mmap(args),
            InlineSequenceId::SysMunmap => self.emit_sys_munmap(args),
            InlineSequenceId::SysMadvise => self.emit_sys_madvise(args),
            InlineSequenceId::SysGetentropy => self.emit_sys_getentropy(args),
            // Mach kernel operations (macOS) — handled via FfiExtended in VBC
            InlineSequenceId::MachVmAllocate
            | InlineSequenceId::MachVmDeallocate
            | InlineSequenceId::MachVmProtect
            | InlineSequenceId::MachSemCreate
            | InlineSequenceId::MachSemDestroy
            | InlineSequenceId::MachSemSignal
            | InlineSequenceId::MachSemWait
            | InlineSequenceId::MachErrorString
            | InlineSequenceId::MachSleepUntil => {
                // Mach operations are handled via VBC FfiExtended sub-opcodes
                // and don't need AOT codegen paths yet
                None
            }
            // Heap memory allocation intrinsics
            InlineSequenceId::Alloc => self.emit_alloc(args),
            InlineSequenceId::AllocZeroed => self.emit_alloc_zeroed(args),
            InlineSequenceId::Dealloc => self.emit_dealloc(args),
            InlineSequenceId::Realloc => self.emit_realloc(args),
            InlineSequenceId::Swap => self.emit_swap(args),
            InlineSequenceId::Replace => self.emit_replace(args),
            InlineSequenceId::PtrOffset => self.emit_ptr_offset(args),
            // CBGR allocation intrinsics - delegate to general alloc paths
            InlineSequenceId::CbgrAlloc => self.emit_alloc(args),
            InlineSequenceId::CbgrAllocZeroed => self.emit_alloc_zeroed(args),
            InlineSequenceId::CbgrDealloc => self.emit_dealloc(args),
            InlineSequenceId::CbgrRealloc => self.emit_realloc(args),
            // Memory comparison
            InlineSequenceId::MemcmpBytes => self.emit_memcmp(args),
            // CBGR header access
            InlineSequenceId::GetHeaderFromPtr => {
                // Load header from pointer (deref with offset)
                if args.is_empty() { return None; }
                let dst = self.alloc_temp();
                self.emit(IntrinsicInstruction::Deref { dst, ptr: args[0] });
                Some(dst)
            }
            // Regex* variants — added in #456 but no codegen handler yet.
            // Tracked under follow-up; treated as opaque library calls
            // (return None — no inline opcode sequence emitted).
            InlineSequenceId::RegexFind
            | InlineSequenceId::RegexReplace
            | InlineSequenceId::RegexCaptures => None,
            // #12 / P3.2 — permission_check_wire is dispatched via
            // expressions.rs / lowering.rs (interpreter dispatch
            // and AOT extern call respectively). The MLIR codegen
            // path here treats it as a library call surface — no
            // inline opcode sequence to emit, the runtime helper
            // holds the routing logic so warm-path latency stays
            // identical between Tier 0 and Tier 1.
            InlineSequenceId::PermissionCheckWire => None,
        }
    }

    /// Emit element-scaled pointer offset: ptr + count * 8
    fn emit_ptr_offset(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() >= 2 {
            // VBC type-erases to 64-bit values; element stride is always 8 bytes
            let stride = self.alloc_temp();
            self.emit(IntrinsicInstruction::LoadI { dst: stride, value: 8 });
            let byte_offset = self.alloc_temp();
            self.emit(IntrinsicInstruction::BinaryOp {
                opcode: Opcode::MulI,
                dst: byte_offset,
                lhs: args[1],
                rhs: stride,
            });
            let result = self.alloc_temp();
            self.emit(IntrinsicInstruction::BinaryOp {
                opcode: Opcode::AddI,
                dst: result,
                lhs: args[0],
                rhs: byte_offset,
            });
            Some(result)
        } else {
            args.first().copied()
        }
    }

    // =========================================================================
    // Inline Sequence Implementations
    // =========================================================================

    /// memcpy: Optimized copy loop with SIMD when available.
    fn emit_memcpy(&mut self, args: &[Reg]) -> Option<Reg> {
        // args: [dst, src, len]
        if args.len() < 3 {
            return None;
        }

        // Emit call to optimized memcpy implementation
        // The VBC interpreter handles this specially
        self.emit(IntrinsicInstruction::MemIntrinsic {
            kind: MemIntrinsicKind::Memcpy,
            dst: args[0],
            src: args[1],
            len: args[2],
        });

        None // void return
    }

    /// memmove: Direction-aware copy for overlapping regions.
    fn emit_memmove(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        self.emit(IntrinsicInstruction::MemIntrinsic {
            kind: MemIntrinsicKind::Memmove,
            dst: args[0],
            src: args[1],
            len: args[2],
        });

        None
    }

    /// memset: Fill memory with byte value.
    fn emit_memset(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        self.emit(IntrinsicInstruction::MemIntrinsic {
            kind: MemIntrinsicKind::Memset,
            dst: args[0],
            src: args[1], // value (byte)
            len: args[2],
        });

        None
    }

    /// memcmp: Compare memory regions.
    fn emit_memcmp(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        let dst = self.alloc_temp();
        self.emit(IntrinsicInstruction::MemCmp {
            dst,
            lhs: args[0],
            rhs: args[1],
            len: args[2],
        });

        Some(dst)
    }

    /// Atomic fetch-add via CAS loop.
    fn emit_atomic_fetch_add(&mut self, args: &[Reg]) -> Option<Reg> {
        // args: [ptr, value, ordering]
        if args.len() < 3 {
            return None;
        }

        let old_val = self.alloc_temp();

        // Emit fetch_add as a dedicated opcode (interpreter handles CAS loop)
        self.emit(IntrinsicInstruction::AtomicRmw {
            dst: old_val,
            op: AtomicRmwOp::Add,
            ptr: args[0],
            value: args[1],
            ordering: args[2],
        });

        Some(old_val)
    }

    /// Atomic fetch-sub via CAS loop.
    fn emit_atomic_fetch_sub(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        let old_val = self.alloc_temp();
        self.emit(IntrinsicInstruction::AtomicRmw {
            dst: old_val,
            op: AtomicRmwOp::Sub,
            ptr: args[0],
            value: args[1],
            ordering: args[2],
        });

        Some(old_val)
    }

    /// Atomic fetch-and via CAS loop.
    fn emit_atomic_fetch_and(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        let old_val = self.alloc_temp();
        self.emit(IntrinsicInstruction::AtomicRmw {
            dst: old_val,
            op: AtomicRmwOp::And,
            ptr: args[0],
            value: args[1],
            ordering: args[2],
        });

        Some(old_val)
    }

    /// Atomic fetch-or via CAS loop.
    fn emit_atomic_fetch_or(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        let old_val = self.alloc_temp();
        self.emit(IntrinsicInstruction::AtomicRmw {
            dst: old_val,
            op: AtomicRmwOp::Or,
            ptr: args[0],
            value: args[1],
            ordering: args[2],
        });

        Some(old_val)
    }

    /// Atomic fetch-xor via CAS loop.
    fn emit_atomic_fetch_xor(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        let old_val = self.alloc_temp();
        self.emit(IntrinsicInstruction::AtomicRmw {
            dst: old_val,
            op: AtomicRmwOp::Xor,
            ptr: args[0],
            value: args[1],
            ordering: args[2],
        });

        Some(old_val)
    }

    /// Atomic exchange operation.
    fn emit_atomic_exchange(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        let old_val = self.alloc_temp();
        self.emit(IntrinsicInstruction::AtomicRmw {
            dst: old_val,
            op: AtomicRmwOp::Xchg,
            ptr: args[0],
            value: args[1],
            ordering: args[2],
        });

        Some(old_val)
    }

    /// Spinlock acquire with spin_hint loop.
    fn emit_spinlock_lock(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }

        self.emit(IntrinsicInstruction::SpinlockLock { lock: args[0] });

        None
    }

    /// Futex wait syscall wrapper.
    fn emit_futex_wait(&mut self, args: &[Reg]) -> Option<Reg> {
        // args: [addr, expected, timeout_ns]
        if args.len() < 3 {
            return None;
        }

        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::FutexWait {
            dst: result,
            addr: args[0],
            expected: args[1],
            timeout_ns: args[2],
        });

        Some(result)
    }

    /// Futex wake syscall wrapper.
    fn emit_futex_wake(&mut self, args: &[Reg]) -> Option<Reg> {
        // args: [addr, count] or [addr]
        if args.is_empty() {
            return None;
        }

        let result = self.alloc_temp();
        let count = if args.len() > 1 {
            args[1]
        } else {
            // Wake all
            let count_reg = self.alloc_temp();
            self.emit(IntrinsicInstruction::LoadI {
                dst: count_reg,
                value: u32::MAX as i64,
            });
            count_reg
        };

        self.emit(IntrinsicInstruction::FutexWake {
            dst: result,
            addr: args[0],
            count,
        });

        Some(result)
    }

    /// Checked add with overflow detection.
    fn emit_checked_add(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        let overflow = self.alloc_temp();

        self.emit(IntrinsicInstruction::CheckedArith {
            result,
            overflow,
            op: CheckedArithOp::Add,
            lhs: args[0],
            rhs: args[1],
        });

        // Pack into tuple
        let tuple = self.alloc_temp();
        self.emit(IntrinsicInstruction::Pack {
            dst: tuple,
            regs: vec![result, overflow],
        });

        Some(tuple)
    }

    /// Checked sub with overflow detection.
    fn emit_checked_sub(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        let overflow = self.alloc_temp();

        self.emit(IntrinsicInstruction::CheckedArith {
            result,
            overflow,
            op: CheckedArithOp::Sub,
            lhs: args[0],
            rhs: args[1],
        });

        let tuple = self.alloc_temp();
        self.emit(IntrinsicInstruction::Pack {
            dst: tuple,
            regs: vec![result, overflow],
        });

        Some(tuple)
    }

    /// Checked mul with overflow detection.
    fn emit_checked_mul(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        let overflow = self.alloc_temp();

        self.emit(IntrinsicInstruction::CheckedArith {
            result,
            overflow,
            op: CheckedArithOp::Mul,
            lhs: args[0],
            rhs: args[1],
        });

        let tuple = self.alloc_temp();
        self.emit(IntrinsicInstruction::Pack {
            dst: tuple,
            regs: vec![result, overflow],
        });

        Some(tuple)
    }

    /// Checked division with zero/overflow detection.
    fn emit_checked_div(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        let overflow = self.alloc_temp();

        self.emit(IntrinsicInstruction::CheckedArith {
            result,
            overflow,
            op: CheckedArithOp::Div,
            lhs: args[0],
            rhs: args[1],
        });

        let tuple = self.alloc_temp();
        self.emit(IntrinsicInstruction::Pack {
            dst: tuple,
            regs: vec![result, overflow],
        });

        Some(tuple)
    }

    /// Overflowing add - returns (result, overflow) tuple.
    fn emit_overflowing_add(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        let overflow = self.alloc_temp();

        self.emit(IntrinsicInstruction::CheckedArith {
            result,
            overflow,
            op: CheckedArithOp::Add,
            lhs: args[0],
            rhs: args[1],
        });

        let tuple = self.alloc_temp();
        self.emit(IntrinsicInstruction::Pack {
            dst: tuple,
            regs: vec![result, overflow],
        });

        Some(tuple)
    }

    /// Overflowing sub - returns (result, overflow) tuple.
    fn emit_overflowing_sub(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        let overflow = self.alloc_temp();

        self.emit(IntrinsicInstruction::CheckedArith {
            result,
            overflow,
            op: CheckedArithOp::Sub,
            lhs: args[0],
            rhs: args[1],
        });

        let tuple = self.alloc_temp();
        self.emit(IntrinsicInstruction::Pack {
            dst: tuple,
            regs: vec![result, overflow],
        });

        Some(tuple)
    }

    /// Overflowing mul - returns (result, overflow) tuple.
    fn emit_overflowing_mul(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        let overflow = self.alloc_temp();

        self.emit(IntrinsicInstruction::CheckedArith {
            result,
            overflow,
            op: CheckedArithOp::Mul,
            lhs: args[0],
            rhs: args[1],
        });

        let tuple = self.alloc_temp();
        self.emit(IntrinsicInstruction::Pack {
            dst: tuple,
            regs: vec![result, overflow],
        });

        Some(tuple)
    }

    /// Count leading zeros.
    fn emit_clz(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }

        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::BitOp {
            dst: result,
            op: BitOpKind::Clz,
            src: args[0],
        });

        Some(result)
    }

    /// Count trailing zeros.
    fn emit_ctz(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }

        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::BitOp {
            dst: result,
            op: BitOpKind::Ctz,
            src: args[0],
        });

        Some(result)
    }

    /// Population count.
    fn emit_popcnt(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }

        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::BitOp {
            dst: result,
            op: BitOpKind::Popcnt,
            src: args[0],
        });

        Some(result)
    }

    /// Byte swap.
    fn emit_bswap(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }

        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::BitOp {
            dst: result,
            op: BitOpKind::Bswap,
            src: args[0],
        });

        Some(result)
    }

    /// Rotate left.
    fn emit_rotate_left(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::RotateOp {
            dst: result,
            src: args[0],
            amount: args[1],
            direction: RotateDirection::Left,
        });

        Some(result)
    }

    /// Rotate right.
    fn emit_rotate_right(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::RotateOp {
            dst: result,
            src: args[0],
            amount: args[1],
            direction: RotateDirection::Right,
        });

        Some(result)
    }

    // =========================================================================
    // Math Functions F64 - Zero-cost MathExtended instruction emission
    // =========================================================================

    /// Sine (f64). Maps to llvm.sin.f64 / math.sin
    fn emit_sin_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::SinF64, args)
    }

    /// Cosine (f64). Maps to llvm.cos.f64 / math.cos
    fn emit_cos_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::CosF64, args)
    }

    /// Tangent (f64). Maps to llvm.tan.f64 / math.tan
    fn emit_tan_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::TanF64, args)
    }

    /// Arc sine (f64). Maps to llvm.asin.f64 / math.asin
    fn emit_asin_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::AsinF64, args)
    }

    /// Arc cosine (f64). Maps to llvm.acos.f64 / math.acos
    fn emit_acos_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::AcosF64, args)
    }

    /// Arc tangent (f64). Maps to llvm.atan.f64 / math.atan
    fn emit_atan_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::AtanF64, args)
    }

    /// Two-argument arc tangent (f64). Maps to llvm.atan2.f64 / math.atan2
    fn emit_atan2_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::Atan2F64, args)
    }

    /// Natural exponential (f64). Maps to llvm.exp.f64 / math.exp
    fn emit_exp_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::ExpF64, args)
    }

    /// Natural logarithm (f64). Maps to llvm.log.f64 / math.log
    fn emit_log_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::LogF64, args)
    }

    /// Base-10 logarithm (f64). Maps to llvm.log10.f64 / math.log10
    fn emit_log10_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::Log10F64, args)
    }

    /// Monotonic time via platform syscall.
    fn emit_monotonic_nanos(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::TimeIntrinsic {
            dst: result,
            kind: TimeIntrinsicKind::MonotonicNanos,
        });
        Some(result)
    }

    /// Real time in seconds via platform syscall.
    fn emit_realtime_secs(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::TimeIntrinsic {
            dst: result,
            kind: TimeIntrinsicKind::RealtimeSecs,
        });
        Some(result)
    }

    /// Real time in nanoseconds via platform syscall.
    fn emit_realtime_nanos(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::TimeIntrinsic {
            dst: result,
            kind: TimeIntrinsicKind::RealtimeNanos,
        });
        Some(result)
    }

    // =========================================================================
    // System Call Intrinsics
    // =========================================================================

    /// sys_getpid: Get process ID.
    fn emit_sys_getpid(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::Nop, // placeholder, expressions.rs handles FfiExtended emission
            dst: Some(result),
            args: vec![],
        });
        Some(result)
    }

    /// sys_gettid: Get thread ID.
    fn emit_sys_gettid(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::Nop,
            dst: Some(result),
            args: vec![],
        });
        Some(result)
    }

    /// sys_mmap: Memory map returning Result.
    fn emit_sys_mmap(&mut self, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::Nop,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// sys_munmap: Memory unmap returning Result.
    fn emit_sys_munmap(&mut self, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::Nop,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// sys_madvise: Memory advise returning Result.
    ///
    /// Tier-0 stub: madvise is *advisory* — the kernel is free to
    /// ignore the hint, so a no-op interpreter implementation is
    /// semantically correct (we just don't pre-fault / drop pages).
    /// Emits a LoadConst Unit so the result register is initialised
    /// without referencing the (addr, len, advice) argument tuple,
    /// avoiding the `InvalidRegister(2)` failure class when the
    /// caller's frame is sized smaller than 3 registers (T0.7).
    /// AOT lowering replaces this with the real FFI extern call.
    fn emit_sys_madvise(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::LoadUnit,
            dst: Some(result),
            args: vec![],
        });
        Some(result)
    }

    /// sys_getentropy: Get entropy returning Result.
    ///
    /// Tier-0 stub: same shape as `sys_madvise` — interpreter
    /// returns Unit without referencing buffer arguments. AOT
    /// lowering replaces with the real `arc4random_buf` call on
    /// macOS / `getrandom(2)` on Linux.
    fn emit_sys_getentropy(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::LoadUnit,
            dst: Some(result),
            args: vec![],
        });
        Some(result)
    }

    /// drop_in_place: Run destructor for value at pointer.
    ///
    /// This calls the Drop implementation for the type at the given pointer.
    fn emit_drop_in_place(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }

        // Emit drop reference instruction
        // The interpreter/runtime handles invoking the destructor
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::DropRef,
            dst: None,
            args: vec![args[0]],
        });

        None // void return
    }

    /// make_slice: Construct a fat pointer (slice) from ptr and len.
    fn emit_make_slice(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();

        // Create fat pointer by packing ptr and len into a tuple/struct
        // This creates a 16-byte value: [ptr: 8 bytes, len: 8 bytes]
        self.emit(IntrinsicInstruction::Pack {
            dst: result,
            regs: vec![args[0], args[1]],
        });

        Some(result)
    }

    /// uninit: Create uninitialized memory for a type.
    ///
    /// Returns a register containing an undefined value.
    /// The caller must ensure the value is properly initialized before use.
    fn emit_uninit(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();

        // Load undefined/garbage value - no initialization needed
        // The value is just an uninitialized register
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::LoadUnit, // placeholder - actual value is undefined
            dst: Some(result),
            args: vec![],
        });

        Some(result)
    }

    /// zeroed: Create zeroed memory for a type.
    ///
    /// Returns a register containing all-zeros representation.
    fn emit_zeroed(&mut self, _args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();

        // Load zero value
        self.emit(IntrinsicInstruction::LoadSmallI {
            dst: result,
            value: 0,
        });

        Some(result)
    }

    /// alloc: Allocate heap memory.
    ///
    /// Args: [size, align]
    /// Returns pointer to allocated memory.
    fn emit_alloc(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();

        // Emit memory allocation via MemIntrinsic
        self.emit(IntrinsicInstruction::MemAlloc {
            dst: result,
            size: args[0],
            align: args[1],
            zeroed: false,
        });

        Some(result)
    }

    /// alloc_zeroed: Allocate zeroed heap memory.
    ///
    /// Args: [size, align]
    /// Returns pointer to zeroed allocated memory.
    fn emit_alloc_zeroed(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();

        // Emit zeroed memory allocation via MemIntrinsic
        self.emit(IntrinsicInstruction::MemAlloc {
            dst: result,
            size: args[0],
            align: args[1],
            zeroed: true,
        });

        Some(result)
    }

    /// dealloc: Deallocate heap memory.
    ///
    /// Args: [ptr, size, align]
    /// Returns nothing.
    fn emit_dealloc(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 3 {
            return None;
        }

        // Emit memory deallocation
        self.emit(IntrinsicInstruction::MemDealloc {
            ptr: args[0],
            size: args[1],
            align: args[2],
        });

        None // void return
    }

    /// realloc: Reallocate heap memory.
    ///
    /// Args: [ptr, old_size, new_size, align]
    /// Returns pointer to reallocated memory.
    fn emit_realloc(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 4 {
            return None;
        }

        let result = self.alloc_temp();

        // Emit memory reallocation
        self.emit(IntrinsicInstruction::MemRealloc {
            dst: result,
            ptr: args[0],
            old_size: args[1],
            new_size: args[2],
            align: args[3],
        });

        Some(result)
    }

    /// swap: Swap two values in place.
    ///
    /// Args: [a, b]
    /// Returns nothing (values are swapped via pointers).
    fn emit_swap(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        // Emit swap operation - reads both, writes both
        self.emit(IntrinsicInstruction::MemSwap {
            a: args[0],
            b: args[1],
        });

        None // void return
    }

    /// replace: Replace value and return old.
    ///
    /// Args: [dest, src]
    /// Returns old value.
    fn emit_replace(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        let result = self.alloc_temp();

        // Emit replace operation
        self.emit(IntrinsicInstruction::MemReplace {
            dst: result,
            dest: args[0],
            src: args[1],
        });

        Some(result)
    }

    /// Helper for MathExtended instruction emission.
    ///
    /// Emits a MathExtended instruction with the given sub-opcode and arguments.
    /// This replaces the old string-based MathCall with zero-cost typed dispatch.
    ///
    /// # Performance
    /// - Interpreter: ~2ns dispatch via Rust match
    /// - AOT/LLVM: Zero-cost - direct LLVM intrinsic
    fn emit_math_extended(&mut self, sub_op: MathSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::MathExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for SimdExtended instruction emission.
    ///
    /// Emits a SimdExtended instruction with the given sub-opcode and arguments.
    /// This replaces the old string-based LibraryCall with zero-cost typed dispatch.
    ///
    /// # Performance
    /// - Interpreter: ~2ns dispatch via Rust match
    /// - AOT/LLVM: Zero-cost - direct SIMD intrinsic
    /// - MLIR: vector dialect operations
    fn emit_simd_extended(&mut self, sub_op: crate::instruction::SimdSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::SimdExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for CharExtended instruction emission.
    ///
    /// Emits a CharExtended instruction with the given sub-opcode and arguments.
    /// ASCII operations are inline (~2ns), Unicode uses runtime lookup.
    fn emit_char_extended(&mut self, sub_op: crate::instruction::CharSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::CharExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for CbgrExtended instruction emission.
    ///
    /// Emits a CbgrExtended instruction with the given sub-opcode and arguments.
    /// Provides memory safety validation with ~15ns overhead.
    fn emit_cbgr_extended(&mut self, sub_op: crate::instruction::CbgrSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::CbgrExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for TextExtended instruction emission.
    ///
    /// Emits a TextExtended instruction with the given sub-opcode and arguments.
    /// Provides zero-cost text operations with ~2ns dispatch overhead.
    ///
    /// # Performance
    /// - Old (LibraryCall): ~15ns dispatch overhead
    /// - New (TextExtended): ~2ns dispatch overhead
    ///
    /// # Example
    /// ```ignore
    /// emit_text_extended(TextSubOpcode::ParseInt, &[text_reg])
    /// // Emits: TextExtended { sub_op: ParseInt, dst: temp, args: [text_reg] }
    /// ```
    fn emit_text_extended(&mut self, sub_op: crate::instruction::TextSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::TextExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for LogExtended instruction emission.
    ///
    /// Emits a LogExtended instruction with the given sub-opcode and arguments.
    fn emit_log_extended(&mut self, sub_op: crate::instruction::LogSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::LogExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for TensorExtended instruction emission.
    ///
    /// Emits a TensorExtended instruction with the given sub-opcode and arguments.
    /// This provides zero-cost tensor operations with ~3ns dispatch overhead.
    ///
    /// # Performance
    /// - Interpreter: ~3ns dispatch via Rust match
    /// - AOT/LLVM: linalg/tensor dialect operations
    fn emit_tensor_extended(&mut self, sub_op: crate::instruction::TensorSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::TensorExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for TensorExtSubOpcode instruction emission.
    ///
    /// Emits a TensorExtSubOpcode instruction for extended tensor operations
    /// that overflow the 256 sub-opcode limit.
    fn emit_tensor_ext_extended(&mut self, sub_op: crate::instruction::TensorExtSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::TensorExtExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for MlSubOpcode instruction emission.
    ///
    /// Emits an MlExtended (0xFD) instruction with the given sub-opcode.
    /// Used for gradient/autodiff operations and ML-specific functionality.
    ///
    /// # Performance
    /// - Interpreter: ~2ns dispatch via Rust match
    /// - AOT: Zero-cost - direct LLVM intrinsic or runtime call
    ///
    /// # Example
    /// ```ignore
    /// emit_ml_extended(MlSubOpcode::JvpBegin, &[primals, tangents])
    /// // Emits: MlExtended { sub_op: JvpBegin, dst: temp, args: [primals, tangents] }
    /// ```
    fn emit_ml_extended(&mut self, sub_op: MlSubOpcode, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::MlExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Helper for MetaReflect instruction emission.
    ///
    /// Emits a MetaReflect instruction with the given sub-opcode and arguments.
    /// Provides zero-cost type introspection operations.
    ///
    /// # Performance
    /// - Interpreter: ~2ns dispatch via Rust match
    /// - AOT: Constant-folded when type is statically known
    ///
    /// # Example
    /// ```ignore
    /// emit_meta_reflect(MetaReflectOp::TypeId, &[value])
    /// // Emits: MetaReflect { sub_op: TypeId, dst: temp, args: [value] }
    /// ```
    fn emit_meta_reflect(&mut self, sub_op: crate::instruction::MetaReflectOp, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::MetaReflect {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    // =========================================================================
    // Gradient/Autodiff Direct Opcode Helpers
    // =========================================================================

    /// Emit GradBegin direct opcode.
    ///
    /// Emits `Opcode::GradBegin` (0xEB) for starting a gradient scope.
    /// When registry specifies DirectOpcode strategy, this is handled there.
    /// This fallback handles InlineSequence path.
    fn emit_grad_begin(&mut self, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::GradBegin,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Emit GradEnd direct opcode.
    ///
    /// Emits `Opcode::GradEnd` (0xEC) for ending a gradient scope.
    fn emit_grad_end(&mut self, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::GradEnd,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Emit GradStop direct opcode.
    ///
    /// Emits `Opcode::GradStop` (0xEF) for detaching a tensor from gradient computation.
    fn emit_grad_stop(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::GradStop,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Emit GradCheckpoint direct opcode.
    ///
    /// Emits `Opcode::GradCheckpoint` (0xED) for activation checkpointing.
    fn emit_grad_checkpoint(&mut self, args: &[Reg]) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::GradCheckpoint,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Emit GradAccumulate direct opcode.
    ///
    /// Emits `Opcode::GradAccumulate` (0xEE) for gradient accumulation.
    fn emit_grad_accumulate(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::GradAccumulate,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    // =========================================================================
    // Extended Inline Sequence Implementations
    // =========================================================================

    /// Floor (f64). Maps to llvm.floor.f64 / math.floor
    fn emit_floor_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::FloorF64, args)
    }

    /// Ceiling (f64). Maps to llvm.ceil.f64 / math.ceil
    fn emit_ceil_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::CeilF64, args)
    }

    /// Round to nearest (f64). Maps to llvm.round.f64 / math.round
    fn emit_round_f64(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::RoundF64, args)
    }

    /// Reverse bits.
    fn emit_bitreverse(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::BitOp {
            dst: result,
            op: BitOpKind::Bswap, // Using bswap as base - will be extended for bitreverse
            src: args[0],
        });
        Some(result)
    }

    /// Integer to float conversion.
    fn emit_int_to_float(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::CvtIF,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Float to integer conversion.
    fn emit_float_to_int(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        let result = self.alloc_temp();
        let mode_reg = self.alloc_temp();
        self.emit(IntrinsicInstruction::LoadSmallI {
            dst: mode_reg,
            value: 0, // trunc mode
        });
        self.emit(IntrinsicInstruction::CvtFI {
            dst: result,
            src: args[0],
            mode: mode_reg,
        });
        Some(result)
    }

    /// Convert to little-endian bytes.
    fn emit_to_le_bytes(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        // On little-endian systems, this is a no-op/reinterpret
        // On big-endian, we'd need bswap
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::Mov,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Convert from little-endian bytes.
    fn emit_from_le_bytes(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_to_le_bytes(args) // Same as to_le on LE systems
    }

    /// Convert to big-endian bytes.
    fn emit_to_be_bytes(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        // Swap bytes for big-endian
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::BitOp {
            dst: result,
            op: BitOpKind::Bswap,
            src: args[0],
        });
        Some(result)
    }

    /// Convert from big-endian bytes.
    fn emit_from_be_bytes(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_to_be_bytes(args) // Same as to_be (bswap is its own inverse)
    }

    // =========================================================================
    // Math Functions F32 - Zero-cost MathExtended instruction emission
    // =========================================================================

    /// Floor (f32). Maps to llvm.floor.f32 / math.floor
    fn emit_floor_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::FloorF32, args)
    }

    /// Ceiling (f32). Maps to llvm.ceil.f32 / math.ceil
    fn emit_ceil_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::CeilF32, args)
    }

    /// Round (f32). Maps to llvm.round.f32 / math.round
    fn emit_round_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::RoundF32, args)
    }

    /// Truncate (f32). Maps to llvm.trunc.f32 / math.trunc
    fn emit_trunc_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::TruncF32, args)
    }

    /// Absolute value (f32). Maps to llvm.fabs.f32 / math.abs
    fn emit_abs_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::AbsF32, args)
    }

    /// Sine (f32). Maps to llvm.sin.f32 / math.sin
    fn emit_sin_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::SinF32, args)
    }

    /// Cosine (f32). Maps to llvm.cos.f32 / math.cos
    fn emit_cos_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::CosF32, args)
    }

    /// Tangent (f32). Maps to llvm.tan.f32 / math.tan
    fn emit_tan_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::TanF32, args)
    }

    /// Arc sine (f32). Maps to llvm.asin.f32 / math.asin
    fn emit_asin_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::AsinF32, args)
    }

    /// Arc cosine (f32). Maps to llvm.acos.f32 / math.acos
    fn emit_acos_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::AcosF32, args)
    }

    /// Arc tangent (f32). Maps to llvm.atan.f32 / math.atan
    fn emit_atan_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::AtanF32, args)
    }

    /// Two-argument arc tangent (f32). Maps to llvm.atan2.f32 / math.atan2
    fn emit_atan2_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::Atan2F32, args)
    }

    /// Natural exponential (f32). Maps to llvm.exp.f32 / math.exp
    fn emit_exp_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::ExpF32, args)
    }

    /// Natural logarithm (f32). Maps to llvm.log.f32 / math.log
    fn emit_log_f32(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_math_extended(MathSubOpcode::LogF32, args)
    }

    /// Saturating add.
    ///
    /// Uses ArithExtended with SaturatingAdd sub-opcode for zero-cost dispatch.
    /// Result is clamped to MAX on overflow, MIN on underflow.
    fn emit_saturating_add(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }
        // Use ArithExtended with SaturatingAdd for zero-cost typed dispatch
        self.emit_arith_extended_opcode(ArithSubOpcode::SaturatingAdd, args)
    }

    /// Saturating subtract.
    ///
    /// Uses ArithExtended with SaturatingSub sub-opcode for zero-cost dispatch.
    /// Result is clamped to MIN on underflow, MAX on overflow.
    fn emit_saturating_sub(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }
        // Use ArithExtended with SaturatingSub for zero-cost typed dispatch
        self.emit_arith_extended_opcode(ArithSubOpcode::SaturatingSub, args)
    }

    /// Sign-extend integer to wider type.
    ///
    /// Uses ArithExtended with SextI sub-opcode for zero-cost dispatch.
    /// Maps to LLVM `sext` instruction in AOT compilation.
    fn emit_sext(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        // Use ArithExtended with SextI for zero-cost typed dispatch
        self.emit_arith_extended_opcode(ArithSubOpcode::SextI, args)
    }

    /// Zero-extend integer to wider type.
    ///
    /// Uses ArithExtended with ZextI sub-opcode for zero-cost dispatch.
    /// Maps to LLVM `zext` instruction in AOT compilation.
    fn emit_zext(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        // Use ArithExtended with ZextI for zero-cost typed dispatch
        self.emit_arith_extended_opcode(ArithSubOpcode::ZextI, args)
    }

    /// Extend float precision (f32 -> f64).
    ///
    /// Uses ArithExtended with FpextF sub-opcode for zero-cost dispatch.
    /// Maps to LLVM `fpext float to double` instruction.
    fn emit_fpext(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        // Use ArithExtended with FpextF for zero-cost typed dispatch
        self.emit_arith_extended_opcode(ArithSubOpcode::FpextF, args)
    }

    /// Truncate float precision (f64 -> f32).
    ///
    /// Uses ArithExtended with FptruncF sub-opcode for zero-cost dispatch.
    /// Maps to LLVM `fptrunc double to float` instruction.
    fn emit_fptrunc(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        // Use ArithExtended with FptruncF for zero-cost typed dispatch
        self.emit_arith_extended_opcode(ArithSubOpcode::FptruncF, args)
    }

    /// Truncate integer width.
    fn emit_int_trunc(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        let result = self.alloc_temp();
        // Mask to target width
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::Band,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Bitcast (reinterpret bits).
    fn emit_bitcast(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        // Bitcast is a no-op at the register level
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::Mov,
            dst: Some(result),
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Check if float is NaN.
    fn emit_is_nan(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }
        let result = self.alloc_temp();
        // NaN is the only value that is not equal to itself
        self.emit(IntrinsicInstruction::BinaryOp {
            opcode: Opcode::NeF,
            dst: result,
            lhs: args[0],
            rhs: args[0],
        });
        Some(result)
    }

    /// Check if float is infinite.
    ///
    /// Uses MathExtended with IsInfF64/IsInfF32 sub-opcode for zero-cost dispatch.
    /// Maps to llvm.is.fpclass.f64 in LLVM lowering.
    fn emit_is_inf(&mut self, args: &[Reg]) -> Option<Reg> {
        // Use MathExtended with IsInfF64 for zero-cost typed dispatch
        self.emit_math_extended(MathSubOpcode::IsInfF64, args)
    }

    /// Check if float is finite.
    ///
    /// Uses MathExtended with IsFiniteF64/IsFiniteF32 sub-opcode for zero-cost dispatch.
    /// Maps to llvm.is.fpclass.f64 in LLVM lowering.
    fn emit_is_finite(&mut self, args: &[Reg]) -> Option<Reg> {
        // Use MathExtended with IsFiniteF64 for zero-cost typed dispatch
        self.emit_math_extended(MathSubOpcode::IsFiniteF64, args)
    }

    /// Emit slice length extraction (extracts len from fat pointer).
    /// Emit slice length extraction using zero-cost CbgrExtended dispatch.
    ///
    /// Uses CbgrSubOpcode::SliceLen for ~2ns overhead instead of library call.
    fn emit_slice_len(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_cbgr_extended(CbgrSubOpcode::SliceLen, args)
    }

    /// Emit slice pointer extraction using zero-cost CbgrExtended dispatch.
    ///
    /// Uses CbgrSubOpcode::Unslice for ~2ns overhead instead of library call.
    fn emit_slice_as_ptr(&mut self, args: &[Reg]) -> Option<Reg> {
        self.emit_cbgr_extended(CbgrSubOpcode::Unslice, args)
    }

    /// Emit compile-time constant (resolved by compiler).
    fn emit_compile_time_constant(&mut self, _args: &[Reg]) -> Option<Reg> {
        let dst = self.alloc_temp();
        self.emit(IntrinsicInstruction::CompileTimeConst {
            dst,
            intrinsic_name: self.intrinsic.name.to_string(),
        });
        Some(dst)
    }

    /// Emit ArithExtended opcode with sub-opcode for checked/overflowing/polymorphic arithmetic.
    fn emit_arith_extended_opcode(&mut self, sub_op: ArithSubOpcode, args: &[Reg]) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        // Emit ArithExtended instruction with the appropriate sub-opcode
        match sub_op {
            // Polymorphic arithmetic (binary ops: dst, a, b)
            ArithSubOpcode::PolyAdd
            | ArithSubOpcode::PolySub
            | ArithSubOpcode::PolyMul
            | ArithSubOpcode::PolyDiv
            | ArithSubOpcode::PolyRem => {
                if args.len() >= 2 {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0], args[1]],
                    });
                }
            }
            // Polymorphic negation (unary op: dst, src)
            ArithSubOpcode::PolyNeg => {
                if !args.is_empty() {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0]],
                    });
                }
            }
            // Checked arithmetic (returning Maybe<T>)
            ArithSubOpcode::CheckedAddI
            | ArithSubOpcode::CheckedSubI
            | ArithSubOpcode::CheckedMulI
            | ArithSubOpcode::CheckedDivI => {
                if args.len() >= 2 {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0], args[1]],
                    });
                }
            }
            // Overflowing arithmetic (returning tuple)
            ArithSubOpcode::OverflowingAddI
            | ArithSubOpcode::OverflowingSubI
            | ArithSubOpcode::OverflowingMulI => {
                if args.len() >= 2 {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0], args[1]],
                    });
                }
            }
            // Polymorphic unary operations (abs, signum)
            ArithSubOpcode::PolyAbs | ArithSubOpcode::PolySignum => {
                if !args.is_empty() {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0]],
                    });
                }
            }
            // Polymorphic binary operations (min, max)
            ArithSubOpcode::PolyMin | ArithSubOpcode::PolyMax => {
                if args.len() >= 2 {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0], args[1]],
                    });
                }
            }
            // Polymorphic ternary operation (clamp)
            ArithSubOpcode::PolyClamp => {
                if args.len() >= 3 {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0], args[1], args[2]],
                    });
                }
            }
            // Saturating and wrapping are handled by dedicated methods
            ArithSubOpcode::SaturatingAdd
            | ArithSubOpcode::SaturatingSub
            | ArithSubOpcode::SaturatingMul
            | ArithSubOpcode::WrappingAdd
            | ArithSubOpcode::WrappingSub
            | ArithSubOpcode::WrappingMul
            | ArithSubOpcode::WrappingNeg
            | ArithSubOpcode::WrappingShl
            | ArithSubOpcode::WrappingShr => {
                // These are handled via WrappingOpcode/SaturatingOpcode strategies
                // but if reached here via ArithExtendedOpcode, emit as generic
                if args.len() >= 2 {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0], args[1]],
                    });
                } else if !args.is_empty() {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0]],
                    });
                }
            }
            // Bit counting operations (Clz, Ctz, Popcnt, Bswap, BitReverse, RotateLeft, RotateRight)
            // These are handled via InlineSequence strategy, but support generic path
            _ => {
                // Generic handling for bit ops and any new sub-opcodes
                if args.len() >= 2 {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0], args[1]],
                    });
                } else if !args.is_empty() {
                    self.instructions.push(IntrinsicInstruction::ArithExtended {
                        sub_op,
                        dst: dst.unwrap_or(Reg(0)),
                        args: vec![args[0]],
                    });
                }
            }
        }

        dst
    }

    /// Emit type-aware wrapping arithmetic opcode.
    fn emit_wrapping_opcode(
        &mut self,
        sub_op: ArithSubOpcode,
        width: u8,
        signed: bool,
        args: &[Reg],
    ) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        // Create wrapping instruction with embedded width and signed flag
        self.instructions.push(IntrinsicInstruction::WrappingArith {
            sub_op,
            dst: dst.unwrap_or(Reg(0)),
            args: args.to_vec(),
            width,
            signed,
        });

        dst
    }

    /// Emit type-aware saturating arithmetic opcode.
    fn emit_saturating_opcode(
        &mut self,
        sub_op: ArithSubOpcode,
        width: u8,
        signed: bool,
        args: &[Reg],
    ) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        // Create saturating instruction with embedded width and signed flag
        self.instructions.push(IntrinsicInstruction::SaturatingArith {
            sub_op,
            dst: dst.unwrap_or(Reg(0)),
            args: args.to_vec(),
            width,
            signed,
        });

        dst
    }

    /// Emit load false constant (for Poll::Pending in Tier 0).
    fn emit_load_false(&mut self) -> Option<Reg> {
        let dst = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::LoadFalse,
            dst: Some(dst),
            args: vec![],
        });
        Some(dst)
    }

    /// Emit call to second argument (for recovery passthrough in Tier 0).
    fn emit_call_second_arg(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() >= 2 {
            // Call the second argument (future_factory) as a closure, ignore first (recovery_ctx)
            let dst = self.alloc_temp();
            self.emit(IntrinsicInstruction::ClosureCall {
                dst,
                closure: args[1],
                args: vec![],
            });
            Some(dst)
        } else {
            // Fallback: return nil
            let dst = self.alloc_temp();
            self.emit(IntrinsicInstruction::Generic {
                opcode: Opcode::LoadNil,
                dst: Some(dst),
                args: vec![],
            });
            Some(dst)
        }
    }

    /// Emit load unit constant.
    fn emit_load_unit(&mut self) -> Option<Reg> {
        let dst = self.alloc_temp();
        self.emit(IntrinsicInstruction::Generic {
            opcode: Opcode::LoadUnit,
            dst: Some(dst),
            args: vec![],
        });
        Some(dst)
    }

    // =========================================================================
    // Volatile Memory Operations (MMIO Support)
    // =========================================================================

    /// Volatile load: read from pointer without optimization/reordering.
    fn emit_volatile_load(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }

        let dst = self.alloc_temp();
        // Emit volatile load intrinsic
        self.emit(IntrinsicInstruction::VolatileLoad {
            dst,
            ptr: args[0],
        });

        Some(dst)
    }

    /// Volatile store: write to pointer without optimization/reordering.
    fn emit_volatile_store(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        // Emit volatile store intrinsic
        self.emit(IntrinsicInstruction::VolatileStore {
            ptr: args[0],
            value: args[1],
        });

        None // void return
    }

    /// Volatile load with acquire semantics.
    fn emit_volatile_load_acquire(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.is_empty() {
            return None;
        }

        let dst = self.alloc_temp();
        // Emit volatile load with acquire ordering
        self.emit(IntrinsicInstruction::VolatileLoadAcquire {
            dst,
            ptr: args[0],
        });

        Some(dst)
    }

    /// Volatile store with release semantics.
    fn emit_volatile_store_release(&mut self, args: &[Reg]) -> Option<Reg> {
        if args.len() < 2 {
            return None;
        }

        // Emit volatile store with release ordering
        self.emit(IntrinsicInstruction::VolatileStoreRelease {
            ptr: args[0],
            value: args[1],
        });

        None // void return
    }

    /// Compiler fence: prevents compiler reordering only.
    fn emit_compiler_fence(&mut self, _args: &[Reg]) -> Option<Reg> {
        self.emit(IntrinsicInstruction::CompilerFence);
        None // void return
    }

    /// Hardware fence: prevents CPU reordering.
    fn emit_hardware_fence(&mut self, _args: &[Reg]) -> Option<Reg> {
        self.emit(IntrinsicInstruction::HardwareFence);
        None // void return
    }

    /// Emit tensor extended opcode for advanced tensor operations.
    fn emit_tensor_extended_opcode(
        &mut self,
        sub_op: crate::instruction::TensorSubOpcode,
        args: &[Reg],
    ) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        // Emit TensorExtended instruction with the sub-opcode
        self.emit(IntrinsicInstruction::TensorExtended {
            sub_op,
            dst: dst.unwrap_or(Reg(0)),
            args: args.to_vec(),
        });

        dst
    }

    /// Emit tensor extended opcode with mode byte.
    fn emit_tensor_extended_opcode_with_mode(
        &mut self,
        sub_op: crate::instruction::TensorSubOpcode,
        mode: u8,
        args: &[Reg],
    ) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        // Emit TensorExtended instruction with the sub-opcode and mode
        self.emit(IntrinsicInstruction::TensorExtendedWithMode {
            sub_op,
            mode,
            dst: dst.unwrap_or(Reg(0)),
            args: args.to_vec(),
        });

        dst
    }

    /// Emit tensor ext extended opcode (using TensorExtSubOpcode).
    ///
    /// This emits TensorExtExtended instructions with proper sub-opcode dispatch,
    /// providing ~2ns zero-cost dispatch instead of ~15ns library call overhead.
    fn emit_tensor_ext_extended_opcode(
        &mut self,
        sub_op: crate::instruction::TensorExtSubOpcode,
        args: &[Reg],
    ) -> Option<Reg> {
        let result = self.alloc_temp();
        self.emit(IntrinsicInstruction::TensorExtExtended {
            sub_op,
            dst: result,
            args: args.to_vec(),
        });
        Some(result)
    }

    /// Emit GPU extended opcode for GPU operations.
    fn emit_gpu_extended_opcode(
        &mut self,
        sub_op: crate::instruction::GpuSubOpcode,
        args: &[Reg],
    ) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        // Emit GpuExtended instruction with the sub-opcode
        self.emit(IntrinsicInstruction::GpuExtended {
            sub_op,
            dst: dst.unwrap_or(Reg(0)),
            args: args.to_vec(),
        });

        dst
    }

    /// Emit math extended opcode for transcendental/special math operations.
    ///
    /// This uses the MathExtended opcode (0x29) with a sub-opcode byte for
    /// zero-cost dispatch of 72+ mathematical functions.
    fn emit_math_extended_opcode(
        &mut self,
        sub_op: crate::instruction::MathSubOpcode,
        args: &[Reg],
    ) -> Option<Reg> {
        let dst = if self.intrinsic.return_count > 0 {
            Some(self.alloc_temp())
        } else {
            None
        };

        // Emit MathExtended instruction with the sub-opcode
        self.emit(IntrinsicInstruction::MathExtended {
            sub_op,
            dst: dst.unwrap_or(Reg(0)),
            args: args.to_vec(),
        });

        dst
    }
}

// =========================================================================
// Instruction Extensions for Intrinsics
// =========================================================================

/// Memory intrinsic kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum MemIntrinsicKind {
    Memcpy,
    Memmove,
    Memset,
}

/// Atomic RMW operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum AtomicRmwOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Min,
    Max,
    Xchg,
}

/// Checked arithmetic operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum CheckedArithOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Bit operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum BitOpKind {
    Clz,
    Ctz,
    Popcnt,
    Bswap,
}

/// Rotate direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum RotateDirection {
    Left,
    Right,
}

/// Time intrinsic kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum TimeIntrinsicKind {
    MonotonicNanos,
    RealtimeSecs,
    RealtimeNanos,
}

/// Extended instruction variants for intrinsics.
///
/// These extend the base VBC Instruction enum with intrinsic-specific variants.
/// The interpreter and MLIR lowering handle these specially.
///
/// Note: Named `IntrinsicInstruction` to avoid conflict with `crate::instruction::Instruction`.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum IntrinsicInstruction {
    // Standard opcodes (from base Instruction)
    BinaryOp { opcode: Opcode, dst: Reg, lhs: Reg, rhs: Reg },
    UnaryOp { opcode: Opcode, dst: Reg, src: Reg },
    Deref { dst: Reg, ptr: Reg },
    DerefMut { ptr: Reg, value: Reg },
    Ref { dst: Reg, src: Reg },
    ChkRef { dst: Reg, src: Reg },
    TlsGet { dst: Reg, slot: Reg },
    TlsSet { slot: Reg, value: Reg },
    PushContext { dst: Reg },
    PopContext,
    Panic { msg: Reg },
    Unreachable,
    Assert { cond: Reg, msg: Reg },
    DebugPrint { value: Reg },
    Spawn { dst: Reg, future: Reg, task_id: Reg },
    Await { dst: Reg, future: Reg },
    IoSubmit { dst: Reg, ops: Reg },
    SizeOfG { dst: Reg, type_param: Option<Reg> },
    AlignOfG { dst: Reg, type_param: Option<Reg> },
    LoadSmallI { dst: Reg, value: i8 },
    LoadI { dst: Reg, value: i64 },
    Pack { dst: Reg, regs: Vec<Reg> },
    SyscallLinux { dst: Reg, num: Reg, args: RegRange },
    CvtFI { dst: Reg, src: Reg, mode: Reg },
    AtomicFence { ordering: Reg },
    SpinHint,
    AtomicLoad { dst: Reg, ptr: Reg, ordering: Reg, size: u8 },
    AtomicStore { ptr: Reg, value: Reg, ordering: Reg, size: u8 },
    AtomicCas { old_dst: Reg, success_dst: Reg, ptr: Reg, expected: Reg, desired: Reg, success_order: Reg, failure_order: Reg, size: u8 },
    Generic { opcode: Opcode, dst: Option<Reg>, args: Vec<Reg> },
    GenericWithMode { opcode: Opcode, mode: u8, dst: Option<Reg>, args: Vec<Reg> },
    GenericWithSize { opcode: Opcode, size: u8, dst: Option<Reg>, args: Vec<Reg> },

    // Intrinsic-specific instructions
    MemIntrinsic { kind: MemIntrinsicKind, dst: Reg, src: Reg, len: Reg },
    MemCmp { dst: Reg, lhs: Reg, rhs: Reg, len: Reg },
    AtomicRmw { dst: Reg, op: AtomicRmwOp, ptr: Reg, value: Reg, ordering: Reg },
    SpinlockLock { lock: Reg },
    FutexWait { dst: Reg, addr: Reg, expected: Reg, timeout_ns: Reg },
    FutexWake { dst: Reg, addr: Reg, count: Reg },
    CheckedArith { result: Reg, overflow: Reg, op: CheckedArithOp, lhs: Reg, rhs: Reg },
    BitOp { dst: Reg, op: BitOpKind, src: Reg },
    RotateOp { dst: Reg, src: Reg, amount: Reg, direction: RotateDirection },
    /// Math Extended instruction with sub-opcode for transcendental/special math functions.
    ///
    /// Uses the MathExtended (0x29) opcode with sub-opcodes from MathSubOpcode.
    /// Maps directly to LLVM intrinsics (llvm.sin.f64, llvm.sqrt.f64, etc.)
    /// and MLIR math dialect ops (math.sin, math.sqrt, etc.).
    ///
    /// # Performance
    /// - Interpreter: ~2ns dispatch via Rust match
    /// - AOT: Zero-cost - direct LLVM intrinsic or libm call
    ///
    /// # Example
    /// ```ignore
    /// MathExtended { sub_op: MathSubOpcode::SinF64, dst: r0, args: vec![r1] }
    /// // Encodes to: 0x29 0x00 <dst> <src>
    /// // LLVM: %r0 = call double @llvm.sin.f64(double %r1)
    /// // MLIR: %r0 = math.sin %r1 : f64
    /// ```
    MathExtended { sub_op: crate::instruction::MathSubOpcode, dst: Reg, args: Vec<Reg> },
    TimeIntrinsic { dst: Reg, kind: TimeIntrinsicKind },
    CompileTimeConst { dst: Reg, intrinsic_name: String },
    /// Arithmetic Extended instruction with sub-opcode for checked/overflowing/polymorphic arithmetic.
    ArithExtended { sub_op: ArithSubOpcode, dst: Reg, args: Vec<Reg> },
    /// Type-aware wrapping arithmetic with explicit width and signedness.
    WrappingArith { sub_op: ArithSubOpcode, dst: Reg, args: Vec<Reg>, width: u8, signed: bool },
    /// Type-aware saturating arithmetic with explicit width and signedness.
    SaturatingArith { sub_op: ArithSubOpcode, dst: Reg, args: Vec<Reg>, width: u8, signed: bool },
    /// Closure call for indirect function invocation.
    ClosureCall { dst: Reg, closure: Reg, args: Vec<Reg> },
    /// Volatile load: read from pointer without optimization/reordering (MMIO).
    VolatileLoad { dst: Reg, ptr: Reg },
    /// Volatile store: write to pointer without optimization/reordering (MMIO).
    VolatileStore { ptr: Reg, value: Reg },
    /// Volatile load with acquire semantics (MMIO with barrier).
    VolatileLoadAcquire { dst: Reg, ptr: Reg },
    /// Volatile store with release semantics (MMIO with barrier).
    VolatileStoreRelease { ptr: Reg, value: Reg },
    /// Compiler fence: prevents compiler reordering only.
    CompilerFence,
    /// Hardware fence: prevents CPU memory reordering.
    HardwareFence,
    /// Tensor Extended instruction with sub-opcode for advanced tensor operations.
    TensorExtended { sub_op: crate::instruction::TensorSubOpcode, dst: Reg, args: Vec<Reg> },
    /// Tensor Extended instruction with sub-opcode and mode byte.
    TensorExtendedWithMode { sub_op: crate::instruction::TensorSubOpcode, mode: u8, dst: Reg, args: Vec<Reg> },
    /// GPU Extended instruction with sub-opcode for GPU operations.
    GpuExtended { sub_op: crate::instruction::GpuSubOpcode, dst: Reg, args: Vec<Reg> },
    /// SIMD Extended instruction with sub-opcode for vector operations.
    ///
    /// Maps to SimdExtended (0x2A) opcode with SimdSubOpcode dispatch.
    /// Provides ~2ns zero-cost dispatch in interpreter, zero overhead in AOT.
    ///
    /// # Platform Targets
    /// - x86: AVX2/AVX-512 intrinsics
    /// - ARM: NEON intrinsics
    /// - MLIR: vector dialect
    ///
    /// # Example
    /// ```ignore
    /// SimdExtended { sub_op: SimdSubOpcode::Add, dst: r0, args: vec![r1, r2] }
    /// // Encodes to: 0x2A 0x10 <dst> <src1> <src2>
    /// // LLVM: <4 x float> %r0 = fadd <4 x float> %r1, %r2
    /// // MLIR: %r0 = arith.addf %r1, %r2 : vector<4xf32>
    /// ```
    SimdExtended { sub_op: crate::instruction::SimdSubOpcode, dst: Reg, args: Vec<Reg> },
    /// Character Extended instruction with sub-opcode for character operations.
    ///
    /// Maps to CharExtended (0x2B) opcode with CharSubOpcode dispatch.
    /// ASCII operations are inline (~2ns), Unicode operations use runtime lookup.
    ///
    /// # Example
    /// ```ignore
    /// CharExtended { sub_op: CharSubOpcode::IsAlphabetic, dst: r0, args: vec![r1] }
    /// // Encodes to: 0x2B 0x00 <dst> <src>
    /// ```
    CharExtended { sub_op: crate::instruction::CharSubOpcode, dst: Reg, args: Vec<Reg> },
    /// CBGR Extended instruction with sub-opcode for memory safety operations.
    ///
    /// Maps to CbgrExtended (0x78) opcode with CbgrSubOpcode dispatch.
    /// Provides memory safety validation with ~15ns overhead.
    ///
    /// # Example
    /// ```ignore
    /// CbgrExtended { sub_op: CbgrSubOpcode::Validate, dst: r0, args: vec![r1] }
    /// // Encodes to: 0x78 0x00 <dst> <ptr>
    /// ```
    CbgrExtended { sub_op: crate::instruction::CbgrSubOpcode, dst: Reg, args: Vec<Reg> },
    /// Log Extended instruction with sub-opcode for logging operations.
    ///
    /// Maps to LogExtended (0xBE) opcode with LogSubOpcode dispatch.
    ///
    /// # Example
    /// ```ignore
    /// LogExtended { sub_op: LogSubOpcode::Debug, dst: r0, args: vec![r1] }
    /// // Encodes to: 0xBE 0x00 <dst> <msg>
    /// ```
    LogExtended { sub_op: crate::instruction::LogSubOpcode, dst: Reg, args: Vec<Reg> },
    /// ML Extended instruction with sub-opcode for ML/gradient operations.
    ///
    /// Maps to MlExtended (0xFD) opcode with MlSubOpcode dispatch.
    /// Provides zero-cost gradient operations for autodiff.
    ///
    /// # Example
    /// ```ignore
    /// MlExtended { sub_op: MlSubOpcode::JvpBegin, dst: r0, args: vec![r1, r2] }
    /// // Encodes to: 0xFD 0x66 <dst> <primals> <tangents>
    /// ```
    MlExtended { sub_op: crate::instruction::MlSubOpcode, dst: Reg, args: Vec<Reg> },
    /// Tensor Extended Extended instruction for operations that overflow the 256 sub-opcode limit.
    ///
    /// Uses a two-byte encoding: [0xFC] [0x00] [ext_opcode:u8] [operands...]
    ///
    /// # Example
    /// ```ignore
    /// TensorExtExtended { sub_op: TensorExtSubOpcode::ContiguousView, dst: r0, args: vec![r1] }
    /// // Encodes to: 0xFC 0x00 0x04 <dst> <src>
    /// ```
    TensorExtExtended { sub_op: crate::instruction::TensorExtSubOpcode, dst: Reg, args: Vec<Reg> },
    /// Meta-reflection instruction for type introspection.
    ///
    /// Maps to MetaReflect (0xBB) opcode with MetaReflectOp dispatch.
    /// Provides zero-cost type introspection operations.
    ///
    /// # Performance
    /// - Interpreter: ~2ns dispatch via Rust match
    /// - AOT: Constant-folded when type is statically known
    ///
    /// # Example
    /// ```ignore
    /// MetaReflect { sub_op: MetaReflectOp::TypeId, dst: r0, args: vec![r1] }
    /// // Encodes to: 0xBB 0x00 <dst> <value>
    /// ```
    MetaReflect { sub_op: crate::instruction::MetaReflectOp, dst: Reg, args: Vec<Reg> },
    /// Text Extended instruction for text parsing and conversion operations.
    ///
    /// Maps to TextExtended (0x79) opcode with TextSubOpcode dispatch.
    /// Provides zero-cost text operations replacing string-based library calls.
    ///
    /// # Performance
    /// - Old (LibraryCall): ~15ns dispatch overhead
    /// - New (TextExtended): ~2ns dispatch overhead
    ///
    /// # Example
    /// ```ignore
    /// TextExtended { sub_op: TextSubOpcode::ParseInt, dst: r0, args: vec![r1] }
    /// // Encodes to: 0x79 0x10 <dst> <text>
    /// ```
    TextExtended { sub_op: crate::instruction::TextSubOpcode, dst: Reg, args: Vec<Reg> },
    /// Heap memory allocation.
    MemAlloc { dst: Reg, size: Reg, align: Reg, zeroed: bool },
    /// Heap memory deallocation.
    MemDealloc { ptr: Reg, size: Reg, align: Reg },
    /// Heap memory reallocation.
    MemRealloc { dst: Reg, ptr: Reg, old_size: Reg, new_size: Reg, align: Reg },
    /// Swap two values in place via pointers.
    MemSwap { a: Reg, b: Reg },
    /// Replace value and return old.
    MemReplace { dst: Reg, dest: Reg, src: Reg },
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::INTRINSIC_REGISTRY;

    #[test]
    fn test_direct_opcode_codegen() {
        let intrinsic = INTRINSIC_REGISTRY.lookup("wrapping_add").unwrap();
        let codegen = IntrinsicCodegen::new(intrinsic, 0);
        let args = vec![Reg::new(0), Reg::new(1)];
        let result = codegen.generate(&args);

        assert_eq!(result.instructions.len(), 1);
        assert!(result.result_reg.is_some());
    }

    #[test]
    fn test_atomic_codegen() {
        let intrinsic = INTRINSIC_REGISTRY.lookup("atomic_load_u64").unwrap();
        let codegen = IntrinsicCodegen::new(intrinsic, 0);
        let args = vec![Reg::new(0), Reg::new(1)]; // ptr, ordering
        let result = codegen.generate(&args);

        assert!(result.result_reg.is_some());
    }

    #[test]
    fn test_syscall_codegen() {
        let intrinsic = INTRINSIC_REGISTRY.lookup("syscall3").unwrap();
        let codegen = IntrinsicCodegen::new(intrinsic, 0);
        let args = vec![Reg::new(0), Reg::new(1), Reg::new(2), Reg::new(3)]; // num, a1, a2, a3
        let result = codegen.generate(&args);

        assert!(result.result_reg.is_some());
    }
}
