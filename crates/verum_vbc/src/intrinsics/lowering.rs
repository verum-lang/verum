//! # Industrial-Grade Intrinsic Lowering
//!
//! This module provides lowering for intrinsics to enable zero-overhead
//! code generation. The lowering strategy depends on the target:
//!
//! ## ARCHITECTURE DECISION: MLIR for GPU only, LLVM for CPU
//!
//! **CRITICAL**: MLIR is used **ONLY** for GPU compilation paths.
//! All CPU code, including math intrinsics, uses LLVM IR directly.
//!
//! | Target | Technology | Math Functions |
//! |--------|------------|----------------|
//! | CPU | LLVM IR (inkwell) | LLVM intrinsics (llvm.sin.f32, etc.) |
//! | GPU | MLIR dialects | MLIR math dialect → GPU kernels |
//!
//! ## NO LIBC ARCHITECTURE
//!
//! Verum does **NOT** link against libc. All math functionality is provided by:
//! - LLVM intrinsics (llvm.sin.f32, llvm.sqrt.f64, llvm.floor.f32, etc.)
//! - Custom implementations in /core/ for functions without LLVM intrinsics
//! - Platform syscalls via /core/sys/ for I/O and threading
//!
//! ## Design Principles
//!
//! 1. **LLVM Transparency**: CPU operations use LLVM intrinsics directly
//! 2. **Optimization Friendly**: Patterns enable constant folding, inlining, vectorization
//! 3. **Target Independence**: Platform-specific code selected during lowering
//! 4. **Debug Info**: Source locations preserved for debugging
//!
//! ## LLVM Intrinsic Usage (CPU Path)
//!
//! | Category | LLVM Intrinsic | Verum Intrinsic |
//! |----------|----------------|-----------------|
//! | Basic Math | llvm.sqrt.f32/f64 | sqrt_f32, sqrt_f64 |
//! | Trig | llvm.sin.f32/f64, llvm.cos.f32/f64 | sin_f32, cos_f64 |
//! | Exp/Log | llvm.exp.f32/f64, llvm.log.f32/f64 | exp_f32, log_f64 |
//! | Rounding | llvm.floor.f32/f64, llvm.ceil.f32/f64 | floor_f32, ceil_f64 |
//! | Hyperbolic | llvm.sinh.f32/f64, llvm.cosh.f32/f64 | sinh_f32, cosh_f64 |
//! | Power | llvm.pow.f64, llvm.powi.f64.i32 | pow_f64, powi_f64 |
//! | FP Class | llvm.is.fpclass | is_inf, is_finite |
//!
//! ## ASCII Character Operations (Inline)
//!
//! Character classification and conversion are implemented inline using
//! arithmetic comparisons, avoiding libc calls (isalpha, isupper, etc.):
//!
//! | Operation | Implementation |
//! |-----------|---------------|
//! | isalpha | (c >= 'A' && c <= 'Z') \|\| (c >= 'a' && c <= 'z') |
//! | isdigit | c >= '0' && c <= '9' |
//! | isspace | c in {' ', '\t', '\n', '\r', '\f', '\v'} |
//! | isupper | c >= 'A' && c <= 'Z' |
//! | islower | c >= 'a' && c <= 'z' |
//! | toupper | if islower(c) then c - 32 else c |
//! | tolower | if isupper(c) then c + 32 else c |
//!
//! **Note**: These are ASCII-only. Unicode support is provided by /core/text.
//!
//! ## MLIR Dialect Usage (GPU Path Only)
//!
//! These dialects are reserved for GPU compilation via MLIR path:
//!
//! | Dialect | Usage | Example Operations |
//! |---------|-------|-------------------|
//! | arith | Arithmetic ops | arith.addi, arith.mulf |
//! | math | Transcendental | math.sin, math.exp (GPU kernels) |
//! | gpu | GPU launch | gpu.launch, gpu.barrier |
//! | vector | SIMD/SIMT | vector.broadcast, vector.contract |

use super::registry::{
    CodegenStrategy, InlineSequenceId, Intrinsic, IntrinsicCategory, IntrinsicHint,
};
use crate::instruction::{ArithSubOpcode, MathSubOpcode, Opcode};

/// MLIR operation representation for intrinsics.
#[derive(Debug, Clone)]
pub struct MlirOp {
    /// MLIR dialect and operation name (e.g., "arith.addi").
    pub name: String,
    /// Operation attributes.
    pub attrs: Vec<MlirAttr>,
    /// Result types.
    pub result_types: Vec<MlirType>,
    /// Operand indices (references to SSA values).
    pub operands: Vec<usize>,
    /// Region (for operations with nested blocks).
    pub region: Option<MlirRegion>,
}

/// MLIR attribute.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub struct MlirAttr {
    pub name: String,
    pub value: MlirAttrValue,
}

/// MLIR attribute value.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum MlirAttrValue {
    Integer(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Type(MlirType),
    Array(Vec<MlirAttrValue>),
    MemoryOrdering(MemoryOrdering),
}

/// MLIR type representation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum MlirType {
    I1,
    I8,
    I16,
    I32,
    I64,
    I128,
    F32,
    F64,
    Ptr,
    Vector(Box<MlirType>, usize),
    Struct(Vec<MlirType>),
    /// Tensor type with element type and shape (empty vec = dynamic shape).
    Tensor { elem: Box<MlirType>, shape: Vec<usize> },
    /// Complex type with underlying float type (F32 or F64).
    Complex(Box<MlirType>),
    /// MemRef type with element type and shape (for memory references).
    MemRef { elem: Box<MlirType>, shape: Vec<usize> },
}

/// MLIR region (for control flow).
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub struct MlirRegion {
    pub blocks: Vec<MlirBlock>,
}

/// MLIR basic block.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub struct MlirBlock {
    pub args: Vec<MlirType>,
    pub ops: Vec<MlirOp>,
}

/// Memory ordering for atomic operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum MemoryOrdering {
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

impl MemoryOrdering {
    /// Convert from Verum ordering constant.
    pub fn from_verum(ordering: u8) -> Self {
        match ordering {
            0 => MemoryOrdering::Relaxed,
            1 => MemoryOrdering::Acquire,
            2 => MemoryOrdering::Release,
            3 => MemoryOrdering::AcqRel,
            4 => MemoryOrdering::SeqCst,
            _ => MemoryOrdering::SeqCst, // Default to strongest
        }
    }

    /// Convert to LLVM ordering attribute string.
    pub fn to_llvm_attr(&self) -> &'static str {
        match self {
            MemoryOrdering::Relaxed => "monotonic",
            MemoryOrdering::Acquire => "acquire",
            MemoryOrdering::Release => "release",
            MemoryOrdering::AcqRel => "acq_rel",
            MemoryOrdering::SeqCst => "seq_cst",
        }
    }
}

/// Intrinsic MLIR lowering context.
pub struct IntrinsicLowering {
    /// Current SSA value index.
    next_value: usize,
    /// Generated operations.
    ops: Vec<MlirOp>,
    /// Target triple for platform-specific lowering.
    target: TargetTriple,
}

/// Target triple for platform-specific lowering.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub struct TargetTriple {
    pub arch: Arch,
    pub os: Os,
    pub features: Vec<String>,
}

/// Target architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Arch {
    X86_64,
    Aarch64,
    Riscv64,
    Wasm32,
}

/// Target operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Os {
    Linux,
    MacOS,
    Windows,
    Wasm,
}

impl Default for TargetTriple {
    fn default() -> Self {
        Self {
            #[cfg(target_arch = "x86_64")]
            arch: Arch::X86_64,
            #[cfg(target_arch = "aarch64")]
            arch: Arch::Aarch64,
            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            arch: Arch::X86_64,

            #[cfg(target_os = "linux")]
            os: Os::Linux,
            #[cfg(target_os = "macos")]
            os: Os::MacOS,
            #[cfg(target_os = "windows")]
            os: Os::Windows,
            #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
            os: Os::Linux,

            features: vec![],
        }
    }
}

/// Result of intrinsic lowering.
#[derive(Debug)]
pub struct LoweringResult {
    /// Generated MLIR operations.
    pub ops: Vec<MlirOp>,
    /// SSA value index of the result (if any).
    pub result: Option<usize>,
}

impl IntrinsicLowering {
    /// Create a new lowering context.
    pub fn new(target: TargetTriple) -> Self {
        Self {
            next_value: 0,
            ops: Vec::new(),
            target,
        }
    }

    /// Allocate a new SSA value.
    fn alloc_value(&mut self) -> usize {
        let v = self.next_value;
        self.next_value += 1;
        v
    }

    /// Emit an MLIR operation and return its result value index.
    fn emit(&mut self, op: MlirOp) -> Option<usize> {
        let has_result = !op.result_types.is_empty();
        self.ops.push(op);
        if has_result {
            Some(self.alloc_value())
        } else {
            None
        }
    }

    /// Lower an intrinsic to MLIR.
    pub fn lower(
        mut self,
        intrinsic: &Intrinsic,
        operands: &[usize],
    ) -> LoweringResult {
        let result = match &intrinsic.strategy {
            CodegenStrategy::DirectOpcode(opcode) => {
                self.lower_direct_opcode(intrinsic, *opcode, operands)
            }
            CodegenStrategy::OpcodeWithMode(opcode, mode) => {
                self.lower_opcode_with_mode(intrinsic, *opcode, *mode, operands)
            }
            CodegenStrategy::OpcodeWithSize(opcode, size) => {
                self.lower_opcode_with_size(intrinsic, *opcode, *size, operands)
            }
            CodegenStrategy::InlineSequence(seq_id) => {
                self.lower_inline_sequence(*seq_id, operands)
            }
            CodegenStrategy::InlineSequenceWithWidth(seq_id, _width) => {
                self.lower_inline_sequence(*seq_id, operands)
            }
            CodegenStrategy::CompileTimeConstant => {
                self.lower_compile_time_constant(intrinsic)
            }
            CodegenStrategy::ArithExtendedOpcode(sub_op) => {
                self.lower_arith_extended_opcode(intrinsic, *sub_op, operands)
            }
            CodegenStrategy::MathExtendedOpcode(sub_op) => {
                self.lower_math_extended_opcode(intrinsic, *sub_op, operands)
            }
            CodegenStrategy::WrappingOpcode(sub_op, width, signed) => {
                self.lower_wrapping_opcode(intrinsic, *sub_op, *width, *signed, operands)
            }
            CodegenStrategy::SaturatingOpcode(sub_op, width, signed) => {
                self.lower_saturating_opcode(intrinsic, *sub_op, *width, *signed, operands)
            }
            CodegenStrategy::TensorExtendedOpcode(sub_op) => {
                self.lower_tensor_extended_opcode(intrinsic, *sub_op, operands)
            }
            CodegenStrategy::TensorExtendedOpcodeWithMode(sub_op, mode) => {
                self.lower_tensor_extended_opcode_with_mode(intrinsic, *sub_op, *mode, operands)
            }
            CodegenStrategy::TensorExtExtendedOpcode(sub_op) => {
                self.lower_tensor_ext_extended_opcode(intrinsic, *sub_op, operands)
            }
            CodegenStrategy::GpuExtendedOpcode(sub_op) => {
                self.lower_gpu_extended_opcode(intrinsic, *sub_op, operands)
            }
        };

        LoweringResult {
            ops: self.ops,
            result,
        }
    }

