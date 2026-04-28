//! # Industrial-Grade Intrinsic Registry
//!
//! This module defines the complete intrinsic registry for Verum, mapping intrinsic
//! names to their optimal implementation strategies across the entire execution stack:
//!
//! - **VBC Interpreter**: Direct dispatch to specialized handlers (~5-20 cycles)
//! - **JIT Compilation**: Inline VBC sequences for hotspot compilation
//! - **MLIR Lowering**: Zero-overhead LLVM IR generation
//! - **AOT Compilation**: Native instruction mapping
//!
//! ## Design Principles
//!
//! 1. **Complete Coverage**: All ~150+ intrinsics from core/sys/intrinsics.vr
//! 2. **Optimal Dispatch**: Category-based lookup with O(1) hash access
//! 3. **Zero Overhead**: Direct VBC opcode mapping where possible
//! 4. **LLVM Transparency**: All operations visible to LLVM optimization passes
//!
//! ## Intrinsic Categories
//!
//! | Category | Count | Primary Opcode Range | Example |
//! |----------|-------|---------------------|---------|
//! | Arithmetic | 20+ | 0x10-0x2F | add_i64, mul_overflow |
//! | Atomic | 25+ | 0x48-0x4B | atomic_load_u64, atomic_cas |
//! | Memory | 10+ | 0x46-0x47, 0x72-0x73 | memcpy, ptr_read |
//! | System | 15+ | 0x45, 0x4E-0x4F | syscall0-6, tls_get |
//! | Math | 20+ | 0x29, 0x2B-0x2C | sqrt_f64, sin_f64 |
//! | Bit | 10+ | 0x20-0x27 | clz, ctz, popcnt |
//! | Control | 5+ | 0xC3-0xC4 | panic, unreachable |
//! | CBGR | 10+ | 0x70-0x77 | cbgr_validate |
//! | Futex | 5+ | Library | futex_wait, futex_wake |

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::instruction::{
    ArithSubOpcode, GpuSubOpcode, MathSubOpcode, Opcode, TensorExtSubOpcode, TensorSubOpcode,
};

/// Global intrinsic registry singleton.
///
/// Thread-safe lazy initialization ensures the registry is built exactly once.
pub static INTRINSIC_REGISTRY: LazyLock<IntrinsicRegistry> =
    LazyLock::new(IntrinsicRegistry::new);

/// Intrinsic category for dispatch optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicCategory {
    /// Integer arithmetic: add_i64, sub_i64, etc.
    Arithmetic,
    /// Floating-point math: sqrt_f64, sin_f64, etc.
    Math,
    /// Atomic operations: atomic_load, atomic_cas, etc.
    Atomic,
    /// Memory operations: memcpy, memset, ptr_read, etc.
    Memory,
    /// System calls: syscall0-6
    Syscall,
    /// Thread-local storage: tls_get, tls_set
    Tls,
    /// Bit manipulation: clz, ctz, popcnt, bswap
    BitManip,
    /// Overflow-checked arithmetic: add_overflow, etc.
    Overflow,
    /// Wrapping arithmetic: wrapping_add, etc.
    Wrapping,
    /// Saturating arithmetic: saturating_add, etc.
    Saturating,
    /// Control flow: panic, unreachable, abort
    Control,
    /// CBGR memory safety: cbgr_validate, etc.
    Cbgr,
    /// Futex synchronization: futex_wait, futex_wake
    Futex,
    /// Spinlock primitives: spinlock_lock, etc.
    Spinlock,
    /// Platform detection: is_debug, target_os, etc.
    Platform,
    /// Execution tier: is_interpreted, get_tier
    Tier,
    /// Time operations: monotonic_nanos, realtime_secs
    Time,
    /// Async/executor operations
    Async,
    /// Context system operations
    Context,
    /// Type conversion: int_to_float, float_to_int
    Conversion,
    /// Comparison operations: eq_i64, eq_f64, etc.
    Comparison,
    /// Byte conversion: *_to_le_bytes, *_from_le_bytes
    ByteConversion,
    /// Character operations: char_is_*, char_to_*
    Char,
    /// 32-bit floating point operations
    Float32,
    /// SIMD vector operations: vec_add, vec_mul, vec_fma, etc.
    Simd,
    /// Tensor operations: ssm_scan, matrix_exp, fft, etc.
    Tensor,
    /// Automatic differentiation: grad, vjp, jvp, etc.
    Autodiff,
    /// GPU compute operations: launch, memory, streams, events, graphs.
    Gpu,
    /// Distributed computing: process groups, collective ops, RDMA.
    Distributed,
    /// Logging and diagnostics: log_info, log_warning, etc.
    Logging,
    /// Regular expressions and text processing.
    Regex,
}

/// Hints for intrinsic optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicHint {
    /// No side effects - can be eliminated if unused.
    Pure,
    /// Can be evaluated at compile time with constant arguments.
    ConstEval,
    /// Should be inlined at call site.
    Inline,
    /// Rarely executed path - don't optimize for speed.
    Cold,
    /// Hot path - optimize aggressively.
    Hot,
    /// Has memory effects (read/write).
    MemoryEffect,
    /// Has I/O effects.
    IoEffect,
    /// Thread synchronization barrier.
    SyncBarrier,
    /// May trap/panic on invalid input.
    MayTrap,
    /// Requires unsafe context.
    Unsafe,
    /// Generic over type parameter.
    Generic,
    /// Returns multiple values (tuple).
    MultiReturn,
    /// Allocates memory.
    Alloc,
    /// Has side effects (writes, I/O, etc.).
    SideEffect,
}

/// Code generation strategy for an intrinsic.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum CodegenStrategy {
    /// Maps directly to a single VBC opcode.
    DirectOpcode(Opcode),

    /// Maps to a VBC opcode with additional mode byte.
    OpcodeWithMode(Opcode, u8),

    /// Maps to a VBC opcode with size specification.
    OpcodeWithSize(Opcode, u8),

    /// Inline sequence of VBC instructions.
    InlineSequence(InlineSequenceId),

    /// Inline sequence with a byte-width parameter (e.g., byte conversions: 1,2,4,8).
    InlineSequenceWithWidth(InlineSequenceId, u8),

    /// Compile-time constant.
    CompileTimeConstant,

    /// Maps to ArithExtended opcode (0xBD) with a sub-opcode.
    /// Used for checked, overflowing, and polymorphic arithmetic operations.
    ArithExtendedOpcode(ArithSubOpcode),

    /// Maps to MathExtended opcode (0x29) with a sub-opcode.
    /// Used for transcendental and special math functions.
    ///
    /// Zero-cost dispatch: ~2ns interpreter, 0ns AOT (direct LLVM intrinsic).
    ///
    /// # Sub-opcode Ranges
    /// - 0x00-0x0F: Trigonometric F64 (sin, cos, tan, asin, acos, atan, atan2)
    /// - 0x10-0x17: Trigonometric F32
    /// - 0x18-0x1F: Hyperbolic F64 (sinh, cosh, tanh, asinh, acosh, atanh)
    /// - 0x20-0x27: Hyperbolic F32
    /// - 0x28-0x2F: Exponential/Log F64 (exp, exp2, expm1, log, log2, log10, log1p, pow)
    /// - 0x30-0x37: Exponential/Log F32
    /// - 0x38-0x3F: Root/Power F64 (sqrt, cbrt, hypot)
    /// - 0x40-0x47: Root/Power F32
    /// - 0x48-0x4F: Rounding F64 (floor, ceil, round, trunc)
    /// - 0x50-0x57: Rounding F32
    /// - 0x58-0x5F: Special F64 (abs, copysign, fma, fmod, remainder, fdim, minnum, maxnum)
    /// - 0x60-0x67: Special F32
    /// - 0x68-0x6F: Classification F64 (is_nan, is_inf, is_finite)
    /// - 0x70-0x77: Classification F32
    MathExtendedOpcode(MathSubOpcode),

    /// Type-aware wrapping arithmetic with explicit bit width.
    /// Format: (sub_op, width, signed)
    /// Used for wrapping operations that need type-specific truncation.
    WrappingOpcode(ArithSubOpcode, u8, bool),

    /// Type-aware saturating arithmetic with explicit bit width.
    /// Format: (sub_op, width, signed)
    /// Used for saturating operations that need type-specific bounds.
    SaturatingOpcode(ArithSubOpcode, u8, bool),

    /// Maps to TensorExtended opcode (0xFF) with a sub-opcode.
    /// Used for advanced tensor operations like matrix decompositions.
    TensorExtendedOpcode(TensorSubOpcode),

    /// Maps to TensorExtended opcode (0xFF) with a sub-opcode and mode byte.
    /// Used for operations with variants like random (uniform/normal/randint).
    TensorExtendedOpcodeWithMode(TensorSubOpcode, u8),

    /// Maps to GpuExtended opcode (0xF8) with a sub-opcode.
    /// Used for GPU operations: kernel launch, memory, streams, events, graphs.
    GpuExtendedOpcode(GpuSubOpcode),

    /// Maps to TensorExtExtended opcode (0xFC 0x00) with an ext sub-opcode.
    /// Used for extended tensor operations: RmsNorm, FlashAttention, Fft, Scatter.
    TensorExtExtendedOpcode(TensorExtSubOpcode),
}

/// Identifier for pre-defined inline instruction sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InlineSequenceId {
    /// memcpy: optimized copy loop
    Memcpy,
    /// memmove: direction-aware copy
    Memmove,
    /// memset: optimized set loop
    Memset,
    /// memcmp: compare with early exit
    Memcmp,
    /// fetch_add: CAS loop for atomic add
    AtomicFetchAdd,
    /// fetch_sub: CAS loop for atomic sub
    AtomicFetchSub,
    /// fetch_and: CAS loop for atomic and
    AtomicFetchAnd,
    /// fetch_or: CAS loop for atomic or
    AtomicFetchOr,
    /// fetch_xor: CAS loop for atomic xor
    AtomicFetchXor,
    /// spinlock_lock: CAS + spin_hint loop
    SpinlockLock,
    /// futex_wait: syscall wrapper
    FutexWait,
    /// futex_wake: syscall wrapper
    FutexWake,
    /// checked_add: add + overflow check
    CheckedAdd,
    /// checked_sub: sub + overflow check
    CheckedSub,
    /// checked_mul: mul + overflow check
    CheckedMul,
    /// checked_div: div + zero/overflow check
    CheckedDiv,
    /// overflowing_add: add with overflow flag
    OverflowingAdd,
    /// overflowing_sub: sub with overflow flag
    OverflowingSub,
    /// overflowing_mul: mul with overflow flag
    OverflowingMul,
    /// clz: count leading zeros
    Clz,
    /// ctz: count trailing zeros
    Ctz,
    /// popcnt: population count
    Popcnt,
    /// ilog2: integer log base 2 (63 - clz)
    Ilog2,
    /// bswap: byte swap
    Bswap,
    /// rotate_left: bit rotation
    RotateLeft,
    /// rotate_right: bit rotation
    RotateRight,
    /// sin_f64: polynomial approximation
    SinF64,
    /// cos_f64: polynomial approximation
    CosF64,
    /// tan_f64: sin/cos ratio
    TanF64,
    /// asin_f64: arc sine
    AsinF64,
    /// acos_f64: arc cosine
    AcosF64,
    /// atan_f64: arc tangent
    AtanF64,
    /// atan2_f64: two-argument arc tangent
    Atan2F64,
    /// exp_f64: natural exponential
    ExpF64,
    /// log_f64: natural logarithm
    LogF64,
    /// log10_f64: base-10 logarithm
    Log10F64,
    /// monotonic_nanos: platform time
    MonotonicNanos,
    /// realtime_secs: wall clock time (seconds)
    RealtimeSecs,
    /// realtime_nanos: wall clock time (nanoseconds)
    RealtimeNanos,
    /// drop_in_place: run destructor via pointer
    DropInPlace,
    /// make_slice: construct fat pointer from ptr + len
    MakeSlice,
    /// slice_len: extract length from fat pointer
    SliceLen,
    /// slice_as_ptr: extract pointer from fat pointer
    SliceAsPtr,
    /// slice_get: bounds-checked element access
    SliceGet,
    /// slice_get_unchecked: unchecked element access
    SliceGetUnchecked,
    /// slice_subslice: create subslice view
    SliceSubslice,
    /// slice_split_at: split slice into two parts
    SliceSplitAt,
    /// text_from_static: create Text from static string
    TextFromStatic,
    /// utf8_decode_char: decode UTF-8 character
    Utf8DecodeChar,
    /// text_parse_int: parse integer from text
    TextParseInt,
    /// text_parse_float: parse float from text
    TextParseFloat,
    /// int_to_text: convert integer to text
    IntToText,
    /// float_to_text: convert float to text
    FloatToText,
    /// text_byte_len: get byte length of Text
    TextByteLen,
    /// uninit: allocate uninitialized memory on stack
    Uninit,
    /// zeroed: allocate zeroed memory on stack
    Zeroed,
    /// cbrt_f64: cube root
    CbrtF64,
    /// expm1_f64: e^x - 1
    Expm1F64,
    /// exp2_f64: 2^x
    Exp2F64,
    /// log1p_f64: ln(1 + x)
    Log1pF64,
    /// log2_f64: base-2 logarithm
    Log2F64,
    /// powi_f64: power with integer exponent
    PowiF64,
    /// trunc_f64: truncate towards zero
    TruncF64,
    /// minnum_f64: NaN-propagating minimum
    MinnumF64,
    /// maxnum_f64: NaN-propagating maximum
    MaxnumF64,
    /// fma_f64: fused multiply-add
    FmaF64,
    /// copysign_f64: copy sign
    CopysignF64,
    /// hypot_f64: hypotenuse
    HypotF64,
    /// sinh_f64: hyperbolic sine
    SinhF64,
    /// cosh_f64: hyperbolic cosine
    CoshF64,
    /// tanh_f64: hyperbolic tangent
    TanhF64,
    /// asinh_f64: inverse hyperbolic sine
    AsinhF64,
    /// acosh_f64: inverse hyperbolic cosine
    AcoshF64,
    /// atanh_f64: inverse hyperbolic tangent
    AtanhF64,
    /// bitreverse: reverse bits
    Bitreverse,
    /// int_to_float: integer to float conversion
    IntToFloat,
    /// float_to_int: float to integer conversion
    FloatToInt,
    /// to_le_bytes: convert to little-endian bytes
    ToLeBytes,
    /// from_le_bytes: convert from little-endian bytes
    FromLeBytes,
    /// to_be_bytes: convert to big-endian bytes
    ToBeBytes,
    /// from_be_bytes: convert from big-endian bytes
    FromBeBytes,
    /// char_is_alphabetic: Unicode alphabetic check
    CharIsAlphabetic,
    /// char_is_numeric: Unicode numeric check
    CharIsNumeric,
    /// char_is_whitespace: Unicode whitespace check
    CharIsWhitespace,
    /// char_is_control: Unicode control character check
    CharIsControl,
    /// char_is_uppercase: Unicode uppercase check
    CharIsUppercase,
    /// char_is_lowercase: Unicode lowercase check
    CharIsLowercase,
    /// char_to_uppercase: Unicode uppercase conversion
    CharToUppercase,
    /// char_to_lowercase: Unicode lowercase conversion
    CharToLowercase,
    /// char_encode_utf8: encode character as UTF-8
    CharEncodeUtf8,
    /// char_escape_debug: escape character for debug output
    CharEscapeDebug,
    /// sqrt_f32: 32-bit square root
    SqrtF32,
    /// floor_f32: 32-bit floor
    FloorF32,
    /// ceil_f32: 32-bit ceiling
    CeilF32,
    /// round_f32: 32-bit round
    RoundF32,
    /// trunc_f32: 32-bit truncate
    TruncF32,
    /// abs_f32: 32-bit absolute value
    AbsF32,
    /// sin_f32: 32-bit sine
    SinF32,
    /// cos_f32: 32-bit cosine
    CosF32,
    /// tan_f32: 32-bit tangent
    TanF32,
    /// asin_f32: 32-bit arc sine
    AsinF32,
    /// acos_f32: 32-bit arc cosine
    AcosF32,
    /// atan_f32: 32-bit arc tangent
    AtanF32,
    /// atan2_f32: 32-bit two-argument arc tangent
    Atan2F32,
    /// sinh_f32: 32-bit hyperbolic sine
    SinhF32,
    /// cosh_f32: 32-bit hyperbolic cosine
    CoshF32,
    /// tanh_f32: 32-bit hyperbolic tangent
    TanhF32,
    /// asinh_f32: 32-bit inverse hyperbolic sine
    AsinhF32,
    /// acosh_f32: 32-bit inverse hyperbolic cosine
    AcoshF32,
    /// atanh_f32: 32-bit inverse hyperbolic tangent
    AtanhF32,
    /// exp_f32: 32-bit natural exponential
    ExpF32,
    /// exp2_f32: 32-bit base-2 exponential
    Exp2F32,
    /// expm1_f32: 32-bit e^x - 1
    Expm1F32,
    /// log_f32: 32-bit natural logarithm
    LogF32,
    /// log2_f32: 32-bit base-2 logarithm
    Log2F32,
    /// log10_f32: 32-bit base-10 logarithm
    Log10F32,
    /// log1p_f32: 32-bit ln(1 + x)
    Log1pF32,
    /// cbrt_f32: 32-bit cube root
    CbrtF32,
    /// hypot_f32: 32-bit hypotenuse
    HypotF32,
    /// fma_f32: 32-bit fused multiply-add
    FmaF32,
    /// copysign_f32: 32-bit copy sign
    CopysignF32,
    /// powi_f32: 32-bit power with integer exponent
    PowiF32,
    /// minnum_f32: 32-bit NaN-propagating minimum
    MinnumF32,
    /// maxnum_f32: 32-bit NaN-propagating maximum
    MaxnumF32,
    /// saturating_add: add with saturation
    SaturatingAdd,
    /// saturating_sub: subtract with saturation
    SaturatingSub,
    /// sext: sign-extend integer
    Sext,
    /// zext: zero-extend integer
    Zext,
    /// fpext: extend float precision
    Fpext,
    /// fptrunc: truncate float precision
    Fptrunc,
    /// int_trunc: truncate integer width
    IntTrunc,
    /// bitcast: reinterpret bits without conversion
    Bitcast,
    /// f32_to_bits: reinterpret f32 as u32
    F32ToBits,
    /// f32_from_bits: reinterpret u32 as f32
    F32FromBits,
    /// f64_to_bits: reinterpret f64 as u64
    F64ToBits,
    /// f64_from_bits: reinterpret u64 as f64
    F64FromBits,
    /// is_nan: check if float is NaN
    IsNan,
    /// is_inf: check if float is infinite
    IsInf,
    /// is_finite: check if float is finite
    IsFinite,
    /// pow_f64: power function
    PowF64,
    /// abs_f64: absolute value
    AbsF64,
    /// floor_f64: floor function
    FloorF64,
    /// ceil_f64: ceiling function
    CeilF64,
    /// round_f64: round to nearest
    RoundF64,
    /// sqrt_f64: square root
    SqrtF64,
    /// random_u64: cryptographically secure random u64
    RandomU64,
    /// random_float: random float in [0, 1)
    RandomFloat,
    /// char_general_category: Unicode general category
    CharGeneralCategory,
    /// global_allocator: get global allocator instance
    GlobalAllocator,
    /// atomic_exchange: atomic exchange operation
    AtomicExchange,
    /// poll_pending: return Poll::Pending for async operations in Tier 0
    PollPending,
    /// call_second_arg: call the second argument as a function (for recovery passthrough)
    CallSecondArg,
    /// load_unit: load unit value ()
    LoadUnit,
    /// volatile_load: volatile memory read (prevents optimization)
    VolatileLoad,
    /// volatile_store: volatile memory write (prevents optimization)
    VolatileStore,
    /// volatile_load_acquire: volatile read with acquire semantics
    VolatileLoadAcquire,
    /// volatile_store_release: volatile write with release semantics
    VolatileStoreRelease,
    /// compiler_fence: compiler memory barrier
    CompilerFence,
    /// hardware_fence: CPU memory barrier
    HardwareFence,

    // =========================================================================
    // SIMD Vector Operations
    // =========================================================================
    /// simd_splat: broadcast scalar to all lanes
    SimdSplat,
    /// simd_extract: extract scalar from lane
    SimdExtract,
    /// simd_insert: insert scalar into lane
    SimdInsert,
    /// simd_add: element-wise addition
    SimdAdd,
    /// simd_sub: element-wise subtraction
    SimdSub,
    /// simd_mul: element-wise multiplication
    SimdMul,
    /// simd_div: element-wise division
    SimdDiv,
    /// simd_neg: element-wise negation
    SimdNeg,
    /// simd_abs: element-wise absolute value
    SimdAbs,
    /// simd_sqrt: element-wise square root
    SimdSqrt,
    /// simd_fma: fused multiply-add (a * b + c)
    SimdFma,
    /// simd_min: element-wise minimum
    SimdMin,
    /// simd_max: element-wise maximum
    SimdMax,
    /// simd_reduce_add: horizontal sum reduction
    SimdReduceAdd,
    /// simd_reduce_mul: horizontal product reduction
    SimdReduceMul,
    /// simd_reduce_min: horizontal minimum reduction
    SimdReduceMin,
    /// simd_reduce_max: horizontal maximum reduction
    SimdReduceMax,
    /// simd_cmp_eq: element-wise equality comparison -> mask
    SimdCmpEq,
    /// simd_cmp_ne: element-wise not-equal comparison -> mask
    SimdCmpNe,
    /// simd_cmp_lt: element-wise less-than comparison -> mask
    SimdCmpLt,
    /// simd_cmp_le: element-wise less-or-equal comparison -> mask
    SimdCmpLe,
    /// simd_cmp_gt: element-wise greater-than comparison -> mask
    SimdCmpGt,
    /// simd_cmp_ge: element-wise greater-or-equal comparison -> mask
    SimdCmpGe,
    /// simd_select: conditional select using mask
    SimdSelect,
    /// simd_load_aligned: load from aligned memory
    SimdLoadAligned,
    /// simd_load_unaligned: load from unaligned memory
    SimdLoadUnaligned,
    /// simd_store_aligned: store to aligned memory
    SimdStoreAligned,
    /// simd_store_unaligned: store to unaligned memory
    SimdStoreUnaligned,
    /// simd_masked_load: load with mask (inactive lanes become zero)
    SimdMaskedLoad,
    /// simd_masked_store: store with mask (inactive lanes skipped)
    SimdMaskedStore,
    /// simd_shuffle: permute elements using constant indices
    SimdShuffle,
    /// simd_gather: indexed load from memory
    SimdGather,
    /// simd_scatter: indexed store to memory
    SimdScatter,
    /// simd_mask_all: create all-true mask
    SimdMaskAll,
    /// simd_mask_none: create all-false mask
    SimdMaskNone,
    /// simd_mask_count: count active mask lanes
    SimdMaskCount,
    /// simd_mask_any: check if any mask lane is active
    SimdMaskAny,
    /// simd_bitwise_and: element-wise bitwise AND
    SimdBitwiseAnd,
    /// simd_bitwise_or: element-wise bitwise OR
    SimdBitwiseOr,
    /// simd_bitwise_xor: element-wise bitwise XOR
    SimdBitwiseXor,
    /// simd_bitwise_not: element-wise bitwise NOT
    SimdBitwiseNot,
    /// simd_shift_left: element-wise left shift
    SimdShiftLeft,
    /// simd_shift_right: element-wise right shift (arithmetic for signed)
    SimdShiftRight,
    /// simd_cast: element-wise type conversion
    SimdCast,

    // =========================================================================
    // Tensor Operations (for SSM, FFT, Linear Algebra)
    // =========================================================================
    /// ssm_scan: parallel associative scan for SSM
    SsmScan,
    /// matrix_exp: matrix exponential (Padé approximation)
    MatrixExp,
    /// matrix_inverse: matrix inverse
    MatrixInverse,
    /// complex_pow: complex number power
    ComplexPow,
    /// complex_mul: complex number multiplication
    ComplexMul,
    /// rfft: real FFT
    Rfft,
    /// irfft: inverse real FFT
    Irfft,
    /// uniform: uniform random tensor
    Uniform,
    /// is_training: check if in training mode
    IsTraining,
    /// bincount: histogram binning
    Bincount,
    /// gather_nd: n-dimensional gather
    GatherNd,
    /// arange_usize: integer range tensor
    ArangeUsize,
    /// repeat: repeat tensor along dimension
    TensorRepeat,
    /// tensor_tanh: element-wise hyperbolic tangent on tensor
    TensorTanh,
    /// tensor_sum: sum reduction on tensor
    TensorSum,
    /// tensor_from_array: create tensor from array
    TensorFromArray,
    /// random_float_01: random float in [0, 1)
    RandomFloat01,

    // =========================================================================
    // Additional Tensor Operations (for core/math)
    // =========================================================================
    /// tensor_unsqueeze: add singleton dimension
    TensorUnsqueeze,
    /// tensor_masked_select: select elements where mask is true
    TensorMaskedSelect,
    /// tensor_leaky_relu: leaky ReLU activation
    TensorLeakyRelu,
    /// tensor_diag: extract diagonal or create diagonal matrix
    TensorDiag,
    /// tensor_triu: upper triangular part
    TensorTriu,
    /// tensor_tril: lower triangular part
    TensorTril,
    /// tensor_nonzero: indices of nonzero elements
    TensorNonzero,
    /// tensor_one_hot: create one-hot encoding
    TensorOneHot,
    /// tensor_split: split tensor into chunks
    TensorSplit,
    /// tensor_split_at: split tensor at indices
    TensorSplitAt,
    /// tensor_get_scalar: get scalar value at indices
    TensorGetScalar,
    /// tensor_set_scalar: set scalar value at indices
    TensorSetScalar,
    /// tensor_contiguous: make tensor contiguous in memory
    TensorContiguous,
    /// tensor_contiguous_view: get contiguous view of tensor
    TensorContiguousView,
    /// tensor_to_device: move tensor to device
    TensorToDevice,
    /// mem_new_id: allocate new memory ID
    MemNewId,
    /// mem_alloc_tensor: allocate tensor memory storage
    MemAllocTensor,

    // =========================================================================
    // Automatic Differentiation Operations
    // =========================================================================
    /// grad_begin: enter gradient computation scope (reverse mode)
    GradBegin,
    /// grad_end: exit gradient computation scope, return pullback
    GradEnd,
    /// jvp_begin: enter JVP computation scope (forward mode)
    JvpBegin,
    /// jvp_end: exit JVP scope, return tangent output
    JvpEnd,
    /// grad_zero_tangent: create zero tangent for a type
    GradZeroTangent,
    /// grad_stop: stop gradient propagation (detach)
    GradStop,
    /// grad_custom: register custom VJP rule
    GradCustom,
    /// grad_checkpoint: checkpoint for recomputation
    GradCheckpoint,
    /// grad_accumulate: accumulate gradients
    GradAccumulate,
    /// grad_recompute: recompute from checkpoint
    GradRecompute,
    /// grad_zero: zero out gradient tensor
    GradZero,

    // =========================================================================
    // CBGR Operations
    // =========================================================================
    /// cbgr_new_generation: create a new CBGR generation
    CbgrNewGeneration,
    /// cbgr_invalidate: invalidate a CBGR generation
    CbgrInvalidate,
    /// cbgr_get_generation: get generation of a reference
    CbgrGetGeneration,
    /// cbgr_advance_generation: advance generation of a reference
    CbgrAdvanceGeneration,
    /// cbgr_get_epoch_caps: get epoch capabilities of a reference
    CbgrGetEpochCaps,
    /// cbgr_bypass_begin: begin CBGR bypass region (unsafe)
    CbgrBypassBegin,
    /// cbgr_bypass_end: end CBGR bypass region (unsafe)
    CbgrBypassEnd,
    /// cbgr_get_stats: get CBGR statistics
    CbgrGetStats,
    /// cbgr_alloc: allocate memory with CBGR tracking
    CbgrAlloc,
    /// cbgr_alloc_zeroed: allocate zeroed memory with CBGR tracking
    CbgrAllocZeroed,
    /// cbgr_dealloc: deallocate CBGR-tracked memory
    CbgrDealloc,
    /// cbgr_realloc: reallocate CBGR-tracked memory
    CbgrRealloc,
    /// memcmp_bytes: compare memory regions byte-by-byte
    MemcmpBytes,
    /// get_header_from_ptr: get CBGR allocation header from pointer
    GetHeaderFromPtr,

    // =========================================================================
    // Logging and I/O Operations
    // =========================================================================
    /// log_info: print info-level message to stdout
    LogInfo,
    /// log_warning: print warning-level message to stderr
    LogWarning,
    /// log_error: print error-level message to stderr
    LogError,
    /// log_debug: print debug-level message to stdout
    LogDebug,

    // =========================================================================
    // Regex Operations
    // =========================================================================
    /// regex_find_all: find all matches of pattern in text
    RegexFindAll,
    /// regex_replace_all: replace all matches of pattern in text
    RegexReplaceAll,
    /// regex_is_match: check if pattern matches text
    RegexIsMatch,
    /// regex_split: split text by pattern
    RegexSplit,
    /// regex_find: first match only
    RegexFind,
    /// regex_replace: first replace only
    RegexReplace,
    /// regex_captures: ordered capture groups of first match
    RegexCaptures,

    // =========================================================================
    // Type Introspection Operations
    // =========================================================================
    /// size_of: get size of type in bytes
    SizeOf,
    /// align_of: get alignment of type in bytes
    AlignOf,
    /// type_id: get unique type identifier
    TypeId,
    /// type_name: get human-readable type name
    TypeName,
    /// needs_drop: check if type needs drop glue
    NeedsDrop,
    /// pow_f32: 32-bit power function
    PowF32,
    /// char_is_alphanumeric: Unicode alphanumeric check
    CharIsAlphanumeric,
    /// rdtsc: read CPU timestamp counter
    Rdtsc,
    /// catch_unwind: catch panics and convert to Result
    CatchUnwind,
    /// ptr_to_ref: convert raw pointer to reference
    PtrToRef,

    // =========================================================================
    // Time Operations (Duration, Instant, system time, sleep)
    // =========================================================================
    /// duration_from_nanos: create Duration from nanoseconds (identity)
    DurationFromNanos,
    /// duration_from_micros: create Duration from microseconds
    DurationFromMicros,
    /// duration_from_millis: create Duration from milliseconds
    DurationFromMillis,
    /// duration_from_secs: create Duration from seconds
    DurationFromSecs,
    /// duration_as_nanos: get Duration as nanoseconds (identity)
    DurationAsNanos,
    /// duration_as_micros: get Duration as microseconds
    DurationAsMicros,
    /// duration_as_millis: get Duration as milliseconds
    DurationAsMillis,
    /// duration_as_secs: get Duration as seconds
    DurationAsSecs,
    /// duration_is_zero: check if Duration is zero
    DurationIsZero,
    /// duration_add: add two Durations
    DurationAdd,
    /// duration_saturating_add: add Durations with ceiling at MAX
    DurationSaturatingAdd,
    /// duration_saturating_sub: subtract Durations with floor at zero
    DurationSaturatingSub,
    /// duration_subsec_nanos: get sub-second nanosecond component
    DurationSubsecNanos,
    /// instant_now: get current monotonic instant
    InstantNow,
    /// instant_elapsed: get elapsed time since instant
    InstantElapsed,
    /// instant_duration_since: get duration between two instants
    InstantDurationSince,
    /// time_monotonic_micros: monotonic time in microseconds
    TimeMonotonicMicros,
    /// time_monotonic_millis: monotonic time in milliseconds
    TimeMonotonicMillis,
    /// time_unix_timestamp: Unix timestamp in seconds
    TimeUnixTimestamp,
    /// time_sleep_ms: sleep for milliseconds
    TimeSleepMs,
    /// time_sleep_us: sleep for microseconds
    TimeSleepUs,
    /// time_sleep_duration: sleep for a Duration
    TimeSleepDuration,
    /// stopwatch_new: create a new running Stopwatch
    StopwatchNew,
    /// stopwatch_elapsed: get elapsed time from Stopwatch
    StopwatchElapsed,
    /// stopwatch_stop: stop the Stopwatch
    StopwatchStop,
    /// stopwatch_start: start/restart the Stopwatch
    StopwatchStart,
    /// stopwatch_reset: reset the Stopwatch
    StopwatchReset,
    /// perf_counter_now: get current performance counter
    PerfCounterNow,
    /// perf_counter_elapsed_since: get Duration since a PerfCounter
    PerfCounterElapsedSince,
    /// perf_counter_as_nanos: get PerfCounter value as nanoseconds
    PerfCounterAsNanos,
    /// deadline_timer_from_duration: create a DeadlineTimer
    DeadlineTimerFromDuration,
    /// deadline_timer_is_expired: check if timer has expired
    DeadlineTimerIsExpired,
    /// deadline_timer_remaining: get remaining time
    DeadlineTimerRemaining,
    // =========================================================================
    // System Call Intrinsics (darwin/libsystem safe wrappers)
    // =========================================================================
    /// sys_getpid: get process ID
    SysGetpid,
    /// sys_gettid: get thread ID
    SysGettid,
    /// sys_mmap: memory map (safe wrapper)
    SysMmap,
    /// sys_munmap: memory unmap (safe wrapper)
    SysMunmap,
    /// sys_madvise: memory advise (safe wrapper)
    SysMadvise,
    /// sys_getentropy: get cryptographic random bytes (safe wrapper)
    SysGetentropy,
    /// mach_vm_allocate: Mach VM allocate (safe wrapper)
    MachVmAllocate,
    /// mach_vm_deallocate: Mach VM deallocate (safe wrapper)
    MachVmDeallocate,
    /// mach_vm_protect: Mach VM protect (safe wrapper)
    MachVmProtect,
    /// mach_sem_create: Mach semaphore create (safe wrapper)
    MachSemCreate,
    /// mach_sem_destroy: Mach semaphore destroy (safe wrapper)
    MachSemDestroy,
    /// mach_sem_signal: Mach semaphore signal (safe wrapper)
    MachSemSignal,
    /// mach_sem_wait: Mach semaphore wait (safe wrapper)
    MachSemWait,
    /// mach_error_string: Mach error string lookup
    MachErrorString,
    /// mach_sleep_until: Mach sleep until deadline (safe wrapper)
    MachSleepUntil,

    // =========================================================================
    // Heap Memory Allocation Intrinsics
    // =========================================================================
    /// alloc: allocate heap memory
    Alloc,
    /// alloc_zeroed: allocate zeroed heap memory
    AllocZeroed,
    /// dealloc: deallocate heap memory
    Dealloc,
    /// realloc: reallocate heap memory
    Realloc,
    /// swap: swap two values in place
    Swap,
    /// replace: replace value and return old
    Replace,
    /// ptr_offset: element-scaled pointer arithmetic (ptr + count * 8)
    PtrOffset,
}

/// Complete intrinsic definition.
#[derive(Debug, Clone)]
pub struct Intrinsic {
    /// Intrinsic name (e.g., "atomic_load_u64").
    pub name: &'static str,

    /// Category for dispatch optimization.
    pub category: IntrinsicCategory,

    /// Optimization hints.
    pub hints: &'static [IntrinsicHint],

    /// Number of input parameters.
    pub param_count: u8,

    /// Number of output values (0 = void, 1 = single, 2+ = tuple).
    pub return_count: u8,

    /// Code generation strategy.
    pub strategy: CodegenStrategy,

    /// MLIR operation name for lowering.
    pub mlir_op: Option<&'static str>,

    /// Documentation comment.
    pub doc: &'static str,
}

impl Intrinsic {
    /// Returns the primary VBC opcode if this intrinsic maps directly.
    pub fn primary_opcode(&self) -> Option<Opcode> {
        match &self.strategy {
            CodegenStrategy::DirectOpcode(op) => Some(*op),
            CodegenStrategy::OpcodeWithMode(op, _) => Some(*op),
            CodegenStrategy::OpcodeWithSize(op, _) => Some(*op),
            _ => None,
        }
    }

    /// Returns true if this intrinsic is pure (no side effects).
    pub fn is_pure(&self) -> bool {
        self.hints.contains(&IntrinsicHint::Pure)
    }

    /// Returns true if this intrinsic can be evaluated at compile time.
    pub fn is_const_eval(&self) -> bool {
        self.hints.contains(&IntrinsicHint::ConstEval)
    }

    /// Returns true if this intrinsic should be inlined.
    pub fn should_inline(&self) -> bool {
        self.hints.contains(&IntrinsicHint::Inline)
    }
}

/// Result type for intrinsic operations.
pub type IntrinsicResult<T> = Result<T, IntrinsicError>;

/// Error type for intrinsic operations.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub enum IntrinsicError {
    /// Intrinsic not found.
    NotFound(String),
    /// Invalid argument count.
    InvalidArgCount { expected: u8, got: u8 },
    /// Invalid argument type.
    InvalidArgType { arg: usize, expected: &'static str },
    /// Platform not supported.
    PlatformNotSupported(&'static str),
    /// Compile-time evaluation failed.
    ConstEvalFailed(String),
}

/// The intrinsic registry containing all intrinsic definitions.
pub struct IntrinsicRegistry {
    /// Name -> Intrinsic lookup.
    by_name: HashMap<&'static str, &'static Intrinsic>,
    /// Category -> Intrinsics lookup.
    by_category: HashMap<IntrinsicCategory, Vec<&'static str>>,
}

impl IntrinsicRegistry {
    /// Creates a new intrinsic registry with all intrinsics registered.
    pub fn new() -> Self {
        let mut by_name = HashMap::new();
        let mut by_category: HashMap<IntrinsicCategory, Vec<&'static str>> = HashMap::new();

        // Register all intrinsics
        for intrinsic in ALL_INTRINSICS.iter() {
            by_name.insert(intrinsic.name, intrinsic);
            by_category
                .entry(intrinsic.category)
                .or_default()
                .push(intrinsic.name);
        }

        Self {
            by_name,
            by_category,
        }
    }

    /// Looks up an intrinsic by name.
    #[inline]
    pub fn lookup(&self, name: &str) -> Option<&'static Intrinsic> {
        self.by_name.get(name).copied()
    }

    /// Checks if a name is a registered intrinsic.
    #[inline]
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Returns all intrinsics in a category.
    pub fn by_category(&self, category: IntrinsicCategory) -> &[&'static str] {
        self.by_category
            .get(&category)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Returns the total number of registered intrinsics.
    pub fn count(&self) -> usize {
        self.by_name.len()
    }

    /// Returns an iterator over all intrinsic names.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_name.keys().copied()
    }

    /// Looks up a generic intrinsic by base name and type suffix.
    ///
    /// This method supports the generic intrinsic declarations in core/sys/intrinsics.vr.
    /// When calling a generic intrinsic like `add<T>(a, b)`, the compiler monomorphizes
    /// and calls this method with the base name ("add") and type suffix ("i64" for Int).
    ///
    /// # Arguments
    /// * `base_name` - The generic intrinsic name (e.g., "add", "checked_add")
    /// * `type_suffix` - The type suffix (e.g., "i64", "u64", "f64", "i32", "u32")
    ///
    /// # Returns
    /// The specific intrinsic if found, or None.
    ///
    /// # Examples
    /// ```
    /// use verum_vbc::intrinsics::registry::IntrinsicRegistry;
    /// let reg = IntrinsicRegistry::new();
    /// // add<Int>(a, b) -> lookup_generic("add", "i64")
    /// let intrinsic = reg.lookup_generic("add", "i64");
    /// ```
    #[inline]
    pub fn lookup_generic(&self, base_name: &str, type_suffix: &str) -> Option<&'static Intrinsic> {
        // First, try exact match (for non-generic intrinsics like "memcpy")
        if let Some(intrinsic) = self.lookup(base_name) {
            return Some(intrinsic);
        }

        // Try with type suffix: base_name + "_" + type_suffix
        let full_name = format!("{}_{}", base_name, type_suffix);
        self.by_name.get(full_name.as_str()).copied()
    }

    /// Resolves a generic intrinsic name to its type-specific name.
    ///
    /// This method returns the full intrinsic name that should be used in codegen.
    ///
    /// # Arguments
    /// * `base_name` - The generic intrinsic name (e.g., "add")
    /// * `type_suffix` - The type suffix (e.g., "i64" for Int)
    ///
    /// # Returns
    /// The resolved intrinsic name, or None if not found.
    pub fn resolve_generic_name(&self, base_name: &str, type_suffix: &str) -> Option<&'static str> {
        // First check if base_name itself is registered
        if let Some((&name, _)) = self.by_name.get_key_value(base_name) {
            return Some(name);
        }

        // Try type-specific name
        let full_name = format!("{}_{}", base_name, type_suffix);
        self.by_name
            .keys()
            .find(|&name| *name == full_name.as_str())
            .copied()
    }
}

impl Default for IntrinsicRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Static Intrinsic Definitions
// =============================================================================
// All intrinsics are defined as static constants for zero-cost lookup.