    /// Lower direct opcode to MLIR.
    fn lower_direct_opcode(
        &mut self,
        intrinsic: &Intrinsic,
        opcode: Opcode,
        operands: &[usize],
    ) -> Option<usize> {
        // Use the MLIR op name from the intrinsic definition
        let mlir_op = intrinsic.mlir_op?;
        let result_type = self.infer_result_type(intrinsic);

        match opcode {
            // Arithmetic operations -> arith dialect
            Opcode::AddI => {
                self.emit(MlirOp {
                    name: "arith.addi".to_string(),
                    attrs: vec![],
                    result_types: vec![result_type],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::SubI => {
                self.emit(MlirOp {
                    name: "arith.subi".to_string(),
                    attrs: vec![],
                    result_types: vec![result_type],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::MulI => {
                self.emit(MlirOp {
                    name: "arith.muli".to_string(),
                    attrs: vec![],
                    result_types: vec![result_type],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::NegI => {
                self.emit(MlirOp {
                    name: "arith.negsi".to_string(),
                    attrs: vec![],
                    result_types: vec![result_type],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::Shl => {
                self.emit(MlirOp {
                    name: "arith.shli".to_string(),
                    attrs: vec![],
                    result_types: vec![result_type],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::Shr => {
                self.emit(MlirOp {
                    name: "arith.shrsi".to_string(),
                    attrs: vec![],
                    result_types: vec![result_type],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Float operations
            Opcode::PowF => {
                self.emit(MlirOp {
                    name: "math.powf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::AbsF => {
                self.emit(MlirOp {
                    name: "math.absf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Memory operations
            Opcode::Deref => {
                self.emit(MlirOp {
                    name: "llvm.load".to_string(),
                    attrs: vec![],
                    result_types: vec![result_type],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::DerefMut => {
                self.emit(MlirOp {
                    name: "llvm.store".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // TLS operations
            Opcode::TlsGet => {
                // Platform-specific TLS access
                match self.target.arch {
                    Arch::X86_64 => {
                        // Read from FS segment on Linux, GS on Windows
                        self.emit(MlirOp {
                            name: "llvm.intr.read.register".to_string(),
                            attrs: vec![MlirAttr {
                                name: "name".to_string(),
                                value: MlirAttrValue::String(
                                    if self.target.os == Os::Linux { "fs" } else { "gs" }.to_string()
                                ),
                            }],
                            result_types: vec![MlirType::Ptr],
                            operands: operands.to_vec(),
                            region: None,
                        })
                    }
                    Arch::Aarch64 => {
                        self.emit(MlirOp {
                            name: "llvm.intr.read.register".to_string(),
                            attrs: vec![MlirAttr {
                                name: "name".to_string(),
                                value: MlirAttrValue::String("tpidr_el0".to_string()),
                            }],
                            result_types: vec![MlirType::Ptr],
                            operands: operands.to_vec(),
                            region: None,
                        })
                    }
                    _ => {
                        // Generic: use llvm.thread.local
                        self.emit(MlirOp {
                            name: "llvm.mlir.addressof".to_string(),
                            attrs: vec![MlirAttr {
                                name: "global_name".to_string(),
                                value: MlirAttrValue::String("__verum_tls_base".to_string()),
                            }],
                            result_types: vec![MlirType::Ptr],
                            operands: vec![],
                            region: None,
                        })
                    }
                }
            }

            // Context operations
            Opcode::PushContext | Opcode::PopContext => {
                self.emit(MlirOp {
                    name: mlir_op.to_string(),
                    attrs: vec![],
                    result_types: if intrinsic.return_count > 0 {
                        vec![MlirType::Ptr]
                    } else {
                        vec![]
                    },
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Control flow
            Opcode::Panic => {
                self.emit(MlirOp {
                    name: "llvm.trap".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::Unreachable => {
                self.emit(MlirOp {
                    name: "llvm.unreachable".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: vec![],
                    region: None,
                })
            }

            // Generic size operations
            Opcode::SizeOfG => {
                self.emit(MlirOp {
                    name: "llvm.mlir.constant".to_string(),
                    attrs: vec![MlirAttr {
                        name: "value".to_string(),
                        // Size will be resolved during monomorphization
                        value: MlirAttrValue::Integer(0),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: vec![],
                    region: None,
                })
            }
            Opcode::AlignOfG => {
                self.emit(MlirOp {
                    name: "llvm.mlir.constant".to_string(),
                    attrs: vec![MlirAttr {
                        name: "value".to_string(),
                        value: MlirAttrValue::Integer(0),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: vec![],
                    region: None,
                })
            }

            // Default: use intrinsic's MLIR op
            _ => {
                self.emit(MlirOp {
                    name: mlir_op.to_string(),
                    attrs: vec![],
                    result_types: if intrinsic.return_count > 0 {
                        vec![result_type]
                    } else {
                        vec![]
                    },
                    operands: operands.to_vec(),
                    region: None,
                })
            }
        }
    }

    /// Lower opcode with mode to MLIR.
    fn lower_opcode_with_mode(
        &mut self,
        intrinsic: &Intrinsic,
        opcode: Opcode,
        mode: u8,
        operands: &[usize],
    ) -> Option<usize> {
        match opcode {
            Opcode::AtomicFence => {
                if mode == 0xFF {
                    // spin_hint -> x86 PAUSE or ARM YIELD
                    match self.target.arch {
                        Arch::X86_64 => {
                            self.emit(MlirOp {
                                name: "llvm.intr.x86.sse2.pause".to_string(),
                                attrs: vec![],
                                result_types: vec![],
                                operands: vec![],
                                region: None,
                            })
                        }
                        Arch::Aarch64 => {
                            self.emit(MlirOp {
                                name: "llvm.intr.aarch64.hint".to_string(),
                                attrs: vec![MlirAttr {
                                    name: "hint".to_string(),
                                    value: MlirAttrValue::Integer(1), // YIELD
                                }],
                                result_types: vec![],
                                operands: vec![],
                                region: None,
                            })
                        }
                        _ => {
                            // Fallback: compiler fence
                            self.emit(MlirOp {
                                name: "llvm.compiler.fence".to_string(),
                                attrs: vec![],
                                result_types: vec![],
                                operands: vec![],
                                region: None,
                            })
                        }
                    }
                } else {
                    let ordering = MemoryOrdering::from_verum(mode);
                    self.emit(MlirOp {
                        name: "llvm.fence".to_string(),
                        attrs: vec![MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::MemoryOrdering(ordering),
                        }],
                        result_types: vec![],
                        operands: vec![],
                        region: None,
                    })
                }
            }
            Opcode::SyscallLinux => {
                // Lower to inline assembly for syscall
                let argc = mode as usize;
                self.emit(MlirOp {
                    name: "llvm.inline_asm".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "asm".to_string(),
                            value: MlirAttrValue::String(
                                match self.target.arch {
                                    Arch::X86_64 => "syscall".to_string(),
                                    Arch::Aarch64 => "svc #0".to_string(),
                                    _ => "syscall".to_string(),
                                }
                            ),
                        },
                        MlirAttr {
                            name: "constraints".to_string(),
                            value: MlirAttrValue::String(
                                self.syscall_constraints(argc)
                            ),
                        },
                    ],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::CvtFI => {
                // mode: 0=trunc, 1=floor, 2=ceil, 3=round
                let op_name = match mode {
                    0 => "llvm.intr.trunc",
                    1 => "math.floor",
                    2 => "math.ceil",
                    3 => "llvm.intr.round",
                    _ => "llvm.intr.trunc",
                };
                self.emit(MlirOp {
                    name: op_name.to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            _ => {
                // Generic lowering
                if let Some(mlir_op) = intrinsic.mlir_op {
                    self.emit(MlirOp {
                        name: mlir_op.to_string(),
                        attrs: vec![MlirAttr {
                            name: "mode".to_string(),
                            value: MlirAttrValue::Integer(mode as i64),
                        }],
                        result_types: if intrinsic.return_count > 0 {
                            vec![self.infer_result_type(intrinsic)]
                        } else {
                            vec![]
                        },
                        operands: operands.to_vec(),
                        region: None,
                    })
                } else {
                    None
                }
            }
        }
    }

    /// Lower opcode with size to MLIR.
    fn lower_opcode_with_size(
        &mut self,
        intrinsic: &Intrinsic,
        opcode: Opcode,
        size: u8,
        operands: &[usize],
    ) -> Option<usize> {
        let elem_type = match size {
            1 => MlirType::I8,
            2 => MlirType::I16,
            4 => MlirType::I32,
            8 => MlirType::I64,
            16 => MlirType::I128,
            _ => MlirType::I64,
        };

        match opcode {
            Opcode::AtomicLoad => {
                // llvm.load with atomic attribute
                self.emit(MlirOp {
                    name: "llvm.load".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::String("acquire".to_string()),
                        },
                        MlirAttr {
                            name: "volatile_".to_string(),
                            value: MlirAttrValue::Bool(false),
                        },
                    ],
                    result_types: vec![elem_type],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::AtomicStore => {
                self.emit(MlirOp {
                    name: "llvm.store".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::String("release".to_string()),
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Opcode::AtomicCas => {
                // llvm.cmpxchg returns {T, i1}
                self.emit(MlirOp {
                    name: "llvm.cmpxchg".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "success_ordering".to_string(),
                            value: MlirAttrValue::String("seq_cst".to_string()),
                        },
                        MlirAttr {
                            name: "failure_ordering".to_string(),
                            value: MlirAttrValue::String("seq_cst".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::Struct(vec![elem_type, MlirType::I1])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            _ => {
                // Generic with size attribute
                if let Some(mlir_op) = intrinsic.mlir_op {
                    self.emit(MlirOp {
                        name: mlir_op.to_string(),
                        attrs: vec![MlirAttr {
                            name: "size".to_string(),
                            value: MlirAttrValue::Integer(size as i64),
                        }],
                        result_types: if intrinsic.return_count > 0 {
                            vec![elem_type]
                        } else {
                            vec![]
                        },
                        operands: operands.to_vec(),
                        region: None,
                    })
                } else {
                    None
                }
            }
        }
    }

    /// Lower inline sequence to MLIR.
    fn lower_inline_sequence(
        &mut self,
        seq_id: InlineSequenceId,
        operands: &[usize],
    ) -> Option<usize> {
        match seq_id {
            InlineSequenceId::Memcpy => {
                self.emit(MlirOp {
                    name: "llvm.intr.memcpy".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "isVolatile".to_string(),
                            value: MlirAttrValue::Bool(false),
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Memmove => {
                self.emit(MlirOp {
                    name: "llvm.intr.memmove".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "isVolatile".to_string(),
                            value: MlirAttrValue::Bool(false),
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Memset => {
                self.emit(MlirOp {
                    name: "llvm.intr.memset".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "isVolatile".to_string(),
                            value: MlirAttrValue::Bool(false),
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Memcmp => {
                // llvm.memcmp or custom comparison loop
                self.emit(MlirOp {
                    name: "llvm.intr.memcmp".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Clz | InlineSequenceId::Ilog2 => {
                self.emit(MlirOp {
                    name: "llvm.intr.ctlz".to_string(),
                    attrs: vec![MlirAttr {
                        name: "is_zero_poison".to_string(),
                        value: MlirAttrValue::Bool(false),
                    }],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Ctz => {
                self.emit(MlirOp {
                    name: "llvm.intr.cttz".to_string(),
                    attrs: vec![MlirAttr {
                        name: "is_zero_poison".to_string(),
                        value: MlirAttrValue::Bool(false),
                    }],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Popcnt => {
                self.emit(MlirOp {
                    name: "llvm.intr.ctpop".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Bswap => {
                self.emit(MlirOp {
                    name: "llvm.intr.bswap".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::RotateLeft => {
                self.emit(MlirOp {
                    name: "llvm.intr.fshl".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::RotateRight => {
                self.emit(MlirOp {
                    name: "llvm.intr.fshr".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CheckedAdd => {
                self.emit(MlirOp {
                    name: "llvm.intr.sadd.with.overflow".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Struct(vec![MlirType::I64, MlirType::I1])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CheckedSub => {
                self.emit(MlirOp {
                    name: "llvm.intr.ssub.with.overflow".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Struct(vec![MlirType::I64, MlirType::I1])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CheckedMul => {
                self.emit(MlirOp {
                    name: "llvm.intr.smul.with.overflow".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Struct(vec![MlirType::I64, MlirType::I1])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CheckedDiv => {
                // Division doesn't have an intrinsic, use sdiv with guard for zero/overflow
                self.emit(MlirOp {
                    name: "arith.divsi".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::OverflowingAdd => {
                self.emit(MlirOp {
                    name: "llvm.intr.sadd.with.overflow".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Struct(vec![MlirType::I64, MlirType::I1])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::OverflowingSub => {
                self.emit(MlirOp {
                    name: "llvm.intr.ssub.with.overflow".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Struct(vec![MlirType::I64, MlirType::I1])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::OverflowingMul => {
                self.emit(MlirOp {
                    name: "llvm.intr.smul.with.overflow".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Struct(vec![MlirType::I64, MlirType::I1])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AtomicFetchAdd => {
                self.emit(MlirOp {
                    name: "llvm.atomicrmw".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "bin_op".to_string(),
                            value: MlirAttrValue::String("add".to_string()),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::String("seq_cst".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AtomicFetchSub => {
                self.emit(MlirOp {
                    name: "llvm.atomicrmw".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "bin_op".to_string(),
                            value: MlirAttrValue::String("sub".to_string()),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::String("seq_cst".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AtomicFetchAnd => {
                self.emit(MlirOp {
                    name: "llvm.atomicrmw".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "bin_op".to_string(),
                            value: MlirAttrValue::String("and".to_string()),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::String("seq_cst".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AtomicFetchOr => {
                self.emit(MlirOp {
                    name: "llvm.atomicrmw".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "bin_op".to_string(),
                            value: MlirAttrValue::String("or".to_string()),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::String("seq_cst".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AtomicFetchXor => {
                self.emit(MlirOp {
                    name: "llvm.atomicrmw".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "bin_op".to_string(),
                            value: MlirAttrValue::String("xor".to_string()),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::String("seq_cst".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Math functions -> math dialect
            InlineSequenceId::SinF64 => {
                self.emit(MlirOp {
                    name: "math.sin".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CosF64 => {
                self.emit(MlirOp {
                    name: "math.cos".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::TanF64 => {
                self.emit(MlirOp {
                    name: "math.tan".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AsinF64 => {
                self.emit(MlirOp {
                    name: "math.asin".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AcosF64 => {
                self.emit(MlirOp {
                    name: "math.acos".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AtanF64 => {
                self.emit(MlirOp {
                    name: "math.atan".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Atan2F64 => {
                self.emit(MlirOp {
                    name: "math.atan2".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::ExpF64 => {
                self.emit(MlirOp {
                    name: "math.exp".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::LogF64 => {
                self.emit(MlirOp {
                    name: "math.log".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Log10F64 => {
                self.emit(MlirOp {
                    name: "math.log10".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Time intrinsics -> platform syscall or VDSO
            InlineSequenceId::MonotonicNanos => {
                self.lower_time_intrinsic("monotonic", operands)
            }
            InlineSequenceId::RealtimeSecs => {
                self.lower_time_intrinsic("realtime", operands)
            }
            InlineSequenceId::RealtimeNanos => {
                self.lower_time_intrinsic("realtime_nanos", operands)
            }
            // Futex/spinlock -> platform-specific
            InlineSequenceId::SpinlockLock => {
                self.lower_spinlock_lock(operands)
            }
            InlineSequenceId::FutexWait => {
                self.lower_futex_wait(operands)
            }
            InlineSequenceId::FutexWake => {
                self.lower_futex_wake(operands)
            }
            // Memory lifecycle intrinsics
            InlineSequenceId::DropInPlace => {
                // Call destructor via LLVM lifetime end + custom drop
                if !operands.is_empty() {
                    self.emit(MlirOp {
                        name: "verum.drop".to_string(),
                        attrs: vec![],
                        result_types: vec![],
                        operands: operands.to_vec(),
                        region: None,
                    })
                } else {
                    None
                }
            }
            InlineSequenceId::MakeSlice => {
                // Pack ptr and len into LLVM fat pointer struct
                self.emit(MlirOp {
                    name: "llvm.mlir.undef".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Struct(vec![
                        MlirType::Ptr,
                        MlirType::I64,
                    ])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Uninit => {
                // Return undef value (uninitialized)
                self.emit(MlirOp {
                    name: "llvm.mlir.undef".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64], // placeholder
                    operands: vec![],
                    region: None,
                })
            }
            InlineSequenceId::Zeroed => {
                // Return zeroinitializer
                self.emit(MlirOp {
                    name: "llvm.mlir.zero".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64], // placeholder
                    operands: vec![],
                    region: None,
                })
            }

            // F64 extended math functions - use LLVM intrinsics (NO LIBC dependency)
            // LLVM will inline or use hardware instructions where available
            InlineSequenceId::CbrtF64 => self.lower_llvm_intrinsic("llvm.cbrt.f64", operands, MlirType::F64),
            InlineSequenceId::Expm1F64 => self.lower_llvm_intrinsic("llvm.expm1.f64", operands, MlirType::F64),
            InlineSequenceId::Exp2F64 => self.lower_llvm_intrinsic("llvm.exp2.f64", operands, MlirType::F64),
            InlineSequenceId::Log1pF64 => self.lower_llvm_intrinsic("llvm.log1p.f64", operands, MlirType::F64),
            InlineSequenceId::Log2F64 => self.lower_llvm_intrinsic("llvm.log2.f64", operands, MlirType::F64),
            InlineSequenceId::PowiF64 => self.lower_llvm_intrinsic("llvm.powi.f64.i32", operands, MlirType::F64),
            InlineSequenceId::TruncF64 => self.lower_llvm_intrinsic("llvm.trunc.f64", operands, MlirType::F64),
            InlineSequenceId::MinnumF64 => self.lower_llvm_intrinsic("llvm.minnum.f64", operands, MlirType::F64),
            InlineSequenceId::MaxnumF64 => self.lower_llvm_intrinsic("llvm.maxnum.f64", operands, MlirType::F64),
            InlineSequenceId::FmaF64 => self.lower_llvm_intrinsic("llvm.fma.f64", operands, MlirType::F64),
            InlineSequenceId::CopysignF64 => self.lower_llvm_intrinsic("llvm.copysign.f64", operands, MlirType::F64),
            InlineSequenceId::HypotF64 => self.lower_llvm_intrinsic("llvm.hypot.f64", operands, MlirType::F64),
            // F64 hyperbolic functions - LLVM intrinsics
            InlineSequenceId::SinhF64 => self.lower_llvm_intrinsic("llvm.sinh.f64", operands, MlirType::F64),
            InlineSequenceId::CoshF64 => self.lower_llvm_intrinsic("llvm.cosh.f64", operands, MlirType::F64),
            InlineSequenceId::TanhF64 => self.lower_llvm_intrinsic("llvm.tanh.f64", operands, MlirType::F64),
            InlineSequenceId::AsinhF64 => self.lower_llvm_intrinsic("llvm.asinh.f64", operands, MlirType::F64),
            InlineSequenceId::AcoshF64 => self.lower_llvm_intrinsic("llvm.acosh.f64", operands, MlirType::F64),
            InlineSequenceId::AtanhF64 => self.lower_llvm_intrinsic("llvm.atanh.f64", operands, MlirType::F64),
            // F64 power and basic functions - LLVM intrinsics
            InlineSequenceId::PowF64 => self.lower_llvm_intrinsic("llvm.pow.f64", operands, MlirType::F64),
            InlineSequenceId::AbsF64 => self.lower_llvm_intrinsic("llvm.fabs.f64", operands, MlirType::F64),
            InlineSequenceId::FloorF64 => self.lower_llvm_intrinsic("llvm.floor.f64", operands, MlirType::F64),
            InlineSequenceId::CeilF64 => self.lower_llvm_intrinsic("llvm.ceil.f64", operands, MlirType::F64),
            InlineSequenceId::RoundF64 => self.lower_llvm_intrinsic("llvm.round.f64", operands, MlirType::F64),
            InlineSequenceId::SqrtF64 => self.lower_llvm_intrinsic("llvm.sqrt.f64", operands, MlirType::F64),

            // Bit manipulation extensions
            InlineSequenceId::Bitreverse => {
                self.emit(MlirOp {
                    name: "llvm.intr.bitreverse".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Type conversions
            InlineSequenceId::IntToFloat => {
                self.emit(MlirOp {
                    name: "arith.sitofp".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::FloatToInt => {
                self.emit(MlirOp {
                    name: "arith.fptosi".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Sext => {
                self.emit(MlirOp {
                    name: "arith.extsi".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Zext => {
                self.emit(MlirOp {
                    name: "arith.extui".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Fpext => {
                self.emit(MlirOp {
                    name: "arith.extf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Fptrunc => {
                self.emit(MlirOp {
                    name: "arith.truncf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::IntTrunc => {
                self.emit(MlirOp {
                    name: "arith.trunci".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Bitcast |
            InlineSequenceId::F32ToBits |
            InlineSequenceId::F32FromBits |
            InlineSequenceId::F64ToBits |
            InlineSequenceId::F64FromBits => {
                self.emit(MlirOp {
                    name: "arith.bitcast".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Byte conversions - use type punning
            InlineSequenceId::ToLeBytes |
            InlineSequenceId::FromLeBytes |
            InlineSequenceId::ToBeBytes |
            InlineSequenceId::FromBeBytes => {
                // Byte reinterpretation - emit as bitcast
                self.emit(MlirOp {
                    name: "arith.bitcast".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Char operations - NO LIBC: use inline ASCII comparisons
            // ASCII-only implementation for Tier 1 AOT (Unicode handled by /core)
            InlineSequenceId::CharIsAlphabetic => {
                // isalpha: (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z')
                self.lower_char_is_alpha(operands)
            }
            InlineSequenceId::CharIsNumeric => {
                // isdigit: c >= '0' && c <= '9'
                self.lower_char_is_digit(operands)
            }
            InlineSequenceId::CharIsWhitespace => {
                // isspace: c in {' ', '\t', '\n', '\r', '\f', '\v'}
                self.lower_char_is_space(operands)
            }
            InlineSequenceId::CharIsControl => {
                // iscntrl: c < 32 || c == 127
                self.lower_char_is_control(operands)
            }
            InlineSequenceId::CharIsUppercase => {
                // isupper: c >= 'A' && c <= 'Z'
                self.lower_char_is_upper(operands)
            }
            InlineSequenceId::CharIsLowercase => {
                // islower: c >= 'a' && c <= 'z'
                self.lower_char_is_lower(operands)
            }
            InlineSequenceId::CharToUppercase => {
                // toupper: if islower(c) then c - 32 else c
                self.lower_char_to_upper(operands)
            }
            InlineSequenceId::CharToLowercase => {
                // tolower: if isupper(c) then c + 32 else c
                self.lower_char_to_lower(operands)
            }
            InlineSequenceId::CharEncodeUtf8 |
            InlineSequenceId::CharEscapeDebug => {
                // Complex char operations - use library calls
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("verum_char_encode_utf8".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // F32 basic operations - use LLVM intrinsics (no libc dependency)
            // LLVM will either inline the implementation or use hardware instructions
            InlineSequenceId::SqrtF32 => self.lower_llvm_intrinsic("llvm.sqrt.f32", operands, MlirType::F32),
            InlineSequenceId::FloorF32 => self.lower_llvm_intrinsic("llvm.floor.f32", operands, MlirType::F32),
            InlineSequenceId::CeilF32 => self.lower_llvm_intrinsic("llvm.ceil.f32", operands, MlirType::F32),
            InlineSequenceId::RoundF32 => self.lower_llvm_intrinsic("llvm.round.f32", operands, MlirType::F32),
            InlineSequenceId::TruncF32 => self.lower_llvm_intrinsic("llvm.trunc.f32", operands, MlirType::F32),
            InlineSequenceId::AbsF32 => self.lower_llvm_intrinsic("llvm.fabs.f32", operands, MlirType::F32),
            // F32 trigonometric functions - LLVM intrinsics
            InlineSequenceId::SinF32 => self.lower_llvm_intrinsic("llvm.sin.f32", operands, MlirType::F32),
            InlineSequenceId::CosF32 => self.lower_llvm_intrinsic("llvm.cos.f32", operands, MlirType::F32),
            InlineSequenceId::TanF32 => self.lower_llvm_intrinsic("llvm.tan.f32", operands, MlirType::F32),
            InlineSequenceId::AsinF32 => self.lower_llvm_intrinsic("llvm.asin.f32", operands, MlirType::F32),
            InlineSequenceId::AcosF32 => self.lower_llvm_intrinsic("llvm.acos.f32", operands, MlirType::F32),
            InlineSequenceId::AtanF32 => self.lower_llvm_intrinsic("llvm.atan.f32", operands, MlirType::F32),
            InlineSequenceId::Atan2F32 => self.lower_llvm_intrinsic("llvm.atan2.f32", operands, MlirType::F32),
            // F32 hyperbolic functions - LLVM intrinsics
            InlineSequenceId::SinhF32 => self.lower_llvm_intrinsic("llvm.sinh.f32", operands, MlirType::F32),
            InlineSequenceId::CoshF32 => self.lower_llvm_intrinsic("llvm.cosh.f32", operands, MlirType::F32),
            InlineSequenceId::TanhF32 => self.lower_llvm_intrinsic("llvm.tanh.f32", operands, MlirType::F32),
            InlineSequenceId::AsinhF32 => self.lower_llvm_intrinsic("llvm.asinh.f32", operands, MlirType::F32),
            InlineSequenceId::AcoshF32 => self.lower_llvm_intrinsic("llvm.acosh.f32", operands, MlirType::F32),
            InlineSequenceId::AtanhF32 => self.lower_llvm_intrinsic("llvm.atanh.f32", operands, MlirType::F32),
            // F32 exponential and logarithmic functions - LLVM intrinsics
            InlineSequenceId::ExpF32 => self.lower_llvm_intrinsic("llvm.exp.f32", operands, MlirType::F32),
            InlineSequenceId::Exp2F32 => self.lower_llvm_intrinsic("llvm.exp2.f32", operands, MlirType::F32),
            InlineSequenceId::Expm1F32 => self.lower_llvm_intrinsic("llvm.expm1.f32", operands, MlirType::F32),
            InlineSequenceId::LogF32 => self.lower_llvm_intrinsic("llvm.log.f32", operands, MlirType::F32),
            InlineSequenceId::Log2F32 => self.lower_llvm_intrinsic("llvm.log2.f32", operands, MlirType::F32),
            InlineSequenceId::Log10F32 => self.lower_llvm_intrinsic("llvm.log10.f32", operands, MlirType::F32),
            InlineSequenceId::Log1pF32 => self.lower_llvm_intrinsic("llvm.log1p.f32", operands, MlirType::F32),
            // F32 power and special functions - LLVM intrinsics
            InlineSequenceId::CbrtF32 => self.lower_llvm_intrinsic("llvm.cbrt.f32", operands, MlirType::F32),
            InlineSequenceId::HypotF32 => self.lower_llvm_intrinsic("llvm.hypot.f32", operands, MlirType::F32),
            InlineSequenceId::FmaF32 => self.lower_llvm_intrinsic("llvm.fma.f32", operands, MlirType::F32),
            InlineSequenceId::CopysignF32 => self.lower_llvm_intrinsic("llvm.copysign.f32", operands, MlirType::F32),
            InlineSequenceId::PowiF32 => self.lower_llvm_intrinsic("llvm.powi.f32.i32", operands, MlirType::F32),
            InlineSequenceId::MinnumF32 => self.lower_llvm_intrinsic("llvm.minnum.f32", operands, MlirType::F32),
            InlineSequenceId::MaxnumF32 => self.lower_llvm_intrinsic("llvm.maxnum.f32", operands, MlirType::F32),

            // Saturating arithmetic
            InlineSequenceId::SaturatingAdd => {
                self.emit(MlirOp {
                    name: "llvm.intr.sadd.sat".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::SaturatingSub => {
                self.emit(MlirOp {
                    name: "llvm.intr.ssub.sat".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Float special checks
            InlineSequenceId::IsNan => {
                self.emit(MlirOp {
                    name: "arith.cmpf".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "predicate".to_string(),
                            value: MlirAttrValue::String("uno".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::IsInf => {
                // NO LIBC: Use LLVM's is.fpclass intrinsic to check for infinity
                // fpclass mask 0x204 = negInf (0x200) | posInf (0x004)
                self.emit(MlirOp {
                    name: "llvm.is.fpclass".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "bit".to_string(),
                            value: MlirAttrValue::Integer(0x204), // posInf | negInf
                        },
                    ],
                    result_types: vec![MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::IsFinite => {
                // NO LIBC: Use LLVM's is.fpclass intrinsic to check for finite
                // fpclass mask 0x1F8 = all finite classes (normal, subnormal, zero)
                // 0x008 posNormal | 0x010 negNormal | 0x080 posSubnormal |
                // 0x100 negSubnormal | 0x040 posZero | 0x020 negZero
                self.emit(MlirOp {
                    name: "llvm.is.fpclass".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "bit".to_string(),
                            value: MlirAttrValue::Integer(0x1F8), // all finite classes
                        },
                    ],
                    result_types: vec![MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Slice operations
            InlineSequenceId::SliceLen => {
                // Extract length from fat pointer struct
                self.emit(MlirOp {
                    name: "llvm.extractvalue".to_string(),
                    attrs: vec![MlirAttr {
                        name: "position".to_string(),
                        value: MlirAttrValue::Array(vec![MlirAttrValue::Integer(1)]), // len is second element
                    }],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::SliceAsPtr => {
                // Extract pointer from fat pointer struct
                self.emit(MlirOp {
                    name: "llvm.extractvalue".to_string(),
                    attrs: vec![MlirAttr {
                        name: "position".to_string(),
                        value: MlirAttrValue::Array(vec![MlirAttrValue::Integer(0)]), // ptr is first element
                    }],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::SliceGet | InlineSequenceId::SliceGetUnchecked => {
                // GEP + load for element access
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_slice_get".to_string()),
                    }],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::SliceSubslice => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_slice_subslice".to_string()),
                    }],
                    result_types: vec![MlirType::Struct(vec![MlirType::Ptr, MlirType::I64])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::SliceSplitAt => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_slice_split_at".to_string()),
                    }],
                    result_types: vec![MlirType::Struct(vec![
                        MlirType::Struct(vec![MlirType::Ptr, MlirType::I64]),
                        MlirType::Struct(vec![MlirType::Ptr, MlirType::I64]),
                    ])],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Text operations
            InlineSequenceId::TextFromStatic => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_text_from_static".to_string()),
                    }],
                    result_types: vec![MlirType::Ptr], // Text struct ptr
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Utf8DecodeChar => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_utf8_decode_char".to_string()),
                    }],
                    result_types: vec![MlirType::I32], // char code point
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::TextParseInt => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_text_parse_int".to_string()),
                    }],
                    result_types: vec![MlirType::Struct(vec![MlirType::I64, MlirType::I1])], // Maybe<Int>
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::TextParseFloat => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_text_parse_float".to_string()),
                    }],
                    result_types: vec![MlirType::Struct(vec![MlirType::F64, MlirType::I1])], // Maybe<Float>
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::IntToText => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_int_to_text".to_string()),
                    }],
                    result_types: vec![MlirType::Ptr], // Text struct ptr
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::FloatToText => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_float_to_text".to_string()),
                    }],
                    result_types: vec![MlirType::Ptr], // Text struct ptr
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::TextByteLen => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("verum_text_byte_len".to_string()),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Random number generation
            InlineSequenceId::RandomU64 => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("__verum_random_u64".to_string()),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: vec![],
                    region: None,
                })
            }
            InlineSequenceId::RandomFloat => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("__verum_random_float".to_string()),
                    }],
                    result_types: vec![MlirType::F64],
                    operands: vec![],
                    region: None,
                })
            }

            // Unicode character classification
            InlineSequenceId::CharGeneralCategory => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("__verum_char_general_category".to_string()),
                    }],
                    result_types: vec![MlirType::I32], // UnicodeCategory enum
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Global allocator access
            InlineSequenceId::GlobalAllocator => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("__verum_global_allocator".to_string()),
                    }],
                    result_types: vec![MlirType::Ptr], // Allocator pointer
                    operands: vec![],
                    region: None,
                })
            }

            // Atomic exchange operation
            InlineSequenceId::AtomicExchange => {
                // atomicrmw xchg ptr, val ordering
                self.emit(MlirOp {
                    name: "llvm.atomicrmw".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "bin_op".to_string(),
                            value: MlirAttrValue::String("xchg".to_string()),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::String("seq_cst".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Poll pending - return false constant for Tier 0 async
            InlineSequenceId::PollPending => {
                self.emit(MlirOp {
                    name: "arith.constant".to_string(),
                    attrs: vec![MlirAttr {
                        name: "value".to_string(),
                        value: MlirAttrValue::Bool(false),
                    }],
                    result_types: vec![MlirType::I1],
                    operands: vec![],
                    region: None,
                })
            }

            // Call second argument as closure (recovery passthrough)
            InlineSequenceId::CallSecondArg => {
                // For MLIR: call the second operand (closure) with no args
                if operands.len() >= 2 {
                    self.emit(MlirOp {
                        name: "func.call_indirect".to_string(),
                        attrs: vec![],
                        result_types: vec![MlirType::I64],
                        operands: vec![operands[1]], // closure is second arg
                        region: None,
                    })
                } else {
                    // Return zero (unit equivalent) if insufficient args
                    self.emit(MlirOp {
                        name: "arith.constant".to_string(),
                        attrs: vec![MlirAttr {
                            name: "value".to_string(),
                            value: MlirAttrValue::Integer(0),
                        }],
                        result_types: vec![MlirType::I64],
                        operands: vec![],
                        region: None,
                    })
                }
            }

            // Load unit constant (represented as i64 zero)
            InlineSequenceId::LoadUnit => {
                self.emit(MlirOp {
                    name: "arith.constant".to_string(),
                    attrs: vec![MlirAttr {
                        name: "value".to_string(),
                        value: MlirAttrValue::Integer(0),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: vec![],
                    region: None,
                })
            }

            // Volatile memory operations (MMIO support)
            InlineSequenceId::VolatileLoad => {
                // llvm.load with volatile=true
                self.emit(MlirOp {
                    name: "llvm.load".to_string(),
                    attrs: vec![MlirAttr {
                        name: "volatile_".to_string(),
                        value: MlirAttrValue::Bool(true),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::VolatileStore => {
                // llvm.store with volatile=true
                self.emit(MlirOp {
                    name: "llvm.store".to_string(),
                    attrs: vec![MlirAttr {
                        name: "volatile_".to_string(),
                        value: MlirAttrValue::Bool(true),
                    }],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::VolatileLoadAcquire => {
                // llvm.load with volatile=true and ordering=acquire
                self.emit(MlirOp {
                    name: "llvm.load".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "volatile_".to_string(),
                            value: MlirAttrValue::Bool(true),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::Integer(4), // acquire
                        },
                    ],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::VolatileStoreRelease => {
                // llvm.store with volatile=true and ordering=release
                self.emit(MlirOp {
                    name: "llvm.store".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "volatile_".to_string(),
                            value: MlirAttrValue::Bool(true),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::Integer(5), // release
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::CompilerFence => {
                // llvm.fence singlethread
                self.emit(MlirOp {
                    name: "llvm.fence".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "syncscope".to_string(),
                            value: MlirAttrValue::String("singlethread".to_string()),
                        },
                        MlirAttr {
                            name: "ordering".to_string(),
                            value: MlirAttrValue::Integer(7), // seq_cst
                        },
                    ],
                    result_types: vec![],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::HardwareFence => {
                // llvm.fence
                self.emit(MlirOp {
                    name: "llvm.fence".to_string(),
                    attrs: vec![MlirAttr {
                        name: "ordering".to_string(),
                        value: MlirAttrValue::Integer(7), // seq_cst
                    }],
                    result_types: vec![],
                    operands: vec![],
                    region: None,
                })
            }

            // =====================================================================
            // SIMD Vector Operations
            // =====================================================================

            InlineSequenceId::SimdSplat => {
                // vector.splat: Broadcast scalar to all lanes
                self.emit(MlirOp {
                    name: "vector.splat".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdExtract => {
                // vector.extractelement: Extract scalar from lane
                self.emit(MlirOp {
                    name: "vector.extractelement".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdInsert => {
                // vector.insertelement: Insert scalar into lane
                self.emit(MlirOp {
                    name: "vector.insertelement".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdAdd => {
                // arith.addf for float vectors
                self.emit(MlirOp {
                    name: "arith.addf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdSub => {
                // arith.subf for float vectors
                self.emit(MlirOp {
                    name: "arith.subf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdMul => {
                // arith.mulf for float vectors
                self.emit(MlirOp {
                    name: "arith.mulf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdDiv => {
                // arith.divf for float vectors
                self.emit(MlirOp {
                    name: "arith.divf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdNeg => {
                // arith.negf for float vectors
                self.emit(MlirOp {
                    name: "arith.negf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdAbs => {
                // math.absf for float vectors
                self.emit(MlirOp {
                    name: "math.absf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdSqrt => {
                // math.sqrt for float vectors
                self.emit(MlirOp {
                    name: "math.sqrt".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdFma => {
                // math.fma: a * b + c (fused multiply-add)
                self.emit(MlirOp {
                    name: "math.fma".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdMin => {
                // arith.minnumf for float vectors
                self.emit(MlirOp {
                    name: "arith.minnumf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdMax => {
                // arith.maxnumf for float vectors
                self.emit(MlirOp {
                    name: "arith.maxnumf".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdReduceAdd => {
                // vector.reduction <add>
                self.emit(MlirOp {
                    name: "vector.reduction".to_string(),
                    attrs: vec![MlirAttr {
                        name: "kind".to_string(),
                        value: MlirAttrValue::String("add".to_string()),
                    }],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdReduceMul => {
                // vector.reduction <mul>
                self.emit(MlirOp {
                    name: "vector.reduction".to_string(),
                    attrs: vec![MlirAttr {
                        name: "kind".to_string(),
                        value: MlirAttrValue::String("mul".to_string()),
                    }],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdReduceMin => {
                // vector.reduction <minnumf>
                self.emit(MlirOp {
                    name: "vector.reduction".to_string(),
                    attrs: vec![MlirAttr {
                        name: "kind".to_string(),
                        value: MlirAttrValue::String("minnumf".to_string()),
                    }],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdReduceMax => {
                // vector.reduction <maxnumf>
                self.emit(MlirOp {
                    name: "vector.reduction".to_string(),
                    attrs: vec![MlirAttr {
                        name: "kind".to_string(),
                        value: MlirAttrValue::String("maxnumf".to_string()),
                    }],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdCmpEq => {
                // arith.cmpf oeq
                self.emit(MlirOp {
                    name: "arith.cmpf".to_string(),
                    attrs: vec![MlirAttr {
                        name: "predicate".to_string(),
                        value: MlirAttrValue::Integer(1), // oeq
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I1), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdCmpNe => {
                // arith.cmpf one
                self.emit(MlirOp {
                    name: "arith.cmpf".to_string(),
                    attrs: vec![MlirAttr {
                        name: "predicate".to_string(),
                        value: MlirAttrValue::Integer(6), // one
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I1), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdCmpLt => {
                // arith.cmpf olt
                self.emit(MlirOp {
                    name: "arith.cmpf".to_string(),
                    attrs: vec![MlirAttr {
                        name: "predicate".to_string(),
                        value: MlirAttrValue::Integer(4), // olt
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I1), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdCmpLe => {
                // arith.cmpf ole
                self.emit(MlirOp {
                    name: "arith.cmpf".to_string(),
                    attrs: vec![MlirAttr {
                        name: "predicate".to_string(),
                        value: MlirAttrValue::Integer(5), // ole
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I1), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdCmpGt => {
                // arith.cmpf ogt
                self.emit(MlirOp {
                    name: "arith.cmpf".to_string(),
                    attrs: vec![MlirAttr {
                        name: "predicate".to_string(),
                        value: MlirAttrValue::Integer(2), // ogt
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I1), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdCmpGe => {
                // arith.cmpf oge
                self.emit(MlirOp {
                    name: "arith.cmpf".to_string(),
                    attrs: vec![MlirAttr {
                        name: "predicate".to_string(),
                        value: MlirAttrValue::Integer(3), // oge
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I1), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdSelect => {
                // arith.select: condition-based lane selection
                self.emit(MlirOp {
                    name: "arith.select".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdLoadAligned => {
                // vector.load (aligned)
                self.emit(MlirOp {
                    name: "vector.load".to_string(),
                    attrs: vec![MlirAttr {
                        name: "alignment".to_string(),
                        value: MlirAttrValue::Integer(16), // 16-byte alignment for 4xf32
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdLoadUnaligned => {
                // vector.load (unaligned)
                self.emit(MlirOp {
                    name: "vector.load".to_string(),
                    attrs: vec![MlirAttr {
                        name: "alignment".to_string(),
                        value: MlirAttrValue::Integer(1), // 1-byte alignment (unaligned)
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdStoreAligned => {
                // vector.store (aligned)
                self.emit(MlirOp {
                    name: "vector.store".to_string(),
                    attrs: vec![MlirAttr {
                        name: "alignment".to_string(),
                        value: MlirAttrValue::Integer(16),
                    }],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdStoreUnaligned => {
                // vector.store (unaligned)
                self.emit(MlirOp {
                    name: "vector.store".to_string(),
                    attrs: vec![MlirAttr {
                        name: "alignment".to_string(),
                        value: MlirAttrValue::Integer(1),
                    }],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdMaskedLoad => {
                // vector.maskedload
                self.emit(MlirOp {
                    name: "vector.maskedload".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdMaskedStore => {
                // vector.maskedstore
                self.emit(MlirOp {
                    name: "vector.maskedstore".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdShuffle => {
                // vector.shuffle
                self.emit(MlirOp {
                    name: "vector.shuffle".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdGather => {
                // vector.gather
                self.emit(MlirOp {
                    name: "vector.gather".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdScatter => {
                // vector.scatter
                self.emit(MlirOp {
                    name: "vector.scatter".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdMaskAll => {
                // arith.constant with all ones mask
                self.emit(MlirOp {
                    name: "arith.constant".to_string(),
                    attrs: vec![MlirAttr {
                        name: "value".to_string(),
                        value: MlirAttrValue::Integer(-1), // All bits set
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I1), 4)],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::SimdMaskNone => {
                // arith.constant with all zeros mask
                self.emit(MlirOp {
                    name: "arith.constant".to_string(),
                    attrs: vec![MlirAttr {
                        name: "value".to_string(),
                        value: MlirAttrValue::Integer(0), // All bits clear
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I1), 4)],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::SimdMaskCount => {
                // llvm.ctpop (population count of set bits)
                self.emit(MlirOp {
                    name: "llvm.intr.ctpop".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdMaskAny => {
                // vector.reduction <or> to check if any lane is set
                self.emit(MlirOp {
                    name: "vector.reduction".to_string(),
                    attrs: vec![MlirAttr {
                        name: "kind".to_string(),
                        value: MlirAttrValue::String("or".to_string()),
                    }],
                    result_types: vec![MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdBitwiseAnd => {
                // arith.andi
                self.emit(MlirOp {
                    name: "arith.andi".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdBitwiseOr => {
                // arith.ori
                self.emit(MlirOp {
                    name: "arith.ori".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdBitwiseXor => {
                // arith.xori
                self.emit(MlirOp {
                    name: "arith.xori".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdBitwiseNot => {
                // Implemented as xori with all-ones
                self.emit(MlirOp {
                    name: "arith.xori".to_string(),
                    attrs: vec![MlirAttr {
                        name: "rhs_constant".to_string(),
                        value: MlirAttrValue::Integer(-1), // XOR with -1 = NOT
                    }],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdShiftLeft => {
                // arith.shli
                self.emit(MlirOp {
                    name: "arith.shli".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdShiftRight => {
                // arith.shrsi (signed shift right)
                self.emit(MlirOp {
                    name: "arith.shrsi".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::I32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::SimdCast => {
                // Type cast using arith.sitofp, arith.fptosi, etc.
                // The actual cast type would be determined at runtime
                self.emit(MlirOp {
                    name: "arith.sitofp".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Vector(Box::new(MlirType::F32), 4)],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // =====================================================================
            // Tensor Operations (SSM, FFT, Linear Algebra)
            // =====================================================================

            InlineSequenceId::SsmScan => {
                self.emit(MlirOp {
                    name: "verum.ssm_scan".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::MatrixExp => {
                self.emit(MlirOp {
                    name: "linalg.matrix_exp".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::MatrixInverse => {
                self.emit(MlirOp {
                    name: "linalg.inv".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::ComplexPow => {
                self.emit(MlirOp {
                    name: "complex.pow".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Complex(Box::new(MlirType::F64))],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::ComplexMul => {
                self.emit(MlirOp {
                    name: "complex.mul".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Complex(Box::new(MlirType::F64))],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::Rfft => {
                self.emit(MlirOp {
                    name: "verum.rfft".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::Complex(Box::new(MlirType::F32))), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::Irfft => {
                self.emit(MlirOp {
                    name: "verum.irfft".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::Uniform | InlineSequenceId::RandomFloat01 => {
                self.emit(MlirOp {
                    name: "verum.random.uniform".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::IsTraining => {
                self.emit(MlirOp {
                    name: "verum.is_training".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I1],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::Bincount => {
                self.emit(MlirOp {
                    name: "verum.bincount".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::I64), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::GatherNd => {
                self.emit(MlirOp {
                    name: "tensor.gather".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::ArangeUsize => {
                self.emit(MlirOp {
                    name: "verum.arange".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::I64), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorRepeat => {
                self.emit(MlirOp {
                    name: "tensor.repeat".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorTanh => {
                self.emit(MlirOp {
                    name: "math.tanh".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorSum => {
                self.emit(MlirOp {
                    name: "linalg.reduce".to_string(),
                    attrs: vec![MlirAttr {
                        name: "reduce_fn".to_string(),
                        value: MlirAttrValue::String("add".to_string()),
                    }],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorFromArray => {
                self.emit(MlirOp {
                    name: "tensor.from_elements".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // =====================================================================
            // Additional Tensor Operations
            // =====================================================================

            InlineSequenceId::TensorUnsqueeze => {
                self.emit(MlirOp {
                    name: "tensor.expand_shape".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorMaskedSelect => {
                self.emit(MlirOp {
                    name: "tensor.masked_select".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorLeakyRelu => {
                self.emit(MlirOp {
                    name: "verum.leaky_relu".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorDiag => {
                self.emit(MlirOp {
                    name: "linalg.diag".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorTriu => {
                self.emit(MlirOp {
                    name: "linalg.triu".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorTril => {
                self.emit(MlirOp {
                    name: "linalg.tril".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorNonzero => {
                self.emit(MlirOp {
                    name: "tensor.nonzero".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::I64), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorOneHot => {
                self.emit(MlirOp {
                    name: "verum.one_hot".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorSplit | InlineSequenceId::TensorSplitAt => {
                self.emit(MlirOp {
                    name: "tensor.split".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorGetScalar => {
                self.emit(MlirOp {
                    name: "tensor.extract".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorSetScalar => {
                self.emit(MlirOp {
                    name: "tensor.insert".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorContiguous | InlineSequenceId::TensorContiguousView => {
                self.emit(MlirOp {
                    name: "memref.copy".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TensorToDevice => {
                self.emit(MlirOp {
                    name: "gpu.memcpy".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // =====================================================================
            // Memory Allocation Operations
            // =====================================================================

            InlineSequenceId::MemNewId => {
                self.emit(MlirOp {
                    name: "verum.mem.new_id".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::MemAllocTensor => {
                self.emit(MlirOp {
                    name: "memref.alloc".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::MemRef { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // =====================================================================
            // Automatic Differentiation Operations
            // =====================================================================

            InlineSequenceId::GradBegin => {
                self.emit(MlirOp {
                    name: "verum.grad.begin".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64], // tape handle
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::GradEnd => {
                self.emit(MlirOp {
                    name: "verum.grad.end".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::JvpBegin => {
                self.emit(MlirOp {
                    name: "verum.jvp.begin".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::JvpEnd => {
                self.emit(MlirOp {
                    name: "verum.jvp.end".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::GradZeroTangent => {
                self.emit(MlirOp {
                    name: "verum.grad.zero_tangent".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::GradStop => {
                self.emit(MlirOp {
                    name: "verum.grad.stop".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::GradCustom => {
                self.emit(MlirOp {
                    name: "verum.grad.custom".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::GradCheckpoint => {
                self.emit(MlirOp {
                    name: "verum.grad.checkpoint".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::GradAccumulate => {
                self.emit(MlirOp {
                    name: "verum.grad.accumulate".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::GradRecompute => {
                self.emit(MlirOp {
                    name: "verum.grad.recompute".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::GradZero => {
                self.emit(MlirOp {
                    name: "verum.grad.zero".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // =====================================================================
            // CBGR Operations
            // =====================================================================

            InlineSequenceId::CbgrNewGeneration => {
                self.emit(MlirOp {
                    name: "verum.cbgr.new_generation".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::CbgrInvalidate => {
                self.emit(MlirOp {
                    name: "verum.cbgr.invalidate".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::CbgrGetGeneration => {
                self.emit(MlirOp {
                    name: "verum.cbgr.get_generation".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::CbgrAdvanceGeneration => {
                self.emit(MlirOp {
                    name: "verum.cbgr.advance_generation".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::CbgrGetEpochCaps => {
                self.emit(MlirOp {
                    name: "verum.cbgr.get_epoch_caps".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::CbgrBypassBegin => {
                self.emit(MlirOp {
                    name: "verum.cbgr.bypass_begin".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::CbgrBypassEnd => {
                self.emit(MlirOp {
                    name: "verum.cbgr.bypass_end".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: vec![],
                    region: None,
                })
            }

            InlineSequenceId::CbgrGetStats => {
                self.emit(MlirOp {
                    name: "verum.cbgr.get_stats".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr], // Returns stats struct pointer
                    operands: vec![],
                    region: None,
                })
            }

            // =====================================================================
            // Logging Operations
            // =====================================================================

            InlineSequenceId::LogInfo => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_log_info".to_string()),
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::LogWarning => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_log_warning".to_string()),
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::LogError => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_log_error".to_string()),
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::LogDebug => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_log_debug".to_string()),
                        },
                    ],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // =====================================================================
            // Regex Operations
            // =====================================================================

            InlineSequenceId::RegexFindAll => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_regex_find_all".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::RegexReplaceAll => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_regex_replace_all".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::RegexIsMatch => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_regex_is_match".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::RegexSplit => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_regex_split".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::RegexFind => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_regex_find".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::RegexReplace => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_regex_replace".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::RegexCaptures => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("__verum_regex_captures".to_string()),
                        },
                    ],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // =====================================================================
            // Permission Gating (#12 / P3.2)
            // =====================================================================
            //
            // The wire-level bridge to the runtime PermissionRouter.
            // AOT lowers this to a thin extern call into
            // `__verum_permission_check_wire(scope_tag: u32,
            // target_id: u64) -> u32` (0 = Allow, 1 = Deny).
            // The Rust-side helper holds the warm-path cache, so
            // even AOT-compiled code hits the same ≤2ns budget on
            // repeats.
            InlineSequenceId::PermissionCheckWire => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String(
                                "__verum_permission_check_wire".to_string(),
                            ),
                        },
                    ],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // =====================================================================
            // Type Introspection Operations
            // =====================================================================

            InlineSequenceId::SizeOf => {
                // Compile-time evaluated - operands[0] contains type ID
                self.emit(MlirOp {
                    name: "verum.size_of".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::AlignOf => {
                self.emit(MlirOp {
                    name: "verum.align_of".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TypeId => {
                self.emit(MlirOp {
                    name: "verum.type_id".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::TypeName => {
                self.emit(MlirOp {
                    name: "verum.type_name".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr], // Returns string pointer
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            InlineSequenceId::NeedsDrop => {
                self.emit(MlirOp {
                    name: "verum.needs_drop".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I1], // Returns bool
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Additional user-facing intrinsics
            InlineSequenceId::PowF32 => {
                self.emit(MlirOp {
                    name: "llvm.intr.pow.f32".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::F32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CharIsAlphanumeric => {
                self.emit(MlirOp {
                    name: "verum.char.is_alphanumeric".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Rdtsc => {
                self.emit(MlirOp {
                    name: "llvm.intr.readcyclecounter".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CatchUnwind => {
                self.emit(MlirOp {
                    name: "verum.catch_unwind".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::PtrToRef => {
                self.emit(MlirOp {
                    name: "verum.ptr_to_ref".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }

            // Note: F32 math intrinsics are handled earlier in this match using C library calls
            // via LLVM IR (sqrtf, sinf, cosf, etc.). MLIR math dialect would be used for GPU targets
            // but those are handled in a separate GPU lowering pass.

            // Note: SaturatingAdd, SaturatingSub, Sext, Zext are handled earlier in this match

            // Time intrinsics (Duration, Instant, Stopwatch, PerfCounter, DeadlineTimer)
            // are handled at the VBC interpreter level via FfiExtended sub-opcodes and
            // inline instruction sequences. They don't need MLIR lowering.
            InlineSequenceId::DurationFromNanos
            | InlineSequenceId::DurationFromMicros
            | InlineSequenceId::DurationFromMillis
            | InlineSequenceId::DurationFromSecs
            | InlineSequenceId::DurationAsNanos
            | InlineSequenceId::DurationAsMicros
            | InlineSequenceId::DurationAsMillis
            | InlineSequenceId::DurationAsSecs
            | InlineSequenceId::DurationSubsecNanos
            | InlineSequenceId::DurationAdd
            | InlineSequenceId::DurationSaturatingAdd
            | InlineSequenceId::DurationSaturatingSub
            | InlineSequenceId::DurationIsZero
            | InlineSequenceId::InstantNow
            | InlineSequenceId::InstantElapsed
            | InlineSequenceId::InstantDurationSince
            | InlineSequenceId::TimeMonotonicMicros
            | InlineSequenceId::TimeMonotonicMillis
            | InlineSequenceId::TimeUnixTimestamp
            | InlineSequenceId::TimeSleepMs
            | InlineSequenceId::TimeSleepUs
            | InlineSequenceId::TimeSleepDuration
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
                // Emit a no-op or pass-through for MLIR — these are VBC-only intrinsics
                if !operands.is_empty() {
                    Some(operands[0])
                } else {
                    self.emit(MlirOp {
                        name: "verum.nop".to_string(),
                        attrs: vec![],
                        result_types: vec![MlirType::I64],
                        operands: vec![],
                        region: None,
                    })
                }
            }
            // System call intrinsics -> platform syscalls
            InlineSequenceId::SysGetpid => {
                self.lower_time_intrinsic("sys.getpid", operands)
            }
            InlineSequenceId::SysGettid => {
                self.lower_time_intrinsic("sys.gettid", operands)
            }
            InlineSequenceId::SysMmap => {
                self.emit(MlirOp {
                    name: "verum.sys.mmap".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::SysMunmap => {
                self.emit(MlirOp {
                    name: "verum.sys.munmap".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::SysMadvise => {
                self.emit(MlirOp {
                    name: "verum.sys.madvise".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::SysGetentropy => {
                self.emit(MlirOp {
                    name: "verum.sys.getentropy".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Mach kernel operations (macOS) — MLIR lowering
            InlineSequenceId::MachVmAllocate => {
                self.emit(MlirOp {
                    name: "verum.mach.vm_allocate".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MachVmDeallocate => {
                self.emit(MlirOp {
                    name: "verum.mach.vm_deallocate".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MachVmProtect => {
                self.emit(MlirOp {
                    name: "verum.mach.vm_protect".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MachSemCreate => {
                self.emit(MlirOp {
                    name: "verum.mach.sem_create".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MachSemDestroy => {
                self.emit(MlirOp {
                    name: "verum.mach.sem_destroy".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MachSemSignal => {
                self.emit(MlirOp {
                    name: "verum.mach.sem_signal".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MachSemWait => {
                self.emit(MlirOp {
                    name: "verum.mach.sem_wait".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MachErrorString => {
                self.emit(MlirOp {
                    name: "verum.mach.error_string".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MachSleepUntil => {
                self.emit(MlirOp {
                    name: "verum.mach.sleep_until".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Heap memory allocation intrinsics
            InlineSequenceId::Alloc => {
                self.emit(MlirOp {
                    name: "llvm.call @malloc".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::AllocZeroed => {
                self.emit(MlirOp {
                    name: "llvm.call @calloc".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Dealloc => {
                self.emit(MlirOp {
                    name: "llvm.call @free".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Realloc => {
                self.emit(MlirOp {
                    name: "llvm.call @realloc".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Swap => {
                self.emit(MlirOp {
                    name: "verum.swap".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::Replace => {
                self.emit(MlirOp {
                    name: "verum.replace".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64], // returns old value
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::PtrOffset => {
                // Element-scaled pointer offset: ptr + count * 8
                // Lower to llvm.getelementptr with i64 element type
                self.emit(MlirOp {
                    name: "llvm.getelementptr".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "elem_type".to_string(),
                            value: MlirAttrValue::Type(MlirType::I64),
                        },
                    ],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // CBGR allocation intrinsics
            InlineSequenceId::CbgrAlloc => {
                self.emit(MlirOp {
                    name: "verum.cbgr_alloc".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CbgrAllocZeroed => {
                self.emit(MlirOp {
                    name: "verum.cbgr_alloc_zeroed".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CbgrDealloc => {
                self.emit(MlirOp {
                    name: "verum.cbgr_dealloc".to_string(),
                    attrs: vec![],
                    result_types: vec![],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::CbgrRealloc => {
                self.emit(MlirOp {
                    name: "verum.cbgr_realloc".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::MemcmpBytes => {
                self.emit(MlirOp {
                    name: "llvm.intr.memcmp".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            InlineSequenceId::GetHeaderFromPtr => {
                self.emit(MlirOp {
                    name: "verum.cbgr_get_header".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::Ptr],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
        }
    }


    /// Lower to LLVM intrinsic (no libc dependency).
    /// LLVM will either inline the implementation or use hardware instructions.
    fn lower_llvm_intrinsic(
        &mut self,
        intrinsic_name: &str,
        operands: &[usize],
        result_type: MlirType,
    ) -> Option<usize> {
        self.emit(MlirOp {
            name: intrinsic_name.to_string(),
            attrs: vec![],
            result_types: vec![result_type],
            operands: operands.to_vec(),
            region: None,
        })
    }


    // ============================================================================
    // NO LIBC: Inline ASCII character operations
    // These use pure LLVM IR comparisons, no libc dependency.
    // Unicode support is provided by /core/text module.
    // ============================================================================

    /// Helper: emit integer constant
    fn emit_const_i32(&mut self, value: i32) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.constant".to_string(),
            attrs: vec![MlirAttr {
                name: "value".to_string(),
                value: MlirAttrValue::Integer(value as i64),
            }],
            result_types: vec![MlirType::I32],
            operands: vec![],
            region: None,
        })
    }

    /// Helper: emit signed >= comparison
    fn emit_sge(&mut self, lhs: usize, rhs: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.cmpi".to_string(),
            attrs: vec![MlirAttr {
                name: "predicate".to_string(),
                value: MlirAttrValue::String("sge".to_string()),
            }],
            result_types: vec![MlirType::I1],
            operands: vec![lhs, rhs],
            region: None,
        })
    }

    /// Helper: emit signed <= comparison
    fn emit_sle(&mut self, lhs: usize, rhs: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.cmpi".to_string(),
            attrs: vec![MlirAttr {
                name: "predicate".to_string(),
                value: MlirAttrValue::String("sle".to_string()),
            }],
            result_types: vec![MlirType::I1],
            operands: vec![lhs, rhs],
            region: None,
        })
    }

    /// Helper: emit signed < comparison
    fn emit_slt(&mut self, lhs: usize, rhs: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.cmpi".to_string(),
            attrs: vec![MlirAttr {
                name: "predicate".to_string(),
                value: MlirAttrValue::String("slt".to_string()),
            }],
            result_types: vec![MlirType::I1],
            operands: vec![lhs, rhs],
            region: None,
        })
    }

    /// Helper: emit equality comparison
    fn emit_eq(&mut self, lhs: usize, rhs: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.cmpi".to_string(),
            attrs: vec![MlirAttr {
                name: "predicate".to_string(),
                value: MlirAttrValue::String("eq".to_string()),
            }],
            result_types: vec![MlirType::I1],
            operands: vec![lhs, rhs],
            region: None,
        })
    }

    /// Helper: emit boolean AND
    fn emit_and(&mut self, lhs: usize, rhs: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.andi".to_string(),
            attrs: vec![],
            result_types: vec![MlirType::I1],
            operands: vec![lhs, rhs],
            region: None,
        })
    }

    /// Helper: emit boolean OR
    fn emit_or(&mut self, lhs: usize, rhs: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.ori".to_string(),
            attrs: vec![],
            result_types: vec![MlirType::I1],
            operands: vec![lhs, rhs],
            region: None,
        })
    }

    /// Helper: emit integer subtraction
    fn emit_sub_i32(&mut self, lhs: usize, rhs: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.subi".to_string(),
            attrs: vec![],
            result_types: vec![MlirType::I32],
            operands: vec![lhs, rhs],
            region: None,
        })
    }

    /// Helper: emit integer addition
    fn emit_add_i32(&mut self, lhs: usize, rhs: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.addi".to_string(),
            attrs: vec![],
            result_types: vec![MlirType::I32],
            operands: vec![lhs, rhs],
            region: None,
        })
    }

    /// Helper: emit select (ternary)
    fn emit_select_i32(&mut self, cond: usize, true_val: usize, false_val: usize) -> Option<usize> {
        self.emit(MlirOp {
            name: "arith.select".to_string(),
            attrs: vec![],
            result_types: vec![MlirType::I32],
            operands: vec![cond, true_val, false_val],
            region: None,
        })
    }

    /// isupper: c >= 'A' && c <= 'Z'
    fn lower_char_is_upper(&mut self, operands: &[usize]) -> Option<usize> {
        let c = operands[0];
        let const_a = self.emit_const_i32('A' as i32)?;
        let const_z = self.emit_const_i32('Z' as i32)?;
        let ge_a = self.emit_sge(c, const_a)?;
        let le_z = self.emit_sle(c, const_z)?;
        self.emit_and(ge_a, le_z)
    }

    /// islower: c >= 'a' && c <= 'z'
    fn lower_char_is_lower(&mut self, operands: &[usize]) -> Option<usize> {
        let c = operands[0];
        let const_a = self.emit_const_i32('a' as i32)?;
        let const_z = self.emit_const_i32('z' as i32)?;
        let ge_a = self.emit_sge(c, const_a)?;
        let le_z = self.emit_sle(c, const_z)?;
        self.emit_and(ge_a, le_z)
    }

    /// isalpha: isupper(c) || islower(c)
    fn lower_char_is_alpha(&mut self, operands: &[usize]) -> Option<usize> {
        let is_upper = self.lower_char_is_upper(operands)?;
        let is_lower = self.lower_char_is_lower(operands)?;
        self.emit_or(is_upper, is_lower)
    }

    /// isdigit: c >= '0' && c <= '9'
    fn lower_char_is_digit(&mut self, operands: &[usize]) -> Option<usize> {
        let c = operands[0];
        let const_0 = self.emit_const_i32('0' as i32)?;
        let const_9 = self.emit_const_i32('9' as i32)?;
        let ge_0 = self.emit_sge(c, const_0)?;
        let le_9 = self.emit_sle(c, const_9)?;
        self.emit_and(ge_0, le_9)
    }

    /// isspace: c in {' ', '\t', '\n', '\r', '\f', '\v'}
    fn lower_char_is_space(&mut self, operands: &[usize]) -> Option<usize> {
        let c = operands[0];
        // Check each whitespace character
        let const_space = self.emit_const_i32(' ' as i32)?;
        let const_tab = self.emit_const_i32('\t' as i32)?;
        let const_newline = self.emit_const_i32('\n' as i32)?;
        let const_cr = self.emit_const_i32('\r' as i32)?;
        let const_ff = self.emit_const_i32('\x0C' as i32)?; // form feed
        let const_vt = self.emit_const_i32('\x0B' as i32)?; // vertical tab

        let is_space = self.emit_eq(c, const_space)?;
        let is_tab = self.emit_eq(c, const_tab)?;
        let is_newline = self.emit_eq(c, const_newline)?;
        let is_cr = self.emit_eq(c, const_cr)?;
        let is_ff = self.emit_eq(c, const_ff)?;
        let is_vt = self.emit_eq(c, const_vt)?;

        // Combine with OR
        let r1 = self.emit_or(is_space, is_tab)?;
        let r2 = self.emit_or(r1, is_newline)?;
        let r3 = self.emit_or(r2, is_cr)?;
        let r4 = self.emit_or(r3, is_ff)?;
        self.emit_or(r4, is_vt)
    }

    /// iscntrl: c < 32 || c == 127
    fn lower_char_is_control(&mut self, operands: &[usize]) -> Option<usize> {
        let c = operands[0];
        let const_32 = self.emit_const_i32(32)?;
        let const_127 = self.emit_const_i32(127)?;
        let lt_32 = self.emit_slt(c, const_32)?;
        let eq_127 = self.emit_eq(c, const_127)?;
        self.emit_or(lt_32, eq_127)
    }

    /// toupper: if islower(c) then c - 32 else c
    fn lower_char_to_upper(&mut self, operands: &[usize]) -> Option<usize> {
        let c = operands[0];
        let is_lower = self.lower_char_is_lower(operands)?;
        let const_32 = self.emit_const_i32(32)?;
        let upper = self.emit_sub_i32(c, const_32)?;
        self.emit_select_i32(is_lower, upper, c)
    }

    /// tolower: if isupper(c) then c + 32 else c
    fn lower_char_to_lower(&mut self, operands: &[usize]) -> Option<usize> {
        let c = operands[0];
        let is_upper = self.lower_char_is_upper(operands)?;
        let const_32 = self.emit_const_i32(32)?;
        let lower = self.emit_add_i32(c, const_32)?;
        self.emit_select_i32(is_upper, lower, c)
    }

    /// Lower time intrinsic to platform-specific VDSO/syscall.
    fn lower_time_intrinsic(&mut self, kind: &str, _operands: &[usize]) -> Option<usize> {
        let (clock_id, result_type) = match kind {
            "monotonic" => (1i64, MlirType::I64), // CLOCK_MONOTONIC
            "realtime" => (0i64, MlirType::I64),  // CLOCK_REALTIME
            _ => (1, MlirType::I64),
        };

        match self.target.os {
            Os::Linux => {
                // Use clock_gettime via VDSO (fast path)
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![
                        MlirAttr {
                            name: "callee".to_string(),
                            value: MlirAttrValue::String("clock_gettime".to_string()),
                        },
                        MlirAttr {
                            name: "clock_id".to_string(),
                            value: MlirAttrValue::Integer(clock_id),
                        },
                    ],
                    result_types: vec![result_type],
                    operands: vec![],
                    region: None,
                })
            }
            Os::MacOS => {
                // Use mach_absolute_time for monotonic
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("mach_absolute_time".to_string()),
                    }],
                    result_types: vec![result_type],
                    operands: vec![],
                    region: None,
                })
            }
            _ => {
                // Generic fallback
                self.emit(MlirOp {
                    name: "verum.time.get".to_string(),
                    attrs: vec![MlirAttr {
                        name: "kind".to_string(),
                        value: MlirAttrValue::String(kind.to_string()),
                    }],
                    result_types: vec![result_type],
                    operands: vec![],
                    region: None,
                })
            }
        }
    }

    /// Lower spinlock_lock to CAS loop.
    fn lower_spinlock_lock(&mut self, operands: &[usize]) -> Option<usize> {
        // Generate a CAS-based spinlock with spin_hint
        self.emit(MlirOp {
            name: "verum.spinlock.lock".to_string(),
            attrs: vec![],
            result_types: vec![],
            operands: operands.to_vec(),
            region: None,
        })
    }

    /// Lower futex_wait to platform syscall.
    fn lower_futex_wait(&mut self, operands: &[usize]) -> Option<usize> {
        match self.target.os {
            Os::Linux => {
                // Linux futex syscall
                self.emit(MlirOp {
                    name: "verum.futex.wait".to_string(),
                    attrs: vec![MlirAttr {
                        name: "syscall_nr".to_string(),
                        value: MlirAttrValue::Integer(202), // SYS_futex on x86_64
                    }],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Os::MacOS => {
                // macOS __ulock_wait
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("__ulock_wait".to_string()),
                    }],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            _ => {
                self.emit(MlirOp {
                    name: "verum.futex.wait".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
        }
    }

    /// Lower futex_wake to platform syscall.
    fn lower_futex_wake(&mut self, operands: &[usize]) -> Option<usize> {
        match self.target.os {
            Os::Linux => {
                self.emit(MlirOp {
                    name: "verum.futex.wake".to_string(),
                    attrs: vec![MlirAttr {
                        name: "syscall_nr".to_string(),
                        value: MlirAttrValue::Integer(202),
                    }],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            Os::MacOS => {
                self.emit(MlirOp {
                    name: "llvm.call".to_string(),
                    attrs: vec![MlirAttr {
                        name: "callee".to_string(),
                        value: MlirAttrValue::String("__ulock_wake".to_string()),
                    }],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            _ => {
                self.emit(MlirOp {
                    name: "verum.futex.wake".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I32],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
        }
    }

    /// Lower compile-time constant.
    fn lower_compile_time_constant(&mut self, intrinsic: &Intrinsic) -> Option<usize> {
        // Value will be resolved during compilation
        self.emit(MlirOp {
            name: "llvm.mlir.constant".to_string(),
            attrs: vec![MlirAttr {
                name: "intrinsic".to_string(),
                value: MlirAttrValue::String(intrinsic.name.to_string()),
            }],
            result_types: vec![self.infer_result_type(intrinsic)],
            operands: vec![],
            region: None,
        })
    }

    /// Infer result type from intrinsic definition.
    fn infer_result_type(&self, intrinsic: &Intrinsic) -> MlirType {
        // Use hints and category to infer type
        if intrinsic.hints.contains(&IntrinsicHint::MultiReturn) {
            MlirType::Struct(vec![MlirType::I64, MlirType::I1])
        } else {
            match intrinsic.category {
                IntrinsicCategory::Math => MlirType::F64,
                IntrinsicCategory::BitManip => MlirType::I32,
                IntrinsicCategory::Platform => MlirType::I8,
                IntrinsicCategory::Memory => MlirType::Ptr,
                _ => MlirType::I64,
            }
        }
    }

    /// Generate syscall constraints for inline assembly.
    fn syscall_constraints(&self, argc: usize) -> String {
        match self.target.arch {
            Arch::X86_64 => {
                // x86_64 Linux syscall ABI: rax=num, rdi/rsi/rdx/r10/r8/r9=args
                let out = "={rax}";
                let inputs = match argc {
                    0 => "{rax}",
                    1 => "{rax},{rdi}",
                    2 => "{rax},{rdi},{rsi}",
                    3 => "{rax},{rdi},{rsi},{rdx}",
                    4 => "{rax},{rdi},{rsi},{rdx},{r10}",
                    5 => "{rax},{rdi},{rsi},{rdx},{r10},{r8}",
                    6 => "{rax},{rdi},{rsi},{rdx},{r10},{r8},{r9}",
                    _ => "{rax},{rdi},{rsi},{rdx},{r10},{r8},{r9}",
                };
                format!("{},{}",out, inputs)
            }
            Arch::Aarch64 => {
                // ARM64 syscall: x8=num, x0-x5=args
                let out = "={x0}";
                let inputs = match argc {
                    0 => "{x8}",
                    1 => "{x8},{x0}",
                    2 => "{x8},{x0},{x1}",
                    3 => "{x8},{x0},{x1},{x2}",
                    4 => "{x8},{x0},{x1},{x2},{x3}",
                    5 => "{x8},{x0},{x1},{x2},{x3},{x4}",
                    6 => "{x8},{x0},{x1},{x2},{x3},{x4},{x5}",
                    _ => "{x8},{x0},{x1},{x2},{x3},{x4},{x5}",
                };
                format!("{},{}", out, inputs)
            }
            _ => String::new(),
        }
    }

    /// Lower ArithExtended opcode to MLIR.
    /// For checked/overflowing/polymorphic arithmetic operations.
    fn lower_arith_extended_opcode(
        &mut self,
        intrinsic: &Intrinsic,
        sub_op: ArithSubOpcode,
        operands: &[usize],
    ) -> Option<usize> {
        match sub_op {
            // Polymorphic arithmetic - MLIR uses arith dialect with type-specific ops
            ArithSubOpcode::PolyAdd => {
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("arith.addi").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::PolySub => {
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("arith.subi").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::PolyMul => {
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("arith.muli").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::PolyDiv => {
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("arith.divsi").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::PolyRem => {
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("arith.remsi").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::PolyNeg => {
                // Negation: 0 - x for integers
                self.emit(MlirOp {
                    name: "arith.subi".to_string(),
                    attrs: vec![MlirAttr {
                        name: "polymorphic_neg".to_string(),
                        value: MlirAttrValue::Bool(true),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Checked arithmetic - emit overflow-checking ops
            ArithSubOpcode::CheckedAddI => {
                self.emit(MlirOp {
                    name: "arith.addui_extended".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64, MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::CheckedSubI => {
                self.emit(MlirOp {
                    name: "arith.subui_extended".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64, MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::CheckedMulI => {
                self.emit(MlirOp {
                    name: "arith.mulsi_extended".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64, MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::CheckedDivI => {
                // Division overflow check (MIN / -1)
                self.emit(MlirOp {
                    name: "arith.divsi".to_string(),
                    attrs: vec![MlirAttr {
                        name: "checked".to_string(),
                        value: MlirAttrValue::Bool(true),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Overflowing arithmetic - return (result, overflow_flag)
            ArithSubOpcode::OverflowingAddI => {
                self.emit(MlirOp {
                    name: "arith.addui_extended".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64, MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::OverflowingSubI => {
                self.emit(MlirOp {
                    name: "arith.subui_extended".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64, MlirType::I1],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            ArithSubOpcode::OverflowingMulI => {
                self.emit(MlirOp {
                    name: "arith.mulsi_extended".to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64, MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Polymorphic absolute value
            ArithSubOpcode::PolyAbs => {
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("math.absf").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Polymorphic signum
            ArithSubOpcode::PolySignum => {
                self.emit(MlirOp {
                    name: "math.copysign".to_string(),
                    attrs: vec![MlirAttr {
                        name: "signum".to_string(),
                        value: MlirAttrValue::Bool(true),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Polymorphic minimum
            ArithSubOpcode::PolyMin => {
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("arith.minsi").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Polymorphic maximum
            ArithSubOpcode::PolyMax => {
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("arith.maxsi").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Polymorphic clamp
            ArithSubOpcode::PolyClamp => {
                // Clamp is: max(min(val, hi), lo)
                self.emit(MlirOp {
                    name: "arith.maxsi".to_string(),
                    attrs: vec![MlirAttr {
                        name: "clamp".to_string(),
                        value: MlirAttrValue::Bool(true),
                    }],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Wrapping and saturating are handled by dedicated methods
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
                // but if reached here, emit a generic fallback
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("arith.addi").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
            // Bit counting operations (Clz, Ctz, Popcnt, Bswap, BitReverse, RotateLeft, RotateRight)
            // These use LLVM intrinsics via the math dialect
            _ => {
                // Generic handling for bit ops and any new sub-opcodes
                self.emit(MlirOp {
                    name: intrinsic.mlir_op.unwrap_or("math.ctlz").to_string(),
                    attrs: vec![],
                    result_types: vec![MlirType::I64],
                    operands: operands.to_vec(),
                    region: None,
                })
            }
        }
    }

    /// Lower MathExtended opcode (0x29) to MLIR/LLVM intrinsics.
    ///
    /// This provides zero-cost lowering for transcendental and special math functions.
    /// Each MathSubOpcode maps directly to an LLVM intrinsic, which LLVM can inline,
    /// vectorize, or lower to hardware instructions as appropriate.
    ///
    /// # LLVM Intrinsic Mapping
    ///
    /// | Category | LLVM Intrinsic | MathSubOpcode |
    /// |----------|----------------|---------------|
    /// | Trig F64 | llvm.sin.f64, llvm.cos.f64, etc. | SinF64, CosF64, etc. |
    /// | Trig F32 | llvm.sin.f32, llvm.cos.f32, etc. | SinF32, CosF32, etc. |
    /// | Hyper F64 | llvm.sinh.f64, llvm.cosh.f64, etc. | SinhF64, CoshF64, etc. |
    /// | Exp/Log | llvm.exp.f64, llvm.log.f64, etc. | ExpF64, LogF64, etc. |
    /// | Root | llvm.sqrt.f64, llvm.cbrt.f64 | SqrtF64, CbrtF64 |
    /// | Round | llvm.floor.f64, llvm.ceil.f64 | FloorF64, CeilF64 |
    /// | Special | llvm.fma.f64, llvm.copysign.f64 | FmaF64, CopysignF64 |
    ///
    /// # GPU Path
    ///
    /// For GPU lowering (future), the mlir_op hint from intrinsic definition
    /// will be used to emit MLIR math dialect ops (math.sin, math.cos, etc.).
    fn lower_math_extended_opcode(
        &mut self,
        intrinsic: &Intrinsic,
        sub_op: MathSubOpcode,
        operands: &[usize],
    ) -> Option<usize> {
        // Use the llvm_intrinsic() method from MathSubOpcode for direct mapping
        let llvm_name = sub_op.llvm_intrinsic();
        let result_type = if sub_op.is_f64() { MlirType::F64 } else { MlirType::F32 };

        // For classification operations, result is i1 (bool)
        let actual_result_type = match sub_op {
            MathSubOpcode::IsNanF64 | MathSubOpcode::IsInfF64 | MathSubOpcode::IsFiniteF64 |
            MathSubOpcode::IsNanF32 | MathSubOpcode::IsInfF32 | MathSubOpcode::IsFiniteF32 => {
                MlirType::I1
            }
            _ => result_type,
        };

        // If intrinsic has mlir_op hint, it can be used for GPU path or MLIR dialect lowering
        // For now, we always use LLVM intrinsics for CPU path (no libc dependency)
        let _ = intrinsic; // Acknowledge unused for future GPU path

        // CPU path: use LLVM intrinsic directly (zero-cost, no libc)
        self.emit(MlirOp {
            name: llvm_name.to_string(),
            attrs: vec![],
            result_types: vec![actual_result_type],
            operands: operands.to_vec(),
            region: None,
        })
    }

    /// Lower type-aware wrapping arithmetic to MLIR.
    fn lower_wrapping_opcode(
        &mut self,
        intrinsic: &Intrinsic,
        sub_op: ArithSubOpcode,
        width: u8,
        _signed: bool,
        operands: &[usize],
    ) -> Option<usize> {
        // For MLIR, wrapping arithmetic is the default behavior
        // We use the appropriate integer type based on width
        let result_type = match width {
            8 => MlirType::I8,
            16 => MlirType::I16,
            32 => MlirType::I32,
            64 => MlirType::I64,
            _ => MlirType::I64,
        };

        let op_name = match sub_op {
            ArithSubOpcode::WrappingAdd => "arith.addi",
            ArithSubOpcode::WrappingSub => "arith.subi",
            ArithSubOpcode::WrappingMul => "arith.muli",
            ArithSubOpcode::WrappingNeg => "arith.subi", // 0 - x
            ArithSubOpcode::WrappingShl => "arith.shli",
            ArithSubOpcode::WrappingShr => "arith.shrui", // Default to logical shift
            _ => intrinsic.mlir_op.unwrap_or("arith.addi"),
        };

        self.emit(MlirOp {
            name: op_name.to_string(),
            attrs: vec![MlirAttr {
                name: "width".to_string(),
                value: MlirAttrValue::String(format!("i{}", width)),
            }],
            result_types: vec![result_type],
            operands: operands.to_vec(),
            region: None,
        })
    }

    /// Lower type-aware saturating arithmetic to MLIR.
    fn lower_saturating_opcode(
        &mut self,
        _intrinsic: &Intrinsic,
        sub_op: ArithSubOpcode,
        width: u8,
        signed: bool,
        operands: &[usize],
    ) -> Option<usize> {
        // For MLIR, saturating arithmetic uses specific ops
        let result_type = match width {
            8 => MlirType::I8,
            16 => MlirType::I16,
            32 => MlirType::I32,
            64 => MlirType::I64,
            _ => MlirType::I64,
        };

        let op_name = match (sub_op, signed) {
            (ArithSubOpcode::SaturatingAdd, true) => "arith.addsi_sat",
            (ArithSubOpcode::SaturatingAdd, false) => "arith.addui_sat",
            (ArithSubOpcode::SaturatingSub, true) => "arith.subsi_sat",
            (ArithSubOpcode::SaturatingSub, false) => "arith.subui_sat",
            (ArithSubOpcode::SaturatingMul, true) => "arith.mulsi_sat",
            (ArithSubOpcode::SaturatingMul, false) => "arith.mului_sat",
            _ => "arith.addsi_sat",
        };

        self.emit(MlirOp {
            name: op_name.to_string(),
            attrs: vec![],
            result_types: vec![result_type],
            operands: operands.to_vec(),
            region: None,
        })
    }

    /// Lower tensor extended opcode to MLIR.
    fn lower_tensor_extended_opcode(
        &mut self,
        intrinsic: &Intrinsic,
        sub_op: crate::instruction::TensorSubOpcode,
        operands: &[usize],
    ) -> Option<usize> {
        // Lower to MLIR tensor ops based on sub-opcode
        use crate::instruction::TensorSubOpcode;
        let op_name = match sub_op {
            TensorSubOpcode::QR => "linalg.qr",
            TensorSubOpcode::SVD => "linalg.svd",
            TensorSubOpcode::LU => "linalg.lu",
            TensorSubOpcode::Eig => "linalg.eig",
            TensorSubOpcode::EigSymmetric => "linalg.eigh",
            TensorSubOpcode::Schur => "linalg.schur",
            TensorSubOpcode::TriSolve => "linalg.triangular_solve",
            TensorSubOpcode::Inverse => "linalg.inv",
            TensorSubOpcode::Det => "linalg.det",
            TensorSubOpcode::Rank => "linalg.rank",
            TensorSubOpcode::Norm => "linalg.norm",
            TensorSubOpcode::Solve => "linalg.solve",
            TensorSubOpcode::Lstsq => "linalg.lstsq",
            TensorSubOpcode::Cond => "linalg.cond",
            TensorSubOpcode::Trace => "linalg.trace",
            TensorSubOpcode::Kron => "linalg.kron",
            TensorSubOpcode::Cross => "linalg.cross",
            TensorSubOpcode::Contract => "linalg.contract",
            TensorSubOpcode::MatrixPower => "linalg.matrix_power",
            TensorSubOpcode::Expm => "linalg.expm",
            TensorSubOpcode::Logm => "linalg.logm",
            _ => intrinsic.mlir_op.unwrap_or("verum.tensor_extended"),
        };

        self.emit(MlirOp {
            name: op_name.to_string(),
            attrs: vec![],
            result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
            operands: operands.to_vec(),
            region: None,
        })
    }

    /// Lower tensor extended opcode with mode to MLIR.
    fn lower_tensor_extended_opcode_with_mode(
        &mut self,
        intrinsic: &Intrinsic,
        _sub_op: crate::instruction::TensorSubOpcode,
        mode: u8,
        operands: &[usize],
    ) -> Option<usize> {
        // Lower to MLIR tensor ops with mode attribute
        let op_name = intrinsic.mlir_op.unwrap_or("verum.tensor_extended");

        self.emit(MlirOp {
            name: op_name.to_string(),
            attrs: vec![MlirAttr {
                name: "mode".to_string(),
                value: MlirAttrValue::Integer(mode as i64),
            }],
            result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
            operands: operands.to_vec(),
            region: None,
        })
    }

    /// Lower tensor ext extended opcode (TensorExtSubOpcode) to MLIR.
    fn lower_tensor_ext_extended_opcode(
        &mut self,
        _intrinsic: &Intrinsic,
        sub_op: crate::instruction::TensorExtSubOpcode,
        operands: &[usize],
    ) -> Option<usize> {
        // Lower to MLIR tensor ops based on ext sub-opcode
        use crate::instruction::TensorExtSubOpcode;
        let op_name = match sub_op {
            TensorExtSubOpcode::RmsNorm => "verum.tensor.rms_norm",
            TensorExtSubOpcode::FlashAttention => "verum.tensor.flash_attention",
            TensorExtSubOpcode::Fft => "verum.tensor.fft",
            TensorExtSubOpcode::Scatter => "verum.tensor.scatter",
            TensorExtSubOpcode::ContiguousView => "verum.tensor.contiguous_view",
            TensorExtSubOpcode::RandomU64 => "verum.random.u64",
            TensorExtSubOpcode::RandomFloat => "verum.random.float",
            TensorExtSubOpcode::GlobalAllocator => "verum.mem.global_allocator",
            TensorExtSubOpcode::MemNewId => "verum.mem.new_id",
            TensorExtSubOpcode::MemAllocTensor => "verum.mem.alloc_tensor",
            TensorExtSubOpcode::RegexFind => "verum.regex.find",
            TensorExtSubOpcode::RegexReplace => "verum.regex.replace",
            TensorExtSubOpcode::RegexCaptures => "verum.regex.captures",
            TensorExtSubOpcode::PermissionCheckWire => "verum.permission.check_wire",
        };

        self.emit(MlirOp {
            name: op_name.to_string(),
            attrs: vec![],
            result_types: vec![MlirType::Tensor { elem: Box::new(MlirType::F32), shape: vec![] }],
            operands: operands.to_vec(),
            region: None,
        })
    }

    /// Lower GPU extended opcode to MLIR.
    fn lower_gpu_extended_opcode(
        &mut self,
        intrinsic: &Intrinsic,
        sub_op: crate::instruction::GpuSubOpcode,
        operands: &[usize],
    ) -> Option<usize> {
        // Lower to MLIR GPU dialect ops based on sub-opcode
        use crate::instruction::GpuSubOpcode;
        let op_name = match sub_op {
            GpuSubOpcode::GetDevice => "gpu.current_device",
            GpuSubOpcode::SetDevice => "gpu.set_device",
            GpuSubOpcode::DeviceReset => "gpu.reset_device",
            GpuSubOpcode::SyncDevice => "gpu.synchronize",
            GpuSubOpcode::Alloc => "gpu.alloc",
            GpuSubOpcode::Free => "gpu.dealloc",
            GpuSubOpcode::Memcpy | GpuSubOpcode::MemcpyAsync => "gpu.memcpy",
            GpuSubOpcode::StreamCreate => "gpu.stream_create",
            GpuSubOpcode::StreamDestroy => "gpu.stream_destroy",
            GpuSubOpcode::SyncStream => "gpu.stream_sync",
            GpuSubOpcode::Launch | GpuSubOpcode::LaunchCooperative => "gpu.launch",
            _ => intrinsic.mlir_op.unwrap_or("verum.gpu_extended"),
        };

        self.emit(MlirOp {
            name: op_name.to_string(),
            attrs: vec![],
            result_types: vec![MlirType::I64],
            operands: operands.to_vec(),
            region: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::INTRINSIC_REGISTRY;

    #[test]
    fn test_arithmetic_lowering() {
        let intrinsic = INTRINSIC_REGISTRY.lookup("wrapping_add").unwrap();
        let lowering = IntrinsicLowering::new(TargetTriple::default());
        let result = lowering.lower(intrinsic, &[0, 1]);

        assert!(!result.ops.is_empty());
        assert_eq!(result.ops[0].name, "arith.addi");
    }

    #[test]
    fn test_atomic_lowering() {
        let intrinsic = INTRINSIC_REGISTRY.lookup("atomic_load_u64").unwrap();
        let lowering = IntrinsicLowering::new(TargetTriple::default());
        let result = lowering.lower(intrinsic, &[0, 1]);

        assert!(!result.ops.is_empty());
        assert!(result.ops[0].name.contains("load"));
    }

    #[test]
    fn test_math_lowering() {
        let intrinsic = INTRINSIC_REGISTRY.lookup("sin_f64").unwrap();
        let lowering = IntrinsicLowering::new(TargetTriple::default());
        let result = lowering.lower(intrinsic, &[0]);

        assert!(!result.ops.is_empty());
        // The lowering uses LLVM intrinsic names for the LLVM backend
        assert!(
            result.ops[0].name == "math.sin" || result.ops[0].name == "llvm.sin.f64",
            "Expected math.sin or llvm.sin.f64, got: {}",
            result.ops[0].name
        );
    }

    #[test]
    fn test_bit_lowering() {
        let intrinsic = INTRINSIC_REGISTRY.lookup("clz").unwrap();
        let lowering = IntrinsicLowering::new(TargetTriple::default());
        let result = lowering.lower(intrinsic, &[0]);

        assert!(!result.ops.is_empty());
        assert!(result.ops[0].name.contains("ctlz"));
    }
}