/// Complete list of all intrinsics.
static ALL_INTRINSICS: &[Intrinsic] = &[
    // =========================================================================
    // Memory Intrinsics
    // =========================================================================
    Intrinsic {
        name: "memcpy",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::MemoryEffect],
        param_count: 3, // dst, src, len
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Memcpy),
        mlir_op: Some("llvm.intr.memcpy"),
        doc: "Copy non-overlapping memory regions",
    },
    Intrinsic {
        name: "memmove",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::MemoryEffect],
        param_count: 3, // dst, src, len
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Memmove),
        mlir_op: Some("llvm.intr.memmove"),
        doc: "Copy potentially overlapping memory regions",
    },
    Intrinsic {
        name: "memset",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::MemoryEffect],
        param_count: 3, // dst, val, len
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Memset),
        mlir_op: Some("llvm.intr.memset"),
        doc: "Fill memory with a byte value",
    },
    Intrinsic {
        name: "memcmp",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::Pure],
        param_count: 3, // lhs, rhs, len
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Memcmp),
        mlir_op: Some("llvm.intr.memcmp"),
        doc: "Compare memory regions",
    },
    Intrinsic {
        name: "ptr_read",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // ptr
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Deref),
        mlir_op: Some("llvm.load"),
        doc: "Read value from pointer",
    },
    Intrinsic {
        name: "intrinsic_ptr_read",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // ptr
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Deref),
        mlir_op: Some("llvm.load"),
        doc: "Read value from pointer (alias for ptr_read)",
    },
    Intrinsic {
        name: "ptr_write",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, value
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::DerefMut),
        mlir_op: Some("llvm.store"),
        doc: "Write value to pointer",
    },
    Intrinsic {
        name: "intrinsic_ptr_write",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, value
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::DerefMut),
        mlir_op: Some("llvm.store"),
        doc: "Write value to pointer (alias for ptr_write)",
    },
    // === Volatile Memory Operations (MMIO support) ===
    Intrinsic {
        name: "volatile_load",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // ptr
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::VolatileLoad),
        mlir_op: Some("llvm.load volatile"),
        doc: "Volatile memory read - prevents compiler optimization/reordering",
    },
    Intrinsic {
        name: "intrinsic_volatile_load",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // ptr
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::VolatileLoad),
        mlir_op: Some("llvm.load volatile"),
        doc: "Volatile memory read (alias for volatile_load)",
    },
    Intrinsic {
        name: "volatile_store",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, value
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::VolatileStore),
        mlir_op: Some("llvm.store volatile"),
        doc: "Volatile memory write - prevents compiler optimization/reordering",
    },
    Intrinsic {
        name: "intrinsic_volatile_store",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, value
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::VolatileStore),
        mlir_op: Some("llvm.store volatile"),
        doc: "Volatile memory write (alias for volatile_store)",
    },
    Intrinsic {
        name: "volatile_load_acquire",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // ptr
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::VolatileLoadAcquire),
        mlir_op: Some("llvm.load acquire volatile"),
        doc: "Volatile memory read with acquire semantics",
    },
    Intrinsic {
        name: "volatile_store_release",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, value
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::VolatileStoreRelease),
        mlir_op: Some("llvm.store release volatile"),
        doc: "Volatile memory write with release semantics",
    },
    Intrinsic {
        name: "hardware_fence",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // ordering
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::HardwareFence),
        mlir_op: Some("llvm.fence"),
        doc: "Hardware memory barrier - prevents CPU reordering",
    },
    Intrinsic {
        name: "ptr_offset",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::Pure,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, count
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PtrOffset),
        mlir_op: Some("llvm.getelementptr"),
        doc: "Offset pointer by element count",
    },
    Intrinsic {
        name: "intrinsic_ptr_offset",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::Pure,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, count
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PtrOffset),
        mlir_op: Some("llvm.getelementptr"),
        doc: "Offset pointer by element count (alias for ptr_offset)",
    },
    Intrinsic {
        name: "ptr_offset_mut",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::Pure,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, count
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PtrOffset),
        mlir_op: Some("llvm.getelementptr"),
        doc: "Offset mutable pointer by element count",
    },
    // =========================================================================
    // Memory Lifecycle Intrinsics
    // =========================================================================
    Intrinsic {
        name: "drop_in_place",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
        ],
        param_count: 1, // ptr
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DropInPlace),
        mlir_op: Some("verum.drop"),
        doc: "Run destructor for value at pointer",
    },
    Intrinsic {
        name: "intrinsic_drop_in_place",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Generic,
        ],
        param_count: 1, // ptr
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DropInPlace),
        mlir_op: Some("verum.drop"),
        doc: "Run destructor for value at pointer (alias for drop_in_place)",
    },
    Intrinsic {
        name: "forget",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // value
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Nop), // ownership transfer, no runtime cost
        mlir_op: None, // compile-time only
        doc: "Prevent value from being dropped",
    },
    Intrinsic {
        name: "intrinsic_forget",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // value
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Nop), // ownership transfer, no runtime cost
        mlir_op: None, // compile-time only
        doc: "Prevent value from being dropped (alias for forget)",
    },
    Intrinsic {
        name: "ptr_is_null",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // ptr
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::EqI), // compare with 0 (null)
        mlir_op: Some("llvm.icmp eq"),
        doc: "Check if pointer is null",
    },
    Intrinsic {
        name: "intrinsic_ptr_is_null",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 1, // ptr
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::EqI), // compare with 0 (null)
        mlir_op: Some("llvm.icmp eq"),
        doc: "Check if pointer is null (alias for ptr_is_null)",
    },
    Intrinsic {
        name: "null_ptr",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::ConstEval,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LoadI), // load 0
        mlir_op: Some("llvm.mlir.null"),
        doc: "Get null pointer",
    },
    Intrinsic {
        name: "intrinsic_null_ptr",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::ConstEval,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LoadI), // load 0
        mlir_op: Some("llvm.mlir.null"),
        doc: "Get null pointer (alias for null_ptr)",
    },
    Intrinsic {
        name: "null_ptr_mut",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::ConstEval,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LoadI), // load 0
        mlir_op: Some("llvm.mlir.null"),
        doc: "Get null mutable pointer",
    },
    // =========================================================================
    // Slice Creation Intrinsics
    // =========================================================================
    Intrinsic {
        name: "slice_from_raw_parts",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, len
        return_count: 1, // fat pointer (ptr, len)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MakeSlice),
        mlir_op: Some("verum.slice.from_raw"),
        doc: "Create slice from raw parts",
    },
    Intrinsic {
        name: "slice_from_raw_parts_mut",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 2, // ptr, len
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MakeSlice),
        mlir_op: Some("verum.slice.from_raw_mut"),
        doc: "Create mutable slice from raw parts",
    },
    Intrinsic {
        name: "slice_len",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1, // slice
        return_count: 1, // len
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceLen),
        mlir_op: Some("verum.slice.len"),
        doc: "Get slice length from fat pointer",
    },
    Intrinsic {
        name: "slice_as_ptr",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1, // slice
        return_count: 1, // ptr
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceAsPtr),
        mlir_op: Some("verum.slice.as_ptr"),
        doc: "Get raw pointer from slice fat pointer",
    },
    Intrinsic {
        name: "slice_as_mut_ptr",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1, // slice
        return_count: 1, // ptr
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceAsPtr),
        mlir_op: Some("verum.slice.as_mut_ptr"),
        doc: "Get mutable raw pointer from slice fat pointer",
    },
    Intrinsic {
        name: "slice_get_unchecked",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // slice, idx
        return_count: 1, // &T
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceGetUnchecked),
        mlir_op: Some("verum.slice.get_unchecked"),
        doc: "Get element at index without bounds check",
    },
    Intrinsic {
        name: "slice_get_unchecked_mut",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // slice, idx
        return_count: 1, // &mut T
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceGetUnchecked),
        mlir_op: Some("verum.slice.get_unchecked_mut"),
        doc: "Get mutable element at index without bounds check",
    },
    Intrinsic {
        name: "slice_subslice",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3, // slice, start, end
        return_count: 1, // &[T]
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceSubslice),
        mlir_op: Some("verum.slice.subslice"),
        doc: "Create subslice from start to end",
    },
    Intrinsic {
        name: "slice_subslice_mut",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3, // slice, start, end
        return_count: 1, // &mut [T]
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceSubslice),
        mlir_op: Some("verum.slice.subslice_mut"),
        doc: "Create mutable subslice from start to end",
    },
    Intrinsic {
        name: "slice_split_at",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::Inline, IntrinsicHint::Generic, IntrinsicHint::MultiReturn],
        param_count: 2, // slice, mid
        return_count: 2, // (&[T], &[T])
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceSplitAt),
        mlir_op: Some("verum.slice.split_at"),
        doc: "Split slice into two parts at index",
    },
    Intrinsic {
        name: "slice_split_at_mut",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::Inline, IntrinsicHint::Generic, IntrinsicHint::MultiReturn],
        param_count: 2, // slice, mid
        return_count: 2, // (&mut [T], &mut [T])
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SliceSplitAt),
        mlir_op: Some("verum.slice.split_at_mut"),
        doc: "Split mutable slice into two parts at index",
    },
    // =========================================================================
    // Text Intrinsics
    // =========================================================================
    Intrinsic {
        name: "text_from_static",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1, // static str
        return_count: 1, // Text
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TextFromStatic),
        mlir_op: Some("verum.text.from_static"),
        doc: "Create Text from static string literal",
    },
    Intrinsic {
        name: "text_parse_int",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // &Text
        return_count: 1, // Maybe<Int>
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TextParseInt),
        mlir_op: Some("verum.text.parse_int"),
        doc: "Parse integer from text",
    },
    Intrinsic {
        name: "text_parse_float",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // &Text
        return_count: 1, // Maybe<Float>
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TextParseFloat),
        mlir_op: Some("verum.text.parse_float"),
        doc: "Parse float from text",
    },
    Intrinsic {
        name: "int_to_text",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // Int
        return_count: 1, // Text
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::IntToText),
        mlir_op: Some("verum.text.from_int"),
        doc: "Convert integer to text",
    },
    Intrinsic {
        name: "float_to_text",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // Float
        return_count: 1, // Text
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::FloatToText),
        mlir_op: Some("verum.text.from_float"),
        doc: "Convert float to text",
    },
    Intrinsic {
        name: "text_byte_len",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1, // &Text
        return_count: 1, // Int
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TextByteLen),
        mlir_op: Some("verum.text.byte_len"),
        doc: "Get byte length of Text",
    },
    // =========================================================================
    // UTF-8 Intrinsics
    // =========================================================================
    Intrinsic {
        name: "utf8_decode_char",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // bytes, idx
        return_count: 1, // Char
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Utf8DecodeChar),
        mlir_op: Some("verum.utf8.decode_char"),
        doc: "Decode UTF-8 character from bytes",
    },
    Intrinsic {
        name: "utf8_decode_char_len",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::MultiReturn],
        param_count: 2, // bytes, idx
        return_count: 2, // (Char, Int)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Utf8DecodeChar),
        mlir_op: Some("verum.utf8.decode_char_len"),
        doc: "Decode UTF-8 character and return length",
    },
    // =========================================================================
    // Uninitialized Memory Intrinsics
    // =========================================================================
    Intrinsic {
        name: "uninit",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Uninit),
        mlir_op: Some("llvm.alloca"),
        doc: "Create uninitialized memory",
    },
    Intrinsic {
        name: "zeroed",
        category: IntrinsicCategory::Memory,
        hints: &[
            IntrinsicHint::Unsafe,
            IntrinsicHint::Generic,
            IntrinsicHint::Inline,
        ],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Zeroed),
        mlir_op: Some("llvm.alloca"),
        doc: "Create zeroed memory",
    },
    // =========================================================================
    // Heap Memory Allocation Intrinsics
    // =========================================================================
    Intrinsic {
        name: "alloc",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::MemoryEffect, IntrinsicHint::Alloc],
        param_count: 2, // size, align
        return_count: 1, // ptr
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Alloc),
        mlir_op: Some("llvm.call @malloc"),
        doc: "Allocate heap memory",
    },
    Intrinsic {
        name: "alloc_zeroed",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::MemoryEffect, IntrinsicHint::Alloc],
        param_count: 2, // size, align
        return_count: 1, // ptr
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::AllocZeroed),
        mlir_op: Some("llvm.call @calloc"),
        doc: "Allocate zeroed heap memory",
    },
    Intrinsic {
        name: "dealloc",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::MemoryEffect],
        param_count: 3, // ptr, size, align
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Dealloc),
        mlir_op: Some("llvm.call @free"),
        doc: "Deallocate heap memory",
    },
    Intrinsic {
        name: "realloc",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Unsafe, IntrinsicHint::MemoryEffect, IntrinsicHint::Alloc],
        param_count: 4, // ptr, old_size, new_size, align
        return_count: 1, // ptr
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Realloc),
        mlir_op: Some("llvm.call @realloc"),
        doc: "Reallocate heap memory",
    },
    Intrinsic {
        name: "swap",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Generic],
        param_count: 2, // a, b
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Swap),
        mlir_op: Some("verum.swap"),
        doc: "Swap two values in place",
    },
    Intrinsic {
        name: "replace",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Generic],
        param_count: 2, // dest, src
        return_count: 1, // old value
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Replace),
        mlir_op: Some("verum.replace"),
        doc: "Replace value and return old",
    },
    // =========================================================================
    // Atomic/Sync Intrinsics
    // =========================================================================
    Intrinsic {
        name: "spin_hint",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::Inline],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::AtomicFence, 0xFF), // special mode
        mlir_op: Some("llvm.intr.x86.sse2.pause"),
        doc: "CPU spin hint for busy-waiting",
    },
    Intrinsic {
        name: "memory_fence",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::SyncBarrier],
        param_count: 1, // ordering
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::AtomicFence),
        mlir_op: Some("llvm.fence"),
        doc: "Memory fence with specified ordering",
    },
    Intrinsic {
        name: "compiler_fence",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::Pure], // No runtime cost
        param_count: 1, // ordering (u8)
        return_count: 0,
        strategy: CodegenStrategy::CompileTimeConstant, // Compile-time only
        mlir_op: Some("llvm.compiler.fence"),
        doc: "Compiler fence (prevents reordering)",
    },
    // Atomic load intrinsics
    Intrinsic {
        name: "atomic_load_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 2, // ptr, ordering
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 1),
        mlir_op: Some("llvm.load atomic"),
        doc: "Atomic load of u8",
    },
    Intrinsic {
        name: "atomic_load_u16",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 2),
        mlir_op: Some("llvm.load atomic"),
        doc: "Atomic load of u16",
    },
    Intrinsic {
        name: "atomic_load_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 4),
        mlir_op: Some("llvm.load atomic"),
        doc: "Atomic load of u32",
    },
    Intrinsic {
        name: "atomic_load_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 8),
        mlir_op: Some("llvm.load atomic"),
        doc: "Atomic load of u64",
    },
    Intrinsic {
        name: "atomic_load_int",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 8), // Int is 64-bit
        mlir_op: Some("llvm.load atomic"),
        doc: "Atomic load of Int (alias for atomic_load_i64)",
    },
    Intrinsic {
        name: "atomic_load_ptr",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
        ],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 8), // pointer size
        mlir_op: Some("llvm.load atomic"),
        doc: "Atomic load of pointer",
    },
    // Atomic store intrinsics
    Intrinsic {
        name: "atomic_store_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3, // ptr, value, ordering
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 1),
        mlir_op: Some("llvm.store atomic"),
        doc: "Atomic store of u8",
    },
    Intrinsic {
        name: "atomic_store_u16",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 2),
        mlir_op: Some("llvm.store atomic"),
        doc: "Atomic store of u16",
    },
    Intrinsic {
        name: "atomic_store_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 4),
        mlir_op: Some("llvm.store atomic"),
        doc: "Atomic store of u32",
    },
    Intrinsic {
        name: "atomic_store_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 8),
        mlir_op: Some("llvm.store atomic"),
        doc: "Atomic store of u64",
    },
    Intrinsic {
        name: "atomic_store_int",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 8), // Int is 64-bit
        mlir_op: Some("llvm.store atomic"),
        doc: "Atomic store of Int (alias for atomic_store_i64)",
    },
    Intrinsic {
        name: "atomic_store_ptr",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
        ],
        param_count: 3,
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 8),
        mlir_op: Some("llvm.store atomic"),
        doc: "Atomic store of pointer",
    },
    // Atomic CAS intrinsics
    Intrinsic {
        name: "atomic_cas_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5, // ptr, expected, desired, success_order, failure_order
        return_count: 2, // (old_value, success)
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 4),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-swap for u32",
    },
    Intrinsic {
        name: "atomic_cas_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5,
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 8),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-swap for u64",
    },
    Intrinsic {
        name: "atomic_cas_ptr",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
            IntrinsicHint::Generic,
        ],
        param_count: 5,
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 8),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-swap for pointer",
    },
    // Atomic fetch operations (CAS loops)
    Intrinsic {
        name: "atomic_fetch_add_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3, // ptr, value, ordering
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAdd, 4),
        mlir_op: Some("llvm.atomicrmw add"),
        doc: "Atomic fetch-and-add for u32",
    },
    Intrinsic {
        name: "atomic_fetch_add_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAdd, 8),
        mlir_op: Some("llvm.atomicrmw add"),
        doc: "Atomic fetch-and-add for u64",
    },
    Intrinsic {
        name: "atomic_fetch_sub_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchSub, 4),
        mlir_op: Some("llvm.atomicrmw sub"),
        doc: "Atomic fetch-and-sub for u32",
    },
    Intrinsic {
        name: "atomic_fetch_sub_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchSub, 8),
        mlir_op: Some("llvm.atomicrmw sub"),
        doc: "Atomic fetch-and-sub for u64",
    },
    Intrinsic {
        name: "atomic_fetch_and_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAnd, 8),
        mlir_op: Some("llvm.atomicrmw and"),
        doc: "Atomic fetch-and-and for u64",
    },
    Intrinsic {
        name: "atomic_fetch_and_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAnd, 4),
        mlir_op: Some("llvm.atomicrmw and"),
        doc: "Atomic fetch-and-and for u32",
    },
    Intrinsic {
        name: "atomic_fetch_and_u16",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAnd, 2),
        mlir_op: Some("llvm.atomicrmw and"),
        doc: "Atomic fetch-and-and for u16",
    },
    Intrinsic {
        name: "atomic_fetch_or_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchOr, 8),
        mlir_op: Some("llvm.atomicrmw or"),
        doc: "Atomic fetch-and-or for u64",
    },
    Intrinsic {
        name: "atomic_fetch_xor_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchXor, 8),
        mlir_op: Some("llvm.atomicrmw xor"),
        doc: "Atomic fetch-and-xor for u64",
    },
    // Additional atomic intrinsics for signed types and aliases
    Intrinsic {
        name: "atomic_load_i32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 2, // ptr, ordering
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 4),
        mlir_op: Some("llvm.load atomic"),
        doc: "Atomic load for signed i32",
    },
    Intrinsic {
        name: "atomic_store_i32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3, // ptr, value, ordering
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 4),
        mlir_op: Some("llvm.store atomic"),
        doc: "Atomic store for signed i32",
    },
    Intrinsic {
        name: "atomic_cas_i32",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5, // ptr, expected, desired, success_ordering, failure_ordering
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 4),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-swap for signed i32",
    },
    Intrinsic {
        name: "atomic_compare_exchange_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5,
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 8),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-exchange for u64 (alias for atomic_cas_u64)",
    },
    Intrinsic {
        name: "atomic_compare_exchange_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5,
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 4),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-exchange for u32",
    },
    Intrinsic {
        name: "atomic_compare_exchange_i32",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5,
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 4),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-exchange for signed i32",
    },
    Intrinsic {
        name: "atomic_cmpxchg_ptr",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
            IntrinsicHint::Generic,
        ],
        param_count: 5,
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 8),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-exchange for pointer (alias for atomic_cas_ptr)",
    },
    Intrinsic {
        name: "atomic_cmpxchg_int",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5, // ptr, expected, desired, success_ordering, failure_ordering
        return_count: 2, // (old_value, success)
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 8), // Int is 64-bit
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-exchange for Int (alias for atomic_cas_i64)",
    },
    Intrinsic {
        name: "atomic_fetch_add_u16",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3, // ptr, value, ordering
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAdd, 2),
        mlir_op: Some("llvm.atomicrmw add"),
        doc: "Atomic fetch-and-add for u16",
    },
    Intrinsic {
        name: "atomic_fetch_add_int",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3, // ptr, value, ordering
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAdd, 8),
        mlir_op: Some("llvm.atomicrmw add"),
        doc: "Atomic fetch-and-add for Int (i64)",
    },
    Intrinsic {
        name: "atomic_exchange_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3, // ptr, value, ordering
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicExchange, 4),
        mlir_op: Some("llvm.atomicrmw xchg"),
        doc: "Atomic exchange for u32",
    },
    Intrinsic {
        name: "atomic_exchange_u64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicExchange, 8),
        mlir_op: Some("llvm.atomicrmw xchg"),
        doc: "Atomic exchange for u64",
    },
    Intrinsic {
        name: "atomic_exchange_i32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicExchange, 4),
        mlir_op: Some("llvm.atomicrmw xchg"),
        doc: "Atomic exchange for signed i32",
    },
    // -----------------------------------------------------------------------
    // 64-bit signed atomics (#100, task #24)
    //
    // Closes the asymmetry where signed 64-bit code had to
    // round through `_u64` with `as UInt64` cast — the
    // bit-pattern survived but the static type information
    // was lost at every atomic operation.  Same opcodes as
    // the unsigned variants (LLVM atomicrmw/load/store
    // doesn't care about signedness for add/sub/xchg/cas;
    // only max/min differ between signed and unsigned).
    // -----------------------------------------------------------------------
    Intrinsic {
        name: "atomic_load_i64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 8),
        mlir_op: Some("llvm.load atomic"),
        doc: "Atomic load for signed i64",
    },
    Intrinsic {
        name: "atomic_store_i64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 8),
        mlir_op: Some("llvm.store atomic"),
        doc: "Atomic store for signed i64",
    },
    Intrinsic {
        name: "atomic_cas_i64",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5,
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 8),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-swap for signed i64",
    },
    Intrinsic {
        name: "atomic_fetch_add_i64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAdd, 8),
        mlir_op: Some("llvm.atomicrmw add"),
        doc: "Atomic fetch-and-add for signed i64",
    },
    Intrinsic {
        name: "atomic_fetch_sub_i64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchSub, 8),
        mlir_op: Some("llvm.atomicrmw sub"),
        doc: "Atomic fetch-and-sub for signed i64",
    },
    Intrinsic {
        name: "atomic_exchange_i64",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicExchange, 8),
        mlir_op: Some("llvm.atomicrmw xchg"),
        doc: "Atomic exchange for signed i64",
    },
    // -----------------------------------------------------------------------
    // u8 atomic counters (#100, task #24)
    //
    // Enables race-hardening of single-byte counters such as
    // the CBGR slot-generation rotation in heap.vr's
    // free_block_xthread (currently uses non-atomic
    // load+store because no UInt8 atomic fetch_add was
    // available).
    // -----------------------------------------------------------------------
    Intrinsic {
        name: "atomic_fetch_add_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAdd, 1),
        mlir_op: Some("llvm.atomicrmw add"),
        doc: "Atomic fetch-and-add for u8",
    },
    Intrinsic {
        name: "atomic_fetch_sub_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchSub, 1),
        mlir_op: Some("llvm.atomicrmw sub"),
        doc: "Atomic fetch-and-sub for u8",
    },
    // -----------------------------------------------------------------------
    // u8 RMW closure (#100, task #34) — bitwise + xchg + cas.
    //
    // Closes the gap that core/sync/atomic.vr's AtomicU8
    // currently papers over with a u32-aligned CAS loop +
    // byte-masking emulation. That emulation is structurally
    // unsound in mixed-field layouts where the load_u32 reads
    // 3 neighbour-field bytes (concurrent neighbour writes
    // cause infinite retry + torn neighbour reads).  Same
    // pattern as task #24's fetch_add_u8/sub_u8 closure.
    // -----------------------------------------------------------------------
    Intrinsic {
        name: "atomic_fetch_and_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchAnd, 1),
        mlir_op: Some("llvm.atomicrmw and"),
        doc: "Atomic fetch-and-and for u8",
    },
    Intrinsic {
        name: "atomic_fetch_or_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchOr, 1),
        mlir_op: Some("llvm.atomicrmw or"),
        doc: "Atomic fetch-and-or for u8",
    },
    Intrinsic {
        name: "atomic_fetch_xor_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchXor, 1),
        mlir_op: Some("llvm.atomicrmw xor"),
        doc: "Atomic fetch-and-xor for u8",
    },
    Intrinsic {
        name: "atomic_exchange_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicExchange, 1),
        mlir_op: Some("llvm.atomicrmw xchg"),
        doc: "Atomic exchange for u8",
    },
    Intrinsic {
        name: "atomic_cas_u8",
        category: IntrinsicCategory::Atomic,
        hints: &[
            IntrinsicHint::MemoryEffect,
            IntrinsicHint::Inline,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 5,
        return_count: 2,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 1),
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Atomic compare-and-swap for u8",
    },
    // -----------------------------------------------------------------------
    // Symmetric coverage closure: fetch_sub_u16, fetch_or_u32,
    // fetch_xor_u32 (each was missing the symmetric variant
    // even though the unsigned-add / u64 counterparts existed).
    // -----------------------------------------------------------------------
    Intrinsic {
        name: "atomic_fetch_sub_u16",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchSub, 2),
        mlir_op: Some("llvm.atomicrmw sub"),
        doc: "Atomic fetch-and-sub for u16",
    },
    Intrinsic {
        name: "atomic_fetch_or_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchOr, 4),
        mlir_op: Some("llvm.atomicrmw or"),
        doc: "Atomic fetch-and-or for u32",
    },
    Intrinsic {
        name: "atomic_fetch_xor_u32",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::AtomicFetchXor, 4),
        mlir_op: Some("llvm.atomicrmw xor"),
        doc: "Atomic fetch-and-xor for u32",
    },
    Intrinsic {
        name: "atomic_fence",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::SyncBarrier],
        param_count: 1, // ordering
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::AtomicFence),
        mlir_op: Some("llvm.intr.atomic.fence"),
        doc: "Atomic memory fence",
    },
    // =========================================================================
    // System Call Intrinsics
    // =========================================================================
    Intrinsic {
        name: "syscall0",
        category: IntrinsicCategory::Syscall,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1, // num
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::SyscallLinux, 0),
        mlir_op: Some("verum.sys.syscall"),
        doc: "Raw syscall with 0 arguments",
    },
    Intrinsic {
        name: "syscall1",
        category: IntrinsicCategory::Syscall,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 2, // num, a1
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::SyscallLinux, 1),
        mlir_op: Some("verum.sys.syscall"),
        doc: "Raw syscall with 1 argument",
    },
    Intrinsic {
        name: "syscall2",
        category: IntrinsicCategory::Syscall,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::SyscallLinux, 2),
        mlir_op: Some("verum.sys.syscall"),
        doc: "Raw syscall with 2 arguments",
    },
    Intrinsic {
        name: "syscall3",
        category: IntrinsicCategory::Syscall,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 4,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::SyscallLinux, 3),
        mlir_op: Some("verum.sys.syscall"),
        doc: "Raw syscall with 3 arguments",
    },
    Intrinsic {
        name: "syscall4",
        category: IntrinsicCategory::Syscall,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 5,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::SyscallLinux, 4),
        mlir_op: Some("verum.sys.syscall"),
        doc: "Raw syscall with 4 arguments",
    },
    Intrinsic {
        name: "syscall5",
        category: IntrinsicCategory::Syscall,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 6,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::SyscallLinux, 5),
        mlir_op: Some("verum.sys.syscall"),
        doc: "Raw syscall with 5 arguments",
    },
    Intrinsic {
        name: "syscall6",
        category: IntrinsicCategory::Syscall,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 7, // num, a1-a6
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::SyscallLinux, 6),
        mlir_op: Some("verum.sys.syscall"),
        doc: "Raw syscall with 6 arguments",
    },
    // =========================================================================
    // TLS Intrinsics
    // =========================================================================
    Intrinsic {
        name: "tls_get_base",
        category: IntrinsicCategory::Tls,
        hints: &[IntrinsicHint::Inline],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::TlsGet, 0), // slot 0 = base
        mlir_op: Some("llvm.read_register"),
        doc: "Get TLS base pointer",
    },
    Intrinsic {
        name: "tls_slot_get",
        category: IntrinsicCategory::Tls,
        hints: &[IntrinsicHint::Inline, IntrinsicHint::Hot],
        param_count: 1, // slot
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet),
        mlir_op: Some("llvm.load"),
        doc: "Get value from TLS slot (~2ns)",
    },
    Intrinsic {
        name: "tls_slot_set",
        category: IntrinsicCategory::Tls,
        hints: &[IntrinsicHint::Inline, IntrinsicHint::Hot],
        param_count: 2, // slot, value
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsSet),
        mlir_op: Some("llvm.store"),
        doc: "Set value in TLS slot",
    },
    Intrinsic {
        name: "tls_slot_clear",
        category: IntrinsicCategory::Tls,
        hints: &[IntrinsicHint::Inline],
        param_count: 1, // slot
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsSet), // with null value
        mlir_op: Some("llvm.store"),
        doc: "Clear TLS slot (set to null)",
    },
    Intrinsic {
        name: "tls_slot_has",
        category: IntrinsicCategory::Tls,
        hints: &[IntrinsicHint::Inline, IntrinsicHint::Hot],
        param_count: 1, // slot
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet), // + null check
        mlir_op: Some("llvm.load"),
        doc: "Check if TLS slot is set",
    },
    Intrinsic {
        name: "tls_frame_push",
        category: IntrinsicCategory::Tls,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::PushContext),
        mlir_op: Some("verum.context.push"),
        doc: "Push TLS context frame",
    },
    Intrinsic {
        name: "tls_frame_pop",
        category: IntrinsicCategory::Tls,
        hints: &[],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::PopContext),
        mlir_op: Some("verum.context.pop"),
        doc: "Pop TLS context frame",
    },
    // =========================================================================
    // Bit Manipulation Intrinsics
    // =========================================================================
    Intrinsic {
        name: "clz",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Clz),
        mlir_op: Some("llvm.intr.ctlz"),
        doc: "Count leading zeros",
    },
    Intrinsic {
        name: "ctz",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Ctz),
        mlir_op: Some("llvm.intr.cttz"),
        doc: "Count trailing zeros",
    },
    Intrinsic {
        name: "ilog2",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Ilog2),
        mlir_op: None,
        doc: "Integer log base 2 (floor)",
    },
    Intrinsic {
        name: "popcnt",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Popcnt),
        mlir_op: Some("llvm.intr.ctpop"),
        doc: "Population count (count set bits)",
    },
    Intrinsic {
        name: "bswap",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Bswap),
        mlir_op: Some("llvm.intr.bswap"),
        doc: "Byte swap (reverse byte order)",
    },
    Intrinsic {
        name: "bitreverse",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Bitreverse),
        mlir_op: Some("llvm.intr.bitreverse"),
        doc: "Reverse bits",
    },
    Intrinsic {
        name: "rotate_left",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // val, amount
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RotateLeft),
        mlir_op: Some("llvm.intr.fshl"),
        doc: "Rotate bits left",
    },
    Intrinsic {
        name: "rotate_right",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RotateRight),
        mlir_op: Some("llvm.intr.fshr"),
        doc: "Rotate bits right",
    },
    Intrinsic {
        name: "rotl",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // val, amount
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RotateLeft),
        mlir_op: Some("llvm.intr.fshl"),
        doc: "Rotate bits left (alias for rotate_left)",
    },
    Intrinsic {
        name: "rotr",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RotateRight),
        mlir_op: Some("llvm.intr.fshr"),
        doc: "Rotate bits right (alias for rotate_right)",
    },
    // Type-suffixed bit manipulation intrinsics (for direct string-based intrinsic calls)
    Intrinsic {
        name: "clz_u64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Clz),
        mlir_op: Some("llvm.intr.ctlz"),
        doc: "Count leading zeros (u64)",
    },
    Intrinsic {
        name: "clz_u32",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Clz),
        mlir_op: Some("llvm.intr.ctlz"),
        doc: "Count leading zeros (u32)",
    },
    Intrinsic {
        name: "ctz_u64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Ctz),
        mlir_op: Some("llvm.intr.cttz"),
        doc: "Count trailing zeros (u64)",
    },
    Intrinsic {
        name: "ctz_u32",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Ctz),
        mlir_op: Some("llvm.intr.cttz"),
        doc: "Count trailing zeros (u32)",
    },
    Intrinsic {
        name: "popcnt_u64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Popcnt),
        mlir_op: Some("llvm.intr.ctpop"),
        doc: "Population count (u64)",
    },
    Intrinsic {
        name: "popcnt_u32",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Popcnt),
        mlir_op: Some("llvm.intr.ctpop"),
        doc: "Population count (u32)",
    },
    Intrinsic {
        name: "bswap_u64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Bswap),
        mlir_op: Some("llvm.intr.bswap"),
        doc: "Byte swap (u64)",
    },
    Intrinsic {
        name: "bswap_u32",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Bswap),
        mlir_op: Some("llvm.intr.bswap"),
        doc: "Byte swap (u32)",
    },
    Intrinsic {
        name: "bswap_u16",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Bswap),
        mlir_op: Some("llvm.intr.bswap"),
        doc: "Byte swap (u16)",
    },
    // =========================================================================
    // Overflow-Checked Arithmetic
    // =========================================================================
    Intrinsic {
        name: "add_overflow",
        category: IntrinsicCategory::Overflow,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 2,
        return_count: 2, // (result, overflowed)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::OverflowingAdd),
        mlir_op: Some("llvm.intr.sadd.with.overflow"),
        doc: "Add with overflow check, returns (result, overflow_flag)",
    },
    Intrinsic {
        name: "sub_overflow",
        category: IntrinsicCategory::Overflow,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 2,
        return_count: 2,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::OverflowingSub),
        mlir_op: Some("llvm.intr.ssub.with.overflow"),
        doc: "Subtract with overflow check, returns (result, overflow_flag)",
    },
    Intrinsic {
        name: "mul_overflow",
        category: IntrinsicCategory::Overflow,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::MultiReturn,
        ],
        param_count: 2,
        return_count: 2,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::OverflowingMul),
        mlir_op: Some("llvm.intr.smul.with.overflow"),
        doc: "Multiply with overflow check, returns (result, overflow_flag)",
    },
    // =========================================================================
    // Saturating Arithmetic
    // =========================================================================
    Intrinsic {
        name: "saturating_add",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::SaturatingAdd),
        mlir_op: Some("llvm.intr.sadd.sat"),
        doc: "Saturating add",
    },
    Intrinsic {
        name: "saturating_sub",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::SaturatingSub),
        mlir_op: Some("llvm.intr.ssub.sat"),
        doc: "Saturating subtract",
    },
    Intrinsic {
        name: "saturating_mul",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::SaturatingMul),
        mlir_op: None, // No direct LLVM intrinsic for saturating mul
        doc: "Saturating multiply",
    },
    // -------------------------------------------------------------------------
    // Saturating signed unary (#100, task #25)
    //
    // Closes the gap where core/intrinsics/arithmetic.vr declared
    // `saturating_neg<T>` / `saturating_abs<T>` (lines 257/260)
    // but the compiler had no lowering — calls would panic at
    // codegen.  Both saturate `T::MIN` to `T::MAX`; the only
    // value where standard negate / abs would overflow.
    // -------------------------------------------------------------------------
    Intrinsic {
        name: "saturating_neg",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::SaturatingNeg),
        mlir_op: None, // No direct LLVM intrinsic; lowered as ssub_sat(0, x)
        doc: "Saturating signed negation (T::MIN → T::MAX)",
    },
    Intrinsic {
        name: "saturating_abs",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::SaturatingAbs),
        mlir_op: Some("llvm.intr.abs"), // with poison-on-overflow=false
        doc: "Saturating signed absolute value (|T::MIN| → T::MAX)",
    },
    // -------------------------------------------------------------------------
    // Checked unary signed (#100, task #25)
    //
    // checked_neg was declared at arithmetic.vr:127 but had no
    // compiler-side entry; checked_abs was missing entirely.
    // Both return Maybe<T>: Some(value) for the typical case,
    // None for the unique-overflow case (T::MIN for signed).
    // -------------------------------------------------------------------------
    Intrinsic {
        name: "checked_neg",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1, // returns Maybe<T> (single allocated value)
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::CheckedNeg),
        mlir_op: None,
        doc: "Checked signed negation, None on T::MIN",
    },
    Intrinsic {
        name: "checked_abs",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::CheckedAbs),
        mlir_op: None,
        doc: "Checked signed absolute value, None on T::MIN",
    },
    // =========================================================================
    // Wrapping Arithmetic
    // =========================================================================
    Intrinsic {
        name: "wrapping_add",
        category: IntrinsicCategory::Wrapping,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::ConstEval,
        ],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::AddI),
        mlir_op: Some("arith.addi"),
        doc: "Wrapping addition",
    },
    Intrinsic {
        name: "wrapping_sub",
        category: IntrinsicCategory::Wrapping,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::ConstEval,
        ],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::SubI),
        mlir_op: Some("arith.subi"),
        doc: "Wrapping subtraction",
    },
    Intrinsic {
        name: "wrapping_mul",
        category: IntrinsicCategory::Wrapping,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::ConstEval,
        ],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::MulI),
        mlir_op: Some("arith.muli"),
        doc: "Wrapping multiplication",
    },
    Intrinsic {
        name: "wrapping_neg",
        category: IntrinsicCategory::Wrapping,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::ConstEval,
        ],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::NegI),
        mlir_op: Some("arith.negsi"),
        doc: "Wrapping negation",
    },
    Intrinsic {
        name: "wrapping_shl",
        category: IntrinsicCategory::Wrapping,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::ConstEval,
        ],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Shl),
        mlir_op: Some("arith.shli"),
        doc: "Wrapping shift left",
    },
    Intrinsic {
        name: "wrapping_shr",
        category: IntrinsicCategory::Wrapping,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::Inline,
            IntrinsicHint::Generic,
            IntrinsicHint::ConstEval,
        ],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Shr),
        mlir_op: Some("arith.shrsi"),
        doc: "Wrapping shift right",
    },
    // =========================================================================
    // Type-Specific Wrapping Arithmetic
    // =========================================================================
    // UInt8 wrapping
    Intrinsic {
        name: "wrapping_add_u8",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingAdd, 8, false),
        mlir_op: Some("arith.addi"),
        doc: "Wrapping addition for UInt8",
    },
    Intrinsic {
        name: "wrapping_sub_u8",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingSub, 8, false),
        mlir_op: Some("arith.subi"),
        doc: "Wrapping subtraction for UInt8",
    },
    Intrinsic {
        name: "wrapping_mul_u8",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingMul, 8, false),
        mlir_op: Some("arith.muli"),
        doc: "Wrapping multiplication for UInt8",
    },
    // Int8 wrapping
    Intrinsic {
        name: "wrapping_add_i8",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingAdd, 8, true),
        mlir_op: Some("arith.addi"),
        doc: "Wrapping addition for Int8",
    },
    Intrinsic {
        name: "wrapping_sub_i8",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingSub, 8, true),
        mlir_op: Some("arith.subi"),
        doc: "Wrapping subtraction for Int8",
    },
    Intrinsic {
        name: "wrapping_neg_i8",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingNeg, 8, true),
        mlir_op: Some("arith.negsi"),
        doc: "Wrapping negation for Int8",
    },
    // UInt16 wrapping
    Intrinsic {
        name: "wrapping_add_u16",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingAdd, 16, false),
        mlir_op: Some("arith.addi"),
        doc: "Wrapping addition for UInt16",
    },
    Intrinsic {
        name: "wrapping_sub_u16",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingSub, 16, false),
        mlir_op: Some("arith.subi"),
        doc: "Wrapping subtraction for UInt16",
    },
    // UInt32 wrapping
    Intrinsic {
        name: "wrapping_add_u32",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingAdd, 32, false),
        mlir_op: Some("arith.addi"),
        doc: "Wrapping addition for UInt32",
    },
    Intrinsic {
        name: "wrapping_sub_u32",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingSub, 32, false),
        mlir_op: Some("arith.subi"),
        doc: "Wrapping subtraction for UInt32",
    },
    Intrinsic {
        name: "wrapping_shl_u32",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingShl, 32, false),
        mlir_op: Some("arith.shli"),
        doc: "Wrapping shift left for UInt32",
    },
    Intrinsic {
        name: "wrapping_shr_u32",
        category: IntrinsicCategory::Wrapping,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::WrappingOpcode(ArithSubOpcode::WrappingShr, 32, false),
        mlir_op: Some("arith.shrui"),
        doc: "Wrapping shift right for UInt32",
    },
    // Type-Specific Saturating Arithmetic
    // UInt8 saturating
    Intrinsic {
        name: "saturating_add_u8",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingAdd, 8, false),
        mlir_op: None,
        doc: "Saturating addition for UInt8",
    },
    Intrinsic {
        name: "saturating_sub_u8",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingSub, 8, false),
        mlir_op: None,
        doc: "Saturating subtraction for UInt8",
    },
    Intrinsic {
        name: "saturating_mul_u8",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingMul, 8, false),
        mlir_op: None,
        doc: "Saturating multiplication for UInt8",
    },
    // Int8 saturating
    Intrinsic {
        name: "saturating_add_i8",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingAdd, 8, true),
        mlir_op: None,
        doc: "Saturating addition for Int8",
    },
    Intrinsic {
        name: "saturating_sub_i8",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingSub, 8, true),
        mlir_op: None,
        doc: "Saturating subtraction for Int8",
    },
    Intrinsic {
        name: "saturating_mul_i8",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingMul, 8, true),
        mlir_op: None,
        doc: "Saturating multiplication for Int8",
    },
    // UInt16 saturating
    Intrinsic {
        name: "saturating_add_u16",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingAdd, 16, false),
        mlir_op: None,
        doc: "Saturating addition for UInt16",
    },
    Intrinsic {
        name: "saturating_sub_u16",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingSub, 16, false),
        mlir_op: None,
        doc: "Saturating subtraction for UInt16",
    },
    Intrinsic {
        name: "saturating_mul_u16",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingMul, 16, false),
        mlir_op: None,
        doc: "Saturating multiplication for UInt16",
    },
    // Int16 saturating
    Intrinsic {
        name: "saturating_add_i16",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingAdd, 16, true),
        mlir_op: None,
        doc: "Saturating addition for Int16",
    },
    Intrinsic {
        name: "saturating_sub_i16",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingSub, 16, true),
        mlir_op: None,
        doc: "Saturating subtraction for Int16",
    },
    Intrinsic {
        name: "saturating_mul_i16",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingMul, 16, true),
        mlir_op: None,
        doc: "Saturating multiplication for Int16",
    },
    // UInt32 saturating
    Intrinsic {
        name: "saturating_add_u32",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingAdd, 32, false),
        mlir_op: None,
        doc: "Saturating addition for UInt32",
    },
    Intrinsic {
        name: "saturating_sub_u32",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingSub, 32, false),
        mlir_op: None,
        doc: "Saturating subtraction for UInt32",
    },
    Intrinsic {
        name: "saturating_mul_u32",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingMul, 32, false),
        mlir_op: None,
        doc: "Saturating multiplication for UInt32",
    },
    // Int32 saturating
    Intrinsic {
        name: "saturating_add_i32",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingAdd, 32, true),
        mlir_op: None,
        doc: "Saturating addition for Int32",
    },
    Intrinsic {
        name: "saturating_sub_i32",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingSub, 32, true),
        mlir_op: None,
        doc: "Saturating subtraction for Int32",
    },
    Intrinsic {
        name: "saturating_mul_i32",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingMul, 32, true),
        mlir_op: None,
        doc: "Saturating multiplication for Int32",
    },
    // UInt64 and Int64 saturating (64-bit)
    Intrinsic {
        name: "saturating_add_u64",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingAdd, 64, false),
        mlir_op: None,
        doc: "Saturating addition for UInt64",
    },
    Intrinsic {
        name: "saturating_add_i64",
        category: IntrinsicCategory::Saturating,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::SaturatingOpcode(ArithSubOpcode::SaturatingAdd, 64, true),
        mlir_op: None,
        doc: "Saturating addition for Int64",
    },
    // =========================================================================
    // Float Math Intrinsics
    // =========================================================================
    // NOTE: sqrt_f64, sin_f64, cos_f64, tan_f64, asin_f64, acos_f64, atan_f64, atan2_f64,
    // exp_f64, log_f64, log10_f64 are defined with ConstEval hints in Float Math Intrinsics section
    Intrinsic {
        name: "pow_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2, // base, exp
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::PowF64),
        mlir_op: Some("math.powf"),
        doc: "Power",
    },
    Intrinsic {
        name: "floor_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::FloorF64),
        mlir_op: Some("math.floor"),
        doc: "Floor (round down) - returns Float",
    },
    Intrinsic {
        name: "ceil_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CeilF64),
        mlir_op: Some("math.ceil"),
        doc: "Ceiling (round up) - returns Float",
    },
    Intrinsic {
        name: "round_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::RoundF64),
        mlir_op: Some("math.round"),
        doc: "Round to nearest",
    },
    Intrinsic {
        name: "abs_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::AbsF),
        mlir_op: Some("math.absf"),
        doc: "Absolute value",
    },
    // =========================================================================
    // Control Flow Intrinsics
    // =========================================================================
    Intrinsic {
        name: "panic",
        category: IntrinsicCategory::Control,
        hints: &[IntrinsicHint::Cold, IntrinsicHint::MayTrap],
        param_count: 1, // message
        return_count: 0, // ! (never returns)
        strategy: CodegenStrategy::DirectOpcode(Opcode::Panic),
        mlir_op: Some("verum.trap"),
        doc: "Trigger panic with message",
    },
    Intrinsic {
        name: "panic_impl",
        category: IntrinsicCategory::Control,
        hints: &[IntrinsicHint::Cold, IntrinsicHint::MayTrap],
        param_count: 1,
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Panic),
        mlir_op: Some("verum.trap"),
        doc: "Panic implementation",
    },
    Intrinsic {
        name: "unreachable",
        category: IntrinsicCategory::Control,
        hints: &[IntrinsicHint::Cold],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Unreachable),
        mlir_op: Some("llvm.unreachable"),
        doc: "Mark code as unreachable",
    },
    Intrinsic {
        name: "unreachable_unchecked",
        category: IntrinsicCategory::Control,
        hints: &[IntrinsicHint::Cold, IntrinsicHint::Unsafe],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Unreachable),
        mlir_op: Some("llvm.unreachable"),
        doc: "Unreachable (UB if reached)",
    },
    Intrinsic {
        name: "abort",
        category: IntrinsicCategory::Control,
        hints: &[IntrinsicHint::Cold, IntrinsicHint::MayTrap],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Unreachable),
        mlir_op: Some("llvm.trap"),
        doc: "Abort immediately",
    },
    Intrinsic {
        name: "debug_assert",
        category: IntrinsicCategory::Control,
        hints: &[IntrinsicHint::Cold],
        param_count: 2, // condition, message
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Assert),
        mlir_op: Some("verum.debug_assert"),
        doc: "Debug assertion (release: no-op)",
    },
    // =========================================================================
    // CBGR Intrinsics
    // =========================================================================
    Intrinsic {
        name: "cbgr_validate",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Generic],
        param_count: 1, // reference
        return_count: 1, // bool
        strategy: CodegenStrategy::DirectOpcode(Opcode::ChkRef),
        mlir_op: Some("verum.cbgr.validate"),
        doc: "Validate CBGR reference",
    },
    Intrinsic {
        name: "cbgr_current_epoch",
        category: IntrinsicCategory::Cbgr,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet), // reads global epoch
        mlir_op: Some("verum.cbgr.epoch"),
        doc: "Get current CBGR epoch",
    },
    Intrinsic {
        name: "cbgr_advance_epoch",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::SyncBarrier],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Nop),
        mlir_op: Some("verum.cbgr.advance_epoch"),
        doc: "Advance CBGR epoch",
    },
    Intrinsic {
        name: "cbgr_get_generation",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Unsafe],
        param_count: 1, // ptr
        return_count: 1, // generation
        strategy: CodegenStrategy::DirectOpcode(Opcode::Deref), // read header
        mlir_op: Some("verum.cbgr.generation"),
        doc: "Get generation counter for allocation",
    },
    // =========================================================================
    // Futex Intrinsics
    // =========================================================================
    Intrinsic {
        name: "futex_wait",
        category: IntrinsicCategory::Futex,
        hints: &[IntrinsicHint::SyncBarrier],
        param_count: 3, // addr, expected, timeout_ns
        return_count: 1, // result
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::FutexWait),
        mlir_op: Some("verum.futex.wait"),
        doc: "Wait on memory address",
    },
    Intrinsic {
        name: "futex_wake",
        category: IntrinsicCategory::Futex,
        hints: &[IntrinsicHint::SyncBarrier],
        param_count: 2, // addr, count
        return_count: 1, // woken count
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::FutexWake),
        mlir_op: Some("verum.futex.wake"),
        doc: "Wake threads waiting on address",
    },
    Intrinsic {
        name: "futex_wake_one",
        category: IntrinsicCategory::Futex,
        hints: &[IntrinsicHint::SyncBarrier, IntrinsicHint::Inline],
        param_count: 1, // addr
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::FutexWake),
        mlir_op: Some("verum.futex.wake"),
        doc: "Wake one thread",
    },
    Intrinsic {
        name: "futex_wake_all",
        category: IntrinsicCategory::Futex,
        hints: &[IntrinsicHint::SyncBarrier],
        param_count: 1, // addr
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::FutexWake),
        mlir_op: Some("verum.futex.wake"),
        doc: "Wake all threads",
    },
    // =========================================================================
    // Spinlock Intrinsics
    // =========================================================================
    Intrinsic {
        name: "spinlock_try_lock",
        category: IntrinsicCategory::Spinlock,
        hints: &[IntrinsicHint::Inline, IntrinsicHint::MemoryEffect],
        param_count: 1, // lock
        return_count: 1, // bool
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicCas, 4), // CAS 0→1
        mlir_op: Some("llvm.cmpxchg"),
        doc: "Try to acquire spinlock",
    },
    Intrinsic {
        name: "spinlock_lock",
        category: IntrinsicCategory::Spinlock,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::SyncBarrier],
        param_count: 1, // lock
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SpinlockLock),
        mlir_op: Some("verum.spinlock.lock"),
        doc: "Acquire spinlock (spinning)",
    },
    Intrinsic {
        name: "spinlock_unlock",
        category: IntrinsicCategory::Spinlock,
        hints: &[IntrinsicHint::Inline, IntrinsicHint::MemoryEffect],
        param_count: 1, // lock
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicStore, 4), // store 0
        mlir_op: Some("llvm.store atomic"),
        doc: "Release spinlock",
    },
    Intrinsic {
        name: "spinlock_is_locked",
        category: IntrinsicCategory::Spinlock,
        hints: &[IntrinsicHint::Inline, IntrinsicHint::MemoryEffect],
        param_count: 1, // lock
        return_count: 1, // bool
        strategy: CodegenStrategy::OpcodeWithSize(Opcode::AtomicLoad, 4),
        mlir_op: Some("llvm.load atomic"),
        doc: "Check if spinlock is held",
    },
    // =========================================================================
    // Platform Detection Intrinsics
    // =========================================================================
    Intrinsic {
        name: "is_debug",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("llvm.mlir.constant"),
        doc: "Check if debug mode",
    },
    Intrinsic {
        name: "target_os",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("llvm.mlir.constant"),
        doc: "Get target OS (0-4)",
    },
    Intrinsic {
        name: "target_arch",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("llvm.mlir.constant"),
        doc: "Get target architecture (0-4)",
    },
    Intrinsic {
        name: "num_cpus",
        category: IntrinsicCategory::Platform,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant, // Returns 1 in interpreter
        mlir_op: Some("verum.sys.num_cpus"),
        doc: "Get number of CPU cores",
    },
    // =========================================================================
    // Execution Tier Intrinsics
    // =========================================================================
    Intrinsic {
        name: "tier_promote",
        category: IntrinsicCategory::Tier,
        hints: &[],
        param_count: 1, // func_id
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Nop), // No-op in interpreter
        mlir_op: None,
        doc: "Request JIT compilation",
    },
    Intrinsic {
        name: "is_interpreted",
        category: IntrinsicCategory::Tier,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant, // false in AOT, true in interpreter
        mlir_op: Some("llvm.mlir.constant"),
        doc: "Check if running in interpreter",
    },
    Intrinsic {
        name: "get_tier",
        category: IntrinsicCategory::Tier,
        hints: &[IntrinsicHint::Pure],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant, // Returns 0 in interpreter
        mlir_op: Some("llvm.mlir.constant"),
        doc: "Get current execution tier (0-3)",
    },
    // =========================================================================
    // Time Intrinsics
    // =========================================================================
    Intrinsic {
        name: "monotonic_nanos",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MonotonicNanos),
        mlir_op: Some("verum.time.monotonic"),
        doc: "Get monotonic time in nanoseconds",
    },
    Intrinsic {
        name: "realtime_secs",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RealtimeSecs),
        mlir_op: Some("verum.time.realtime"),
        doc: "Get wall-clock time as Unix timestamp",
    },
    // Duration intrinsics (pure arithmetic on UInt64 nanoseconds)
    Intrinsic {
        name: "time_duration_from_nanos",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationFromNanos),
        mlir_op: None,
        doc: "Create Duration from nanoseconds (identity)",
    },
    Intrinsic {
        name: "time_duration_from_micros",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationFromMicros),
        mlir_op: None,
        doc: "Create Duration from microseconds",
    },
    Intrinsic {
        name: "time_duration_from_millis",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationFromMillis),
        mlir_op: None,
        doc: "Create Duration from milliseconds",
    },
    Intrinsic {
        name: "time_duration_from_secs",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationFromSecs),
        mlir_op: None,
        doc: "Create Duration from seconds",
    },
    Intrinsic {
        name: "time_duration_as_nanos",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationAsNanos),
        mlir_op: None,
        doc: "Get Duration as nanoseconds (identity)",
    },
    Intrinsic {
        name: "time_duration_as_micros",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationAsMicros),
        mlir_op: None,
        doc: "Get Duration as microseconds",
    },
    Intrinsic {
        name: "time_duration_as_millis",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationAsMillis),
        mlir_op: None,
        doc: "Get Duration as milliseconds",
    },
    Intrinsic {
        name: "time_duration_as_secs",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationAsSecs),
        mlir_op: None,
        doc: "Get Duration as seconds",
    },
    Intrinsic {
        name: "time_duration_is_zero",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationIsZero),
        mlir_op: None,
        doc: "Check if Duration is zero",
    },
    Intrinsic {
        name: "time_duration_add",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationAdd),
        mlir_op: None,
        doc: "Add two Durations",
    },
    Intrinsic {
        name: "time_duration_saturating_add",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationSaturatingAdd),
        mlir_op: None,
        doc: "Add Durations (ceiling at MAX)",
    },
    Intrinsic {
        name: "time_duration_saturating_sub",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationSaturatingSub),
        mlir_op: None,
        doc: "Subtract Durations (floor at 0)",
    },
    Intrinsic {
        name: "time_duration_subsec_nanos",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DurationSubsecNanos),
        mlir_op: None,
        doc: "Get sub-second nanosecond component",
    },
    // Instant intrinsics (system calls)
    Intrinsic {
        name: "time_instant_now",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::InstantNow),
        mlir_op: None,
        doc: "Get current monotonic instant",
    },
    Intrinsic {
        name: "time_instant_elapsed",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::InstantElapsed),
        mlir_op: None,
        doc: "Get elapsed time since instant",
    },
    Intrinsic {
        name: "time_instant_duration_since",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::InstantDurationSince),
        mlir_op: None,
        doc: "Get Duration between two instants",
    },
    // System time intrinsics
    Intrinsic {
        name: "time_monotonic_micros",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TimeMonotonicMicros),
        mlir_op: None,
        doc: "Get monotonic time in microseconds",
    },
    Intrinsic {
        name: "time_monotonic_millis",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TimeMonotonicMillis),
        mlir_op: None,
        doc: "Get monotonic time in milliseconds",
    },
    Intrinsic {
        name: "time_monotonic_raw_nanos",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MonotonicNanos),
        mlir_op: None,
        doc: "Get raw monotonic time in nanoseconds",
    },
    Intrinsic {
        name: "time_realtime_nanos",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RealtimeNanos),
        mlir_op: None,
        doc: "Get realtime in nanoseconds",
    },
    Intrinsic {
        name: "time_unix_timestamp",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TimeUnixTimestamp),
        mlir_op: None,
        doc: "Get Unix timestamp in seconds",
    },
    // Sleep intrinsics
    Intrinsic {
        name: "time_sleep_ms",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TimeSleepMs),
        mlir_op: None,
        doc: "Sleep for milliseconds",
    },
    Intrinsic {
        name: "time_sleep_us",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TimeSleepUs),
        mlir_op: None,
        doc: "Sleep for microseconds",
    },
    Intrinsic {
        name: "time_sleep_duration",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TimeSleepDuration),
        mlir_op: None,
        doc: "Sleep for a Duration",
    },
    // Stopwatch intrinsics (record type: {start, running, accumulated})
    Intrinsic {
        name: "time_stopwatch_new",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::StopwatchNew),
        mlir_op: None,
        doc: "Create a new running Stopwatch",
    },
    Intrinsic {
        name: "time_stopwatch_elapsed",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::StopwatchElapsed),
        mlir_op: None,
        doc: "Get elapsed time from Stopwatch",
    },
    Intrinsic {
        name: "time_stopwatch_stop",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::StopwatchStop),
        mlir_op: None,
        doc: "Stop the Stopwatch",
    },
    Intrinsic {
        name: "time_stopwatch_start",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::StopwatchStart),
        mlir_op: None,
        doc: "Start/restart the Stopwatch",
    },
    Intrinsic {
        name: "time_stopwatch_reset",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::StopwatchReset),
        mlir_op: None,
        doc: "Reset the Stopwatch",
    },
    // PerfCounter intrinsics
    Intrinsic {
        name: "time_perf_counter_now",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PerfCounterNow),
        mlir_op: None,
        doc: "Get current performance counter",
    },
    Intrinsic {
        name: "time_perf_counter_elapsed_since",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PerfCounterElapsedSince),
        mlir_op: None,
        doc: "Get Duration since a PerfCounter",
    },
    Intrinsic {
        name: "time_perf_counter_as_nanos",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PerfCounterAsNanos),
        mlir_op: None,
        doc: "Get PerfCounter value as nanoseconds",
    },
    // DeadlineTimer intrinsics
    Intrinsic {
        name: "time_deadline_timer_from_duration",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DeadlineTimerFromDuration),
        mlir_op: None,
        doc: "Create a DeadlineTimer from Duration",
    },
    Intrinsic {
        name: "time_deadline_timer_is_expired",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DeadlineTimerIsExpired),
        mlir_op: None,
        doc: "Check if timer has expired",
    },
    Intrinsic {
        name: "time_deadline_timer_remaining",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::DeadlineTimerRemaining),
        mlir_op: None,
        doc: "Get remaining time until deadline",
    },
    // =========================================================================
    // Random Number Generation Intrinsics
    // =========================================================================
    Intrinsic {
        name: "random_u64",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RandomU64),
        mlir_op: None,
        doc: "Generate cryptographically secure random u64",
    },
    Intrinsic {
        name: "random_float",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RandomFloat),
        mlir_op: None,
        doc: "Generate random float in [0, 1)",
    },
    // =========================================================================
    // Async/Executor Intrinsics (Library-level)
    // =========================================================================
    Intrinsic {
        name: "spawn_with_env",
        category: IntrinsicCategory::Async,
        hints: &[IntrinsicHint::Generic],
        param_count: 2, // future, task_id
        return_count: 1, // JoinHandle
        strategy: CodegenStrategy::DirectOpcode(Opcode::Spawn),
        mlir_op: Some("verum.async.spawn"),
        doc: "Spawn async task with environment",
    },
    Intrinsic {
        name: "spawn_supervised",
        category: IntrinsicCategory::Async,
        hints: &[IntrinsicHint::Generic],
        param_count: 4, // supervisor, future, spec, env
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Spawn),
        mlir_op: Some("verum.async.spawn_supervised"),
        doc: "Spawn supervised task",
    },
    Intrinsic {
        name: "executor_spawn",
        category: IntrinsicCategory::Async,
        hints: &[IntrinsicHint::Generic],
        param_count: 2, // executor, task
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Spawn),
        mlir_op: Some("verum.async.spawn"),
        doc: "Spawn task on executor",
    },
    Intrinsic {
        name: "executor_block_on",
        category: IntrinsicCategory::Async,
        hints: &[IntrinsicHint::Generic],
        param_count: 2, // executor, future
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Await),
        mlir_op: Some("verum.async.block_on"),
        doc: "Block on future until completion",
    },
    Intrinsic {
        name: "single_thread_block_on",
        category: IntrinsicCategory::Async,
        hints: &[IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Await),
        mlir_op: Some("verum.async.block_on"),
        doc: "Block on future (single-threaded)",
    },
    Intrinsic {
        name: "future_poll_sync",
        category: IntrinsicCategory::Async,
        hints: &[IntrinsicHint::Generic],
        param_count: 1, // future
        return_count: 1,
        // Tier 0 interpreter: poll returns pending (None/false) as async is not supported
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PollPending),
        mlir_op: Some("verum.async.poll"),
        doc: "Poll future synchronously",
    },
    Intrinsic {
        name: "async_sleep_ms",
        category: IntrinsicCategory::Async,
        hints: &[],
        param_count: 1, // ms
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::IoSubmit),
        mlir_op: Some("verum.async.sleep"),
        doc: "Async sleep (milliseconds)",
    },
    Intrinsic {
        name: "async_sleep_ns",
        category: IntrinsicCategory::Async,
        hints: &[],
        param_count: 1, // ns
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::IoSubmit),
        mlir_op: Some("verum.async.sleep"),
        doc: "Async sleep (nanoseconds)",
    },
    // =========================================================================
    // Context/Supervisor Intrinsics (Library-level)
    // =========================================================================
    Intrinsic {
        name: "supervisor_log_escalation",
        category: IntrinsicCategory::Context,
        hints: &[IntrinsicHint::Generic],
        param_count: 1, // reason
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::DebugPrint),
        mlir_op: Some("verum.supervisor.log"),
        doc: "Log supervisor escalation",
    },
    Intrinsic {
        name: "supervisor_set_parent",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 2, // child, parent
        return_count: 0,
        // Tier 0 interpreter: supervisor hierarchy is no-op
        strategy: CodegenStrategy::DirectOpcode(Opcode::Nop),
        mlir_op: Some("verum.supervisor.set_parent"),
        doc: "Set parent supervisor link",
    },
    Intrinsic {
        name: "exec_with_recovery",
        category: IntrinsicCategory::Context,
        hints: &[IntrinsicHint::Generic],
        param_count: 2, // recovery_ctx, future_factory
        return_count: 1,
        // Tier 0 interpreter: execute without recovery, call factory directly
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CallSecondArg),
        mlir_op: Some("verum.recovery.exec"),
        doc: "Execute with recovery strategy",
    },
    Intrinsic {
        name: "inline_cb_as_ref",
        category: IntrinsicCategory::Context,
        hints: &[IntrinsicHint::Generic, IntrinsicHint::Inline],
        param_count: 1, // inline_storage
        return_count: 1, // &CircuitBreaker
        strategy: CodegenStrategy::DirectOpcode(Opcode::Ref),
        mlir_op: Some("llvm.bitcast"),
        doc: "Get circuit breaker reference",
    },
    // =========================================================================
    // Registry Intrinsics (Library-level)
    // =========================================================================
    Intrinsic {
        name: "global_allocator",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet), // or static
        mlir_op: Some("verum.registry.allocator"),
        doc: "Get global allocator",
    },
    Intrinsic {
        name: "global_allocator_intrinsic",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet),
        mlir_op: Some("verum.registry.allocator"),
        doc: "Get global allocator (alias for global_allocator)",
    },
    Intrinsic {
        name: "default_executor",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet),
        mlir_op: Some("verum.registry.executor"),
        doc: "Get default executor",
    },
    Intrinsic {
        name: "default_executor_intrinsic",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet),
        mlir_op: Some("verum.registry.executor"),
        doc: "Get default executor (alias for default_executor)",
    },
    Intrinsic {
        name: "default_io_driver",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet),
        mlir_op: Some("verum.registry.io_driver"),
        doc: "Get default I/O driver",
    },
    Intrinsic {
        name: "default_io_driver_intrinsic",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet),
        mlir_op: Some("verum.registry.io_driver"),
        doc: "Get default I/O driver (alias for default_io_driver)",
    },
    Intrinsic {
        name: "shared_registry_global",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 0,
        return_count: 1,
        // Tier 0 interpreter: use TLS to get registry (same pattern as other registry intrinsics)
        strategy: CodegenStrategy::DirectOpcode(Opcode::TlsGet),
        mlir_op: Some("verum.registry.shared"),
        doc: "Get global shared registry",
    },
    Intrinsic {
        name: "middleware_chain_empty",
        category: IntrinsicCategory::Context,
        hints: &[],
        param_count: 0,
        return_count: 1,
        // Tier 0 interpreter: return unit/empty tuple as empty chain
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::LoadUnit),
        mlir_op: Some("verum.middleware.empty"),
        doc: "Create empty middleware chain",
    },
    // =========================================================================
    // Basic Arithmetic Intrinsics (Int - i64)
    // =========================================================================
    // Generic arithmetic intrinsics - use polymorphic opcodes for type-dispatched operations
    Intrinsic {
        name: "add",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyAdd),
        mlir_op: Some("arith.addi"), // MLIR uses separate ops, but VBC interpreter dispatches at runtime
        doc: "Generic addition - dispatches to int or float based on operand type",
    },
    Intrinsic {
        name: "sub",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolySub),
        mlir_op: Some("arith.subi"),
        doc: "Generic subtraction - dispatches to int or float based on operand type",
    },
    Intrinsic {
        name: "mul",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyMul),
        mlir_op: Some("arith.muli"),
        doc: "Generic multiplication - dispatches to int or float based on operand type",
    },
    Intrinsic {
        name: "div",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyDiv),
        mlir_op: Some("arith.divsi"),
        doc: "Generic division - dispatches to int or float based on operand type",
    },
    Intrinsic {
        name: "rem",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyRem),
        mlir_op: Some("arith.remsi"),
        doc: "Generic remainder - dispatches to int or float based on operand type",
    },
    Intrinsic {
        name: "neg",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyNeg),
        mlir_op: Some("arith.negi"),
        doc: "Generic negation - dispatches to int or float based on operand type",
    },
    Intrinsic {
        name: "abs_signed",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyAbs),
        mlir_op: Some("math.absf"),
        doc: "Generic absolute value for signed types - returns |x|",
    },
    Intrinsic {
        name: "signum",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolySignum),
        mlir_op: Some("math.copysign"),
        doc: "Generic signum - returns -1, 0, or 1 based on sign",
    },
    Intrinsic {
        name: "min",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyMin),
        mlir_op: Some("arith.minsi"),
        doc: "Generic minimum - returns smaller of two values",
    },
    Intrinsic {
        name: "max",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyMax),
        mlir_op: Some("arith.maxsi"),
        doc: "Generic maximum - returns larger of two values",
    },
    Intrinsic {
        name: "clamp",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::PolyClamp),
        mlir_op: None,
        doc: "Generic clamp - returns value clamped to [min, max] range",
    },
    // Generic bitwise intrinsics
    Intrinsic {
        name: "bitand",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Band),
        mlir_op: Some("arith.andi"),
        doc: "Generic bitwise AND",
    },
    Intrinsic {
        name: "bitor",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Bor),
        mlir_op: Some("arith.ori"),
        doc: "Generic bitwise OR",
    },
    Intrinsic {
        name: "bitxor",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Bxor),
        mlir_op: Some("arith.xori"),
        doc: "Generic bitwise XOR",
    },
    Intrinsic {
        name: "bitnot",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Bnot),
        mlir_op: Some("arith.noti"),
        doc: "Generic bitwise NOT",
    },
    Intrinsic {
        name: "shl",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Shl),
        mlir_op: Some("arith.shli"),
        doc: "Generic shift left",
    },
    Intrinsic {
        name: "shr",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Shr),
        mlir_op: Some("arith.shrsi"),
        doc: "Generic arithmetic shift right",
    },
    // Type-specific arithmetic intrinsics
    Intrinsic {
        name: "add_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::AddI),
        mlir_op: Some("arith.addi"),
        doc: "Add two i64 values",
    },
    Intrinsic {
        name: "sub_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::SubI),
        mlir_op: Some("arith.subi"),
        doc: "Subtract two i64 values",
    },
    Intrinsic {
        name: "mul_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::MulI),
        mlir_op: Some("arith.muli"),
        doc: "Multiply two i64 values",
    },
    Intrinsic {
        name: "div_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::MayTrap],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::DivI),
        mlir_op: Some("arith.divsi"),
        doc: "Divide two i64 values (signed)",
    },
    Intrinsic {
        name: "rem_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::MayTrap],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::ModI),
        mlir_op: Some("arith.remsi"),
        doc: "Remainder of i64 division (signed)",
    },
    Intrinsic {
        name: "neg_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::NegI),
        mlir_op: Some("arith.negi"),
        doc: "Negate an i64 value",
    },
    // Bitwise operations (Int - i64)
    Intrinsic {
        name: "and_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::And),
        mlir_op: Some("arith.andi"),
        doc: "Bitwise AND of two i64 values",
    },
    Intrinsic {
        name: "or_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Or),
        mlir_op: Some("arith.ori"),
        doc: "Bitwise OR of two i64 values",
    },
    Intrinsic {
        name: "xor_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Xor),
        mlir_op: Some("arith.xori"),
        doc: "Bitwise XOR of two i64 values",
    },
    Intrinsic {
        name: "not_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Not),
        mlir_op: Some("arith.xori"), // NOT is XOR with -1
        doc: "Bitwise NOT of an i64 value",
    },
    Intrinsic {
        name: "shl_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Shl),
        mlir_op: Some("arith.shli"),
        doc: "Shift left i64 value",
    },
    Intrinsic {
        name: "shr_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::Shr),
        mlir_op: Some("arith.shrsi"),
        doc: "Arithmetic shift right i64 value",
    },
    // =========================================================================
    // Float Arithmetic Intrinsics (Float - f64)
    // =========================================================================
    Intrinsic {
        name: "add_f64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::AddF),
        mlir_op: Some("arith.addf"),
        doc: "Add two f64 values",
    },
    Intrinsic {
        name: "sub_f64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::SubF),
        mlir_op: Some("arith.subf"),
        doc: "Subtract two f64 values",
    },
    Intrinsic {
        name: "mul_f64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::MulF),
        mlir_op: Some("arith.mulf"),
        doc: "Multiply two f64 values",
    },
    Intrinsic {
        name: "div_f64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::DivF),
        mlir_op: Some("arith.divf"),
        doc: "Divide two f64 values",
    },
    Intrinsic {
        name: "rem_f64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::ModF),
        mlir_op: Some("arith.remf"),
        doc: "Remainder of f64 division",
    },
    Intrinsic {
        name: "neg_f64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::NegF),
        mlir_op: Some("arith.negf"),
        doc: "Negate an f64 value",
    },
    // =========================================================================
    // Comparison Intrinsics
    // =========================================================================
    // Generic comparison intrinsics
    Intrinsic {
        name: "eq",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::EqI),
        mlir_op: Some("arith.cmpi eq"),
        doc: "Generic equality comparison",
    },
    Intrinsic {
        name: "ne",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::NeI),
        mlir_op: Some("arith.cmpi ne"),
        doc: "Generic inequality comparison",
    },
    Intrinsic {
        name: "lt",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LtI),
        mlir_op: Some("arith.cmpi slt"),
        doc: "Generic less-than comparison",
    },
    Intrinsic {
        name: "le",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LeI),
        mlir_op: Some("arith.cmpi sle"),
        doc: "Generic less-than-or-equal comparison",
    },
    Intrinsic {
        name: "gt",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GtI),
        mlir_op: Some("arith.cmpi sgt"),
        doc: "Generic greater-than comparison",
    },
    Intrinsic {
        name: "ge",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GeI),
        mlir_op: Some("arith.cmpi sge"),
        doc: "Generic greater-than-or-equal comparison",
    },
    // Type-specific comparison intrinsics
    Intrinsic {
        name: "eq_i64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::EqI),
        mlir_op: Some("arith.cmpi eq"),
        doc: "Compare two i64 for equality",
    },
    Intrinsic {
        name: "ne_i64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::NeI),
        mlir_op: Some("arith.cmpi ne"),
        doc: "Compare two i64 for inequality",
    },
    Intrinsic {
        name: "lt_i64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LtI),
        mlir_op: Some("arith.cmpi slt"),
        doc: "Compare two i64: less than (signed)",
    },
    Intrinsic {
        name: "le_i64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LeI),
        mlir_op: Some("arith.cmpi sle"),
        doc: "Compare two i64: less than or equal (signed)",
    },
    Intrinsic {
        name: "gt_i64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GtI),
        mlir_op: Some("arith.cmpi sgt"),
        doc: "Compare two i64: greater than (signed)",
    },
    Intrinsic {
        name: "ge_i64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GeI),
        mlir_op: Some("arith.cmpi sge"),
        doc: "Compare two i64: greater than or equal (signed)",
    },
    Intrinsic {
        name: "eq_f64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::EqF),
        mlir_op: Some("arith.cmpf oeq"),
        doc: "Compare two f64 for equality",
    },
    Intrinsic {
        name: "ne_f64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::NeF),
        mlir_op: Some("arith.cmpf one"),
        doc: "Compare two f64 for inequality",
    },
    Intrinsic {
        name: "lt_f64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LtF),
        mlir_op: Some("arith.cmpf olt"),
        doc: "Compare two f64: less than",
    },
    Intrinsic {
        name: "le_f64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LeF),
        mlir_op: Some("arith.cmpf ole"),
        doc: "Compare two f64: less than or equal",
    },
    Intrinsic {
        name: "gt_f64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GtF),
        mlir_op: Some("arith.cmpf ogt"),
        doc: "Compare two f64: greater than",
    },
    Intrinsic {
        name: "ge_f64",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GeF),
        mlir_op: Some("arith.cmpf oge"),
        doc: "Compare two f64: greater than or equal",
    },
    Intrinsic {
        name: "eq_bool",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::EqI),
        mlir_op: Some("arith.cmpi eq"),
        doc: "Compare two bools for equality",
    },
    Intrinsic {
        name: "eq_u8",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::EqI),
        mlir_op: Some("arith.cmpi eq"),
        doc: "Compare two bytes (u8) for equality",
    },
    // =========================================================================
    // Type Conversion Intrinsics
    // =========================================================================
    Intrinsic {
        name: "int_to_float",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::CvtIF),
        mlir_op: Some("arith.sitofp"),
        doc: "Convert signed int to float",
    },
    Intrinsic {
        name: "float_to_int",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::CvtFI),
        mlir_op: Some("arith.fptosi"),
        doc: "Convert float to signed int",
    },
    Intrinsic {
        name: "i64_to_i32",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::IntTrunc),
        mlir_op: Some("arith.trunci"),
        doc: "Truncate i64 to i32",
    },
    Intrinsic {
        name: "i32_to_i64",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Sext),
        mlir_op: Some("arith.extsi"),
        doc: "Sign-extend i32 to i64",
    },
    Intrinsic {
        name: "u32_to_u64",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Zext),
        mlir_op: Some("arith.extui"),
        doc: "Zero-extend u32 to u64",
    },
    Intrinsic {
        name: "f32_to_f64",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Fpext),
        mlir_op: Some("arith.extf"),
        doc: "Extend f32 to f64",
    },
    Intrinsic {
        name: "f64_to_f32",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Fptrunc),
        mlir_op: Some("arith.truncf"),
        doc: "Truncate f64 to f32",
    },
    // =========================================================================
    // Checked Arithmetic Intrinsics (Int types)
    // =========================================================================
    // Generic checked_add - the compiler will monomorphize to specific types
    Intrinsic {
        name: "checked_add",
        category: IntrinsicCategory::Overflow,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::ConstEval,
            IntrinsicHint::MultiReturn,
            IntrinsicHint::Generic,
        ],
        param_count: 2,
        return_count: 2, // (result, overflowed)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CheckedAdd),
        mlir_op: Some("arith.addui_extended"),
        doc: "Generic checked addition with overflow detection",
    },
    // Generic checked_sub - the compiler will monomorphize to specific types
    Intrinsic {
        name: "checked_sub",
        category: IntrinsicCategory::Overflow,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::ConstEval,
            IntrinsicHint::MultiReturn,
            IntrinsicHint::Generic,
        ],
        param_count: 2,
        return_count: 2, // (result, overflowed)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CheckedSub),
        mlir_op: Some("arith.subui_extended"),
        doc: "Generic checked subtraction with overflow detection",
    },
    // Generic checked_mul - the compiler will monomorphize to specific types
    Intrinsic {
        name: "checked_mul",
        category: IntrinsicCategory::Overflow,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::ConstEval,
            IntrinsicHint::MultiReturn,
            IntrinsicHint::Generic,
        ],
        param_count: 2,
        return_count: 2, // (result, overflowed)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CheckedMul),
        mlir_op: Some("arith.mului_extended"),
        doc: "Generic checked multiplication with overflow detection",
    },
    // Generic checked_div - the compiler will monomorphize to specific types
    Intrinsic {
        name: "checked_div",
        category: IntrinsicCategory::Overflow,
        hints: &[
            IntrinsicHint::Pure,
            IntrinsicHint::ConstEval,
            IntrinsicHint::MultiReturn,
            IntrinsicHint::Generic,
        ],
        param_count: 2,
        return_count: 2, // (result, is_zero_or_overflow)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CheckedDiv),
        mlir_op: None, // No direct LLVM intrinsic for checked division
        doc: "Generic checked division with zero/overflow detection",
    },
    Intrinsic {
        name: "checked_add_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::MultiReturn],
        param_count: 2,
        return_count: 2, // (result, overflow_flag)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CheckedAdd),
        mlir_op: Some("llvm.intr.sadd.with.overflow"),
        doc: "Checked addition returning (result, overflow)",
    },
    Intrinsic {
        name: "checked_sub_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::MultiReturn],
        param_count: 2,
        return_count: 2,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CheckedSub),
        mlir_op: Some("llvm.intr.ssub.with.overflow"),
        doc: "Checked subtraction returning (result, overflow)",
    },
    Intrinsic {
        name: "checked_mul_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::MultiReturn],
        param_count: 2,
        return_count: 2,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CheckedMul),
        mlir_op: Some("llvm.intr.smul.with.overflow"),
        doc: "Checked multiplication returning (result, overflow)",
    },
    Intrinsic {
        name: "checked_div_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::MultiReturn],
        param_count: 2,
        return_count: 2,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CheckedDiv),
        mlir_op: None, // No direct LLVM intrinsic
        doc: "Checked division returning (result, error)",
    },
    // Checked for unsigned types (u64)
    Intrinsic {
        name: "checked_add_u64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::MultiReturn],
        param_count: 2,
        return_count: 2,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::CheckedAddU),
        mlir_op: Some("llvm.intr.uadd.with.overflow"),
        doc: "Checked unsigned addition returning (result, overflow)",
    },
    Intrinsic {
        name: "checked_sub_u64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::MultiReturn],
        param_count: 2,
        return_count: 2,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::CheckedSubU),
        mlir_op: Some("llvm.intr.usub.with.overflow"),
        doc: "Checked unsigned subtraction returning (result, overflow)",
    },
    Intrinsic {
        name: "checked_mul_u64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::MultiReturn],
        param_count: 2,
        return_count: 2,
        strategy: CodegenStrategy::ArithExtendedOpcode(ArithSubOpcode::CheckedMulU),
        mlir_op: Some("llvm.intr.umul.with.overflow"),
        doc: "Checked unsigned multiplication returning (result, overflow)",
    },
    // =========================================================================
    // Wrapping Arithmetic Intrinsics
    // =========================================================================
    Intrinsic {
        name: "wrapping_add_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::AddI),
        mlir_op: Some("arith.addi"),
        doc: "Wrapping addition (modular)",
    },
    Intrinsic {
        name: "wrapping_sub_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::SubI),
        mlir_op: Some("arith.subi"),
        doc: "Wrapping subtraction (modular)",
    },
    Intrinsic {
        name: "wrapping_mul_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::MulI),
        mlir_op: Some("arith.muli"),
        doc: "Wrapping multiplication (modular)",
    },
    // =========================================================================
    // Saturating Arithmetic Intrinsics (subtraction only - add defined in Saturating category)
    // =========================================================================
    Intrinsic {
        name: "saturating_sub_i64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SaturatingSub),
        mlir_op: Some("llvm.intr.ssub.sat"),
        doc: "Saturating signed subtraction",
    },
    Intrinsic {
        name: "saturating_sub_u64",
        category: IntrinsicCategory::Arithmetic,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SaturatingSub),
        mlir_op: Some("llvm.intr.usub.sat"),
        doc: "Saturating unsigned subtraction",
    },
    // =========================================================================
    // Bit Manipulation Intrinsics (signed variants)
    // =========================================================================
    Intrinsic {
        name: "clz_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Clz),
        mlir_op: Some("llvm.intr.ctlz"),
        doc: "Count leading zeros in i64",
    },
    Intrinsic {
        name: "ctz_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Ctz),
        mlir_op: Some("llvm.intr.cttz"),
        doc: "Count trailing zeros in i64",
    },
    Intrinsic {
        name: "popcount_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Popcnt),
        mlir_op: Some("llvm.intr.ctpop"),
        doc: "Count set bits (population count) in i64",
    },
    Intrinsic {
        name: "bswap_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Bswap),
        mlir_op: Some("llvm.intr.bswap"),
        doc: "Byte swap i64",
    },
    Intrinsic {
        name: "bitreverse_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Bitreverse),
        mlir_op: Some("llvm.intr.bitreverse"),
        doc: "Reverse bits in i64",
    },
    Intrinsic {
        name: "rotl_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RotateLeft),
        mlir_op: Some("llvm.intr.fshl"),
        doc: "Rotate left i64",
    },
    Intrinsic {
        name: "rotr_i64",
        category: IntrinsicCategory::BitManip,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RotateRight),
        mlir_op: Some("llvm.intr.fshr"),
        doc: "Rotate right i64",
    },
    // =========================================================================
    // Float Math Intrinsics (f64)
    // =========================================================================
    // =========================================================================
    // Float Math Intrinsics (f64) - Using MathExtended opcode for zero-cost dispatch
    // MathExtended (0x29) + MathSubOpcode provides ~2ns dispatch via computed goto
    // =========================================================================
    Intrinsic {
        name: "sqrt_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::SqrtF64),
        mlir_op: Some("math.sqrt"),
        doc: "Square root of f64",
    },
    Intrinsic {
        name: "trunc_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::TruncF64),
        mlir_op: Some("math.trunc"),
        doc: "Truncate f64 towards zero",
    },
    // Trigonometric functions
    Intrinsic {
        name: "sin_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::SinF64),
        mlir_op: Some("math.sin"),
        doc: "Sine of f64 (radians)",
    },
    Intrinsic {
        name: "cos_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CosF64),
        mlir_op: Some("math.cos"),
        doc: "Cosine of f64 (radians)",
    },
    Intrinsic {
        name: "tan_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::TanF64),
        mlir_op: Some("math.tan"),
        doc: "Tangent of f64 (radians)",
    },
    Intrinsic {
        name: "asin_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AsinF64),
        mlir_op: Some("math.asin"),
        doc: "Arcsine of f64",
    },
    Intrinsic {
        name: "acos_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AcosF64),
        mlir_op: Some("math.acos"),
        doc: "Arccosine of f64",
    },
    Intrinsic {
        name: "atan_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AtanF64),
        mlir_op: Some("math.atan"),
        doc: "Arctangent of f64",
    },
    Intrinsic {
        name: "atan2_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Atan2F64),
        mlir_op: Some("math.atan2"),
        doc: "Two-argument arctangent of f64",
    },
    // Hyperbolic functions
    Intrinsic {
        name: "sinh_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::SinhF64),
        mlir_op: Some("math.sinh"),
        doc: "Hyperbolic sine of f64",
    },
    Intrinsic {
        name: "cosh_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CoshF64),
        mlir_op: Some("math.cosh"),
        doc: "Hyperbolic cosine of f64",
    },
    Intrinsic {
        name: "tanh_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::TanhF64),
        mlir_op: Some("math.tanh"),
        doc: "Hyperbolic tangent of f64",
    },
    Intrinsic {
        name: "asinh_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AsinhF64),
        mlir_op: Some("math.asinh"),
        doc: "Inverse hyperbolic sine of f64",
    },
    Intrinsic {
        name: "acosh_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AcoshF64),
        mlir_op: Some("math.acosh"),
        doc: "Inverse hyperbolic cosine of f64",
    },
    Intrinsic {
        name: "atanh_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AtanhF64),
        mlir_op: Some("math.atanh"),
        doc: "Inverse hyperbolic tangent of f64",
    },
    // Exponential and logarithmic functions
    Intrinsic {
        name: "exp_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::ExpF64),
        mlir_op: Some("math.exp"),
        doc: "Exponential e^x of f64",
    },
    Intrinsic {
        name: "exp2_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Exp2F64),
        mlir_op: Some("math.exp2"),
        doc: "Base-2 exponential 2^x of f64",
    },
    Intrinsic {
        name: "expm1_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Expm1F64),
        mlir_op: Some("math.expm1"),
        doc: "Exponential e^x - 1 of f64 (accurate for small x)",
    },
    Intrinsic {
        name: "log_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::LogF64),
        mlir_op: Some("math.log"),
        doc: "Natural logarithm of f64",
    },
    Intrinsic {
        name: "log2_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Log2F64),
        mlir_op: Some("math.log2"),
        doc: "Base-2 logarithm of f64",
    },
    Intrinsic {
        name: "log10_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Log10F64),
        mlir_op: Some("math.log10"),
        doc: "Base-10 logarithm of f64",
    },
    Intrinsic {
        name: "log1p_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Log1pF64),
        mlir_op: Some("math.log1p"),
        doc: "Natural log of (1 + x) for f64 (accurate for small x)",
    },
    // Power and special functions
    Intrinsic {
        name: "powi_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::PowiF64),
        mlir_op: Some("llvm.intr.powi"),
        doc: "Power x^n where n is integer",
    },
    Intrinsic {
        name: "cbrt_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CbrtF64),
        mlir_op: Some("math.cbrt"),
        doc: "Cube root of f64",
    },
    Intrinsic {
        name: "hypot_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::HypotF64),
        mlir_op: Some("math.hypot"),
        doc: "Hypotenuse sqrt(x² + y²) of f64",
    },
    Intrinsic {
        name: "fma_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::FmaF64),
        mlir_op: Some("math.fma"),
        doc: "Fused multiply-add (a * b + c) of f64",
    },
    Intrinsic {
        name: "copysign_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CopysignF64),
        mlir_op: Some("math.copysign"),
        doc: "Copy sign from y to x for f64",
    },
    Intrinsic {
        name: "min_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::MinnumF64),
        mlir_op: Some("arith.minnumf"),
        doc: "Minimum of two f64 (NaN-propagating)",
    },
    Intrinsic {
        name: "minnum_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::MinnumF64),
        mlir_op: Some("arith.minnumf"),
        doc: "Alias for min_f64 - NaN-propagating minimum",
    },
    Intrinsic {
        name: "max_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::MaxnumF64),
        mlir_op: Some("arith.maxnumf"),
        doc: "Maximum of two f64 (NaN-propagating)",
    },
    Intrinsic {
        name: "maxnum_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::MaxnumF64),
        mlir_op: Some("arith.maxnumf"),
        doc: "Alias for max_f64 - NaN-propagating maximum",
    },
    // F64 additional functions
    Intrinsic {
        name: "fmod_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::FmodF64),
        mlir_op: Some("llvm.fmod.f64"),
        doc: "Floating-point remainder (IEEE 754)",
    },
    Intrinsic {
        name: "remainder_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::RemainderF64),
        mlir_op: Some("llvm.remainder.f64"),
        doc: "IEEE 754 remainder operation",
    },
    Intrinsic {
        name: "fdim_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::FdimF64),
        mlir_op: Some("llvm.fdim.f64"),
        doc: "Positive difference: max(x-y, 0)",
    },
    // Float bit manipulation
    Intrinsic {
        name: "f64_to_bits",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::F64ToBits),
        mlir_op: Some("arith.bitcast"),
        doc: "Reinterpret f64 bits as u64",
    },
    Intrinsic {
        name: "f64_from_bits",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::F64FromBits),
        mlir_op: Some("arith.bitcast"),
        doc: "Reinterpret u64 bits as f64",
    },
    // Float special values
    Intrinsic {
        name: "f64_infinity",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("arith.constant"),
        doc: "Positive infinity for f64",
    },
    Intrinsic {
        name: "f64_neg_infinity",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("arith.constant"),
        doc: "Negative infinity for f64",
    },
    Intrinsic {
        name: "f64_nan",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("arith.constant"),
        doc: "NaN (Not a Number) for f64",
    },
    Intrinsic {
        name: "is_nan_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::IsNan),
        mlir_op: Some("arith.cmpf uno"),
        doc: "Check if f64 is NaN",
    },
    Intrinsic {
        name: "is_infinite_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::IsInf),
        mlir_op: Some("arith.cmpf ord"),
        doc: "Check if f64 is infinite",
    },
    Intrinsic {
        name: "is_finite_f64",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::IsFinite),
        mlir_op: None,
        doc: "Check if f64 is finite (not NaN or infinity)",
    },
    // =========================================================================
    // Byte Conversion Intrinsics
    // =========================================================================
    // Generic byte conversion intrinsics (default to 8 bytes = 64-bit)
    Intrinsic {
        name: "to_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 8),
        mlir_op: None,
        doc: "Convert value to little-endian bytes (generic)",
    },
    Intrinsic {
        name: "from_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromLeBytes, 8),
        mlir_op: None,
        doc: "Convert little-endian bytes to value (generic)",
    },
    Intrinsic {
        name: "to_be_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToBeBytes, 8),
        mlir_op: None,
        doc: "Convert value to big-endian bytes (generic)",
    },
    Intrinsic {
        name: "from_be_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromBeBytes, 8),
        mlir_op: None,
        doc: "Convert big-endian bytes to value (generic)",
    },
    // Generic byte conversion (native endian = little-endian on x86_64/aarch64)
    Intrinsic {
        name: "int_to_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 8),
        mlir_op: None,
        doc: "Convert Int to native-endian bytes (alias for int_to_le_bytes)",
    },
    // Type-specific byte conversion intrinsics (8 bytes = 64-bit Int)
    Intrinsic {
        name: "int_to_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 8),
        mlir_op: None,
        doc: "Convert Int to little-endian bytes",
    },
    Intrinsic {
        name: "int_from_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromLeBytes, 8),
        mlir_op: None,
        doc: "Convert little-endian bytes to Int",
    },
    Intrinsic {
        name: "int_to_be_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToBeBytes, 8),
        mlir_op: None,
        doc: "Convert Int to big-endian bytes",
    },
    Intrinsic {
        name: "int_from_be_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromBeBytes, 8),
        mlir_op: None,
        doc: "Convert big-endian bytes to Int",
    },
    Intrinsic {
        name: "u64_to_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 8),
        mlir_op: None,
        doc: "Convert u64 to little-endian bytes",
    },
    Intrinsic {
        name: "u64_from_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromLeBytes, 8),
        mlir_op: None,
        doc: "Convert little-endian bytes to u64",
    },
    Intrinsic {
        name: "i32_to_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 4),
        mlir_op: None,
        doc: "Convert i32 to little-endian bytes",
    },
    Intrinsic {
        name: "i32_from_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromLeBytes, 4),
        mlir_op: None,
        doc: "Convert little-endian bytes to i32",
    },
    Intrinsic {
        name: "u32_to_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 4),
        mlir_op: None,
        doc: "Convert u32 to little-endian bytes",
    },
    Intrinsic {
        name: "u32_from_le_bytes",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromLeBytes, 4),
        mlir_op: None,
        doc: "Convert little-endian bytes to u32",
    },
    // Width-specific byte conversion intrinsics (named by width suffix)
    Intrinsic {
        name: "to_le_bytes_2",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 2),
        mlir_op: None,
        doc: "Convert value to 2 little-endian bytes",
    },
    Intrinsic {
        name: "to_le_bytes_4",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 4),
        mlir_op: None,
        doc: "Convert value to 4 little-endian bytes",
    },
    Intrinsic {
        name: "to_le_bytes_8",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToLeBytes, 8),
        mlir_op: None,
        doc: "Convert value to 8 little-endian bytes",
    },
    Intrinsic {
        name: "to_be_bytes_2",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToBeBytes, 2),
        mlir_op: None,
        doc: "Convert value to 2 big-endian bytes",
    },
    Intrinsic {
        name: "to_be_bytes_4",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToBeBytes, 4),
        mlir_op: None,
        doc: "Convert value to 4 big-endian bytes",
    },
    Intrinsic {
        name: "to_be_bytes_8",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::ToBeBytes, 8),
        mlir_op: None,
        doc: "Convert value to 8 big-endian bytes",
    },
    Intrinsic {
        name: "from_le_bytes_2",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromLeBytes, 2),
        mlir_op: None,
        doc: "Convert 2 little-endian bytes to value",
    },
    Intrinsic {
        name: "from_le_bytes_4",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromLeBytes, 4),
        mlir_op: None,
        doc: "Convert 4 little-endian bytes to value",
    },
    Intrinsic {
        name: "from_le_bytes_8",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromLeBytes, 8),
        mlir_op: None,
        doc: "Convert 8 little-endian bytes to value",
    },
    Intrinsic {
        name: "from_be_bytes_2",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromBeBytes, 2),
        mlir_op: None,
        doc: "Convert 2 big-endian bytes to value",
    },
    Intrinsic {
        name: "from_be_bytes_4",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromBeBytes, 4),
        mlir_op: None,
        doc: "Convert 4 big-endian bytes to value",
    },
    Intrinsic {
        name: "from_be_bytes_8",
        category: IntrinsicCategory::ByteConversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequenceWithWidth(InlineSequenceId::FromBeBytes, 8),
        mlir_op: None,
        doc: "Convert 8 big-endian bytes to value",
    },
    // =========================================================================
    // Char Intrinsics
    // =========================================================================
    Intrinsic {
        name: "char_is_alphabetic",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharIsAlphabetic),
        mlir_op: None,
        doc: "Check if char is alphabetic",
    },
    Intrinsic {
        name: "char_is_numeric",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharIsNumeric),
        mlir_op: None,
        doc: "Check if char is numeric (digit)",
    },
    Intrinsic {
        name: "char_is_whitespace",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharIsWhitespace),
        mlir_op: None,
        doc: "Check if char is whitespace",
    },
    Intrinsic {
        name: "char_is_control",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharIsControl),
        mlir_op: None,
        doc: "Check if char is control character",
    },
    Intrinsic {
        name: "char_is_uppercase",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharIsUppercase),
        mlir_op: None,
        doc: "Check if char is uppercase",
    },
    Intrinsic {
        name: "char_is_lowercase",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharIsLowercase),
        mlir_op: None,
        doc: "Check if char is lowercase",
    },
    Intrinsic {
        name: "char_to_uppercase",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharToUppercase),
        mlir_op: None,
        doc: "Convert char to uppercase",
    },
    Intrinsic {
        name: "char_to_lowercase",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharToLowercase),
        mlir_op: None,
        doc: "Convert char to lowercase",
    },
    Intrinsic {
        name: "char_encode_utf8",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2, // char, buffer
        return_count: 1, // bytes written
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharEncodeUtf8),
        mlir_op: None,
        doc: "Encode char as UTF-8 bytes",
    },
    Intrinsic {
        name: "char_escape_debug",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharEscapeDebug),
        mlir_op: None,
        doc: "Escape char for debug output",
    },
    Intrinsic {
        name: "char_general_category",
        category: IntrinsicCategory::Char,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1, // char
        return_count: 1, // category enum value
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharGeneralCategory),
        mlir_op: None,
        doc: "Get Unicode general category of character",
    },
    // =========================================================================
    // Float32 Intrinsics
    // =========================================================================
    Intrinsic {
        name: "add_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::AddF),
        mlir_op: Some("arith.addf"),
        doc: "Add two f32 values",
    },
    Intrinsic {
        name: "sub_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::SubF),
        mlir_op: Some("arith.subf"),
        doc: "Subtract two f32 values",
    },
    Intrinsic {
        name: "mul_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::MulF),
        mlir_op: Some("arith.mulf"),
        doc: "Multiply two f32 values",
    },
    Intrinsic {
        name: "div_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::DivF),
        mlir_op: Some("arith.divf"),
        doc: "Divide two f32 values",
    },
    Intrinsic {
        name: "neg_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::NegF),
        mlir_op: Some("arith.negf"),
        doc: "Negate an f32 value",
    },
    Intrinsic {
        name: "sqrt_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::SqrtF32),
        mlir_op: Some("math.sqrt"),
        doc: "Square root of f32",
    },
    Intrinsic {
        name: "abs_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AbsF32),
        mlir_op: Some("math.absf"),
        doc: "Absolute value of f32",
    },
    Intrinsic {
        name: "floor_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::FloorF32),
        mlir_op: Some("math.floor"),
        doc: "Floor of f32",
    },
    Intrinsic {
        name: "ceil_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CeilF32),
        mlir_op: Some("math.ceil"),
        doc: "Ceiling of f32",
    },
    Intrinsic {
        name: "round_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::RoundF32),
        mlir_op: Some("math.round"),
        doc: "Round f32 to nearest integer",
    },
    Intrinsic {
        name: "trunc_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::TruncF32),
        mlir_op: Some("math.trunc"),
        doc: "Truncate f32 towards zero",
    },
    // ==========================================================================
    // F32 Trigonometric Functions
    // Uses LLVM intrinsics (llvm.sin.f32, etc.) - no libc dependency
    // ==========================================================================
    Intrinsic {
        name: "sin_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::SinF32),
        mlir_op: Some("llvm.sin.f32"),
        doc: "Sine of f32 (radians)",
    },
    Intrinsic {
        name: "cos_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CosF32),
        mlir_op: Some("llvm.cos.f32"),
        doc: "Cosine of f32 (radians)",
    },
    Intrinsic {
        name: "tan_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::TanF32),
        mlir_op: Some("llvm.tan.f32"),
        doc: "Tangent of f32 (radians)",
    },
    Intrinsic {
        name: "asin_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AsinF32),
        mlir_op: Some("llvm.asin.f32"),
        doc: "Arcsine of f32",
    },
    Intrinsic {
        name: "acos_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AcosF32),
        mlir_op: Some("llvm.acos.f32"),
        doc: "Arccosine of f32",
    },
    Intrinsic {
        name: "atan_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AtanF32),
        mlir_op: Some("llvm.atan.f32"),
        doc: "Arctangent of f32",
    },
    Intrinsic {
        name: "atan2_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Atan2F32),
        mlir_op: Some("llvm.atan2.f32"),
        doc: "Two-argument arctangent of f32",
    },
    // ==========================================================================
    // F32 Hyperbolic Functions
    // ==========================================================================
    Intrinsic {
        name: "sinh_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::SinhF32),
        mlir_op: Some("llvm.sinh.f32"),
        doc: "Hyperbolic sine of f32",
    },
    Intrinsic {
        name: "cosh_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CoshF32),
        mlir_op: Some("llvm.cosh.f32"),
        doc: "Hyperbolic cosine of f32",
    },
    Intrinsic {
        name: "tanh_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::TanhF32),
        mlir_op: Some("llvm.tanh.f32"),
        doc: "Hyperbolic tangent of f32",
    },
    Intrinsic {
        name: "asinh_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AsinhF32),
        mlir_op: Some("llvm.asinh.f32"),
        doc: "Inverse hyperbolic sine of f32",
    },
    Intrinsic {
        name: "acosh_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AcoshF32),
        mlir_op: Some("llvm.acosh.f32"),
        doc: "Inverse hyperbolic cosine of f32",
    },
    Intrinsic {
        name: "atanh_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::AtanhF32),
        mlir_op: Some("llvm.atanh.f32"),
        doc: "Inverse hyperbolic tangent of f32",
    },
    // ==========================================================================
    // F32 Exponential and Logarithmic Functions
    // ==========================================================================
    Intrinsic {
        name: "exp_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::ExpF32),
        mlir_op: Some("llvm.exp.f32"),
        doc: "Exponential e^x of f32",
    },
    Intrinsic {
        name: "exp2_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Exp2F32),
        mlir_op: Some("llvm.exp2.f32"),
        doc: "Base-2 exponential 2^x of f32",
    },
    Intrinsic {
        name: "expm1_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Expm1F32),
        mlir_op: Some("llvm.expm1.f32"),
        doc: "Exponential e^x - 1 of f32 (accurate for small x)",
    },
    Intrinsic {
        name: "log_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::LogF32),
        mlir_op: Some("llvm.log.f32"),
        doc: "Natural logarithm of f32",
    },
    Intrinsic {
        name: "log2_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Log2F32),
        mlir_op: Some("llvm.log2.f32"),
        doc: "Base-2 logarithm of f32",
    },
    Intrinsic {
        name: "log10_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Log10F32),
        mlir_op: Some("llvm.log10.f32"),
        doc: "Base-10 logarithm of f32",
    },
    Intrinsic {
        name: "log1p_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::Log1pF32),
        mlir_op: Some("llvm.log1p.f32"),
        doc: "ln(1 + x) of f32 (accurate for small x)",
    },
    // ==========================================================================
    // F32 Power and Special Functions
    // ==========================================================================
    Intrinsic {
        name: "cbrt_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CbrtF32),
        mlir_op: Some("llvm.cbrt.f32"),
        doc: "Cube root of f32",
    },
    Intrinsic {
        name: "hypot_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::HypotF32),
        mlir_op: Some("llvm.hypot.f32"),
        doc: "Hypotenuse sqrt(x^2 + y^2) of f32",
    },
    Intrinsic {
        name: "fma_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::FmaF32),
        mlir_op: Some("llvm.fma.f32"),
        doc: "Fused multiply-add (a * b + c) of f32",
    },
    Intrinsic {
        name: "copysign_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::CopysignF32),
        mlir_op: Some("llvm.copysign.f32"),
        doc: "Copy sign from y to x for f32",
    },
    Intrinsic {
        name: "powi_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::PowiF32),
        mlir_op: Some("llvm.powi.f32.i32"),
        doc: "Power x^n with integer exponent for f32",
    },
    Intrinsic {
        name: "minnum_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::MinnumF32),
        mlir_op: Some("llvm.minnum.f32"),
        doc: "NaN-propagating minimum of two f32",
    },
    Intrinsic {
        name: "maxnum_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::MaxnumF32),
        mlir_op: Some("llvm.maxnum.f32"),
        doc: "NaN-propagating maximum of two f32",
    },
    // F32 additional functions
    Intrinsic {
        name: "fmod_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::FmodF32),
        mlir_op: Some("llvm.fmod.f32"),
        doc: "Floating-point remainder (IEEE 754) for f32",
    },
    Intrinsic {
        name: "remainder_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::RemainderF32),
        mlir_op: Some("llvm.remainder.f32"),
        doc: "IEEE 754 remainder operation for f32",
    },
    Intrinsic {
        name: "fdim_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::MathExtendedOpcode(MathSubOpcode::FdimF32),
        mlir_op: Some("llvm.fdim.f32"),
        doc: "Positive difference: max(x-y, 0) for f32",
    },
    Intrinsic {
        name: "f32_to_bits",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::F32ToBits),
        mlir_op: Some("arith.bitcast"),
        doc: "Reinterpret f32 bits as u32",
    },
    Intrinsic {
        name: "f32_from_bits",
        category: IntrinsicCategory::Conversion,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::F32FromBits),
        mlir_op: Some("arith.bitcast"),
        doc: "Reinterpret u32 bits as f32",
    },
    Intrinsic {
        name: "f32_infinity",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("arith.constant"),
        doc: "Positive infinity for f32",
    },
    Intrinsic {
        name: "f32_neg_infinity",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("arith.constant"),
        doc: "Negative infinity for f32",
    },
    Intrinsic {
        name: "f32_nan",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::CompileTimeConstant,
        mlir_op: Some("arith.constant"),
        doc: "NaN (Not a Number) for f32",
    },
    Intrinsic {
        name: "is_nan_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::IsNan),
        mlir_op: Some("arith.cmpf uno"),
        doc: "Check if f32 is NaN",
    },
    Intrinsic {
        name: "is_infinite_f32",
        category: IntrinsicCategory::Float32,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::IsInf),
        mlir_op: Some("arith.cmpf ord"),
        doc: "Check if f32 is infinite",
    },
    Intrinsic {
        name: "eq_f32",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::EqF),
        mlir_op: Some("arith.cmpf oeq"),
        doc: "Compare two f32 for equality",
    },
    Intrinsic {
        name: "lt_f32",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LtF),
        mlir_op: Some("arith.cmpf olt"),
        doc: "Compare two f32: less than",
    },
    Intrinsic {
        name: "le_f32",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::LeF),
        mlir_op: Some("arith.cmpf ole"),
        doc: "Compare two f32: less than or equal",
    },
    Intrinsic {
        name: "gt_f32",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GtF),
        mlir_op: Some("arith.cmpf ogt"),
        doc: "Compare two f32: greater than",
    },
    Intrinsic {
        name: "ge_f32",
        category: IntrinsicCategory::Comparison,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GeF),
        mlir_op: Some("arith.cmpf oge"),
        doc: "Compare two f32: greater than or equal",
    },
    // =========================================================================
    // SIMD Vector Intrinsics
    // =========================================================================
    // --- Vector Creation ---
    Intrinsic {
        name: "simd_splat",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1, // scalar value
        return_count: 1, // vector with all lanes = value
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdSplat),
        mlir_op: Some("vector.splat"),
        doc: "Broadcast scalar to all vector lanes",
    },
    Intrinsic {
        name: "simd_extract",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // vector, lane index
        return_count: 1, // scalar value
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdExtract),
        mlir_op: Some("vector.extractelement"),
        doc: "Extract scalar from vector lane",
    },
    Intrinsic {
        name: "simd_insert",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3, // vector, lane index, value
        return_count: 1, // new vector
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdInsert),
        mlir_op: Some("vector.insertelement"),
        doc: "Insert scalar into vector lane",
    },
    // --- Arithmetic Operations ---
    Intrinsic {
        name: "simd_add",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdAdd),
        mlir_op: Some("arith.addf"),
        doc: "Element-wise vector addition",
    },
    Intrinsic {
        name: "simd_sub",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdSub),
        mlir_op: Some("arith.subf"),
        doc: "Element-wise vector subtraction",
    },
    Intrinsic {
        name: "simd_mul",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMul),
        mlir_op: Some("arith.mulf"),
        doc: "Element-wise vector multiplication",
    },
    Intrinsic {
        name: "simd_div",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdDiv),
        mlir_op: Some("arith.divf"),
        doc: "Element-wise vector division",
    },
    Intrinsic {
        name: "simd_neg",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdNeg),
        mlir_op: Some("arith.negf"),
        doc: "Element-wise vector negation",
    },
    Intrinsic {
        name: "simd_abs",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdAbs),
        mlir_op: Some("math.absf"),
        doc: "Element-wise vector absolute value",
    },
    Intrinsic {
        name: "simd_sqrt",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdSqrt),
        mlir_op: Some("math.sqrt"),
        doc: "Element-wise vector square root",
    },
    Intrinsic {
        name: "simd_fma",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3, // a, b, c -> a * b + c
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdFma),
        mlir_op: Some("math.fma"),
        doc: "Fused multiply-add: a * b + c (single rounding)",
    },
    Intrinsic {
        name: "simd_min",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMin),
        mlir_op: Some("arith.minnumf"),
        doc: "Element-wise vector minimum",
    },
    Intrinsic {
        name: "simd_max",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMax),
        mlir_op: Some("arith.maxnumf"),
        doc: "Element-wise vector maximum",
    },
    // --- Reductions ---
    Intrinsic {
        name: "simd_reduce_add",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1, // scalar
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdReduceAdd),
        mlir_op: Some("vector.reduction <add>"),
        doc: "Horizontal sum of all vector lanes",
    },
    Intrinsic {
        name: "simd_reduce_mul",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdReduceMul),
        mlir_op: Some("vector.reduction <mul>"),
        doc: "Horizontal product of all vector lanes",
    },
    Intrinsic {
        name: "simd_reduce_min",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdReduceMin),
        mlir_op: Some("vector.reduction <minnumf>"),
        doc: "Horizontal minimum of all vector lanes",
    },
    Intrinsic {
        name: "simd_reduce_max",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdReduceMax),
        mlir_op: Some("vector.reduction <maxnumf>"),
        doc: "Horizontal maximum of all vector lanes",
    },
    // --- Comparisons (return masks) ---
    Intrinsic {
        name: "simd_cmp_eq",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1, // mask
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdCmpEq),
        mlir_op: Some("arith.cmpf oeq"),
        doc: "Element-wise equality comparison, returns mask",
    },
    Intrinsic {
        name: "simd_cmp_ne",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdCmpNe),
        mlir_op: Some("arith.cmpf one"),
        doc: "Element-wise not-equal comparison, returns mask",
    },
    Intrinsic {
        name: "simd_cmp_lt",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdCmpLt),
        mlir_op: Some("arith.cmpf olt"),
        doc: "Element-wise less-than comparison, returns mask",
    },
    Intrinsic {
        name: "simd_cmp_le",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdCmpLe),
        mlir_op: Some("arith.cmpf ole"),
        doc: "Element-wise less-or-equal comparison, returns mask",
    },
    Intrinsic {
        name: "simd_cmp_gt",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdCmpGt),
        mlir_op: Some("arith.cmpf ogt"),
        doc: "Element-wise greater-than comparison, returns mask",
    },
    Intrinsic {
        name: "simd_cmp_ge",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdCmpGe),
        mlir_op: Some("arith.cmpf oge"),
        doc: "Element-wise greater-or-equal comparison, returns mask",
    },
    Intrinsic {
        name: "simd_select",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3, // mask, if_true, if_false
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdSelect),
        mlir_op: Some("arith.select"),
        doc: "Conditional select: mask ? if_true : if_false",
    },
    // --- Memory Operations ---
    Intrinsic {
        name: "simd_load_aligned",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1, // ptr
        return_count: 1, // vector
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdLoadAligned),
        mlir_op: Some("vector.load"),
        doc: "Load vector from aligned memory",
    },
    Intrinsic {
        name: "simd_load_unaligned",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdLoadUnaligned),
        mlir_op: Some("vector.load unaligned"),
        doc: "Load vector from unaligned memory",
    },
    Intrinsic {
        name: "simd_store_aligned",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // ptr, vector
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdStoreAligned),
        mlir_op: Some("vector.store"),
        doc: "Store vector to aligned memory",
    },
    Intrinsic {
        name: "simd_store_unaligned",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdStoreUnaligned),
        mlir_op: Some("vector.store unaligned"),
        doc: "Store vector to unaligned memory",
    },
    Intrinsic {
        name: "simd_masked_load",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // ptr, mask
        return_count: 1, // vector (inactive lanes become zero)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMaskedLoad),
        mlir_op: Some("vector.maskedload"),
        doc: "Load vector with mask (inactive lanes become zero)",
    },
    Intrinsic {
        name: "simd_masked_store",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3, // ptr, mask, vector
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMaskedStore),
        mlir_op: Some("vector.maskedstore"),
        doc: "Store vector with mask (inactive lanes skipped)",
    },
    // --- Shuffles and Permutes ---
    Intrinsic {
        name: "simd_shuffle",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3, // vec1, vec2, indices (compile-time constant)
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdShuffle),
        mlir_op: Some("vector.shuffle"),
        doc: "Shuffle elements from two vectors using constant indices",
    },
    Intrinsic {
        name: "simd_gather",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // base_ptr, index_vector
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdGather),
        mlir_op: Some("vector.gather"),
        doc: "Gather: indexed load from memory",
    },
    Intrinsic {
        name: "simd_scatter",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::MemoryEffect, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 3, // base_ptr, index_vector, value_vector
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdScatter),
        mlir_op: Some("vector.scatter"),
        doc: "Scatter: indexed store to memory",
    },
    // --- Mask Operations ---
    Intrinsic {
        name: "simd_mask_all",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 0,
        return_count: 1, // all-true mask
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMaskAll),
        mlir_op: Some("vector.constant_mask"),
        doc: "Create all-true mask (all lanes active)",
    },
    Intrinsic {
        name: "simd_mask_none",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 0,
        return_count: 1, // all-false mask
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMaskNone),
        mlir_op: Some("vector.constant_mask"),
        doc: "Create all-false mask (no lanes active)",
    },
    Intrinsic {
        name: "simd_mask_count",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1, // mask
        return_count: 1, // count of active lanes
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMaskCount),
        mlir_op: Some("llvm.intr.ctpop"),
        doc: "Count number of active lanes in mask",
    },
    Intrinsic {
        name: "simd_mask_any",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1, // bool
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdMaskAny),
        mlir_op: Some("vector.reduction <or>"),
        doc: "Check if any mask lane is active",
    },
    // --- Bitwise Operations ---
    Intrinsic {
        name: "simd_bitwise_and",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdBitwiseAnd),
        mlir_op: Some("arith.andi"),
        doc: "Element-wise bitwise AND",
    },
    Intrinsic {
        name: "simd_bitwise_or",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdBitwiseOr),
        mlir_op: Some("arith.ori"),
        doc: "Element-wise bitwise OR",
    },
    Intrinsic {
        name: "simd_bitwise_xor",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdBitwiseXor),
        mlir_op: Some("arith.xori"),
        doc: "Element-wise bitwise XOR",
    },
    Intrinsic {
        name: "simd_bitwise_not",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdBitwiseNot),
        mlir_op: None, // XOR with all-ones
        doc: "Element-wise bitwise NOT",
    },
    Intrinsic {
        name: "simd_shift_left",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2, // vector, shift_amount
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdShiftLeft),
        mlir_op: Some("arith.shli"),
        doc: "Element-wise left shift",
    },
    Intrinsic {
        name: "simd_shift_right",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdShiftRight),
        mlir_op: Some("arith.shrsi"), // arithmetic shift for signed
        doc: "Element-wise right shift (arithmetic for signed)",
    },
    Intrinsic {
        name: "simd_cast",
        category: IntrinsicCategory::Simd,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline, IntrinsicHint::Generic],
        param_count: 1, // source vector
        return_count: 1, // target vector (different element type)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SimdCast),
        mlir_op: Some("arith.sitofp"), // or other casts based on types
        doc: "Element-wise type conversion",
    },
    // =========================================================================
    // Basic Tensor Operations (Direct Opcodes)
    // =========================================================================
    // Tensor Creation
    Intrinsic {
        name: "TENSOR_NEW",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // shape, dtype
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorNew),
        mlir_op: Some("verum.tensor_new"),
        doc: "Create uninitialized tensor with shape and dtype",
    },
    Intrinsic {
        name: "TENSOR_FILL",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // shape, value, dtype
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorFull),
        mlir_op: Some("verum.tensor_fill"),
        doc: "Create tensor filled with a scalar value",
    },
    Intrinsic {
        name: "TENSOR_FROM_SLICE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // data, shape, dtype
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorFromSlice),
        mlir_op: Some("verum.tensor_from_slice"),
        doc: "Create tensor from slice data",
    },
    Intrinsic {
        name: "TENSOR_FROM_DATA",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // data, shape, device
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorFromSlice),
        mlir_op: Some("verum.tensor_from_data"),
        doc: "Create tensor from data array",
    },
    Intrinsic {
        name: "TENSOR_ARANGE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 4, // start, end, step, device
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Arange),
        mlir_op: Some("verum.tensor_arange"),
        doc: "Create tensor with evenly spaced values",
    },
    Intrinsic {
        name: "TENSOR_LINSPACE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 4, // start, end, num, device
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Linspace),
        mlir_op: Some("verum.tensor_linspace"),
        doc: "Create tensor with linearly spaced values",
    },
    Intrinsic {
        name: "TENSOR_RAND",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // shape, key, device
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Rand),
        mlir_op: Some("verum.tensor_rand"),
        doc: "Create tensor with uniform random values in [0, 1)",
    },
    Intrinsic {
        name: "TENSOR_RANDN",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // shape, key, device
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcodeWithMode(TensorSubOpcode::Rand, 1),
        mlir_op: Some("verum.tensor_randn"),
        doc: "Create tensor with standard normal random values",
    },
    Intrinsic {
        name: "TENSOR_RANDINT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 5, // shape, low, high, key, device
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcodeWithMode(TensorSubOpcode::Rand, 2),
        mlir_op: Some("verum.tensor_randint"),
        doc: "Create tensor with random integers in [low, high)",
    },
    Intrinsic {
        name: "TENSOR_CLONE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Clone),
        mlir_op: Some("verum.tensor_clone"),
        doc: "Create a copy of a tensor",
    },
    Intrinsic {
        name: "TENSOR_EYE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // n, device
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Identity),
        mlir_op: Some("verum.tensor_eye"),
        doc: "Create identity matrix",
    },
    // Tensor Shape Operations
    Intrinsic {
        name: "TENSOR_RESHAPE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, new_shape
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorReshape),
        mlir_op: Some("verum.tensor_reshape"),
        doc: "Reshape tensor to new shape",
    },
    Intrinsic {
        name: "TENSOR_TRANSPOSE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorTranspose),
        mlir_op: Some("verum.tensor_transpose"),
        doc: "Transpose tensor (swap last two dimensions)",
    },
    Intrinsic {
        name: "TENSOR_PERMUTE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, dims
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Permute),
        mlir_op: Some("verum.tensor_permute"),
        doc: "Permute tensor dimensions",
    },
    Intrinsic {
        name: "TENSOR_SLICE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, ranges
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorSlice),
        mlir_op: Some("verum.tensor_slice"),
        doc: "Slice tensor along dimensions",
    },
    Intrinsic {
        name: "TENSOR_INDEX",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, indices
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Index),
        mlir_op: Some("verum.tensor_index"),
        doc: "Index into tensor",
    },
    Intrinsic {
        name: "TENSOR_INDEX_SELECT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, axis, indices
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Index),
        mlir_op: Some("verum.tensor_index_select"),
        doc: "Select indices along axis",
    },
    Intrinsic {
        name: "TENSOR_GATHER",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, axis, indices
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Gather),
        mlir_op: Some("verum.tensor_gather"),
        doc: "Gather elements along axis",
    },
    Intrinsic {
        name: "TENSOR_CONCAT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tensors, axis
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Concat),
        mlir_op: Some("verum.tensor_concat"),
        doc: "Concatenate tensors along axis",
    },
    Intrinsic {
        name: "TENSOR_CAT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tensors, axis
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Concat),
        mlir_op: Some("verum.tensor_cat"),
        doc: "Concatenate tensors along axis (alias)",
    },
    Intrinsic {
        name: "TENSOR_STACK",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tensors, axis
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Stack),
        mlir_op: Some("verum.tensor_stack"),
        doc: "Stack tensors along new axis",
    },
    Intrinsic {
        name: "TENSOR_BROADCAST",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, shape
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Broadcast),
        mlir_op: Some("verum.tensor_broadcast"),
        doc: "Broadcast tensor to shape",
    },
    Intrinsic {
        name: "TENSOR_EXPAND",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, new_shape
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Broadcast),
        mlir_op: Some("verum.tensor_expand"),
        doc: "Expand tensor to new shape",
    },
    Intrinsic {
        name: "TENSOR_SQUEEZE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, axis
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Squeeze),
        mlir_op: Some("verum.tensor_squeeze"),
        doc: "Remove singleton dimension",
    },
    Intrinsic {
        name: "TENSOR_UNSQUEEZE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, axis
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorUnsqueeze),
        mlir_op: Some("verum.tensor_unsqueeze"),
        doc: "Add singleton dimension",
    },
    // Tensor Element-wise Operations
    Intrinsic {
        name: "TENSOR_BINOP",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // a, b, op
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorBinop),
        mlir_op: Some("verum.tensor_binop"),
        doc: "Element-wise binary operation",
    },
    Intrinsic {
        name: "TENSOR_UNOP",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, op
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorUnop),
        mlir_op: Some("verum.tensor_unop"),
        doc: "Element-wise unary operation",
    },
    Intrinsic {
        name: "TENSOR_CMP",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // a, b, op
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Cmp),
        mlir_op: Some("verum.tensor_cmp"),
        doc: "Element-wise comparison",
    },
    Intrinsic {
        name: "TENSOR_WHERE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // condition, x, y
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Where),
        mlir_op: Some("verum.tensor_where"),
        doc: "Element-wise conditional selection",
    },
    Intrinsic {
        name: "TENSOR_CLAMP",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, min, max
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Clamp),
        mlir_op: Some("verum.tensor_clamp"),
        doc: "Clamp tensor values to range",
    },
    Intrinsic {
        name: "TENSOR_CAST",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, dtype
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Cast),
        mlir_op: Some("verum.tensor_cast"),
        doc: "Cast tensor to different dtype",
    },
    Intrinsic {
        name: "TENSOR_MASKED_FILL",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, mask, value
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::MaskedFill),
        mlir_op: Some("verum.tensor_masked_fill"),
        doc: "Fill tensor where mask is true",
    },
    Intrinsic {
        name: "TENSOR_MASKED_SELECT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tensor, mask
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorMaskedSelect),
        mlir_op: Some("verum.tensor_masked_select"),
        doc: "Select elements where mask is true",
    },
    Intrinsic {
        name: "TENSOR_LERP",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // start, end, weight
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Lerp),
        mlir_op: Some("verum.tensor_lerp"),
        doc: "Linear interpolation",
    },
    Intrinsic {
        name: "TENSOR_LEAKY_RELU",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, alpha
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorLeakyRelu),
        mlir_op: Some("verum.tensor_leaky_relu"),
        doc: "Leaky ReLU activation",
    },
    // Tensor Linear Algebra
    Intrinsic {
        name: "TENSOR_MATMUL",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // a, b
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorMatmul),
        mlir_op: Some("verum.tensor_matmul"),
        doc: "Matrix multiplication",
    },
    Intrinsic {
        name: "TENSOR_MM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // a, b
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorMatmul),
        mlir_op: Some("verum.tensor_mm"),
        doc: "Matrix multiplication (alias)",
    },
    Intrinsic {
        name: "TENSOR_MV",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // matrix, vector
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorMatmul),
        mlir_op: Some("verum.tensor_mv"),
        doc: "Matrix-vector multiplication",
    },
    Intrinsic {
        name: "TENSOR_BMM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // a, b
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::BatchMatmul),
        mlir_op: Some("verum.tensor_bmm"),
        doc: "Batched matrix multiplication",
    },
    Intrinsic {
        name: "TENSOR_DOT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // a, b
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Dot),
        mlir_op: Some("verum.tensor_dot"),
        doc: "Dot product",
    },
    Intrinsic {
        name: "TENSOR_OUTER",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // a, b
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Outer),
        mlir_op: Some("verum.tensor_outer"),
        doc: "Outer product",
    },
    Intrinsic {
        name: "TENSOR_EINSUM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // equation, tensors
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Einsum),
        mlir_op: Some("verum.tensor_einsum"),
        doc: "Einstein summation",
    },
    Intrinsic {
        name: "TENSOR_CONV",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // input, weight, params
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Conv),
        mlir_op: Some("verum.tensor_conv"),
        doc: "Convolution operation",
    },
    Intrinsic {
        name: "TENSOR_CONV2D",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 5, // input, weight, stride, padding, dilation
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Conv),
        mlir_op: Some("verum.tensor_conv2d"),
        doc: "2D convolution operation with explicit stride, padding, and dilation",
    },
    // Tensor Decompositions
    Intrinsic {
        name: "TENSOR_SVD",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::MultiReturn],
        param_count: 1, // tensor
        return_count: 3, // u, s, vh
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::SVD),
        mlir_op: Some("verum.tensor_svd"),
        doc: "Singular value decomposition",
    },
    Intrinsic {
        name: "TENSOR_QR",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::MultiReturn],
        param_count: 1, // tensor
        return_count: 2, // q, r
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::QR),
        mlir_op: Some("verum.tensor_qr"),
        doc: "QR decomposition",
    },
    Intrinsic {
        name: "TENSOR_LU",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::MultiReturn],
        param_count: 1, // tensor
        return_count: 3, // p, l, u
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::LU),
        mlir_op: Some("verum.tensor_lu"),
        doc: "LU decomposition with pivoting",
    },
    Intrinsic {
        name: "TENSOR_CHOLESKY",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Cholesky),
        mlir_op: Some("verum.tensor_cholesky"),
        doc: "Cholesky decomposition",
    },
    Intrinsic {
        name: "TENSOR_EIG",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::MultiReturn],
        param_count: 1, // tensor
        return_count: 2, // eigenvalues, eigenvectors
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Eig),
        mlir_op: Some("verum.tensor_eig"),
        doc: "Eigenvalue decomposition",
    },
    Intrinsic {
        name: "TENSOR_EIGH",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::MultiReturn],
        param_count: 1, // tensor
        return_count: 2, // eigenvalues, eigenvectors
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::EigSymmetric),
        mlir_op: Some("verum.tensor_eigh"),
        doc: "Symmetric eigenvalue decomposition",
    },
    // Tensor Solvers
    Intrinsic {
        name: "TENSOR_SOLVE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // a, b
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Solve),
        mlir_op: Some("verum.tensor_solve"),
        doc: "Solve linear system Ax = B",
    },
    Intrinsic {
        name: "TENSOR_TRI_SOLVE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // a, b
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TriSolve),
        mlir_op: Some("verum.tensor_tri_solve"),
        doc: "Triangular solve",
    },
    Intrinsic {
        name: "TENSOR_INVERSE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Inverse),
        mlir_op: Some("verum.tensor_inverse"),
        doc: "Matrix inverse",
    },
    Intrinsic {
        name: "TENSOR_DET",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Det),
        mlir_op: Some("verum.tensor_det"),
        doc: "Matrix determinant",
    },
    Intrinsic {
        name: "TENSOR_TRACE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Trace),
        mlir_op: Some("verum.tensor_trace"),
        doc: "Matrix trace",
    },
    Intrinsic {
        name: "TENSOR_DIAG",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, offset
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorDiag),
        mlir_op: Some("verum.tensor_diag"),
        doc: "Extract diagonal or create diagonal matrix",
    },
    Intrinsic {
        name: "TENSOR_TRIU",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, diagonal
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorTriu),
        mlir_op: Some("verum.tensor_triu"),
        doc: "Upper triangular part",
    },
    Intrinsic {
        name: "TENSOR_TRIL",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, diagonal
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorTril),
        mlir_op: Some("verum.tensor_tril"),
        doc: "Lower triangular part",
    },
    // Tensor Reductions
    Intrinsic {
        name: "TENSOR_REDUCE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, op, axis
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorReduce),
        mlir_op: Some("verum.tensor_reduce"),
        doc: "Reduce tensor along axis",
    },
    Intrinsic {
        name: "TENSOR_REDUCE_ALL",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, op
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::TensorReduce, 0xFF),
        mlir_op: Some("verum.tensor_reduce_all"),
        doc: "Reduce tensor over all elements",
    },
    Intrinsic {
        name: "TENSOR_REDUCE_KEEPDIM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, op, axis
        return_count: 1,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::TensorReduce, 1),
        mlir_op: Some("verum.tensor_reduce_keepdim"),
        doc: "Reduce tensor keeping dimension",
    },
    Intrinsic {
        name: "TENSOR_ARGMAX",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, axis
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Argmax),
        mlir_op: Some("verum.tensor_argmax"),
        doc: "Index of maximum value",
    },
    Intrinsic {
        name: "TENSOR_TOPK",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::MultiReturn],
        param_count: 3, // tensor, k, axis
        return_count: 2, // values, indices
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Topk),
        mlir_op: Some("verum.tensor_topk"),
        doc: "Top k values and indices",
    },
    Intrinsic {
        name: "TENSOR_CUMULATIVE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, op, axis
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Cumulative),
        mlir_op: Some("verum.tensor_cumulative"),
        doc: "Cumulative operation along axis",
    },
    // Tensor Normalization
    Intrinsic {
        name: "TENSOR_SOFTMAX",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, axis
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Softmax),
        mlir_op: Some("verum.tensor_softmax"),
        doc: "Softmax activation",
    },
    Intrinsic {
        name: "TENSOR_LOG_SOFTMAX",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, axis
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcodeWithMode(TensorSubOpcode::Softmax, 1),
        mlir_op: Some("verum.tensor_log_softmax"),
        doc: "Log-softmax activation",
    },
    Intrinsic {
        name: "TENSOR_LAYER_NORM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 4, // tensor, normalized_shape, weight, bias
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::LayerNorm),
        mlir_op: Some("verum.tensor_layer_norm"),
        doc: "Layer normalization",
    },
    Intrinsic {
        name: "TENSOR_BATCH_NORM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 5, // tensor, mean, var, weight, bias
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::BatchNorm),
        mlir_op: Some("verum.tensor_batch_norm"),
        doc: "Batch normalization",
    },
    Intrinsic {
        name: "TENSOR_RMS_NORM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, weight
        return_count: 1,
        strategy: CodegenStrategy::TensorExtExtendedOpcode(TensorExtSubOpcode::RmsNorm),
        mlir_op: Some("verum.tensor_rms_norm"),
        doc: "RMS normalization",
    },
    // Tensor Advanced Operations
    Intrinsic {
        name: "TENSOR_SCATTER",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 4, // tensor, axis, indices, src
        return_count: 0,
        strategy: CodegenStrategy::TensorExtExtendedOpcode(TensorExtSubOpcode::Scatter),
        mlir_op: Some("verum.tensor_scatter"),
        doc: "Scatter values into tensor",
    },
    Intrinsic {
        name: "TENSOR_NONZERO",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorNonzero),
        mlir_op: Some("verum.tensor_nonzero"),
        doc: "Indices of nonzero elements",
    },
    Intrinsic {
        name: "TENSOR_ONE_HOT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // indices, num_classes, dtype
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorOneHot),
        mlir_op: Some("verum.tensor_one_hot"),
        doc: "Create one-hot encoding",
    },
    Intrinsic {
        name: "TENSOR_SPLIT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // tensor, chunks, axis
        return_count: 1, // list of tensors
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorSplit),
        mlir_op: Some("verum.tensor_split"),
        doc: "Split tensor into chunks",
    },
    Intrinsic {
        name: "TENSOR_SPLIT_AT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // tensor, indices, axis
        return_count: 1, // list of tensors
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorSplitAt),
        mlir_op: Some("verum.tensor_split_at"),
        doc: "Split tensor at indices",
    },
    Intrinsic {
        name: "TENSOR_REPEAT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tensor, repeats
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Repeat),
        mlir_op: Some("verum.tensor_repeat"),
        doc: "Repeat tensor along dimensions",
    },
    // Tensor Element Access
    Intrinsic {
        name: "TENSOR_GET_SCALAR",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, index
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::GetElementFromArgs),
        mlir_op: Some("verum.tensor_get_scalar"),
        doc: "Get scalar value at flat index",
    },
    Intrinsic {
        name: "TENSOR_SET_SCALAR",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 3, // tensor, index, value
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::SetElementFromArgs),
        mlir_op: Some("verum.tensor_set_scalar"),
        doc: "Set scalar value at flat index (returns new tensor)",
    },
    // Tensor Device Operations
    Intrinsic {
        name: "TENSOR_CONTIGUOUS",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorContiguous),
        mlir_op: Some("verum.tensor_contiguous"),
        doc: "Make tensor contiguous in memory",
    },
    Intrinsic {
        name: "TENSOR_CONTIGUOUS_VIEW",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // tensor
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorContiguousView),
        mlir_op: Some("verum.tensor_contiguous_view"),
        doc: "Get contiguous view of tensor",
    },
    Intrinsic {
        name: "TENSOR_TO_DEVICE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tensor, device
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TensorToDevice),
        mlir_op: Some("verum.tensor_to_device"),
        doc: "Move tensor to device",
    },
    Intrinsic {
        name: "TENSOR_FFT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // tensor, n
        return_count: 1,
        strategy: CodegenStrategy::TensorExtExtendedOpcode(TensorExtSubOpcode::Fft),
        mlir_op: Some("verum.tensor_fft"),
        doc: "Fast Fourier Transform",
    },
    Intrinsic {
        name: "TENSOR_FLASH_ATTENTION",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 4, // q, k, v, mask
        return_count: 1,
        strategy: CodegenStrategy::TensorExtExtendedOpcode(TensorExtSubOpcode::FlashAttention),
        mlir_op: Some("verum.tensor_flash_attention"),
        doc: "Flash attention",
    },
    // Memory Operations for Tensors
    Intrinsic {
        name: "MEM_NEW_ID",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MemNewId),
        mlir_op: Some("verum.mem_new_id"),
        doc: "Allocate new memory ID for tensor storage",
    },
    Intrinsic {
        name: "MEM_ALLOC_TENSOR",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // size, device
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MemAllocTensor),
        mlir_op: Some("verum.mem_alloc_tensor"),
        doc: "Allocate tensor memory storage",
    },
    // =========================================================================
    // Tensor Operations (SSM, FFT, Linear Algebra)
    // =========================================================================
    Intrinsic {
        name: "SSM_SCAN",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 4, // op, init, elements, dim
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::SsmScan),
        mlir_op: Some("verum.ssm_scan"),
        doc: "Parallel associative scan for State Space Models (Blelloch algorithm)",
    },
    Intrinsic {
        name: "MATRIX_EXP",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // A
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Expm),
        mlir_op: Some("verum.matrix_exp"),
        doc: "Matrix exponential using Padé approximation",
    },
    Intrinsic {
        name: "MATRIX_INVERSE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // A
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Inverse),
        mlir_op: Some("verum.matrix_inverse"),
        doc: "Matrix inverse using LU decomposition",
    },
    Intrinsic {
        name: "COMPLEX_POW",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // base, exp
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ComplexPow),
        mlir_op: Some("verum.complex_pow"),
        doc: "Complex number power operation",
    },
    Intrinsic {
        name: "COMPLEX_MUL",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // a, b
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ComplexMul),
        mlir_op: Some("verum.complex_mul"),
        doc: "Complex number multiplication",
    },
    Intrinsic {
        name: "RFFT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // x, n
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Rfft),
        mlir_op: Some("verum.rfft"),
        doc: "Real-to-complex FFT",
    },
    Intrinsic {
        name: "IRFFT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // x, n
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Irfft),
        mlir_op: Some("verum.irfft"),
        doc: "Complex-to-real inverse FFT",
    },
    Intrinsic {
        name: "UNIFORM",
        category: IntrinsicCategory::Tensor,
        hints: &[],
        param_count: 3, // shape, low, high
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Uniform),
        mlir_op: Some("verum.uniform"),
        doc: "Uniform random tensor in [low, high)",
    },
    Intrinsic {
        name: "IS_TRAINING",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::IsTraining),
        mlir_op: Some("verum.is_training"),
        doc: "Check if in training mode",
    },
    Intrinsic {
        name: "BINCOUNT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // indices, num_bins
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Bincount),
        mlir_op: Some("verum.bincount"),
        doc: "Histogram binning for expert load statistics",
    },
    Intrinsic {
        name: "GATHER_ND",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // x, indices
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::GatherNd),
        mlir_op: Some("verum.gather_nd"),
        doc: "N-dimensional gather operation",
    },
    Intrinsic {
        name: "ARANGE_USIZE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // start, end, step
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ArangeUsize),
        mlir_op: Some("verum.arange_usize"),
        doc: "Integer range tensor",
    },
    Intrinsic {
        name: "REPEAT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // x, times
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Repeat),
        mlir_op: Some("verum.repeat"),
        doc: "Repeat tensor along new dimension",
    },
    Intrinsic {
        name: "TANH",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // x
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Tanh),
        mlir_op: Some("verum.tanh"),
        doc: "Element-wise hyperbolic tangent on tensor",
    },
    Intrinsic {
        name: "SUM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // x
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::SumAll),
        mlir_op: Some("verum.sum"),
        doc: "Sum all elements in tensor",
    },
    Intrinsic {
        name: "TENSOR_FROM_ARRAY",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // values
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::FromArray),
        mlir_op: Some("verum.tensor_from_array"),
        doc: "Create tensor from array",
    },
    Intrinsic {
        name: "RANDOM_FLOAT_01",
        category: IntrinsicCategory::Tensor,
        hints: &[],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::RandomFloat01),
        mlir_op: Some("verum.random_float"),
        doc: "Random float in [0, 1)",
    },
    // =========================================================================
    // Tokenizer Operations
    // =========================================================================
    Intrinsic {
        name: "TOKENIZER_LOAD_BPE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // vocab_path, merges_path
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TokenizerLoadBpe),
        mlir_op: Some("verum.tokenizer_load_bpe"),
        doc: "Load BPE tokenizer from vocabulary and merges files",
    },
    Intrinsic {
        name: "TOKENIZER_LOAD_PRETRAINED",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // model_name
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TokenizerLoadPretrained),
        mlir_op: Some("verum.tokenizer_load_pretrained"),
        doc: "Load pretrained tokenizer by model name (e.g., 'gpt2', 'llama')",
    },
    Intrinsic {
        name: "TOKENIZER_ENCODE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tokenizer, text
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TokenizerEncode),
        mlir_op: Some("verum.tokenizer_encode"),
        doc: "Encode text to tokens using BPE tokenizer",
    },
    Intrinsic {
        name: "TOKENIZER_DECODE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tokenizer, tokens
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TokenizerDecode),
        mlir_op: Some("verum.tokenizer_decode"),
        doc: "Decode tokens to text using BPE tokenizer",
    },
    Intrinsic {
        name: "TOKENIZER_LOAD_SPM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // model_path
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TokenizerLoadSpm),
        mlir_op: Some("verum.tokenizer_load_spm"),
        doc: "Load SentencePiece tokenizer from model file",
    },
    Intrinsic {
        name: "TOKENIZER_SPM_ENCODE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tokenizer, text
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TokenizerSpmEncode),
        mlir_op: Some("verum.tokenizer_spm_encode"),
        doc: "Encode text to tokens using SentencePiece",
    },
    Intrinsic {
        name: "TOKENIZER_SPM_DECODE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tokenizer, tokens
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TokenizerSpmDecode),
        mlir_op: Some("verum.tokenizer_spm_decode"),
        doc: "Decode tokens to text using SentencePiece",
    },
    // =========================================================================
    // Sampling Operations
    // =========================================================================
    Intrinsic {
        name: "SAMPLE_TOP_P",
        category: IntrinsicCategory::Tensor,
        hints: &[],
        param_count: 2, // logits, p
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::SampleTopP),
        mlir_op: Some("verum.sample_top_p"),
        doc: "Top-p (nucleus) sampling from logits",
    },
    Intrinsic {
        name: "SAMPLE_TEMPERATURE",
        category: IntrinsicCategory::Tensor,
        hints: &[],
        param_count: 2, // logits, temperature
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::SampleTemperature),
        mlir_op: Some("verum.sample_temperature"),
        doc: "Temperature-scaled sampling from logits",
    },
    Intrinsic {
        name: "TENSOR_PAGED_ATTENTION",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 4, // q, kv_cache, block_table, context_len
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::PagedAttention),
        mlir_op: Some("verum.paged_attention"),
        doc: "Paged attention for efficient KV cache",
    },
    // =========================================================================
    // Inference Utility Operations
    // =========================================================================
    Intrinsic {
        name: "PARSE_TOOL_CALL",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // action
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ParseToolCall),
        mlir_op: Some("verum.parse_tool_call"),
        doc: "Parse tool call from action string",
    },
    Intrinsic {
        name: "FORMAT_VALUE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // value
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::FormatValue),
        mlir_op: Some("verum.format_value"),
        doc: "Format value for display",
    },
    Intrinsic {
        name: "TENSOR_FROM_SLICE_USIZE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // values
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TensorFromSliceUsize),
        mlir_op: Some("verum.tensor_from_slice_usize"),
        doc: "Create tensor from USize slice",
    },
    Intrinsic {
        name: "QUANTIZED_MATMUL",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 4, // input, weight, scale, zero_point
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::QuantizedMatmul),
        mlir_op: Some("verum.quantized_matmul"),
        doc: "Quantized matrix multiplication (int8 with scale/zero-point)",
    },
    Intrinsic {
        name: "TENSOR_NORM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // x
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::TensorNorm),
        mlir_op: Some("verum.tensor_norm"),
        doc: "Compute tensor norm (L2 by default)",
    },
    Intrinsic {
        name: "GENERATE_REQUEST_ID",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::GenerateRequestId),
        mlir_op: Some("verum.generate_request_id"),
        doc: "Generate unique request ID",
    },
    Intrinsic {
        name: "JSON_SCHEMA_TO_JSON",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // schema
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::JsonSchemaToJson),
        mlir_op: Some("verum.json_schema_to_json"),
        doc: "Convert JSON schema to JSON representation",
    },
    Intrinsic {
        name: "FUNCTION_SCHEMA_TO_JSON",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // schema
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::FunctionSchemaToJson),
        mlir_op: Some("verum.function_schema_to_json"),
        doc: "Convert function schema to JSON representation",
    },
    Intrinsic {
        name: "PARSE_FUNCTION_CALLS",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // response
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ParseFunctionCalls),
        mlir_op: Some("verum.parse_function_calls"),
        doc: "Parse function calls from model response",
    },
    // =========================================================================
    // Distributed/Collective Operations
    // =========================================================================
    Intrinsic {
        name: "COLLECTIVE_ALL_REDUCE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 3, // tensor, group, op
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::AllReduce),
        mlir_op: Some("verum.collective_all_reduce"),
        doc: "All-reduce: reduce tensor across all ranks and distribute result",
    },
    Intrinsic {
        name: "COLLECTIVE_ALL_GATHER",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 2, // tensor, group
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::AllGather),
        mlir_op: Some("verum.collective_all_gather"),
        doc: "All-gather: gather tensors from all ranks to all ranks",
    },
    Intrinsic {
        name: "COLLECTIVE_BROADCAST",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 3, // tensor, src, group
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Broadcast),
        mlir_op: Some("verum.collective_broadcast"),
        doc: "Broadcast: send tensor from src rank to all ranks",
    },
    Intrinsic {
        name: "COLLECTIVE_REDUCE_SCATTER",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 3, // tensor, group, op
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ReduceScatter),
        mlir_op: Some("verum.collective_reduce_scatter"),
        doc: "Reduce-scatter: reduce then scatter result",
    },
    Intrinsic {
        name: "COLLECTIVE_BARRIER",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // group
        return_count: 0,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Barrier),
        mlir_op: Some("verum.collective_barrier"),
        doc: "Barrier: synchronize all ranks",
    },
    Intrinsic {
        name: "PMAP_PSUM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 2, // tensor, axis_name
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::PmapPsum),
        mlir_op: Some("verum.pmap_psum"),
        doc: "Pmap parallel sum collective",
    },
    Intrinsic {
        name: "PMAP_PMEAN",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 2, // tensor, axis_name
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::PmapPmean),
        mlir_op: Some("verum.pmap_pmean"),
        doc: "Pmap parallel mean collective",
    },
    Intrinsic {
        name: "PMAP_PMAX",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 2, // tensor, axis_name
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::PmapPmax),
        mlir_op: Some("verum.pmap_pmax"),
        doc: "Pmap parallel max collective",
    },
    Intrinsic {
        name: "PMAP_ALL_GATHER",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 2, // tensor, axis_name
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::PmapAllGather),
        mlir_op: Some("verum.pmap_all_gather"),
        doc: "Pmap all-gather collective",
    },
    Intrinsic {
        name: "VMAP_TRANSFORM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // func, in_axes, out_axes
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::VmapTransform),
        mlir_op: Some("verum.vmap_transform"),
        doc: "Vmap transformation for automatic vectorization",
    },
    Intrinsic {
        name: "PMAP_TRANSFORM",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 4, // func, axis_name, in_axes, out_axes
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::PmapTransform),
        mlir_op: Some("verum.pmap_transform"),
        doc: "Pmap transformation for parallel execution across devices",
    },
    // =========================================================================
    // Automatic Differentiation Intrinsics
    // =========================================================================
    Intrinsic {
        name: "GRAD_BEGIN",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // mode byte (0x00=reverse, 0x01=forward)
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradBegin),
        mlir_op: Some("verum.grad_begin"),
        doc: "Enter gradient computation scope",
    },
    Intrinsic {
        name: "GRAD_END",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 0,
        return_count: 1, // pullback function
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradEnd),
        mlir_op: Some("verum.grad_end"),
        doc: "Exit gradient scope and return pullback function",
    },
    Intrinsic {
        name: "JVP_BEGIN",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // tangent input
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::JvpBegin),
        mlir_op: Some("verum.jvp_begin"),
        doc: "Enter JVP (forward mode) computation scope",
    },
    Intrinsic {
        name: "JVP_END",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 0,
        return_count: 1, // tangent output
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::JvpEnd),
        mlir_op: Some("verum.jvp_end"),
        doc: "Exit JVP scope and return tangent output",
    },
    Intrinsic {
        name: "GRAD_ZERO_TANGENT",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // zero tangent
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::GradZeroTangent),
        mlir_op: Some("verum.grad_zero_tangent"),
        doc: "Create zero tangent vector for current type",
    },
    Intrinsic {
        name: "GRAD_STOP",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // value to detach
        return_count: 1, // detached value
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradStop),
        mlir_op: Some("verum.grad_stop"),
        doc: "Stop gradient propagation (detach from computation graph)",
    },
    Intrinsic {
        name: "GRAD_CUSTOM",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // primal function, vjp function
        return_count: 1, // wrapped function
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::GradCustom),
        mlir_op: Some("verum.grad_custom"),
        doc: "Register custom VJP rule for a function",
    },
    Intrinsic {
        name: "GRAD_CHECKPOINT",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect, IntrinsicHint::Alloc],
        param_count: 2, // checkpoint_id, tensors list
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradCheckpoint),
        mlir_op: Some("verum.grad_checkpoint"),
        doc: "Checkpoint tensors for gradient recomputation",
    },
    Intrinsic {
        name: "GRAD_ACCUMULATE",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // destination, source gradient
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradAccumulate),
        mlir_op: Some("verum.grad_accumulate"),
        doc: "Accumulate gradient into destination",
    },
    Intrinsic {
        name: "GRAD_RECOMPUTE",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect, IntrinsicHint::Alloc],
        param_count: 1, // checkpoint_id
        return_count: 1, // recomputed tensors
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::GradRecompute),
        mlir_op: Some("verum.grad_recompute"),
        doc: "Recompute tensors from checkpoint",
    },
    Intrinsic {
        name: "GRAD_ZERO",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // gradient tensor
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::GradZero),
        mlir_op: Some("verum.grad_zero"),
        doc: "Zero out gradient tensor",
    },
    // =========================================================================
    // Additional Autodiff Operations
    // =========================================================================
    Intrinsic {
        name: "GRAD_BACKWARD",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // loss tensor
        return_count: 0,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ModuleBackward),
        mlir_op: Some("verum.grad_backward"),
        doc: "Execute backward pass from loss",
    },
    Intrinsic {
        name: "GRAD_SYNC",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradEnd),
        mlir_op: Some("verum.grad_sync"),
        doc: "Synchronize gradient computation",
    },
    Intrinsic {
        name: "GRAD_SCOPE_PUSH",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // recording_enabled
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradBegin),
        mlir_op: Some("verum.grad_scope_push"),
        doc: "Push gradient recording scope",
    },
    Intrinsic {
        name: "GRAD_SCOPE_PUSH_INFERENCE",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradBegin),
        mlir_op: Some("verum.grad_scope_push_inference"),
        doc: "Push inference-only scope (no gradient recording)",
    },
    Intrinsic {
        name: "GRAD_SCOPE_POP",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // restore_state
        return_count: 0,
        strategy: CodegenStrategy::DirectOpcode(Opcode::GradEnd),
        mlir_op: Some("verum.grad_scope_pop"),
        doc: "Pop gradient recording scope",
    },
    Intrinsic {
        name: "GRAD_GET_VJP_RULE",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // function
        return_count: 1, // vjp rule
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::GradCustom),
        mlir_op: Some("verum.grad_get_vjp_rule"),
        doc: "Get VJP rule for a differentiable function",
    },
    Intrinsic {
        name: "GRAD_GET_JVP_RULE",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // function
        return_count: 1, // jvp rule
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::GradCustom),
        mlir_op: Some("verum.grad_get_jvp_rule"),
        doc: "Get JVP rule for a differentiable function",
    },
    // =========================================================================
    // Distributed Operations (Process Groups)
    // =========================================================================
    Intrinsic {
        name: "DIST_WORLD_GROUP",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Pure],
        param_count: 0,
        return_count: 1, // world group handle
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::DistWorldGroup),
        mlir_op: Some("verum.dist_world_group"),
        doc: "Get the world process group (all ranks)",
    },
    Intrinsic {
        name: "DIST_NEW_GROUP",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // ranks list
        return_count: 1, // group handle
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::DistNewGroup),
        mlir_op: Some("verum.dist_new_group"),
        doc: "Create a new process group from ranks",
    },
    Intrinsic {
        name: "DIST_GET_RANK",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // group
        return_count: 1, // rank
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::DistGetRank),
        mlir_op: Some("verum.dist_get_rank"),
        doc: "Get current rank in a process group",
    },
    // =========================================================================
    // Point-to-Point Communication
    // =========================================================================
    Intrinsic {
        name: "P2P_SEND",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // tensor, dst_rank
        return_count: 0,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::P2PSend),
        mlir_op: Some("verum.p2p_send"),
        doc: "Send tensor to destination rank",
    },
    Intrinsic {
        name: "P2P_RECV",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 1, // src_rank
        return_count: 1, // received tensor
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::P2PRecv),
        mlir_op: Some("verum.p2p_recv"),
        doc: "Receive tensor from source rank",
    },
    // =========================================================================
    // Additional Collective Operations
    // =========================================================================
    Intrinsic {
        name: "COLLECTIVE_GATHER",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 3, // tensor, dst_rank, group
        return_count: 1, // gathered tensor (only at dst)
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::CollectiveGather),
        mlir_op: Some("verum.collective_gather"),
        doc: "Gather tensors to one rank",
    },
    Intrinsic {
        name: "COLLECTIVE_SCATTER",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 2, // tensor, group
        return_count: 1, // scattered chunk
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::CollectiveScatter),
        mlir_op: Some("verum.collective_scatter"),
        doc: "Scatter tensor chunks to all ranks",
    },
    // =========================================================================
    // Gradient/Parameter Operations
    // =========================================================================
    Intrinsic {
        name: "BUCKET_GRADIENTS",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // params, bucket_size
        return_count: 1, // bucketed gradients
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::BucketGradients),
        mlir_op: Some("verum.bucket_gradients"),
        doc: "Bucket gradients for efficient communication",
    },
    Intrinsic {
        name: "GET_GRAD",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // parameter
        return_count: 1, // gradient
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::GetGrad),
        mlir_op: Some("verum.get_grad"),
        doc: "Get gradient of a parameter",
    },
    Intrinsic {
        name: "SET_GRAD",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // parameter, gradient
        return_count: 0,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::SetGrad),
        mlir_op: Some("verum.set_grad"),
        doc: "Set gradient on a parameter",
    },
    Intrinsic {
        name: "MODULE_BACKWARD",
        category: IntrinsicCategory::Autodiff,
        hints: &[IntrinsicHint::SideEffect, IntrinsicHint::Alloc],
        param_count: 2, // module, grad_output
        return_count: 1, // grad_input
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ModuleBackward),
        mlir_op: Some("verum.module_backward"),
        doc: "Execute backward pass on a module",
    },
    // =========================================================================
    // Actor/Mesh Operations
    // =========================================================================
    Intrinsic {
        name: "MESH_SELECT",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // mesh, selector
        return_count: 1, // selected submesh
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::MeshSelect),
        mlir_op: Some("verum.mesh_select"),
        doc: "Select actors from a mesh",
    },
    Intrinsic {
        name: "ACTOR_NEW_ID",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // actor id
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::ActorNewId),
        mlir_op: Some("verum.actor_new_id"),
        doc: "Generate a new actor ID",
    },
    // =========================================================================
    // RDMA Operations
    // =========================================================================
    Intrinsic {
        name: "RDMA_CREATE_REF",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // tensor, actor_id
        return_count: 1, // rdma ref
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::RdmaCreateRef),
        mlir_op: Some("verum.rdma_create_ref"),
        doc: "Create RDMA reference to tensor",
    },
    Intrinsic {
        name: "RDMA_FETCH",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 1, // rdma ref
        return_count: 1, // tensor
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::RdmaFetch),
        mlir_op: Some("verum.rdma_fetch"),
        doc: "Fetch tensor via RDMA",
    },
    Intrinsic {
        name: "RDMA_WRITE",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // rdma ref, data
        return_count: 0,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::RdmaWrite),
        mlir_op: Some("verum.rdma_write"),
        doc: "Write tensor via RDMA",
    },
    Intrinsic {
        name: "RDMA_CHECK_VALID",
        category: IntrinsicCategory::Distributed,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // rdma ref
        return_count: 1, // bool
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::RdmaCheckValid),
        mlir_op: Some("verum.rdma_check_valid"),
        doc: "Check if RDMA reference is valid",
    },
    // =========================================================================
    // Additional Tensor Operations
    // =========================================================================
    Intrinsic {
        name: "TENSOR_SIGMOID",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // x
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorUnop),
        mlir_op: Some("verum.tensor_sigmoid"),
        doc: "Element-wise sigmoid activation",
    },
    Intrinsic {
        name: "TENSOR_BROADCAST_TO",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // x, target_shape
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Broadcast),
        mlir_op: Some("verum.tensor_broadcast_to"),
        doc: "Broadcast tensor to target shape",
    },
    Intrinsic {
        name: "TENSOR_COMPARE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // a, b, op
        return_count: 1,
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Cmp),
        mlir_op: Some("verum.tensor_compare"),
        doc: "Element-wise comparison operation",
    },
    Intrinsic {
        name: "TENSOR_ATTENTION_BACKWARD",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 4, // q, k, v, grad_output
        return_count: 3, // grad_q, grad_k, grad_v
        strategy: CodegenStrategy::TensorExtExtendedOpcode(TensorExtSubOpcode::FlashAttention),
        mlir_op: Some("verum.tensor_attention_backward"),
        doc: "Backward pass for attention",
    },
    Intrinsic {
        name: "TENSOR_CONV2D_BACKWARD_INPUT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // grad_output, weight
        return_count: 1, // grad_input
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Conv),
        mlir_op: Some("verum.tensor_conv2d_backward_input"),
        doc: "Backward pass for Conv2D w.r.t. input",
    },
    Intrinsic {
        name: "TENSOR_CONV2D_BACKWARD_WEIGHT",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // grad_output, input
        return_count: 1, // grad_weight
        strategy: CodegenStrategy::TensorExtendedOpcode(TensorSubOpcode::Conv),
        mlir_op: Some("verum.tensor_conv2d_backward_weight"),
        doc: "Backward pass for Conv2D w.r.t. weight",
    },
    Intrinsic {
        name: "FLATTEN",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, start_dim, end_dim
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorReshape),
        mlir_op: Some("verum.flatten"),
        doc: "Flatten tensor dimensions",
    },
    Intrinsic {
        name: "TRANSPOSE",
        category: IntrinsicCategory::Tensor,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // tensor, dim0, dim1
        return_count: 1,
        strategy: CodegenStrategy::DirectOpcode(Opcode::TensorTranspose),
        mlir_op: Some("verum.transpose"),
        doc: "Transpose tensor dimensions",
    },
    // =========================================================================
    // CBGR Operations
    // =========================================================================
    Intrinsic {
        name: "CBGR_NEW_GENERATION",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // generation id
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrNewGeneration),
        mlir_op: Some("verum.cbgr_new_generation"),
        doc: "Create a new CBGR generation",
    },
    Intrinsic {
        name: "CBGR_INVALIDATE",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // generation
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrInvalidate),
        mlir_op: Some("verum.cbgr_invalidate"),
        doc: "Invalidate a CBGR generation",
    },
    Intrinsic {
        name: "CBGR_GET_GENERATION",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // reference
        return_count: 1, // generation id
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrGetGeneration),
        mlir_op: Some("verum.cbgr_get_generation"),
        doc: "Get generation of a CBGR reference",
    },
    Intrinsic {
        name: "CBGR_ADVANCE_GENERATION",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // reference
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrAdvanceGeneration),
        mlir_op: Some("verum.cbgr_advance_generation"),
        doc: "Advance generation of a CBGR reference",
    },
    Intrinsic {
        name: "CBGR_GET_EPOCH_CAPS",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // reference
        return_count: 1, // epoch capabilities
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrGetEpochCaps),
        mlir_op: Some("verum.cbgr_get_epoch_caps"),
        doc: "Get epoch capabilities of a CBGR reference",
    },
    Intrinsic {
        name: "CBGR_BYPASS_BEGIN",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::SideEffect, IntrinsicHint::Unsafe],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrBypassBegin),
        mlir_op: Some("verum.cbgr_bypass_begin"),
        doc: "Begin CBGR bypass region (unsafe)",
    },
    Intrinsic {
        name: "CBGR_BYPASS_END",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::SideEffect, IntrinsicHint::Unsafe],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrBypassEnd),
        mlir_op: Some("verum.cbgr_bypass_end"),
        doc: "End CBGR bypass region (unsafe)",
    },
    Intrinsic {
        name: "CBGR_GET_STATS",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Pure],
        param_count: 0,
        return_count: 1, // stats structure
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrGetStats),
        mlir_op: Some("verum.cbgr_get_stats"),
        doc: "Get CBGR runtime statistics",
    },
    Intrinsic {
        name: "cbgr_alloc",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 2, // size, align
        return_count: 1, // Result<(ptr, generation, epoch)>
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrAlloc),
        mlir_op: Some("verum.cbgr_alloc"),
        doc: "Allocate memory with CBGR tracking",
    },
    Intrinsic {
        name: "cbgr_alloc_zeroed",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 2, // size, align
        return_count: 1, // Result<(ptr, generation, epoch)>
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrAllocZeroed),
        mlir_op: Some("verum.cbgr_alloc_zeroed"),
        doc: "Allocate zeroed memory with CBGR tracking",
    },
    Intrinsic {
        name: "cbgr_dealloc",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 3, // ptr, size, align
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrDealloc),
        mlir_op: Some("verum.cbgr_dealloc"),
        doc: "Deallocate CBGR-tracked memory",
    },
    Intrinsic {
        name: "cbgr_realloc",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Alloc, IntrinsicHint::SideEffect],
        param_count: 4, // ptr, old_size, new_size, align
        return_count: 1, // Result<(ptr, generation, epoch)>
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CbgrRealloc),
        mlir_op: Some("verum.cbgr_realloc"),
        doc: "Reallocate CBGR-tracked memory",
    },
    Intrinsic {
        name: "memcmp_bytes",
        category: IntrinsicCategory::Memory,
        hints: &[IntrinsicHint::Pure],
        param_count: 3, // lhs, rhs, len
        return_count: 1, // Int comparison result
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MemcmpBytes),
        mlir_op: Some("llvm.intr.memcmp"),
        doc: "Compare memory regions byte-by-byte",
    },
    Intrinsic {
        name: "get_header_from_ptr",
        category: IntrinsicCategory::Cbgr,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Unsafe],
        param_count: 1, // ptr
        return_count: 1, // header
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::GetHeaderFromPtr),
        mlir_op: Some("verum.cbgr_get_header"),
        doc: "Get CBGR allocation header from pointer",
    },
    // =========================================================================
    // GPU Operations - Device Management
    // =========================================================================
    Intrinsic {
        name: "GPU_GET_DEVICE",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Pure],
        param_count: 0,
        return_count: 1, // device id
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GetDevice),
        mlir_op: Some("verum.gpu_get_device"),
        doc: "Get current GPU device ID",
    },
    Intrinsic {
        name: "GPU_SET_DEVICE",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // device id
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::SetDevice),
        mlir_op: Some("verum.gpu_set_device"),
        doc: "Set current GPU device",
    },
    Intrinsic {
        name: "GPU_DEVICE_RESET",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::DeviceReset),
        mlir_op: Some("verum.gpu_device_reset"),
        doc: "Reset GPU device",
    },
    Intrinsic {
        name: "GPU_MEM_INFO",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::MultiReturn],
        param_count: 0,
        return_count: 2, // (free, total)
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GetMemoryInfo),
        mlir_op: Some("verum.gpu_mem_info"),
        doc: "Get GPU memory info (free, total)",
    },
    Intrinsic {
        name: "GPU_CAN_PEER",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // peer device
        return_count: 1, // bool
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::CanAccessPeer),
        mlir_op: Some("verum.gpu_can_peer"),
        doc: "Check if can access peer device",
    },
    Intrinsic {
        name: "GPU_ENABLE_PEER",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // peer device
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EnablePeerAccess),
        mlir_op: Some("verum.gpu_enable_peer"),
        doc: "Enable peer device access",
    },
    Intrinsic {
        name: "GPU_DISABLE_PEER",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // peer device
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::DisablePeerAccess),
        mlir_op: Some("verum.gpu_disable_peer"),
        doc: "Disable peer device access",
    },
    // =========================================================================
    // GPU Operations - Memory
    // =========================================================================
    Intrinsic {
        name: "GPU_MALLOC",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // size, memory_space
        return_count: 1, // ptr
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::Alloc),
        mlir_op: Some("verum.gpu_malloc"),
        doc: "Allocate GPU device memory",
    },
    Intrinsic {
        name: "GPU_MALLOC_MANAGED",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // size
        return_count: 1, // ptr
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::MallocManaged),
        mlir_op: Some("verum.gpu_malloc_managed"),
        doc: "Allocate unified/managed memory",
    },
    Intrinsic {
        name: "GPU_FREE",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // ptr
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::Free),
        mlir_op: Some("verum.gpu_free"),
        doc: "Free GPU memory",
    },
    Intrinsic {
        name: "GPU_PIN_MEMORY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // ptr, size
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::PinMemory),
        mlir_op: Some("verum.gpu_pin_memory"),
        doc: "Pin host memory for faster transfers",
    },
    Intrinsic {
        name: "GPU_UNPIN_MEMORY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // ptr
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::UnpinMemory),
        mlir_op: Some("verum.gpu_unpin_memory"),
        doc: "Unpin host memory",
    },
    Intrinsic {
        name: "GPU_PREFETCH",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 3, // ptr, size, device
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::Prefetch),
        mlir_op: Some("verum.gpu_prefetch"),
        doc: "Prefetch memory to device",
    },
    // =========================================================================
    // GPU Operations - Memory Transfer
    // =========================================================================
    Intrinsic {
        name: "GPU_MEMCPY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 3, // dst, src, size
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::Memcpy),
        mlir_op: Some("verum.gpu_memcpy"),
        doc: "Synchronous GPU memory copy",
    },
    Intrinsic {
        name: "GPU_MEMCPY_ASYNC",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 4, // dst, src, size, stream
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::MemcpyAsync),
        mlir_op: Some("verum.gpu_memcpy_async"),
        doc: "Asynchronous GPU memory copy",
    },
    Intrinsic {
        name: "GPU_MEMCPY_H2D",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 3, // dst, src, size
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::MemcpyH2D),
        mlir_op: Some("verum.gpu_memcpy_h2d"),
        doc: "Copy host to device",
    },
    Intrinsic {
        name: "GPU_MEMCPY_D2H",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 3, // dst, src, size
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::MemcpyD2H),
        mlir_op: Some("verum.gpu_memcpy_d2h"),
        doc: "Copy device to host",
    },
    Intrinsic {
        name: "GPU_MEMCPY_D2D",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 3, // dst, src, size
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::MemcpyD2D),
        mlir_op: Some("verum.gpu_memcpy_d2d"),
        doc: "Copy device to device",
    },
    Intrinsic {
        name: "GPU_MEMSET",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 3, // ptr, value, size
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::Memset),
        mlir_op: Some("verum.gpu_memset"),
        doc: "Set GPU memory",
    },
    Intrinsic {
        name: "GPU_MEMSET_ASYNC",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 4, // ptr, value, size, stream
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::MemsetAsync),
        mlir_op: Some("verum.gpu_memset_async"),
        doc: "Async set GPU memory",
    },
    // =========================================================================
    // GPU Operations - Streams
    // =========================================================================
    Intrinsic {
        name: "GPU_STREAM_CREATE",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // stream handle
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::StreamCreate),
        mlir_op: Some("verum.gpu_stream_create"),
        doc: "Create GPU stream",
    },
    Intrinsic {
        name: "GPU_STREAM_CREATE_PRIO",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // priority
        return_count: 1, // stream handle
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::StreamCreateWithPriority),
        mlir_op: Some("verum.gpu_stream_create_prio"),
        doc: "Create GPU stream with priority",
    },
    Intrinsic {
        name: "GPU_STREAM_DESTROY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // stream
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::StreamDestroy),
        mlir_op: Some("verum.gpu_stream_destroy"),
        doc: "Destroy GPU stream",
    },
    Intrinsic {
        name: "GPU_STREAM_QUERY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // stream
        return_count: 1, // bool (is_complete)
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::StreamQuery),
        mlir_op: Some("verum.gpu_stream_query"),
        doc: "Query stream completion status",
    },
    Intrinsic {
        name: "GPU_STREAM_WAIT_EVENT",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // stream, event
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::StreamWaitEvent),
        mlir_op: Some("verum.gpu_stream_wait_event"),
        doc: "Make stream wait for event",
    },
    // =========================================================================
    // GPU Operations - Synchronization
    // =========================================================================
    Intrinsic {
        name: "GPU_SYNC",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // stream
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::SyncStream),
        mlir_op: Some("verum.gpu_sync"),
        doc: "Synchronize GPU stream",
    },
    Intrinsic {
        name: "GPU_SYNC_ALL",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::SyncDevice),
        mlir_op: Some("verum.gpu_sync_all"),
        doc: "Synchronize all GPU streams",
    },
    // =========================================================================
    // GPU Operations - Events
    // =========================================================================
    Intrinsic {
        name: "GPU_EVENT_CREATE",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // event handle
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EventCreate),
        mlir_op: Some("verum.gpu_event_create"),
        doc: "Create GPU event",
    },
    Intrinsic {
        name: "GPU_EVENT_CREATE_F",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // flags
        return_count: 1, // event handle
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EventCreateWithFlags),
        mlir_op: Some("verum.gpu_event_create_with_flags"),
        doc: "Create GPU event with flags",
    },
    Intrinsic {
        name: "GPU_EVENT_DESTROY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // event
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EventDestroy),
        mlir_op: Some("verum.gpu_event_destroy"),
        doc: "Destroy GPU event",
    },
    Intrinsic {
        name: "GPU_EVENT_RECORD",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // event, stream
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EventRecord),
        mlir_op: Some("verum.gpu_event_record"),
        doc: "Record event on stream",
    },
    Intrinsic {
        name: "GPU_EVENT_SYNC",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // event
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EventSynchronize),
        mlir_op: Some("verum.gpu_event_sync"),
        doc: "Wait for event to complete",
    },
    Intrinsic {
        name: "GPU_EVENT_QUERY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Pure],
        param_count: 1, // event
        return_count: 1, // bool
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EventQuery),
        mlir_op: Some("verum.gpu_event_query"),
        doc: "Query event completion status",
    },
    Intrinsic {
        name: "GPU_EVENT_ELAPSED",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // start_event, end_event
        return_count: 1, // elapsed_ms
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EventElapsed),
        mlir_op: Some("verum.gpu_event_elapsed"),
        doc: "Get elapsed time between events",
    },
    // =========================================================================
    // GPU Operations - Graphs
    // =========================================================================
    Intrinsic {
        name: "GPU_GRAPH_CREATE",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // graph handle
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GraphCreate),
        mlir_op: Some("verum.gpu_graph_create"),
        doc: "Create GPU graph",
    },
    Intrinsic {
        name: "GPU_GRAPH_BEGIN",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // stream
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GraphBeginCapture),
        mlir_op: Some("verum.gpu_graph_begin"),
        doc: "Begin graph capture on stream",
    },
    Intrinsic {
        name: "GPU_GRAPH_END",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // stream
        return_count: 1, // graph handle
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GraphEndCapture),
        mlir_op: Some("verum.gpu_graph_end"),
        doc: "End graph capture",
    },
    Intrinsic {
        name: "GPU_GRAPH_INST",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 1, // graph
        return_count: 1, // exec handle
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GraphInstantiate),
        mlir_op: Some("verum.gpu_graph_instantiate"),
        doc: "Instantiate graph for execution",
    },
    Intrinsic {
        name: "GPU_GRAPH_LAUNCH",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // exec, stream
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GraphLaunch),
        mlir_op: Some("verum.gpu_graph_launch"),
        doc: "Launch graph on stream",
    },
    Intrinsic {
        name: "GPU_GRAPH_DESTROY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // graph
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GraphDestroy),
        mlir_op: Some("verum.gpu_graph_destroy"),
        doc: "Destroy GPU graph",
    },
    Intrinsic {
        name: "GPU_GRAPH_EXEC_DESTROY",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // exec
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GraphExecDestroy),
        mlir_op: Some("verum.gpu_graph_exec_destroy"),
        doc: "Destroy graph executable",
    },
    Intrinsic {
        name: "GPU_GRAPH_EXEC_UPDATE",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 2, // exec, graph
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::GraphExecUpdate),
        mlir_op: Some("verum.gpu_graph_exec_update"),
        doc: "Update graph executable",
    },
    // =========================================================================
    // GPU Operations - Kernel Launch
    // =========================================================================
    Intrinsic {
        name: "GPU_LAUNCH",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 6, // kernel_id, grid, block, shared_mem, stream, args
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::Launch),
        mlir_op: Some("verum.gpu_launch"),
        doc: "Launch GPU kernel",
    },
    Intrinsic {
        name: "GPU_LAUNCH_COOP",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 6, // kernel_id, grid, block, shared_mem, stream, args
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::LaunchCooperative),
        mlir_op: Some("verum.gpu_launch_coop"),
        doc: "Launch cooperative GPU kernel",
    },
    // =========================================================================
    // GPU Operations - Profiling
    // =========================================================================
    Intrinsic {
        name: "GPU_MARKER_PUSH",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 1, // name
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::ProfileMarkerPush),
        mlir_op: Some("verum.gpu_marker_push"),
        doc: "Push profiling marker",
    },
    Intrinsic {
        name: "GPU_MARKER_POP",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::ProfileMarkerPop),
        mlir_op: Some("verum.gpu_marker_pop"),
        doc: "Pop profiling marker",
    },
    // =========================================================================
    // GPU Operations - Device Enumeration
    // =========================================================================
    Intrinsic {
        name: "GPU_ENUMERATE_CUDA",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // list of devices
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EnumerateCuda),
        mlir_op: Some("verum.gpu_enumerate_cuda"),
        doc: "Enumerate available CUDA devices",
    },
    Intrinsic {
        name: "GPU_ENUMERATE_METAL",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // list of devices
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EnumerateMetal),
        mlir_op: Some("verum.gpu_enumerate_metal"),
        doc: "Enumerate available Metal devices",
    },
    Intrinsic {
        name: "GPU_ENUMERATE_ROCM",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // list of devices
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EnumerateRocm),
        mlir_op: Some("verum.gpu_enumerate_rocm"),
        doc: "Enumerate available ROCm devices",
    },
    Intrinsic {
        name: "GPU_ENUMERATE_VULKAN",
        category: IntrinsicCategory::Gpu,
        hints: &[IntrinsicHint::Alloc],
        param_count: 0,
        return_count: 1, // list of devices
        strategy: CodegenStrategy::GpuExtendedOpcode(GpuSubOpcode::EnumerateVulkan),
        mlir_op: Some("verum.gpu_enumerate_vulkan"),
        doc: "Enumerate available Vulkan devices",
    },
    // =========================================================================
    // Logging Operations
    // =========================================================================
    Intrinsic {
        name: "LOG_INFO",
        category: IntrinsicCategory::Logging,
        hints: &[IntrinsicHint::SideEffect, IntrinsicHint::IoEffect],
        param_count: 1, // message
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::LogInfo),
        mlir_op: Some("verum.log_info"),
        doc: "Log info message",
    },
    Intrinsic {
        name: "LOG_WARNING",
        category: IntrinsicCategory::Logging,
        hints: &[IntrinsicHint::SideEffect, IntrinsicHint::IoEffect],
        param_count: 1, // message
        return_count: 0,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::LogWarning),
        mlir_op: Some("verum.log_warning"),
        doc: "Log warning message",
    },
    // =========================================================================
    // Regex Operations — backed by `regex` 1.x in the runtime kernel
    // (verum_vbc/src/interpreter/kernel/mod.rs::dispatch_regex_*). Names are
    // lowercase to match the canonical `@intrinsic("…")` attribute convention
    // used everywhere else in the registry; the previous uppercase spelling
    // for FIND_ALL/REPLACE_ALL was a typo that prevented Verum-side
    // `core/text/regex.vr` from resolving them.
    // =========================================================================
    Intrinsic {
        name: "regex_find_all",
        category: IntrinsicCategory::Regex,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // pattern, text
        return_count: 1, // matches list
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RegexFindAll),
        mlir_op: Some("verum.regex_find_all"),
        doc: "Find all regex matches in text",
    },
    Intrinsic {
        name: "regex_replace_all",
        category: IntrinsicCategory::Regex,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // pattern, text, replacement
        return_count: 1, // result text
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RegexReplaceAll),
        mlir_op: Some("verum.regex_replace_all"),
        doc: "Replace all regex matches in text",
    },
    Intrinsic {
        name: "regex_is_match",
        category: IntrinsicCategory::Regex,
        hints: &[IntrinsicHint::Pure],
        param_count: 2, // pattern, text
        return_count: 1, // bool
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RegexIsMatch),
        mlir_op: Some("verum.regex_is_match"),
        doc: "Test whether a regex pattern matches anywhere in text",
    },
    Intrinsic {
        name: "regex_split",
        category: IntrinsicCategory::Regex,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // pattern, text
        return_count: 1, // parts list
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RegexSplit),
        mlir_op: Some("verum.regex_split"),
        doc: "Split text by a regex pattern",
    },
    Intrinsic {
        name: "regex_find",
        category: IntrinsicCategory::Regex,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // pattern, text
        return_count: 1, // Maybe<Text>
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RegexFind),
        mlir_op: Some("verum.regex_find"),
        doc: "Find the first regex match in text",
    },
    Intrinsic {
        name: "regex_replace",
        category: IntrinsicCategory::Regex,
        hints: &[IntrinsicHint::Alloc],
        param_count: 3, // pattern, text, replacement
        return_count: 1, // result text (one replacement)
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RegexReplace),
        mlir_op: Some("verum.regex_replace"),
        doc: "Replace the first regex match in text",
    },
    Intrinsic {
        name: "regex_captures",
        category: IntrinsicCategory::Regex,
        hints: &[IntrinsicHint::Alloc],
        param_count: 2, // pattern, text
        return_count: 1, // Maybe<List<Text>> — group 0 + capture groups
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RegexCaptures),
        mlir_op: Some("verum.regex_captures"),
        doc: "Run a capturing regex; return ordered group captures of the first match",
    },
    // =========================================================================
    // Time Operations
    // =========================================================================
    Intrinsic {
        name: "TIME_NOW_UNIX",
        category: IntrinsicCategory::Time,
        hints: &[IntrinsicHint::SideEffect],
        param_count: 0,
        return_count: 1, // unix timestamp
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::RealtimeSecs),
        mlir_op: Some("verum.time_now_unix"),
        doc: "Get current Unix timestamp",
    },
    // =========================================================================
    // Type Introspection Operations
    // =========================================================================
    Intrinsic {
        name: "SIZE_OF",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Generic],
        param_count: 0, // type parameter only
        return_count: 1, // size in bytes
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SizeOf),
        mlir_op: Some("verum.size_of"),
        doc: "Get size of type in bytes (compile-time evaluated)",
    },
    Intrinsic {
        name: "ALIGN_OF",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Generic],
        param_count: 0, // type parameter only
        return_count: 1, // alignment in bytes
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::AlignOf),
        mlir_op: Some("verum.align_of"),
        doc: "Get alignment of type in bytes (compile-time evaluated)",
    },
    Intrinsic {
        name: "TYPE_ID",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Generic],
        param_count: 0, // type parameter only
        return_count: 1, // unique type id
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TypeId),
        mlir_op: Some("verum.type_id"),
        doc: "Get unique type identifier (compile-time evaluated)",
    },
    Intrinsic {
        name: "TYPE_NAME",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Generic],
        param_count: 0, // type parameter only
        return_count: 1, // type name string
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::TypeName),
        mlir_op: Some("verum.type_name"),
        doc: "Get human-readable type name (compile-time evaluated)",
    },
    Intrinsic {
        name: "NEEDS_DROP",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::ConstEval, IntrinsicHint::Generic],
        param_count: 0, // type parameter only
        return_count: 1, // bool
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::NeedsDrop),
        mlir_op: Some("verum.needs_drop"),
        doc: "Check if type needs drop glue (compile-time evaluated)",
    },
    // =========================================================================
    // Additional intrinsics for user-facing stdlib functions
    // =========================================================================
    Intrinsic {
        name: "pow_f32",
        category: IntrinsicCategory::Math,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PowF32),
        mlir_op: Some("llvm.intr.pow.f32"),
        doc: "Float32 power function",
    },
    Intrinsic {
        name: "char_is_alphanumeric",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CharIsAlphanumeric),
        mlir_op: None,
        doc: "Check if character is alphanumeric",
    },
    Intrinsic {
        name: "rdtsc",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Inline],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::Rdtsc),
        mlir_op: Some("llvm.intr.readcyclecounter"),
        doc: "Read CPU timestamp counter",
    },
    Intrinsic {
        name: "catch_unwind",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::MemoryEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::CatchUnwind),
        mlir_op: None,
        doc: "Execute closure and catch panics, returning Result",
    },
    Intrinsic {
        name: "ptr_to_ref",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::Pure, IntrinsicHint::Inline],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::PtrToRef),
        mlir_op: None,
        doc: "Convert raw pointer to reference",
    },
    Intrinsic {
        name: "spin_loop_hint",
        category: IntrinsicCategory::Atomic,
        hints: &[IntrinsicHint::Inline],
        param_count: 0,
        return_count: 0,
        strategy: CodegenStrategy::OpcodeWithMode(Opcode::AtomicFence, 0xFF),
        mlir_op: Some("llvm.intr.x86.sse2.pause"),
        doc: "CPU spin hint for busy-waiting (alias for spin_hint)",
    },
    // =========================================================================
    // System Call Intrinsics (darwin/libsystem safe wrappers)
    // =========================================================================
    Intrinsic {
        name: "sys_getpid",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SysGetpid),
        mlir_op: Some("verum.sys.getpid"),
        doc: "Get process ID (always succeeds)",
    },
    Intrinsic {
        name: "sys_gettid",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 0,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SysGettid),
        mlir_op: Some("verum.sys.gettid"),
        doc: "Get thread ID",
    },
    Intrinsic {
        name: "sys_mmap",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect, IntrinsicHint::MemoryEffect],
        param_count: 6,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SysMmap),
        mlir_op: Some("verum.sys.mmap"),
        doc: "Memory map (safe wrapper returning Result)",
    },
    Intrinsic {
        name: "sys_munmap",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect, IntrinsicHint::MemoryEffect],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SysMunmap),
        mlir_op: Some("verum.sys.munmap"),
        doc: "Memory unmap (safe wrapper returning Result)",
    },
    Intrinsic {
        name: "sys_madvise",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect, IntrinsicHint::MemoryEffect],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SysMadvise),
        mlir_op: Some("verum.sys.madvise"),
        doc: "Memory advise (safe wrapper returning Result)",
    },
    Intrinsic {
        name: "sys_getentropy",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::SysGetentropy),
        mlir_op: Some("verum.sys.getentropy"),
        doc: "Get cryptographic random bytes (safe wrapper returning Result)",
    },
    // =========================================================================
    // Mach Kernel Operations (macOS)
    // =========================================================================
    Intrinsic {
        name: "mach_vm_allocate",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect, IntrinsicHint::MemoryEffect],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachVmAllocate),
        mlir_op: Some("verum.mach.vm_allocate"),
        doc: "Mach VM allocate (safe wrapper returning Result<VmAddress, KernReturn>)",
    },
    Intrinsic {
        name: "mach_vm_deallocate",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect, IntrinsicHint::MemoryEffect],
        param_count: 2,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachVmDeallocate),
        mlir_op: Some("verum.mach.vm_deallocate"),
        doc: "Mach VM deallocate (safe wrapper returning Result<(), KernReturn>)",
    },
    Intrinsic {
        name: "mach_vm_protect",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect, IntrinsicHint::MemoryEffect],
        param_count: 3,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachVmProtect),
        mlir_op: Some("verum.mach.vm_protect"),
        doc: "Mach VM protect (safe wrapper returning Result<(), KernReturn>)",
    },
    Intrinsic {
        name: "mach_sem_create",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachSemCreate),
        mlir_op: Some("verum.mach.sem_create"),
        doc: "Mach semaphore create (safe wrapper returning Result<SemaphoreT, KernReturn>)",
    },
    Intrinsic {
        name: "mach_sem_destroy",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachSemDestroy),
        mlir_op: Some("verum.mach.sem_destroy"),
        doc: "Mach semaphore destroy (safe wrapper returning Result<(), KernReturn>)",
    },
    Intrinsic {
        name: "mach_sem_signal",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachSemSignal),
        mlir_op: Some("verum.mach.sem_signal"),
        doc: "Mach semaphore signal (safe wrapper returning Result<(), KernReturn>)",
    },
    Intrinsic {
        name: "mach_sem_wait",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachSemWait),
        mlir_op: Some("verum.mach.sem_wait"),
        doc: "Mach semaphore wait (safe wrapper returning Result<(), KernReturn>)",
    },
    Intrinsic {
        name: "mach_error_string",
        category: IntrinsicCategory::Platform,
        hints: &[],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachErrorString),
        mlir_op: Some("verum.mach.error_string"),
        doc: "Mach error string lookup (returns Text)",
    },
    Intrinsic {
        name: "mach_sleep_until",
        category: IntrinsicCategory::Platform,
        hints: &[IntrinsicHint::IoEffect],
        param_count: 1,
        return_count: 1,
        strategy: CodegenStrategy::InlineSequence(InlineSequenceId::MachSleepUntil),
        mlir_op: Some("verum.mach.sleep_until"),
        doc: "Mach sleep until deadline (safe wrapper returning Result<(), KernReturn>)",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_initialization() {
        let registry = IntrinsicRegistry::new();
        assert!(registry.count() > 100, "Should have 100+ intrinsics");
    }

    #[test]
    fn test_intrinsic_lookup() {
        let info = INTRINSIC_REGISTRY.lookup("atomic_load_u64");
        assert!(info.is_some());
        let intrinsic = info.unwrap();
        assert_eq!(intrinsic.category, IntrinsicCategory::Atomic);
        assert_eq!(intrinsic.param_count, 2);
        assert_eq!(intrinsic.return_count, 1);
    }

    #[test]
    fn test_direct_opcode_intrinsics() {
        let info = INTRINSIC_REGISTRY.lookup("wrapping_add").unwrap();
        assert!(matches!(
            info.strategy,
            CodegenStrategy::DirectOpcode(Opcode::AddI)
        ));
    }

    #[test]
    fn test_syscall_intrinsics() {
        for i in 0..=6 {
            let name = format!("syscall{}", i);
            let info = INTRINSIC_REGISTRY.lookup(&name);
            assert!(info.is_some(), "Missing syscall{}", i);
            assert_eq!(info.unwrap().param_count, (i + 1) as u8);
        }
    }

    #[test]
    fn test_atomic_intrinsic_hints() {
        let info = INTRINSIC_REGISTRY.lookup("atomic_cas_u64").unwrap();
        assert!(info.hints.contains(&IntrinsicHint::MemoryEffect));
        assert!(info.hints.contains(&IntrinsicHint::MultiReturn));
    }

    #[test]
    fn test_pure_intrinsics() {
        let info = INTRINSIC_REGISTRY.lookup("wrapping_add").unwrap();
        assert!(info.is_pure());
        assert!(info.is_const_eval());
    }

    #[test]
    fn test_category_lookup() {
        let atomics = INTRINSIC_REGISTRY.by_category(IntrinsicCategory::Atomic);
        assert!(atomics.len() > 15, "Should have 15+ atomic intrinsics");
        assert!(atomics.contains(&"atomic_load_u64"));
    }

    #[test]
    fn test_unknown_intrinsic() {
        assert!(INTRINSIC_REGISTRY.lookup("nonexistent").is_none());
    }

    /// Pin the regex intrinsic naming. Earlier two of the four entries used
    /// uppercase ("REGEX_FIND_ALL") which broke `core/text/regex.vr` because
    /// `lookup` is case-sensitive and every other entry uses lowercase. Two
    /// more (is_match, split) had no entry at all even though the dispatcher
    /// existed in the interpreter kernel. This regression test fails loudly
    /// if any of the four lowercase names disappears or returns the wrong
    /// strategy.
    #[test]
    fn test_regex_intrinsics_registered_lowercase() {
        for (name, expected_params, expected_returns) in [
            ("regex_is_match", 2u8, 1u8),
            ("regex_find_all", 2, 1),
            ("regex_replace_all", 3, 1),
            ("regex_split", 2, 1),
            // #25 close-out: single-match / capture variants.
            ("regex_find", 2, 1),
            ("regex_replace", 3, 1),
            ("regex_captures", 2, 1),
        ] {
            let intr = INTRINSIC_REGISTRY
                .lookup(name)
                .unwrap_or_else(|| panic!("regex intrinsic `{name}` must be registered"));
            assert_eq!(intr.category, IntrinsicCategory::Regex, "{name} category");
            assert_eq!(intr.param_count, expected_params, "{name} param_count");
            assert_eq!(intr.return_count, expected_returns, "{name} return_count");
            assert!(
                matches!(intr.strategy, CodegenStrategy::InlineSequence(_)),
                "{name} strategy should be InlineSequence"
            );
        }
        // Guard against re-introduction of the uppercase typo.
        assert!(INTRINSIC_REGISTRY.lookup("REGEX_FIND_ALL").is_none());
        assert!(INTRINSIC_REGISTRY.lookup("REGEX_REPLACE_ALL").is_none());
        assert!(INTRINSIC_REGISTRY.lookup("REGEX_FIND").is_none());
        assert!(INTRINSIC_REGISTRY.lookup("REGEX_REPLACE").is_none());
        assert!(INTRINSIC_REGISTRY.lookup("REGEX_CAPTURES").is_none());
    }
}
